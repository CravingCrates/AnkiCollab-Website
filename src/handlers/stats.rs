use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

use crate::database::{self, owned_deck_id, AppState};
use crate::error::Error;
use crate::handlers::common::{access_check, check_login, error_page};
use crate::stats_cache::DeckStatsPayload;
use crate::stats_manager;
use crate::structs::DeckBaseStatsInfo;
use crate::user::User;

#[derive(Deserialize, Default)]
pub(crate) struct StatisticsQuery {
    #[serde(default)]
    pub sync: Option<u8>,
}

static STATS_CACHE_KEY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    std::env::var("STATS_CACHE_KEY").expect("STATS_CACHE_KEY must be set")
});

fn no_store_html(html: String) -> Response {
    (
        StatusCode::OK,
        [(
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, no-store"),
        )],
        Html(html),
    )
        .into_response()
}

fn insert_stats_payload_context(context: &mut tera::Context, payload: &DeckStatsPayload) {
    let count_attention = payload
        .notes
        .iter()
        .filter(|n| {
            i64::from(n.sample_size) >= stats_manager::MIN_LEARNER_SAMPLE
                && !n.has_pending_suggestion
                && !n.revised_recently
        })
        .count();
    let count_confident = payload
        .notes
        .iter()
        .filter(|n| i64::from(n.sample_size) >= stats_manager::MIN_LEARNER_SAMPLE)
        .count();
    let count_leech = payload.notes.iter().filter(|n| n.is_leech).count();
    let count_revised = payload.notes.iter().filter(|n| n.revised_recently).count();
    let confident_bucket_total: i64 = payload
        .retention_buckets
        .iter()
        .map(|b| b.confident_count)
        .sum();

    context.insert("decks", &payload.decks);
    context.insert("notes", &payload.notes);
    context.insert("base", &payload.base);
    context.insert("retention_buckets", &payload.retention_buckets);
    context.insert("confident_bucket_total", &confident_bucket_total);
    context.insert("retention_trend", &payload.retention_trend);
    context.insert("tag_stats", &payload.tag_stats);
    context.insert("notetype_stats", &payload.notetype_stats);
    context.insert("count_all", &payload.notes.len());
    context.insert("count_attention", &count_attention);
    context.insert("count_confident", &count_confident);
    context.insert("count_leech", &count_leech);
    context.insert("count_revised", &count_revised);
    context.insert(
        "hist_labels",
        &payload
            .retention_buckets
            .iter()
            .map(|b| b.label.clone())
            .collect::<Vec<_>>(),
    );
    context.insert(
        "hist_all",
        &payload
            .retention_buckets
            .iter()
            .map(|b| b.all_count)
            .collect::<Vec<_>>(),
    );
    context.insert(
        "hist_confident",
        &payload
            .retention_buckets
            .iter()
            .map(|b| b.confident_count)
            .collect::<Vec<_>>(),
    );
    context.insert(
        "trend_labels",
        &payload
            .retention_trend
            .iter()
            .map(|p| p.captured_on.clone())
            .collect::<Vec<_>>(),
    );
    context.insert(
        "trend_retention",
        &payload
            .retention_trend
            .iter()
            .map(|p| p.avg_retention)
            .collect::<Vec<_>>(),
    );
    context.insert(
        "trend_lapses",
        &payload
            .retention_trend
            .iter()
            .map(|p| p.avg_lapses)
            .collect::<Vec<_>>(),
    );
}

pub(crate) async fn refresh_stats_cache(
    State(appstate): State<Arc<AppState>>,
    Path(secret): Path<String>,
) -> Result<impl IntoResponse, Error> {
    if secret != *STATS_CACHE_KEY {
        return Ok(Redirect::to("/"));
    }
    let db_state_clone = Arc::clone(&appstate);
    tokio::spawn(async move {
        if let Err(err) = stats_manager::update_stats(&db_state_clone).await {
            tracing::warn!(error = %err, "Failed to refresh stats cache");
        }
    });
    Ok(Redirect::to("/"))
}

pub(crate) async fn toggle_stats(
    State(appstate): State<Arc<AppState>>,
    Path(deck_hash): Path<String>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let client = database::client(&appstate).await?;
    let owned_info = client
        .query(
            "Select owner from decks where human_hash = $1 AND deleted_at IS NULL",
            &[&deck_hash],
        )
        .await?;
    if owned_info.is_empty() {
        return Ok(Redirect::to("/"));
    }
    let owner: i32 = owned_info[0].get(0);

    if owner != user.id() {
        return Ok(Redirect::to("/"));
    }

    let deck_id = owned_deck_id(&appstate, &deck_hash, user.id()).await?;

    stats_manager::toggle_stats(&appstate, deck_id)
        .await
        .map_err(|err| {
            tracing::warn!(error = %err, "Failed to toggle deck stats");
            Error::Unknown
        })?;
    appstate.stats_cache.invalidate_deck(deck_id);

    Ok(Redirect::to("/ManageDecks"))
}

pub(crate) async fn show_statistics(
    State(appstate): State<Arc<AppState>>,
    Path(deck_hash): Path<String>,
    Query(query): Query<StatisticsQuery>,
    user: Option<User>,
) -> Result<Response, Error> {
    let user = check_login(user)?;
    let Some(shell) = stats_manager::get_deck_stats_shell(&appstate, &deck_hash).await? else {
        return Ok(error_page(&appstate, "Deck not found")?.into_response());
    };

    if !access_check(&appstate, shell.deck_id, &user).await? {
        return Err(Error::Unauthorized);
    }

    let mut context = tera::Context::new();
    context.insert("user", &user);
    context.insert("deck_hash", &deck_hash);
    context.insert("deck_name", &shell.deck_name);
    context.insert("stats_enabled", &shell.stats_enabled);
    context.insert("active_tab", "statistics");

    if shell.note_count == 0 {
        let total_reviewed_notes =
            stats_manager::get_total_reviewed_notes(&appstate, &deck_hash).await?;
        let base = DeckBaseStatsInfo {
            note_count: 0,
            total_reviewed_notes,
            coverage_pct: 0.0,
            lapses_avg: 0.0,
            reps_avg: 0.0,
            retention_avg: 0.0,
            contributor_count: 0,
            median_sample_size: 0.0,
            leech_count: 0,
            recently_revised_struggling: 0,
            trend_delta_30d: None,
        };
        context.insert("base", &base);
        let rendered = appstate.tera.render("empty_stats.html", &context)?;
        return Ok(no_store_html(rendered));
    }

    if query.sync == Some(1) {
        let payload = match stats_manager::get_or_load_deck_stats(&appstate, &shell, &deck_hash)
            .await
        {
            Ok(payload) => payload,
            Err(err) => {
                tracing::warn!(error = %err, "Failed to load deck statistics synchronously");
                return Ok(error_page(&appstate, "Error loading deck statistics.")?.into_response());
            }
        };
        insert_stats_payload_context(&mut context, &payload);
        context.insert("is_deferred", &false);
        let rendered = appstate.tera.render("statistics.html", &context)?;
        return Ok(no_store_html(rendered));
    }

    if let Some(payload) = appstate.stats_cache.get(shell.deck_id) {
        insert_stats_payload_context(&mut context, &payload);
        context.insert("is_deferred", &false);
        let rendered = appstate.tera.render("statistics.html", &context)?;
        return Ok(no_store_html(rendered));
    }

    stats_manager::warm_deck_stats_in_background(
        Arc::clone(&appstate),
        shell.clone(),
        deck_hash.clone(),
    );

    context.insert("is_deferred", &true);
    context.insert("shell_note_count", &shell.note_count);
    context.insert("shell_retention_avg", &shell.retention_avg);

    let rendered = appstate.tera.render("statistics.html", &context)?;
    Ok(no_store_html(rendered))
}

pub(crate) async fn show_statistics_sections(
    State(appstate): State<Arc<AppState>>,
    Path(deck_hash): Path<String>,
    user: Option<User>,
) -> Result<Response, Error> {
    let user = check_login(user)?;
    let Some(shell) = stats_manager::get_deck_stats_shell(&appstate, &deck_hash).await? else {
        return Err(Error::DeckNotFound);
    };

    if !access_check(&appstate, shell.deck_id, &user).await? {
        return Err(Error::Unauthorized);
    }

    let payload = stats_manager::get_or_load_deck_stats(&appstate, &shell, &deck_hash)
        .await
        .map_err(|_| Error::Unknown)?;

    let mut context = tera::Context::new();
    context.insert("deck_hash", &deck_hash);
    insert_stats_payload_context(&mut context, &payload);

    let rendered = appstate
        .tera
        .render("partials/statistics_body.html", &context)?;
    Ok(no_store_html(rendered))
}
