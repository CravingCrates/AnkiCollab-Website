/**
 * optional_tags.js - Optional tags management page
 * Reads deck hash from data-deck-hash attribute on #optional-tags-page element
 */
$(document).ready(function () {
    var pageElement = document.getElementById('optional-tags-page');
    var deckHash = pageElement ? pageElement.dataset.deckHash : '';

    function toast_error(msg) {
        toastr.error(msg, "Oh no!", {
            positionClass: "toast-top-right",
            timeOut: 5e3,
            closeButton: !0,
            debug: !1,
            newestOnTop: !0,
            progressBar: !0,
            preventDuplicates: !0,
            onclick: null,
            showDuration: "300",
            hideDuration: "1000",
            extendedTimeOut: "1000",
            showEasing: "swing",
            hideEasing: "linear",
            showMethod: "fadeIn",
            hideMethod: "fadeOut",
            tapToDismiss: !1,
        });
    }

    function toast_success(msg) {
        toastr.success(msg, "Success!", {
            timeOut: 5e3,
            closeButton: !0,
            debug: !1,
            newestOnTop: !0,
            progressBar: !0,
            positionClass: "toast-top-right",
            preventDuplicates: !0,
            onclick: null,
            showDuration: "300",
            hideDuration: "1000",
            extendedTimeOut: "1000",
            showEasing: "swing",
            hideEasing: "linear",
            showMethod: "fadeIn",
            hideMethod: "fadeOut",
            tapToDismiss: !1,
        });
    }

    var ACTION_ADD = 1;
    var ACTION_REMOVE = 0;

    function sendData(action, taggroup) {
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
        .then(function(response) { return response.text(); })
        .then(function(text) {
            if (text === 'added') {
                $('.tdl-content2 ul').append(
                    '<li>' +
                    '<label>' +
                    '<span>' + taggroup + '</span>' +
                    '<a href="#" class="ti-trash"></a>' +
                    '</label>' +
                    '</li>'
                );
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
        });
    }

    function removeTag(taggroup) {
        sendData(ACTION_REMOVE, taggroup);
    }

    function addTag(taggroup) {
        sendData(ACTION_ADD, taggroup);
    }

    // addTag on enter
    $('.tdl-new2').on('keypress', function (e) {
        var code = (e.keyCode ? e.keyCode : e.which);
        if (code == 13) { // Enter key
            e.preventDefault();
            var taggroup = $(this).val();
            addTag(taggroup);
            $(this).val(''); // Clear the input field
        }
    });

    $(document).on("click", ".ti-trash", function () {
        var taggroup = $(this).siblings("span").text().trim();
        removeTag(taggroup);
        var _li = $(this).parent().parent("li");
        _li.addClass("remove").stop().delay(100).slideUp("fast", function() {
            _li.remove();
        });
        return false;
    });
});
