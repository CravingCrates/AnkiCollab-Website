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

        // Deletions view: remove <ins data-diff> elements, convert <del data-diff> to highlighted spans
        var insElements = delContainer.querySelectorAll('ins[data-diff]');
        for (var i = insElements.length - 1; i >= 0; i--) {
            insElements[i].parentNode.removeChild(insElements[i]);
        }
        var delElements = delContainer.querySelectorAll('del[data-diff]');
        for (var i = 0; i < delElements.length; i++) {
            var span = doc.createElement('span');
            span.className = 'diff-highlight-del';
            span.innerHTML = delElements[i].innerHTML;
            delElements[i].parentNode.replaceChild(span, delElements[i]);
        }

        // Insertions view: remove <del data-diff> elements, convert <ins data-diff> to highlighted spans
        var delElements2 = insContainer.querySelectorAll('del[data-diff]');
        for (var i = delElements2.length - 1; i >= 0; i--) {
            delElements2[i].parentNode.removeChild(delElements2[i]);
        }
        var insElements2 = insContainer.querySelectorAll('ins[data-diff]');
        for (var i = 0; i < insElements2.length; i++) {
            var span = doc.createElement('span');
            span.className = 'diff-highlight-ins';
            span.innerHTML = insElements2[i].innerHTML;
            insElements2[i].parentNode.replaceChild(span, insElements2[i]);
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