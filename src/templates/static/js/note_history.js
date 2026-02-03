/**
 * note_history.js - Filter and UI interactions for note history page
 */
function toggleFilter(buttonElement) {
    var button = buttonElement || document.getElementById('toggleFilterBtn');
    var menu = document.getElementById('filterMenu');
    
    if (menu.classList.contains('show')) {
        menu.classList.remove('show');
    } else {
        // Position the menu relative to the button
        var buttonRect = button.getBoundingClientRect();
        var menuTop = buttonRect.bottom + 8;
        var menuRight = window.innerWidth - buttonRect.right;
        
        // Ensure menu doesn't go off screen
        menu.style.top = Math.max(8, menuTop) + 'px';
        menu.style.right = Math.max(8, Math.min(menuRight, window.innerWidth - 340)) + 'px';
        
        // On small screens, center the menu
        if (window.innerWidth < 768) {
            menu.style.right = '8px';
            menu.style.left = '8px';
            menu.style.minWidth = 'auto';
        }
        
        menu.classList.add('show');
    }
}

function toggleFieldDiff(element) {
    element.classList.toggle('expanded');
}

function toggleSnapshot(id) {
    var content = document.getElementById('snapshot-' + id);
    content.classList.toggle('show');
}

function applyFilters() {
    var typeCheckboxes = document.querySelectorAll('input[name="eventType"]:checked');
    var actorCheckboxes = document.querySelectorAll('input[name="actor"]:checked');
    
    var selectedTypes = Array.from(typeCheckboxes).map(function(cb) { return cb.value; });
    var selectedActors = Array.from(actorCheckboxes).map(function(cb) { return cb.value; });
    
    // Hide all groups initially
    document.querySelectorAll('[data-group]').forEach(function(group) {
        group.classList.add('hidden');
    });
    
    // Show groups and events that match filters
    document.querySelectorAll('[data-event-type]').forEach(function(event) {
        var eventType = event.getAttribute('data-event-type');
        var eventActor = event.getAttribute('data-event-actor') || 'Anonymous';
        
        var typeMatch = selectedTypes.length === 0 || selectedTypes.includes(eventType);
        var actorMatch = selectedActors.length === 0 || selectedActors.includes(eventActor);
        
        if (typeMatch && actorMatch) {
            event.classList.remove('hidden');
            // Show the parent group
            var group = event.closest('[data-group]');
            if (group) group.classList.remove('hidden');
        } else {
            event.classList.add('hidden');
        }
    });
    
    // Hide groups with no visible events
    document.querySelectorAll('[data-group]').forEach(function(group) {
        var visibleEvents = group.querySelectorAll('[data-event-type]:not(.hidden)');
        if (visibleEvents.length === 0) {
            group.classList.add('hidden');
        }
    });
    
    toggleFilter();
}

function clearFilters() {
    document.querySelectorAll('input[type="checkbox"]').forEach(function(cb) { 
        cb.checked = false; 
    });
    document.querySelectorAll('[data-group], [data-event-type]').forEach(function(el) {
        el.classList.remove('hidden');
    });
    toggleFilter();
}

// Close filter menu when clicking outside
document.addEventListener('click', function(event) {
    var filterDropdown = document.querySelector('.filter-dropdown');
    var filterMenu = document.getElementById('filterMenu');
    
    if (filterDropdown && filterMenu && !filterDropdown.contains(event.target)) {
        filterMenu.classList.remove('show');
    }
});
// CSP-compliant event delegation (replaces inline onclick handlers)
document.addEventListener('DOMContentLoaded', function() {
    // Toggle filter button
    var toggleFilterBtn = document.getElementById('toggleFilterBtn');
    if (toggleFilterBtn) {
        toggleFilterBtn.addEventListener('click', function(e) {
            e.stopPropagation();
            toggleFilter(this);
        });
    }
    
    // Clear filters button
    var clearFiltersBtn = document.getElementById('clearFiltersBtn');
    if (clearFiltersBtn) {
        clearFiltersBtn.addEventListener('click', clearFilters);
    }
    
    // Apply filters button
    var applyFiltersBtn = document.getElementById('applyFiltersBtn');
    if (applyFiltersBtn) {
        applyFiltersBtn.addEventListener('click', applyFilters);
    }
    
    // Snapshot headers - use event delegation for dynamically generated content
    document.addEventListener('click', function(e) {
        var snapshotHeader = e.target.closest('.snapshot-header[data-snapshot-id]');
        if (snapshotHeader) {
            var snapshotId = snapshotHeader.getAttribute('data-snapshot-id');
            toggleSnapshot(snapshotId);
        }
    });
});