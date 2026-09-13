/* Reviews keeps selection in the list and renders only the preview excerpt in
   the detail pane; the full editor remains the single destination for edits. */
$(function() {
    const page = $('#reviewsPage');
    const list = $('#reviewsList');
    let selectedRow = null;
    let activeDeck = '';
    let request = null;

    function escapeHtml(value) {
        return $('<div>').text(value || '').html();
    }

    function initials(name) {
        const parts = String(name || '').trim().split(/\s+/).filter(Boolean);
        return (parts.length > 1 ? parts[0][0] + parts[parts.length - 1][0] : (parts[0] || '?').slice(0, 2)).toUpperCase();
    }

    $('.review-avatar').each(function() { $(this).text(initials($(this).data('author'))); });

    function rows() { return list.children('.review-row'); }

    function filteredRows() {
        const query = $('#reviewsSearch').val().toLowerCase().trim();
        return rows().filter(function() {
            const row = $(this);
            const deck = String(row.data('deck') || '').toLowerCase();
            const author = String(row.data('author') || '').toLowerCase();
            const deckMatch = !activeDeck || deck === activeDeck.toLowerCase() || deck.indexOf(activeDeck.toLowerCase() + '::') === 0;
            return deckMatch && (!query || author.indexOf(query) !== -1 || deck.indexOf(query) !== -1);
        });
    }

    function topLevelDeck(deck) {
        return String(deck || '').split('::')[0] || '(Unnamed)';
    }

    function refreshList() {
        const visible = filteredRows();
        rows().toggle(false);
        visible.toggle(true);
        $('#reviewsEmptyList').prop('hidden', visible.length !== 0);
        $('#reviewsCount').text(visible.length);
        if (selectedRow && visible.filter(selectedRow).length === 0) selectRow(visible.first(), false);
        if (!visible.length) clearDetail();
        updateNavigation();
    }

    function buildDeckControl() {
        const names = {};
        rows().each(function() { names[topLevelDeck($(this).data('deck'))] = true; });
        const decks = Object.keys(names).sort((a, b) => a.localeCompare(b));
        const visibleDecks = decks.slice(0, 5);
        let html = '<button class="reviews-deck-option selected" type="button" data-deck="">All decks</button>';
        visibleDecks.forEach(deck => { html += `<button class="reviews-deck-option" type="button" data-deck="${escapeHtml(deck)}" title="${escapeHtml(deck)}">${escapeHtml(deck)}</button>`; });
        $('#deckFilterOptions').html(html);
        $('#deckFilterSearch').toggleClass('visible', decks.length > visibleDecks.length);
    }

    function updateDeckControl() {
        $('#deckFilterOptions .reviews-deck-option').each(function() { $(this).toggleClass('selected', $(this).data('deck') === activeDeck); });
    }

    function clearDetail() {
        selectedRow = null;
        rows().removeClass('selected').attr('aria-selected', 'false');
        $('#reviewsDetail').prop('hidden', true);
        $('#reviewsDetailEmpty').show();
        page.removeClass('is-detail-open');
    }

    function updateNavigation() {
        const visible = filteredRows().toArray();
        const index = selectedRow ? visible.indexOf(selectedRow[0]) : -1;
        $('#btnPrev').prop('disabled', index <= 0);
        $('#btnNext').prop('disabled', index < 0 || index >= visible.length - 1);
    }

    function renderDetail(row) {
        const commitId = row.data('commit-id');
        $('#detailAuthor').text(row.data('author'));
        $('#detailDeck').text(row.data('deck'));
        $('#detailInfo').text(row.data('info') || '').toggle(Boolean(row.data('info')));
        $('#btnFullEditor').attr('href', `/commit/${commitId}`);
        $('#drawerBody').html('<p class="text-secondary">Loading changes...</p>');
        if (request) request.abort();
        request = $.ajax({ url: `/commit_preview/${commitId}`, method: 'GET' })
            .done(function(response) {
                $('#drawerBody').html(response);
                const moreChanges = $('#drawerBody .reviews-more-count').detach();
                $('#detailChangeSummary').empty().append(moreChanges);
                if (window.SharedUI) $('#drawerBody').find('.note-context').each(function() { window.SharedUI.initializeNoteCard(this); });
            })
            .fail(function(xhr, status) { if (status !== 'abort') $('#drawerBody').html('<p class="text-danger">Failed to load this preview.</p>'); });
    }

    function selectRow(row, focus) {
        if (!row || !row.length) { clearDetail(); return; }
        selectedRow = row;
        rows().removeClass('selected').attr('aria-selected', 'false');
        row.addClass('selected').attr('aria-selected', 'true');
        $('#reviewsDetailEmpty').hide();
        $('#reviewsDetail').prop('hidden', false);
        page.addClass('is-detail-open');
        renderDetail(row);
        updateNavigation();
        if (focus) row.trigger('focus');
    }

    function moveSelection(direction, origin) {
        const visible = filteredRows().toArray();
        if (!visible.length) return;
        let index = origin ? visible.indexOf(origin[0]) : (selectedRow ? visible.indexOf(selectedRow[0]) : (direction > 0 ? -1 : visible.length));
        index = Math.max(0, Math.min(visible.length - 1, index + direction));
        selectRow($(visible[index]), true);
    }

    function removeSelectedRow() {
        const visible = filteredRows().toArray();
        const index = selectedRow ? visible.indexOf(selectedRow[0]) : -1;
        const next = $(visible[index + 1] || visible[index - 1]);
        const row = selectedRow;
        row.addClass('removing');
        setTimeout(function() {
            row.remove();
            selectedRow = null;
            clearDetail();
            buildDeckControl();
            refreshList();
            if (next.length && next.closest('body').length) selectRow(next, false);
        }, 190);
    }

    async function actOnSelected(action) {
        if (!selectedRow) return;
        const commitId = selectedRow.data('commit-id');
        const buttons = $('.reviews-action').prop('disabled', true);
        try {
            const response = await fetch(`/${action === 'approve' ? 'ApproveCommit' : 'DenyCommit'}/${commitId}`, {
                method: 'POST', credentials: 'same-origin', redirect: 'manual',
                headers: action === 'deny' ? { 'Content-Type': 'application/json' } : {},
                body: action === 'deny' ? JSON.stringify({ silent: true }) : undefined
            });
            if (response.type !== 'opaqueredirect' && !response.ok) throw new Error(`Request failed: ${response.status}`);
            removeSelectedRow();
        } catch (error) {
            console.error('Review action failed:', error);
            alert('An error occurred while processing this review.');
        } finally { buttons.prop('disabled', false); }
    }

    buildDeckControl();
    refreshList();
    selectRow(filteredRows().first(), false);

    $('#reviewsSearch').on('input', refreshList);
    $('#deckFilterSearch').on('input', function() {
        const query = $(this).val().toLowerCase().trim();
        $('#deckFilterOptions .reviews-deck-option').each(function() { $(this).toggle(!query || !$(this).data('deck') || String($(this).data('deck')).toLowerCase().indexOf(query) !== -1); });
    });
    $('#deckFilterOptions').on('click', '.reviews-deck-option', function() { activeDeck = $(this).data('deck') || ''; updateDeckControl(); refreshList(); });
    list.on('click', '.review-row', function() { selectRow($(this), false); });
    list.on('keydown', '.review-row', function(event) {
        if (event.key === 'ArrowUp' || event.key === 'ArrowDown') { event.preventDefault(); moveSelection(event.key === 'ArrowDown' ? 1 : -1, $(this)); }
        if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); selectRow($(this), false); }
    });
    $('#btnPrev').on('click', function() { moveSelection(-1); });
    $('#btnNext').on('click', function() { moveSelection(1); });
    $('.reviews-action').on('click', function() { actOnSelected($(this).data('review-action')); });
    $('#reviewsBack').on('click', function() { page.removeClass('is-detail-open'); });
});
