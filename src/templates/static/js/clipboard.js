document.addEventListener('DOMContentLoaded', function() {
    var timers = new WeakMap();
    var labels = new WeakMap();

    function scheduleLabelReset(button) {
        clearTimeout(timers.get(button));
        timers.set(button, window.setTimeout(function() {
            button.textContent = labels.get(button) || 'Copy';
            button.classList.remove('is-copied', 'is-copy-failed');
        }, 1800));
    }

    function showCopyError(button) {
        button.textContent = 'Copy failed';
        button.classList.remove('is-copied');
        button.classList.add('is-copy-failed');
        if (window.a11yAnnounce) {
            window.a11yAnnounce('Failed to copy subscription key to clipboard.', 'assertive');
        }
        scheduleLabelReset(button);
    }

    document.addEventListener('click', function(e) {
        var button = e.target.closest('.btn-copy-key');
        if (!button) {
            return;
        }

        if (!labels.has(button)) {
            labels.set(button, button.textContent ? button.textContent.trim() : 'Copy');
        }

        var value = button.dataset.subscriptionKey;
        if (!value || !navigator.clipboard || !window.isSecureContext) {
            showCopyError(button);
            return;
        }

        navigator.clipboard.writeText(value).then(function() {
            button.textContent = '\u2713 Copied';
            button.classList.remove('is-copy-failed');
            button.classList.add('is-copied');
            if (window.a11yAnnounce) {
                window.a11yAnnounce('Subscription key copied to clipboard.', 'polite');
            }
            Swal.mixin({
                toast: true,
                position: 'top-end',
                showConfirmButton: false,
                timer: 5000,
                timerProgressBar: true,
                didOpen: function(toast) {
                    toast.onmouseenter = Swal.stopTimer;
                    toast.onmouseleave = Swal.resumeTimer;
                }
            }).fire({
                icon: 'success',
                html: '<p class="sheet-text">Open Anki Desktop and navigate to: <br><span class="sheet-path">AnkiCollab \u2192 Manage Subscriptions</span> to subscribe.</p>'
            });
            scheduleLabelReset(button);
        }).catch(function(error) {
            console.error('Clipboard copy failed:', error);
            showCopyError(button);
        });
    });
});