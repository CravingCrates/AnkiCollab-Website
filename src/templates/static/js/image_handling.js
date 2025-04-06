/**
 * Image handling specific to VISUALIZING the diff.
 * Adds wrappers around images identified within <ins> and <del> tags
 * after the diff HTML is rendered in the DOM.
 */
window.ImageHandler = (function() {

    /**
     * Processes rendered diff HTML to add visual wrappers around added/removed images.
     * Assumes backend provides diff with <ins>/<del> tags around changed blocks.
     * Operates on the already rendered HTML string.
     *
     * @param {string} _originalHtml - Unescaped original HTML (used to identify context, maybe).
     * @param {string} _newHtml - Unescaped new HTML (used to identify context, maybe).
     * @param {string} diffHtml - The rendered diff HTML string containing <ins>, <del>, and <img> tags.
     * @returns {string} - Diff HTML string with added wrappers around diffed images.
     */
    function processHtmlDiffs(_originalHtml, _newHtml, diffHtml) {
        if (typeof diffHtml !== 'string' || diffHtml.trim() === '') {
            return diffHtml;
        }

        try {
            // Use DOMParser to safely parse and manipulate the diff fragment
            const parser = new DOMParser();
            // Wrap in a div to ensure proper parsing of fragments
            const doc = parser.parseFromString(`<div>${diffHtml}</div>`, 'text/html');
            const container = doc.body.firstChild; // Get the container div

            // Process images within <ins> tags
            container.querySelectorAll('ins img').forEach(img => {
                // Check if it's already wrapped (shouldn't happen with clean input, but defensively check)
                if (!img.closest('span.img-diff-wrapper')) {
                    _addImageWrapper(img, 'added');
                }
            });

            // Process images within <del> tags
            container.querySelectorAll('del img').forEach(img => {
                 if (!img.closest('span.img-diff-wrapper')) {
                    _addImageWrapper(img, 'removed');
                }
            });

            // Return the modified inner HTML of the container
            return container.innerHTML;

        } catch (e) {
            console.error("Error processing HTML diffs for image wrappers:", e);
            return diffHtml; // Return original diff HTML on error
        }
    }

    /**
     * Wraps an image element with a diff indicator span.
     * Modifies the passed image element and its parent in the temporary document.
     * @param {HTMLImageElement} img - The image element to wrap.
     * @param {'added' | 'removed'} type - The type of change.
     */
    function _addImageWrapper(img, type) {
        const wrapper = img.ownerDocument.createElement('span'); // Use ownerDocument
        const indicator = img.ownerDocument.createElement('span');

        const wrapperClass = `img-diff-wrapper ${type}`;
        const indicatorClass = `img-diff-indicator ${type}`;
        const labelText = type === 'added' ? 'Added' : 'Removed';

        wrapper.className = wrapperClass;
        indicator.className = indicatorClass;
        indicator.textContent = labelText;

        // Style the image itself
        img.style.maxWidth = '150px';
        img.style.maxHeight = '150px';
        img.style.verticalAlign = 'middle'; // Align better with text if needed
        if (type === 'removed') {
            img.style.opacity = '0.6';
        }

        // Perform the wrapping in the DOM fragment
        const parent = img.parentNode;
        if (parent) {
            parent.replaceChild(wrapper, img); // Replace img with wrapper
            wrapper.appendChild(img);         // Move img inside wrapper
            wrapper.appendChild(indicator);   // Add indicator inside wrapper
        } else {
            console.warn("Image element had no parent during wrapping:", img);
        }
    }

    // --- Public API ---
    return {
        processHtmlDiffs: processHtmlDiffs
    };

})();