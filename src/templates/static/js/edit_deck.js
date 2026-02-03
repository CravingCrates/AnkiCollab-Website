/**
 * edit_deck.js - Edit deck page functionality
 * Reads deck hash from data-deck-hash attribute on the form element
 */
jQuery(document).ready(function() {
    // Initialize Trumbowyg editors
    if (jQuery.fn.trumbowyg) {
        $("#desc-editor").trumbowyg({
            btns: [
                ['viewHTML'],
                ['formatting'],
                ['strong', 'em', 'del'],
                ['unorderedList', 'orderedList'],
                ['removeformat'],
                ['fullscreen']
            ],
            autogrow: true,
            tagsToRemove: ['script'], // Basic security
            removeformatPasted: true, // Clean pasted content
            resetCss: true, // Helps in isolating editor styling
            changeActiveDropdownIcon: true,
        });

        // Load description from JSON data if present
        var descDataElement = document.getElementById('desc-data');
        if (descDataElement) {
            var descHtml = JSON.parse(descDataElement.textContent);
            $('#desc-editor').trumbowyg('html', descHtml);
        }

        $("#changelog-editor").trumbowyg({
            btns: [
                ['fullscreen']
            ],
            tagsToRemove: ['script', 'link'], // Basic security
            removeformatPasted: true, // Clean pasted content
            resetCss: true, // Helps in isolating editor styling
            changeActiveDropdownIcon: true,
            autogrow: true,
        });
    }
});

// Form submission handling
document.addEventListener('DOMContentLoaded', function() {
    var form = document.querySelector('form[data-deck-hash]');
    if (!form) return;
    
    var deckHash = form.dataset.deckHash;

    form.addEventListener('submit', function(event) {
        event.preventDefault();

        var description = $('#desc-editor').trumbowyg('html').replace(/<p><br><\/p>/g, '').trim();
        var isPrivate = document.querySelector('input[name="private"]').checked;
        var preventSubdecks = document.querySelector('input[name="prevent_subdecks"]').checked;
        var restrictNotetypes = document.querySelector('input[name="restrict_notetypes"]').checked;
        var changelog = $('#changelog-editor').trumbowyg('html').trim();
        changelog = changelog.replace(/<\/p>/g, '\n'); // Replace </p> with newline
        changelog = changelog.replace(/<[^>]*>/g, ''); // Remove all other HTML tags
        if (changelog === '\n') { // If the changelog is empty, set it to an empty string
            changelog = '';
        }

        var data = {
            description: description,
            hash: deckHash,
            is_private: isPrivate,
            prevent_subdecks: preventSubdecks,
            restrict_notetypes: restrictNotetypes,
            changelog: changelog
        };

        window.ApiService.apiCall('/EditDeck', 'POST', data);

        // Reset to empty string to prevent accidental double submits
        $('#changelog-editor').trumbowyg('empty');
        swal("Success!", "All changes have been saved!", "success");
    });
});

// Delete deck confirmation
document.addEventListener('DOMContentLoaded', function() {
    var deleteBtn = document.querySelector(".sweet-success-cancel");
    if (!deleteBtn) return;
    
    var form = document.querySelector('form[data-deck-hash]');
    var deckHash = form ? form.dataset.deckHash : '';
    
    deleteBtn.onclick = function () {
        swal(
            {
                title: "Are you sure you want to delete this deck?",
                text: "This action cannot be undone.",
                type: "warning",
                showCancelButton: true,
                confirmButtonColor: "#DD6B55",
                confirmButtonText: "Yes, delete it!",
                cancelButtonText: "No, cancel!",
                closeOnConfirm: false,
                closeOnCancel: false,
            },
            function (e) {
                if (e) {
                    window.location.href = '/DeleteDeck/' + deckHash;
                } else {
                    swal("Cancelled", "Your deck is safe :)", "error");
                }
            }
        );
    };
});
