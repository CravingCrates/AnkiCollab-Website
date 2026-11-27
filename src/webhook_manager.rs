use std::sync::{Arc, LazyLock};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::commit_manager::get_string_from_rationale;
use crate::database::{self, AppState};
use crate::error::Error;
use crate::webhook_crypto::WebhookCipher;
use crate::Return;

pub const MAX_WEBHOOKS_PER_DECK: i64 = 5;
pub const MAX_LABEL_LEN: usize = 64;

pub const SUBSCRIBER_MILESTONES: [i32; 12] = [
    25, 50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 25_000, 50_000, 100_000,
];

pub const ALLOWED_DISCORD_HOSTS: [&str; 4] = [
    "discord.com",
    "discordapp.com",
    "ptb.discord.com",
    "canary.discord.com",
];

static RE_WEBHOOK_PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^/api(?:/v[0-9]{1,2})?/webhooks/([0-9]{16,22})/([A-Za-z0-9_-]{50,160})$")
        .expect("valid Discord webhook path regex")
});

static RE_THREAD_QUERY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^thread_id=([0-9]{16,22})$").expect("valid Discord thread_id query regex")
});

static RE_BLOCK_BREAKS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)<br\s*/?>|</(?:p|div|li)>").expect("valid block break regex")
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedWebhookUrl {
    pub canonical_url: String,
    pub preview: String,
}

/// Validates that `raw` is strictly an HTTPS Discord webhook URL (optionally with
/// `?thread_id=<snowflake>`) and returns a normalized URL plus a masked preview
/// that never exposes the webhook secret token.
pub fn validate_discord_webhook_url(raw: &str) -> Result<ValidatedWebhookUrl, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Webhook URL is required.".to_string());
    }
    if trimmed.len() > 300 {
        return Err("Webhook URL is too long.".to_string());
    }
    if trimmed.chars().any(|c| c.is_whitespace() || c.is_control())
        || trimmed.contains('@')
        || trimmed.contains('#')
        || trimmed.contains('\\')
    {
        return Err("Webhook URL contains invalid characters.".to_string());
    }

    let Some(scheme_prefix) = trimmed.get(..8) else {
        return Err("Webhook URL must start with https://.".to_string());
    };
    if !scheme_prefix.eq_ignore_ascii_case("https://") {
        return Err("Webhook URL must use HTTPS.".to_string());
    }

    let rest = &trimmed[8..];
    let Some(slash_idx) = rest.find('/') else {
        return Err("Invalid Discord webhook URL path.".to_string());
    };
    let host_raw = &rest[..slash_idx];
    let host = host_raw.to_ascii_lowercase();
    if !ALLOWED_DISCORD_HOSTS.contains(&host.as_str()) {
        return Err(
            "Webhook URL must point to discord.com, discordapp.com, ptb.discord.com, or canary.discord.com."
                .to_string(),
        );
    }

    let path_and_query = &rest[slash_idx..];
    let (raw_path, thread_id) = match path_and_query.split_once('?') {
        Some((p, q)) => {
            let caps = RE_THREAD_QUERY.captures(q).ok_or_else(|| {
                "Only an optional ?thread_id=<digits> query parameter is allowed.".to_string()
            })?;
            (p, Some(caps[1].to_string()))
        }
        None => (path_and_query, None),
    };

    let normalized_path = raw_path.strip_suffix('/').unwrap_or(raw_path);
    let caps = RE_WEBHOOK_PATH.captures(normalized_path).ok_or_else(|| {
        "Invalid Discord webhook URL format. Expected https://discord.com/api/webhooks/<id>/<token>."
            .to_string()
    })?;

    let webhook_id = &caps[1];
    let token = &caps[2];

    let canonical_url = match &thread_id {
        Some(tid) => format!("https://{host}/api/webhooks/{webhook_id}/{token}?thread_id={tid}"),
        None => format!("https://{host}/api/webhooks/{webhook_id}/{token}"),
    };

    let id_prefix: String = webhook_id.chars().take(4).collect();
    let token_suffix: String = token
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let preview = match &thread_id {
        Some(tid) => {
            let tid_suffix: String = tid
                .chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("{host}/api/webhooks/{id_prefix}…/••••{token_suffix} (thread …{tid_suffix})")
        }
        None => format!("{host}/api/webhooks/{id_prefix}…/••••{token_suffix}"),
    };

    Ok(ValidatedWebhookUrl {
        canonical_url,
        preview,
    })
}

const fn default_true() -> bool {
    true
}

fn default_none() -> String {
    "none".to_string()
}

const fn default_digest_hour() -> i16 {
    9
}

const fn default_digest_day() -> i16 {
    1
}

#[derive(Debug, Clone, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct WebhookView {
    pub id: i64,
    pub label: String,
    pub url_preview: String,
    pub enabled: bool,
    pub on_suggestion_submitted: bool,
    pub on_suggestion_approved: bool,
    pub on_suggestion_denied: bool,
    pub on_direct_push: bool,
    pub on_changelog_published: bool,
    pub on_maintainer_changed: bool,
    pub on_subscriber_milestone: bool,
    pub active_event_count: usize,
    pub digest_frequency: String,
    pub digest_hour_utc: i16,
    pub digest_day_of_week: i16,
    pub skip_empty_digest: bool,
    pub consecutive_failures: i32,
    pub last_error: Option<String>,
    pub last_delivered_at: Option<String>,
    pub disabled_reason: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct CreateWebhookInput {
    #[serde(default)]
    pub label: String,
    pub webhook_url: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub on_suggestion_submitted: bool,
    #[serde(default)]
    pub on_suggestion_approved: bool,
    #[serde(default)]
    pub on_suggestion_denied: bool,
    #[serde(default)]
    pub on_direct_push: bool,
    #[serde(default)]
    pub on_changelog_published: bool,
    #[serde(default)]
    pub on_maintainer_changed: bool,
    #[serde(default)]
    pub on_subscriber_milestone: bool,
    #[serde(default = "default_none")]
    pub digest_frequency: String,
    #[serde(default = "default_digest_hour")]
    pub digest_hour_utc: i16,
    #[serde(default = "default_digest_day")]
    pub digest_day_of_week: i16,
    #[serde(default = "default_true")]
    pub skip_empty_digest: bool,
}

#[derive(Debug, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct UpdateWebhookInput {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub webhook_url: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub on_suggestion_submitted: bool,
    #[serde(default)]
    pub on_suggestion_approved: bool,
    #[serde(default)]
    pub on_suggestion_denied: bool,
    #[serde(default)]
    pub on_direct_push: bool,
    #[serde(default)]
    pub on_changelog_published: bool,
    #[serde(default)]
    pub on_maintainer_changed: bool,
    #[serde(default)]
    pub on_subscriber_milestone: bool,
    #[serde(default = "default_none")]
    pub digest_frequency: String,
    #[serde(default = "default_digest_hour")]
    pub digest_hour_utc: i16,
    #[serde(default = "default_digest_day")]
    pub digest_day_of_week: i16,
    #[serde(default = "default_true")]
    pub skip_empty_digest: bool,
}

fn validate_settings(
    label: &str,
    digest_frequency: &str,
    digest_hour_utc: i16,
    digest_day_of_week: i16,
) -> Result<String, Error> {
    let cleaned_label: String = label.trim().chars().filter(|c| !c.is_control()).collect();
    if cleaned_label.chars().count() > MAX_LABEL_LEN {
        return Err(Error::BadRequest(format!(
            "Label must be at most {MAX_LABEL_LEN} characters."
        )));
    }
    if !matches!(digest_frequency, "none" | "daily" | "weekly") {
        return Err(Error::BadRequest(
            "Digest frequency must be 'none', 'daily', or 'weekly'.".to_string(),
        ));
    }
    if !(0..=23).contains(&digest_hour_utc) {
        return Err(Error::BadRequest(
            "Digest hour (UTC) must be between 0 and 23.".to_string(),
        ));
    }
    if !(1..=7).contains(&digest_day_of_week) {
        return Err(Error::BadRequest(
            "Digest day of week must be between 1 (Monday) and 7 (Sunday).".to_string(),
        ));
    }
    Ok(cleaned_label)
}

fn row_to_view(row: &tokio_postgres::Row) -> WebhookView {
    let on_suggestion_submitted: bool = row.get(4);
    let on_suggestion_approved: bool = row.get(5);
    let on_suggestion_denied: bool = row.get(6);
    let on_direct_push: bool = row.get(7);
    let on_changelog_published: bool = row.get(8);
    let on_maintainer_changed: bool = row.get(9);
    let on_subscriber_milestone: bool = row.get(10);
    let active_event_count = [
        on_suggestion_submitted,
        on_suggestion_approved,
        on_suggestion_denied,
        on_direct_push,
        on_changelog_published,
        on_maintainer_changed,
        on_subscriber_milestone,
    ]
    .into_iter()
    .filter(|&enabled| enabled)
    .count();

    WebhookView {
        id: row.get(0),
        label: row.get(1),
        url_preview: row.get(2),
        enabled: row.get(3),
        on_suggestion_submitted,
        on_suggestion_approved,
        on_suggestion_denied,
        on_direct_push,
        on_changelog_published,
        on_maintainer_changed,
        on_subscriber_milestone,
        active_event_count,
        digest_frequency: row.get(11),
        digest_hour_utc: row.get(12),
        digest_day_of_week: row.get(13),
        skip_empty_digest: row.get(14),
        consecutive_failures: row.get(15),
        last_error: row.get(16),
        last_delivered_at: row.get(17),
        disabled_reason: row.get(18),
        created_at: row.get(19),
    }
}

const VIEW_COLUMNS: &str = "
    id,
    label,
    url_preview,
    enabled,
    on_suggestion_submitted,
    on_suggestion_approved,
    on_suggestion_denied,
    on_direct_push,
    on_changelog_published,
    on_maintainer_changed,
    on_subscriber_milestone,
    digest_frequency,
    digest_hour_utc,
    digest_day_of_week,
    skip_empty_digest,
    consecutive_failures,
    last_error,
    TO_CHAR(last_delivered_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI \"UTC\"'),
    disabled_reason,
    TO_CHAR(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI \"UTC\"')
";

#[must_use]
pub fn is_undefined_table(err: &tokio_postgres::Error) -> bool {
    err.code()
        .is_some_and(|code| *code == tokio_postgres::error::SqlState::UNDEFINED_TABLE)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnedDeckResolution {
    Root { id: i64, name: String },
    Subdeck { root_hash: String },
}

pub async fn resolve_owned_deck(
    db_state: &AppState,
    deck_hash: &str,
    user_id: i32,
) -> Return<OwnedDeckResolution> {
    let client = database::client(db_state).await?;
    let Some(row) = client
        .query_opt(
            "SELECT id, parent, COALESCE(name, 'Unnamed Deck')
             FROM anki.decks
             WHERE human_hash = $1 AND owner = $2 AND deleted_at IS NULL",
            &[&deck_hash, &user_id],
        )
        .await?
    else {
        return Err(Error::Unauthorized);
    };

    let id: i64 = row.get(0);
    let parent: Option<i64> = row.get(1);
    let name: String = row.get(2);

    if parent.is_none() {
        return Ok(OwnedDeckResolution::Root { id, name });
    }

    let Some((root_id, _, root_hash)) = resolve_root_deck(&client, id).await? else {
        return Err(Error::Unauthorized);
    };
    if root_hash.is_empty() {
        return Err(Error::Unauthorized);
    }

    let root_owned = client
        .query_opt(
            "SELECT 1 FROM anki.decks WHERE id = $1 AND parent IS NULL AND owner = $2 AND deleted_at IS NULL",
            &[&root_id, &user_id],
        )
        .await?
        .is_some();
    if !root_owned {
        return Err(Error::Unauthorized);
    }

    Ok(OwnedDeckResolution::Subdeck { root_hash })
}

pub async fn list_for_deck(db_state: &AppState, deck_id: i64) -> Return<Vec<WebhookView>> {
    let client = database::client(db_state).await?;
    let query = format!(
        "SELECT {VIEW_COLUMNS}
         FROM anki.deck_webhooks
         WHERE deck_id = $1
         ORDER BY created_at ASC, id ASC"
    );
    match client.query(&query, &[&deck_id]).await {
        Ok(rows) => Ok(rows.iter().map(row_to_view).collect()),
        Err(err) if is_undefined_table(&err) => {
            tracing::warn!("anki.deck_webhooks table missing; returning empty webhook list");
            Ok(Vec::new())
        }
        Err(err) => Err(err.into()),
    }
}

pub async fn get_webhook_view(
    db_state: &AppState,
    deck_id: i64,
    webhook_id: i64,
) -> Return<Option<WebhookView>> {
    let client = database::client(db_state).await?;
    let query = format!(
        "SELECT {VIEW_COLUMNS}
         FROM anki.deck_webhooks
         WHERE id = $1 AND deck_id = $2"
    );
    let row_opt = client.query_opt(&query, &[&webhook_id, &deck_id]).await?;
    Ok(row_opt.as_ref().map(row_to_view))
}

async fn current_subtree_subscriber_count(
    client: &impl tokio_postgres::GenericClient,
    deck_id: i64,
) -> Return<i32> {
    let row = client
        .query_one(
            "WITH RECURSIVE subtree AS (
                 SELECT id FROM anki.decks WHERE id = $1 AND deleted_at IS NULL
                 UNION ALL
                 SELECT d.id FROM anki.decks d JOIN subtree s ON d.parent = s.id
                 WHERE d.deleted_at IS NULL
             )
             SELECT COUNT(*)::int
             FROM anki.subscriptions s
             WHERE s.deck_id IN (SELECT id FROM subtree)",
            &[&deck_id],
        )
        .await?;
    Ok(row.get(0))
}

pub async fn create_webhook(
    db_state: &AppState,
    cipher: &WebhookCipher,
    deck_id: i64,
    input: CreateWebhookInput,
) -> Return<WebhookView> {
    let label = validate_settings(
        &input.label,
        &input.digest_frequency,
        input.digest_hour_utc,
        input.digest_day_of_week,
    )?;
    let validated_url =
        validate_discord_webhook_url(&input.webhook_url).map_err(Error::BadRequest)?;

    let mut client = database::client(db_state).await?;
    let tx = client.transaction().await?;

    let deck_exists = tx
        .query_opt(
            "SELECT id FROM anki.decks WHERE id = $1 AND parent IS NULL AND deleted_at IS NULL FOR UPDATE",
            &[&deck_id],
        )
        .await?
        .is_some();
    if !deck_exists {
        return Err(Error::DeckNotFound);
    }

    let count_row = tx
        .query_one(
            "SELECT COUNT(*)::bigint FROM anki.deck_webhooks WHERE deck_id = $1",
            &[&deck_id],
        )
        .await?;
    let existing_count: i64 = count_row.get(0);
    if existing_count >= MAX_WEBHOOKS_PER_DECK {
        return Err(Error::BadRequest(format!(
            "A deck can have at most {MAX_WEBHOOKS_PER_DECK} webhooks."
        )));
    }

    let (ciphertext, nonce) = cipher
        .encrypt_url(deck_id, &validated_url.canonical_url)
        .map_err(|_| Error::Unknown)?;
    let nonce_slice: &[u8] = &nonce;

    // Seed milestone baseline to current subscribers so existing decks do not
    // immediately emit historical milestones on creation.
    let current_subs = current_subtree_subscriber_count(&tx, deck_id).await?;
    let initial_milestone = SUBSCRIBER_MILESTONES
        .iter()
        .copied()
        .filter(|&m| m <= current_subs)
        .max()
        .unwrap_or(0);

    let insert_sql = format!(
        "INSERT INTO anki.deck_webhooks (
             deck_id, label, webhook_url_ciphertext, webhook_url_nonce, url_preview,
             enabled, on_suggestion_submitted, on_suggestion_approved, on_suggestion_denied,
             on_direct_push, on_changelog_published, on_maintainer_changed,
             on_subscriber_milestone, digest_frequency, digest_hour_utc, digest_day_of_week,
             skip_empty_digest, last_milestone_notified
         ) VALUES (
             $1, $2, $3, $4, $5,
             $6, $7, $8, $9,
             $10, $11, $12,
             $13, $14, $15, $16,
             $17, $18
         )
         RETURNING {VIEW_COLUMNS}"
    );

    let row = tx
        .query_one(
            &insert_sql,
            &[
                &deck_id,
                &label,
                &ciphertext,
                &nonce_slice,
                &validated_url.preview,
                &input.enabled,
                &input.on_suggestion_submitted,
                &input.on_suggestion_approved,
                &input.on_suggestion_denied,
                &input.on_direct_push,
                &input.on_changelog_published,
                &input.on_maintainer_changed,
                &input.on_subscriber_milestone,
                &input.digest_frequency,
                &input.digest_hour_utc,
                &input.digest_day_of_week,
                &input.skip_empty_digest,
                &initial_milestone,
            ],
        )
        .await?;

    tx.commit().await?;
    Ok(row_to_view(&row))
}

pub async fn update_webhook(
    db_state: &AppState,
    cipher: &WebhookCipher,
    deck_id: i64,
    webhook_id: i64,
    input: UpdateWebhookInput,
) -> Return<WebhookView> {
    let label = validate_settings(
        &input.label,
        &input.digest_frequency,
        input.digest_hour_utc,
        input.digest_day_of_week,
    )?;

    let new_url = match input.webhook_url.as_deref().map(str::trim) {
        Some(raw) if !raw.is_empty() => {
            let validated = validate_discord_webhook_url(raw).map_err(Error::BadRequest)?;
            let (ct, nonce) = cipher
                .encrypt_url(deck_id, &validated.canonical_url)
                .map_err(|_| Error::Unknown)?;
            Some((ct, nonce.to_vec(), validated.preview))
        }
        _ => None,
    };

    let client = database::client(db_state).await?;
    let row_opt = if let Some((ciphertext, nonce, preview)) = new_url {
        let sql = format!(
            "UPDATE anki.deck_webhooks
             SET label = $3,
                 webhook_url_ciphertext = $4,
                 webhook_url_nonce = $5,
                 url_preview = $6,
                 enabled = $7,
                 on_suggestion_submitted = $8,
                 on_suggestion_approved = $9,
                 on_suggestion_denied = $10,
                 on_direct_push = $11,
                 on_changelog_published = $12,
                 on_maintainer_changed = $13,
                 on_subscriber_milestone = $14,
                 digest_frequency = $15,
                 digest_hour_utc = $16,
                 digest_day_of_week = $17,
                 skip_empty_digest = $18,
                 consecutive_failures = CASE WHEN $7 THEN 0 ELSE consecutive_failures END,
                 disabled_reason = CASE WHEN $7 THEN NULL ELSE disabled_reason END,
                 last_error = CASE WHEN $7 THEN NULL ELSE last_error END,
                 updated_at = now()
             WHERE id = $1 AND deck_id = $2
             RETURNING {VIEW_COLUMNS}"
        );
        client
            .query_opt(
                &sql,
                &[
                    &webhook_id,
                    &deck_id,
                    &label,
                    &ciphertext,
                    &nonce,
                    &preview,
                    &input.enabled,
                    &input.on_suggestion_submitted,
                    &input.on_suggestion_approved,
                    &input.on_suggestion_denied,
                    &input.on_direct_push,
                    &input.on_changelog_published,
                    &input.on_maintainer_changed,
                    &input.on_subscriber_milestone,
                    &input.digest_frequency,
                    &input.digest_hour_utc,
                    &input.digest_day_of_week,
                    &input.skip_empty_digest,
                ],
            )
            .await?
    } else {
        let sql = format!(
            "UPDATE anki.deck_webhooks
             SET label = $3,
                 enabled = $4,
                 on_suggestion_submitted = $5,
                 on_suggestion_approved = $6,
                 on_suggestion_denied = $7,
                 on_direct_push = $8,
                 on_changelog_published = $9,
                 on_maintainer_changed = $10,
                 on_subscriber_milestone = $11,
                 digest_frequency = $12,
                 digest_hour_utc = $13,
                 digest_day_of_week = $14,
                 skip_empty_digest = $15,
                 consecutive_failures = CASE WHEN $4 THEN 0 ELSE consecutive_failures END,
                 disabled_reason = CASE WHEN $4 THEN NULL ELSE disabled_reason END,
                 last_error = CASE WHEN $4 THEN NULL ELSE last_error END,
                 updated_at = now()
             WHERE id = $1 AND deck_id = $2
             RETURNING {VIEW_COLUMNS}"
        );
        client
            .query_opt(
                &sql,
                &[
                    &webhook_id,
                    &deck_id,
                    &label,
                    &input.enabled,
                    &input.on_suggestion_submitted,
                    &input.on_suggestion_approved,
                    &input.on_suggestion_denied,
                    &input.on_direct_push,
                    &input.on_changelog_published,
                    &input.on_maintainer_changed,
                    &input.on_subscriber_milestone,
                    &input.digest_frequency,
                    &input.digest_hour_utc,
                    &input.digest_day_of_week,
                    &input.skip_empty_digest,
                ],
            )
            .await?
    };

    let view = row_opt
        .as_ref()
        .map(row_to_view)
        .ok_or_else(|| Error::BadRequest("Webhook not found for this deck.".to_string()))?;

    if !input.enabled {
        let _ = client
            .execute(
                "DELETE FROM anki.webhook_deliveries WHERE webhook_id = $1 AND state = 'pending'",
                &[&webhook_id],
            )
            .await;
    }

    Ok(view)
}

pub async fn delete_webhook(db_state: &AppState, deck_id: i64, webhook_id: i64) -> Return<bool> {
    let client = database::client(db_state).await?;
    let deleted = client
        .execute(
            "DELETE FROM anki.deck_webhooks WHERE id = $1 AND deck_id = $2",
            &[&webhook_id, &deck_id],
        )
        .await?;
    Ok(deleted > 0)
}

pub async fn load_webhook_url_for_test(
    db_state: &AppState,
    cipher: &WebhookCipher,
    deck_id: i64,
    webhook_id: i64,
) -> Return<Option<(String, String, String)>> {
    let client = database::client(db_state).await?;
    let row_opt = client
        .query_opt(
            "SELECT w.webhook_url_ciphertext,
                    w.webhook_url_nonce,
                    COALESCE(d.name, 'Unnamed Deck'),
                    COALESCE(d.human_hash, '')
             FROM anki.deck_webhooks w
             JOIN anki.decks d ON d.id = w.deck_id
             WHERE w.id = $1
               AND w.deck_id = $2
               AND d.deleted_at IS NULL
               AND d.parent IS NULL",
            &[&webhook_id, &deck_id],
        )
        .await?;

    let Some(row) = row_opt else {
        return Ok(None);
    };
    let ciphertext: Vec<u8> = row.get(0);
    let nonce: Vec<u8> = row.get(1);
    let deck_name: String = row.get(2);
    let deck_hash: String = row.get(3);
    let url = cipher
        .decrypt_url(deck_id, &ciphertext, &nonce)
        .map_err(|_| Error::Unknown)?;
    Ok(Some((url, deck_name, deck_hash)))
}

async fn resolve_root_deck(
    client: &tokio_postgres::Client,
    any_deck_id: i64,
) -> Result<Option<(i64, String, String)>, tokio_postgres::Error> {
    let row = client
        .query_opt(
            "WITH RECURSIVE up AS (
                 SELECT id, parent, name, human_hash FROM anki.decks WHERE id = $1 AND deleted_at IS NULL
                 UNION ALL
                 SELECT d.id, d.parent, d.name, d.human_hash
                 FROM anki.decks d
                 JOIN up ON d.id = up.parent
                 WHERE d.deleted_at IS NULL
             )
             SELECT id, COALESCE(name, 'Unnamed Deck'), COALESCE(human_hash, '')
             FROM up
             WHERE parent IS NULL
             LIMIT 1",
            &[&any_deck_id],
        )
        .await?;
    Ok(row.map(|r| (r.get(0), r.get(1), r.get(2))))
}

async fn enqueue_for_root_deck(
    client: &tokio_postgres::Client,
    root_deck_id: i64,
    event_type: &str,
    payload: &Value,
) -> Result<(), tokio_postgres::Error> {
    let sql = match event_type {
        "suggestion_submitted" => {
            "INSERT INTO anki.webhook_deliveries (webhook_id, event_type, payload)
             SELECT id, $2, $3
             FROM anki.deck_webhooks
             WHERE deck_id = $1 AND enabled = true AND on_suggestion_submitted = true"
        }
        "suggestion_approved" => {
            "INSERT INTO anki.webhook_deliveries (webhook_id, event_type, payload)
             SELECT id, $2, $3
             FROM anki.deck_webhooks
             WHERE deck_id = $1 AND enabled = true AND on_suggestion_approved = true"
        }
        "suggestion_denied" => {
            "INSERT INTO anki.webhook_deliveries (webhook_id, event_type, payload)
             SELECT id, $2, $3
             FROM anki.deck_webhooks
             WHERE deck_id = $1 AND enabled = true AND on_suggestion_denied = true"
        }
        "direct_push" => {
            "INSERT INTO anki.webhook_deliveries (webhook_id, event_type, payload)
             SELECT id, $2, $3
             FROM anki.deck_webhooks
             WHERE deck_id = $1 AND enabled = true AND on_direct_push = true"
        }
        "changelog_published" => {
            "INSERT INTO anki.webhook_deliveries (webhook_id, event_type, payload)
             SELECT id, $2, $3
             FROM anki.deck_webhooks
             WHERE deck_id = $1 AND enabled = true AND on_changelog_published = true"
        }
        "maintainer_changed" => {
            "INSERT INTO anki.webhook_deliveries (webhook_id, event_type, payload)
             SELECT id, $2, $3
             FROM anki.deck_webhooks
             WHERE deck_id = $1 AND enabled = true AND on_maintainer_changed = true"
        }
        _ => return Ok(()),
    };

    client
        .execute(sql, &[&root_deck_id, &event_type, payload])
        .await?;
    Ok(())
}

/// Counts distinct pending notes attached to `commit_id` prior to approval/denial.
pub async fn count_pending_commit_notes(db_state: &AppState, commit_id: i32) -> usize {
    let Ok(client) = database::client(db_state).await else {
        return 0;
    };
    let row = client
        .query_opt(
            "SELECT COUNT(DISTINCT note)::bigint FROM (
                 SELECT note FROM anki.fields WHERE commit = $1 AND reviewed = false
                 UNION ALL
                 SELECT note FROM anki.tags WHERE commit = $1 AND reviewed = false
                 UNION ALL
                 SELECT note FROM anki.card_deletion_suggestions WHERE commit = $1
                 UNION ALL
                 SELECT note FROM anki.note_move_suggestions WHERE commit = $1
             ) AS pending_notes",
            &[&commit_id],
        )
        .await;
    match row {
        Ok(Some(r)) => usize::try_from(r.get::<_, i64>(0).max(0)).unwrap_or(0),
        _ => 0,
    }
}

/// Returns `true` if `note_id` still has any unreviewed suggestion items in `commit_id`.
pub async fn note_has_pending_in_commit(db_state: &AppState, commit_id: i32, note_id: i64) -> bool {
    let Ok(client) = database::client(db_state).await else {
        return false;
    };
    let row = client
        .query_opt(
            "SELECT EXISTS (
                 SELECT 1 FROM anki.fields WHERE commit = $1 AND note = $2 AND reviewed = false
                 UNION ALL
                 SELECT 1 FROM anki.tags WHERE commit = $1 AND note = $2 AND reviewed = false
                 UNION ALL
                 SELECT 1 FROM anki.card_deletion_suggestions WHERE commit = $1 AND note = $2
                 UNION ALL
                 SELECT 1 FROM anki.note_move_suggestions WHERE commit = $1 AND note = $2
             )",
            &[&commit_id, &note_id],
        )
        .await;
    row.ok().flatten().is_some_and(|r| r.get(0))
}

/// Looks up a pending commit ID associated with `note_id`, if any.
pub async fn pending_commit_for_note(db_state: &AppState, note_id: i64) -> Option<i32> {
    let Ok(client) = database::client(db_state).await else {
        return None;
    };
    client
        .query_opt(
            "SELECT commit FROM (
                 SELECT commit FROM anki.fields WHERE note = $1 AND reviewed = false AND commit IS NOT NULL
                 UNION ALL
                 SELECT commit FROM anki.tags WHERE note = $1 AND reviewed = false AND commit IS NOT NULL
                 UNION ALL
                 SELECT commit FROM anki.card_deletion_suggestions WHERE note = $1 AND commit IS NOT NULL
                 UNION ALL
                 SELECT commit FROM anki.note_move_suggestions WHERE note = $1 AND commit IS NOT NULL
             ) AS pending
             LIMIT 1",
            &[&note_id],
        )
        .await
        .ok()
        .flatten()
        .map(|r| r.get(0))
}

#[allow(clippy::too_many_arguments)]
async fn enqueue_commit_decision_webhook(
    state: &AppState,
    commit_id: i32,
    approved: bool,
    reviewer_username: &str,
    reason: Option<&str>,
    silent: bool,
    note_count: usize,
    partial: bool,
) {
    let Ok(client) = database::client(state).await else {
        return;
    };
    let commit_row = match client
        .query_opt(
            "SELECT c.deck, COALESCE(c.rationale, 0), COALESCE(c.info, ''), COALESCE(u.username, 'Anonymous')
             FROM anki.commits c
             LEFT JOIN public.users u ON u.id = c.user_id
             WHERE c.commit_id = $1",
            &[&commit_id],
        )
        .await
    {
        Ok(Some(r)) => r,
        _ => return,
    };

    let Some(deck_id) = commit_row.get::<_, Option<i64>>(0) else {
        return;
    };
    let rationale_code: i32 = commit_row.get(1);
    let commit_info: String = commit_row.get(2);
    let contributor: String = commit_row.get(3);

    let Ok(Some((root_id, deck_name, deck_hash))) = resolve_root_deck(&client, deck_id).await
    else {
        return;
    };

    let event_type = if approved {
        "suggestion_approved"
    } else {
        "suggestion_denied"
    };
    let payload = json!({
        "commit_id": commit_id,
        "deck_name": deck_name,
        "deck_hash": deck_hash,
        "rationale": get_string_from_rationale(rationale_code),
        "commit_info": commit_info,
        "contributor": contributor,
        "reviewer": reviewer_username,
        "reason": reason,
        "silent": silent,
        "note_count": note_count,
        "partial": partial,
    });

    if let Err(err) = enqueue_for_root_deck(&client, root_id, event_type, &payload).await {
        if !is_undefined_table(&err) {
            tracing::warn!(error = %err, commit_id, "Failed to enqueue commit decision webhook");
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_commit_decision_webhook(
    state: Arc<AppState>,
    commit_id: i32,
    approved: bool,
    reviewer_username: String,
    reason: Option<String>,
    silent: bool,
    note_count: usize,
    partial: bool,
) {
    tokio::spawn(async move {
        enqueue_commit_decision_webhook(
            &state,
            commit_id,
            approved,
            &reviewer_username,
            reason.as_deref(),
            silent,
            note_count,
            partial,
        )
        .await;
    });
}

pub fn spawn_single_note_review_webhook(
    state: Arc<AppState>,
    commit_id: i32,
    note_id: i64,
    approved: bool,
    reviewer_username: String,
    check_note_cleared: bool,
) {
    tokio::spawn(async move {
        if check_note_cleared && note_has_pending_in_commit(&state, commit_id, note_id).await {
            return;
        }
        let remaining = count_pending_commit_notes(&state, commit_id).await;
        enqueue_commit_decision_webhook(
            &state,
            commit_id,
            approved,
            &reviewer_username,
            None,
            false,
            1,
            remaining > 0,
        )
        .await;
    });
}

pub fn spawn_changelog_webhook(
    state: Arc<AppState>,
    deck_hash: String,
    author_username: String,
    message: String,
) {
    if clean_raw_to_plain(&message, true).trim().is_empty() {
        return;
    }
    tokio::spawn(async move {
        let Ok(client) = database::client(&state).await else {
            return;
        };
        let deck_row = match client
            .query_opt(
                "SELECT id FROM anki.decks WHERE human_hash = $1 AND deleted_at IS NULL",
                &[&deck_hash],
            )
            .await
        {
            Ok(Some(r)) => r,
            _ => return,
        };
        let deck_id: i64 = deck_row.get(0);
        let Ok(Some((root_id, deck_name, root_hash))) = resolve_root_deck(&client, deck_id).await
        else {
            return;
        };

        let payload = json!({
            "deck_name": deck_name,
            "deck_hash": root_hash,
            "author": author_username,
            "message": message,
        });

        if let Err(err) =
            enqueue_for_root_deck(&client, root_id, "changelog_published", &payload).await
        {
            if !is_undefined_table(&err) {
                tracing::warn!(error = %err, deck_hash = %deck_hash, "Failed to enqueue changelog webhook");
            }
        }
    });
}

pub fn spawn_maintainer_webhook(
    state: Arc<AppState>,
    deck_id: i64,
    actor_username: String,
    target_username: String,
    added: bool,
) {
    tokio::spawn(async move {
        let Ok(client) = database::client(&state).await else {
            return;
        };
        let Ok(Some((root_id, deck_name, deck_hash))) = resolve_root_deck(&client, deck_id).await
        else {
            return;
        };

        let payload = json!({
            "deck_name": deck_name,
            "deck_hash": deck_hash,
            "actor": actor_username,
            "target": target_username,
            "action": if added { "added" } else { "removed" },
        });

        if let Err(err) =
            enqueue_for_root_deck(&client, root_id, "maintainer_changed", &payload).await
        {
            if !is_undefined_table(&err) {
                tracing::warn!(error = %err, deck_id, "Failed to enqueue maintainer webhook");
            }
        }
    });
}

/// Decodes common HTML entities in a single pass so `&amp;lt;` becomes `&lt;`, never `<`.
fn decode_html_entities(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(amp_idx) = rest.find('&') {
        out.push_str(&rest[..amp_idx]);
        let after_amp = &rest[amp_idx..];
        if let Some(semi_rel) = after_amp[1..].find(';') {
            if semi_rel <= 10 {
                let entity = &after_amp[..=semi_rel + 1];
                let decoded = match entity {
                    "&amp;" => Some('&'),
                    "&lt;" => Some('<'),
                    "&gt;" => Some('>'),
                    "&quot;" => Some('"'),
                    "&#39;" | "&#x27;" | "&#X27;" | "&apos;" => Some('\''),
                    "&nbsp;" | "&#160;" | "&#xA0;" | "&#xa0;" => Some(' '),
                    _ => None,
                };
                if let Some(ch) = decoded {
                    out.push(ch);
                    rest = &after_amp[semi_rel + 2..];
                    continue;
                }
            }
        }
        out.push('&');
        rest = &after_amp[1..];
    }
    out.push_str(rest);
    out
}

fn clean_raw_to_plain(text: &str, preserve_newlines: bool) -> String {
    let break_replacement = if preserve_newlines { "\n" } else { " " };
    let replaced = RE_BLOCK_BREAKS.replace_all(text, break_replacement);
    let stripped = ammonia::Builder::new()
        .tags(std::collections::HashSet::new())
        .clean(&replaced)
        .to_string();
    decode_html_entities(&stripped)
}

fn neutralize_discord_syntax(text: &str) -> String {
    // Belt-and-suspenders mention neutralization + escape masked links (`[text](url)`) & backticks.
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '@' => {
                out.push('@');
                out.push('\u{200B}');
            }
            '[' | ']' | '`' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

fn truncate_discord_safe(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    if max_chars == 1 {
        return "…".to_string();
    }
    let mut out: String = text.chars().take(max_chars - 1).collect();
    // Avoid leaving a trailing escape backslash if truncation cut right after `\`.
    let trailing_backslashes = out.chars().rev().take_while(|&c| c == '\\').count();
    if trailing_backslashes % 2 == 1 {
        out.pop();
    }
    out.push('…');
    out
}

/// Sanitizes untrusted single-line text for Discord embeds: strips HTML/control chars,
/// breaks `@everyone`/`@here`/user-role mention syntax and masked links, and truncates safely.
#[must_use]
pub fn sanitize_discord_text(text: &str, max_chars: usize) -> String {
    let plain = clean_raw_to_plain(text, false);
    let cleaned: String = plain
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let neutralized = neutralize_discord_syntax(&cleaned);
    truncate_discord_safe(&neutralized, max_chars)
}

/// Sanitizes untrusted multi-line text for Discord embeds while preserving line breaks.
#[must_use]
pub fn sanitize_discord_multiline(text: &str, max_chars: usize, max_lines: usize) -> String {
    let plain = clean_raw_to_plain(text, true);
    let mut lines: Vec<String> = Vec::new();
    let mut prev_blank = false;

    for raw_line in plain.split('\n') {
        let line: String = raw_line
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if line.is_empty() {
            if !lines.is_empty() && !prev_blank {
                lines.push(String::new());
                prev_blank = true;
            }
        } else {
            lines.push(line);
            prev_blank = false;
        }
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }

    let capped = if max_lines == 0 || lines.len() <= max_lines {
        lines.join("\n")
    } else {
        let head = lines[..max_lines - 1].join("\n");
        let tail = lines[max_lines - 1..]
            .iter()
            .filter(|l| !l.is_empty())
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        if head.is_empty() {
            tail
        } else if tail.is_empty() {
            head
        } else {
            format!("{head}\n{tail}")
        }
    };

    let neutralized = neutralize_discord_syntax(&capped);
    truncate_discord_safe(&neutralized, max_chars)
}

fn sanitize_discord_field(text: &str, fallback: &str, max_chars: usize) -> String {
    let cleaned = sanitize_discord_text(text, max_chars);
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned
    }
}

fn escape_discord_inline_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '*' | '_' | '~' | '`' | '|' | '>') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn deck_url(origin: &str, prefix: &str, deck_hash: &str) -> String {
    if deck_hash.is_empty() {
        origin.to_string()
    } else {
        format!("{origin}/{prefix}/{deck_hash}")
    }
}

fn commit_url(origin: &str, prefix: &str, commit_id: i64) -> String {
    if commit_id <= 0 {
        origin.to_string()
    } else {
        format!("{origin}/{prefix}/{commit_id}")
    }
}

fn json_str<'a>(val: &'a Value, key: &str, default: &'a str) -> &'a str {
    val.get(key).and_then(Value::as_str).unwrap_or(default)
}

fn json_i64(val: &Value, key: &str) -> Option<i64> {
    val.get(key).and_then(Value::as_i64)
}

fn json_bool(val: &Value, key: &str) -> bool {
    val.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn format_timestamp_iso() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| String::new())
}

/// Formats an outbox event into a Discord Webhook JSON body.
#[must_use]
pub fn build_discord_payload(event_type: &str, payload: &Value, site_origin: &str) -> Value {
    let origin = site_origin.trim_end_matches('/');
    let deck_name = sanitize_discord_field(json_str(payload, "deck_name", "Deck"), "Deck", 100);
    let deck_hash = json_str(payload, "deck_hash", "").trim();
    let timestamp = format_timestamp_iso();

    let embed = match event_type {
        "suggestion_submitted" => {
            let commit_id = json_i64(payload, "commit_id").unwrap_or(0);
            let is_deletion = json_bool(payload, "is_deletion");
            let rationale =
                sanitize_discord_field(json_str(payload, "rationale", "Other"), "Other", 64);
            let contributor = sanitize_discord_field(
                json_str(payload, "contributor", "Anonymous"),
                "Anonymous",
                64,
            );
            let note_count = json_i64(payload, "note_count").unwrap_or(0);
            let deck_path = sanitize_discord_text(json_str(payload, "deck_path", ""), 180);
            let commit_info =
                sanitize_discord_multiline(json_str(payload, "commit_info", ""), 300, 6);

            let title = if is_deletion {
                format!("Card Deletion Request — {deck_name}")
            } else {
                format!("New Suggestion — {deck_name}")
            };
            let color = if is_deletion {
                0x00ED_4245
            } else {
                0x0058_65F2
            };
            let url = commit_url(origin, "commit", commit_id);

            let mut fields = vec![
                json!({ "name": "Rationale", "value": rationale, "inline": true }),
                json!({ "name": "Contributor", "value": contributor, "inline": true }),
                json!({ "name": "Notes", "value": note_count.to_string(), "inline": true }),
            ];
            if !deck_path.is_empty() && deck_path != deck_name {
                fields.push(json!({ "name": "Target Deck", "value": deck_path, "inline": false }));
            }
            if !commit_info.is_empty() {
                fields.push(json!({ "name": "Comment", "value": commit_info, "inline": false }));
            }

            json!({
                "title": title,
                "url": url,
                "color": color,
                "fields": fields,
                "footer": { "text": "AnkiCollab" },
                "timestamp": timestamp,
            })
        }
        "suggestion_approved" => {
            let commit_id = json_i64(payload, "commit_id").unwrap_or(0);
            let partial = json_bool(payload, "partial");
            let reviewer = sanitize_discord_field(
                json_str(payload, "reviewer", "Maintainer"),
                "Maintainer",
                64,
            );
            let contributor = sanitize_discord_field(
                json_str(payload, "contributor", "Anonymous"),
                "Anonymous",
                64,
            );
            let rationale =
                sanitize_discord_field(json_str(payload, "rationale", "Other"), "Other", 64);
            let note_count = json_i64(payload, "note_count").unwrap_or(0);

            let title = if partial {
                format!("Partial Approval — {deck_name}")
            } else {
                format!("Suggestion Approved — {deck_name}")
            };
            let url = commit_url(origin, "commit_history", commit_id);

            json!({
                "title": title,
                "url": url,
                "color": 0x002E_CC71,
                "fields": [
                    { "name": "Reviewed by", "value": reviewer, "inline": true },
                    { "name": "Contributor", "value": contributor, "inline": true },
                    { "name": "Notes approved", "value": note_count.to_string(), "inline": true },
                    { "name": "Rationale", "value": rationale, "inline": true },
                ],
                "footer": { "text": "AnkiCollab" },
                "timestamp": timestamp,
            })
        }
        "suggestion_denied" => {
            let commit_id = json_i64(payload, "commit_id").unwrap_or(0);
            let partial = json_bool(payload, "partial");
            let silent = json_bool(payload, "silent");
            let reviewer = sanitize_discord_field(
                json_str(payload, "reviewer", "Maintainer"),
                "Maintainer",
                64,
            );
            let contributor = sanitize_discord_field(
                json_str(payload, "contributor", "Anonymous"),
                "Anonymous",
                64,
            );
            let rationale =
                sanitize_discord_field(json_str(payload, "rationale", "Other"), "Other", 64);
            let note_count = json_i64(payload, "note_count").unwrap_or(0);
            let reason = if silent {
                String::new()
            } else {
                sanitize_discord_multiline(json_str(payload, "reason", ""), 400, 6)
            };

            let suffix = if silent { " (Silent)" } else { "" };
            let title = if partial {
                format!("Notes Denied{suffix} — {deck_name}")
            } else {
                format!("Suggestion Denied{suffix} — {deck_name}")
            };
            let url = commit_url(origin, "commit_history", commit_id);

            let mut fields = vec![
                json!({ "name": "Reviewed by", "value": reviewer, "inline": true }),
                json!({ "name": "Contributor", "value": contributor, "inline": true }),
                {
                    let label = if partial { "Notes denied" } else { "Notes" };
                    json!({ "name": label, "value": note_count.to_string(), "inline": true })
                },
                json!({ "name": "Rationale", "value": rationale, "inline": true }),
            ];
            if !reason.is_empty() {
                fields.push(json!({ "name": "Feedback", "value": reason, "inline": false }));
            }

            json!({
                "title": title,
                "url": url,
                "color": 0x00E7_4C3C,
                "fields": fields,
                "footer": { "text": "AnkiCollab" },
                "timestamp": timestamp,
            })
        }
        "direct_push" => {
            let commit_id = json_i64(payload, "commit_id").unwrap_or(0);
            let contributor = sanitize_discord_field(
                json_str(payload, "contributor", "Maintainer"),
                "Maintainer",
                64,
            );
            let note_count = json_i64(payload, "note_count").unwrap_or(0);
            let commit_info =
                sanitize_discord_multiline(json_str(payload, "commit_info", ""), 300, 6);

            let url = commit_url(origin, "commit_history", commit_id);
            let mut fields = vec![
                json!({ "name": "Updated by", "value": contributor, "inline": true }),
                json!({ "name": "Notes updated", "value": note_count.to_string(), "inline": true }),
            ];
            if !commit_info.is_empty() {
                fields.push(json!({ "name": "Summary", "value": commit_info, "inline": false }));
            }

            json!({
                "title": format!("Direct Deck Update — {deck_name}"),
                "url": url,
                "color": 0x001A_BC9C,
                "fields": fields,
                "footer": { "text": "AnkiCollab" },
                "timestamp": timestamp,
            })
        }
        "changelog_published" => {
            let author =
                sanitize_discord_field(json_str(payload, "author", "Maintainer"), "Maintainer", 64);
            let message = sanitize_discord_multiline(json_str(payload, "message", ""), 900, 12);
            let description = if message.is_empty() {
                "Changelog entry published.".to_string()
            } else {
                message
            };
            let url = deck_url(origin, "notes", deck_hash);

            json!({
                "title": format!("New Changelog — {deck_name}"),
                "url": url,
                "description": description,
                "color": 0x0034_98DB,
                "fields": [
                    { "name": "Published by", "value": author, "inline": true },
                ],
                "footer": { "text": "AnkiCollab" },
                "timestamp": timestamp,
            })
        }
        "maintainer_changed" => {
            let action = json_str(payload, "action", "updated");
            let actor = sanitize_discord_field(json_str(payload, "actor", "Owner"), "Owner", 64);
            let target =
                sanitize_discord_field(json_str(payload, "target", "Unknown"), "Unknown", 64);
            let url = deck_url(origin, "Maintainers", deck_hash);
            let title_action = if action == "added" {
                "Maintainer Added"
            } else if action == "removed" {
                "Maintainer Removed"
            } else {
                "Maintainer Updated"
            };

            json!({
                "title": format!("{title_action} — {deck_name}"),
                "url": url,
                "color": 0x009B_59B6,
                "fields": [
                    { "name": "Maintainer", "value": target, "inline": true },
                    { "name": "Changed by", "value": actor, "inline": true },
                ],
                "footer": { "text": "AnkiCollab" },
                "timestamp": timestamp,
            })
        }
        "subscriber_milestone" => {
            let milestone = json_i64(payload, "milestone").unwrap_or(0);
            let current = json_i64(payload, "current_subscribers").unwrap_or(milestone);
            let url = deck_url(origin, "Contributions", deck_hash);
            let escaped_deck = escape_discord_inline_markdown(&deck_name);

            json!({
                "title": format!("Subscriber Milestone Reached — {deck_name}"),
                "url": url,
                "description": format!("**{escaped_deck}** just crossed **{milestone}** subscribers!"),
                "color": 0x00F1_C40F,
                "fields": [
                    { "name": "Milestone", "value": milestone.to_string(), "inline": true },
                    { "name": "Total subscribers", "value": current.to_string(), "inline": true },
                ],
                "footer": { "text": "AnkiCollab" },
                "timestamp": timestamp,
            })
        }
        "periodic_digest" => {
            let period_label = sanitize_discord_field(
                json_str(payload, "period_label", "Periodic"),
                "Periodic",
                32,
            );
            let open_suggestions = json_i64(payload, "open_suggestions").unwrap_or(0);
            let oldest_days = json_i64(payload, "oldest_pending_days");
            let submitted = json_i64(payload, "submitted").unwrap_or(0);
            let approved = json_i64(payload, "approved").unwrap_or(0);
            let denied = json_i64(payload, "denied").unwrap_or(0);
            let total_subscribers = json_i64(payload, "total_subscribers").unwrap_or(0);
            let net_subscribers = json_i64(payload, "net_subscribers");
            let active_30d = json_i64(payload, "active_30d").unwrap_or(0);
            let retention = payload.get("retention").and_then(Value::as_f64);
            let leech_count = json_i64(payload, "leech_count");
            let url = deck_url(origin, "Contributions", deck_hash);

            let queue_text = match oldest_days {
                Some(d) if open_suggestions > 0 => {
                    format!("{open_suggestions} open (oldest: {d}d)")
                }
                _ => format!("{open_suggestions} open"),
            };
            let subs_text = match net_subscribers {
                Some(delta) if delta > 0 => format!("{total_subscribers} (+{delta})"),
                Some(delta) if delta < 0 => format!("{total_subscribers} ({delta})"),
                Some(_) => format!("{total_subscribers} (±0)"),
                None => total_subscribers.to_string(),
            };
            let activity_text =
                format!("{submitted} submitted · {approved} approved · {denied} denied");

            let mut fields = vec![
                json!({ "name": "Review Queue", "value": queue_text, "inline": true }),
                json!({ "name": "Subscribers", "value": subs_text, "inline": true }),
                json!({ "name": "Active (30d)", "value": active_30d.to_string(), "inline": true }),
                json!({ "name": format!("{period_label} Activity"), "value": activity_text, "inline": false }),
            ];

            if let Some(ret) = retention {
                let stats_text = match leech_count {
                    Some(leeches) => format!("{ret:.1}% avg retention · {leeches} leeches"),
                    None => format!("{ret:.1}% avg retention"),
                };
                fields
                    .push(json!({ "name": "Review Stats", "value": stats_text, "inline": false }));
            }

            json!({
                "title": format!("{period_label} Deck Digest — {deck_name}"),
                "url": url,
                "color": 0x0047_52C4,
                "fields": fields,
                "footer": { "text": "AnkiCollab" },
                "timestamp": timestamp,
            })
        }
        _ => {
            let url = deck_url(origin, "Webhooks", deck_hash);
            let escaped_deck = escape_discord_inline_markdown(&deck_name);
            json!({
                "title": "AnkiCollab Webhook Connected",
                "url": url,
                "description": format!("Test notification succeeded for **{escaped_deck}**."),
                "color": 0x002E_CC71,
                "footer": { "text": "AnkiCollab" },
                "timestamp": timestamp,
            })
        }
    };

    json!({
        "allowed_mentions": { "parse": [] },
        "embeds": [embed],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_TOKEN: &str =
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-abcd";

    #[test]
    fn valid_discord_webhook_urls_accepted_and_masked() {
        for host in ALLOWED_DISCORD_HOSTS {
            let url = format!("https://{host}/api/webhooks/123456789012345678/{VALID_TOKEN}");
            let res = validate_discord_webhook_url(&url).unwrap();
            assert_eq!(res.canonical_url, url);
            assert!(res.preview.contains("1234…"));
            assert!(res.preview.contains("••••abcd"));
            assert!(!res.preview.contains(VALID_TOKEN));
        }
    }

    #[test]
    fn valid_discord_webhook_with_thread_id_and_trailing_slash() {
        let url = format!(
            "  HTTPS://Discord.com/api/webhooks/123456789012345678/{VALID_TOKEN}/?thread_id=987654321098765432  "
        );
        let res = validate_discord_webhook_url(&url).unwrap();
        assert_eq!(
            res.canonical_url,
            format!(
                "https://discord.com/api/webhooks/123456789012345678/{VALID_TOKEN}?thread_id=987654321098765432"
            )
        );
        assert!(res.preview.contains("thread …5432"));
    }

    #[test]
    fn valid_versioned_and_boundary_snowflakes_accepted() {
        let short_token = "a".repeat(50);
        let url_v10 =
            format!("https://discord.com/api/v10/webhooks/1234567890123456/{short_token}");
        let res = validate_discord_webhook_url(&url_v10).unwrap();
        assert_eq!(
            res.canonical_url,
            format!("https://discord.com/api/webhooks/1234567890123456/{short_token}")
        );

        let url_21 = format!(
            "https://discord.com/api/webhooks/123456789012345678901/{VALID_TOKEN}?thread_id=123456789012345678901"
        );
        assert!(validate_discord_webhook_url(&url_21).is_ok());
    }

    #[test]
    fn invalid_and_ssrf_urls_rejected() {
        let cases = [
            "",
            "http://discord.com/api/webhooks/123456789012345678/abc",
            &format!("https://evil.example/api/webhooks/123456789012345678/{VALID_TOKEN}"),
            &format!("https://127.0.0.1/api/webhooks/123456789012345678/{VALID_TOKEN}"),
            &format!("https://169.254.169.254/api/webhooks/123456789012345678/{VALID_TOKEN}"),
            &format!("https://discord.com:8443/api/webhooks/123456789012345678/{VALID_TOKEN}"),
            &format!("https://user@discord.com/api/webhooks/123456789012345678/{VALID_TOKEN}"),
            &format!("https://discord.com/api/webhooks/123456789012345678/{VALID_TOKEN}?thread_id=123&wait=true"),
            &format!("https://discord.com/api/webhooks/123456789012345678/{VALID_TOKEN}#frag"),
            "https://discord.com/api/webhooks/notdigits/short",
        ];
        for bad in cases {
            assert!(
                validate_discord_webhook_url(bad).is_err(),
                "expected rejection for {bad}"
            );
        }
    }

    #[test]
    fn sanitize_neutralizes_mentions_html_masked_links_and_truncates() {
        let s = sanitize_discord_text("Hello @everyone\nand\t@here!", 40);
        assert!(!s.contains("@everyone"));
        assert!(!s.contains("@here"));
        assert!(s.contains("@\u{200B}everyone"));

        let html = sanitize_discord_text("<p>Fix &amp; update</p><br>Line &amp;lt;2&amp;gt;", 100);
        assert_eq!(html, "Fix & update Line &lt;2&gt;");

        let masked = sanitize_discord_text("[Click here](https://evil.example) `code`", 100);
        assert!(masked.contains("\\[Click here\\](https://evil.example)"));
        assert!(masked.contains("\\`code\\`"));

        let multi = sanitize_discord_multiline("<p>First &amp; best</p><br>Second line", 100, 5);
        assert_eq!(multi, "First & best\n\nSecond line");
        assert_eq!(
            sanitize_discord_multiline("One<br>Two<br><br><br>Three", 100, 5),
            "One\nTwo\n\nThree"
        );

        let long = "a".repeat(50);
        let trunc = sanitize_discord_text(&long, 10);
        assert_eq!(trunc.chars().count(), 10);
        assert!(trunc.ends_with('…'));

        // Truncating right after an escaped `[` should drop the trailing `\` before `…`.
        let esc_trunc = sanitize_discord_text("aaaa[bbbb", 6);
        assert!(!esc_trunc.ends_with("\\…"));
        assert!(esc_trunc.ends_with('…'));
    }

    #[test]
    fn build_discord_payload_always_disables_mentions_and_avoids_empty_values() {
        let samples = [
            (
                "suggestion_submitted",
                json!({
                    "commit_id": 42,
                    "deck_name": "Anatomy",
                    "deck_hash": "a-b-c",
                    "deck_path": "Anatomy::Thorax",
                    "rationale": "Content error",
                    "contributor": "@everyone",
                    "note_count": 3,
                    "commit_info": "Fix @here typo",
                    "is_deletion": false
                }),
            ),
            (
                "suggestion_approved",
                json!({
                    "commit_id": 42,
                    "deck_name": "Anatomy",
                    "deck_hash": "a-b-c",
                    "reviewer": "alice",
                    "contributor": "bob",
                    "rationale": "Content error",
                    "note_count": 3,
                    "partial": false
                }),
            ),
            (
                "suggestion_denied",
                json!({
                    "commit_id": 42,
                    "deck_name": "Anatomy",
                    "deck_hash": "a-b-c",
                    "reviewer": "alice",
                    "contributor": "bob",
                    "rationale": "Other",
                    "reason": "Duplicate card",
                    "silent": true,
                    "note_count": 1,
                    "partial": false
                }),
            ),
            (
                "direct_push",
                json!({
                    "commit_id": 99,
                    "deck_name": "Anatomy",
                    "deck_hash": "a-b-c",
                    "contributor": "alice",
                    "note_count": 12,
                    "commit_info": "Updated images"
                }),
            ),
            (
                "changelog_published",
                json!({
                    "deck_name": "Anatomy",
                    "deck_hash": "a-b-c",
                    "author": "alice",
                    "message": "Added 50 cards"
                }),
            ),
            (
                "maintainer_changed",
                json!({
                    "deck_name": "Anatomy",
                    "deck_hash": "a-b-c",
                    "actor": "alice",
                    "target": "bob",
                    "action": "added"
                }),
            ),
            (
                "subscriber_milestone",
                json!({
                    "deck_name": "Anatomy",
                    "deck_hash": "a-b-c",
                    "milestone": 100,
                    "current_subscribers": 104
                }),
            ),
            (
                "periodic_digest",
                json!({
                    "deck_name": "Anatomy",
                    "deck_hash": "a-b-c",
                    "period_label": "Weekly",
                    "open_suggestions": 4,
                    "oldest_pending_days": 3,
                    "submitted": 10,
                    "approved": 7,
                    "denied": 1,
                    "total_subscribers": 240,
                    "net_subscribers": 12,
                    "active_30d": 180,
                    "retention": 84.5,
                    "leech_count": 6
                }),
            ),
            (
                "test",
                json!({
                    "deck_name": "Anatomy",
                    "deck_hash": "a-b-c"
                }),
            ),
            // Empty / whitespace / control-only edge cases
            (
                "maintainer_changed",
                json!({
                    "deck_name": "   ",
                    "deck_hash": "",
                    "actor": "\n\t",
                    "target": "",
                    "action": "added"
                }),
            ),
            (
                "changelog_published",
                json!({
                    "deck_name": "",
                    "deck_hash": "",
                    "author": "  ",
                    "message": "<p><br></p>"
                }),
            ),
            (
                "suggestion_submitted",
                json!({
                    "commit_id": 0,
                    "deck_name": "",
                    "deck_hash": "",
                    "rationale": "",
                    "contributor": "   ",
                    "note_count": 1
                }),
            ),
        ];

        for (event_type, payload) in samples {
            let body = build_discord_payload(event_type, &payload, "https://ankicollab.com");
            assert_eq!(body["allowed_mentions"]["parse"], json!([]));
            let embeds = body["embeds"].as_array().unwrap();
            assert_eq!(embeds.len(), 1);
            assert!(embeds[0]["title"].as_str().is_some_and(|s| !s.is_empty()));
            assert!(embeds[0]["url"]
                .as_str()
                .is_some_and(|u| u.starts_with("https://ankicollab.com")));
            if let Some(fields) = embeds[0]["fields"].as_array() {
                for f in fields {
                    assert!(
                        f["name"].as_str().is_some_and(|s| !s.is_empty()),
                        "empty field name in {event_type}"
                    );
                    assert!(
                        f["value"].as_str().is_some_and(|s| !s.is_empty()),
                        "empty field value in {event_type}: {f}"
                    );
                }
            }
        }
    }
}
