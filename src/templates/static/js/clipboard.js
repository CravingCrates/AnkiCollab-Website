/**
 * clipboard.js - Shared clipboard utility
 */
function CopyToClipboard(button) {
    var text = button.innerText;
    navigator.clipboard.writeText(text).then(function() {
        console.log('Copying to clipboard was successful!');
    }, function(err) {
        console.error('Could not copy text: ', err);
    });
}
// CSP-compliant event delegation
document.addEventListener('DOMContentLoaded', function() {
    document.addEventListener('click', function(e) {
        var copyBtn = e.target.closest('.copy-to-clipboard-btn');
        if (copyBtn) {
            CopyToClipboard(copyBtn);
        }
    });
});