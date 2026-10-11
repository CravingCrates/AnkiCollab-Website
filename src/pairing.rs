use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect};
use axum::Form;
use axum_client_ip::ClientIp;
use serde::Deserialize;

use crate::database::{client, AppState};
use crate::error::Error;
use crate::user::User;

/// Digits of a code, excluding the separator.
const CODE_LENGTH: usize = 6;

/// An unresolved code means a typo, an expired one, or someone working through the
/// space. The first two are ordinary, so this sits well above what a person reaches
/// before giving up and starting a new pairing.
const GUESS_ALERT_THRESHOLD: u32 = 10;

/// Wider than a code's own lifetime, so a burst spread across several codes still adds
/// up instead of resetting with each one.
const GUESS_WINDOW: Duration = Duration::from_mins(10);

#[derive(Debug)]
pub struct CodeGuessTracker {
    window: Duration,
    attempts: Mutex<HashMap<IpAddr, Window>>,
}

#[derive(Debug)]
struct Window {
    count: u32,
    started: Instant,
}

impl CodeGuessTracker {
    #[must_use]
    pub fn new() -> Self {
        Self::with_window(GUESS_WINDOW)
    }

    /// A tracker with a shorter window, so a test can watch one lapse without waiting
    /// ten minutes for it.
    #[must_use]
    fn with_window(window: Duration) -> Self {
        Self {
            window,
            attempts: Mutex::new(HashMap::new()),
        }
    }

    pub fn record_failure(&self, ip: IpAddr) -> bool {
        let Ok(mut attempts) = self.attempts.lock() else {
            // A poisoned lock means another thread panicked mid-count. Losing a count
            // is not worth failing a user's request over.
            return false;
        };

        let window = attempts.entry(ip).or_insert_with(|| Window {
            count: 0,
            started: Instant::now(),
        });

        if window.started.elapsed() > self.window {
            *window = Window {
                count: 0,
                started: Instant::now(),
            };
        }

        window.count += 1;
        window.count == GUESS_ALERT_THRESHOLD
    }

    /// Forgets addresses that have gone quiet, so the map holds live ones only.
    pub fn retain_recent(&self) {
        let Ok(mut attempts) = self.attempts.lock() else {
            return;
        };
        attempts.retain(|_, window| window.started.elapsed() <= self.window);
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.attempts.lock().map_or(0, |inner| inner.len())
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for CodeGuessTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Deserialize)]
pub struct PairQuery {
    #[serde(default)]
    pub code: Option<String>,
}

#[derive(Deserialize)]
pub struct PairDecision {
    pub code: String,
    #[serde(default)]
    pub decision: String,
}

fn normalise_code(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| match c.to_ascii_uppercase() {
            // Crockford normalization substitutions
            'O' => '0',
            'I' | 'L' => '1',
            other => other,
        })
        .collect();

    if cleaned.len() != CODE_LENGTH {
        return None;
    }

    Some(format!("{}-{}", &cleaned[..3], &cleaned[3..]))
}

/// Percent-encodes a path for use as a query value. Placed here rather than pulled
/// from a crate because it is only ever handed values this module built itself.
fn encode_path(path: &str) -> String {
    path.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn return_path(code: Option<&str>) -> String {
    match code.and_then(normalise_code) {
        Some(code) => format!("/pair?code={code}"),
        None => "/pair".to_string(),
    }
}

fn render_page(
    db_state: &AppState,
    user: &User,
    state: &str,
    code: &str,
) -> Result<Html<String>, Error> {
    let mut context = tera::Context::new();
    context.insert("user", user);
    context.insert("state", state);
    context.insert("code", code);
    Ok(Html(db_state.tera.render("pair.html", &context)?))
}

fn trimmed_bounded(raw: &str) -> String {
    raw.trim().chars().take(64).collect()
}

pub async fn get_pair(
    State(db_state): State<Arc<AppState>>,
    Query(query): Query<PairQuery>,
    user: Option<User>,
) -> Result<axum::response::Response, Error> {
    let Some(user) = user else {
        return Err(Error::Redirect(format!(
            "/login?next={}",
            encode_path(&return_path(query.code.as_deref()))
        )));
    };

    let code = query
        .code
        .as_deref()
        .and_then(normalise_code)
        .unwrap_or_default();

    Ok(render_page(&db_state, &user, "form", &code)?.into_response())
}

fn is_same_origin(headers: &HeaderMap, site_origin: &str) -> bool {
    // Sent by every current browser and not settable from page JS.
    if let Some(v) = headers.get("sec-fetch-site") {
        return v.as_bytes() == b"same-origin";
    }
    // Fallback for browsers without Fetch Metadata. Fail closed if absent.
    headers
        .get(header::ORIGIN)
        .is_some_and(|v| v.as_bytes() == site_origin.as_bytes())
}

pub async fn post_pair_approve(
    headers: HeaderMap,
    ClientIp(ip): ClientIp,
    State(db_state): State<Arc<AppState>>,
    user: Option<User>,
    Form(decision): Form<PairDecision>,
) -> Result<impl IntoResponse, Error> {
    if !is_same_origin(&headers, &db_state.site_origin) {
        tracing::warn!("rejected cross-origin pairing decision");
        return Ok(StatusCode::FORBIDDEN.into_response());
    }

    let Some(user) = user else {
        return Ok(Redirect::to("/login?next=%2Fpair").into_response());
    };

    let Some(code) = normalise_code(&decision.code) else {
        // Hand back what they typed so a typo is one correction, not a retype.
        let typed = trimmed_bounded(&decision.code);
        return Ok(render_page(&db_state, &user, "malformed", &typed)?.into_response());
    };

    let approve = decision.decision == "approve";
    let status = if approve { "approved" } else { "rejected" };

    let conn = client(&db_state).await?;
    let changed = conn
        .execute(
            "UPDATE public.device_pair_sessions
                SET status = $3, user_id = $2
              WHERE code = $1
                AND status = 'pending'
                AND expires_at > now()",
            &[&code, &user.id(), &status],
        )
        .await?;

    if changed == 0 {
        tracing::warn!(%ip, user_id = user.id(), "unresolved pairing code");
        if db_state.pairing_attempts.record_failure(ip) {
            sentry::capture_message(
                &format!(
                    "{} unresolved pairing codes from {ip} in {} minutes",
                    GUESS_ALERT_THRESHOLD,
                    GUESS_WINDOW.as_secs() / 60
                ),
                sentry::Level::Warning,
            );
        }

        return Ok(render_page(&db_state, &user, "unknown", &code)?.into_response());
    }

    sentry::add_breadcrumb(sentry::Breadcrumb {
        category: Some("pairing".into()),
        message: Some(format!(
            "device pairing {} for user {}",
            if approve { "approved" } else { "denied" },
            user.id()
        )),
        level: sentry::Level::Info,
        ..Default::default()
    });

    let state = if approve { "approved" } else { "denied" };
    Ok(render_page(&db_state, &user, state, &code)?.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_is_accepted_however_it_was_copied() {
        assert_eq!(normalise_code("ABC234").as_deref(), Some("ABC-234"));
        assert_eq!(normalise_code("abc-234").as_deref(), Some("ABC-234"));
        assert_eq!(normalise_code(" ABC 234 ").as_deref(), Some("ABC-234"));
        assert_eq!(normalise_code("abc234").as_deref(), Some("ABC-234"));
    }

    #[test]
    fn anything_that_is_not_a_code_is_refused() {
        assert_eq!(normalise_code(""), None);
        assert_eq!(normalise_code("ABC"), None);
        assert_eq!(normalise_code("ABC2345"), None);
        // Stripping the separators must not turn a wrong length into a right one.
        assert_eq!(normalise_code("A-B-C-2-3-4-5"), None);
    }

    #[test]
    fn a_return_path_survives_being_put_in_a_query() {
        assert_eq!(
            encode_path("/pair?code=ABC-234"),
            "%2Fpair%3Fcode%3DABC-234"
        );
        assert_eq!(encode_path("/pair"), "%2Fpair");
    }

    #[test]
    fn a_return_path_keeps_a_valid_code_and_drops_a_bad_one() {
        assert_eq!(return_path(Some("abc-234")), "/pair?code=ABC-234");
        assert_eq!(return_path(Some("nonsense")), "/pair");
        assert_eq!(return_path(None), "/pair");
    }

    #[test]
    fn a_run_of_unresolved_codes_alerts_exactly_once() {
        let tracker = CodeGuessTracker::new();
        let ip = IpAddr::from([203, 0, 113, 7]);

        for attempt in 1..GUESS_ALERT_THRESHOLD {
            assert!(
                !tracker.record_failure(ip),
                "alerted on attempt {attempt}, before the threshold"
            );
        }
        assert!(
            tracker.record_failure(ip),
            "the threshold attempt must alert"
        );
        assert!(
            !tracker.record_failure(ip),
            "a sustained run must not alert again inside the same window"
        );
    }

    #[test]
    fn addresses_are_counted_separately() {
        let tracker = CodeGuessTracker::new();
        let noisy = IpAddr::from([203, 0, 113, 7]);
        let quiet = IpAddr::from([198, 51, 100, 9]);

        for _ in 0..GUESS_ALERT_THRESHOLD {
            tracker.record_failure(noisy);
        }

        assert!(
            !tracker.record_failure(quiet),
            "one address reaching the threshold must not alert for another"
        );
        assert_eq!(tracker.len(), 2);
    }

    #[test]
    fn a_lapsed_window_starts_over() {
        let tracker = CodeGuessTracker::with_window(Duration::from_millis(1));
        let ip = IpAddr::from([203, 0, 113, 7]);

        for _ in 0..GUESS_ALERT_THRESHOLD - 1 {
            tracker.record_failure(ip);
        }
        assert!(tracker.record_failure(ip));

        std::thread::sleep(Duration::from_millis(20));

        // Back to a count of one, so the far side of the gap cannot alert on the
        // strength of attempts that should have been forgotten.
        assert!(!tracker.record_failure(ip));
    }

    #[test]
    fn quiet_addresses_are_forgotten() {
        let tracker = CodeGuessTracker::with_window(Duration::from_millis(1));
        tracker.record_failure(IpAddr::from([203, 0, 113, 7]));
        assert_eq!(tracker.len(), 1);

        std::thread::sleep(Duration::from_millis(20));
        tracker.retain_recent();

        assert!(tracker.is_empty(), "a lapsed address must not be kept");
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            let key: axum::http::HeaderName = name.parse().unwrap();
            let val: axum::http::HeaderValue = value.parse().unwrap();
            headers.insert(key, val);
        }
        headers
    }

    #[test]
    fn fetch_metadata_is_trusted_when_it_is_present() {
        assert!(is_same_origin(
            &headers(&[("sec-fetch-site", "same-origin")]),
            "https://ankicollab.com"
        ));
        assert!(!is_same_origin(
            &headers(&[("sec-fetch-site", "cross-site")]),
            "https://ankicollab.com"
        ));
        // A sibling subdomain is same-site, not same-origin, and must not pass.
        assert!(!is_same_origin(
            &headers(&[("sec-fetch-site", "same-site")]),
            "https://ankicollab.com"
        ));
    }

    #[test]
    fn origin_is_the_fallback_and_must_match_exactly() {
        assert!(is_same_origin(
            &headers(&[("origin", "https://ankicollab.com")]),
            "https://ankicollab.com"
        ));
        assert!(!is_same_origin(
            &headers(&[("origin", "https://evil.example")]),
            "https://ankicollab.com"
        ));
        // Neither header, or an opaque origin, is refused rather than waved through.
        assert!(!is_same_origin(&headers(&[]), "https://ankicollab.com"));
        assert!(!is_same_origin(
            &headers(&[("origin", "null")]),
            "https://ankicollab.com"
        ));
    }

    #[test]
    fn fetch_metadata_wins_over_an_origin_that_looks_right() {
        // A cross-site form cannot suppress the header its browser sets, so a matching
        // `Origin` alongside `cross-site` must not be enough to pass.
        assert!(!is_same_origin(
            &headers(&[
                ("sec-fetch-site", "cross-site"),
                ("origin", "https://ankicollab.com"),
            ]),
            "https://ankicollab.com"
        ));
    }
}
