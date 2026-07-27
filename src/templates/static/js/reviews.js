/**
 * reviews.js - DataTable initialization for reviews list page
 */

const reviewsDataTableConfig = {
    destroy: true,
    dom: '<"reviews-table-toolbar"l<"reviews-toolbar-right"f>>rtip',
    language: {
        search: '',
        searchPlaceholder: 'Search commits…'
    },
    columnDefs: [
    {
        "targets": 4,
        "render": function ( data, type, row ) {
            if (type === 'sort' || type === 'type') {
                if (typeof moment !== 'undefined') {
                    return moment(data, 'MM/DD/YYYY').format('YYYYMMDD');
                }
                return data;
            }
            return data;
        }
    }],
    stripeClasses: ['odd', 'even'],
    order: [[4, 'desc']],
    "pageLength": 25,
};

$(document).ready(function() {
    // Initialize DataTable with date sorting
    let table = $('#deckOverview').DataTable(reviewsDataTableConfig);

    let currentRow = null;

    // ── Deck Filter Module ──────────────────────────────────────────
    var DECK_FILTER_KEY = 'reviews_deck_filter';
    var DECK_FILTER_ALL = '__all__';
    var SEARCH_THRESHOLD = 8;
    var activeDeckFilter = DECK_FILTER_ALL;

    // Register one custom DataTable search function that filters on data-deck
    // with prefix matching so "A" also shows "A::A1".
    $.fn.dataTable.ext.search.push(function(settings, data, dataIndex) {
        if (activeDeckFilter === DECK_FILTER_ALL) return true;
        var row = settings.aoData[dataIndex].nTr;
        if (!row) return true;
        var rowDeck = ($(row).attr('data-deck') || '').trim();
        if (!rowDeck) return false;
        // Prefix match: selected deck matches row deck exactly or as parent
        return rowDeck === activeDeckFilter ||
               rowDeck.indexOf(activeDeckFilter + '::') === 0;
    });

    /** Extract unique deck names and their commit counts from table rows. */
    function extractDecksFromTable() {
        var deckMap = {};
        $('#deckOverview tbody tr[data-deck]').each(function() {
            var deckName = ($(this).attr('data-deck') || '').trim();
            if (!deckName) {
                deckName = '(Unnamed)';
            }
            deckMap[deckName] = (deckMap[deckName] || 0) + 1;
        });
        // Sort alphabetically, case-insensitive
        var sorted = Object.keys(deckMap).sort(function(a, b) {
            return a.toLowerCase().localeCompare(b.toLowerCase());
        });
        var result = {};
        sorted.forEach(function(k) { result[k] = deckMap[k]; });
        return result;
    }

    /** Build and populate the dropdown menu from a deck→count map. */
    function buildDeckFilterMenu(deckMap, dataTable) {
        var $options = $('#deckFilterOptions');
        var $searchWrap = $('#deckFilterSearchWrap');
        var $searchInput = $('#deckFilterSearch');
        var deckNames = Object.keys(deckMap);
        var totalCommits = 0;
        deckNames.forEach(function(k) { totalCommits += deckMap[k]; });

        // Show/hide search input based on deck count
        if (deckNames.length > SEARCH_THRESHOLD) {
            $searchWrap.addClass('reviews-filter-search-visible');
            $searchInput.val('');
        } else {
            $searchWrap.removeClass('reviews-filter-search-visible');
            $searchInput.val('');
        }

        // Build option HTML
        var html = '';
        // "All Decks" option
        html += '<button class="reviews-filter-option selected" data-deck="' + DECK_FILTER_ALL + '" type="button">';
        html += '<span class="deck-name-text">All Decks</span>';
        html += '<span class="deck-count">' + totalCommits + '</span>';
        html += '</button>';

        deckNames.forEach(function(deckName) {
            var count = deckMap[deckName];
            html += '<button class="reviews-filter-option" data-deck="' + escapeHtmlAttr(deckName) + '" type="button">';
            html += '<span class="deck-name-text" title="' + escapeHtmlAttr(deckName) + '">' + escapeHtml(deckName) + '</span>';
            html += '<span class="deck-count">' + count + '</span>';
            html += '</button>';
        });

        $options.html(html);
    }

    /** Escape a string for safe use in HTML attributes. */
    function escapeHtmlAttr(str) {
        return String(str)
            .replace(/&/g, '&amp;')
            .replace(/"/g, '&quot;')
            .replace(/</g, '&lt;')
            .replace(/>/g, '&gt;');
    }

    /** Escape a string for safe use in HTML text content. */
    function escapeHtml(str) {
        return String(str)
            .replace(/&/g, '&amp;')
            .replace(/</g, '&lt;')
            .replace(/>/g, '&gt;');
    }

    /** Apply a deck filter to the DataTable and update UI state. */
    function applyDeckFilter(dataTable, deckName, skipSave) {
        var $label = $('#deckFilterLabel');
        var $btn = $('#deckFilterBtn');

        if (!deckName || deckName === DECK_FILTER_ALL) {
            // Show all
            activeDeckFilter = DECK_FILTER_ALL;
            dataTable.draw();
            $label.text('All Decks');
            $btn.removeClass('active');
            $('#deckFilterOptions .reviews-filter-option').removeClass('selected');
            $('#deckFilterOptions .reviews-filter-option[data-deck="' + DECK_FILTER_ALL + '"]').addClass('selected');
            if (!skipSave) {
                try { sessionStorage.removeItem(DECK_FILTER_KEY); } catch(e) {}
            }
        } else {
            // Filter to specific deck (prefix match via ext.search)
            activeDeckFilter = deckName;
            dataTable.draw();
            var displayName = deckName.length > 30 ? deckName.substring(0, 28) + '…' : deckName;
            $label.text(displayName);
            $label.attr('title', deckName);
            $btn.addClass('active');
            $('#deckFilterOptions .reviews-filter-option').removeClass('selected');
            $('#deckFilterOptions .reviews-filter-option[data-deck="' + escapeHtmlAttr(deckName) + '"]').addClass('selected');
            if (!skipSave) {
                try { sessionStorage.setItem(DECK_FILTER_KEY, deckName); } catch(e) {}
            }
        }
    }

    /** Rebuild the menu and re-apply the stored filter (called after table refresh). */
    function reapplyDeckFilter(dataTable) {
        var deckMap = extractDecksFromTable();
        var deckNames = Object.keys(deckMap);

        if (deckNames.length <= 1) {
            // 0 or 1 unique deck: hide filter, clear any stored selection
            $('#deckFilterDropdown').addClass('reviews-filter-dropdown-hidden');
            try { sessionStorage.removeItem(DECK_FILTER_KEY); } catch(e) {}
            // Clear the custom filter
            activeDeckFilter = DECK_FILTER_ALL;
            dataTable.draw();
            return;
        }

        $('#deckFilterDropdown').removeClass('reviews-filter-dropdown-hidden');
        // Safety: ensure filter dropdown is mounted in toolbar
        if ($('#deckFilterDropdown').parent().closest('.reviews-table-toolbar').length === 0) {
            var $right = $('.reviews-toolbar-right');
            if ($right.length) {
                $right.append($('#deckFilterDropdown'));
            }
        }
        buildDeckFilterMenu(deckMap, dataTable);

        var stored = null;
        try { stored = sessionStorage.getItem(DECK_FILTER_KEY); } catch(e) {}

        if (stored && deckMap[stored]) {
            applyDeckFilter(dataTable, stored, true);
        } else {
            applyDeckFilter(dataTable, DECK_FILTER_ALL, true);
        }
    }

    /** One-time initialization of the deck filter on page load. */
    function initDeckFilter(dataTable) {
        var $mount = $('#reviewsDeckFilterMount');
        var $right = $('.reviews-toolbar-right');

        // Mount the deck filter dropdown into the DataTable toolbar's right section
        if ($mount.length && $right.length) {
            $mount.children().first().detach().appendTo($right);
            $mount.remove();
        }

        var deckMap = extractDecksFromTable();
        var deckNames = Object.keys(deckMap);

        if (deckNames.length <= 1) {
            $('#deckFilterDropdown').addClass('reviews-filter-dropdown-hidden');
            return;
        }

        $('#deckFilterDropdown').removeClass('reviews-filter-dropdown-hidden');
        buildDeckFilterMenu(deckMap, dataTable);

        // Restore persisted filter
        var stored = null;
        try { stored = sessionStorage.getItem(DECK_FILTER_KEY); } catch(e) {}
        if (stored && deckMap[stored]) {
            applyDeckFilter(dataTable, stored, true);
        }

        // ── Event handlers ──────────────────────────────────────

        // Toggle dropdown
        $('#deckFilterBtn').on('click', function(e) {
            e.stopPropagation();
            var $menu = $('#deckFilterMenu');
            var $btn = $(this);
            if ($menu.hasClass('show')) {
                $menu.removeClass('show');
                $btn.removeClass('active');
            } else {
                $menu.addClass('show');
                $btn.addClass('active');
                // Focus search input if visible
                if ($('#deckFilterSearchWrap').hasClass('reviews-filter-search-visible')) {
                    setTimeout(function() { $('#deckFilterSearch').focus(); }, 50);
                }
            }
        });

        // Close dropdown on outside click
        $(document).on('click', function(e) {
            if (!$(e.target).closest('#deckFilterDropdown').length) {
                $('#deckFilterMenu').removeClass('show');
                $('#deckFilterBtn').removeClass('active');
            }
        });

        // Option selection via delegation
        $('#deckFilterOptions').on('click', '.reviews-filter-option', function(e) {
            e.stopPropagation();
            var deckName = $(this).attr('data-deck');
            if (deckName === DECK_FILTER_ALL) {
                applyDeckFilter(table, DECK_FILTER_ALL);
            } else {
                // Decode the attribute (it was HTML-escaped on build)
                applyDeckFilter(table, deckName);
            }
            $('#deckFilterMenu').removeClass('show');
            $('#deckFilterBtn').removeClass('active');
        });

        // Search within dropdown
        var searchTimeout;
        $('#deckFilterSearch').on('input', function() {
            clearTimeout(searchTimeout);
            var self = this;
            searchTimeout = setTimeout(function() {
                var query = self.value.toLowerCase().trim();
                var $options = $('#deckFilterOptions .reviews-filter-option');
                var visibleCount = 0;
                $options.each(function() {
                    var $opt = $(this);
                    var deckVal = ($opt.attr('data-deck') || '').toLowerCase();
                    var matches = deckVal === DECK_FILTER_ALL || deckVal.indexOf(query) !== -1;
                    if (matches) {
                        $opt.removeClass('reviews-filter-option-hidden');
                        visibleCount++;
                    } else {
                        $opt.addClass('reviews-filter-option-hidden');
                    }
                });
                // Show/hide "no results" message
                var $noResults = $('#deckFilterNoResults');
                if (visibleCount === 0) {
                    if (!$noResults.length) {
                        $('#deckFilterOptions').after('<div class="reviews-filter-no-results" id="deckFilterNoResults">No matching decks</div>');
                    }
                } else {
                    $noResults.remove();
                }
            }, 150);
        });
    }

    // ── Initialize deck filter ──────────────────────────────────
    initDeckFilter(table);

    // Row click listener bound to the parent table to survive DataTables redraws
    $('#deckOverview').on('click', 'tbody tr', function (e) {
        // If the user clicked an actual link, allow normal navigation
        if ($(e.target).is('a') || $(e.target).closest('a').length) {
            return;
        }

        currentRow = $(this);
        openDrawer(currentRow);
    });

    function loadDrawerContent(row) {
        // Update table active state
        $('#deckOverview tbody tr').removeClass('table-active');
        row.addClass('table-active');
        
        const commitId = row.data('commit-id');
        
        if (!commitId) return false;

        $('#btnFullEditor').attr('href', `/commit/${commitId}`);

        $('#drawerBody').html('<div class="d-flex justify-content-center align-items-center" style="min-height: 200px;"><div class="review-spinner"></div></div>');

        // Fetch /commit_preview/{commit_id} and inject into #drawer-body 
        if (loadDrawerContent._xhr) {
            loadDrawerContent._xhr.abort();
        }
        loadDrawerContent._xhr = $.ajax({
            url: `/commit_preview/${commitId}`,
            method: 'GET',
            success: function(response) {
                $('#drawerBody').html(response);
                if (window.SharedUI) {
                    $('#drawerBody').find('.note-context').each(function() {
                        window.SharedUI.initializeNoteCard(this);
                    });
                }
            },
            error: function(xhr, status) {
                if (status === 'abort') return;
                $('#drawerBody').html('<div class="alert alert-danger mt-3">Failed to load preview. Please try again later.</div>');
            }
        });

        updateNavigationButtons(row);
        return true;
    }

    function updateNavigationButtons(row) {
        // Find visible rows in case of search/sort
        const allVisibleRows = table.rows({ page: 'current', search: 'applied' }).nodes();
        const currentIndex = $(allVisibleRows).index(row[0]);
        
        $('#btnPrev').prop('disabled', currentIndex <= 0);
        $('#btnNext').prop('disabled', currentIndex >= allVisibleRows.length - 1 || currentIndex < 0);
    }

    $('#btnPrev').on('click', function() {
        if (!currentRow) return;
        const allVisibleRows = table.rows({ page: 'current', search: 'applied' }).nodes();
        const currentIndex = $(allVisibleRows).index(currentRow[0]);
        if (currentIndex > 0) {
            const prevRow = $(allVisibleRows[currentIndex - 1]);
            currentRow = prevRow;
            loadDrawerContent(prevRow);
        }
    });

    $('#btnNext').on('click', function() {
        if (!currentRow) return;
        const allVisibleRows = table.rows({ page: 'current', search: 'applied' }).nodes();
        const currentIndex = $(allVisibleRows).index(currentRow[0]);
        if (currentIndex >= 0 && currentIndex < allVisibleRows.length - 1) {
            const nextRow = $(allVisibleRows[currentIndex + 1]);
            currentRow = nextRow;
            loadDrawerContent(nextRow);
        }
    });

    function openDrawer(row) {
        if (!loadDrawerContent(row)) return;
        $('#quick-preview-drawer').addClass('open');
        $('#drawer-overlay').addClass('show');
        $('body').css('overflow', 'hidden'); // Prevent main page scrolling
    }

    function closeDrawer() {
        $('#quick-preview-drawer').removeClass('open');
        $('#drawer-overlay').removeClass('show');
        $('#deckOverview tbody tr').removeClass('table-active');
        $('body').css('overflow', ''); // Restore scrolling
        currentRow = null;
    }

    // ---- Drawer Loading Overlay ----
    function showDrawerLoading() {
        $('.drawer-loading-overlay').remove();
        $('#drawerBody').append(
            '<div class="drawer-loading-overlay visible">' +
                '<div class="review-spinner"></div>' +
            '</div>'
        );
        $('#drawerBody').addClass('drawer-body-processing');
    }

    function hideDrawerLoading() {
        $('.drawer-loading-overlay').removeClass('visible');
        $('#drawerBody').removeClass('drawer-body-processing');
        // Remove the element after the transition completes
        setTimeout(() => $('.drawer-loading-overlay').remove(), 200);
    }

    async function refreshTableAndOpenNext() {
        try {
            const response = await fetch(window.location.href, { credentials: 'same-origin' });
            const html = await response.text();
            const parser = new DOMParser();
            const doc = parser.parseFromString(html, 'text/html');
            const newTbody = doc.querySelector('#deckOverview tbody');

            if (!newTbody || !newTbody.querySelector('tr')) {
                // No commits left — close drawer and reload to show empty state
                hideDrawerLoading();
                closeDrawer();
                window.location.reload();
                return;
            }

            // Detach the deck filter dropdown before DataTable is destroyed
            var $savedFilter = $('#deckFilterDropdown').detach();

            // Destroy existing DataTable instance
            if ($.fn.DataTable.isDataTable('#deckOverview')) {
                $('#deckOverview').DataTable().destroy();
            }

            // Replace tbody with fresh HTML from server
            $('#deckOverview tbody').html(newTbody.innerHTML);

            // Re-initialize DataTable with stored config
            table = $('#deckOverview').DataTable(reviewsDataTableConfig);

            // Re-mount deck filter into the new toolbar
            var $right = $('.reviews-toolbar-right');
            if ($right.length && $savedFilter.length) {
                $right.append($savedFilter);
            }

            // Re-apply deck filter if one was stored
            reapplyDeckFilter(table);

            // Hide the loading overlay
            hideDrawerLoading();

            // Open the first commit in the drawer
            const firstRow = $(table.row(0).node());
            if (firstRow.length) {
                currentRow = firstRow;
                loadDrawerContent(firstRow);
            }
        } catch (err) {
            console.error('Failed to refresh table:', err);
            hideDrawerLoading();
            window.location.reload();
        }
    }

    $('#closeDrawer').on('click', function() {
        closeDrawer();
    });

    $('#drawer-overlay').on('click', function() {
        closeDrawer();
    });

    // ---- Helper: reset all quick-action buttons to idle state ----
    function resetQuickActionButtons() {
        $('.global-action-preview-btn')
            .data('clicked', false)
            .removeClass('global-loading disabled')
            .css('pointer-events', '')
            .attr('aria-busy', 'false')
            .each(function() {
                const $b = $(this);
                const $icon = $b.find('i');
                $icon.removeClass('review-spinner')
                     .css({ width: '', height: '', borderWidth: '', fontSize: '' });
                const action = $b.data('global-action');
                if (action === 'approve') {
                    $icon.addClass('fa fa-check-circle');
                } else if (action === 'deny') {
                    $icon.addClass('fa fa-times-circle');
                }
            });
    }

    // Handle Quick Action Approve/Deny
    $(document).on('click', '.global-action-preview-btn', async function(e) {
        const $btn = $(this);
        if ($btn.data('clicked')) return false;

        const action = $btn.data('global-action');
        const commitId = $btn.data('commit-id');
        if (!action || !commitId) return;

        const isApprove = action === 'approve';

        // Lock ALL quick-action buttons to prevent double-submit
        $('.global-action-preview-btn')
            .data('clicked', true)
            .addClass('global-loading disabled')
            .css('pointer-events', 'none')
            .attr('aria-busy', 'true');
        

        $('.global-action-preview-btn').each(function() {
            const $b = $(this);
            if (!$b.find('.review-spinner').length) {
                $b.find('i').removeClass().addClass('review-spinner')
                  .css({ width: '14px', height: '14px', borderWidth: '2px', fontSize: '0' });
            }
        });

        showDrawerLoading();

        const url = isApprove ? `/ApproveCommit/${commitId}` : `/DenyCommit/${commitId}`;
        const fetchOptions = {
            method: 'POST',
            credentials: 'same-origin',
            // Don't follow the 303 redirect — we handle the success ourselves
            redirect: 'manual',
        };

        if (!isApprove) {
            fetchOptions.headers = { 'Content-Type': 'application/json' };
            fetchOptions.body = JSON.stringify({ silent: true }); // By default, silent deny for quick action
        }

        try {
            const r = await fetch(url, fetchOptions);

            if (r.type === 'opaqueredirect') {
                await refreshTableAndOpenNext();
                resetQuickActionButtons();
                return;
            }

            if (!r.ok) {
                throw new Error(`Request failed: ${r.status}`);
            }

            await refreshTableAndOpenNext();
            resetQuickActionButtons();
        } catch (err) {
            console.error(err);
            hideDrawerLoading();
            resetQuickActionButtons();
            alert('An error occurred while processing the commit.');
        }
    });

    // Close on escape key (drawer first, then filter dropdown)
    $(document).keyup(function(e) {
        if (e.key === "Escape") {
            if ($('#quick-preview-drawer').hasClass('open')) {
                closeDrawer();
            } else if ($('#deckFilterMenu').hasClass('show')) {
                $('#deckFilterMenu').removeClass('show');
                $('#deckFilterBtn').removeClass('active');
            }
        }
    });
});
