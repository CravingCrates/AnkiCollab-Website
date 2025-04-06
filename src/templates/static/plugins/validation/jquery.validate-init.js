jQuery.validator.addMethod("pattern", function(value, element, regexp) {
  var re = new RegExp(regexp);
  return this.optional(element) || re.test(value);
}, "Please check your input.");

jQuery.validator.addMethod("nowhitespace", function(value, element) {
  return this.optional(element) || !(/\s/.test(value));
}, "Spaces are not allowed in the username");

jQuery(".form-valide").validate({
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
  "username": {
    required: true,
    minlength: 3,
    maxlength: 30,
    nowhitespace: true,
    pattern: /^[a-zA-Z0-9_-]+$/  // Only allow letters, numbers, underscore, and hyphen
  },
  "password": {
      required: true,
      minlength: 8,
      pattern: /^(?=.*[a-z])(?=.*[A-Z])(?=.*\d).+$/,
  },
  "val-terms": { required: true },
},
messages: {    
  "username": {
    required: "Please enter a username",
    minlength: "Username must be at least 3 characters long",
    maxlength: "Username cannot be longer than 30 characters",
    nowhitespace: "Username cannot contain spaces",
    pattern: "Username can only contain letters, numbers, underscores, and hyphens"
  },
  "password": {
    required: "Please provide a password",
    minlength: "Your password must be at least 8 characters long",
    pattern: "Your password needs a mix of uppercase, lowercase letters, and a number",
  },
  "val-terms": "Please agree to our Terms to continue",
},
});