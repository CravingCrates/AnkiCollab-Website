/**
 * reviews.js - DataTable initialization for reviews list page
 */
$(document).ready(function() {
    // Initialize DataTable with date sorting
    $('#deckOverview').DataTable({
        destroy: true, // Destroy existing DataTable instance
        columnDefs: [
        {
            "targets": 4,
            "render": function ( data, type, row ) {
                if (type === 'sort' || type === 'type') {
                    return moment(data, 'MM/DD/YYYY').format('YYYYMMDD');
                }
                return data;
            }
        }],
        stripeClasses: ['odd', 'even'],
        order: [[4, 'desc']],
        "pageLength": 25,
    });
});
