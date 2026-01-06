/**
 * Shared UI initializa        initializeAllNoteCards();on, event handling, and update logic
 * for commit and review pages.
 */
window.SharedUI = (function() {

    // --- Constants ---
    const CONTAINER_SELECTOR = '.container-fluid';
    const NOTE_CONTEXT_SELECTOR = '.note-context';
    const ORIGINAL_CONTENT_SELECTOR = '.original-content';
    const CURRENT_CONTENT_SELECTOR = '.current-content'; // For review page 'left' side
    const DIFF_CONTENT_SELECTOR = '.diff-content';
    const SUGGESTION_BOX_SELECTOR = '.suggestion-box';
    const FIELD_ACTIONS_SELECTOR = '.field-actions';
    const TAG_CONTAINER_SELECTOR = '.note_tag_container';
    const LAZY_IMG_SELECTOR = 'img.lazy-load-image';
    const ACTION_BUTTON_SELECTOR = '[data-action]';
    const PLACEHOLDER_IMG_SRC = '/static/images/click_to_show.webp';
    const ERROR_IMG_SRC = '/static/images/placeholder-error.png';

    // --- Initialization ---

    /**
     * Initializes the entire page (commit or review).
     */
    function initializePage() {
        initializeAllNoteCards();
        setupEventHandlers();
    }

    /**
     * Processes all note cards on the page.
     */
    function initializeAllNoteCards() {
        $(NOTE_CONTEXT_SELECTOR).each(function() {
            initializeNoteCard(this);
        });
    }

    /**
     * Initializes a single note card: renders content and processes diffs.
     * @param {HTMLElement} noteCardElement - The note card element.
     */
    function initializeNoteCard(noteCardElement) {
        const $noteCard = $(noteCardElement);
        const noteId = $noteCard.attr('id');

        // 1. Render Original/Current Content (Left/Top Side) & Prepare Images
        $noteCard.find(`${ORIGINAL_CONTENT_SELECTOR}, ${CURRENT_CONTENT_SELECTOR}`).each(function() {
            const $contentDiv = $(this);
            const htmlContent = HtmlDiffUtils.unescapeHtml($contentDiv.data('content') || '');
            // Render the HTML content FIRST
            $contentDiv.html(htmlContent);
            // THEN explicitly find images *within this div* and prepare them
            prepareImagesForLazyLoad($contentDiv.find('img')); // Pass the jQuery object of images
        });

        // 2. Process Diff Content (Right/Bottom Side)
        $noteCard.find(DIFF_CONTENT_SELECTOR).each(function() {
            processDiffField(this);
            // Note: processDiffField calls prepareImagesForLazyLoad internally on the diff div's images
        });
    }

    /**
      * Processes a single diff field: applies image wrappers and prepares images.
      * @param {HTMLElement} diffElement - The .diff-content element.
      */
    function processDiffField(diffElement) {
        const $diffDiv = $(diffElement);
        if (!$diffDiv.length) return;

        const diffHtml = $diffDiv.html(); // Current HTML (backend generated diff)
        const originalHtml = HtmlDiffUtils.unescapeHtml($diffDiv.data('original') || '');
        const newHtml = HtmlDiffUtils.unescapeHtml($diffDiv.data('new-content') || '');

        // Apply visual wrappers for images within <ins>/<del>
        const processedDiffHtml = ImageHandler.processHtmlDiffs(originalHtml, newHtml, diffHtml);

        if (processedDiffHtml !== diffHtml) {
            $diffDiv.html(processedDiffHtml);
        }
        // Ensure lazy loading is applied to any images *within* this processed diff
        // Target images that might be inside wrappers or directly in the diff content
        prepareImagesForLazyLoad($diffDiv.find('img')); // Pass the jQuery object of images
    }


    // --- Image Handling ---

    /**
     * Finds images needing lazy loading and prepares them.
     * @param {jQuery} $imageTargets - A jQuery object containing the specific image elements to process.
     */
    function prepareImagesForLazyLoad($imageTargets) {
        if (!$imageTargets || $imageTargets.length === 0) {
            // console.log("prepareImagesForLazyLoad: No images provided.");
            return;
        }
        // console.log(`Preparing ${$imageTargets.length} images for lazy load.`);

        $imageTargets.each(function() {
            const $img = $(this);

            // --- Basic Sanity Checks ---
            // Skip if already processed for lazy loading OR if it's clearly an error/placeholder image
            if ($img.hasClass('lazy-load-image') || $img.attr('src') === PLACEHOLDER_IMG_SRC || $img.attr('src') === ERROR_IMG_SRC) {
                // console.log(" -> Skipping: Already lazy or placeholder", $img.attr('src'));
                return; // continue .each
            }

            const currentSrc = $img.attr('src');

            // --- Identify Local Filenames ---
            // Check if the src looks like a local filename we should handle
            if (currentSrc && !currentSrc.startsWith('http') && !currentSrc.startsWith('data:image') && currentSrc.includes('.')) {
                const filename = currentSrc; // Assume src is the filename

                // --- Check Context ---
                const context = ApiService.getContext(this);
                if (!context.type || !context.id) {
                    console.warn("Could not find context for image, cannot lazy load:", filename, this);
                    // Optionally mark as error? $img.attr('src', ERROR_IMG_SRC);
                    return; // Skip if no context found
                }

                // --- Apply Lazy Load ---
                // console.log(` -> Applying lazy load to: ${filename}`);
                $img.addClass('lazy-load-image')
                    .attr('src', PLACEHOLDER_IMG_SRC) // Set placeholder image
                    .attr('data-filename', filename); // Store original filename for loading

            } else {
                // console.log(" -> Skipping image (not a local filename):", currentSrc);
            }
        });
    }


    /**
     * Handles the click on a lazy-load image placeholder.
     * @param {HTMLElement} imgElement - The placeholder <img> element.
     */
    async function loadPresignedImage(imgElement) {
        const $img = $(imgElement);
        // Ensure it's actually a lazy-load image that hasn't been loaded/failed
        if (!$img.hasClass('lazy-load-image') || $img.hasClass('loading')) return;

        const filename = $img.data('filename');
        if (!filename) {
            console.error("Lazy load image missing data-filename:", imgElement);
            $img.attr('src', ERROR_IMG_SRC).removeClass('lazy-load-image').addClass('error');
            return;
        }

        const context = ApiService.getContext(imgElement);
        if (!context.type || !context.id) {
            $img.attr('src', ERROR_IMG_SRC).removeClass('lazy-load-image').addClass('error');
            // Error logged by getContext
            return;
        }

        $img.addClass('loading').removeClass('error'); // Show loading state

        try {
            const result = await ApiService.getPresignedImageUrl(filename, context.type, context.id);
            if (result && result.presigned_url) {
                $img.attr('src', result.presigned_url);
                // Keep data-filename until save/clean! Needed for editor prep.
                $img.removeClass('lazy-load-image loading').addClass('loaded'); // Mark as loaded
                $img.off('click.lazyload'); // Remove specific listener for this image
            } else {
                throw new Error(result?.error || 'Presigned URL not found in response');
            }
        } catch (error) {
            console.error(`Failed to load presigned URL for ${filename}:`, error);
            $img.attr('src', ERROR_IMG_SRC)
                .removeClass('lazy-load-image loading')
                .addClass('error');
            // Keep listener? Maybe offer retry? For now, just show error.
        }
    }

    // --- Event Handling ---

    /**
     * Sets up delegated event handlers for the page.
     */
    function setupEventHandlers() {
        const $container = $(CONTAINER_SELECTOR);

        // Cleanup existing handlers to prevent duplicates if re-initialized
        $container.off('.sharedUI');

        // Lazy Load Image Click (Delegate to container)
        $container.on('click.sharedUI', LAZY_IMG_SELECTOR, function(event) {
            event.preventDefault();
            loadPresignedImage(this);
        });

        // General Action Buttons (Accept/Deny Tag/Field/Move, Edit, Cancel)
        $container.on('click.sharedUI', ACTION_BUTTON_SELECTOR, function(event) {
            event.preventDefault();
            const $button = $(this);
            const action = $button.data('action');
            const noteId = $button.data('note-id') || ApiService.getContext(this).id;

            if ($button.prop('disabled')) return;
            $button.prop('disabled', true); // Disable button immediately

            // Use a variable to re-enable button in case of error/completion if needed
            let enableButtonOnError = true;

            // --- Route action ---
            switch (action) {
                case 'accept-tag':
                    handleTagAction(ApiService.acceptTag, $button.data('tag-id'), $button);
                    enableButtonOnError = false; // Action removes element or button
                    break;
                case 'deny-tag':
                    handleTagAction(ApiService.denyTag, $button.data('tag-id'), $button);
                     enableButtonOnError = false;
                    break;
                case 'accept-move':
                    handleMoveAction(ApiService.acceptMove, $button.data('move-id'), $button);
                    enableButtonOnError = false;
                    break;
                case 'deny-move':
                    handleMoveAction(ApiService.denyMove, $button.data('move-id'), $button);
                    enableButtonOnError = false;
                    break;
                case 'accept-field':
                    handleFieldAction(ApiService.acceptField, $button.data('field-id'), $button);
                     enableButtonOnError = false;
                    break;
                case 'deny-field':
                    handleFieldAction(ApiService.denyField, $button.data('field-id'), $button);
                     enableButtonOnError = false;
                    break;
                case 'accept-note':
                    handleNoteAction(ApiService.acceptNote, noteId, $button, {
                        action: 'accept-note',
                        errorMessage: 'Failed to publish the note. Please try again.'
                    });
                    enableButtonOnError = false;
                    break;
                case 'delete-note':
                    handleNoteAction(ApiService.deleteNote, noteId, $button, {
                        action: 'delete-note',
                        errorMessage: 'Failed to delete the note. Please try again.'
                    });
                    enableButtonOnError = false;
                    break;
                case 'accept-note-removal':
                    handleNoteAction(ApiService.acceptNoteRemoval, noteId, $button, {
                        action: 'accept-note-removal',
                        errorMessage: 'Failed to confirm the note deletion. Please try again.'
                    });
                    enableButtonOnError = false;
                    break;
                case 'deny-note-removal':
                    handleNoteAction(ApiService.denyNoteRemoval, noteId, $button, {
                        action: 'deny-note-removal',
                        errorMessage: 'Failed to keep the note. Please try again.'
                    });
                    enableButtonOnError = false;
                    break;
                case 'edit-all-fields':
                    handleEditAllFields(noteId, $button);
                    enableButtonOnError = false; // Button state managed by FieldEditPanel
                    break;
                default:
                    console.warn("Unhandled action:", action);
                    enableButtonOnError = true; // Re-enable if unknown action
            }

            // Re-enable button if the action failed and didn't handle its own state
            if (enableButtonOnError) {
                // Check if the button still exists (might have been removed on success)
                if ($button.closest('body').length) {
                     $button.prop('disabled', false);
                }
            }
        });
    }

    // --- Action Handlers (Called by Event Delegator) ---

    async function handleTagAction(apiMethod, tagId, $button) {
        // Look for container - try multiple approaches for robustness
        let $container = $button.closest('.note_tag_container, .suggestion-box');
        
        // If not found, try traversing up manually for review page structure
        if ($container.length === 0) {
            $container = $button.closest('.tag-suggestion').parent();
        }
        
        const isAcceptAction = apiMethod === ApiService.acceptTag;
        
        $container.css('opacity', 0.5);
        try {
            await apiMethod(tagId); // No content expected
            
            if (isAcceptAction) {
                // For accept: extract tag content and add to published tags
                const $tagChip = $container.find('.tag-chip');
                const tagText = $tagChip.text().trim();
                const isAddAction = $tagChip.hasClass('add');
                const isRemoveAction = $tagChip.hasClass('remove');
                
                // Add to or remove from the published tags container
                const $noteCard = $button.closest('.note-context');
                let $publishedTagsContainer = $noteCard.find('.reviewed-tags-panel .collapsible-body');

                if ($publishedTagsContainer.length === 0) {
                    $publishedTagsContainer = $('#mainTags');

                    // If no published tags container exists, create it (replace empty state)
                    if ($publishedTagsContainer.length === 0) {
                        const $emptyState = $('.published-side .empty-state');
                        if ($emptyState.length > 0) {
                            const $tagsSection = $(`
                                <div class="field-item">
                                    <div class="field-header">
                                        <div class="field-name">🏷️ Current Tags</div>
                                    </div>
                                    <div class="tag-container" id="mainTags"></div>
                                </div>
                            `);
                            $emptyState.replaceWith($tagsSection);
                            $publishedTagsContainer = $('#mainTags');
                        }
                    }
                }
                if ($publishedTagsContainer.length) {
                    const panelEl = $publishedTagsContainer.closest('details')[0];
                    if (panelEl) {
                        panelEl.open = true;
                    }
                    if (isAddAction) {
                        // Add new tag to published tags
                        // Extract just the tag text, removing the plus icon
                        const cleanTagText = tagText.replace(/^[\+\s]*/, '').trim();
                        const $newTag = $('<span class="tag-chip reviewed-tag-chip">' + cleanTagText + '</span>');
                        $publishedTagsContainer.append($newTag);
                        
                        // Add a small animation to highlight the new tag
                        $newTag.css({
                            'background': '#10b981',
                            'color': 'white',
                            'transform': 'scale(1.1)'
                        });
                        setTimeout(() => {
                            $newTag.css({
                                'background': '',
                                'color': '',
                                'transform': ''
                            });
                        }, 1000);
                    } else if (isRemoveAction) {
                        // Remove tag from published tags
                        // Extract just the tag text, removing the minus icon
                        const cleanTagText = tagText.replace(/^[\-\s]*/, '').trim();
                        $publishedTagsContainer.find('.tag-chip').each(function() {
                            if ($(this).text().trim() === cleanTagText) {
                                $(this).fadeOut(300, function() { $(this).remove(); });
                                return false; // Break out of loop
                            }
                        });
                    }
                }
            }
            
            // Remove the suggestion container
            $container.fadeOut(300, function() { $(this).remove(); });
        } catch (error) {
            console.error(`Tag action failed for ID ${tagId}:`, error);
            $container.css('opacity', 1);
            $button.prop('disabled', false); // Re-enable button on failure
        }
    }

     async function handleMoveAction(apiMethod, moveId, $button) {
         const $container = $button.closest(TAG_CONTAINER_SELECTOR);
         $container.css('opacity', 0.5);
         try {
             await apiMethod(moveId); // No content expected
             if (apiMethod === ApiService.acceptMove) {
                 $container.css({'border': '2px solid mediumseagreen', 'opacity': 1})
                           .find('.tag_actions').remove(); // Remove buttons on success
             } else { // Deny Move
                 $container.fadeOut(300, function() { $(this).remove(); });
             }
         } catch (error) {
             console.error(`Move action failed for ID ${moveId}:`, error);
             $container.css('opacity', 1);
             $button.prop('disabled', false); // Re-enable on failure
         }
     }

     /**
      * Handles Accept/Deny for Field suggestions.
      * Assumes API returns success status only (no content).
      */
     async function handleFieldAction(apiMethod, fieldId, $button) {
         const $suggestionBox = $button.closest(SUGGESTION_BOX_SELECTOR);
         const $diffDiv = $suggestionBox.find(`${DIFF_CONTENT_SELECTOR}[data-field-id='${fieldId}']`);
         const $fieldActions = $button.closest(FIELD_ACTIONS_SELECTOR);
         $suggestionBox.css('opacity', 0.5);

         try {
             // Make the API call - expecting success status only, no content
             await apiMethod(fieldId);

             if (apiMethod === ApiService.acceptField) {
                 // --- Acceptance Succeeded ---
                 const newContent = $diffDiv.data('new-content'); // Get suggested content from data attr

                 if (newContent !== undefined) {
                     const unescapedContent = HtmlDiffUtils.unescapeHtml(newContent);

                     // Update diff view to show the final accepted content
                     $diffDiv.html(unescapedContent);
                     // Update data attributes: original now matches the accepted content
                     $diffDiv.data('original', newContent);
                     // data-new-content remains the accepted content

                     // Update the corresponding 'original' side display if it exists on the page
                     const $originalDiv = $suggestionBox.closest('.row').find(`${ORIGINAL_CONTENT_SELECTOR}[data-field-id='${fieldId}']`);
                     if ($originalDiv.length) {
                         $originalDiv.html(unescapedContent);
                         $originalDiv.data('content', newContent); // Update its data cache too
                         // Re-prepare images in the updated original div
                         prepareImagesForLazyLoad($originalDiv.find('img'));
                     }
                     // Re-prepare images in the updated diff div
                     prepareImagesForLazyLoad($diffDiv.find('img'));

                 } else {
                     console.warn(`AcceptField success for ID ${fieldId}, but data('new-content') was missing.`);
                     // Fallback: Just mark as accepted visually without changing content?
                     $diffDiv.html('<em>Content accepted (preview unavailable).</em>');
                 }
                 // Mark visually as accepted and remove action buttons
                 $fieldActions.remove();
                 $suggestionBox.removeClass('suggestion-box').css({'opacity': 1, 'border-left': '3px solid mediumseagreen'});

             } else { // --- Deny Field Succeeded ---
                 // Remove the entire suggestion box from the UI
                 $suggestionBox.fadeOut(300, function() { $(this).remove(); });
             }

         } catch (error) {
             console.error(`Field action failed for ID ${fieldId}:`, error);
             // TODO: Show user-friendly error (toast?)
             $suggestionBox.css('opacity', 1); // Restore opacity
             $button.prop('disabled', false); // Re-enable the specific button that failed
         }
         // No finally block needed as success cases remove the buttons/box
     }

     async function handleNoteAction(apiMethod, noteId, $trigger, options = {}) {
         const normalizedId = normalizeNoteId(noteId);

         if (!normalizedId) {
             console.error('Note action is missing a valid note id.', {
                 action: options.action,
                 noteId
             });
             restoreInteractiveButton($trigger);
             return;
         }

         const $noteCard = findNoteCardElement(normalizedId, $trigger);
         setNoteProcessingState($noteCard, true);

        try {
            await apiMethod(normalizedId);

            const meta = typeof ApiService.getLastResponseMeta === 'function'
                ? ApiService.getLastResponseMeta()
                : null;

            if (meta && meta.redirected) {
                let finalPath = '';
                try {
                    finalPath = new URL(meta.url, window.location.origin).pathname || '';
                } catch (urlError) {
                    finalPath = '';
                }

                if (finalPath === '/' || finalPath === '') {
                    throw new Error('Note action redirected to an unexpected location.');
                }
            }

            removeNoteCard($noteCard, options.action);
         } catch (error) {
             console.error(`Note action failed for ID ${normalizedId}:`, error);
             setNoteProcessingState($noteCard, false);
             restoreInteractiveButton($trigger);
             if (options.errorMessage) {
                 alert(options.errorMessage);
             }
         }
     }

     function normalizeNoteId(noteId) {
         if (noteId === undefined || noteId === null) {
             return '';
         }

         if (typeof noteId === 'number') {
             return Number.isFinite(noteId) ? String(noteId) : '';
         }

         if (typeof noteId === 'string') {
             return noteId.trim();
         }

         return String(noteId).trim();
     }

     function findNoteCardElement(noteId, $origin) {
         if (noteId) {
             const $byId = $(`#${noteId}`);
             if ($byId.length) {
                 if ($byId.hasClass('note-card')) {
                     return $byId;
                 }

                 const $idCardParent = $byId.closest('.note-card');
                 if ($idCardParent.length) {
                     return $idCardParent;
                 }

                 return $byId;
             }
         }

         if ($origin && $origin.length) {
             const $card = $origin.closest('.note-card');
             if ($card.length) {
                 return $card;
             }

             const $context = $origin.closest(NOTE_CONTEXT_SELECTOR);
             if ($context.length) {
                 return $context;
             }
         }

         return $();
     }

     function setNoteProcessingState($noteCard, isProcessing) {
         if (!$noteCard || !$noteCard.length) {
             return;
         }

         if (isProcessing) {
             $noteCard.css('opacity', 0.55);
         } else {
             $noteCard.css('opacity', '');
         }
     }

     function removeNoteCard($noteCard, action) {
         if (!$noteCard || !$noteCard.length) {
             $(document).trigger('note:resolved', { action });
             return;
         }

         $noteCard.fadeOut(220, function() {
             $(this).remove();
             $(document).trigger('note:resolved', { action });
         });
     }

     function restoreInteractiveButton($element) {
         if (!$element || !$element.length) {
             return;
         }

         if (typeof window.restoreButton === 'function') {
             window.restoreButton($element);
             return;
         }

         $element.removeClass('disabled loading')
             .prop('disabled', false)
             .css('pointer-events', '')
             .removeAttr('aria-busy');
     }


    /**
     * Handles the "Edit All Fields" button click.
     * Opens the FieldEditPanel for editing all fields of a note.
     * @param {string|number} noteId - The note ID
     * @param {jQuery} $button - The clicked button element
     */
    function handleEditAllFields(noteId, $button) {
        const commitId = $button.data('commit-id');
        
        if (!commitId) {
            console.error('No commit ID available for Edit All Fields');
            $button.prop('disabled', false);
            return;
        }

        // Check if FieldEditPanel is available
        if (typeof window.FieldEditPanel === 'undefined') {
            console.error('FieldEditPanel module not loaded');
            $button.prop('disabled', false);
            return;
        }

        // Open the panel - it handles its own button state management
        window.FieldEditPanel.open(noteId, commitId, $button[0])
            .finally(() => {
                // Re-enable button after panel operation completes
                if ($button.closest('body').length) {
                    $button.prop('disabled', false);
                }
            });
    }


    // --- Public API ---
    return {
        initializePage,
    };
})();