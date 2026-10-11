// Password reveal toggles and the two-step sign-in flow.
//
// `script-src 'self'` blocks inline handlers, so everything is wired with
// addEventListener here.
(function () {
  "use strict";

  function wireToggle(button) {
    var field = document.getElementById(button.getAttribute("data-password-toggle"));

    if (!field) {
      // Nothing to control, so there must be no control on screen either.
      button.hidden = true;
      return;
    }

    button.addEventListener("click", function () {
      var isRevealed = field.type === "text";
      field.type = isRevealed ? "password" : "text";
      button.setAttribute("aria-pressed", String(!isRevealed));
      button.setAttribute("aria-label", isRevealed ? "Show password" : "Hide password");
      button.textContent = isRevealed ? "Show" : "Hide";
    });
  }

  // One question at a time: who, then how. Both steps are in the markup, so a browser
  // without scripting gets the whole form at once.
  function wireSignInSteps() {
    var usernameStep = document.querySelector('[data-login-step="username"]');
    var verifyStep = document.querySelector('[data-login-step="verification"]');
    var continueButton = document.querySelector("[data-login-continue]");
    var usernameField = document.querySelector('input[name="username"]');
    if (!usernameStep || !verifyStep || !continueButton || !usernameField) {
      return;
    }

    var identity = verifyStep.querySelector("[data-login-identity]");
    var changeButton = verifyStep.querySelector("[data-login-change]");
    var stepError = usernameStep.querySelector("[data-login-error]");
    var passkeyButton = verifyStep.querySelector('[data-passkey-action="sign-in"]');
    var revealButton = verifyStep.querySelector("[data-login-password-reveal]");
    var passwordFields = verifyStep.querySelector("[data-login-password-fields]");
    var passwordField = passwordFields
      ? passwordFields.querySelector('input[name="password"]')
      : null;

    function showStepError(message) {
      if (!stepError) {
        return;
      }
      stepError.textContent = message;
      stepError.hidden = !message;
    }

    function focus(field) {
      if (field) {
        field.focus();
      }
    }

    // Asking for the password hands the emphasis over: two filled buttons on one step
    // would leave the user guessing which one is the way in.
    function showPassword(show) {
      if (!passwordFields || passwordFields.hidden === !show) {
        return;
      }
      passwordFields.hidden = !show;
      if (revealButton) {
        revealButton.hidden = show;
      }
      if (passkeyButton) {
        passkeyButton.classList.toggle("auth-btn--primary", !show);
        passkeyButton.classList.toggle("auth-btn--secondary", show);
      }
    }

    function showVerifyStep() {
      // Every visit starts from the passkey, so a password asked for last time is not
      // still open this time.
      showPassword(false);

      if (identity) {
        identity.textContent = usernameField.value.trim();
      }
      usernameStep.hidden = true;
      verifyStep.hidden = false;

      // Where a passkey cannot be used at all there is no action to focus, so ask for
      // the password instead of putting the cursor on a button that would do nothing.
      //
      // The passkey button is focused but not clicked. Pressing Enter on it is what
      // starts the ceremony, and that press is a fresh user activation; firing the
      // ceremony from the same Enter that moved the step would depend on the keydown's
      // activation surviving the round trip that fetches the challenge, which it does
      // not on a slow connection - the browser would refuse the prompt with no
      // `NotAllowedError` the user could make sense of.
      if (passkeyButton && !passkeyButton.hidden) {
        focus(passkeyButton);
      } else {
        showPassword(true);
        focus(passwordField);
      }
    }

    continueButton.addEventListener("click", function () {
      var username = usernameField.value.trim();
      if (!username) {
        showStepError("Enter your username to continue.");
        focus(usernameField);
        return;
      }

      showStepError("");

      // That field is about to be hidden, so the autofill request armed on it can no
      // longer be answered.
      if (window.passkeys && window.passkeys.abandonConditionalSignIn) {
        window.passkeys.abandonConditionalSignIn();
      }

      showVerifyStep();
    });

    // Enter means the same as the button. Without this the form is submitted instead,
    // and the browser then refuses it over a password field it cannot reach: nothing
    // happens, and nothing is said.
    usernameField.addEventListener("keydown", function (event) {
      // Enter while an IME is composing commits the candidate rather than meaning
      // "continue", so it must not move the step mid-word on a Japanese or Chinese
      // keyboard.
      if (event.key === "Enter" && !event.isComposing) {
        event.preventDefault();
        continueButton.click();
      }
    });

    if (revealButton) {
      revealButton.addEventListener("click", function () {
        showPassword(true);
        focus(passwordField);
      });
    }

    // The password is required, so a submission attempted while it is hidden is refused
    // over a field the browser cannot reach, and refused silently. Pressing Enter on the
    // session checkbox is enough to get here.
    if (passwordField) {
      passwordField.addEventListener("invalid", function () {
        showPassword(true);
        focus(passwordField);
      });
    }

    if (changeButton) {
      changeButton.addEventListener("click", function () {
        verifyStep.hidden = true;
        usernameStep.hidden = false;
        usernameField.select();
        focus(usernameField);
      });
    }

    // A step-one message is about what was typed, so it stops being true the moment
    // the field changes.
    usernameField.addEventListener("input", function () {
      showStepError("");
    });
  }

  function wireControls() {
    document.querySelectorAll("[data-password-toggle]").forEach(wireToggle);
    wireSignInSteps();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", wireControls);
  } else {
    wireControls();
  }
})();
