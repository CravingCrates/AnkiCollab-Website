/**
 * EditorControls for managing Trumbowyg instances.
 * Handles editor initialization, content preparation, and persistence for note suggestions.
 */
window.EditorControls = (() => {
    const EDITOR_SELECTOR = '.diff-content';
    const PARAGRAPH_CLEANUP_EVENTS = 'tbwpaste.paragraphCleanup tbwblur.paragraphCleanup';
    const fieldSnapshots = new Map();

    const TRUMBOWYG_BUTTON_GROUPS = [
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
    ];

    const TRUMBOWYG_OPTIONS = {
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
        btns: TRUMBOWYG_BUTTON_GROUPS,
    };

    async function initializeEditorsForNote(noteId) {
        const $noteCard = $(`#${noteId}`);
        const contextElement = $noteCard[0];

        if (!$noteCard.length || !contextElement) {
            return;
        }

        ensureFieldContentExists($noteCard);
        destroyEditorsForNote(noteId);

        const fields = getFieldElements($noteCard);

        if (!fields.length) {
            console.warn(`No editable fields found for note ${noteId}`);
            return;
        }

        await Promise.all(
            fields.map($field =>
                setupFieldEditor($field, contextElement).catch(error => {
                    const fieldId = $field.data('field-id');
                    console.error(`Failed to initialize editor for field ${fieldId}:`, error);
                    $field.html('<p style="color:red;">Error initializing editor.</p>').css('border', '2px solid red');
                }),
            ),
        );

        $(document).trigger('editors:initialized', [noteId]);
    }

    async function saveEditorsForNote(noteId) {
        const $noteCard = $(`#${noteId}`);
        const fields = getFieldElements($noteCard);

        if (!fields.length) {
            return true;
        }

        const results = await Promise.all(fields.map(saveFieldEditor));
        return results.every(Boolean);
    }

    function destroyEditorsForNote(noteId) {
        const $noteCard = $(`#${noteId}`);

        if (!$noteCard.length) {
            return;
        }

        $noteCard.find('.trumbowyg-editor-box').each(function() {
            const $box = $(this);
            const $field = $box.find(EDITOR_SELECTOR).first();
            if ($field.length) {
                destroyEditorInstance($field);
            } else {
                const $fieldContainer = $box.closest('.field-item.suggestion-box');
                const fieldId = $fieldContainer.find('[data-field-id]').first().data('field-id');
                const snapshot = fieldSnapshots.get(fieldId);

                if (!fieldId || !snapshot) {
                    console.warn('Trumbowyg wrapper without snapshot data; removing box for field', fieldId);
                    $box.remove();
                    return;
                }

                const $newField = $('<div/>', {
                    class: 'field-content note-content-display diff-content',
                    'data-field-id': fieldId || '',
                });

                $newField.html(snapshot.diffHtml || '');
                $newField.attr('data-original', snapshot.original || '');
                $newField.data('original', snapshot.original || '');
                $newField.attr('data-new-content', snapshot.newContent || '');
                $newField.data('new-content', snapshot.newContent || '');

                if ($fieldContainer.length) {
                    $fieldContainer.find('.field-header').after($newField);
                }

                $box.remove();
            }
        });

        getFieldElements($noteCard).forEach($field => {
            if (isEditorActive($field)) {
                try {
                    $field.off('.editorControls');
                    $field.trumbowyg('destroy');
                } catch (error) {
                    console.warn(`Could not destroy Trumbowyg instance for field ${$field.data('field-id')}:`, error);
                } finally {
                    ensureFieldElementRestored($field);
                }
            } else {
                ensureFieldElementRestored($field);
            }
        });

        const strayBoxes = $noteCard.find('.trumbowyg-editor-box');
        if (strayBoxes.length) {
            strayBoxes.remove();
        }

        ensureFieldContentExists($noteCard);
    }

    function ensureFieldElementRestored($field) {
        if (!$field || !$field.length) {
            return;
        }

        const $box = $field.closest('.trumbowyg-editor-box');
        if ($box.length) {
            $box.after($field);
            $box.remove();
        }

        if (!$field.hasClass('diff-content')) {
            $field.addClass('diff-content');
        }

        $field.removeClass('trumbowyg-textarea trumbowyg-textarea-visible');

        const inlineStyle = $field.attr('style');
        if (inlineStyle && inlineStyle.includes('display')) {
            $field.css('display', '');
        }
    }

    function ensureFieldContentExists($noteCard) {
        if (!$noteCard || !$noteCard.length) {
            return;
        }

        $noteCard.find('.field-item.suggestion-box').each(function() {
            const $container = $(this);
            let $field = $container.find(EDITOR_SELECTOR);
            if ($field.length) {
                const strayBoxes = $container.find('.trumbowyg-editor-box');
                if (strayBoxes.length) {
                    strayBoxes.remove();
                }
                return;
            }

            const fieldId = $container.find('[data-field-id]').first().data('field-id');
            const snapshot = fieldSnapshots.get(fieldId);

            if (!fieldId || !snapshot) {
                console.warn('Unable to reconstruct missing field content: snapshot not found.', fieldId);
                return;
            }

            $field = $('<div/>', {
                class: 'field-content note-content-display diff-content',
                'data-field-id': fieldId || '',
            });

            if (snapshot.diffHtml !== undefined) {
                $field.html(snapshot.diffHtml);
            }
            if (snapshot.original !== undefined) {
                $field.attr('data-original', snapshot.original);
                $field.data('original', snapshot.original);
            }
            if (snapshot.newContent !== undefined) {
                $field.attr('data-new-content', snapshot.newContent);
                $field.data('new-content', snapshot.newContent);
            }

            $container.find('.field-header').after($field);
        });
    }

    function getFieldElements($noteCard) {
        const collected = new Map();

        const collectIfEligible = element => {
            if (!element) {
                return;
            }

            const $element = $(element);
            const withinSuggestionSide = $element.closest('.suggestion-side').length > 0;
            if (!withinSuggestionSide) {
                return;
            }

            const hasSuggestionData = $element.data('new-content') !== undefined || $element.attr('data-new-content') !== undefined;
            const fieldId = $element.data('field-id');
            if (!hasSuggestionData || !fieldId) {
                return;
            }

            if (!collected.has(fieldId)) {
                collected.set(fieldId, $element);
            }
        };

        $noteCard.find(`.suggestion-side ${EDITOR_SELECTOR}`).each(function() {
            collectIfEligible(this);
        });

        if (collected.size === 0) {
            $noteCard.find('.suggestion-side [data-field-id]').each(function() {
                const $candidate = $(this);
                collectIfEligible($candidate[0]);
                if (collected.has($candidate[0]) && !$candidate.hasClass('diff-content')) {
                    $candidate.addClass('diff-content');
                }
            });
        }

        return Array.from(collected.values());
    }

    async function setupFieldEditor($field, contextElement) {
        const fieldId = $field.data('field-id');
        const preEditHtml = $field.html();
        $field.data('pre-edit-html', preEditHtml);

        const suggestedHtml = HtmlDiffUtils.unescapeHtml($field.data('new-content') || '');
        const preparedHtml = await prepareContentForEditor(suggestedHtml, contextElement);
        $field.html(preparedHtml);

        const baseline = getOriginalBaseline($field, suggestedHtml);
        $field.data('paragraph-cleanup-baseline', baseline);

        if (fieldId) {
            const snapshot = {
                diffHtml: $field.html(),
                original: $field.data('original') || $field.attr('data-original') || '',
                newContent: $field.data('new-content') || $field.attr('data-new-content') || '',
            };
            fieldSnapshots.set(fieldId, snapshot);
        }

        return attachTrumbowyg($field);
    }

    function getOriginalBaseline($field, fallbackHtml) {
        const originalAttr = $field.data('original') || $field.attr('data-original');
        if (!originalAttr) {
            return fallbackHtml;
        }

        try {
            return HtmlDiffUtils.unescapeHtml(originalAttr);
        } catch (error) {
            console.warn('Failed to unescape original content:', error);
            return fallbackHtml;
        }
    }

    function attachTrumbowyg($field) {
        return new Promise((resolve, reject) => {
            try {
                $field.trumbowyg(TRUMBOWYG_OPTIONS).on('tbwinit', () => {
                    installEnterKeyHandler($field);
                    installParagraphCleanup($field);
                    resolve();
                });
            } catch (error) {
                reject(error);
            }
        });
    }

    function installEnterKeyHandler($field) {
        $field.off('keydown.editorControls').on('keydown.editorControls', event => {
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
            const zws = document.createTextNode('\u200B');

            range.deleteContents();
            range.insertNode(br);
            range.setStartAfter(br);
            range.insertNode(zws);

            range.setStartAfter(br);
            range.collapse(true);
            selection.removeAllRanges();
            selection.addRange(range);

            $(event.currentTarget).trigger('tbwchange');
        });
    }

    function installParagraphCleanup($field) {
        $field.off(PARAGRAPH_CLEANUP_EVENTS);

        let isCleaning = false;
        const cleanup = () => {
            if (isCleaning) {
                return;
            }

            let currentHtml;
            try {
                currentHtml = $field.trumbowyg('html');
            } catch (error) {
                console.warn('Failed to fetch editor HTML during cleanup:', error);
                return;
            }

            const baseline = $field.data('paragraph-cleanup-baseline') || '';
            const stripped = stripParagraphTags(currentHtml, baseline);
            if (stripped === currentHtml) {
                return;
            }

            const instance = $field.data('trumbowyg');
            const editorBody = instance && instance.$ed ? instance.$ed : null;
            const editingElement = editorBody && editorBody.length ? editorBody[0] : null;
            const shouldPreserveSelection = editingElement && document.activeElement === editingElement;
            const previousScrollTop = editorBody ? editorBody.scrollTop() : null;

            if (shouldPreserveSelection && instance && typeof instance.saveRange === 'function') {
                try {
                    instance.saveRange();
                } catch (error) {
                    console.warn('Failed to save selection range before cleanup:', error);
                }
            }

            isCleaning = true;
            try {
                $field.trumbowyg('html', stripped);
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
                    console.warn('Failed to restore selection range after cleanup:', error);
                    if (editingElement && typeof editingElement.focus === 'function') {
                        editingElement.focus();
                    }
                }
            }
        };

        $field.on(PARAGRAPH_CLEANUP_EVENTS, cleanup);
        cleanup();
    }

    function stripParagraphTags(content, originalContent = '') {
        if (typeof content !== 'string') {
            return '';
        }

        const trimmedContent = content.trim();
        const trimmedOriginal = (originalContent || '').trim();

        if (!trimmedContent) {
            return '';
        }

        const blockLevelPattern = /<(ul|ol|li|table|thead|tbody|tfoot|tr|td|th|blockquote|pre|code|section|article|header|footer|nav|figure|h[1-6])/i;
        if (blockLevelPattern.test(trimmedContent)) {
            return trimmedContent;
        }

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

        if (!sawParagraphLikeWrapper || originalHadBlockWrappers) {
            return trimmedContent;
        }

        if (!segments.length) {
            return '';
        }

        let sanitised = segments.join('<br>');
        sanitised = sanitised
            .replace(/(<br>\s*){2,}/gi, '<br>')
            .replace(/^<br>\s*/i, '')
            .replace(/\s*<br>$/i, '')
            .trim();

        return sanitised;
    }

    async function prepareContentForEditor(htmlContent, contextElement) {
        if (typeof htmlContent !== 'string') {
            return '';
        }

        const context = ApiService.getContext(contextElement) || {};
        if (!context.type || !context.id) {
            console.error('Cannot prepare content for editing: Missing context.');
            return `<!-- Error: Missing context --> ${htmlContent}`;
        }

        const parser = new DOMParser();
        const doc = parser.parseFromString(htmlContent, 'text/html');
        const body = doc.body;

        body.querySelectorAll('ins, del').forEach(el => {
            el.replaceWith(...el.childNodes);
        });

        body.querySelectorAll('span.img-diff-wrapper').forEach(wrapper => {
            const img = wrapper.querySelector('img');
            if (img) {
                wrapper.replaceWith(img);
            } else {
                wrapper.remove();
            }
        });

        const fetchPromises = [];
        body.querySelectorAll('img').forEach(img => {
            const currentSrc = img.getAttribute('src') || '';
            const filename = img.dataset.filename || (currentSrc && !currentSrc.startsWith('http') && !currentSrc.startsWith('data:') && currentSrc.includes('.') ? currentSrc : null);

            if (filename) {
                img.dataset.filename = filename;
                img.src = '/static/images/click_to_show.webp';

                fetchPromises.push(
                    ApiService.getPresignedImageUrl(filename, context.type, context.id)
                        .then(result => {
                            if (result && result.presigned_url) {
                                img.src = result.presigned_url;
                            } else {
                                console.warn(`Failed to pre-fetch image ${filename} for editor: ${result?.error || 'No URL'}`);
                                img.src = '/static/images/placeholder-error.png';
                                img.classList.add('fetch-error');
                            }
                        })
                        .catch(error => {
                            console.error(`Error pre-fetching image ${filename} for editor:`, error);
                            img.src = '/static/images/placeholder-error.png';
                            img.classList.add('fetch-error');
                        }),
                );
            } else if (currentSrc.startsWith('/static/images/click_to_show')) {
                img.src = '/static/images/placeholder-error.png';
                img.classList.add('fetch-error');
            }
        });

        await Promise.all(fetchPromises);
        return body.innerHTML;
    }

    async function saveFieldEditor($field) {
        const fieldId = $field.data('field-id');

        if (!isEditorActive($field)) {
            restorePreEditHtml($field);
            return true;
        }

        const preEditHtml = $field.data('pre-edit-html');
        let editorContent;

        try {
            editorContent = $field.trumbowyg('html');
        } catch (error) {
            console.error(`Error getting content for field ${fieldId}:`, error);
            renderFieldError($field, preEditHtml, 'Error getting content.');
            destroyEditorInstance($field);
            return false;
        }

        const baseline = $field.data('paragraph-cleanup-baseline') || '';
        const normalisedContent = stripParagraphTags(editorContent, baseline);
        const cleanedContent = cleanHtmlForSave(normalisedContent);

        destroyEditorInstance($field);

        if (!fieldId) {
            console.error('Cannot save field: Missing field-id.', $field);
            renderFieldError($field, preEditHtml, 'Error: Missing Field ID.');
            return false;
        }

        try {
            const result = await ApiService.updateFieldSuggestion(fieldId, cleanedContent);
            const newDiffHtml = typeof result === 'string' ? result : result?.diff_html;
            if (typeof newDiffHtml !== 'string') {
                throw new Error(`Invalid diff response for field ${fieldId}`);
            }

            $field.html(newDiffHtml);
            $field.data('new-content', cleanedContent);
            $field.attr('data-new-content', cleanedContent);
            $field.addClass('needs-reprocess');
            $field.css('border', '');
            $field.removeData('pre-edit-html');

            fieldSnapshots.set(fieldId, {
                diffHtml: newDiffHtml,
                original: $field.data('original') || $field.attr('data-original') || '',
                newContent: cleanedContent,
            });
            return true;
        } catch (error) {
            console.error(`Error updating field ${fieldId}:`, error);
            renderFieldError($field, preEditHtml, 'Error updating field. Cannot restore previous view.');
            return false;
        }
    }

    function isEditorActive($field) {
        return Boolean($field.data('trumbowyg') || $field.parent().hasClass('trumbowyg-editor'));
    }

    function restorePreEditHtml($field) {
        const preEditHtml = $field.data('pre-edit-html');
        if (preEditHtml !== undefined) {
            $field.html(preEditHtml);
        }
        $field.removeData('pre-edit-html');
    }

    function destroyEditorInstance($field) {
        try {
            $field.off('.editorControls');
            $field.off(PARAGRAPH_CLEANUP_EVENTS);
            $field.trumbowyg('destroy');
        } catch (error) {
            console.warn(`Error destroying Trumbowyg instance for field ${$field.data('field-id')}:`, error);
        } finally {
            ensureFieldElementRestored($field);
        }
    }

    function renderFieldError($field, preEditHtml, message) {
        if (preEditHtml !== undefined) {
            $field.html(preEditHtml);
        } else {
            $field.html(`<p style="color: red;">${message}</p>`);
        }
        $field.css('border', '2px solid red');
        $field.removeData('pre-edit-html');
    }

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
                const alt = img.getAttribute('alt');
                img.setAttribute('src', filename);
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
                console.warn('Removing placeholder image during save:', currentSrc);
                img.remove();
            } else if (currentSrc.startsWith('data:image/') && currentSrc.length > 10000) {
                console.warn('Removing large data URI image during save.');
                img.remove();
            } else if (!currentSrc) {
                img.remove();
            } else {
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

            let cleanedHtml = body.innerHTML.replace(/(<br\s*\/?>\s*)+$/, '').trim();
        return cleanedHtml;
    }

    return {
        initializeEditorsForNote,
        saveEditorsForNote,
        destroyEditorsForNote,
    };
})();