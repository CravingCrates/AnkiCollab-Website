/**
 * statistics.js - DataTable initialization for statistics page
 */
$(document).ready(function() {
    $('#deckOverview').DataTable({
        "pageLength": 10
    });
    $('#notesOverview').DataTable({
        "pageLength": 25
    });
});
