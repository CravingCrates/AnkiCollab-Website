/**
 * democracy.js - Deck Community Moderation & Governance management
 */
function toast_error(msg) {
    if (window.a11yAnnounce) { window.a11yAnnounce('Error: ' + msg, 'assertive'); }
    toastr.error(msg, "Error", {
        positionClass: "toast-top-right",
        timeOut: 4000,
        closeButton: true,
        progressBar: true,
    });
}

function toast_success(msg) {
    if (window.a11yAnnounce) { window.a11yAnnounce(msg, 'polite'); }
    toastr.success(msg, "Success", {
        timeOut: 3000,
        closeButton: true,
        progressBar: true,
        positionClass: "toast-top-right",
    });
}

$(document).ready(function () {
    var pageElement = document.getElementById('democracy-page');
    var deckHash = pageElement ? pageElement.dataset.deckHash : '';

    $('#save-config-btn').on('click', function () {
        var payload = {
            deck: deckHash,
            enabled: $('#democracy-enabled').is(':checked'),
            mode: $('#democracy-mode').val(),
            approval_threshold: parseInt($('#approval-threshold').val(), 10) || 5,
            require_unanimity: $('#require-unanimity').is(':checked'),
            weekly_vote_limit: parseInt($('#weekly-vote-limit').val(), 10) || 20,
            min_days_subscribed: parseInt($('#min-days-subscribed').val(), 10) || 14,
            min_cards_reviewed: parseInt($('#min-cards-reviewed').val(), 10) || 200,
            notifications_enabled: $('#notifications-enabled').is(':checked')
        };

        $.ajax({
            type: 'POST',
            url: '/Democracy',
            data: JSON.stringify(payload),
            contentType: 'application/json',
            success: function () {
                toast_success('Democracy configuration saved successfully.');
            },
            error: function (xhr) {
                toast_error(xhr.responseText || 'Failed to update democracy settings.');
            }
        });
    });

    $('#add-roster-btn').on('click', function () {
        var username = $('#roster-username').val().trim();
        var status = $('#roster-status').val();
        var allowed_tags = $('#roster-tags').val().trim();

        if (!username) {
            toast_error('Please specify a username.');
            return;
        }

        var payload = {
            deck: deckHash,
            username: username,
            status: status,
            action: 1, // Add
            allowed_tags: allowed_tags || null,
            reason: null
        };

        $.ajax({
            type: 'POST',
            url: '/Democracy/Roster',
            data: JSON.stringify(payload),
            contentType: 'application/json',
            success: function () {
                toast_success('User successfully added to roster.');
                setTimeout(function () { location.reload(); }, 600);
            },
            error: function (xhr) {
                toast_error(xhr.responseText || 'Failed to add user to roster.');
            }
        });
    });

    $(document).on('click', '.btn-remove-roster', function () {
        var username = $(this).data('username');
        if (!username) return;

        var payload = {
            deck: deckHash,
            username: username,
            status: '',
            action: 2, // Remove
            allowed_tags: null,
            reason: null
        };

        $.ajax({
            type: 'POST',
            url: '/Democracy/Roster',
            data: JSON.stringify(payload),
            contentType: 'application/json',
            success: function () {
                toast_success('User removed from roster.');
                setTimeout(function () { location.reload(); }, 600);
            },
            error: function (xhr) {
                toast_error(xhr.responseText || 'Failed to remove user from roster.');
            }
        });
    });
});
