jQuery.validator.addMethod("pattern", function(value, element, regexp) {
    var re = new RegExp(regexp);
    return this.optional(element) || re.test(value);
}, "Please check your input.");

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
    "email": { required: !0, email: !0 },
    "password": {
        required: !0,
        minlength: 8,
        pattern: /^(?=.*[a-z])(?=.*[A-Z])(?=.*\d).+$/,
    },
    "val-terms": { required: !0 },
  },
  messages: {    
    "email": "Please enter a valid email address",
    "password": {
      required: "Please provide a password",
      minlength: "Your password must be at least 8 characters long",
      pattern: "Your password needs a mix of uppercase, lowercase letters, and a number",
    },
    "val-terms": "Please agree to our Terms of Service",
  },
});
