/**
 * decks.js - DataTable initialization for decks overview page
 */
$(document).ready(function() {
    // Custom sorting for notes column (handles "k" suffix like "1.5k")
    $.fn.dataTable.ext.type.order['notes-pre'] = function(data) {
        // Extract the actual number from the <a> tag
        const match = data.match(/>(\d+k?)</);
        if (!match) return 0;
        
        const value = match[1];
        
        if (value.endsWith('k')) {
            return parseFloat(value.slice(0, -1)) * 1000;
        }
        // For regular numbers, just convert to float
        return parseFloat(value) || 0;
    };

    // Initialize DataTable with the custom sorting
    $('#deckOverview').DataTable({
        destroy: true, // Destroy existing DataTable instance
        columnDefs: [
        {
            "targets": 0,
            "render": function ( data, type, row ) {
                if (type === 'sort' || type === 'type') {
                    return moment(data, 'MM/DD/YYYY').format('YYYYMMDD');
                }
                return data;
            }
        },
        {
            targets: 3, // Notes column (0-based index)
            type: 'notes'
        }],
        stripeClasses: ['odd', 'even'],
        order: [[4, 'desc']],
        "pageLength": 25,
    });
});
