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
        if (typeof html !== 'string') return '';
        // Use DOMParser for robust unescaping
        const temp = document.createElement('textarea');
        temp.innerHTML = html;
        return temp.value;
    }

    // --- Public API ---
    return {
        unescapeHtml: unescapeHtml
    };
})();