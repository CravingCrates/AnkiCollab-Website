/**
 * profile.js - Password validation and account management
 */
// Password validation
jQuery.validator.addMethod("pattern", function(value, element, regexp) {
    var re = new RegExp(regexp);
    return this.optional(element) || re.test(value);
}, "Please check your input.");

jQuery(".form-valide-password").validate({
    ignore: [],
    errorClass: "invalid-feedback animated fadeInDown",
    errorElement: "div",
    errorPlacement: function (e, a) {
        jQuery(a).parents(".form-group").append(e);
    },
    highlight: function (e) {
        jQuery(e)
            .closest(".form-group")
            .removeClass("is-invalid")
            .addClass("is-invalid");
    },
    success: function (e) {
        jQuery(e).closest(".form-group").removeClass("is-invalid"),
            jQuery(e).remove();
    },
    rules: {
        "current_password": {
            required: true
        },
        "new_password": {
            required: true,
            minlength: 8,
            pattern: /^(?=.*[a-z])(?=.*[A-Z])(?=.*\d).+$/,
        },
        "confirm_password": {
            required: true,
            equalTo: "#new_password"
        },
    },
    messages: {
        "current_password": {
            required: "Please enter your current password"
        },
        "new_password": {
            required: "Please provide a new password",
            minlength: "Your password must be at least 8 characters long",
            pattern: "Your password needs a mix of uppercase, lowercase letters, and a number",
        },
        "confirm_password": {
            required: "Please confirm your new password",
            equalTo: "Passwords do not match"
        },
    },
});

// Delete account confirmation
document.addEventListener('DOMContentLoaded', function() {
    var deleteBtn = document.getElementById('deleteAccountBtn');
    if (deleteBtn) {
        deleteBtn.addEventListener('click', function() {
            swal({
                title: "Are you sure?",
                text: "This will permanently delete your account and all associated data. This action cannot be undone!",
                type: "warning",
                showCancelButton: true,
                confirmButtonClass: "btn-danger",
                confirmButtonText: "Yes, delete my account",
                cancelButtonText: "Cancel",
                closeOnConfirm: false
            }, function(isConfirm) {
                if (isConfirm) {
                    window.location.href = "/profile/delete-account";
                }
            });
        });
    }
});
