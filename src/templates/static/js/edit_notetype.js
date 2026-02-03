/**
 * edit_notetype.js - Edit notetype page functionality
 * Reads notetype_id from data-notetype-id attribute on #submitForm
 * Reads templates from JSON script tag #templates-data
 */
document.addEventListener('DOMContentLoaded', function() {
    // Parse JSON from script tag to properly handle escaped content
    var templatesScript = document.getElementById('templates-data');
    if (!templatesScript) return;
    
    // Create a temporary element to decode HTML entities
    var tempDiv = document.createElement('div');
    tempDiv.innerHTML = templatesScript.innerHTML;
    var decodedJSON = tempDiv.textContent || tempDiv.innerText;
    
    var TEMPLATES = JSON.parse(decodedJSON);
    
    var form = document.querySelector('#submitForm');
    if (!form) return;
    
    var NOTETYPE_ID = parseInt(form.dataset.notetypeId, 10);

    form.addEventListener('submit', function(event) {
        event.preventDefault();

        // Collect protected field checkbox states
        var protectedMap = {};
        document.querySelectorAll('input[type="checkbox"]').forEach(function(cb) {
            var key = parseInt(cb.name, 10);
            if (!Number.isNaN(key)) {
                protectedMap[key] = cb.checked;
            }
        });

        // Collect updated template front/back content
        var templateData = TEMPLATES.map(function(t) {
            var frontEl = document.getElementById(t.template_id + '-front');
            var backEl = document.getElementById(t.template_id + '-back');
            return {
                template_id: t.template_id,
                name: t.name,
                front: frontEl ? frontEl.value : '',
                back: backEl ? backEl.value : '',
            };
        });

        var payload = {
            items: protectedMap,
            styling: document.querySelector('textarea[name="styling_textarea"]')?.value || '',
            notetype_id: NOTETYPE_ID,
            templates: templateData,
        };

        fetch('/EditNotetype', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload)
        }).then(function(r) {
            if (!r.ok) throw new Error('Save failed with status: ' + r.status);
            var confirmation = document.getElementById('confirmation');
            if (confirmation) {
                confirmation.style.display = 'block';
            }
        }).catch(function(err) {
            console.error('Save error:', err);
            alert('Saving failed. See console for details.');
        });
    });
});
