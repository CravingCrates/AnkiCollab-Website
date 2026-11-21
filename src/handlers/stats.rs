use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect},
};

use crate::database::{self, owned_deck_id, AppState};
use crate::error::Error;
use crate::handlers::common::{access_check, check_login};
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
        stats_manager::update_stats(&db_state_clone).await.unwrap();
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
            "Select owner from decks where human_hash = $1",
            &[&deck_hash],
        )
        .await
        .expect("Error preparing edit deck statement");
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
        .unwrap();

    Ok(Redirect::to("/ManageDecks"))
}

pub(crate) async fn show_statistics(
    State(appstate): State<Arc<AppState>>,
    Path(deck_hash): Path<String>,
    user: Option<User>,
) -> Result<impl IntoResponse, Error> {
    let user = check_login(user)?;
    let client = database::client(&appstate).await?;
    let owned_info = client
        .query("Select id from decks where human_hash = $1", &[&deck_hash])
        .await
        .expect("Error preparing edit deck statement");
    if owned_info.is_empty() {
        return Ok(Html("Deck not found.".to_string()));
    }
    let deck_id: i64 = owned_info[0].get(0);

    if !access_check(&appstate, deck_id, &user).await? {
        return Ok(Html("Unauthorized.".to_string()));
    }

    let mut context = tera::Context::new();

    let deck_base_info = match stats_manager::get_base_deck_info(&appstate, &deck_hash).await {
        Ok(deck_base_info) => deck_base_info,
        Err(error) => {
            tracing::warn!(error = %error, "Failed to get base deck info for statistics");
            return Ok(Html("Error showing the statistics.".to_string()));
        }
    };

    if deck_base_info.note_count == 0 {
        let rendered_template = appstate
            .tera
            .render("empty_stats.html", &context)
            .expect("Failed to render template");
        return Ok(Html(rendered_template));
    }

    let deck_info = match stats_manager::get_deck_stat_info(&appstate, &deck_hash).await {
        Ok(deck_info) => deck_info,
        Err(error) => {
            tracing::warn!(error = %error, "Failed to get deck stat info");
            return Ok(Html("Error showing the statistics.".to_string()));
        }
    };

    let notes_info = match stats_manager::get_worst_notes_info(&appstate, &deck_hash).await {
        Ok(notes_info) => notes_info,
        Err(error) => {
            tracing::warn!(error = %error, "Failed to get worst notes info");
            return Ok(Html("Error showing the statistics.".to_string()));
        }
    };

    context.insert("decks", &deck_info);
    context.insert("notes", &notes_info);
    context.insert("base", &deck_base_info);
    context.insert("deck_hash", &deck_hash);

    let rendered_template = appstate
        .tera
        .render("statistics.html", &context)
        .expect("Failed to render template");
    Ok(Html(rendered_template))
}
