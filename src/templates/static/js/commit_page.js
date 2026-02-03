/**
 * commit_page.js - Commit review page functionality
 * Reads commit_id from data-commit-id attribute on #notes-container element
 */

// Mark that this page handles its own state restoration
window.commitPageRestoreHandled = true;

// Initialize the shared UI components after the page loads
$(document).ready(function() {
    SharedUI.initializePage();
    initButtonFeatures();
    initPaginationControls();
    initBulkSelectionControls();
    initStateRestoration(); // Must come after pagination controls
    
    /**
     * State Restoration - Restores scroll position and pagination state
     * after editing a note suggestion causes a page reload.
     */
    function initStateRestoration() {
        var savedState = sessionStorage.getItem('fieldEditPanel_restoreState');
        if (!savedState) {
            return;
        }
        
        var state;
        try {
            state = JSON.parse(savedState);
        } catch (e) {
            sessionStorage.removeItem('fieldEditPanel_restoreState');
            return;
        }
        
        // Validate required properties exist
        if (!state || typeof state.timestamp !== 'number' || !state.noteId) {
            sessionStorage.removeItem('fieldEditPanel_restoreState');
            return;
        }
        
        // Check if state is not too old (5 minutes)
        var STATE_EXPIRY_MS = 5 * 60 * 1000;
        if (Date.now() - state.timestamp > STATE_EXPIRY_MS) {
            sessionStorage.removeItem('fieldEditPanel_restoreState');
            return;
        }
        
        // Clear the stored state immediately to prevent re-triggering
        sessionStorage.removeItem('fieldEditPanel_restoreState');
        
        // Sanitize noteId for use in selector (should be numeric, but escape for safety)
        var targetNoteId = String(state.noteId).replace(/[^\w-]/g, '');
        if (!targetNoteId) {
            return;
        }
        
        var targetLoaded = Number(state.loaded) || 0;
        var $notesContainer = $('#notes-container');
        var commitId = $notesContainer.data('commit-id');
        
        var currentLoaded = Number($notesContainer.data('loaded')) || 0;
        
        // Check if target note is already visible (use getElementById for safety)
        var targetElement = document.getElementById(targetNoteId);
        if (targetElement) {
            // Note is already on page, just scroll to it
            scrollToNoteAndHighlight($(targetElement));
            return;
        }
        
        // Need to load more notes to reach the target
        if (targetLoaded > currentLoaded) {
            loadNotesUntilTarget(commitId, targetNoteId, targetLoaded, currentLoaded);
        } else {
            // Target should be on page but isn't - scroll to approximate position
            var scrollPos = Number(state.scrollPosition) || 0;
            if (scrollPos > 0) {
                window.scrollTo({ top: scrollPos, behavior: 'instant' });
            }
        }
    }
    
    /**
     * Loads notes in batches until the target note is found.
     */
    async function loadNotesUntilTarget(commitId, targetNoteId, targetLoaded, currentLoaded) {
        var $notesContainer = $('#notes-container');
        var $loadMoreContainer = $('#load-more-container');
        var $loadMoreBtn = $('#load-more-btn');
        var $loadMoreStatus = $('#load-more-status');
        var $loadedCount = $('#notes-loaded-count');
        
        // Show loading indicator
        $loadMoreStatus.html('<i class="fa fa-spinner fa-spin"></i> Restoring your position...');
        $loadMoreBtn.addClass('loading');
        
        var nextOffset = $notesContainer.data('nextOffset');
        // If no nextOffset, there's nothing more to load
        if (nextOffset === undefined || nextOffset === null || nextOffset === '') {
            $loadMoreStatus.text('');
            $loadMoreBtn.removeClass('loading');
            return;
        }
        
        var loaded = currentLoaded;
        var limit = 50;
        var maxIterations = Math.ceil((targetLoaded - currentLoaded) / limit) + 2; // Safety limit
        
        for (var i = 0; i < maxIterations && nextOffset !== null && nextOffset !== ''; i++) {
            try {
                var response = await fetch('/commit/' + commitId + '?offset=' + nextOffset + '&limit=' + limit + '&format=json', {
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
                
                loaded += payload.loaded || 0;
                nextOffset = payload.next_offset;
                
                // Update pagination state
                $notesContainer.data('loaded', loaded);
                $notesContainer.data('nextOffset', nextOffset ?? '');
                $loadedCount.text(loaded);
                
                // Check if target note is now visible (use getElementById for safety)
                var targetElement = document.getElementById(targetNoteId);
                if (targetElement) {
                    // Found the note - finish up
                    $loadMoreStatus.text('');
                    $loadMoreBtn.removeClass('loading');
                    
                    // Update load more button visibility
                    if (nextOffset === null || nextOffset === '') {
                        $loadMoreContainer.addClass('hidden');
                    }
                    
                    // Small delay to ensure DOM is settled
                    requestAnimationFrame(function() {
                        scrollToNoteAndHighlight($(targetElement));
                    });
                    return;
                }
                
                // Stop if we've loaded enough or no more to load
                if (loaded >= targetLoaded || !payload.loaded) {
                    break;
                }
                
            } catch (error) {
                console.error('Failed to load notes during state restoration:', error);
                $loadMoreStatus.text('Failed to restore position. Please scroll manually.');
                break;
            }
        }
        
        // Cleanup loading state
        $loadMoreStatus.text('');
        $loadMoreBtn.removeClass('loading');
        
        // Update load more button visibility
        if (nextOffset === null || nextOffset === '') {
            $loadMoreContainer.addClass('hidden');
        }
        
        // Even if we didn't find the exact note, scroll to approximate position
        // This handles cases where the note was deleted or moved
        var finalTargetElement = document.getElementById(targetNoteId);
        if (finalTargetElement) {
            scrollToNoteAndHighlight($(finalTargetElement));
        }
    }
    
    /**
     * Scrolls to a note card and highlights it briefly.
     */
    function scrollToNoteAndHighlight($noteCard) {
        if (!$noteCard.length) return;
        
        // Scroll to the note, centering it in the viewport
        $noteCard[0].scrollIntoView({ behavior: 'instant', block: 'center' });
        
        // Add a highlight effect to help user find their place
        $noteCard.addClass('save-highlight');
        setTimeout(function() {
            $noteCard.removeClass('save-highlight');
        }, 2500);
    }

    function initButtonFeatures() {
        // Global action buttons (Approve All / Deny All) - CSP-compliant click handler
        // NOTE: These buttons use href navigation - we only add visual feedback and prevent spam clicks
        $(document).on('click', '.global-action-btn', function(e) {
            var $btn = $(this);
            
            // If already clicked, prevent spam clicks
            if ($btn.data('clicked')) {
                e.preventDefault();
                return false;
            }
            
            // Mark as clicked and add visual loading state
            $btn.data('clicked', true)
                .addClass('global-loading')
                .css('pointer-events', 'none')
                .attr('aria-busy', 'true');
            
            // Let the navigation happen (don't call preventDefault)
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
        
        // Keyboard navigation support - exclude editor areas
        $('.suggestion-box').attr('tabindex', '0').on('keydown', function(e) {
            // Don't intercept keyboard events if we're inside an editor
            if ($(e.target).closest('.trumbowyg-box').length) {
                return; // Let the editor handle it
            }
            
            if (e.key === 'Enter' || e.key === ' ') {
                e.preventDefault();
                $(this).find('.modern-btn').first().click();
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
        
        // Preload critical resources
        var criticalResources = [
            '/static/plugins/trumbowyg/trumbowyg.min.js',
            '/static/plugins/trumbowyg/plugins/colors/trumbowyg.colors.js'
        ];
        
        criticalResources.forEach(function(resource) {
            var link = document.createElement('link');
            link.rel = 'preload';
            link.as = 'script';
            link.href = resource;
            document.head.appendChild(link);
        });
        
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
     * Manages selection state in localStorage per commit
     * and handles bulk approve/deny actions
     */
    function initBulkSelectionControls() {
        var $bulkSection = $('#bulk-actions-section');
        if (!$bulkSection.length) return; // Not shown for non-owners

        var commitId = $bulkSection.data('commit-id');
        if (!commitId) return;

        var storageKey = 'commit_select::' + commitId;
        var $notesContainer = $('#notes-container');
        var $selectionCount = $('#selection-count');
        var $selectAllBtn = $('#select-all-visible-btn');
        var $deselectAllBtn = $('#deselect-all-btn');
        var $bulkApproveBtn = $('#bulk-approve-btn');
        var $bulkDenyBtn = $('#bulk-deny-btn');

        // Load selection from localStorage
        function getSelection() {
            try {
                var stored = localStorage.getItem(storageKey);
                return stored ? JSON.parse(stored) : [];
            } catch (e) {
                console.warn('Failed to parse selection from localStorage:', e);
                return [];
            }
        }

        // Save selection to localStorage
        function saveSelection(noteIds) {
            try {
                localStorage.setItem(storageKey, JSON.stringify(noteIds));
            } catch (e) {
                console.warn('Failed to save selection to localStorage:', e);
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
        async function performBulkAction(action) {
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
                        action: action
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
                            // Trigger note:resolved for pagination count update
                            $(document).trigger('note:resolved', { action: 'bulk-' + action });
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
                    alert((isApprove ? 'Approved' : 'Denied') + ' ' + successCount + ' note(s). ' + failCount + ' note(s) failed. Check the console for details.');
                }

            } catch (error) {
                console.error('Bulk ' + action + ' failed:', error);
                alert('Failed to ' + action + ' selected notes: ' + error.message);

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
        $bulkApproveBtn.on('click', function() {
            if ($(this).prop('disabled')) return;
            
            var selection = getSelection();
            if (selection.length > 50) {
                if (!confirm('You are about to approve ' + selection.length + ' notes. This may take a moment. Continue?')) {
                    return;
                }
            }
            performBulkAction('approve');
        });

        // Event: Bulk Deny
        $bulkDenyBtn.on('click', function() {
            if ($(this).prop('disabled')) return;
            
            var selection = getSelection();
            if (!confirm('Are you sure you want to deny ' + selection.length + ' selected note(s)?')) {
                return;
            }
            performBulkAction('deny');
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

        // Listen for individual note resolutions to update selection
        $(document).on('note:resolved', function(event, detail) {
            // Get the note ID from the event if possible
            // For individual actions, the card is removed - clean up selection
            var selection = getSelection();
            var existingNoteIds = [];
            $notesContainer.find('.note-card').each(function() {
                existingNoteIds.push(String($(this).attr('id')));
            });
            
            // Remove any selected notes that no longer exist in the DOM
            var cleanedSelection = selection.filter(function(id) { return existingNoteIds.includes(id); });
            if (cleanedSelection.length !== selection.length) {
                saveSelection(cleanedSelection);
                updateSelectionCount();
            }
        });

        // Initialize: Apply saved selection state
        applySelectionToAllCheckboxes();
    }
});
