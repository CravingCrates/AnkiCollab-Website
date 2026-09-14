document.addEventListener('DOMContentLoaded', function() {
    document.addEventListener('click', function(e) {
        var button = e.target.closest('.btn-copy-key');
        if (!button || !navigator.clipboard || !window.isSecureContext) {
            return;
        }

        var originalText = button.textContent.trim();
        navigator.clipboard.writeText(button.dataset.subscriptionKey).then(function() {
            button.textContent = '\u2713 Copied';
            button.classList.add('is-copied');
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
                html: '<p class="sheet-text">Open Anki Desktop and navigate to: <br><span class="sheet-path">AnkiCollab → Manage Subscriptions</span> to subscribe.</p>'
            });
            window.setTimeout(function() {
                button.textContent = originalText;
                button.classList.remove('is-copied');
            }, 1800);
        });
    });
});