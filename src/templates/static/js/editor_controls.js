/**
 * Editor functionality using Summernote.
 * Manages initialization, content preparation, and saving for note fields.
 */
window.EditorControls = (function() {

    const EDITOR_SELECTOR = '.diff-content'; // Target diff divs for editing

    /**
     * Initializes Summernote editors for all editable fields within a specific note card.
     * @param {string} noteId - The ID of the note card.
     * @returns {Promise<void>} - Resolves when all editors are initialized.
     */
    async function initializeEditorsForNote(noteId) {
        const $noteCard = $(`#${noteId}`);
        const $diffFields = $noteCard.find(`.suggestion-side ${EDITOR_SELECTOR}`);
        const contextElement = $noteCard[0]; // The note card element itself for context

        if (!contextElement || !$diffFields.length) {
            console.warn(`No editable fields found or missing context for note ${noteId}`);
            return Promise.resolve(); // Nothing to do
        }

        console.log(`Initializing ${$diffFields.length} editors for note ${noteId}...`);
        const initPromises = [];

        $diffFields.each(function() {
            const fieldDiv = this;
            const $fieldDiv = $(this);
            const fieldId = $fieldDiv.data('field-id');

            // Store pre-edit HTML for cancellation
            $fieldDiv.data('pre-edit-html', $fieldDiv.html());

            // Get the *current* suggested content (unescaped)
            const currentSuggestedHtml = HtmlDiffUtils.unescapeHtml($fieldDiv.data('new-content') || '');

            // Add the promise for initializing this single editor
            initPromises.push(
                _initializeSingleEditor(fieldDiv, currentSuggestedHtml, contextElement)
                    .catch(error => {
                        console.error(`Failed to initialize editor for field ${fieldId}:`, error);
                        $fieldDiv.html('<p style="color:red;">Error initializing editor.</p>').css('border', '2px solid red');
                        // Allow Promise.all to continue, but log the error
                    })
            );
        });

        // Wait for all editors to finish initializing (or fail individually)
        await Promise.all(initPromises);
        console.log(`Finished initializing editors for note ${noteId}`);
    }

    /**
     * Initializes a single Summernote editor instance.
     * @param {HTMLElement} element - The DOM element (div) to attach Summernote to.
     * @param {string} initialContentHtml - The starting HTML content (unescaped).
     * @param {HTMLElement} contextElement - The parent element providing context (type/id).
     * @returns {Promise<void>}
     */
    async function _initializeSingleEditor(element, initialContentHtml, contextElement) {
        const $editor = $(element);

        // 1. Prepare content: Remove diff markup, pre-fetch image URLs
        const preparedContent = await _prepareContentForEditing(initialContentHtml, contextElement);
        $editor.html(preparedContent); // Set the prepared content

        // 2. Initialize Summernote
        return new Promise((resolve, reject) => {
            try {
                $editor.summernote({
                    // height: 300, // Optional: Set height
                    focus: false, // Don't autofocus first editor usually
                    toolbar: [
                        ['style', ['style', 'bold', 'italic', 'underline', 'clear']],
                        ['font', ['strikethrough', 'superscript', 'subscript']],
                        ['fontsize', ['fontsize']],
                        ['color', ['color']],
                        ['para', ['ul', 'ol', 'paragraph']],
                        ['table', ['table']],
                        ['insert', ['link', /*'picture',*/ 'hr']],
                        ['view', ['fullscreen', 'codeview', 'undo', 'redo', 'help']],
                    ],
                    callbacks: {
                        onInit: function() {
                            // console.log(`Summernote initialized for field ${$editor.data('field-id')}`);
                            resolve(); // Resolve promise when initialized
                        },
                        onImageUpload: function(files) {
                            alert('Direct image upload is not supported. Please add images via Anki and suggest the change.');
                        },
                        // Add other callbacks if needed
                    },
                    enterHtml: '<br>',
                });
                 // Optional: Add small delay before focus if needed, but usually not required on manual edit start
                 // setTimeout(() => $editor.summernote('focus'), 50);
            } catch (error) {
                 console.error("Summernote initialization failed:", error);
                 reject(error);
            }
        });
    }

    /**
     * Prepares HTML content for the editor: removes diff markup, pre-fetches presigned URLs.
     * @param {string} htmlContent - Unescaped HTML content (e.g., the 'new-content' data).
     * @param {HTMLElement} contextElement - The element providing context (type/id).
     * @returns {Promise<string>} - Resolves with HTML ready for the editor.
     */
    async function _prepareContentForEditing(htmlContent, contextElement) {
        if (typeof htmlContent !== 'string') return '';

        const context = ApiService.getContext(contextElement);
        if (!context.type || !context.id) {
            console.error("Cannot prepare content for editing: Missing context.");
            // Return original content with an error comment?
             return `<!-- Error: Missing context --> ${htmlContent}`;
        }

        // Use DOMParser for safer and more robust HTML manipulation
        const parser = new DOMParser();
        const doc = parser.parseFromString(htmlContent, 'text/html');
        const body = doc.body; // Work within the body element

        // 1. Remove diff markers (<ins>, <del>) - Keep their content
        body.querySelectorAll('ins, del').forEach(el => {
            // Replace the element with its child nodes
            el.replaceWith(...el.childNodes);
        });

        // 2. Remove image diff wrappers, keeping the image
        body.querySelectorAll('span.img-diff-wrapper').forEach(wrapper => {
            const img = wrapper.querySelector('img');
            if (img) {
                wrapper.replaceWith(img); // Replace wrapper with the image it contains
            } else {
                wrapper.remove(); // Remove wrapper if it somehow doesn't contain an image
            }
        });

        // 3. Find images and fetch presigned URLs for those needing them
        const images = Array.from(body.querySelectorAll('img'));
        const fetchPromises = [];

        images.forEach(img => {
            const currentSrc = img.getAttribute('src');
            const filename = img.dataset.filename || ( (currentSrc && !currentSrc.startsWith('http') && !currentSrc.startsWith('data:') && currentSrc.includes('.')) ? currentSrc : null );

            // Only fetch if we identified a local filename (either from data-filename or src)
            if (filename) {
                 // Ensure data-filename is set for later saving
                 img.dataset.filename = filename;

                 // Set placeholder initially while fetching
                 img.src = '/static/images/click_to_show.webp'; // Loading placeholder

                 fetchPromises.push(
                     ApiService.getPresignedImageUrl(filename, context.type, context.id)
                         .then(result => {
                             if (result && result.presigned_url) {
                                 img.src = result.presigned_url;
                                 // Keep data-filename!
                             } else {
                                 console.warn(`Failed to pre-fetch image ${filename} for editor: ${result?.error || 'No URL'}`);
                                 img.src = '/static/images/placeholder-error.png'; // Error placeholder
                                 img.classList.add('fetch-error');
                             }
                         })
                         .catch(error => {
                             console.error(`Error pre-fetching image ${filename} for editor:`, error);
                             img.src = '/static/images/placeholder-error.png'; // Error placeholder
                             img.classList.add('fetch-error');
                         })
                 );
            } else if (currentSrc && currentSrc.startsWith('/static/images/click_to_show')) {
                 // If it was a lazy load placeholder that wasn't clicked, show error state
                 img.src = '/static/images/placeholder-error.png';
                 img.classList.add('fetch-error');
                 // Try to recover filename if it exists in data attribute
                 const originalFilename = img.dataset.filename;
                 if(originalFilename) img.dataset.filename = originalFilename; // Ensure filename persists if possible

            } else {
                // Assume it's an external image or data URI - leave src as is, ensure no data-filename
                // img.removeAttribute('data-filename'); // Clean up just in case
            }
        });

        // Wait for all necessary image fetches to complete
        await Promise.all(fetchPromises);

        // 4. Return the processed HTML content from the body
        return body.innerHTML;
    }


    /**
     * Saves changes from all Summernote editors within a note card.
     * @param {string} noteId - The ID of the note card.
     * @returns {Promise<boolean>} - Resolves with true if all fields saved successfully, false otherwise.
     */
    async function saveEditorsForNote(noteId) {
        const $noteCard = $(`#${noteId}`);
        const $diffFields = $noteCard.find(`.suggestion-side ${EDITOR_SELECTOR}`);
        let allUpdatesSuccessful = true;
        const updatePromises = [];

        console.log(`Saving changes for ${$diffFields.length} fields in note ${noteId}...`);

        $diffFields.each(function() {
            const $fieldDiv = $(this);
            const fieldId = $fieldDiv.data('field-id');

            // Check if Summernote was initialized on this element
            if (!$fieldDiv.data('summernote')) {
                 console.warn(`Field ${fieldId} skipped: Not in edit mode or editor not initialized.`);
                 // Restore pre-edit state if available? Or just skip.
                 const preEditHtml = $fieldDiv.data('pre-edit-html');
                 if (preEditHtml !== undefined) {
                    $fieldDiv.html(preEditHtml).removeData('pre-edit-html');
                 }
                 return; // Skip this field
            }

            let content = '';
            try {
                content = $fieldDiv.summernote('code');
            } catch (e) {
                console.error(`Error getting code for field ${fieldId}:`, e);
                allUpdatesSuccessful = false;
                $fieldDiv.html('<p style="color: red;">Error getting content.</p>').css('border', '2px solid red');
                // Attempt to destroy anyway
                try { $fieldDiv.summernote('destroy'); } catch (err) {}
                $fieldDiv.removeData('pre-edit-html');
                return; // Skip update for this field
            }

            // Clean the HTML (replace presigned URLs with filenames, etc.)
            const cleanedContent = cleanHtmlForSave(content);

            try {
                $fieldDiv.summernote('destroy');
            } catch (e) { console.warn(`Error destroying summernote post-save for field ${fieldId}:`, e); }

            // --- Make API Call to Update Field ---
            if (fieldId) {
                console.log(`Updating field ${fieldId}...`); // Content logged below if successful
                updatePromises.push(
                    ApiService.updateFieldSuggestion(fieldId, cleanedContent)
                        .then(result => { // Backend returns new diff HTML string or { diff_html: "..." }
                            let newDiffHtml = '';
                            if (typeof result === 'string') {
                                newDiffHtml = result;
                            } else if (result && typeof result.diff_html === 'string') {
                                newDiffHtml = result.diff_html;
                            } else {
                                 throw new Error(`Invalid diff response for field ${fieldId}`);
                            }

                            console.log(`Field ${fieldId} updated successfully.`);
                            // Update the field div with the new diff HTML from the server
                            $fieldDiv.html(newDiffHtml);
                            // Update data attributes
                            $fieldDiv.data('new-content', cleanedContent); // Store the cleaned content we sent
                            // Original content is fetched from data-original when needed for diff processing
                            // Mark for re-processing (image wrappers, lazy load) by SharedUI
                            $fieldDiv.addClass('needs-reprocess');
                            $fieldDiv.css('border', ''); // Clear any error border
                            $fieldDiv.removeData('pre-edit-html');
                            return true; // Indicate success for this field
                        })
                        .catch(error => {
                            console.error(`Error updating field ${fieldId}:`, error);
                            allUpdatesSuccessful = false;
                            // Display error, keep pre-edit HTML if possible
                            const preEditHtml = $fieldDiv.data('pre-edit-html');
                            if (preEditHtml) {
                                $fieldDiv.html(preEditHtml); // Restore previous view
                            } else {
                                $fieldDiv.html('<p style="color: red;">Error updating field. Cannot restore previous view.</p>');
                            }
                            $fieldDiv.css('border', '2px solid red');
                            $fieldDiv.removeData('pre-edit-html');
                            return false; // Indicate failure for this field
                        })
                );
            } else {
                console.error("Cannot save field: Missing field-id.", $fieldDiv);
                $fieldDiv.html('<p style="color: red;">Error: Missing Field ID.</p>').css('border', '2px solid red');
                allUpdatesSuccessful = false;
                $fieldDiv.removeData('pre-edit-html');
            }
        }); // End $diffFields.each

        // Wait for all individual field updates to complete
        await Promise.all(updatePromises);

        console.log(`Finished saving fields for note ${noteId}. Overall success: ${allUpdatesSuccessful}`);
        return allUpdatesSuccessful; // Return overall success status
    }

    /**
     * Cleans HTML content from Summernote before saving.
     * Replaces temporary image URLs (presigned) with permanent filenames.
     * @param {string} summernoteHtml - HTML content from summernote('code').
     * @returns {string} - Cleaned HTML ready for database.
     */
    function cleanHtmlForSave(summernoteHtml) {
        if (typeof summernoteHtml !== 'string') return '';

        const parser = new DOMParser();
        const doc = parser.parseFromString(summernoteHtml, 'text/html');
        const body = doc.body;

        // Process images: Replace relevant src attributes with filenames stored in data-filename
        body.querySelectorAll('img').forEach(img => {
            const filename = img.dataset.filename; // Check for our preserved filename
            const currentSrc = img.getAttribute('src');

            if (filename) {
                // If we have a filename stored, ALWAYS use it as the src for saving
                img.setAttribute('src', filename);
                // Remove potentially temporary attributes but keep alt
                const alt = img.getAttribute('alt');
                // Clear all attributes except src and alt (more targeted)
                Array.from(img.attributes).forEach(attr => {
                    if (attr.name !== 'src' && attr.name !== 'alt' && attr.name !== 'data-filename') {
                        img.removeAttribute(attr.name);
                    }
                });
                 img.removeAttribute('data-filename'); // Remove after use
                 if (alt) img.setAttribute('alt', alt); // Restore alt if it existed

            } else if (currentSrc) {
                // No data-filename. This is likely an external image or wasn't handled correctly.
                // Basic check: avoid saving potentially broken placeholders or large data URIs
                 if (currentSrc.includes('placeholder-error.png') || currentSrc.includes('click_to_show.webp')) {
                     console.warn("Removing placeholder image during save:", currentSrc);
                     img.remove(); // Remove the placeholder image entirely
                 } else if (currentSrc.startsWith('data:image/') && currentSrc.length > 10000) { // Limit data URI size
                     console.warn("Removing large data URI image during save.");
                     img.remove();
                 } else {
                     // Keep legitimate external URLs or small data URIs
                     // Clean potentially harmful attributes anyway
                     const alt = img.getAttribute('alt');
                     Array.from(img.attributes).forEach(attr => {
                         if (attr.name !== 'src' && attr.name !== 'alt') {
                             img.removeAttribute(attr.name);
                         }
                     });
                     if (alt) img.setAttribute('alt', alt);
                 }
            } else {
                 // Image has no src and no filename? Remove it.
                 img.remove();
            }
        });

        // Basic cleaning: remove trailing <br> tags which Summernote sometimes adds
        let cleanedHtml = body.innerHTML.replace(/(<br\s*\/?>\s*)+$/, '').trim();

        // Remove leading/trailing whitespace
        cleanedHtml = cleanedHtml.trim();

        return cleanedHtml;
    }


    /**
     * Destroys all Summernote instances within a note card.
     * @param {string} noteId - The ID of the note card.
     */
    function destroyEditorsForNote(noteId) {
        const $noteCard = $(`#${noteId}`);
        $noteCard.find(`.suggestion-side ${EDITOR_SELECTOR}`).each(function() {
            const $fieldDiv = $(this);
            // Check if summernote instance exists before destroying
            if ($fieldDiv.data('summernote')) {
                try {
                    $fieldDiv.summernote('destroy');
                } catch (e) {
                    console.warn(`Could not destroy summernote instance for field ${$fieldDiv.data('field-id')}:`, e);
                }
            }
        });
        console.log(`Destroyed editors for note ${noteId}`);
    }


    // --- Public API ---
    return {
        initializeEditorsForNote,
        saveEditorsForNote,
        destroyEditorsForNote,
    };
})();