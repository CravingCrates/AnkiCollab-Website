/**
 * commit_page.js - Commit review page functionality
 * Reads commit_id from data-commit-id attribute on #notes-container element
 */

// Initialize the shared UI components after the page loads
$(document).ready(function() {
    SharedUI.initializePage();
    initButtonFeatures();
    initPaginationControls();
    initBulkSelectionControls();

    var isReducedMotion = function() {
        return window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    };

    function openModalDialog(options) {
        return new Promise(function(resolve) {
            var settled = false;
            var isClosing = false;
            var closeTimer = null;
            var dialog = document.createElement('dialog');
            dialog.className = 'ac-modal-card';

            var headingId = 'ac-modal-heading-' + Date.now().toString(36) + '-' + Math.random().toString(36).slice(2);
            dialog.setAttribute('aria-labelledby', headingId);

            var heading = document.createElement('h3');
            heading.className = 'ac-modal-title';
            heading.id = headingId;
            heading.textContent = options.title;

            var body = document.createElement('p');
            body.className = 'ac-modal-body';
            body.textContent = options.message;

            dialog.appendChild(heading);
            dialog.appendChild(body);

            function finish(value) {
                if (settled) return;
                settled = true;
                if (closeTimer) {
                    clearTimeout(closeTimer);
                    closeTimer = null;
                }
                dialog.returnValue = '';
                if (dialog.parentNode) {
                    dialog.parentNode.removeChild(dialog);
                }
                resolve(value);
            }

            function closeWithValue(value, returnCode) {
                if (settled || isClosing) return;
                isClosing = true;
                dialog._pendingResult = value;
                dialog.classList.remove('is-visible');
                var delay = isReducedMotion() ? 0 : 200;
                closeTimer = setTimeout(function() {
                    closeTimer = null;
                    if (dialog.open) {
                        dialog.close(returnCode || '');
                    } else {
                        finish(value);
                    }
                }, delay);
            }

            if (typeof options.buildContent === 'function') {
                options.buildContent(dialog, closeWithValue);
            }

            function isOutsideDialog(event) {
                var rect = dialog.getBoundingClientRect();
                return (
                    event.clientX < rect.left ||
                    event.clientX > rect.right ||
                    event.clientY < rect.top ||
                    event.clientY > rect.bottom
                );
            }

            var pressedOutside = false;
            dialog.addEventListener('pointerdown', function(event) {
                pressedOutside = isOutsideDialog(event);
            });
            dialog.addEventListener('pointerup', function(event) {
                if (pressedOutside && isOutsideDialog(event)) {
                    closeWithValue(options.dismissValue);
                }
                pressedOutside = false;
            });

            dialog.addEventListener('cancel', function(event) {
                // Pressing Escape during a closing fade-out should always cancel rather than confirm.
                event.preventDefault();
                if (closeTimer) {
                    clearTimeout(closeTimer);
                    closeTimer = null;
                }
                isClosing = false;
                if (dialog.open) {
                    dialog.close('');
                } else {
                    finish(options.dismissValue);
                }
            });

            dialog.addEventListener('close', function() {
                var result = isClosing ? dialog._pendingResult : options.dismissValue;
                finish(result);
            });

            document.body.appendChild(dialog);
            dialog.showModal();
            requestAnimationFrame(function() {
                dialog.classList.add('is-visible');
            });
        });
    }

    function showSimpleConfirmModal(title, message) {
        return openModalDialog({
            title: title,
            message: message,
            dismissValue: false,
            buildContent: function(dialog, closeWithValue) {
                var footer = document.createElement('div');
                footer.className = 'ac-modal-footer';

                var footerLeft = document.createElement('div');
                footerLeft.className = 'ac-modal-footer-left';

                var footerRight = document.createElement('div');
                footerRight.className = 'ac-modal-footer-right';

                var cancelBtn = document.createElement('button');
                cancelBtn.className = 'ac-modal-btn ac-modal-btn-secondary';
                cancelBtn.type = 'button';
                cancelBtn.textContent = 'Cancel';
                // Focus the safe action by default on confirmation dialogs
                cancelBtn.autofocus = true;

                var confirmBtn = document.createElement('button');
                confirmBtn.className = 'ac-modal-btn ac-modal-btn-primary';
                confirmBtn.type = 'button';
                confirmBtn.textContent = 'Confirm';

                cancelBtn.addEventListener('click', function() {
                    closeWithValue(false, 'cancel');
                });
                confirmBtn.addEventListener('click', function() {
                    closeWithValue(true, 'confirm');
                });

                footerLeft.appendChild(cancelBtn);
                footerRight.appendChild(confirmBtn);
                footer.appendChild(footerLeft);
                footer.appendChild(footerRight);
                dialog.appendChild(footer);
            }
        });
    }

    function showInfoModal(title, message) {
        return openModalDialog({
            title: title,
            message: message,
            dismissValue: undefined,
            buildContent: function(dialog, closeWithValue) {
                var footer = document.createElement('div');
                footer.className = 'ac-modal-footer';

                var footerRight = document.createElement('div');
                footerRight.className = 'ac-modal-footer-right';
                footerRight.style.marginLeft = 'auto';

                var okBtn = document.createElement('button');
                okBtn.className = 'ac-modal-btn ac-modal-btn-primary';
                okBtn.type = 'button';
                okBtn.textContent = 'OK';
                okBtn.autofocus = true;

                okBtn.addEventListener('click', function() {
                    closeWithValue(undefined, 'ok');
                });

                footerRight.appendChild(okBtn);
                footer.appendChild(footerRight);
                dialog.appendChild(footer);
            }
        });
    }

    function showDenySuggestionModal() {
        return openModalDialog({
            title: 'Deny Suggestion',
            message: 'You can deny silently or send optional feedback to the user.',
            dismissValue: { cancelled: true },
            buildContent: function(dialog, closeWithValue) {
                var textarea = document.createElement('textarea');
                textarea.className = 'ac-modal-textarea';
                textarea.placeholder = 'Optional feedback for the user\u2026';
                textarea.maxLength = 2000;
                textarea.autofocus = true;
                textarea.setAttribute('aria-label', 'Feedback message');

                var charCount = document.createElement('div');
                charCount.className = 'ac-modal-charcount';
                charCount.textContent = '0 / 2000';

                var footer = document.createElement('div');
                footer.className = 'ac-modal-footer';

                var leftActions = document.createElement('div');
                leftActions.className = 'ac-modal-footer-left';

                var rightActions = document.createElement('div');
                rightActions.className = 'ac-modal-footer-right';

                var cancelBtn = document.createElement('button');
                cancelBtn.className = 'ac-modal-btn ac-modal-btn-plain';
                cancelBtn.type = 'button';
                cancelBtn.textContent = 'Cancel';

                var silentBtn = document.createElement('button');
                silentBtn.className = 'ac-modal-btn ac-modal-btn-secondary';
                silentBtn.type = 'button';
                silentBtn.textContent = 'Deny Silently';

                var notifyBtn = document.createElement('button');
                notifyBtn.className = 'ac-modal-btn ac-modal-btn-danger';
                notifyBtn.type = 'button';
                notifyBtn.textContent = 'Deny & Notify';

                cancelBtn.addEventListener('click', function() {
                    closeWithValue({ cancelled: true }, 'cancel');
                });

                silentBtn.addEventListener('click', function() {
                    closeWithValue({
                        cancelled: false,
                        silent: true,
                        reason: textarea.value || ''
                    }, 'silent');
                });

                notifyBtn.addEventListener('click', function() {
                    closeWithValue({
                        cancelled: false,
                        silent: false,
                        reason: textarea.value || ''
                    }, 'notify');
                });

                textarea.addEventListener('input', function() {
                    var len = textarea.value.length;
                    charCount.textContent = String(len) + ' / 2000';
                    charCount.classList.toggle('near-limit', len >= 1600 && len < 1900);
                    charCount.classList.toggle('at-limit', len >= 1900);
                });

                leftActions.appendChild(cancelBtn);
                rightActions.appendChild(silentBtn);
                rightActions.appendChild(notifyBtn);
                footer.appendChild(leftActions);
                footer.appendChild(rightActions);

                dialog.appendChild(textarea);
                dialog.appendChild(charCount);
                dialog.appendChild(footer);
            }
        });
    }

    function initButtonFeatures() {
        // Global action buttons (Approve All / Deny All)
        $(document).on('click', '.global-action-btn', async function(e) {
            var $btn = $(this);
            
            // If already clicked, prevent spam clicks
            if ($btn.data('clicked')) {
                e.preventDefault();
                return false;
            }
            
            var action = $btn.data('global-action');
            var commitId = $btn.data('commit-id');
            if (!action || !commitId) return;

            var denyOptions = null;
            if (action === 'approve') {
                var approveConfirmed = await showSimpleConfirmModal('Approve All', 'Accept all notes in this commit?');
                if (!approveConfirmed) return;
            } else {
                denyOptions = await showDenySuggestionModal();
                if (!denyOptions || denyOptions.cancelled) return;
            }
            
            // Mark as clicked and add visual loading state
            $btn.data('clicked', true)
                .addClass('global-loading')
                .css('pointer-events', 'none')
                .attr('aria-busy', 'true');
            
            var url = action === 'approve' ? '/ApproveCommit/' + commitId : '/DenyCommit/' + commitId;
            var fetchOptions = { method: 'POST', credentials: 'same-origin' };
            if (denyOptions) {
                fetchOptions.headers = { 'Content-Type': 'application/json' };
                fetchOptions.body = JSON.stringify({
                    silent: !!denyOptions.silent,
                    reason: denyOptions.reason || ''
                });
            }

            fetch(url, fetchOptions)
                .then(function(r) {
                    if (r.ok || r.redirected) {
                        try {
                            sessionStorage.removeItem('commit_select::' + commitId);
                        } catch (storageErr) {
                            // Ignore storage access errors
                        }
                    }
                    if (r.redirected) {
                        window.location.href = r.url;
                    } else {
                        location.reload();
                    }
                })
                .catch(function(err) {
                    console.error('Global action failed:', err);
                    $btn.data('clicked', false)
                        .removeClass('global-loading')
                        .css('pointer-events', '')
                        .attr('aria-busy', 'false');
                });
        });
        
        // Comprehensive spam-click protection for all interactive buttons (EXCEPT editor buttons, editor content, and global actions)
        // Use event delegation with a delay to allow original handlers to run first
        $(document).on('click.protection', '.action-btn, .tag_accept_button, .tag_deny_button, [data-action]:not([data-action="toggle-edit"]):not([data-action="cancel-edit"])', function(e) {
            var $btn = $(this);
            
            // Skip if click originated from within Trumbowyg editor
            if ($btn.closest('.trumbowyg-editor-box, .trumbowyg-modal-box, .trumbowyg-dropdown').length) {
                return; // Allow editor interactions to proceed normally
            }
            
            // If button is already disabled from previous click, prevent this click
            if ($btn.hasClass('disabled') || $btn.prop('disabled')) {
                e.preventDefault();
                e.stopPropagation();
                return false;
            }
            
            // Allow the original click to proceed, then apply protection with a small delay
            setTimeout(function() {
                // Check again - if button was already processed by original handler, don't interfere
                if ($btn.hasClass('disabled')) {
                    return; // Already handled by existing protection
                }
                
                if (typeof $btn.data('original-html') === 'undefined') {
                    $btn.data('original-html', $btn.html());
                }
                if (typeof $btn.data('original-aria-busy') === 'undefined') {
                    $btn.data('original-aria-busy', $btn.attr('aria-busy'));
                }

                // Apply protection
                $btn.addClass('disabled loading')
                    .prop('disabled', true)
                    .css('pointer-events', 'none')
                    .attr('aria-busy', 'true');

                // Timestamp for failsafe restoration
                $btn.data('click-timestamp', Date.now());
                
            }, 10); // Small delay to let original handlers run first
            
            // Set up failsafe restore
            setTimeout(function() {
                if ($btn.hasClass('disabled')) {
                    window.restoreButton($btn);
                }
            }, 5000); // 5 second failsafe
        });
        
        // Function to restore button state (can be called externally) - EXCLUDES editor buttons
        window.restoreButton = function($btn) {
            if ($btn && $btn.length) {
                // Don't restore editor buttons - let them manage their own state
                var action = $btn.data('action');
                if (action === 'toggle-edit' || action === 'cancel-edit') {
                    return;
                }
                
                $btn.removeClass('disabled loading')
                    .prop('disabled', false)
                    .css('pointer-events', '');

                var originalHtml = $btn.data('original-html');
                if (typeof originalHtml !== 'undefined') {
                    $btn.html(originalHtml);
                }

                var originalAriaBusy = $btn.data('original-aria-busy');
                if (typeof originalAriaBusy !== 'undefined' && originalAriaBusy !== null && originalAriaBusy !== '') {
                    $btn.attr('aria-busy', originalAriaBusy);
                } else {
                    $btn.removeAttr('aria-busy');
                }

                $btn.removeData('original-html');
                $btn.removeData('original-aria-busy');
                $btn.removeData('click-timestamp');

            }
        };
                        
        // Listen for successful API responses to restore buttons
        $(document).on('api:success', function(e, data) {
            if (data && data.buttonId) {
                var $btn = $('#' + data.buttonId);
                window.restoreButton($btn);
            }
        });
        
        // Listen for API errors to restore buttons
        $(document).on('api:error', function(e, data) {
            if (data && data.buttonId) {
                var $btn = $('#' + data.buttonId);
                window.restoreButton($btn);
                // Show error state briefly
                $btn.addClass('field-error');
                setTimeout(function() { $btn.removeClass('field-error'); }, 2000);
            }
        });
        
        // Auto-scroll to active edit field
        $(document).on('editors:initialized', function(e, noteId) {
            var $activeField = $('#' + noteId + ' .trumbowyg-editor').first();
            if ($activeField.length) {
                $activeField[0].scrollIntoView({ behavior: 'smooth', block: 'center' });
            }
        });
        
        // Visual feedback for successful actions
        $(document).on('field:accepted field:denied', function(e, fieldId) {
            var $field = $('[data-field-id="' + fieldId + '"]').closest('.suggestion-box');
            var isAccepted = e.type === 'field:accepted';
            
            $field.addClass(isAccepted ? 'field-success' : 'field-error');
            
            // Show brief success/error state
            setTimeout(function() {
                $field.fadeOut(300, function() {
                    $(this).remove();
                });
            }, 1000);
        });
        
        // Improved error handling with user feedback
        window.addEventListener('error', function(e) {
            console.error('JavaScript error:', e.error);
            // Could integrate with error reporting service here
        });
                        
        // Page visibility change - restore buttons when page becomes visible
        // (handles cases where user switches tabs during processing)
        document.addEventListener('visibilitychange', function() {
            if (!document.hidden) {
                // Check for buttons disabled longer than 10 seconds (EXCLUDE editor buttons)
                $('.disabled').each(function() {
                    var $btn = $(this);
                    var action = $btn.data('action');
                    var timestamp = $btn.data('click-timestamp');
                    
                    // Skip editor buttons - they manage their own state
                    if (action === 'toggle-edit' || action === 'cancel-edit') {
                        return;
                    }
                    
                    if (timestamp && (Date.now() - timestamp > 10000)) {
                        window.restoreButton($btn);
                    }
                });
            }
        });
        
        // Performance monitoring
        if (window.performance && window.performance.mark) {
            window.performance.mark('commit-page-interactive');
        }
    }

    function initPaginationControls() {
        var $notesContainer = $('#notes-container');
        if (!$notesContainer.length) {
            return;
        }

        var commitId = $notesContainer.data('commit-id');
        var $loadMoreContainer = $('#load-more-container');
        var $loadMoreBtn = $('#load-more-btn');
        var $loadMoreStatus = $('#load-more-status');
        var $loadedCount = $('#notes-loaded-count');
        var $totalCount = $('#notes-total-count');

        var paginationState = {
            commitId: commitId,
            total: Number($notesContainer.data('total')) || 0,
            loaded: Number($notesContainer.data('loaded')) || 0,
            nextOffset: parseOptionalNumber($notesContainer.data('nextOffset')),
        };

        updateControls();

        $(document).off('note:resolved.commitPagination');
        $(document).on('note:resolved.commitPagination', function(_event, detail) {
            paginationState.total = Math.max(0, paginationState.total - 1);
            paginationState.loaded = Math.max(0, paginationState.loaded - 1);

            $notesContainer.data('total', paginationState.total);
            $notesContainer.data('loaded', paginationState.loaded);

            $totalCount.text(paginationState.total);
            $loadedCount.text(paginationState.loaded);

            updateControls();
        });

        $loadMoreBtn.on('click', async function(event) {
            event.preventDefault();

            if ($loadMoreBtn.hasClass('disabled') || paginationState.nextOffset === null) {
                return;
            }

            setLoading(true);

            try {
                var response = await fetch('/commit/' + paginationState.commitId + '?offset=' + paginationState.nextOffset + '&limit=50&format=json', {
                    headers: { 'Accept': 'application/json' }
                });

                if (!response.ok) {
                    throw new Error('Request failed with status ' + response.status);
                }

                var payload = await response.json();

                if (payload.html) {
                    var temp = document.createElement('div');
                    temp.innerHTML = payload.html;
                    while (temp.firstChild) {
                        $notesContainer[0].appendChild(temp.firstChild);
                    }
                    SharedUI.initializePage();
                }

                paginationState.loaded += payload.loaded || 0;
                if (payload.total !== undefined && payload.total !== null) {
                    paginationState.total = Number(payload.total);
                }
                paginationState.nextOffset = parseOptionalNumber(payload.next_offset);

                $notesContainer.data('loaded', paginationState.loaded);
                $notesContainer.data('nextOffset', paginationState.nextOffset ?? '');

                $loadedCount.text(paginationState.loaded);
                $totalCount.text(paginationState.total);

                if (!payload.loaded) {
                    $loadMoreStatus.text('No additional notes available.');
                }
            } catch (error) {
                console.error('Failed to load more notes:', error);
                $loadMoreStatus.text('Failed to load more notes. Please try again.');
            } finally {
                setLoading(false);
                updateControls();
            }
        });

        function parseOptionalNumber(value) {
            if (value === undefined || value === null || value === '') {
                return null;
            }
            var numeric = Number(value);
            return Number.isFinite(numeric) ? numeric : null;
        }

        function setLoading(isLoading) {
            if (isLoading) {
                $loadMoreBtn.addClass('disabled loading').attr('aria-busy', 'true');
                $loadMoreStatus.text('Loading...');
            } else {
                $loadMoreBtn.removeClass('disabled loading').removeAttr('aria-busy');
                $loadMoreStatus.text('');
            }
        }

        function updateControls() {
            if (paginationState.nextOffset === null) {
                $loadMoreContainer.addClass('hidden');
            } else {
                $loadMoreContainer.removeClass('hidden');
            }
        }
    }

    /**
     * Bulk Selection Controls
     * Manages selection state in sessionStorage per commit
     * and handles bulk approve/deny actions
     */
    function initBulkSelectionControls() {
        var $bulkSection = $('#bulk-actions-section');
        if (!$bulkSection.length) return; // Not shown for non-owners

        var commitId = $bulkSection.data('commit-id');
        if (!commitId) return;

        // Sweep legacy commit_select::* keys previously persisted in localStorage
        try {
            for (var i = localStorage.length - 1; i >= 0; i--) {
                var legacyKey = localStorage.key(i);
                if (legacyKey && legacyKey.indexOf('commit_select::') === 0) {
                    localStorage.removeItem(legacyKey);
                }
            }
        } catch (e) {
            // Ignore storage access errors
        }

        var storageKey = 'commit_select::' + commitId;
        var $notesContainer = $('#notes-container');
        var $selectionCount = $('#selection-count');
        var $selectAllBtn = $('#select-all-visible-btn');
        var $deselectAllBtn = $('#deselect-all-btn');
        var $bulkApproveBtn = $('#bulk-approve-btn');
        var $bulkDenyBtn = $('#bulk-deny-btn');

        // Load selection from sessionStorage
        function getSelection() {
            try {
                var stored = sessionStorage.getItem(storageKey);
                var parsed = stored ? JSON.parse(stored) : [];
                return Array.isArray(parsed) ? parsed : [];
            } catch (e) {
                console.warn('Failed to parse selection from sessionStorage:', e);
                return [];
            }
        }

        // Save selection to sessionStorage (delete empty keys to avoid stale entries)
        function saveSelection(noteIds) {
            try {
                if (!noteIds || noteIds.length === 0) {
                    sessionStorage.removeItem(storageKey);
                } else {
                    sessionStorage.setItem(storageKey, JSON.stringify(noteIds));
                }
            } catch (e) {
                console.warn('Failed to save selection to sessionStorage:', e);
            }
        }

        // Update the selection count display
        function updateSelectionCount() {
            var selection = getSelection();
            var count = selection.length;
            $selectionCount.text(count);

            // Enable/disable bulk action buttons based on selection
            var hasSelection = count > 0;
            $bulkApproveBtn.prop('disabled', !hasSelection);
            $bulkDenyBtn.prop('disabled', !hasSelection);
            
            // Toggle visibility of deselect button
            if (hasSelection) {
                $deselectAllBtn.removeClass('hidden');
            } else {
                $deselectAllBtn.addClass('hidden');
            }
        }

        // Apply selection state to a checkbox
        function applySelectionToCheckbox($checkbox) {
            var noteId = String($checkbox.data('note-id'));
            var selection = getSelection();
            var isSelected = selection.includes(noteId);
            $checkbox.prop('checked', isSelected);
            
            var $noteCard = $checkbox.closest('.note-card');
            if (isSelected) {
                $noteCard.addClass('selected');
            } else {
                $noteCard.removeClass('selected');
            }
        }

        // Apply selection state to all visible checkboxes
        function applySelectionToAllCheckboxes() {
            $notesContainer.find('.note-select-checkbox').each(function() {
                applySelectionToCheckbox($(this));
            });
            updateSelectionCount();
        }

        // Toggle selection for a single note
        function toggleNoteSelection(noteId, isSelected) {
            var selection = getSelection();
            var noteIdStr = String(noteId);
            var index = selection.indexOf(noteIdStr);

            if (isSelected && index === -1) {
                selection.push(noteIdStr);
            } else if (!isSelected && index !== -1) {
                selection.splice(index, 1);
            }

            saveSelection(selection);
            updateSelectionCount();
        }

        // Event: Checkbox change
        $notesContainer.on('change', '.note-select-checkbox', function() {
            var $checkbox = $(this);
            var noteId = $checkbox.data('note-id');
            var isSelected = $checkbox.prop('checked');

            toggleNoteSelection(noteId, isSelected);

            var $noteCard = $checkbox.closest('.note-card');
            if (isSelected) {
                $noteCard.addClass('selected');
            } else {
                $noteCard.removeClass('selected');
            }
        });

        // Event: Select All Visible
        $selectAllBtn.on('click', function() {
            var selection = getSelection();
            $notesContainer.find('.note-select-checkbox').each(function() {
                var $checkbox = $(this);
                var noteId = String($checkbox.data('note-id'));
                
                if (!selection.includes(noteId)) {
                    selection.push(noteId);
                }
                $checkbox.prop('checked', true);
                $checkbox.closest('.note-card').addClass('selected');
            });
            saveSelection(selection);
            updateSelectionCount();
        });

        // Event: Deselect All (clears only visible notes from selection)
        $deselectAllBtn.on('click', function() {
            var selection = getSelection();
            var visibleNoteIds = [];
            
            $notesContainer.find('.note-select-checkbox').each(function() {
                var noteId = String($(this).data('note-id'));
                visibleNoteIds.push(noteId);
                $(this).prop('checked', false);
                $(this).closest('.note-card').removeClass('selected');
            });

            // Remove only visible notes from selection
            var newSelection = selection.filter(function(id) { return !visibleNoteIds.includes(id); });
            saveSelection(newSelection);
            updateSelectionCount();
        });

        // Bulk action: Approve or Deny selected notes
        async function performBulkAction(action, options) {
            var selection = getSelection();
            if (selection.length === 0) {
                return;
            }

            var isApprove = action === 'approve';

            // Disable all bulk buttons
            $bulkApproveBtn.prop('disabled', true).addClass('loading');
            $bulkDenyBtn.prop('disabled', true).addClass('loading');
            $selectAllBtn.prop('disabled', true);
            $deselectAllBtn.prop('disabled', true);

            // Mark selected notes as processing
            selection.forEach(function(noteId) {
                $('#' + noteId).addClass('bulk-processing');
            });

            try {
                var response = await fetch('/BulkNoteAction/' + commitId, {
                    method: 'POST',
                    headers: {
                        'Content-Type': 'application/json',
                    },
                    body: JSON.stringify({
                        note_ids: selection.map(function(id) { return parseInt(id, 10); }),
                        action: action,
                        silent: options && typeof options.silent === 'boolean' ? options.silent : undefined,
                        reason: options && typeof options.reason === 'string' ? options.reason : undefined
                    })
                });

                if (!response.ok) {
                    var errorData = await response.json().catch(function() { return {}; });
                    throw new Error(errorData.error || 'HTTP error! Status: ' + response.status);
                }

                var result = await response.json();

                // Process successful notes
                result.succeeded.forEach(function(noteId) {
                    var $noteCard = $('#' + noteId);
                    $noteCard.removeClass('bulk-processing selected')
                             .addClass('bulk-success');
                    
                    // Fade out and remove the card
                    setTimeout(function() {
                        $noteCard.fadeOut(300, function() {
                            $(this).remove();
                            if (window.MathJaxEngine && typeof window.MathJaxEngine.updateToggleVisibility === 'function') {
                                window.MathJaxEngine.updateToggleVisibility();
                            }
                            // Trigger note:resolved for pagination count update
                            $(document).trigger('note:resolved', { action: 'bulk-' + action, noteId: String(noteId) });
                        });
                    }, 500);
                });

                // Process failed notes
                result.failed.forEach(function(failure) {
                    var $noteCard = $('#' + failure.id);
                    $noteCard.removeClass('bulk-processing')
                             .addClass('bulk-error');
                    
                    // Show error tooltip/message
                    console.error('Failed to ' + action + ' note ' + failure.id + ': ' + failure.reason);
                    
                    // Remove error state after a moment
                    setTimeout(function() {
                        $noteCard.removeClass('bulk-error');
                    }, 3000);
                });

                // Update selection - remove succeeded notes, keep failed ones
                var failedIds = result.failed.map(function(f) { return String(f.id); });
                var newSelection = selection.filter(function(id) { return failedIds.includes(id); });
                saveSelection(newSelection);

                // Show summary message if there were failures
                if (result.failed.length > 0) {
                    var successCount = result.succeeded.length;
                    var failCount = result.failed.length;
                    await showInfoModal('Bulk Action Complete', (isApprove ? 'Approved' : 'Denied') + ' ' + successCount + ' note(s). ' + failCount + ' note(s) failed. Check the console for details.');
                }

            } catch (error) {
                console.error('Bulk ' + action + ' failed:', error);
                await showInfoModal('Action Failed', 'Failed to ' + action + ' selected notes: ' + error.message);

                // Remove processing state from all selected notes
                selection.forEach(function(noteId) {
                    $('#' + noteId).removeClass('bulk-processing');
                });
            } finally {
                // Re-enable buttons
                $bulkApproveBtn.removeClass('loading');
                $bulkDenyBtn.removeClass('loading');
                $selectAllBtn.prop('disabled', false);
                $deselectAllBtn.prop('disabled', false);
                
                // Update selection state and re-apply to checkboxes
                applySelectionToAllCheckboxes();
            }
        }

        // Event: Bulk Approve
        $bulkApproveBtn.on('click', async function() {
            if ($(this).prop('disabled')) return;
            
            var selection = getSelection();
            if (selection.length > 50) {
                var confirmed = await showSimpleConfirmModal('Approve Selected', 'You are about to approve ' + selection.length + ' notes. This may take a moment. Continue?');
                if (!confirmed) {
                    return;
                }
            }
            performBulkAction('approve', { silent: false, reason: '' });
        });

        // Event: Bulk Deny
        $bulkDenyBtn.on('click', async function() {
            if ($(this).prop('disabled')) return;
            
            var selection = getSelection();
            var denyOptions = await showDenySuggestionModal();
            if (!denyOptions || denyOptions.cancelled) {
                return;
            }
            performBulkAction('deny', denyOptions);
        });

        // Listen for new notes being loaded (pagination)
        // Re-apply selection state when new notes are appended
        var observer = new MutationObserver(function(mutations) {
            mutations.forEach(function(mutation) {
                if (mutation.addedNodes.length > 0) {
                    mutation.addedNodes.forEach(function(node) {
                        if (node.nodeType === 1 && $(node).hasClass('note-card')) {
                            var $checkbox = $(node).find('.note-select-checkbox');
                            if ($checkbox.length) {
                                applySelectionToCheckbox($checkbox);
                            }
                        }
                    });
                    updateSelectionCount();
                }
            });
        });

        observer.observe($notesContainer[0], { childList: true });

        // Listen for individual note resolutions to update selection without dropping off-page selections
        $(document).on('note:resolved', function(event, detail) {
            var selection = getSelection();
            if (!selection.length) {
                return;
            }

            var cleanedSelection;
            if (detail && detail.noteId !== undefined && detail.noteId !== null && String(detail.noteId) !== '') {
                var resolvedId = String(detail.noteId);
                cleanedSelection = selection.filter(function(id) { return id !== resolvedId; });
            } else {
                var existingNoteIds = [];
                $notesContainer.find('.note-card').each(function() {
                    existingNoteIds.push(String($(this).attr('id')));
                });
                cleanedSelection = selection.filter(function(id) { return existingNoteIds.includes(id); });
            }

            if (cleanedSelection.length !== selection.length) {
                saveSelection(cleanedSelection);
                updateSelectionCount();
            }
        });

        // Initialize: Apply saved selection state
        applySelectionToAllCheckboxes();
    }
});
