use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use time::{OffsetDateTime, UtcOffset};

use crate::database::{self, AppState};
use crate::webhook_crypto::WebhookCipher;
use crate::webhook_manager::{
    build_discord_payload, get_webhook_view, is_undefined_table, load_webhook_url_for_test,
    validate_discord_webhook_url, WebhookView,
};

const MAX_DELIVERY_ATTEMPTS: i32 = 5;
const MAX_RATE_LIMIT_ATTEMPTS: i32 = 10;
const AUTO_DISABLE_FAILURES: i32 = 10;
const DELIVERIES_PER_TICK: usize = 25;
const CONCURRENT_WORKERS: usize = 4;
const RETENTION_DAYS: i32 = 14;
const MAX_DISCORD_ERROR_BODY_BYTES: usize = 8 * 1024;
pub const TEST_COOLDOWN: Duration = Duration::from_secs(5);

/// Per-webhook in-memory cooldown for manual "Send test" button presses.
#[derive(Debug, Default)]
pub struct WebhookTestCooldown {
    last_tested: Mutex<HashMap<i64, Instant>>,
}

impl WebhookTestCooldown {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn try_acquire(&self, webhook_id: i64) -> Result<(), u64> {
        self.try_acquire_at(webhook_id, Instant::now())
    }

    fn try_acquire_at(&self, webhook_id: i64, now: Instant) -> Result<(), u64> {
        let Ok(mut guard) = self.last_tested.lock() else {
            return Ok(());
        };
        if let Some(prev) = guard.get(&webhook_id) {
            let elapsed = now.saturating_duration_since(*prev);
            if elapsed < TEST_COOLDOWN {
                let remaining_ms = TEST_COOLDOWN
                    .as_millis()
                    .saturating_sub(elapsed.as_millis());
                let wait_secs = u64::try_from(remaining_ms.div_ceil(1000))
                    .unwrap_or(1)
                    .max(1);
                return Err(wait_secs);
            }
        }
        guard.insert(webhook_id, now);
        Ok(())
    }

    pub fn retain_recent(&self) {
        let Ok(mut guard) = self.last_tested.lock() else {
            return;
        };
        guard.retain(|_, ts| ts.elapsed() <= Duration::from_secs(60));
    }
}

#[must_use]
pub fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .user_agent("AnkiCollab-Webhooks/1.0")
        .build()
        .expect("failed to build webhook HTTP client")
}

#[must_use]
pub fn backoff_seconds(attempts: i32) -> i32 {
    match attempts {
        ..=1 => 60,
        2 => 120,
        3 => 300,
        4 => 900,
        _ => 1_800,
    }
}

#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn parse_retry_after_secs(
    headers: &reqwest::header::HeaderMap,
    body_json: Option<&Value>,
) -> i32 {
    let from_header = |name: &str| -> Option<f64> {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0)
    };

    let raw_secs = from_header("x-ratelimit-reset-after")
        .or_else(|| from_header(reqwest::header::RETRY_AFTER.as_str()))
        .or_else(|| {
            body_json
                .and_then(|v| v.get("retry_after"))
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite() && *v > 0.0)
        });

    raw_secs.map_or(30, |v| v.ceil().clamp(5.0, 300.0) as i32)
}

async fn read_discord_json(resp: reqwest::Response) -> Option<Value> {
    let bytes = resp.bytes().await.ok()?;
    if bytes.len() > MAX_DISCORD_ERROR_BODY_BYTES {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

#[must_use]
pub fn classify_discord_error(status: reqwest::StatusCode, body: Option<&Value>) -> (String, bool) {
    let discord_code = body.and_then(|v| v.get("code")).and_then(Value::as_i64);
    match discord_code {
        Some(220_001) => (
            "Discord Forum/Media channels require ?thread_id=<post_id> in the webhook URL"
                .to_string(),
            true,
        ),
        Some(10_015) => (
            "Discord webhook no longer exists (Unknown Webhook)".to_string(),
            true,
        ),
        Some(50_027) => ("Invalid Discord webhook token".to_string(), true),
        _ if is_permanent_discord_status(status) => {
            (format!("Discord returned HTTP {}", status.as_u16()), true)
        }
        _ if status.is_client_error() => (
            format!("Discord rejected payload (HTTP {})", status.as_u16()),
            false,
        ),
        _ => (format!("Discord returned HTTP {}", status.as_u16()), false),
    }
}

/// Returns `true` when a webhook configured for `daily` or `weekly` digests
/// should emit a summary at `now`.
#[must_use]
pub fn is_digest_due(
    now: OffsetDateTime,
    frequency: &str,
    hour_utc: i16,
    day_of_week: i16,
    last_sent: Option<OffsetDateTime>,
) -> bool {
    let now_utc = now.to_offset(UtcOffset::UTC);
    let Ok(target_hour) = u8::try_from(hour_utc) else {
        return false;
    };
    if target_hour > 23 || now_utc.hour() < target_hour {
        return false;
    }

    match frequency {
        "daily" => match last_sent {
            None => true,
            Some(ts) => {
                let ts_utc = ts.to_offset(UtcOffset::UTC);
                ts_utc.date() < now_utc.date() && (now_utc - ts_utc) >= time::Duration::hours(12)
            }
        },
        "weekly" => {
            if i16::from(now_utc.weekday().number_from_monday()) != day_of_week {
                return false;
            }
            match last_sent {
                None => true,
                Some(ts) => (now_utc - ts.to_offset(UtcOffset::UTC)) >= time::Duration::days(6),
            }
        }
        _ => false,
    }
}

fn is_permanent_discord_status(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::UNAUTHORIZED
            | reqwest::StatusCode::FORBIDDEN
            | reqwest::StatusCode::NOT_FOUND
            | reqwest::StatusCode::GONE
    )
}

pub async fn send_test_webhook(
    state: &AppState,
    cipher: &WebhookCipher,
    deck_id: i64,
    webhook_id: i64,
) -> Result<WebhookView, String> {
    let Some((raw_url, deck_name, deck_hash)) =
        load_webhook_url_for_test(state, cipher, deck_id, webhook_id)
            .await
            .map_err(|_| "Failed to load webhook configuration.".to_string())?
    else {
        return Err("Webhook not found for this deck.".to_string());
    };

    let validated = validate_discord_webhook_url(&raw_url)?;
    let body = build_discord_payload(
        "test",
        &json!({
            "deck_name": deck_name,
            "deck_hash": deck_hash,
        }),
        &state.site_origin,
    );

    let resp = state
        .http_client
        .post(&validated.canonical_url)
        .json(&body)
        .send()
        .await;

    let client = database::client(state)
        .await
        .map_err(|_| "Database connection error.".to_string())?;

    match resp {
        Ok(r) if r.status().is_success() => {
            let _ = client
                .execute(
                    "UPDATE anki.deck_webhooks
                     SET last_delivered_at = now(),
                         last_error = NULL,
                         consecutive_failures = 0,
                         updated_at = now()
                     WHERE id = $1 AND deck_id = $2",
                    &[&webhook_id, &deck_id],
                )
                .await;
            get_webhook_view(state, deck_id, webhook_id)
                .await
                .map_err(|_| "Failed to reload webhook status.".to_string())?
                .ok_or_else(|| "Webhook not found for this deck.".to_string())
        }
        Ok(r) if r.status() == reqwest::StatusCode::TOO_MANY_REQUESTS => Err(
            "Discord rate-limited this webhook (HTTP 429). Please wait a moment and try again."
                .to_string(),
        ),
        Ok(r) => {
            let status = r.status();
            let err_body = read_discord_json(r).await;
            let (err_msg, _) = classify_discord_error(status, err_body.as_ref());
            let _ = client
                .execute(
                    "UPDATE anki.deck_webhooks
                     SET last_error = $3,
                         updated_at = now()
                     WHERE id = $1 AND deck_id = $2",
                    &[&webhook_id, &deck_id, &err_msg],
                )
                .await;
            Err(format!("Test delivery failed: {err_msg}."))
        }
        Err(err) => {
            let err_msg = if err.is_timeout() {
                "Request to Discord timed out".to_string()
            } else {
                "Failed to reach Discord".to_string()
            };
            let _ = client
                .execute(
                    "UPDATE anki.deck_webhooks
                     SET last_error = $3,
                         updated_at = now()
                     WHERE id = $1 AND deck_id = $2",
                    &[&webhook_id, &deck_id, &err_msg],
                )
                .await;
            Err(format!("Test delivery failed: {err_msg}."))
        }
    }
}

struct ClaimedDelivery {
    id: i64,
    webhook_id: i64,
    event_type: String,
    payload: Value,
    attempts: i32,
    deck_id: i64,
    ciphertext: Vec<u8>,
    nonce: Vec<u8>,
    webhook_enabled: bool,
    deck_deleted: bool,
    consecutive_failures: i32,
}

async fn claim_next_delivery(state: &AppState) -> Result<Option<ClaimedDelivery>, String> {
    let client = database::client(state)
        .await
        .map_err(|_| "db connection error".to_string())?;

    let row_opt = match client
        .query_opt(
            "WITH candidate AS (
                 SELECT d.id
                 FROM anki.webhook_deliveries d
                 WHERE (
                     (d.state = 'pending' AND d.next_attempt_at <= now())
                     OR (d.state = 'running' AND d.claimed_at < now() - INTERVAL '5 minutes')
                 )
                 AND NOT EXISTS (
                     SELECT 1
                     FROM anki.webhook_deliveries r
                     WHERE r.webhook_id = d.webhook_id
                       AND r.state = 'running'
                       AND r.claimed_at >= now() - INTERVAL '5 minutes'
                       AND r.id <> d.id
                 )
                 ORDER BY d.next_attempt_at ASC, d.id ASC
                 LIMIT 1
                 FOR UPDATE OF d SKIP LOCKED
             ),
             claimed AS (
                 UPDATE anki.webhook_deliveries d
                 SET state = 'running',
                     attempts = d.attempts + 1,
                     claimed_at = now()
                 FROM candidate
                 WHERE d.id = candidate.id
                 RETURNING d.id, d.webhook_id, d.event_type, d.payload, d.attempts
             )
             SELECT c.id,
                    c.webhook_id,
                    c.event_type,
                    c.payload,
                    c.attempts,
                    w.deck_id,
                    w.webhook_url_ciphertext,
                    w.webhook_url_nonce,
                    w.enabled,
                    (dk.deleted_at IS NOT NULL) AS deck_deleted,
                    w.consecutive_failures
             FROM claimed c
             JOIN anki.deck_webhooks w ON w.id = c.webhook_id
             JOIN anki.decks dk ON dk.id = w.deck_id",
            &[],
        )
        .await
    {
        Ok(opt) => opt,
        Err(err) if is_undefined_table(&err) => return Ok(None),
        Err(err) => return Err(format!("claim failed: {err}")),
    };

    Ok(row_opt.map(|r| ClaimedDelivery {
        id: r.get(0),
        webhook_id: r.get(1),
        event_type: r.get(2),
        payload: r.get(3),
        attempts: r.get(4),
        deck_id: r.get(5),
        ciphertext: r.get(6),
        nonce: r.get(7),
        webhook_enabled: r.get(8),
        deck_deleted: r.get(9),
        consecutive_failures: r.get(10),
    }))
}

async fn discard_disabled_delivery(state: &AppState, delivery_id: i64) {
    let Ok(client) = database::client(state).await else {
        return;
    };
    let _ = client
        .execute(
            "DELETE FROM anki.webhook_deliveries WHERE id = $1",
            &[&delivery_id],
        )
        .await;
}

async fn mark_delivery_done(state: &AppState, delivery_id: i64, webhook_id: i64) {
    let Ok(client) = database::client(state).await else {
        return;
    };
    let _ = client
        .execute(
            "UPDATE anki.webhook_deliveries
             SET state = 'done', payload = '{}'::jsonb, finished_at = now(), last_error = NULL
             WHERE id = $1",
            &[&delivery_id],
        )
        .await;
    let _ = client
        .execute(
            "UPDATE anki.deck_webhooks
             SET consecutive_failures = 0,
                 last_delivered_at = now(),
                 last_error = NULL,
                 disabled_reason = NULL,
                 updated_at = now()
             WHERE id = $1",
            &[&webhook_id],
        )
        .await;
}

async fn reschedule_delivery(
    state: &AppState,
    delivery_id: i64,
    webhook_id: i64,
    delay_secs: i32,
    error_msg: &str,
) {
    let Ok(client) = database::client(state).await else {
        return;
    };
    let _ = client
        .execute(
            "UPDATE anki.webhook_deliveries
             SET state = 'pending',
                 claimed_at = NULL,
                 next_attempt_at = now() + make_interval(secs => $2),
                 last_error = $3
             WHERE id = $1",
            &[&delivery_id, &delay_secs, &error_msg],
        )
        .await;
    let _ = client
        .execute(
            "UPDATE anki.deck_webhooks
             SET last_error = $2,
                 updated_at = now()
             WHERE id = $1",
            &[&webhook_id, &error_msg],
        )
        .await;
}

async fn reschedule_rate_limited_delivery(
    state: &AppState,
    delivery_id: i64,
    webhook_id: i64,
    delay_secs: i32,
    error_msg: &str,
) {
    let Ok(client) = database::client(state).await else {
        return;
    };
    let _ = client
        .execute(
            "UPDATE anki.webhook_deliveries
             SET state = 'pending',
                 claimed_at = NULL,
                 next_attempt_at = now() + make_interval(secs => $2),
                 last_error = $3
             WHERE id = $1",
            &[&delivery_id, &delay_secs, &error_msg],
        )
        .await;
    // Also push back sibling pending deliveries for the same webhook so the next tick
    // doesn't immediately burn another attempt while the Discord bucket is resetting.
    let _ = client
        .execute(
            "UPDATE anki.webhook_deliveries
             SET next_attempt_at = GREATEST(next_attempt_at, now() + make_interval(secs => $2))
             WHERE webhook_id = $1 AND state = 'pending'",
            &[&webhook_id, &delay_secs],
        )
        .await;
    let _ = client
        .execute(
            "UPDATE anki.deck_webhooks
             SET last_error = $2,
                 updated_at = now()
             WHERE id = $1",
            &[&webhook_id, &error_msg],
        )
        .await;
}

async fn mark_rate_limit_exhausted(
    state: &AppState,
    delivery_id: i64,
    webhook_id: i64,
    error_msg: &str,
) {
    let Ok(client) = database::client(state).await else {
        return;
    };
    let _ = client
        .execute(
            "UPDATE anki.webhook_deliveries
             SET state = 'failed', payload = '{}'::jsonb, finished_at = now(), last_error = $2
             WHERE id = $1",
            &[&delivery_id, &error_msg],
        )
        .await;
    let _ = client
        .execute(
            "UPDATE anki.deck_webhooks
             SET last_error = $2,
                 updated_at = now()
             WHERE id = $1",
            &[&webhook_id, &error_msg],
        )
        .await;
}

async fn mark_delivery_failed(
    state: &AppState,
    delivery_id: i64,
    webhook_id: i64,
    previous_failures: i32,
    error_msg: &str,
    force_disable_reason: Option<&str>,
) {
    let Ok(client) = database::client(state).await else {
        return;
    };
    let _ = client
        .execute(
            "UPDATE anki.webhook_deliveries
             SET state = 'failed', payload = '{}'::jsonb, finished_at = now(), last_error = $2
             WHERE id = $1",
            &[&delivery_id, &error_msg],
        )
        .await;

    let next_failures = previous_failures.saturating_add(1);
    let auto_reason = if let Some(reason) = force_disable_reason {
        Some(reason.to_string())
    } else if next_failures >= AUTO_DISABLE_FAILURES {
        Some(format!(
            "Auto-disabled after {next_failures} consecutive delivery failures ({error_msg})"
        ))
    } else {
        None
    };

    if let Some(reason) = auto_reason {
        let _ = client
            .execute(
                "UPDATE anki.deck_webhooks
                 SET enabled = false,
                     consecutive_failures = $2,
                     last_error = $3,
                     disabled_reason = $4,
                     updated_at = now()
                 WHERE id = $1",
                &[&webhook_id, &next_failures, &error_msg, &reason],
            )
            .await;
        let _ = client
            .execute(
                "UPDATE anki.webhook_deliveries
                 SET state = 'failed',
                     payload = '{}'::jsonb,
                     finished_at = now(),
                     last_error = $2
                 WHERE webhook_id = $1 AND state = 'pending'",
                &[&webhook_id, &reason],
            )
            .await;
    } else {
        let _ = client
            .execute(
                "UPDATE anki.deck_webhooks
                 SET consecutive_failures = $2,
                     last_error = $3,
                     updated_at = now()
                 WHERE id = $1",
                &[&webhook_id, &next_failures, &error_msg],
            )
            .await;
    }
}

async fn deliver_single_job(
    state: &AppState,
    cipher: &WebhookCipher,
    job: ClaimedDelivery,
) -> bool {
    if !job.webhook_enabled || job.deck_deleted {
        discard_disabled_delivery(state, job.id).await;
        return false;
    }

    let decrypted_url = match cipher.decrypt_url(job.deck_id, &job.ciphertext, &job.nonce) {
        Ok(url) => url,
        Err(err) => {
            tracing::error!(
                webhook_id = job.webhook_id,
                deck_id = job.deck_id,
                "Failed to decrypt stored webhook URL; auto-disabling webhook"
            );
            sentry::capture_message(
                "Webhook cipher failed to decrypt stored webhook URL (check WEBHOOK_ENCRYPTION_KEY)",
                sentry::Level::Warning,
            );
            mark_delivery_failed(
                state,
                job.id,
                job.webhook_id,
                job.consecutive_failures,
                &err,
                Some("Auto-disabled: stored webhook URL could not be decrypted"),
            )
            .await;
            return false;
        }
    };

    let validated = match validate_discord_webhook_url(&decrypted_url) {
        Ok(v) => v,
        Err(err) => {
            mark_delivery_failed(
                state,
                job.id,
                job.webhook_id,
                job.consecutive_failures,
                &err,
                Some("Auto-disabled: webhook URL failed validation"),
            )
            .await;
            return false;
        }
    };

    let body = build_discord_payload(&job.event_type, &job.payload, &state.site_origin);
    let resp = state
        .http_client
        .post(&validated.canonical_url)
        .json(&body)
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            mark_delivery_done(state, job.id, job.webhook_id).await;
            true
        }
        Ok(r) if r.status() == reqwest::StatusCode::TOO_MANY_REQUESTS => {
            let headers = r.headers().clone();
            let err_body = read_discord_json(r).await;
            let delay = parse_retry_after_secs(&headers, err_body.as_ref());
            if job.attempts < MAX_RATE_LIMIT_ATTEMPTS {
                reschedule_rate_limited_delivery(
                    state,
                    job.id,
                    job.webhook_id,
                    delay,
                    "Rate-limited by Discord (HTTP 429)",
                )
                .await;
            } else {
                mark_rate_limit_exhausted(
                    state,
                    job.id,
                    job.webhook_id,
                    "Exceeded retry limit after HTTP 429",
                )
                .await;
            }
            false
        }
        Ok(r) => {
            let status = r.status();
            let err_body = read_discord_json(r).await;
            let (msg, force_disable) = classify_discord_error(status, err_body.as_ref());
            if force_disable {
                let reason = format!("Auto-disabled: {msg}");
                mark_delivery_failed(
                    state,
                    job.id,
                    job.webhook_id,
                    job.consecutive_failures,
                    &msg,
                    Some(&reason),
                )
                .await;
            } else if status.is_client_error() {
                mark_delivery_failed(
                    state,
                    job.id,
                    job.webhook_id,
                    job.consecutive_failures,
                    &msg,
                    None,
                )
                .await;
            } else if job.attempts < MAX_DELIVERY_ATTEMPTS {
                reschedule_delivery(
                    state,
                    job.id,
                    job.webhook_id,
                    backoff_seconds(job.attempts),
                    &msg,
                )
                .await;
            } else {
                mark_delivery_failed(
                    state,
                    job.id,
                    job.webhook_id,
                    job.consecutive_failures,
                    &msg,
                    None,
                )
                .await;
            }
            false
        }
        Err(err) => {
            let msg = if err.is_timeout() {
                "Request to Discord timed out"
            } else {
                "Network error while contacting Discord"
            };
            if job.attempts < MAX_DELIVERY_ATTEMPTS {
                reschedule_delivery(
                    state,
                    job.id,
                    job.webhook_id,
                    backoff_seconds(job.attempts),
                    msg,
                )
                .await;
            } else {
                mark_delivery_failed(
                    state,
                    job.id,
                    job.webhook_id,
                    job.consecutive_failures,
                    msg,
                    None,
                )
                .await;
            }
            false
        }
    }
}

pub async fn process_pending_deliveries(state: &Arc<AppState>) -> usize {
    let Some(cipher) = state.webhook_cipher.clone() else {
        return 0;
    };

    let claimed_budget = Arc::new(AtomicUsize::new(0));
    let mut join_set = tokio::task::JoinSet::new();

    for _ in 0..CONCURRENT_WORKERS {
        let worker_state = Arc::clone(state);
        let worker_cipher = Arc::clone(&cipher);
        let worker_budget = Arc::clone(&claimed_budget);
        join_set.spawn(async move {
            let mut processed = 0usize;
            loop {
                if worker_budget.fetch_add(1, Ordering::Relaxed) >= DELIVERIES_PER_TICK {
                    break;
                }

                let job = match claim_next_delivery(&worker_state).await {
                    Ok(Some(job)) => job,
                    Ok(None) => break,
                    Err(err) => {
                        tracing::warn!(error = %err, "Failed to claim webhook delivery");
                        break;
                    }
                };

                if deliver_single_job(&worker_state, &worker_cipher, job).await {
                    processed += 1;
                }
            }
            processed
        });
    }

    let mut total_processed = 0usize;
    while let Some(res) = join_set.join_next().await {
        if let Ok(count) = res {
            total_processed += count;
        }
    }
    total_processed
}

struct DigestCandidate {
    webhook_id: i64,
    deck_id: i64,
    deck_name: String,
    deck_hash: String,
    stats_enabled: bool,
    retention: Option<f32>,
    frequency: String,
    hour_utc: i16,
    day_of_week: i16,
    skip_empty: bool,
    last_sent: Option<OffsetDateTime>,
}

async fn build_digest_payload(
    client: &tokio_postgres::Client,
    cand: &DigestCandidate,
) -> Result<Option<Value>, tokio_postgres::Error> {
    let lookback_days: i32 = if cand.frequency == "weekly" { 7 } else { 1 };
    let period_label = if cand.frequency == "weekly" {
        "Weekly"
    } else {
        "Daily"
    };

    let row = client
        .query_one(
            "WITH RECURSIVE subtree AS (
                 SELECT id FROM anki.decks WHERE id = $1 AND deleted_at IS NULL
                 UNION ALL
                 SELECT d.id FROM anki.decks d JOIN subtree s ON d.parent = s.id
                 WHERE d.deleted_at IS NULL
             ),
             pending_commits AS (
                 SELECT c.commit_id, c.timestamp
                 FROM anki.commits c
                 WHERE c.deck IN (SELECT id FROM subtree)
                   AND (
                       EXISTS (SELECT 1 FROM anki.fields f WHERE f.commit = c.commit_id AND f.reviewed = false)
                       OR EXISTS (SELECT 1 FROM anki.tags t WHERE t.commit = c.commit_id AND t.reviewed = false)
                       OR EXISTS (SELECT 1 FROM anki.card_deletion_suggestions cds WHERE cds.commit = c.commit_id)
                       OR EXISTS (SELECT 1 FROM anki.note_move_suggestions nms WHERE nms.commit = c.commit_id)
                   )
             ),
             period_commits AS (
                 SELECT COUNT(*)::bigint AS submitted
                 FROM anki.commits c
                 WHERE c.deck IN (SELECT id FROM subtree)
                   AND c.timestamp >= now() - make_interval(days => $2)
                   AND COALESCE(c.rationale, 0) <> 1
                   AND (
                       c.commit_id IN (SELECT commit_id FROM pending_commits)
                       OR EXISTS (SELECT 1 FROM anki.notifications n WHERE n.commit_id = c.commit_id)
                       OR EXISTS (
                           SELECT 1 FROM anki.note_events ne
                           WHERE ne.commit_id = c.commit_id
                             AND ne.event_type IN (
                                 'commit_approved_effect', 'commit_denied_effect', 'note_denied',
                                 'field_change_denied', 'tag_change_denied', 'suggestion_denied'
                             )
                       )
                   )
             ),
             period_decisions AS (
                 SELECT
                     COUNT(DISTINCT d.commit_id) FILTER (WHERE d.status = 'approved')::bigint AS approved,
                     COUNT(DISTINCT d.commit_id) FILTER (WHERE d.status = 'denied')::bigint AS denied
                 FROM (
                     SELECT n.commit_id, n.status
                     FROM anki.notifications n
                     WHERE n.deck_id IN (SELECT id FROM subtree)
                       AND n.created_at >= now() - make_interval(days => $2)
                     UNION
                     SELECT ne.commit_id,
                            CASE
                                WHEN ne.event_type = 'commit_approved_effect' THEN 'approved'
                                ELSE 'denied'
                            END AS status
                     FROM anki.note_events ne
                     JOIN anki.commits c ON c.commit_id = ne.commit_id
                     WHERE ne.commit_id IS NOT NULL
                       AND c.deck IN (SELECT id FROM subtree)
                       AND ne.created_at >= now() - make_interval(days => $2)
                       AND ne.event_type IN ('commit_approved_effect', 'commit_denied_effect', 'note_denied')
                 ) AS d
             ),
             subs AS (
                 SELECT
                     COUNT(*)::bigint AS total_subs,
                     COUNT(*) FILTER (WHERE s.last_synced_at >= now() - INTERVAL '30 days')::bigint AS active_30d
                 FROM anki.subscriptions s
                 WHERE s.deck_id IN (SELECT id FROM subtree)
             ),
             base_snap AS (
                 SELECT subscriber_count::bigint AS cnt
                 FROM anki.deck_stats_daily
                 WHERE deck_id = $1 AND captured_on <= CURRENT_DATE - $2
                 ORDER BY captured_on DESC
                 LIMIT 1
             ),
             leeches AS (
                 SELECT COUNT(*)::bigint AS leech_count
                 FROM anki.notes n
                 JOIN anki.calculated_stats cs ON cs.note_id = n.id
                 WHERE n.deck IN (SELECT id FROM subtree)
                   AND NOT n.deleted
                   AND cs.sample_size >= 5
                   AND (cs.retention < 70.0 OR cs.lapses >= 4.0)
             )
             SELECT
                 (SELECT COUNT(*)::bigint FROM pending_commits) AS open_suggestions,
                 (SELECT EXTRACT(EPOCH FROM (now() - MIN(timestamp)))::bigint / 86400 FROM pending_commits) AS oldest_days,
                 (SELECT submitted FROM period_commits) AS submitted,
                 (SELECT approved FROM period_decisions) AS approved,
                 (SELECT denied FROM period_decisions) AS denied,
                 (SELECT total_subs FROM subs) AS total_subscribers,
                 (SELECT active_30d FROM subs) AS active_30d,
                 (SELECT cnt FROM base_snap) AS base_cnt,
                 (SELECT leech_count FROM leeches) AS leech_count",
            &[&cand.deck_id, &lookback_days],
        )
        .await?;

    let open_suggestions: i64 = row.get(0);
    let oldest_pending_days: Option<i64> = row.get(1);
    let submitted: i64 = row.get(2);
    let approved: i64 = row.get(3);
    let denied: i64 = row.get(4);
    let total_subscribers: i64 = row.get(5);
    let active_30d: i64 = row.get(6);
    let base_cnt: Option<i64> = row.get(7);
    let leech_count: i64 = row.get(8);

    let net_subscribers = base_cnt.map(|base| total_subscribers - base);

    if cand.skip_empty
        && open_suggestions == 0
        && submitted == 0
        && approved == 0
        && denied == 0
        && net_subscribers.unwrap_or(0) == 0
    {
        return Ok(None);
    }

    let retention_val = if cand.stats_enabled {
        cand.retention.map(f64::from)
    } else {
        None
    };
    let leech_val = if cand.stats_enabled {
        Some(leech_count)
    } else {
        None
    };

    Ok(Some(json!({
        "deck_name": cand.deck_name,
        "deck_hash": cand.deck_hash,
        "period_label": period_label,
        "open_suggestions": open_suggestions,
        "oldest_pending_days": oldest_pending_days,
        "submitted": submitted,
        "approved": approved,
        "denied": denied,
        "total_subscribers": total_subscribers,
        "net_subscribers": net_subscribers,
        "active_30d": active_30d,
        "retention": retention_val,
        "leech_count": leech_val,
    })))
}

pub async fn process_due_digests(state: &Arc<AppState>) -> usize {
    if state.webhook_cipher.is_none() {
        return 0;
    }

    let Ok(client) = database::client(state).await else {
        return 0;
    };

    let rows = match client
        .query(
            "SELECT w.id,
                    w.deck_id,
                    COALESCE(d.name, 'Unnamed Deck'),
                    COALESCE(d.human_hash, ''),
                    COALESCE(d.stats_enabled, false),
                    d.retention,
                    w.digest_frequency,
                    w.digest_hour_utc,
                    w.digest_day_of_week,
                    w.skip_empty_digest,
                    w.last_digest_sent_at
             FROM anki.deck_webhooks w
             JOIN anki.decks d ON d.id = w.deck_id
             WHERE w.enabled = true
               AND w.digest_frequency IN ('daily', 'weekly')
               AND d.deleted_at IS NULL",
            &[],
        )
        .await
    {
        Ok(rows) => rows,
        Err(err) if is_undefined_table(&err) => return 0,
        Err(err) => {
            tracing::warn!(error = %err, "Failed to query webhook digest candidates");
            return 0;
        }
    };

    let now = OffsetDateTime::now_utc();
    let mut enqueued = 0;

    for row in rows {
        let cand = DigestCandidate {
            webhook_id: row.get(0),
            deck_id: row.get(1),
            deck_name: row.get(2),
            deck_hash: row.get(3),
            stats_enabled: row.get(4),
            retention: row.get(5),
            frequency: row.get(6),
            hour_utc: row.get(7),
            day_of_week: row.get(8),
            skip_empty: row.get(9),
            last_sent: row.get(10),
        };

        if !is_digest_due(
            now,
            &cand.frequency,
            cand.hour_utc,
            cand.day_of_week,
            cand.last_sent,
        ) {
            continue;
        }

        let payload_opt = match build_digest_payload(&client, &cand).await {
            Ok(opt) => opt,
            Err(err) => {
                tracing::warn!(error = %err, webhook_id = cand.webhook_id, "Failed to build periodic digest payload");
                continue;
            }
        };

        if let Some(payload) = payload_opt {
            match client
                .execute(
                    "WITH claimed AS (
                         UPDATE anki.deck_webhooks
                         SET last_digest_sent_at = now(), updated_at = now()
                         WHERE id = $1
                           AND enabled = true
                           AND (last_digest_sent_at IS NULL OR last_digest_sent_at < now() - INTERVAL '12 hours')
                         RETURNING id
                     )
                     INSERT INTO anki.webhook_deliveries (webhook_id, event_type, payload)
                     SELECT id, 'periodic_digest', $2
                     FROM claimed",
                    &[&cand.webhook_id, &payload],
                )
                .await
            {
                Ok(rows_affected) if rows_affected > 0 => enqueued += 1,
                Ok(_) => {}
                Err(err) => {
                    tracing::warn!(error = %err, webhook_id = cand.webhook_id, "Failed to enqueue periodic digest");
                }
            }
        } else if let Err(err) = client
            .execute(
                "UPDATE anki.deck_webhooks
                 SET last_digest_sent_at = now(), updated_at = now()
                 WHERE id = $1
                   AND enabled = true
                   AND (last_digest_sent_at IS NULL OR last_digest_sent_at < now() - INTERVAL '12 hours')",
                &[&cand.webhook_id],
            )
            .await
        {
            tracing::warn!(error = %err, webhook_id = cand.webhook_id, "Failed to record skipped periodic digest");
        }
    }

    enqueued
}

pub async fn prune_old_deliveries(state: &Arc<AppState>) {
    let Ok(client) = database::client(state).await else {
        return;
    };
    match client
        .execute(
            "DELETE FROM anki.webhook_deliveries
             WHERE state IN ('done', 'failed')
               AND COALESCE(finished_at, created_at) < now() - make_interval(days => $1)",
            &[&RETENTION_DAYS],
        )
        .await
    {
        Ok(0) => {}
        Ok(deleted) => tracing::info!(deleted, "Pruned finished webhook deliveries"),
        Err(err) if is_undefined_table(&err) => {}
        Err(err) => tracing::warn!(error = %err, "Failed to prune finished webhook deliveries"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::{Date, Month};

    fn utc(year: i32, month: Month, day: u8, hour: u8, minute: u8) -> OffsetDateTime {
        Date::from_calendar_date(year, month, day)
            .unwrap()
            .with_hms(hour, minute, 0)
            .unwrap()
            .assume_utc()
    }

    #[test]
    fn digest_due_none_never_fires() {
        let now = utc(2026, Month::November, 26, 12, 0);
        assert!(!is_digest_due(now, "none", 9, 1, None));
    }

    #[test]
    fn digest_due_daily_respects_hour_and_same_day() {
        let before = utc(2026, Month::November, 26, 8, 59);
        assert!(!is_digest_due(before, "daily", 9, 1, None));

        let at_hour = utc(2026, Month::November, 26, 9, 0);
        assert!(is_digest_due(at_hour, "daily", 9, 1, None));

        let sent_today = utc(2026, Month::November, 26, 9, 0);
        let later_today = utc(2026, Month::November, 26, 18, 0);
        assert!(!is_digest_due(later_today, "daily", 9, 1, Some(sent_today)));

        let next_day = utc(2026, Month::November, 27, 9, 5);
        assert!(is_digest_due(next_day, "daily", 9, 1, Some(sent_today)));
    }

    #[test]
    fn digest_due_weekly_respects_weekday_and_interval() {
        // 2026-11-26 is a Thursday (weekday 4)
        let thu = utc(2026, Month::November, 26, 10, 0);
        assert!(!is_digest_due(thu, "weekly", 9, 1, None));
        assert!(is_digest_due(thu, "weekly", 9, 4, None));

        let sent_recently = utc(2026, Month::November, 24, 9, 0);
        assert!(!is_digest_due(thu, "weekly", 9, 4, Some(sent_recently)));

        let sent_last_week = utc(2026, Month::November, 19, 9, 0);
        assert!(is_digest_due(thu, "weekly", 9, 4, Some(sent_last_week)));
    }

    #[test]
    fn parse_retry_after_clamps_and_defaults() {
        let mut headers = reqwest::header::HeaderMap::new();
        assert_eq!(parse_retry_after_secs(&headers, None), 30);

        headers.insert(reqwest::header::RETRY_AFTER, "2".parse().unwrap());
        assert_eq!(parse_retry_after_secs(&headers, None), 5);

        headers.insert(reqwest::header::RETRY_AFTER, "12.3".parse().unwrap());
        assert_eq!(parse_retry_after_secs(&headers, None), 13);

        headers.insert(reqwest::header::RETRY_AFTER, "9999".parse().unwrap());
        assert_eq!(parse_retry_after_secs(&headers, None), 300);

        headers.insert("x-ratelimit-reset-after", "17.2".parse().unwrap());
        assert_eq!(parse_retry_after_secs(&headers, None), 18);

        let empty_headers = reqwest::header::HeaderMap::new();
        let body = json!({ "retry_after": 9.4 });
        assert_eq!(parse_retry_after_secs(&empty_headers, Some(&body)), 10);
    }

    #[test]
    fn classify_discord_errors_distinguishes_permanent_and_transient() {
        let forum_body = json!({ "code": 220_001, "message": "Requires thread_id" });
        let (msg, disable) =
            classify_discord_error(reqwest::StatusCode::BAD_REQUEST, Some(&forum_body));
        assert!(disable);
        assert!(msg.contains("thread_id"));

        let unknown_wh = json!({ "code": 10_015 });
        let (msg, disable) =
            classify_discord_error(reqwest::StatusCode::NOT_FOUND, Some(&unknown_wh));
        assert!(disable);
        assert!(msg.contains("Unknown Webhook"));

        let bad_tok = json!({ "code": 50_027 });
        let (msg, disable) =
            classify_discord_error(reqwest::StatusCode::UNAUTHORIZED, Some(&bad_tok));
        assert!(disable);
        assert!(msg.contains("token"));

        for status in [
            reqwest::StatusCode::UNAUTHORIZED,
            reqwest::StatusCode::FORBIDDEN,
            reqwest::StatusCode::NOT_FOUND,
            reqwest::StatusCode::GONE,
        ] {
            let (_, disable) = classify_discord_error(status, None);
            assert!(disable, "expected auto-disable for {status}");
        }

        let (_, disable_400) = classify_discord_error(reqwest::StatusCode::BAD_REQUEST, None);
        assert!(!disable_400);

        let (_, disable_500) =
            classify_discord_error(reqwest::StatusCode::INTERNAL_SERVER_ERROR, None);
        assert!(!disable_500);
    }

    #[test]
    fn webhook_test_cooldown_enforces_window_and_allows_other_webhooks() {
        let cooldown = WebhookTestCooldown::new();
        let t0 = Instant::now();
        assert!(cooldown.try_acquire_at(1, t0).is_ok());
        assert!(cooldown.try_acquire_at(2, t0).is_ok());

        let err = cooldown
            .try_acquire_at(1, t0 + Duration::from_secs(2))
            .unwrap_err();
        assert_eq!(err, 3);

        assert!(cooldown.try_acquire_at(1, t0 + TEST_COOLDOWN).is_ok());
    }

    #[test]
    fn backoff_increases_and_caps() {
        assert_eq!(backoff_seconds(1), 60);
        assert_eq!(backoff_seconds(2), 120);
        assert_eq!(backoff_seconds(3), 300);
        assert_eq!(backoff_seconds(4), 900);
        assert_eq!(backoff_seconds(5), 1800);
    }
}
