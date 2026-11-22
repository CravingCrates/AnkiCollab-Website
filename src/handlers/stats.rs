use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect},
};

use crate::contribution_manager;
use crate::database::{self, owned_deck_id, AppState};
use crate::error::{self, Error};
use crate::handlers::common::{access_check, check_login, error_page};
use crate::stats_manager;
use crate::user::User;

static STATS_CACHE_KEY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    std::env::var("STATS_CACHE_KEY").expect("STATS_CACHE_KEY must be set")
});

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

    Ok(Redirect::to("/ManageDecks"))
}

pub(crate) async fn show_statistics(
    State(appstate): State<Arc<AppState>>,
    Path(deck_hash): Path<String>,
    user: Option<User>,
) -> Result<impl IntoResponse, Error> {
    let user = check_login(user)?;
    let client = database::client(&appstate).await?;
    let deck_row = client
        .query_opt(
            "SELECT id,
                    COALESCE(NULLIF(BTRIM(full_path), ''), NULLIF(BTRIM(name), ''), ''),
                    COALESCE(stats_enabled, false)
               FROM anki.decks
              WHERE human_hash = $1 AND deleted_at IS NULL",
            &[&deck_hash],
        )
        .await?;

    let Some(deck_row) = deck_row else {
        return error_page(&appstate, "Deck not found");
    };
    let deck_id: i64 = deck_row.get(0);
    let deck_path: String = deck_row.get(1);
    let deck_name = contribution_manager::deck_display_name_from_path(&deck_path);
    let stats_enabled: bool = deck_row.get(2);

    if !access_check(&appstate, deck_id, &user).await? {
        return Err(error::Error::Unauthorized);
    }

    let mut context = tera::Context::new();
    context.insert("user", &user);
    context.insert("deck_hash", &deck_hash);
    context.insert("deck_name", &deck_name);
    context.insert("stats_enabled", &stats_enabled);
    context.insert("active_tab", "statistics");

    let mut deck_base_info = match stats_manager::get_base_deck_info(&appstate, &deck_hash).await {
        Ok(deck_base_info) => deck_base_info,
        Err(error) => {
            tracing::warn!(error = %error, "Failed to get base deck info for statistics");
            return error_page(&appstate, "Error loading deck statistics.");
        }
    };

    if deck_base_info.note_count == 0 {
        context.insert("base", &deck_base_info);
        let rendered_template = appstate.tera.render("empty_stats.html", &context)?;
        return Ok(Html(rendered_template));
    }

    let deck_info = match stats_manager::get_deck_stat_info(&appstate, &deck_hash).await {
        Ok(deck_info) => deck_info,
        Err(error) => {
            tracing::warn!(error = %error, "Failed to get deck stat info");
            return error_page(&appstate, "Error loading deck statistics.");
        }
    };

    let notes_info = match stats_manager::get_worst_notes_info(&appstate, &deck_hash).await {
        Ok(notes_info) => notes_info,
        Err(error) => {
            tracing::warn!(error = %error, "Failed to get worst notes info");
            return error_page(&appstate, "Error loading deck statistics.");
        }
    };

    let retention_buckets =
        match stats_manager::get_retention_distribution(&appstate, &deck_hash).await {
            Ok(buckets) => buckets,
            Err(error) => {
                tracing::warn!(error = %error, "Failed to get retention distribution");
                Vec::new()
            }
        };

    let (retention_trend, trend_delta_30d) =
        match stats_manager::get_retention_trend(&appstate, deck_id).await {
            Ok(trend) => trend,
            Err(error) => {
                tracing::warn!(error = %error, "Failed to get retention trend");
                (Vec::new(), None)
            }
        };
    deck_base_info.trend_delta_30d = trend_delta_30d;

    let tag_stats = match stats_manager::get_tag_stats(&appstate, &deck_hash).await {
        Ok(tags) => tags,
        Err(error) => {
            tracing::warn!(error = %error, "Failed to get tag stats");
            Vec::new()
        }
    };

    let notetype_stats = match stats_manager::get_notetype_stats(&appstate, &deck_hash).await {
        Ok(notetypes) => notetypes,
        Err(error) => {
            tracing::warn!(error = %error, "Failed to get notetype stats");
            Vec::new()
        }
    };

    let count_attention = notes_info
        .iter()
        .filter(|n| {
            i64::from(n.sample_size) >= stats_manager::MIN_LEARNER_SAMPLE
                && !n.has_pending_suggestion
                && !n.revised_recently
        })
        .count();
    let count_confident = notes_info
        .iter()
        .filter(|n| i64::from(n.sample_size) >= stats_manager::MIN_LEARNER_SAMPLE)
        .count();
    let count_leech = notes_info.iter().filter(|n| n.is_leech).count();
    let count_revised = notes_info.iter().filter(|n| n.revised_recently).count();
    let confident_bucket_total: i64 = retention_buckets.iter().map(|b| b.confident_count).sum();

    context.insert("decks", &deck_info);
    context.insert("notes", &notes_info);
    context.insert("base", &deck_base_info);
    context.insert("retention_buckets", &retention_buckets);
    context.insert("confident_bucket_total", &confident_bucket_total);
    context.insert("retention_trend", &retention_trend);
    context.insert("tag_stats", &tag_stats);
    context.insert("notetype_stats", &notetype_stats);
    context.insert("count_all", &notes_info.len());
    context.insert("count_attention", &count_attention);
    context.insert("count_confident", &count_confident);
    context.insert("count_leech", &count_leech);
    context.insert("count_revised", &count_revised);
    context.insert(
        "hist_labels",
        &retention_buckets
            .iter()
            .map(|b| b.label.clone())
            .collect::<Vec<_>>(),
    );
    context.insert(
        "hist_all",
        &retention_buckets
            .iter()
            .map(|b| b.all_count)
            .collect::<Vec<_>>(),
    );
    context.insert(
        "hist_confident",
        &retention_buckets
            .iter()
            .map(|b| b.confident_count)
            .collect::<Vec<_>>(),
    );
    context.insert(
        "trend_labels",
        &retention_trend
            .iter()
            .map(|p| p.captured_on.clone())
            .collect::<Vec<_>>(),
    );
    context.insert(
        "trend_retention",
        &retention_trend
            .iter()
            .map(|p| p.avg_retention)
            .collect::<Vec<_>>(),
    );
    context.insert(
        "trend_lapses",
        &retention_trend
            .iter()
            .map(|p| p.avg_lapses)
            .collect::<Vec<_>>(),
    );

    let rendered_template = appstate.tera.render("statistics.html", &context)?;
    Ok(Html(rendered_template))
}
