/**
 * commit_history.js - Toggle functionality for commit history page
 */
function toggleEvents(noteId, button) {
    var container = document.getElementById('events-' + noteId);
    
    if (container.classList.contains('events-hidden')) {
        container.classList.remove('events-hidden');
        container.classList.add('events-visible');
        button.textContent = 'Hide Details';
    } else {
        container.classList.remove('events-visible');
        container.classList.add('events-hidden');
        button.textContent = 'Show Details';
    }
}

// CSP-compliant event delegation
document.addEventListener('DOMContentLoaded', function() {
    document.addEventListener('click', function(e) {
        var toggleBtn = e.target.closest('.toggle-events-btn');
        if (toggleBtn) {
            var noteId = toggleBtn.getAttribute('data-note-id');
            toggleEvents(noteId, toggleBtn);
        }
    });
});
