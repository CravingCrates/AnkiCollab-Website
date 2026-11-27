use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse},
    Json,
};

use crate::database::{owned_deck_id, AppState};
use crate::error::Error;
use crate::user::User;
use crate::{maintainer_manager, structs, webhook_manager};

async fn render_maintainers(
    appstate: &Arc<AppState>,
    deck_hash: &String,
    deck_id: i64,
    user: User,
) -> impl IntoResponse {
    // Get Maintainers by deck id
    let maintainers = match maintainer_manager::get_maintainers(appstate, deck_id).await {
        Ok(maintainers) => maintainers,
        Err(e) => {
            tracing::warn!(error = %e, deck_id = deck_id, "Failed to get maintainers");
            return Html("Error getting maintainers.".to_string());
        }
    };

    let mut context = tera::Context::new();
    context.insert("maintainers", &maintainers);
    context.insert("hash", &deck_hash);
    context.insert("user", &user);

    let rendered_template = appstate
        .tera
        .render("maintainers.html", &context)
        .expect("Failed to render template");
    Html(rendered_template)
}

pub(crate) async fn post_maintainers(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Json(edit_maintainer): Json<structs::UpdateMaintainer>,
) -> Result<impl IntoResponse, Error> {
    let data = edit_maintainer;

    let deck_id: i64 = owned_deck_id(&appstate, &data.deck, user.id()).await?;

    let normalized_target = data.username.trim().to_lowercase();

    // Add new maintainer
    if data.action == 1 {
        let res = maintainer_manager::add_maintainer(&appstate, deck_id, normalized_target.clone())
            .await?;
        webhook_manager::spawn_maintainer_webhook(
            Arc::clone(&appstate),
            deck_id,
            user.username(),
            normalized_target,
            true,
        );
        Ok(res)
    } else {
        // Delete existing maintainer
        let res =
            maintainer_manager::remove_maintainer(&appstate, deck_id, normalized_target.clone())
                .await?;
        webhook_manager::spawn_maintainer_webhook(
            Arc::clone(&appstate),
            deck_id,
            user.username(),
            normalized_target,
            false,
        );
        Ok(res)
    }
}

pub(crate) async fn show_maintainers(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Path(deck_hash): Path<String>,
) -> Result<impl IntoResponse, Error> {
    let deck_id: i64 = owned_deck_id(&appstate, &deck_hash, user.id()).await?;

    Ok(render_maintainers(&appstate, &deck_hash, deck_id, user).await)
}
