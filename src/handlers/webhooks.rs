use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    Json,
};
use serde_json::json;

use crate::database::{self, AppState};
use crate::error::Error;
use crate::handlers::common::{check_login, error_page};
use crate::user::User;
use crate::webhook_manager::{
    self, CreateWebhookInput, OwnedDeckResolution, UpdateWebhookInput, MAX_WEBHOOKS_PER_DECK,
};
use crate::webhook_worker;

fn is_same_origin(headers: &HeaderMap, site_origin: &str) -> bool {
    if let Some(v) = headers.get("sec-fetch-site") {
        return v.as_bytes() == b"same-origin";
    }
    headers
        .get(header::ORIGIN)
        .is_some_and(|v| v.as_bytes() == site_origin.as_bytes())
}

fn no_store(mut resp: Response) -> Response {
    resp.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    resp
}

fn json_error(status: StatusCode, msg: &str) -> Response {
    no_store((status, Json(json!({ "error": msg }))).into_response())
}

async fn require_root_deck_api(
    state: &AppState,
    deck_hash: &str,
    user: Option<User>,
) -> Result<i64, (StatusCode, &'static str)> {
    let Some(user) = user else {
        return Err((
            StatusCode::UNAUTHORIZED,
            "Please sign in again to manage webhooks.",
        ));
    };

    match webhook_manager::resolve_owned_deck(state, deck_hash, user.id()).await {
        Ok(OwnedDeckResolution::Root { id, .. }) => Ok(id),
        Ok(OwnedDeckResolution::Subdeck { .. }) => Err((
            StatusCode::BAD_REQUEST,
            "Webhooks can only be configured on the root deck.",
        )),
        Err(Error::Unauthorized) => Err((
            StatusCode::FORBIDDEN,
            "You do not have permission to manage webhooks for this deck.",
        )),
        Err(_) => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Service temporarily unavailable. Please try again.",
        )),
    }
}

pub(crate) async fn show_webhooks(
    State(appstate): State<Arc<AppState>>,
    user: Option<User>,
    Path(deck_hash): Path<String>,
) -> Result<Response, Error> {
    let user = check_login(user)?;
    let (deck_id, deck_name) =
        match webhook_manager::resolve_owned_deck(&appstate, &deck_hash, user.id()).await {
            Ok(OwnedDeckResolution::Root { id, name }) => (id, name),
            Ok(OwnedDeckResolution::Subdeck { root_hash }) => {
                return Ok(no_store(
                    Redirect::to(&format!("/Webhooks/{root_hash}")).into_response(),
                ));
            }
            Err(Error::Unauthorized) => {
                return error_page(&appstate, &Error::Unauthorized.to_string())
                    .map(|html| no_store(html.into_response()));
            }
            Err(err) => return Err(err),
        };

    // Keep database module imported cleanly for future page queries if needed.
    let _ = database::client;
    let webhooks = webhook_manager::list_for_deck(&appstate, deck_id).await?;

    let mut context = tera::Context::new();
    context.insert("user", &user);
    context.insert("hash", &deck_hash);
    context.insert("deck_name", &deck_name);
    context.insert("webhooks", &webhooks);
    context.insert("webhooks_configured", &appstate.webhook_cipher.is_some());
    context.insert("max_webhooks", &MAX_WEBHOOKS_PER_DECK);

    let rendered = appstate.tera.render("webhooks.html", &context)?;
    Ok(no_store(Html(rendered).into_response()))
}

pub(crate) async fn api_create_webhook(
    headers: HeaderMap,
    State(appstate): State<Arc<AppState>>,
    user: Option<User>,
    Path(deck_hash): Path<String>,
    Json(input): Json<CreateWebhookInput>,
) -> Result<Response, Error> {
    if !is_same_origin(&headers, &appstate.site_origin) {
        return Ok(json_error(
            StatusCode::FORBIDDEN,
            "Cross-origin request rejected.",
        ));
    }

    let deck_id = match require_root_deck_api(&appstate, &deck_hash, user).await {
        Ok(id) => id,
        Err((status, msg)) => return Ok(json_error(status, msg)),
    };

    let Some(cipher) = appstate.webhook_cipher.as_deref() else {
        return Ok(json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Webhook notifications are not enabled on this server.",
        ));
    };

    match webhook_manager::create_webhook(&appstate, cipher, deck_id, input).await {
        Ok(view) => Ok(no_store((StatusCode::CREATED, Json(view)).into_response())),
        Err(Error::BadRequest(msg)) => Ok(json_error(StatusCode::BAD_REQUEST, &msg)),
        Err(Error::DeckNotFound) => Ok(json_error(StatusCode::NOT_FOUND, "Deck not found.")),
        Err(err) => Err(err),
    }
}

pub(crate) async fn api_update_webhook(
    headers: HeaderMap,
    State(appstate): State<Arc<AppState>>,
    user: Option<User>,
    Path((deck_hash, webhook_id)): Path<(String, i64)>,
    Json(input): Json<UpdateWebhookInput>,
) -> Result<Response, Error> {
    if !is_same_origin(&headers, &appstate.site_origin) {
        return Ok(json_error(
            StatusCode::FORBIDDEN,
            "Cross-origin request rejected.",
        ));
    }

    let deck_id = match require_root_deck_api(&appstate, &deck_hash, user).await {
        Ok(id) => id,
        Err((status, msg)) => return Ok(json_error(status, msg)),
    };

    let Some(cipher) = appstate.webhook_cipher.as_deref() else {
        return Ok(json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Webhook notifications are not enabled on this server.",
        ));
    };

    match webhook_manager::update_webhook(&appstate, cipher, deck_id, webhook_id, input).await {
        Ok(view) => Ok(no_store((StatusCode::OK, Json(view)).into_response())),
        Err(Error::BadRequest(msg)) => Ok(json_error(StatusCode::BAD_REQUEST, &msg)),
        Err(err) => Err(err),
    }
}

pub(crate) async fn api_delete_webhook(
    headers: HeaderMap,
    State(appstate): State<Arc<AppState>>,
    user: Option<User>,
    Path((deck_hash, webhook_id)): Path<(String, i64)>,
) -> Result<Response, Error> {
    if !is_same_origin(&headers, &appstate.site_origin) {
        return Ok(json_error(
            StatusCode::FORBIDDEN,
            "Cross-origin request rejected.",
        ));
    }

    let deck_id = match require_root_deck_api(&appstate, &deck_hash, user).await {
        Ok(id) => id,
        Err((status, msg)) => return Ok(json_error(status, msg)),
    };

    let deleted = webhook_manager::delete_webhook(&appstate, deck_id, webhook_id).await?;
    if !deleted {
        return Ok(json_error(
            StatusCode::NOT_FOUND,
            "Webhook not found for this deck.",
        ));
    }

    Ok(no_store(
        (StatusCode::OK, Json(json!({ "deleted": true }))).into_response(),
    ))
}

pub(crate) async fn api_test_webhook(
    headers: HeaderMap,
    State(appstate): State<Arc<AppState>>,
    user: Option<User>,
    Path((deck_hash, webhook_id)): Path<(String, i64)>,
) -> Result<Response, Error> {
    if !is_same_origin(&headers, &appstate.site_origin) {
        return Ok(json_error(
            StatusCode::FORBIDDEN,
            "Cross-origin request rejected.",
        ));
    }

    let deck_id = match require_root_deck_api(&appstate, &deck_hash, user).await {
        Ok(id) => id,
        Err((status, msg)) => return Ok(json_error(status, msg)),
    };

    let Some(cipher) = appstate.webhook_cipher.as_deref() else {
        return Ok(json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Webhook notifications are not enabled on this server.",
        ));
    };

    if let Err(wait_secs) = appstate.webhook_test_cooldown.try_acquire(webhook_id) {
        return Ok(no_store(
            (
                StatusCode::TOO_MANY_REQUESTS,
                [(header::RETRY_AFTER, wait_secs.to_string())],
                Json(json!({
                    "error": format!("Please wait {wait_secs}s before sending another test notification.")
                })),
            )
                .into_response(),
        ));
    }

    match webhook_worker::send_test_webhook(&appstate, cipher, deck_id, webhook_id).await {
        Ok(view) => Ok(no_store(
            (
                StatusCode::OK,
                Json(json!({
                    "success": true,
                    "message": "Test notification sent to Discord.",
                    "webhook": view,
                })),
            )
                .into_response(),
        )),
        Err(msg) => Ok(json_error(StatusCode::UNPROCESSABLE_ENTITY, &msg)),
    }
}
