/**
 * review.js - Review page functionality
 * Reads hidden-count from data-hidden-count attribute on toggle button
 */
function toggleTags(button) {
    var collapsedTags = document.getElementById("collapsedTags");
    var hiddenCount = button.dataset.hiddenCount || '0';
    // Check for d-none class (Bootstrap) or inline display:none
    var isHidden = collapsedTags.classList.contains('d-none') || 
                   collapsedTags.style.display === "none";
    if (isHidden) {
        collapsedTags.classList.remove('d-none');
        collapsedTags.style.display = "flex";
        button.textContent = "Show less";
    } else {
        collapsedTags.classList.add('d-none');
        collapsedTags.style.display = "";
        button.textContent = "+" + hiddenCount + " more";
    }
}

function groupSuggestionsByCommit() {
    var suggestionsContainer = $('.suggestions-body').length ? $('.suggestions-body') : $('.suggestions-side');
    var suggestions = $('.suggestion-box').toArray();
    
    // Skip if no suggestions to group
    if (suggestions.length === 0) return;
    
    // Group suggestions by commit ID
    var commitGroups = {};
    
    suggestions.forEach(function(suggestion) {
        var $suggestion = $(suggestion);
        var commitId = null;
        
        // Try to find commit ID from field suggestions
        var commitLink = $suggestion.find('a[href*="/commit/"]');
        if (commitLink.length > 0) {
            var href = commitLink.attr('href');
            var match = href.match(/\/commit\/(\d+)/);
            if (match) {
                commitId = match[1];
            }
        }
        
        // For tags, we need to extract from data attributes
        if (!commitId) {
            var attrCommitId = $suggestion.attr('data-commit-id');
            if (attrCommitId && /^\d+$/.test(attrCommitId)) {
                commitId = attrCommitId;
            }
        }
        
        // Default group for items without commit ID
        if (!commitId) {
            commitId = 'other';
        }
        
        if (!commitGroups[commitId]) {
            commitGroups[commitId] = [];
        }
        
        commitGroups[commitId].push($suggestion.detach());
    });
    
    // Create grouped DOM structure safely
    Object.keys(commitGroups).forEach(function(commitId) {
        var items = commitGroups[commitId];
        if (items.length === 0) return;
        
        if (commitId === 'other') {
            // Keep individual styling for non-commit items
            items.forEach(function(item) {
                suggestionsContainer.append(item);
            });
        } else {
            var groupDiv = document.createElement('div');
            groupDiv.className = 'commit-group';

            var headerDiv = document.createElement('div');
            headerDiv.className = 'commit-header';

            var badgeLink = document.createElement('a');
            badgeLink.href = '/commit/' + encodeURIComponent(commitId);
            badgeLink.className = 'commit-badge';
            badgeLink.title = 'View Source Commit';

            var branchIcon = document.createElement('i');
            branchIcon.className = 'fa-solid fa-code-branch';
            branchIcon.setAttribute('aria-hidden', 'true');
            badgeLink.appendChild(branchIcon);
            badgeLink.appendChild(document.createTextNode(' Commit #' + commitId));

            var countSpan = document.createElement('span');
            countSpan.className = 'commit-change-count';
            countSpan.textContent = items.length + ' ' + (items.length === 1 ? 'change' : 'changes');

            headerDiv.appendChild(badgeLink);
            headerDiv.appendChild(countSpan);

            var suggestionsListEl = document.createElement('div');
            suggestionsListEl.className = 'commit-suggestions';

            groupDiv.appendChild(headerDiv);
            groupDiv.appendChild(suggestionsListEl);

            var suggestionsList = $(suggestionsListEl);
            items.forEach(function(item) {
                // Convert suggestion-box to suggestion-item
                item.removeClass('suggestion-box').addClass('suggestion-item');
                // Remove individual commit links since they're now in the group header
                item.find('.commit-link-badge').remove();
                suggestionsList.append(item);
            });
            
            suggestionsContainer.append(groupDiv);
        }
    });
}

function initiButtonFeatures() {
    // Function to restore button state
    window.restoreButton = function($btn) {
        if ($btn && $btn.length) {
            $btn.removeClass('disabled loading')
                .prop('disabled', false)
                .css('pointer-events', '');
            
            var originalText = $btn.data('original-text');
            if (originalText) {
                $btn.html(originalText);
            }
        }
    };
    
    // Global function to restore all buttons
    window.restoreAllButtons = function() {
        $('.disabled').each(function() {
            window.restoreButton($(this));
        });
    };
                
    // Page visibility change - restore buttons when page becomes visible
    document.addEventListener('visibilitychange', function() {
        if (!document.hidden) {
            // Check for buttons disabled longer than 10 seconds
            $('.disabled').each(function() {
                var $btn = $(this);
                var timestamp = $btn.data('click-timestamp');
                
                if (timestamp && (Date.now() - timestamp > 10000)) {
                    window.restoreButton($btn);
                }
            });
        }
    });
    
    // Visual feedback for successful actions
    $(document).on('field:accepted field:denied tag:accepted tag:denied', function(e, itemId) {
        var isAccepted = e.type.includes('accepted');
        var $item = $('[data-field-id="' + itemId + '"], [data-tag-id="' + itemId + '"], [data-move-id="' + itemId + '"]').closest('.suggestion-box, .suggestion-item, .field-diff-row, .field-item');
        
        $item.addClass(isAccepted ? 'field-success' : 'field-error');
        
        // Announce to screen readers
        if (window.a11yAnnounce) {
            window.a11yAnnounce(isAccepted ? 'Suggestion accepted' : 'Suggestion denied', 'polite');
        }
        
        // Show brief success/error state then fade out
        setTimeout(function() {
            $item.fadeOut(300, function() {
                $(this).remove();
            });
        }, 1000);
    });

    // Empty state after resolving the entire note from the header
    $(document).on('note:resolved', function(_e, detail) {
        var $page = $('.note-review-page');
        if (!$page.length) return;

        $page.find('[data-action="accept-note"], [data-action="delete-note"], [data-action="accept-note-removal"], [data-action="deny-note-removal"]')
            .prop('disabled', true);

        var $container = $page.children('.container-fluid');
        if (!$container.length || $container.find('.note-context, .commit-empty-state').length) {
            return;
        }

        var emptyEl = document.createElement('div');
        emptyEl.className = 'commit-empty-state';

        var iconWrap = document.createElement('div');
        iconWrap.className = 'commit-empty-icon';
        var icon = document.createElement('i');
        icon.className = 'fa-regular fa-circle-check';
        icon.setAttribute('aria-hidden', 'true');
        iconWrap.appendChild(icon);

        var heading = document.createElement('h2');
        heading.textContent = 'Note resolved';

        var desc = document.createElement('p');
        var action = detail && detail.action;
        if (action === 'delete-note' || action === 'accept-note-removal') {
            desc.textContent = 'This note has been removed.';
        } else if (action === 'deny-note-removal') {
            desc.textContent = 'The removal request was declined and this note was kept.';
        } else {
            desc.textContent = 'This note has been accepted and published.';
        }

        var backBtn = document.createElement('a');
        backBtn.className = 'commit-btn commit-btn-neutral';
        backBtn.href = '/reviews';
        backBtn.textContent = 'Back to Reviews';

        emptyEl.appendChild(iconWrap);
        emptyEl.appendChild(heading);
        emptyEl.appendChild(desc);
        emptyEl.appendChild(backBtn);
        $container[0].appendChild(emptyEl);
    });
    
    // Performance monitoring
    if (window.performance && window.performance.mark) {
        window.performance.mark('review-page-interactive');
    }
    
    // Enhanced spam-click protection
    $(document).on('click.protection', '.action-btn:not(.tag_accept_button):not(.tag_deny_button), [data-action="accept-field"], [data-action="deny-field"]', function(e) {
        var $btn = $(this);
        
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
            
            var action = $btn.data('action') || 'processing';
            var originalText = $btn.html();
            
            // Apply protection
            $btn.addClass('disabled loading')
                .prop('disabled', true)
                .css('pointer-events', 'none');
            
            // Update button text based on action
            var actionTexts = {
                'accept-field': '✓ Accepting...',
                'deny-field': '✗ Denying...',
                'accept-tag': '✓ Accepting...',
                'deny-tag': '✗ Denying...',
                'accept-move': '✓ Moving...',
                'deny-move': '✗ Denying...'
            };
            
            var processingText = actionTexts[action] || '⏳ Processing...';
            
            // For buttons with only icons, don't change text
            if (!originalText.includes('fa-') && originalText.trim().length > 2) {
                $btn.text(processingText);
            }
            
            // Store original state for potential restoration
            $btn.data('original-text', originalText);
            $btn.data('click-timestamp', Date.now());
            
        }, 10); // Small delay to let original handlers run first
        
        // Set up failsafe restore
        setTimeout(function() {
            if ($btn.hasClass('disabled')) {
                window.restoreButton($btn);
            }
        }, 5000); // 5 second failsafe
    });
}

// Initialize the shared UI components after the page loads
$(document).ready(function() {
    SharedUI.initializePage();
    
    initiButtonFeatures();
    
    // Group suggestions by commit
    groupSuggestionsByCommit();

    // Toggle tags button
    $(document).on('click', '.toggle-tags-btn', function() {
        toggleTags(this);
    });
});
