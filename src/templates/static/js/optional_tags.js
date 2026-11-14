/**
 * optional_tags.js - Optional tags management page
 * Reads deck hash from data-deck-hash attribute on #optional-tags-page element
 */
$(document).ready(function () {
    var pageElement = document.getElementById('optional-tags-page');
    var deckHash = pageElement ? pageElement.dataset.deckHash : '';

    // SweetAlert2 replaces toastr. The mixin mirrors the pattern already used in
    // clipboard.js for the copy-key confirmation.
    var SwalToast = Swal.mixin({
        toast: true,
        position: 'top-end',
        showConfirmButton: false,
        timer: 5000,
        timerProgressBar: true,
        didOpen: function (toast) {
            toast.onmouseenter = Swal.stopTimer;
            toast.onmouseleave = Swal.resumeTimer;
        }
    });

    function toast_error(msg) {
        if (window.a11yAnnounce) { window.a11yAnnounce('Error: ' + msg, 'assertive'); }
        SwalToast.fire({ icon: 'error', title: msg });
    }

    function toast_success(msg) {
        if (window.a11yAnnounce) { window.a11yAnnounce(msg, 'polite'); }
        SwalToast.fire({ icon: 'success', title: msg });
    }

    var ACTION_ADD = 1;
    var ACTION_REMOVE = 0;
    var isProcessing = false;

    function sendData(action, taggroup) {
        if (isProcessing) return;
        isProcessing = true;

        var data = {
            deck: deckHash,
            taggroup: taggroup,
            action: action,
        };

        fetch("/OptionalTags", {
            method: "POST",
            body: JSON.stringify(data),
            headers: {
                "Content-Type": "application/json",
            },
        })
        .then(function(response) {
            if (!response.ok) throw new Error('HTTP ' + response.status);
            return response.text();
        })
        .then(function(text) {
            if (text === 'added') {
                var li = document.createElement('li');
                var label = document.createElement('label');
                var span = document.createElement('span');
                span.textContent = taggroup;
                var removeLink = document.createElement('a');
                removeLink.href = '#';
                removeLink.className = 'fa-solid fa-trash js-remove-tag';
                removeLink.setAttribute('aria-label', 'Remove optional tag ' + taggroup);
                removeLink.setAttribute('role', 'button');
                label.appendChild(span);
                label.appendChild(removeLink);
                li.appendChild(label);
                $('.tdl-content2 ul').append(li);
                toast_success('Optional Tag added');
            }
            else if(text === 'removed') {
                toast_success('Optional Tag removed');
            }
            else {
                toast_error(text);
            }
        })
        .catch(function(error) {
            console.error(error);
            toast_error('An error occurred. Please try again.');
        })
        .finally(function() {
            isProcessing = false;
        });
    }

    function removeTag(taggroup) {
        sendData(ACTION_REMOVE, taggroup);
    }

    function addTag(taggroup) {
        sendData(ACTION_ADD, taggroup);
    }

    // addTag on enter
    $('.tdl-new2').on('keydown', function (e) {
        if (e.key === 'Enter' && !e.isComposing && e.keyCode !== 229) {
            e.preventDefault();
            var taggroup = $(this).val().trim();
            if (!taggroup) return;
            addTag(taggroup);
            $(this).val(''); // Clear the input field
        }
    });

    $(document).on("click", ".js-remove-tag", function () {
        var taggroup = $(this).siblings("span").text().trim();
        removeTag(taggroup);
        var _li = $(this).parent().parent("li");
        _li.addClass("remove").stop().delay(100).slideUp("fast", function() {
            _li.remove();
        });
        return false;
    });
});
