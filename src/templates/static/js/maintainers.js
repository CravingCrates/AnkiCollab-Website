/**
 * maintainers.js - Maintainer management page
 * Reads deck hash from data-deck-hash attribute on #maintainers-page element
 */
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
                var li = document.createElement('li');
                var label = document.createElement('label');
                var span = document.createElement('span');
                span.textContent = username;
                var removeLink = document.createElement('a');
                removeLink.href = '#';
                removeLink.className = 'fa-solid fa-xmark js-remove-maintainer';
                removeLink.setAttribute('aria-label', 'Remove maintainer ' + username);
                removeLink.setAttribute('role', 'button');
                label.appendChild(span);
                label.appendChild(removeLink);
                li.appendChild(label);
                $('.tdl-content2 ul').append(li);
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
    $('.tdl-new2').on('keydown', function (e) {
        if (e.key === 'Enter' && !e.isComposing && e.keyCode !== 229) {
            e.preventDefault();
            var username = $(this).val().trim();
            if (!username) return;
            addMaintainer(username);
            $(this).val(''); // Clear the input field
        }
    });

    $(document).on("click", ".js-remove-maintainer", function () {
        var username = $(this).siblings("span").text().trim();
        removeMaintainer(username);
        var _li = $(this).parent().parent("li");
        _li.addClass("remove").stop().delay(100).slideUp("fast", function() {
            _li.remove();
        });
        return false;
    });
});
