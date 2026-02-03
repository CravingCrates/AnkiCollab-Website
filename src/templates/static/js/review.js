/**
 * review.js - Review page functionality
 * Reads hidden-count from data-hidden-count attribute on toggle button
 */
function toggleTags(button) {
    var collapsedTags = document.getElementById("collapsedTags");
    var hiddenCount = button.dataset.hiddenCount || '0';
    if (collapsedTags.style.display === "none") {
        collapsedTags.style.display = "flex";
        button.innerHTML = "Show less";
    } else {
        collapsedTags.style.display = "none";
        button.innerHTML = "+" + hiddenCount + " more";
    }
}

function groupSuggestionsByCommit() {
    var suggestionsContainer = $('.suggestions-side');
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
            commitId = $suggestion.attr('data-commit-id');
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
    
    // Create grouped HTML structure
    Object.keys(commitGroups).forEach(function(commitId) {
        var items = commitGroups[commitId];
        if (items.length === 0) return;
        
        var groupHtml;
        if (commitId === 'other') {
            // Keep individual styling for non-commit items
            items.forEach(function(item) {
                suggestionsContainer.append(item);
            });
        } else {
            // Create commit group container
            groupHtml = $(
                '<div class="commit-group">' +
                '<div class="commit-header">' +
                '<a href="/commit/' + commitId + '" class="commit-badge" title="View Source Commit">' +
                '<i class="fa fa-code-fork"></i> Commit #' + commitId +
                '</a>' +
                '<span style="color: #6b7280; font-size: 0.9rem;">' +
                items.length + ' ' + (items.length === 1 ? 'change' : 'changes') +
                '</span>' +
                '</div>' +
                '<div class="commit-suggestions"></div>' +
                '</div>'
            );
            
            var suggestionsList = groupHtml.find('.commit-suggestions');
            items.forEach(function(item) {
                // Convert suggestion-box to suggestion-item
                item.removeClass('suggestion-box').addClass('suggestion-item');
                // Remove individual commit links since they're now in the group header
                item.find('a[href*="/commit/"]').parent().remove();
                suggestionsList.append(item);
            });
            
            suggestionsContainer.append(groupHtml);
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
        var $item = $('[data-field-id="' + itemId + '"], [data-tag-id="' + itemId + '"], [data-move-id="' + itemId + '"]').closest('.suggestion-box, .field-item');
        
        $item.addClass(isAccepted ? 'field-success' : 'field-error');
        
        // Show brief success/error state then fade out
        setTimeout(function() {
            $item.fadeOut(300, function() {
                $(this).remove();
            });
        }, 1000);
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
                $btn.html(processingText);
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
    
    // CSP-compliant event handlers
    // Toggle tags button (replaces inline onclick)
    $(document).on('click', '.toggle-tags-btn', function() {
        toggleTags(this);
    });
    
    // Action links (replaces inline onclick for navigation links)
    $(document).on('click', '.action-link', function(e) {
        var $link = $(this);
        if ($link.hasClass('disabled')) {
            e.preventDefault();
            return false;
        }
        // Disable and show loading state
        $link.addClass('disabled')
            .css('pointer-events', 'none');
        
        var loadingText = $link.data('loading-text');
        if (loadingText) {
            $link.html(loadingText);
        }
    });
});
