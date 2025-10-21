jQuery(document).ready(function() {
    if (jQuery.fn.trumbowyg) {
        $(".trumbowyg-editor").trumbowyg({
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
            autogrow: true,

        });
    }
});