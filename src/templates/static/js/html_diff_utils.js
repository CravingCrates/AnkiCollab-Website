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
        try {
            const parser = new DOMParser();
            const doc = parser.parseFromString(html, 'text/html');
            return doc.documentElement.textContent || '';
        } catch (e) {
            console.error("Error unescaping HTML:", e);
            // Fallback for simple cases if DOMParser fails unexpectedly
            const temp = document.createElement('textarea');
            temp.innerHTML = html;
            return temp.value;
        }
    }

    // --- Public API ---
    return {
        unescapeHtml: unescapeHtml
    };
})();