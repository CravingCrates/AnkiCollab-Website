/**
 * Utility functions for HTML Diff handling.
 */
window.HtmlDiffUtils = (function() {

    /**
     * Unescapes HTML entities in a string.
     * @param {string} html - The HTML string with entities.
     * @returns {string} - The unescaped HTML string.
     */
    function unescapeHtml(html) {
        if (html === undefined || html === null) return '';
        // jQuery's data() can coerce numbers/booleans; normalize to string before unescaping
        const value = (typeof html === 'string') ? html : String(html);
        const temp = document.createElement('textarea');
        temp.innerHTML = value;
        return temp.value;
    }

    const BLOCK_ELEMENTS_SELECTOR =
        'ul, ol, li, div, p, table, thead, tbody, tfoot, tr, td, th, blockquote, pre, hr, h1, h2, h3, h4, h5, h6, dl, dt, dd, figure, section, article';

    /**
     * Replaces a diff marker (<ins data-diff> or <del data-diff>) with a
     * semantically valid highlight container:
     * - Direct <li> / nested list children inside <ul>/<ol> are promoted so
     *   <ul>/<ol> never contains an invalid wrapper element.
     * - Diffs containing block elements use <div class="... diff-highlight-block">.
     * - Pure inline diffs keep <span> so inline text and MathJax diffs work.
     *
     * @param {Document} doc
     * @param {Element} diffEl
     * @param {string} highlightClass
     */
    function replaceDiffElement(doc, diffEl, highlightClass) {
        var parent = diffEl.parentNode;
        if (!parent) return;

        if (parent.nodeType === 1 && (parent.tagName === 'UL' || parent.tagName === 'OL')) {
            var hasListElement = false;
            var onlyListChildren = true;
            for (var c = 0; c < diffEl.childNodes.length; c++) {
                var childNode = diffEl.childNodes[c];
                if (childNode.nodeType === 1) {
                    var childTag = childNode.tagName;
                    if (childTag === 'LI' || childTag === 'UL' || childTag === 'OL') {
                        hasListElement = true;
                    } else {
                        onlyListChildren = false;
                        break;
                    }
                } else if (childNode.nodeType === 3 && childNode.nodeValue.trim() === '') {
                    continue;
                } else {
                    onlyListChildren = false;
                    break;
                }
            }

            if (hasListElement && onlyListChildren) {
                while (diffEl.firstChild) {
                    var moved = diffEl.firstChild;
                    if (moved.nodeType === 1) {
                        if (moved.tagName === 'LI') {
                            moved.classList.add(highlightClass);
                        } else if (moved.tagName === 'UL' || moved.tagName === 'OL') {
                            moved.classList.add(highlightClass, 'diff-highlight-block');
                        }
                    }
                    parent.insertBefore(moved, diffEl);
                }
                parent.removeChild(diffEl);
                return;
            }
        }

        // Pure inline diffs stay <span> (mathjax_engine.js relies on SPAN for inline diff flattening);
        // block-containing diffs use <div> so inline boxes don't split around <ul>/<ol>/<p>.
        var wrapper = diffEl.querySelector(BLOCK_ELEMENTS_SELECTOR)
            ? doc.createElement('div')
            : doc.createElement('span');
        wrapper.className = wrapper.tagName === 'DIV'
            ? highlightClass + ' diff-highlight-block'
            : highlightClass;

        while (diffEl.firstChild) {
            wrapper.appendChild(diffEl.firstChild);
        }
        parent.replaceChild(wrapper, diffEl);
    }

    /**
     * Splits inline diff HTML (containing <ins>/<del> tags) into two separate views:
     * - deletions: the old text with removed words highlighted (for left column)
     * - insertions: the new text with added words highlighted (for right column)
     *
     * @param {string} diffHtml - HTML string containing <ins> and <del> tags.
     * @returns {{ deletions: string, insertions: string }}
     */
    function splitDiff(diffHtml) {
        if (!diffHtml || typeof diffHtml !== 'string' || diffHtml.trim() === '') {
            return { deletions: '', insertions: '' };
        }

        var parser = new DOMParser();

        // Parse once, clone for each view
        var doc = parser.parseFromString('<div>' + diffHtml + '</div>', 'text/html');
        var delContainer = doc.body.firstChild.cloneNode(true);
        var insContainer = doc.body.firstChild.cloneNode(true);

        // Deletions view: remove <ins data-diff> elements, convert <del data-diff> to highlight wrappers
        var insElements = delContainer.querySelectorAll('ins[data-diff]');
        for (var i = insElements.length - 1; i >= 0; i--) {
            insElements[i].parentNode.removeChild(insElements[i]);
        }
        var delElements = delContainer.querySelectorAll('del[data-diff]');
        for (var i = delElements.length - 1; i >= 0; i--) {
            replaceDiffElement(doc, delElements[i], 'diff-highlight-del');
        }

        // Insertions view: remove <del data-diff> elements, convert <ins data-diff> to highlight wrappers
        var delElements2 = insContainer.querySelectorAll('del[data-diff]');
        for (var i = delElements2.length - 1; i >= 0; i--) {
            delElements2[i].parentNode.removeChild(delElements2[i]);
        }
        var insElements2 = insContainer.querySelectorAll('ins[data-diff]');
        for (var i = insElements2.length - 1; i >= 0; i--) {
            replaceDiffElement(doc, insElements2[i], 'diff-highlight-ins');
        }

        return {
            deletions: delContainer.innerHTML,
            insertions: insContainer.innerHTML
        };
    }

    // --- Public API ---
    return {
        unescapeHtml: unescapeHtml,
        splitDiff: splitDiff
    };
})();