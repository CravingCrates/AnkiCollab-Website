/**
 * reviews.js - DataTable initialization for reviews list page
 */

const reviewsDataTableConfig = {
    destroy: true,
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

            // Destroy existing DataTable instance
            if ($.fn.DataTable.isDataTable('#deckOverview')) {
                $('#deckOverview').DataTable().destroy();
            }

            // Replace tbody with fresh HTML from server
            $('#deckOverview tbody').html(newTbody.innerHTML);

            // Re-initialize DataTable with stored config
            table = $('#deckOverview').DataTable(reviewsDataTableConfig);

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
        if (isApprove) {
            if (!confirm('Accept all notes in this commit?')) return;
        } else {
            if (!confirm('Deny all notes in this commit?')) return;
        }

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

    // Close on escape key
    $(document).keyup(function(e) {
        if (e.key === "Escape" && $('#quick-preview-drawer').hasClass('open')) {
            closeDrawer();
        }
    });
});
