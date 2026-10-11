(function () {
  "use strict";

  function b64urlToBuffer(value) {
    const normalised = value.replace(/-/g, "+").replace(/_/g, "/");
    const padding = normalised.length % 4 === 0 ? "" : "=".repeat(4 - (normalised.length % 4));
    const binary = window.atob(normalised + padding);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
    return bytes.buffer;
  }

  function bufferToB64url(buffer) {
    const bytes = new Uint8Array(buffer);
    let binary = "";
    for (let i = 0; i < bytes.length; i += 1) binary += String.fromCharCode(bytes[i]);
    return window.btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  }

  function showError(message) {
    const target = document.getElementById("passkey-error");
    if (target) {
      target.textContent = message;
      target.hidden = false;
    } else {
      window.alert(message);
    }
  }

  function clearError() {
    const target = document.getElementById("passkey-error");
    if (target) {
      target.textContent = "";
      target.hidden = true;
    }
  }

  function isSupported() {
    return (
      typeof window.PublicKeyCredential !== "undefined" &&
      typeof navigator.credentials !== "undefined"
    );
  }

  function optionsFrom(start) {
    const options = start.publicKey;
    if (!options) {
      throw new Error("unexpected ceremony response shape");
    }
    return options;
  }

  function unlockName() {
    const ua = navigator.userAgent;
    if (/iPhone|iPod/.test(ua)) return "Face ID or Touch ID";
    // iPadOS 13 and later report themselves as a Mac. The touch points are what tell a
    // tablet from a laptop, and a tablet answers to its own two names.
    if (/iPad/.test(ua) || (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1)) {
      return "Face ID or Touch ID";
    }
    if (/Android/.test(ua)) return "your fingerprint or screen lock";
    if (/Macintosh|Mac OS X/.test(ua)) return "Touch ID";
    if (/Windows/.test(ua)) return "Windows Hello";
    return "";
  }

  
  const PLATFORM_NAME_FALLBACK = "device biometrics or security key";
  const ACTION_NAME_FALLBACK = "biometrics or passkey";

  function assertionToJson(credential) {
    const response = credential.response;
    return {
      id: credential.id,
      rawId: bufferToB64url(credential.rawId),
      type: credential.type,
      response: {
        clientDataJSON: bufferToB64url(response.clientDataJSON),
        authenticatorData: bufferToB64url(response.authenticatorData),
        signature: bufferToB64url(response.signature),
        userHandle: response.userHandle ? bufferToB64url(response.userHandle) : null,
      },
      clientExtensionResults: credential.getClientExtensionResults(),
    };
  }

  function attestationToJson(credential) {
    const response = credential.response;
    return {
      id: credential.id,
      rawId: bufferToB64url(credential.rawId),
      type: credential.type,
      response: {
        clientDataJSON: bufferToB64url(response.clientDataJSON),
        attestationObject: bufferToB64url(response.attestationObject),
        transports: typeof response.getTransports === "function" ? response.getTransports() : [],
      },
      clientExtensionResults: credential.getClientExtensionResults(),
    };
  }

  async function postJson(url, body) {
    const response = await fetch(url, {
      method: "POST",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });
    if (!response.ok) {
      const error = new Error("request failed: " + response.status);
      error.status = response.status;
      throw error;
    }
    return response.json();
  }

  // The username the sign-in form holds, which is what scopes every ceremony here.
  function usernameFromForm() {
    const field = document.querySelector('input[name="username"]');
    return field ? field.value.trim() : "";
  }

  function focusUsernameField() {
    const field = document.querySelector('input[name="username"]');
    if (field) {
      field.focus();
    }
  }

  // Opens the password field by pressing the control that owns it, so the step's state
  // stays in one place.
  function revealPasswordField() {
    const reveal = document.querySelector("[data-login-password-reveal]");
    if (reveal) {
      reveal.click();
      return;
    }
    // No control to press (a page that renders the fields with no reveal step): open
    // them directly, so a failed passkey sign-in is never left with nowhere to type.
    const fields = document.getElementById("login-password-fields");
    if (fields) {
      fields.hidden = false;
    }
  }

  // The password is the way out of a failed passkey sign-in, so the cursor is put in it
  // rather than left for the user to find. A no-op while the field is still hidden.
  function focusPasswordField() {
    const field = document.querySelector('input[name="password"]');
    if (field) {
      field.focus();
    }
  }

  // The box sits with the passkey button, but the choice is about the session, so both
  // ways in read it.
  function keepSignedIn() {
    const box = document.querySelector('input[name="cookie"]');
    return Boolean(box && box.checked);
  }

  // Compared by origin rather than by prefix: "/\evil.com" and "/<TAB>/evil.com" both
  // start with a single slash and normalise to "//evil.com", which is another host.
  function afterSignIn() {
    const field = document.querySelector('input[name="next"]');
    const target = field && field.value ? field.value : "/";
    try {
      const url = new URL(target, window.location.origin);
      return url.origin === window.location.origin
        ? url.pathname + url.search + url.hash
        : "/";
    } catch (error) {
      return "/";
    }
  }

  // The step-up password proves the person at the keyboard knows it, rather than merely
  // holding a session cookie.
  function stepUpPassword() {
    const field = document.getElementById("passkey-password");
    return field ? field.value : "";
  }

  // The welcome page omits that field while the signup that minted the session is itself
  // proof of the password. No field means the password is left out, not sent empty.
  function hasPasswordField() {
    return document.getElementById("passkey-password") !== null;
  }

  // Builds the step-up field when the page has none, which is the onboarding page's shape
  // while the session is fresh. A tab left open past the step-up window is then asked for
  // a password there is nowhere to type; the field is added, in the same markup the
  // server renders, rather than dead-ending the user on a sign-out and a fresh sign-in.
  function ensurePasswordField() {
    const existing = document.getElementById("passkey-password");
    if (existing) {
      return existing;
    }

    const anchor =
      document.querySelector('[data-passkey-action="add-onboarding"]') ||
      document.querySelector('[data-passkey-action="add"]');
    if (!anchor || !anchor.parentNode) {
      return null;
    }

    const wrapper = document.createElement("div");
    wrapper.className = "auth-password";

    const label = document.createElement("label");
    label.className = "visually-hidden";
    label.setAttribute("for", "passkey-password");
    label.textContent = "Your password";

    const field = document.createElement("input");
    field.type = "password";
    field.className = "form-control";
    field.id = "passkey-password";
    field.placeholder = "Your password, to confirm it's you";
    field.autocomplete = "current-password";

    wrapper.appendChild(label);
    wrapper.appendChild(field);
    anchor.parentNode.insertBefore(wrapper, anchor);
    return field;
  }

  // A ceremony is not re-entrant: two concurrent `credentials.get` calls are refused, and
  // a double click would spend two challenges.
  let signInInFlight = false;

  // One scoped ceremony: the server answers with the account's credential ids, so the
  // authenticator is told exactly what it may offer.
  async function runSignInCeremony(username) {
    const start = await postJson("/login/passkey/start", { username });
    const publicKey = optionsFrom(start);
    publicKey.challenge = b64urlToBuffer(publicKey.challenge);
    if (Array.isArray(publicKey.allowCredentials)) {
      publicKey.allowCredentials = publicKey.allowCredentials.map((entry) => ({
        ...entry,
        id: b64urlToBuffer(entry.id),
      }));
    }

    const credential = await navigator.credentials.get({ publicKey });
    await postJson("/login/passkey/finish", {
      challenge_id: start.challenge_id,
      persistent: keepSignedIn(),
      credential: assertionToJson(credential),
    });
  }

  // The button on the second sign-in step. A username is already given by then, so this
  // always runs the scoped ceremony.
  async function signInWithPasskey() {
    if (signInInFlight) {
      return;
    }
    clearError();

    const username = usernameFromForm();
    if (!username) {
      focusUsernameField();
      showError("Enter your username first.");
      return;
    }

    signInInFlight = true;
    try {
      await runSignInCeremony(username);

      // A full navigation, so the new session cookie shows up in server-rendered state.
      window.location.href = afterSignIn();
    } catch (error) {
      if (error && error.name === "NotAllowedError") {
        // A dismissed prompt and an authenticator that offered nothing look identical.
        showError("Nothing was offered. Check the username, or enter your password.");
        return;
      }
      if (error && error.status === 401) {
        // 401 from `start` means the account holds no passkeys, from `finish` means the
        // assertion did not verify. Either way the password is the way out, so it is
        // opened and focused rather than left behind the link that asks for it.
        revealPasswordField();
        focusPasswordField();
        showError("No passkey is set up for that username. Enter your password instead.");
        return;
      }
      console.error(error);
      showError("Sign-in failed. Please enter your password.");
    } finally {
      signInInFlight = false;
    }
  }

  // Where the device holds a platform authenticator there is somewhere a discoverable
  // credential could have been kept, so the nameless ceremony is worth offering. The answer is
  // about the device, not about this site, so a yes can also come from somebody who has no
  // passkey here - which is the case the nudge below exists for.
  async function platformAuthenticatorAvailable() {
    if (
      !isSupported() ||
      typeof window.PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable !==
        "function"
    ) {
      return false;
    }
    try {
      return await window.PublicKeyCredential.isUserVerifyingPlatformAuthenticatorAvailable();
    } catch (error) {
      // A refusing implementation. The button stays hidden, which is the safer of the two
      // answers: it is the one that promises nothing.
      return false;
    }
  }

  // Says what to do instead, beside the field that is filled in next. Step one owns the
  // message area this writes to, and `auth.js` clears it the moment the username changes -
  // which is exactly when a nudge stops being true.
  function nudgeToUsername(message) {
    const stepError = document.querySelector("[data-login-error]");
    if (!stepError) {
      // A page without the two-step form: the generic error area is all there is.
      showError(message);
      focusUsernameField();
      return;
    }
    stepError.textContent = message;
    stepError.hidden = false;
    focusUsernameField();
  }

  // The button on the first sign-in step, which `revealDeviceSignIn` shows only where the
  // device has a platform authenticator. It is the conditional-UI ceremony asked for on
  // demand: the same nameless request, with `mediation` left off so the browser offers what it
  // has now instead of waiting for the user to reach for the field. Nothing guarantees that
  // anything answers - only a credential the device already made discoverable can - so a
  // device with nothing to offer lands on the username below rather than on an error.
  async function signInWithDevicePasskey() {
    if (signInInFlight) {
      return;
    }
    clearError();

    signInInFlight = true;
    try {
      await runSignInCeremony(null);

      // A full navigation, so the new session cookie shows up in server-rendered state.
      window.location.href = afterSignIn();
    } catch (error) {
      if (error && error.name === "NotAllowedError") {
        // Nothing on this device could answer, or the prompt was dismissed. A credential the
        // device cannot find on its own is not reachable this way at all, so the username is
        // what to ask for next - which is the whole point of the second step.
        nudgeToUsername(
          "No passkey was found on this device. Type your username, then Continue, to use a security key or your password."
        );
        return;
      }
      // Anything else, a refused assertion included: the username is still the way into every
      // other route, and saying so beats a dead end.
      console.error(error);
      nudgeToUsername("Sign-in failed. Type your username to try another way in.");
    } finally {
      signInInFlight = false;
    }
  }

  // ── Conditional UI ───────────────────────────────────────────────────────────
  //
  // The username field carries `autocomplete="... webauthn"`, so the browser offers a passkey
  // there the way it offers a saved password. This is the only place `mediation` is set, and
  // the only request that is left waiting: the nameless one the button makes is answered there
  // and then, because the user asked for it. A request that is never answered is the normal
  // case here, not a failure, so none of this may surface an error.

  let conditionalAbort = null;
  let conditionalArmed = false;
  // When the last request stopped waiting, so a burst of focus events cannot pile up
  // challenges.
  let conditionalFinishedAt = 0;

  // One re-arm per cooldown at most.
  const CONDITIONAL_REARM_COOLDOWN_MS = 30 * 1000;

  function conditionalUiAvailable() {
    return (
      isSupported() &&
      typeof window.PublicKeyCredential.isConditionalMediationAvailable === "function"
    );
  }

  // Opt-in per page, so the profile and onboarding pages are untouched.
  function conditionalField() {
    return document.querySelector('input[autocomplete~="webauthn"]');
  }

  function hasConditionalField() {
    return conditionalField() !== null;
  }

  // Two concurrent `credentials.get` calls are not allowed, and the one the user asked
  // for has to win.
  function abandonConditionalSignIn() {
    if (conditionalAbort) {
      conditionalAbort.abort();
      conditionalAbort = null;
    }
  }

  // The browser gives up after its own timeout and rejects with the same
  // `NotAllowedError` it uses for a dismissal, so without a re-arm the username field
  // would quietly stop offering a passkey for the rest of the page's life.
  function maybeRearmConditionalSignIn() {
    if (conditionalArmed || signInInFlight) {
      return;
    }
    if (Date.now() - conditionalFinishedAt < CONDITIONAL_REARM_COOLDOWN_MS) {
      return;
    }
    startConditionalSignIn();
  }

  async function startConditionalSignIn() {
    if (conditionalArmed || !hasConditionalField() || !conditionalUiAvailable()) {
      return;
    }

    conditionalArmed = true;
    try {
      if (!(await window.PublicKeyCredential.isConditionalMediationAvailable())) {
        return;
      }

      const controller = new AbortController();
      conditionalAbort = controller;

      // A challenge of its own, separate from the one the button requests.
      const start = await postJson("/login/passkey/start", { username: null });

      const publicKey = optionsFrom(start);
      publicKey.challenge = b64urlToBuffer(publicKey.challenge);

      let credential;
      try {
        credential = await navigator.credentials.get({
          publicKey,
          mediation: "conditional",
          signal: controller.signal,
        });
      } catch (error) {
        // Aborted, nothing selected, or the browser's own timeout. The user did not ask
        // for this, so say nothing; focus re-arms it if they come back to the field.
        return;
      }

      // Reaching here means they deliberately picked one, so a failure is worth reporting.
      conditionalAbort = null;
      try {
        await postJson("/login/passkey/finish", {
          challenge_id: start.challenge_id,
          persistent: keepSignedIn(),
          credential: assertionToJson(credential),
        });
        window.location.href = afterSignIn();
      } catch (error) {
        console.error(error);
        showError("Sign-in failed. Please enter your password.");
      }
    } catch (error) {
      // Autofill must never break the page; the password form depends on none of this.
      console.error(error);
    } finally {
      conditionalAbort = null;
      conditionalArmed = false;
      conditionalFinishedAt = Date.now();
    }
  }

  async function addPasskey(displayName, redirectTo) {
    clearError();

    const password = stepUpPassword();
    if (hasPasswordField() && !password) {
      showError("Enter your password below first, so only you can add a passkey.");
      return;
    }

    try {
      // The password goes in the *start* body so a wrong one is rejected before any
      // ceremony exists. Null only when the page rendered no field.
      const start = await postJson("/profile/passkeys/register/start", {
        password: password || null,
      });
      const publicKey = optionsFrom(start);
      publicKey.challenge = b64urlToBuffer(publicKey.challenge);
      publicKey.user.id = b64urlToBuffer(publicKey.user.id);
      // These bytes need converting like `allowCredentials` does; the browser otherwise
      // rejects the call, which is why a second passkey used to fail.
      if (Array.isArray(publicKey.excludeCredentials)) {
        publicKey.excludeCredentials = publicKey.excludeCredentials.map((entry) => ({
          ...entry,
          id: b64urlToBuffer(entry.id),
        }));
      }

      const credential = await navigator.credentials.create({ publicKey });
      await postJson("/profile/passkeys/register/finish", {
        challenge_id: start.challenge_id,
        name: displayName || null,
        credential: attestationToJson(credential),
      });

      // Onboarding moves on to the dashboard; the profile page redraws in place.
      if (redirectTo) {
        window.location.href = redirectTo;
      } else {
        window.location.reload();
      }
    } catch (error) {
      if (error && error.name === "NotAllowedError") {
        showError("Passkey setup was cancelled. You can try again, or continue without one.");
        return;
      }
      if (error && error.status === 401) {
        // Either the step-up password was wrong, or the session is now older than the
        // step-up window. The second case is only reachable from a page that rendered no
        // password field: the onboarding page whose session was fresh when it loaded.
        if (hasPasswordField()) {
          showError("That password did not match. Try again.");
          return;
        }
        const field = ensurePasswordField();
        if (field) {
          field.focus();
          showError(
            "For your security, please confirm your password to attach this passkey."
          );
          return;
        }
        showError("Your sign-in has expired. Sign in again, then set up the passkey.");
        return;
      }
      if (error && error.status === 400) {
        // A 400 here is the server turning down the credential itself - a bad attestation,
        // or a challenge that had expired. The name is never the reason: the server
        // strips anything it dislikes from the label rather than refusing a key the user
        // has already created on their device.
        showError("That passkey could not be registered. Please try again.");
        return;
      }
      console.error(error);
      showError("Passkey setup failed.");
    }
  }

  // A credential change too, so it takes the same step-up password. JSON rather than a
  // form post, so a wrong password is reported in place instead of becoming a redirect.
  async function deletePasskey(passkeyId, passkeyName) {
    clearError();

    const password = stepUpPassword();
    if (!password) {
      showError("Enter your password below first, so only you can remove a passkey.");
      return;
    }

    const label = passkeyName ? ' "' + passkeyName + '"' : "";
    const confirmed = window.confirm(
      "Remove the passkey" + label + "? You will no longer be able to sign in with it."
    );
    if (!confirmed) {
      return;
    }

    try {
      await postJson("/profile/passkeys/delete", {
        passkey_id: passkeyId,
        password: password,
      });
      window.location.reload();
    } catch (error) {
      if (error && error.status === 401) {
        showError("That password did not match. Try again.");
        return;
      }
      console.error(error);
      showError("Could not remove the passkey.");
    }
  }

  // `script-src 'self'` blocks inline handlers, so every control is wired up here.
  // Controls that cannot work are hidden rather than left to fail on click.
  function wireControls() {
    const supported = isSupported();

    const unlock = unlockName();
    document.querySelectorAll("[data-passkey-platform]").forEach(function (element) {
      element.textContent = unlock || PLATFORM_NAME_FALLBACK;
    });
    document.querySelectorAll("[data-passkey-label]").forEach(function (element) {
      element.textContent = "Sign in with " + (unlock || ACTION_NAME_FALLBACK);
    });
    document.querySelectorAll("[data-passkey-unsupported]").forEach(function (element) {
      element.hidden = supported;
    });
    document.querySelectorAll("[data-passkey-section]").forEach(function (element) {
      element.hidden = !supported;
    });

    document.querySelectorAll("[data-passkey-action]").forEach(function (element) {
      if (!supported) {
        element.hidden = true;
        return;
      }

      const action = element.getAttribute("data-passkey-action");
      element.addEventListener("click", function (event) {
        event.preventDefault();
        if (action === "sign-in") {
          // Drop any pending autofill request first: two concurrent ceremonies are
          // not allowed, and this one is the one the user asked for.
          abandonConditionalSignIn();
          signInWithPasskey();
        } else if (action === "sign-in-device") {
          // Same reason, and this one is answered by the same kind of credential the autofill
          // request is waiting for.
          abandonConditionalSignIn();
          signInWithDevicePasskey();
        } else if (action === "add") {
          addPasskeyWithPrompt();
        } else if (action === "add-onboarding") {
          addPasskey(null, "/");
        } else if (action === "delete") {
          deletePasskey(
            Number(element.getAttribute("data-passkey-id")),
            element.getAttribute("data-passkey-name")
          );
        }
      });
    });
  }

  async function revealDeviceSignIn() {
    const buttons = document.querySelectorAll('[data-passkey-action="sign-in-device"]');
    if (!buttons.length) {
      return;
    }

    if (await platformAuthenticatorAvailable()) {
      buttons.forEach(function (element) {
        element.hidden = false;
      });
      return;
    }

    // Nothing above the username field to be the filled button, so the username gets to be it.
    const continueButton = document.querySelector("[data-login-continue]");
    if (continueButton) {
      continueButton.classList.remove("auth-btn--secondary");
      continueButton.classList.add("auth-btn--primary");
    }
  }

  document.addEventListener("DOMContentLoaded", function () {
    wireControls();

    // Reaching for the username box is when autofill has to be live again.
    const field = conditionalField();
    if (field) {
      field.addEventListener("focus", maybeRearmConditionalSignIn);
    }

    // Not awaited: it stays pending for as long as the page is open.
    startConditionalSignIn();

    // Nor this one: the button appears if and when the device answers for it.
    revealDeviceSignIn();
  });

  function addPasskeyWithPrompt() {
    if (typeof window.Swal === "undefined") {
      addPasskey(window.prompt("Name this passkey (for example: MacBook)") || null);
      return;
    }

    window.Swal.fire({
      title: "Name this passkey",
      input: "text",
      inputPlaceholder: "e.g. MacBook, YubiKey",
      showCancelButton: true,
      confirmButtonText: "Continue",
      inputValidator: function (value) {
        return value && value.length > 64 ? "Please use 64 characters or fewer." : undefined;
      },
    }).then(function (result) {
      if (result.isConfirmed) {
        addPasskey(result.value || null);
      }
    });
  }

  window.passkeys = {
    isSupported,
    signInWithPasskey,
    startConditionalSignIn,
    abandonConditionalSignIn,
    addPasskey,
    addPasskeyWithPrompt,
    deletePasskey,
    showError,
  };
})();
