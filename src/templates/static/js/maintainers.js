/**
 * maintainers.js - Maintainer management page
 * Reads deck hash from data-deck-hash attribute on #maintainers-page element
 */
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

$(document).ready(function () {
    var pageElement = document.getElementById('maintainers-page');
    var deckHash = pageElement ? pageElement.dataset.deckHash : '';
    
    var ACTION_ADD = 1;
    var ACTION_REMOVE = 0;

    function sendData(action, username) {
        var data = {
            deck: deckHash,
            username: username,
            action: action,
        };

        fetch("/Maintainers", {
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
                    '<span>' + username + '</span>' +
                    '<a href="#" class="ti-close"></a>' +
                    '</label>' +
                    '</li>'
                );
                toast_success('Maintainer added');
            }
            else if(text === 'removed') {
                toast_success('Maintainer removed');
            }
            else {
                toast_error('Invalid username');
            }
        })
        .catch(function(error) {
            console.error(error);
            toast_error('An error occurred. Please try again.');
        });
    }

    function removeMaintainer(username) {
        sendData(ACTION_REMOVE, username);
    }

    function addMaintainer(username) {
        sendData(ACTION_ADD, username);
    }

    // addMaintainer
    $('.tdl-new2').on('keypress', function (e) {
        var code = (e.keyCode ? e.keyCode : e.which);
        if (code == 13) { // Enter key
            e.preventDefault();
            var username = $(this).val();
            addMaintainer(username);
            $(this).val(''); // Clear the input field
        }
    });

    $(document).on("click", ".ti-close", function () {
        var username = $(this).siblings("span").text().trim();
        removeMaintainer(username);
        var _li = $(this).parent().parent("li");
        _li.addClass("remove").stop().delay(100).slideUp("fast", function() {
            _li.remove();
        });
        return false;
    });
});
