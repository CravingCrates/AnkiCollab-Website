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

    // --- Public API ---
    return {
        unescapeHtml: unescapeHtml
    };
})();