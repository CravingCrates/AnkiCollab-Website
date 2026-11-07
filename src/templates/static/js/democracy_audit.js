/**
 * democracy_audit.js - Deck Community Moderation Audit & Retroactive Control
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
    var pageEl = document.getElementById('democracy-audit-page');
    var deckHash = pageEl ? pageEl.dataset.deckHash : '';

    // Toggle voter audit drawer
    $(document).on('click', '.toggle-voters-btn', function () {
        var targetId = $(this).data('target');
        $(targetId).slideToggle(200);
    });

    // Filtering by status
    $('.filter-btn').on('click', function () {
        $('.filter-btn').removeClass('active');
        $(this).addClass('active');

        var filter = $(this).data('filter');
        applyFilters(filter, $('#audit-search-input').val().toLowerCase());
    });

    // Real-time search filter
    $('#audit-search-input').on('keyup', function () {
        var currentFilter = $('.filter-btn.active').data('filter') || 'all';
        var query = $(this).val().toLowerCase();
        applyFilters(currentFilter, query);
    });

    function applyFilters(statusFilter, searchQuery) {
        $('.audit-card').each(function () {
            var card = $(this);
            var cardStatus = card.data('status');
            var cardText = card.text().toLowerCase();

            var matchesStatus = true;
            if (statusFilter === 'approved') {
                matchesStatus = (cardStatus === 'approved' || cardStatus === 'approved_by_admin');
            } else if (statusFilter === 'flagged') {
                matchesStatus = (cardStatus === 'flagged_for_maintainer');
            } else if (statusFilter === 'reverted') {
                matchesStatus = (cardStatus === 'reverted_by_admin');
            } else if (statusFilter === 'rejected') {
                matchesStatus = (cardStatus === 'rejected' || cardStatus === 'rejected_by_admin');
            }

            var matchesSearch = (!searchQuery || cardText.indexOf(searchQuery) !== -1);

            if (matchesStatus && matchesSearch) {
                card.show();
            } else {
                card.hide();
            }
        });
    }

    // Revert action (Retro-control)
    $(document).on('click', '.btn-revert-action', function () {
        var type = $(this).data('type');
        var id = $(this).data('id');

        var reason = prompt("Maintainer Revert: Please provide a reason for reversing this adoption (e.g. invalid consensus, error):");
        if (reason === null) return; // User cancelled

        var payload = {
            deck: deckHash,
            suggestion_type: type,
            suggestion_id: parseInt(id, 10),
            action: 'revert',
            reason: reason.trim() || null
        };

        $.ajax({
            type: 'POST',
            url: '/Democracy/Audit/Action',
            data: JSON.stringify(payload),
            contentType: 'application/json',
            success: function (res) {
                if (res.success) {
                    toast_success('Adoption reverted successfully. Original note state restored.');
                    setTimeout(function () { location.reload(); }, 700);
                } else {
                    toast_error(res.message || 'Failed to revert adoption.');
                }
            },
            error: function (xhr) {
                toast_error(xhr.responseText || 'Error reverting adoption.');
            }
        });
    });

    // Confirm approval action
    $(document).on('click', '.btn-confirm-action', function () {
        var type = $(this).data('type');
        var id = $(this).data('id');

        if (!confirm('Confirm permanent approval of this suggestion for all subscribers?')) {
            return;
        }

        var payload = {
            deck: deckHash,
            suggestion_type: type,
            suggestion_id: parseInt(id, 10),
            action: 'confirm_approval',
            reason: 'Manual confirmation by deck maintainer'
        };

        $.ajax({
            type: 'POST',
            url: '/Democracy/Audit/Action',
            data: JSON.stringify(payload),
            contentType: 'application/json',
            success: function (res) {
                if (res.success) {
                    toast_success('Suggestion approved and permanently applied.');
                    setTimeout(function () { location.reload(); }, 700);
                } else {
                    toast_error(res.message || 'Failed to confirm approval.');
                }
            },
            error: function (xhr) {
                toast_error(xhr.responseText || 'Error confirming approval.');
            }
        });
    });

    // Reject action
    $(document).on('click', '.btn-reject-action', function () {
        var type = $(this).data('type');
        var id = $(this).data('id');

        if (!confirm('Permanently reject this suggestion?')) {
            return;
        }

        var payload = {
            deck: deckHash,
            suggestion_type: type,
            suggestion_id: parseInt(id, 10),
            action: 'admin_reject',
            reason: 'Rejected by deck maintainer'
        };

        $.ajax({
            type: 'POST',
            url: '/Democracy/Audit/Action',
            data: JSON.stringify(payload),
            contentType: 'application/json',
            success: function (res) {
                if (res.success) {
                    toast_success('Suggestion rejected successfully.');
                    setTimeout(function () { location.reload(); }, 700);
                } else {
                    toast_error(res.message || 'Failed to reject suggestion.');
                }
            },
            error: function (xhr) {
                toast_error(xhr.responseText || 'Error rejecting suggestion.');
            }
        });
    });

    // Blacklist voter action
    $(document).on('click', '.btn-blacklist-voter', function () {
        var username = $(this).data('username');
        if (!username) return;

        var reason = prompt("Blacklist voter '" + username + "' from moderating this deck? Please specify a reason:");
        if (reason === null) return;

        var payload = {
            deck: deckHash,
            username: username,
            reason: reason.trim() || null
        };

        $.ajax({
            type: 'POST',
            url: '/Democracy/Audit/Blacklist',
            data: JSON.stringify(payload),
            contentType: 'application/json',
            success: function (res) {
                if (res.success) {
                    toast_success('Voter ' + username + ' has been added to blacklist.');
                    setTimeout(function () { location.reload(); }, 700);
                } else {
                    toast_error(res.message || 'Failed to blacklist voter.');
                }
            },
            error: function (xhr) {
                toast_error(xhr.responseText || 'Error blacklisting voter.');
            }
        });
    });
});
