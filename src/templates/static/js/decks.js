(function($) {
    'use strict';

    function navigateToDeck(url) {
        window.location.href = url;
    }

    function parseDeckTimestamp(value) {
        var match = String(value).match(/^(\d{2})\/(\d{2})\/(\d{4})/);
        if (!match) {
            return NaN;
        }
        return new Date(Number(match[3]), Number(match[1]) - 1, Number(match[2])).getTime();
    }

    function formatDeckUpdate(cell) {
        var original = $(cell).data('last-update');
        var timestamp = parseDeckTimestamp(original);
        if (Number.isNaN(timestamp)) {
            return;
        }

        $(cell).attr('data-order', timestamp);

        var elapsed = Math.max(0, Date.now() - timestamp);
        var minutes = Math.floor(elapsed / 60000);
        var hours = Math.floor(minutes / 60);
        var days = Math.floor(hours / 24);
        var label;
        if (days > 30) {
            label = 'Updated on ' + original;
        } else if (minutes < 1) {
            label = 'Updated just now';
        } else if (minutes < 60) {
            label = 'Updated ' + minutes + (minutes === 1 ? ' minute' : ' minutes') + ' ago';
        } else if (hours < 24) {
            label = 'Updated ' + hours + (hours === 1 ? ' hour' : ' hours') + ' ago';
        } else {
            label = 'Updated ' + days + (days === 1 ? ' day' : ' days') + ' ago';
        }
        $(cell).text(label);
    }

    $(function() {
        $('.deck-updated').each(function() {
            formatDeckUpdate(this);
        });

        var table = $('#deckOverview').DataTable({
            destroy: true,
            order: [[2, 'desc']],
            pageLength: 25,
            stripeClasses: [],
            language: {
                search: '',
                searchPlaceholder: 'Search decks'
            }
        });

        $('#deckOverview tbody').on('click', 'tr', function(event) {
            if ($(event.target).closest('a, button').length) {
                return;
            }
            navigateToDeck($(this).data('deck-url'));
        });

        $('#deckOverview tbody').on('keydown', 'tr', function(event) {
            if (event.key !== 'Enter' && event.key !== ' ') {
                return;
            }
            event.preventDefault();
            navigateToDeck($(this).data('deck-url'));
        });

        $(document).on('keydown', function(event) {
            if (event.key === '/' && !$(event.target).is('input, textarea, select')) {
                event.preventDefault();
                $('.dataTables_filter input').focus();
            }
        });
    });
}(jQuery));
