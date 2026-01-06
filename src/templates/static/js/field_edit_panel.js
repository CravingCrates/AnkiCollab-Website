/**
 * FieldEditPanel - Handles the "Edit All Fields" panel for editing suggestions.
 * Provides an inline expansion to edit all fields of a note at once.
 */
window.FieldEditPanel = (function() {
    'use strict';

    // --- State ---
    let activePanel = null;
    let originalFieldData = new Map(); // Store original data for cancel/restore

    // --- Constants ---
    const PARAGRAPH_CLEANUP_EVENTS = 'tbwpaste.paragraphCleanup tbwblur.paragraphCleanup';

    // --- Configuration ---
    const TRUMBOWYG_OPTIONS = {
        svgPath: '/static/plugins/trumbowyg/ui/icons.svg',
        semantic: false,
        tagsToRemove: ['script'],
        removeformatPasted: true,
        resetCss: true,
        changeActiveDropdownIcon: true,
        autogrow: true,
        btnsDef: {
            customImage: {
                fn() {
                    alert('Direct image upload is not supported. Please add images via Anki and suggest the change.');
                },
                title: 'Insert Image (Disabled)',
                text: 'Image',
                ico: 'insertImage',
            },
            underline: {
                key: 'U',
                tag: 'u',
            },
        },
        btns: [
            ['viewHTML'],
            ['undo', 'redo'],
            ['strong', 'em', 'underline'],
            ['superscript', 'subscript'],
            ['link'],
            ['customImage'],
            ['foreColor'],
            ['justifyLeft', 'justifyCenter', 'justifyRight', 'justifyFull'],
            ['unorderedList', 'orderedList'],
            ['horizontalRule'],
            ['removeformat'],
            ['fullscreen'],
        ],
    };

    // --- Public API ---

    /**
     * Opens the edit panel for a note within a specific commit context.
     * @param {string|number} noteId - The note ID
     * @param {string|number} commitId - The commit ID to scope changes to
     * @param {HTMLElement} triggerElement - The button that triggered this action
     */
    async function open(noteId, commitId, triggerElement) {
        const $noteCard = $(`#${noteId}`);
        if (!$noteCard.length) {
            console.error('Could not find note card for ID:', noteId);
            return;
        }

        // Close any existing panel first
        if (activePanel) {
            close(activePanel.noteId);
        }

        // Show loading state
        const $trigger = $(triggerElement);
        $trigger.prop('disabled', true).addClass('loading');
        const originalHtml = $trigger.html();
        $trigger.html('<i class="fa fa-spinner fa-spin"></i> Loading...');

        // Helper to restore button state
        const restoreButton = () => {
            $trigger.prop('disabled', false).removeClass('loading').html(originalHtml);
        };

        try {
            // Fetch all fields data from API
            const response = await ApiService.getAllFieldsForEdit(noteId, commitId);
            
            if (response.error) {
                console.error('Error fetching fields:', response.error);
                showError($noteCard, response.error);
                restoreButton();
                return;
            }

            // Store state
            activePanel = {
                noteId: noteId,
                commitId: commitId,
                $noteCard: $noteCard,
                fieldsData: response.fields,
                noteReviewed: response.note_reviewed
            };

            // Store original data for each field
            originalFieldData.clear();
            response.fields.forEach(field => {
                originalFieldData.set(field.position, {
                    reviewedContent: field.reviewed_content,
                    suggestionContent: field.suggestion_content,
                    suggestionId: field.suggestion_id,
                    inherited: field.inherited
                });
            });

            // Render the panel
            renderPanel($noteCard, response, commitId);

            // Hide entire content-comparison to prevent empty space
            $noteCard.find('.content-comparison').hide();
            
            // Show and animate panel
            const $panel = $noteCard.find('.edit-all-fields-panel');
            $panel.hide();

            // Wait for slideDown animation to complete before initializing editors
            // Trumbowyg needs visible elements to initialize properly
            await new Promise((resolve) => {
                $panel.slideDown(300, resolve);
            });
            
            // Initialize editors AFTER slideDown completes (outside the callback)
            await initializeEditors($panel, noteId);

            // Hide Edit All Fields button while panel is open
            $noteCard.find('.edit-all-fields-btn').hide();
            
            // Restore button after successful initialization
            restoreButton();

        } catch (error) {
            console.error('Failed to open edit panel:', error);
            showError($noteCard, 'Failed to load fields. Please try again.');
            restoreButton();
        }
    }

    /**
     * Closes the edit panel and restores the original view.
     * @param {string|number} noteId - The note ID
     */
    function close(noteId) {
        const $noteCard = $(`#${noteId}`);
        if (!$noteCard.length) return;

        // Destroy all editors and event handlers
        $noteCard.find('.edit-field-editor').each(function() {
            try {
                $(this).off('.fieldEditPanel');
                $(this).off(PARAGRAPH_CLEANUP_EVENTS);
                $(this).trumbowyg('destroy');
            } catch (e) {
                // Editor might already be destroyed
            }
        });

        // Remove the panel
        $noteCard.find('.edit-all-fields-panel').slideUp(200, function() {
            $(this).remove();
        });

        // Show original content areas
        $noteCard.find('.content-comparison').show();

        // Show Edit All Fields button again
        $noteCard.find('.edit-all-fields-btn').show();

        // Clear state
        if (activePanel && String(activePanel.noteId) === String(noteId)) {
            activePanel = null;
        }
        originalFieldData.clear();
    }

    /**
     * Saves all modified fields in the panel.
     * @param {string|number} noteId - The note ID
     * @returns {Promise<boolean>} - True if save was successful
     */
    async function save(noteId) {
        if (!activePanel || String(activePanel.noteId) !== String(noteId)) {
            console.error('No active panel for note:', noteId);
            return false;
        }

        const $noteCard = activePanel.$noteCard;
        const $panel = $noteCard.find('.edit-all-fields-panel');
        const $saveBtn = $panel.find('.save-all-fields-btn');
        const $cancelBtn = $panel.find('.cancel-all-fields-btn');
        const $status = $panel.find('.edit-panel-status');

        // Collect modified fields
        const modifiedFields = collectModifiedFields($panel);
        
        if (modifiedFields.length === 0) {
            $status.html('<span class="text-muted">No changes to save.</span>');
            setTimeout(() => $status.html(''), 3000);
            return true;
        }

        // Show saving state - disable both buttons to prevent duplicate calls
        $saveBtn.prop('disabled', true).addClass('loading');
        $cancelBtn.prop('disabled', true);
        const originalBtnHtml = $saveBtn.html();
        $saveBtn.html('<i class="fa fa-spinner fa-spin"></i> Saving...');
        $status.html('<span class="text-info">Saving changes...</span>');

        try {
            const response = await ApiService.batchUpdateFieldSuggestions(
                activePanel.noteId,
                activePanel.commitId,
                modifiedFields
            );

            if (!response.success) {
                throw new Error('Save failed');
            }

            // Show success
            const changeCount = response.updated_count + response.created_count;
            $status.html(`<span class="text-success"><i class="fa fa-check"></i> Saved ${changeCount} field(s)! Reloading...</span>`);

            // Store complete page state for restoration after reload
            const scrollPosition = window.scrollY || document.documentElement.scrollTop;
            const $notesContainer = $('#notes-container');
            const restoreState = {
                scrollPosition: scrollPosition,
                noteId: String(noteId),
                // Pagination state
                loaded: $notesContainer.data('loaded') || 0,
                nextOffset: $notesContainer.data('nextOffset') || null,
                // Store timestamp to auto-expire the state
                timestamp: Date.now()
            };
            sessionStorage.setItem('fieldEditPanel_restoreState', JSON.stringify(restoreState));

            // Smooth transition before reload
            $panel.css('opacity', '0.6');
            
            // Brief delay to show success message, then reload
            setTimeout(() => {
                window.location.reload();
            }, 600);

            return true;

        } catch (error) {
            console.error('Failed to save fields:', error);
            $status.html('<span class="text-danger"><i class="fa fa-exclamation-circle"></i> Failed to save. Please try again.</span>');
            // Re-enable buttons only on error
            $saveBtn.prop('disabled', false).removeClass('loading').html(originalBtnHtml);
            $cancelBtn.prop('disabled', false);
            return false;
        }
        // Note: No finally block - buttons stay disabled during successful reload
    }

    // --- Private Functions ---

    /**
     * Renders the edit panel HTML structure.
     */
    function renderPanel($noteCard, response, commitId) {
        const fields = response.fields;
        
        let fieldsHtml = '';
        
        if (!fields || fields.length === 0) {
            fieldsHtml = '<div class="edit-field-item"><div class="edit-field-header"><span class="edit-field-name">No fields found</span></div></div>';
        } else {
            fields.forEach((field) => {
                
                const hasChange = field.suggestion_content !== null;
                const isEmpty = !field.reviewed_content && !field.suggestion_content;
                
                // Determine the content to show in the editor
                const editorContent = field.suggestion_content !== null 
                    ? field.suggestion_content 
                    : field.reviewed_content;
                
                // Build status badge
                let statusBadge = '';
                if (field.inherited) {
                    statusBadge = '<span class="field-status-badge inherited"><i class="fa fa-link"></i> Inherited (Read-only)</span>';
                } else if (hasChange) {
                    statusBadge = '<span class="field-status-badge has-suggestion"><i class="fa fa-pencil"></i> Has suggestion</span>';
                } else if (field.has_other_suggestions) {
                    statusBadge = '<span class="field-status-badge other-suggestion"><i class="fa fa-info-circle"></i> Other commits have changes</span>';
                } else if (isEmpty) {
                    statusBadge = '<span class="field-status-badge empty"><i class="fa fa-circle-o"></i> Empty</span>';
                } else {
                    statusBadge = '<span class="field-status-badge unchanged"><i class="fa fa-check"></i> Unchanged</span>';
                }

                const readOnlyClass = field.inherited ? 'inherited readonly' : '';

                // For non-inherited fields, render empty initially - content will be set via Trumbowyg after init
                // For inherited fields (read-only), we can safely show escaped content since no editor is initialized
                const initialContent = field.inherited ? escapeHtml(editorContent || '') : '';

                fieldsHtml += `
                    <div class="edit-field-item ${readOnlyClass}" data-position="${field.position}">
                        <div class="edit-field-header">
                            <span class="edit-field-name">${escapeHtml(field.name || 'Field ' + field.position)}</span>
                            ${statusBadge}
                        </div>
                        <div class="edit-field-editor"
                             data-position="${field.position}"
                             data-original-content="${escapeAttr(field.reviewed_content || '')}"
                             data-suggestion-content="${escapeAttr(field.suggestion_content || '')}"
                             data-editor-content="${escapeAttr(editorContent || '')}"
                             data-inherited="${String(field.inherited)}">${initialContent}</div>
                    </div>
                `;
            });
        }

        const panelHtml = `
            <div class="edit-all-fields-panel" data-commit-id="${commitId}">
                <div class="edit-panel-header">
                    <div class="edit-panel-title">
                        <i class="fa fa-edit"></i>
                        <span>Edit All Fields</span>
                    </div>
                    <div class="edit-panel-hint">
                        Changes affect commit #${commitId}
                    </div>
                </div>
                <div class="edit-fields-container">
                    ${fieldsHtml}
                </div>
                <div class="edit-panel-footer">
                    <div class="edit-panel-status"></div>
                    <div class="edit-panel-actions">
                        <button class="modern-btn btn-secondary cancel-all-fields-btn" data-note-id="${response.note_id}">
                            <i class="fa fa-times"></i> Cancel
                        </button>
                        <button class="modern-btn btn-success save-all-fields-btn" data-note-id="${response.note_id}">
                            <i class="fa fa-save"></i> Save Changes
                        </button>
                    </div>
                </div>
            </div>
        `;

        // Insert after the content-comparison div
        const $contentComparison = $noteCard.find('.content-comparison');
        if ($contentComparison.length) {
            $contentComparison.after(panelHtml);
        } else {
            $noteCard.append(panelHtml);
        }
    }

    /**
     * Initializes Trumbowyg editors for all editable fields.
     */
    async function initializeEditors($panel, noteId) {
        // Verify Trumbowyg is available
        if (typeof $.fn.trumbowyg !== 'function') {
            console.error('Trumbowyg is not loaded. Cannot initialize editors.');
            return;
        }

        const $editors = $panel.find('.edit-field-editor:not([data-inherited="true"])');
        
        if ($editors.length === 0) {
            console.warn('No editable fields found in panel');
            return;
        }

        await Promise.all(
            $editors.map(function() {
                const $editor = $(this);
                return setupFieldEditor($editor, noteId).catch(error => {
                    const position = $editor.data('position');
                    console.error(`Failed to initialize editor for position ${position}:`, error);
                    $editor.html('<p style="color:red;">Error initializing editor.</p>').css('border', '2px solid red');
                });
            }).get()
        );
    }

    /**
     * Sets up a single field editor
     * Content is set via .html() BEFORE initializing Trumbowyg.
     */
    async function setupFieldEditor($editor, noteId) {
        // jQuery converts data-editor-content to camelCase: editorContent
        // jQuery might parse data attributes as JSON/number, so ensure it's a string
        let editorContent = $editor.data('editorContent');
        if (editorContent == null) {
            editorContent = $editor.attr('data-editor-content') || '';
        }
        // Ensure it's always a string (jQuery may return numbers, objects, etc.)
        editorContent = String(editorContent);
        
        const unescapedContent = HtmlDiffUtils.unescapeHtml(editorContent);
        
        // Prepare content for editor: strip diff markers and image wrappers
        const preparedContent = prepareContentForEditor(unescapedContent);
        
        $editor.html(preparedContent);
        
        // Get original baseline for paragraph cleanup
        const originalAttr = $editor.data('originalContent') || $editor.attr('data-original-content');
        let baseline = unescapedContent;
        if (originalAttr) {
            try {
                baseline = HtmlDiffUtils.unescapeHtml(String(originalAttr));
            } catch (error) {
                // Fall back to unescaped content
            }
        }
        $editor.data('paragraph-cleanup-baseline', baseline);
        
        // Pre-fetch images before editor init
        await prefetchImagesInElement($editor, noteId);
        
        return attachTrumbowyg($editor);
    }

    /**
     * Prepares HTML content for the editor by stripping diff markers and image wrappers.
     * @param {string} htmlContent - The HTML content to prepare
     * @returns {string} - The prepared HTML
     */
    function prepareContentForEditor(htmlContent) {
        if (typeof htmlContent !== 'string') {
            return '';
        }

        const parser = new DOMParser();
        const doc = parser.parseFromString(htmlContent, 'text/html');
        const body = doc.body;

        // Remove <ins> and <del> diff markers, keeping their content
        body.querySelectorAll('ins, del').forEach(el => {
            el.replaceWith(...el.childNodes);
        });

        // Remove image diff wrappers, keeping the image
        body.querySelectorAll('span.img-diff-wrapper').forEach(wrapper => {
            const img = wrapper.querySelector('img');
            if (img) {
                wrapper.replaceWith(img);
            } else {
                wrapper.remove();
            }
        });

        return body.innerHTML;
    }

    /**
     * Attaches Trumbowyg to a field element.
     */
    function attachTrumbowyg($editor) {
        return new Promise((resolve, reject) => {
            try {
                $editor.trumbowyg(TRUMBOWYG_OPTIONS).on('tbwinit', () => {
                    installEnterKeyHandler($editor);
                    installParagraphCleanup($editor);
                    resolve();
                });
            } catch (error) {
                reject(error);
            }
        });
    }

    /**
     * Pre-fetches images within an element before editor initialization.
     */
    async function prefetchImagesInElement($element, noteId) {
        // Ensure noteId is a string - the API expects context_id as a string
        const context = { type: 'note', id: String(noteId) };
        const fetchPromises = [];
        
        $element.find('img').each(function() {
            const $img = $(this);
            const currentSrc = $img.attr('src') || '';
            
            // Check for existing data-filename first, then use src if it looks like a local filename
            const existingFilename = $img.attr('data-filename') || $img.data('filename');
            const filename = existingFilename || 
                (currentSrc && !currentSrc.startsWith('http') && !currentSrc.startsWith('data:') && currentSrc.includes('.') ? currentSrc : null);
            
            if (filename) {
                // Store filename in data attribute for later restoration
                $img.attr('data-filename', filename);
                $img.attr('src', '/static/images/click_to_show.webp');
                
                fetchPromises.push(
                    ApiService.getPresignedImageUrl(filename, context.type, context.id)
                        .then(result => {
                            if (result && result.presigned_url) {
                                $img.attr('src', result.presigned_url);
                            } else {
                                console.warn(`Failed to pre-fetch image ${filename} for editor: ${result?.error || 'No URL'}`);
                                $img.attr('src', '/static/images/placeholder-error.png');
                                $img.addClass('fetch-error');
                            }
                        })
                        .catch(error => {
                            console.error(`Error pre-fetching image ${filename} for editor:`, error);
                            $img.attr('src', '/static/images/placeholder-error.png');
                            $img.addClass('fetch-error');
                        })
                );
            } else if (currentSrc.startsWith('/static/images/click_to_show')) {
                // Image without filename that's still showing placeholder
                $img.attr('src', '/static/images/placeholder-error.png');
                $img.addClass('fetch-error');
            }
        });
        
        await Promise.all(fetchPromises);
    }

    /**
     * Installs Enter key handler to insert <br> instead of <p> tags.
     * Anki fields typically don't use paragraph tags.
     */
    function installEnterKeyHandler($editor) {
        $editor.off('keydown.fieldEditPanel').on('keydown.fieldEditPanel', function(event) {
            if (event.which !== 13 || event.shiftKey) {
                return;
            }

            event.preventDefault();
            event.stopPropagation();

            const selection = window.getSelection();
            if (!selection || selection.rangeCount === 0) {
                return;
            }

            const range = selection.getRangeAt(0);
            const br = document.createElement('br');
            const zws = document.createTextNode('\u200B'); // Zero-width space for cursor positioning

            range.deleteContents();
            range.insertNode(br);
            range.setStartAfter(br);
            range.insertNode(zws);

            range.setStartAfter(br);
            range.collapse(true);
            selection.removeAllRanges();
            selection.addRange(range);

            $editor.trigger('tbwchange');
        });
    }

    /**
     * Installs paragraph cleanup to strip unwanted <p> and <div> wrappers.
     * Runs on paste and blur events.
     */
    function installParagraphCleanup($editor) {
        $editor.off(PARAGRAPH_CLEANUP_EVENTS);

        let isCleaning = false;
        const cleanup = () => {
            if (isCleaning) {
                return;
            }

            let currentHtml;
            try {
                currentHtml = $editor.trumbowyg('html');
            } catch (error) {
                return;
            }

            const baseline = $editor.data('paragraph-cleanup-baseline') || '';
            const stripped = stripParagraphTags(currentHtml, baseline);
            if (stripped === currentHtml) {
                return;
            }

            const instance = $editor.data('trumbowyg');
            const editorBody = instance && instance.$ed ? instance.$ed : null;
            const editingElement = editorBody && editorBody.length ? editorBody[0] : null;
            const shouldPreserveSelection = editingElement && document.activeElement === editingElement;
            const previousScrollTop = editorBody ? editorBody.scrollTop() : null;

            if (shouldPreserveSelection && instance && typeof instance.saveRange === 'function') {
                try {
                    instance.saveRange();
                } catch (error) {
                    // Ignore
                }
            }

            isCleaning = true;
            try {
                $editor.trumbowyg('html', stripped);
            } finally {
                isCleaning = false;
            }

            if (editorBody && previousScrollTop !== null) {
                editorBody.scrollTop(previousScrollTop);
            }

            if (shouldPreserveSelection && instance && typeof instance.restoreRange === 'function') {
                try {
                    instance.restoreRange();
                } catch (error) {
                    if (editingElement && typeof editingElement.focus === 'function') {
                        editingElement.focus();
                    }
                }
            }
        };

        $editor.on(PARAGRAPH_CLEANUP_EVENTS, cleanup);
        // Run cleanup immediately in case content was already set
        cleanup();
    }

    /**
     * Strips paragraph (<p>) and <div> tags from content if the original didn't have them.
     * Preserves block-level structures like tables, lists, etc.
     */
    function stripParagraphTags(content, originalContent = '') {
        // Ensure both params are strings
        if (content == null || typeof content !== 'string') {
            return '';
        }
        // Safely convert originalContent to string
        const safeOriginal = (originalContent == null) ? '' : String(originalContent);

        const trimmedContent = content.trim();
        const trimmedOriginal = safeOriginal.trim();

        if (!trimmedContent) {
            return '';
        }

        // Don't strip if content has block-level elements (tables, lists, etc.)
        const blockLevelPattern = /<(ul|ol|li|table|thead|tbody|tfoot|tr|td|th|blockquote|pre|code|section|article|header|footer|nav|figure|h[1-6])/i;
        if (blockLevelPattern.test(trimmedContent)) {
            return trimmedContent;
        }

        // If original content had paragraph/div wrappers, keep them
        const originalHadBlockWrappers = /<(p|div)[\s>]/i.test(trimmedOriginal);

        const parser = new DOMParser();
        const doc = parser.parseFromString(`<div>${trimmedContent}</div>`, 'text/html');
        const container = doc.body.firstChild || doc.body;

        const ELEMENT_NODE = typeof Node !== 'undefined' ? Node.ELEMENT_NODE : 1;
        const TEXT_NODE = typeof Node !== 'undefined' ? Node.TEXT_NODE : 3;

        const segments = [];
        let sawParagraphLikeWrapper = false;

        Array.from(container.childNodes).forEach(node => {
            if (node.nodeType === ELEMENT_NODE && (node.tagName === 'P' || node.tagName === 'DIV')) {
                sawParagraphLikeWrapper = true;
                const inner = node.innerHTML.trim();
                if (inner) {
                    segments.push(inner);
                }
            } else {
                const asString = node.nodeType === TEXT_NODE ? node.textContent : node.outerHTML;
                if (asString && asString.trim()) {
                    segments.push(asString.trim());
                }
            }
        });

        // If original had block wrappers or we didn't see any, keep as-is
        if (!sawParagraphLikeWrapper || originalHadBlockWrappers) {
            return trimmedContent;
        }

        if (!segments.length) {
            return '';
        }

        // Join segments with <br> instead of paragraph breaks
        let sanitised = segments.join('<br>');
        sanitised = sanitised
            .replace(/(<br>\s*){2,}/gi, '<br>')
            .replace(/^<br>\s*/i, '')
            .replace(/\s*<br>$/i, '')
            .trim();

        return sanitised;
    }

    /**
     * Collects all modified fields from the panel.
     */
    function collectModifiedFields($panel) {
        const modified = [];
        
        $panel.find('.edit-field-editor:not([data-inherited="true"])').each(function() {
            const $editor = $(this);
            const position = parseInt($editor.data('position'), 10);
            
            // Safely get data attributes as strings (jQuery may parse them)
            let originalContent = $editor.data('originalContent');
            if (originalContent == null) {
                originalContent = $editor.attr('data-original-content') || '';
            }
            originalContent = String(originalContent);
            
            let suggestionContent = $editor.data('suggestionContent');
            if (suggestionContent == null) {
                suggestionContent = $editor.attr('data-suggestion-content') || '';
            }
            suggestionContent = String(suggestionContent);
            
            let cleanupBaseline = $editor.data('paragraph-cleanup-baseline');
            cleanupBaseline = cleanupBaseline == null ? '' : String(cleanupBaseline);
            
            let currentContent;
            try {
                currentContent = $editor.trumbowyg('html') || '';
            } catch (e) {
                console.error('Failed to get trumbowyg html for position ' + position + ':', e);
                currentContent = $editor.html() || '';
            }
            
            currentContent = stripParagraphTags(currentContent, cleanupBaseline);
            
            // Clean HTML for save (restore image filenames, remove placeholders, etc.)
            currentContent = cleanHtmlForSave(currentContent);
            
            // Determine what the "comparison baseline" should be - what's already in this commit's suggestion
            const comparisonBaseline = suggestionContent || originalContent;
            
            // Normalize comparison baseline the same way we normalize current content
            const normalizedBaseline = cleanHtmlForSave(HtmlDiffUtils.unescapeHtml(comparisonBaseline));
            
            // Check if content has changed from the baseline
            if (currentContent !== normalizedBaseline) {
                modified.push({
                    position: position,
                    content: currentContent
                });
            }
        });
        
        return modified;
    }



    /**
     * Cleans HTML before saving - restores image filenames from data attributes,
     * removes placeholder images, strips trailing <br> tags.
     */
    function cleanHtmlForSave(trumbowygHtml) {
        if (typeof trumbowygHtml !== 'string') {
            return '';
        }

        const parser = new DOMParser();
        const doc = parser.parseFromString(trumbowygHtml, 'text/html');
        const body = doc.body;

        body.querySelectorAll('img').forEach(img => {
            const filename = img.dataset.filename;
            const currentSrc = img.getAttribute('src') || '';

            if (filename) {
                // Restore original filename as src
                const alt = img.getAttribute('alt');
                img.setAttribute('src', filename);
                // Remove all attributes except src, alt
                Array.from(img.attributes).forEach(attr => {
                    if (!['src', 'alt', 'data-filename'].includes(attr.name)) {
                        img.removeAttribute(attr.name);
                    }
                });
                img.removeAttribute('data-filename');
                if (alt) {
                    img.setAttribute('alt', alt);
                }
            } else if (currentSrc.includes('placeholder-error.png') || currentSrc.includes('click_to_show.webp')) {
                // Remove placeholder images
                console.warn('Removing placeholder image during save:', currentSrc);
                img.remove();
            } else if (currentSrc.startsWith('data:image/') && currentSrc.length > 10000) {
                // Remove large data URI images
                console.warn('Removing large data URI image during save.');
                img.remove();
            } else if (!currentSrc) {
                // Remove images without src
                img.remove();
            } else {
                // Keep external URLs but clean attributes
                const alt = img.getAttribute('alt');
                Array.from(img.attributes).forEach(attr => {
                    if (!['src', 'alt'].includes(attr.name)) {
                        img.removeAttribute(attr.name);
                    }
                });
                if (alt) {
                    img.setAttribute('alt', alt);
                }
            }
        });

        // Strip trailing <br> tags and trim
        let cleanedHtml = body.innerHTML.replace(/(<br\s*\/?>\s*)+$/, '').trim();
        
        // Also clean zero-width spaces
        cleanedHtml = cleanedHtml.replace(/\u200B/g, '');
        
        return cleanedHtml;
    }

    /**
     * Shows an error message in the note card.
     */
    function showError($noteCard, message) {
        const $error = $(`
            <div class="edit-panel-error alert alert-danger" style="margin: 16px;">
                <i class="fa fa-exclamation-triangle"></i> ${escapeHtml(message)}
            </div>
        `);
        $noteCard.find('.content-comparison').after($error);
        setTimeout(() => $error.fadeOut(300, () => $error.remove()), 5000);
    }

    /**
     * Escapes HTML for safe insertion.
     */
    function escapeHtml(text) {
        if (!text) return '';
        const div = document.createElement('div');
        div.textContent = text;
        return div.innerHTML;
    }

    /**
     * Escapes for use in HTML attributes.
     */
    function escapeAttr(text) {
        if (!text) return '';
        return text
            .replace(/&/g, '&amp;')
            .replace(/"/g, '&quot;')
            .replace(/'/g, '&#39;')
            .replace(/</g, '&lt;')
            .replace(/>/g, '&gt;');
    }

    // --- Event Handlers ---

    // Delegate click events for panel buttons
    $(document).on('click', '.save-all-fields-btn', function(e) {
        e.preventDefault();
        const noteId = $(this).data('note-id');
        save(noteId);
    });

    $(document).on('click', '.cancel-all-fields-btn', function(e) {
        e.preventDefault();
        const noteId = $(this).data('note-id');
        close(noteId);
    });

    // --- Scroll Restoration on Page Load ---
    
    // Note: Main restoration is now handled in commit.html's initStateRestoration()
    // This handles any edge cases where the page-specific handler doesn't exist
    $(document).ready(function() {
        // Check if we're on the commit page (which has its own handler)
        if ($('#notes-container').length && typeof window.commitPageRestoreHandled !== 'undefined') {
            return; // Let commit.html handle it
        }
        
        // Fallback for other pages that might use this panel
        const savedState = sessionStorage.getItem('fieldEditPanel_restoreState');
        if (savedState) {
            try {
                const state = JSON.parse(savedState);
                
                // Validate required properties
                if (!state || typeof state.timestamp !== 'number') {
                    sessionStorage.removeItem('fieldEditPanel_restoreState');
                    return;
                }
                
                // Check if state is not too old (5 minutes)
                const STATE_EXPIRY_MS = 5 * 60 * 1000;
                if (Date.now() - state.timestamp > STATE_EXPIRY_MS) {
                    sessionStorage.removeItem('fieldEditPanel_restoreState');
                    return;
                }
                
                sessionStorage.removeItem('fieldEditPanel_restoreState');
                
                // Simple scroll restoration for non-commit pages
                if (state.noteId) {
                    // Sanitize noteId and use getElementById for safety
                    const noteId = String(state.noteId).replace(/[^\w-]/g, '');
                    const noteElement = document.getElementById(noteId);
                    if (noteElement) {
                        requestAnimationFrame(() => {
                            noteElement.scrollIntoView({ behavior: 'instant', block: 'center' });
                            const $noteCard = $(noteElement);
                            $noteCard.addClass('save-highlight');
                            setTimeout(() => $noteCard.removeClass('save-highlight'), 2000);
                        });
                    }
                }
            } catch (e) {
                sessionStorage.removeItem('fieldEditPanel_restoreState');
            }
        }
    });

    // --- Public Interface ---
    return {
        open: open,
        close: close,
        save: save,
        isActive: () => activePanel !== null,
        getActiveNoteId: () => activePanel ? activePanel.noteId : null
    };
})();
