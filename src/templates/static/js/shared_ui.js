/**
 * Shared UI initialization, event handling, and update logic
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
    const EDIT_BTN_SELECTOR = '.edit-suggestion-btn';
    const ABORT_BTN_SELECTOR = '.abort-edit-btn';
    const ACTION_BUTTON_SELECTOR = 'button[data-action]';
    const PLACEHOLDER_IMG_SRC = '/static/images/click_to_show.webp';
    const ERROR_IMG_SRC = '/static/images/placeholder-error.png';

    // --- Initialization ---

    /**
     * Initializes the entire page (commit or review).
     */
    function initializePage() {
        console.log("Initializing page...");
        initializeAllNoteCards();
        setupEventHandlers();
        console.log("Page initialization complete.");
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
        console.log(`Initializing note card: ${noteId}`);

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
                case 'toggle-edit':
                    handleToggleEdit(noteId, $button);
                    // Button state managed internally by toggle/save logic
                    enableButtonOnError = false; // Don't re-enable here
                    break;
                case 'cancel-edit':
                    handleCancelEdit(noteId, $button);
                    // Re-enable immediately after cancel
                    $button.prop('disabled', false);
                    enableButtonOnError = false; // Already handled
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
        const $container = $button.closest(TAG_CONTAINER_SELECTOR);
        $container.css('opacity', 0.5);
        try {
            await apiMethod(tagId); // No content expected
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


    // --- Edit Mode Handling ---
    // (handleToggleEdit, handleCancelEdit, resetEditButtonState - No changes needed based on feedback)
     function handleToggleEdit(noteId, $button) {
        const $noteCard = $(`#${noteId}`);
        const $textSpan = $button.find('span');
        const $iconElement = $button.find('i');

        if ($textSpan.text().trim() === 'Edit Suggestion') {
            // --- Switch to Edit Mode ---
            $textSpan.text('Update Suggestion');
            $button.removeClass('btn-light').addClass('btn-primary');
            $iconElement.removeClass('icon-pencil').addClass('icon-check');
            $noteCard.find(ABORT_BTN_SELECTOR).show();
            $noteCard.find(FIELD_ACTIONS_SELECTOR).hide(); // Hide accept/deny for fields

            // Start async editor initialization
            EditorControls.initializeEditorsForNote(noteId)
                .then(() => {
                    console.log(`Editors initialized for note ${noteId}`);
                    $button.prop('disabled', false); // Re-enable button after init
                })
                .catch(error => {
                    console.error(`Failed to initialize editors for note ${noteId}:`, error);
                    alert("Error initializing editor(s). Please check console.");
                    handleCancelEdit(noteId, $noteCard.find(ABORT_BTN_SELECTOR)); // Attempt to cancel/revert
                    $button.prop('disabled', false);
                });
        } else {
            // --- Save Changes ---
            $textSpan.text('Updating...');
            // Button remains disabled until save completes (success or fail)
            EditorControls.saveEditorsForNote(noteId)
                .then(allSuccess => {
                    console.log(`Save finished for note ${noteId}. All successful: ${allSuccess}`);
                    if (!allSuccess) {
                        alert("One or more fields failed to update. Please check field highlights and console.");
                    }
                    // Reset button state regardless of success/failure of individual fields
                    resetEditButtonState(noteId);
                    // Show field actions again (if any fields remain)
                    $noteCard.find(FIELD_ACTIONS_SELECTOR).show();
                    $noteCard.find(ABORT_BTN_SELECTOR).hide();

                    // Re-process diffs and images for updated fields
                    $noteCard.find(`${DIFF_CONTENT_SELECTOR}.needs-reprocess`).each(function() {
                        processDiffField(this); // This handles wrappers and lazy loading
                        $(this).removeClass('needs-reprocess');
                    });
                })
                .catch(error => {
                    console.error(`Critical error saving changes for note ${noteId}:`, error);
                    alert("A critical error occurred while saving. Please try again or refresh.");
                     resetEditButtonState(noteId);
                     $noteCard.find(FIELD_ACTIONS_SELECTOR).show();
                     $noteCard.find(ABORT_BTN_SELECTOR).hide();
                })
                .finally(() => {
                    // Ensure button is re-enabled in all cases after promise settles
                    // Check button still exists before trying to enable
                     if ($button.closest('body').length) {
                        $button.prop('disabled', false);
                     }
                });
        }
    }

    function handleCancelEdit(noteId, $button) {
        const $noteCard = $(`#${noteId}`);
        console.log(`Cancelling edit for note ${noteId}`);

        EditorControls.destroyEditorsForNote(noteId); // Destroy editors first

        // Restore pre-edit diff views
        $noteCard.find(DIFF_CONTENT_SELECTOR).each(function() {
            const $fieldDiv = $(this);
            const preEditHtml = $fieldDiv.data('pre-edit-html');
            if (preEditHtml !== undefined) {
                $fieldDiv.html(preEditHtml).removeData('pre-edit-html');
                // Re-apply image processing and lazy loading to restored HTML
                processDiffField(this);
            }
             // Clean up any error states
             $fieldDiv.css('border', '');
        });

        // Reset UI elements
        $noteCard.find(FIELD_ACTIONS_SELECTOR).show(); // Show accept/deny again
        $noteCard.find(ABORT_BTN_SELECTOR).hide();
        resetEditButtonState(noteId); // Reset main edit button
    }

    function resetEditButtonState(noteId) {
        const $editButton = $(`#${noteId}`).find(EDIT_BTN_SELECTOR);
        // Check if button exists before manipulating
        if (!$editButton.length) return;

        const $textSpan = $editButton.find('span');
        const $iconElement = $editButton.find('i');

        $textSpan.text('Edit Suggestion');
        $editButton.removeClass('btn-primary').addClass('btn-light');
        $iconElement.removeClass('icon-check').addClass('icon-pencil');
        $editButton.prop('disabled', false); // Ensure enabled
    }


    // --- Public API ---
    return {
        initializePage,
    };
})();