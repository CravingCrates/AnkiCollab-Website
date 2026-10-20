/* Reviews keeps selection in the list and renders only the preview excerpt in
   the detail pane; the full editor remains the single destination for edits. */
$(function() {
    const page = $('#reviewsPage');
    const list = $('#reviewsList');
    let selectedRow = null;
    let activeDeck = '';
    let request = null;
    const SORT_OFF = 0, SORT_ASC = 1, SORT_DESC = 2;
    const sortState = { author: SORT_OFF, time: SORT_OFF, info: SORT_OFF };

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

    function sortValue(row, key) {
        if (key === 'time') return String(row.data('timestamp') || '');
        const text = key === 'author' ? String(row.data('author') || '') : String(row.data('info') || '');
        return text.toLowerCase();
    }

    function sortRows(rows) {
        const activeKey = Object.keys(sortState).find(key => sortState[key] !== SORT_OFF);
        if (!activeKey) return rows;
        const direction = sortState[activeKey];
        const arr = rows.toArray();
        arr.sort(function(a, b) {
            const va = sortValue($(a), activeKey);
            const vb = sortValue($(b), activeKey);
            const cmp = va < vb ? -1 : (va > vb ? 1 : 0);
            return direction === SORT_ASC ? cmp : -cmp;
        });
        return $(arr);
    }

    function updateSortButtons() {
        $('.reviews-sort-btn').each(function() {
            const button = $(this);
            const key = button.data('sort');
            const state = sortState[key];
            const icon = button.children('i');
            const active = state !== SORT_OFF;
            button.toggleClass('active', active).attr('aria-pressed', String(active));
            if (key === 'time') {
                icon.attr('class', state === SORT_ASC ? 'fa-solid fa-sort-up' : (state === SORT_DESC ? 'fa-solid fa-sort-down' : 'fa-solid fa-sort'));
            } else {
                icon.attr('class', state === SORT_ASC ? 'fa-solid fa-arrow-down-a-z' : (state === SORT_DESC ? 'fa-solid fa-arrow-up-a-z' : 'fa-solid fa-sort'));
            }
        });
    }

    function refreshList() {
        const visible = sortRows(filteredRows());
        rows().toggle(false);
        visible.each(function() { list.append(this); });
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
    $('.reviews-sort-btn').on('click', function() {
        const key = $(this).data('sort');
        sortState[key] = (sortState[key] + 1) % 3;
        Object.keys(sortState).forEach(function(k) { if (k !== key) sortState[k] = SORT_OFF; });
        updateSortButtons();
        refreshList();
    });
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
