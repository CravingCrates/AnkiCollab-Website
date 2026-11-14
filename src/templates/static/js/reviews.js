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
    let currentDomSortKey = null;
    let currentDomSortDirection = SORT_OFF;

    function initials(name) {
        const parts = String(name || '').trim().split(/\s+/).filter(Boolean);
        return (parts.length > 1 ? parts[0][0] + parts[parts.length - 1][0] : (parts[0] || '?').slice(0, 2)).toUpperCase();
    }

    function topLevelDeck(deck) {
        return String(deck || '').split('::')[0] || '(Unnamed)';
    }

    const rowMetaMap = new WeakMap();
    let rowItems = [];

    const rawRows = list[0] ? list[0].querySelectorAll('.review-row') : [];
    for (let i = 0; i < rawRows.length; i++) {
        const el = rawRows[i];
        const author = el.getAttribute('data-author') || '';
        const deck = el.getAttribute('data-deck') || '';
        const info = el.getAttribute('data-info') || '';
        const timestamp = el.getAttribute('data-timestamp') || '';
        const commitId = el.getAttribute('data-commit-id') || '';
        const topDeck = topLevelDeck(deck);

        const avatar = el.querySelector('.review-avatar');
        if (avatar) {
            avatar.textContent = initials(author);
        }

        const meta = {
            element: el,
            commitId: commitId,
            author: author,
            authorLower: author.toLowerCase(),
            deck: deck,
            deckLower: deck.toLowerCase(),
            info: info,
            infoLower: info.toLowerCase(),
            timestamp: timestamp,
            topDeck: topDeck,
            originalIndex: i
        };
        rowMetaMap.set(el, meta);
        rowItems.push(meta);
    }

    function getRowMeta(el) {
        if (!el) return null;
        let meta = rowMetaMap.get(el);
        if (!meta) {
            const author = el.getAttribute('data-author') || '';
            const deck = el.getAttribute('data-deck') || '';
            const info = el.getAttribute('data-info') || '';
            meta = {
                element: el,
                commitId: el.getAttribute('data-commit-id') || '',
                author: author,
                authorLower: author.toLowerCase(),
                deck: deck,
                deckLower: deck.toLowerCase(),
                info: info,
                infoLower: info.toLowerCase(),
                timestamp: el.getAttribute('data-timestamp') || '',
                topDeck: topLevelDeck(deck),
                originalIndex: rowItems.length
            };
            rowMetaMap.set(el, meta);
        }
        return meta;
    }

    function filteredRows() {
        return list.children('.review-row:not([hidden])');
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
        const activeKey = Object.keys(sortState).find(key => sortState[key] !== SORT_OFF);
        const direction = activeKey ? sortState[activeKey] : SORT_OFF;
        const sortChanged = activeKey !== currentDomSortKey || direction !== currentDomSortDirection;

        if (sortChanged) {
            if (activeKey) {
                const sorted = rowItems.slice().sort(function(a, b) {
                    let cmp = 0;
                    if (activeKey === 'time') {
                        cmp = a.timestamp < b.timestamp ? -1 : (a.timestamp > b.timestamp ? 1 : 0);
                    } else if (activeKey === 'author') {
                        cmp = a.authorLower < b.authorLower ? -1 : (a.authorLower > b.authorLower ? 1 : 0);
                    } else if (activeKey === 'info') {
                        cmp = a.infoLower < b.infoLower ? -1 : (a.infoLower > b.infoLower ? 1 : 0);
                    }
                    if (cmp === 0) return a.originalIndex - b.originalIndex;
                    return direction === SORT_ASC ? cmp : -cmp;
                });
                const fragment = document.createDocumentFragment();
                for (let i = 0; i < sorted.length; i++) {
                    fragment.appendChild(sorted[i].element);
                }
                if (list[0]) list[0].appendChild(fragment);
                currentDomSortKey = activeKey;
                currentDomSortDirection = direction;
            } else if (currentDomSortKey !== null) {
                const sorted = rowItems.slice().sort(function(a, b) {
                    return a.originalIndex - b.originalIndex;
                });
                const fragment = document.createDocumentFragment();
                for (let i = 0; i < sorted.length; i++) {
                    fragment.appendChild(sorted[i].element);
                }
                if (list[0]) list[0].appendChild(fragment);
                currentDomSortKey = null;
                currentDomSortDirection = SORT_OFF;
            }
        }

        const query = ($('#reviewsSearch').val() || '').toLowerCase().trim();
        const activeDeckLower = activeDeck.toLowerCase();
        let visibleCount = 0;

        for (let i = 0; i < rowItems.length; i++) {
            const item = rowItems[i];
            const deckMatch = !activeDeck || item.deckLower === activeDeckLower || item.deckLower.startsWith(activeDeckLower + '::');
            const queryMatch = !query || item.authorLower.includes(query) || item.deckLower.includes(query);
            const isVisible = Boolean(deckMatch && queryMatch);
            item.element.hidden = !isVisible;
            if (isVisible) {
                visibleCount++;
            }
        }

        $('#reviewsEmptyList').prop('hidden', visibleCount !== 0);
        $('#reviewsCount').text(visibleCount);

        const visible = filteredRows();
        if (selectedRow && (!visible.length || selectedRow[0].hidden)) {
            selectRow(visible.first(), false);
        } else if (!visibleCount) {
            clearDetail();
        }
        updateNavigation();
    }

    function buildDeckControl() {
        const names = {};
        for (let i = 0; i < rowItems.length; i++) {
            names[rowItems[i].topDeck] = true;
        }
        const decks = Object.keys(names).sort((a, b) => a.localeCompare(b));
        const visibleDecks = decks.slice(0, 5);
        const $container = $('#deckFilterOptions').empty();
        $container.append($('<button>', {
            class: 'reviews-deck-option' + (!activeDeck ? ' selected' : ''),
            type: 'button',
            'data-deck': '',
            text: 'All decks'
        }));
        visibleDecks.forEach(deck => {
            $container.append($('<button>', {
                class: 'reviews-deck-option' + (deck === activeDeck ? ' selected' : ''),
                type: 'button',
                'data-deck': deck,
                title: deck,
                text: deck
            }));
        });
        $('#deckFilterSearch').toggleClass('visible', decks.length > visibleDecks.length);
    }

    function updateDeckControl() {
        $('#deckFilterOptions .reviews-deck-option').each(function() {
            const deck = this.getAttribute('data-deck') || '';
            $(this).toggleClass('selected', deck === activeDeck);
        });
    }

    function syncMathJaxVisibility() {
        if (window.MathJaxEngine && typeof window.MathJaxEngine.updateToggleVisibility === 'function') {
            window.MathJaxEngine.updateToggleVisibility();
        }
    }

    function clearDetail() {
        selectedRow = null;
        list.children('.review-row.selected').removeClass('selected').attr('aria-selected', 'false');
        $('#drawerBody').empty();
        $('#reviewsDetail').prop('hidden', true);
        $('#reviewsDetailEmpty').show();
        page.removeClass('is-detail-open');
        syncMathJaxVisibility();
    }

    function updateNavigation() {
        const visible = filteredRows().toArray();
        const index = selectedRow ? visible.indexOf(selectedRow[0]) : -1;
        $('#btnPrev').prop('disabled', index <= 0);
        $('#btnNext').prop('disabled', index < 0 || index >= visible.length - 1);
    }

    function renderDetail(row) {
        const meta = getRowMeta(row[0]);
        const commitId = meta ? meta.commitId : row.data('commit-id');
        const author = meta ? meta.author : row.data('author');
        const deck = meta ? meta.deck : row.data('deck');
        const info = meta ? meta.info : (row.data('info') || '');

        $('#detailAuthor').text(author);
        $('#detailDeck').text(deck);
        $('#detailInfo').text(info).toggle(Boolean(info));
        $('#btnFullEditor').attr('href', `/commit/${commitId}`);
        $('#drawerBody').html('<p class="text-secondary">Loading changes...</p>');
        syncMathJaxVisibility();
        if (request) request.abort();
        request = $.ajax({ url: `/commit_preview/${commitId}`, method: 'GET' })
            .done(function(response) {
                const drawerBodyEl = document.getElementById('drawerBody');
                $('#drawerBody').html(response);
                const moreChanges = $('#drawerBody .reviews-more-count').detach();
                $('#detailChangeSummary').empty().append(moreChanges);
                if (window.SharedUI) {
                    $('#drawerBody').find('.note-context').each(function() {
                        window.SharedUI.initializeNoteCard(this, { skipVisibilityUpdate: true, skipMathJax: true });
                    });
                }
                if (window.MathJaxEngine && window.MathJaxEngine.isEnabled() && drawerBodyEl) {
                    window.MathJaxEngine.renderInElement(drawerBodyEl);
                }
                syncMathJaxVisibility();
            })
            .fail(function(xhr, status) {
                if (status !== 'abort') {
                    $('#drawerBody').html('<p class="text-danger">Failed to load this preview.</p>');
                    syncMathJaxVisibility();
                }
            });
    }

    function selectRow(row, focus) {
        const $row = row && (row instanceof $ ? row : $(row));
        if (!$row || !$row.length) { clearDetail(); return; }
        selectedRow = $row;
        list.children('.review-row.selected').removeClass('selected').attr('aria-selected', 'false');
        $row.addClass('selected').attr('aria-selected', 'true');
        $('#reviewsDetailEmpty').hide();
        $('#reviewsDetail').prop('hidden', false);
        page.addClass('is-detail-open');
        renderDetail($row);
        updateNavigation();
        if (focus) $row.trigger('focus');
    }

    function moveSelection(direction, origin) {
        const visible = filteredRows().toArray();
        if (!visible.length) return;
        const originEl = origin ? (origin[0] || origin) : null;
        let index = originEl ? visible.indexOf(originEl) : (selectedRow ? visible.indexOf(selectedRow[0]) : (direction > 0 ? -1 : visible.length));
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
            const rowEl = row[0];
            row.remove();
            rowItems = rowItems.filter(item => item.element !== rowEl);
            selectedRow = null;
            clearDetail();
            buildDeckControl();
            refreshList();
            if (next.length && next.closest('body').length) selectRow(next, false);
        }, 190);
    }

    async function actOnSelected(action) {
        if (!selectedRow) return;
        const meta = getRowMeta(selectedRow[0]);
        const commitId = meta ? meta.commitId : selectedRow.data('commit-id');
        const buttons = $('.reviews-action').prop('disabled', true);
        try {
            const response = await fetch(`/${action === 'approve' ? 'ApproveCommit' : 'DenyCommit'}/${commitId}`, {
                method: 'POST', credentials: 'same-origin', redirect: 'manual',
                headers: action === 'deny' ? { 'Content-Type': 'application/json' } : {},
                body: action === 'deny' ? JSON.stringify({ silent: true }) : undefined
            });
            if (response.type !== 'opaqueredirect' && !response.ok) throw new Error(`Request failed: ${response.status}`);
            try { sessionStorage.removeItem(`commit_select::${commitId}`); } catch (e) {}
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
        $('#deckFilterOptions .reviews-deck-option').each(function() {
            const optionDeck = (this.getAttribute('data-deck') || '').toLowerCase();
            $(this).toggle(!query || !optionDeck || optionDeck.indexOf(query) !== -1);
        });
    });
    $('#deckFilterOptions').on('click', '.reviews-deck-option', function() {
        activeDeck = this.getAttribute('data-deck') || '';
        updateDeckControl();
        refreshList();
    });
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
