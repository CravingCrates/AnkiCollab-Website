use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::{IntoResponse, Redirect},
    Json,
};
use axum_client_ip::ClientIp;

use crate::database::{self, AppState};
use crate::error::{self, Error};
use crate::handlers::common::access_check;
use crate::structs::{DeckId, FieldId, Return};
use crate::user::User;
use crate::{cleanser, media_reference_manager, structs, suggestion_manager, webhook_manager};

async fn get_deck_id(
    appstate: &Arc<AppState>,
    query: &str,
    param: &(dyn tokio_postgres::types::ToSql + Sync),
) -> Return<DeckId> {
    let client = database::client(appstate).await?;
    let q_guid = match client.query(query, &[param]).await {
        Ok(q_guid) => q_guid,
        Err(_error) => return Ok(0),
    };
    if q_guid.is_empty() {
        return Ok(0);
    }
    let deck_id: DeckId = q_guid[0].get(0);

    Ok(deck_id)
}

async fn get_deck_by_tag_id(appstate: &Arc<AppState>, tag_id: i64) -> Return<DeckId> {
    let query = "Select deck from notes where id = (select note from tags where id = $1)";
    get_deck_id(appstate, query, &tag_id).await
}

async fn get_deck_by_field_id(appstate: &Arc<AppState>, field_id: FieldId) -> Return<DeckId> {
    let query = "Select deck from notes where id = (select note from fields where id = $1)";
    get_deck_id(appstate, query, &field_id).await
}

async fn get_deck_by_move_id(appstate: &Arc<AppState>, move_id: i32) -> Return<DeckId> {
    let query = "Select original_deck from note_move_suggestions where id = $1";
    get_deck_id(appstate, query, &move_id).await
}

async fn pending_item_context(
    client: &tokio_postgres::Client,
    query: &str,
    param: &(dyn tokio_postgres::types::ToSql + Sync),
) -> Option<(i64, i32)> {
    let row = client.query_opt(query, &[param]).await.ok().flatten()?;
    let note_id: i64 = row.get(0);
    let commit_id: Option<i32> = row.get(1);
    commit_id.map(|cid| (note_id, cid))
}

pub(crate) async fn deny_tag(
    State(appstate): State<Arc<AppState>>,
    Path(tag_id): Path<i64>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let deck_id = match get_deck_by_tag_id(&appstate, tag_id).await {
        Ok(deck_id) => deck_id,
        Err(error) => {
            tracing::warn!(error = %error, tag_id = tag_id, "Failed to get deck by tag ID");
            return Err(error);
        }
    };

    if !access_check(&appstate, deck_id, &user).await? {
        return Err(error::Error::Unauthorized);
    }

    let reviewer_username = user.username();
    let mut client = database::client(&appstate).await?; // needs mutable for transaction
    let item_ctx = pending_item_context(
        &client,
        "SELECT note, commit FROM tags WHERE id = $1 AND reviewed = false",
        &tag_id,
    )
    .await;
    let tx = client.transaction().await?;
    match suggestion_manager::deny_tag_change(&tx, tag_id, user.id()).await {
        Ok(res) => {
            tx.commit().await?;
            if let Some((nid, cid)) = item_ctx {
                webhook_manager::spawn_single_note_review_webhook(
                    Arc::clone(&appstate),
                    cid,
                    nid,
                    false,
                    reviewer_username,
                    true,
                );
            }
            Ok(Redirect::to(&format!("/review/{res}")))
        }
        Err(error) => {
            tracing::warn!(error = %error, tag_id = tag_id, "Failed to deny tag change");
            let _ = tx.rollback().await;
            Err(error)
        }
    }
}

pub(crate) async fn deny_note_move(
    State(appstate): State<Arc<AppState>>,
    Path(move_id): Path<i32>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let deck_id = match get_deck_by_move_id(&appstate, move_id).await {
        Ok(deck_id) => deck_id,
        Err(error) => {
            tracing::warn!(error = %error, move_id = move_id, "Failed to get deck by move ID");
            return Err(error);
        }
    };

    if !access_check(&appstate, deck_id, &user).await? {
        return Err(error::Error::Unauthorized);
    }

    let reviewer_username = user.username();
    let mut client = database::client(&appstate).await?;
    let item_ctx = pending_item_context(
        &client,
        "SELECT note, commit FROM note_move_suggestions WHERE id = $1",
        &move_id,
    )
    .await;
    let tx = client.transaction().await?;
    match suggestion_manager::deny_note_move_request(&tx, move_id, user.id()).await {
        Ok(res) => {
            tx.commit().await?;
            if let Some((nid, cid)) = item_ctx {
                webhook_manager::spawn_single_note_review_webhook(
                    Arc::clone(&appstate),
                    cid,
                    nid,
                    false,
                    reviewer_username,
                    true,
                );
            }
            Ok(Redirect::to(&format!("/review/{res}")))
        }
        Err(error) => {
            tracing::warn!(error = %error, move_id = move_id, "Failed to deny note move request");
            let _ = tx.rollback().await;
            Err(error)
        }
    }
}

pub(crate) async fn accept_note_move(
    State(appstate): State<Arc<AppState>>,
    Path(move_id): Path<i32>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let deck_id = match get_deck_by_move_id(&appstate, move_id).await {
        Ok(deck_id) => deck_id,
        Err(error) => {
            tracing::warn!(error = %error, move_id = move_id, "Failed to get deck by move ID");
            return Err(error);
        }
    };

    if !access_check(&appstate, deck_id, &user).await? {
        return Err(error::Error::Unauthorized);
    }

    let reviewer_username = user.username();
    let mut client = database::client(&appstate).await?;
    let item_ctx = pending_item_context(
        &client,
        "SELECT note, commit FROM note_move_suggestions WHERE id = $1",
        &move_id,
    )
    .await;
    let tx = client.transaction().await?;
    match suggestion_manager::approve_move_note_request_by_moveid(&tx, move_id, user.id()).await {
        Ok(res) => {
            tx.commit().await?;
            if let Some((nid, cid)) = item_ctx {
                webhook_manager::spawn_single_note_review_webhook(
                    Arc::clone(&appstate),
                    cid,
                    nid,
                    true,
                    reviewer_username,
                    true,
                );
            }
            Ok(Redirect::to(&format!("/review/{res}")))
        }
        Err(error) => {
            tracing::warn!(error = %error, move_id = move_id, "Failed to accept note move request");
            let _ = tx.rollback().await;
            Err(error)
        }
    }
}

pub(crate) async fn accept_tag(
    State(appstate): State<Arc<AppState>>,
    Path(tag_id): Path<i64>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let deck_id = match get_deck_by_tag_id(&appstate, tag_id).await {
        Ok(deck_id) => deck_id,
        Err(error) => {
            tracing::warn!(error = %error, tag_id = tag_id, "Failed to get deck by tag ID");
            return Err(error);
        }
    };

    if !access_check(&appstate, deck_id, &user).await? {
        return Err(error::Error::Unauthorized);
    }

    let reviewer_username = user.username();
    let mut client = database::client(&appstate).await?;
    let item_ctx = pending_item_context(
        &client,
        "SELECT note, commit FROM tags WHERE id = $1 AND reviewed = false",
        &tag_id,
    )
    .await;
    let tx = client.transaction().await?;
    match suggestion_manager::approve_tag_change(&tx, tag_id, true, user.id()).await {
        Ok(res) => {
            tx.commit().await?;
            if let Some((nid, cid)) = item_ctx {
                webhook_manager::spawn_single_note_review_webhook(
                    Arc::clone(&appstate),
                    cid,
                    nid,
                    true,
                    reviewer_username,
                    true,
                );
            }
            Ok(Redirect::to(&format!("/review/{res}")))
        }
        Err(error) => {
            tracing::warn!(error = %error, tag_id = tag_id, "Failed to approve tag change");
            let _ = tx.rollback().await;
            Err(error)
        }
    }
}

pub(crate) async fn deny_field(
    State(appstate): State<Arc<AppState>>,
    Path(field_id): Path<i64>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let deck_id = match get_deck_by_field_id(&appstate, field_id).await {
        Ok(deck_id) => deck_id,
        Err(error) => {
            tracing::warn!(error = %error, field_id = field_id, "Failed to get deck by field ID");
            return Err(error);
        }
    };

    if !access_check(&appstate, deck_id, &user).await? {
        return Err(error::Error::Unauthorized);
    }

    let reviewer_username = user.username();
    let mut client = database::client(&appstate).await?;
    let item_ctx = pending_item_context(
        &client,
        "SELECT note, commit FROM fields WHERE id = $1 AND reviewed = false",
        &field_id,
    )
    .await;
    let tx = client.transaction().await?;
    match suggestion_manager::deny_field_change(&tx, field_id, user.id()).await {
        Ok(res) => {
            tx.commit().await?;
            if let Some((nid, cid)) = item_ctx {
                webhook_manager::spawn_single_note_review_webhook(
                    Arc::clone(&appstate),
                    cid,
                    nid,
                    false,
                    reviewer_username,
                    true,
                );
            }
            Ok(Redirect::to(&format!("/review/{res}")))
        }
        Err(error) => {
            tracing::warn!(error = %error, field_id = field_id, "Failed to deny field change");
            let _ = tx.rollback().await;
            Err(error)
        }
    }
}

pub(crate) async fn accept_field(
    State(appstate): State<Arc<AppState>>,
    Path(field_id): Path<i64>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let deck_id = match get_deck_by_field_id(&appstate, field_id).await {
        Ok(deck_id) => deck_id,
        Err(error) => {
            tracing::warn!(error = %error, field_id = field_id, "Failed to get deck by field ID");
            return Err(error);
        }
    };

    if !access_check(&appstate, deck_id, &user).await? {
        return Err(error::Error::Unauthorized);
    }

    let reviewer_username = user.username();
    let mut client = database::client(&appstate).await?;
    let item_ctx = pending_item_context(
        &client,
        "SELECT note, commit FROM fields WHERE id = $1 AND reviewed = false",
        &field_id,
    )
    .await;
    let tx = client.transaction().await?;
    match suggestion_manager::approve_field_change(&tx, field_id, true, user.id()).await {
        Ok(res) => {
            tx.commit().await?;
            if let Some((nid, cid)) = item_ctx {
                webhook_manager::spawn_single_note_review_webhook(
                    Arc::clone(&appstate),
                    cid,
                    nid,
                    true,
                    reviewer_username,
                    true,
                );
            }
            // Best-effort post-commit media reference refresh for this note
            if let Ok(nid) = res.parse::<i64>() {
                let state_clone = appstate.clone();
                tokio::spawn(async move {
                    if let Err(e) =
                        media_reference_manager::update_media_references_for_approved_note(
                            &state_clone,
                            nid,
                        )
                        .await
                    {
                        tracing::warn!(error = ?e, note_id = nid, "Failed to update media references");
                    }
                });
            }
            Ok(Redirect::to(&format!("/review/{res}")))
        }
        Err(error) => {
            tracing::warn!(error = %error, field_id = field_id, "Failed to approve field change");
            let _ = tx.rollback().await;
            Err(error)
        }
    }
}

/// Get all fields for a note, for the "Edit All Fields" panel
pub(crate) async fn get_all_fields_for_edit(
    State(appstate): State<Arc<AppState>>,
    Path((note_id, commit_id)): Path<(i64, i32)>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    // Get the deck ID for this note to check authorization
    let client = database::client(&appstate).await?;
    let deck_row = client
        .query_opt(
            "SELECT deck FROM notes WHERE id = $1 AND deleted = false",
            &[&note_id],
        )
        .await?;

    let deck_id: i64 = match deck_row {
        Some(row) => row.get(0),
        None => {
            return Ok(Json(serde_json::json!({
                "error": "Note not found"
            })));
        }
    };

    // Check user has access to this deck
    if !access_check(&appstate, deck_id, &user).await? {
        return Ok(Json(serde_json::json!({
            "error": "Unauthorized"
        })));
    }

    // Verify the commit exists and is associated with a deck the user can access
    let commit_deck_row = client
        .query_opt(
            "SELECT deck FROM commits WHERE commit_id = $1",
            &[&commit_id],
        )
        .await?;

    if commit_deck_row.is_none() {
        return Ok(Json(serde_json::json!({
            "error": "Commit not found"
        })));
    }

    let commit_deck_id: i64 = commit_deck_row.unwrap().get(0);

    if !access_check(&appstate, commit_deck_id, &user).await? {
        return Ok(Json(serde_json::json!({
            "error": "Unauthorized"
        })));
    }

    match suggestion_manager::get_all_fields_for_edit(&appstate, note_id, commit_id).await {
        Ok(response) => Ok(Json(serde_json::json!(response))),
        Err(error) => {
            tracing::warn!(error = %error, note_id = note_id, commit_id = commit_id, "Failed to get all fields for edit");
            Ok(Json(serde_json::json!({
                "error": format!("{}", error)
            })))
        }
    }
}

/// Batch create or update field suggestions
pub(crate) async fn batch_update_field_suggestions(
    State(appstate): State<Arc<AppState>>,
    ClientIp(client_ip): ClientIp,
    user: User,
    Json(payload): Json<structs::BatchFieldSuggestionRequest>,
) -> Result<impl IntoResponse, Error> {
    // Get the deck ID for this note to check authorization
    let client = database::client(&appstate).await?;
    let deck_row = client
        .query_opt(
            "SELECT deck FROM notes WHERE id = $1 AND deleted = false",
            &[&payload.note_id],
        )
        .await?;

    let deck_id: i64 = match deck_row {
        Some(row) => row.get(0),
        None => {
            return Ok(Json(structs::BatchFieldSuggestionResponse {
                success: false,
                updated_count: 0,
                created_count: 0,
                fields: vec![],
            }));
        }
    };

    // Check user has access to this deck
    if !access_check(&appstate, deck_id, &user).await? {
        return Ok(Json(structs::BatchFieldSuggestionResponse {
            success: false,
            updated_count: 0,
            created_count: 0,
            fields: vec![],
        }));
    }

    // Verify the commit exists
    let commit_exists = client
        .query_opt(
            "SELECT 1 FROM commits WHERE commit_id = $1",
            &[&payload.commit_id],
        )
        .await?
        .is_some();

    if !commit_exists {
        return Ok(Json(structs::BatchFieldSuggestionResponse {
            success: false,
            updated_count: 0,
            created_count: 0,
            fields: vec![],
        }));
    }

    let ip_str = client_ip.to_string();
    let mut db_client = database::client(&appstate).await?;
    let tx = db_client.transaction().await?;

    match suggestion_manager::batch_create_or_update_field_suggestions(
        &tx,
        payload.note_id,
        payload.commit_id,
        &payload.fields,
        user.id(),
        &ip_str,
    )
    .await
    {
        Ok(results) => {
            // Calculate diffs for each modified field
            let mut field_results: Vec<structs::FieldUpdateResult> =
                Vec::with_capacity(results.len());
            let mut updated_count = 0;
            let mut created_count = 0;

            for result in results {
                match result.action.as_str() {
                    "updated" => updated_count += 1,
                    "created" => created_count += 1,
                    _ => {}
                }

                // Compute diff against the actual reviewed/published content
                let diff_html = htmldiff::htmldiff(&result.reviewed_content, &result.new_content);

                field_results.push(structs::FieldUpdateResult {
                    position: result.position,
                    field_id: result.field_id,
                    action: result.action,
                    diff_html,
                });
            }

            tx.commit().await?;

            // Update media references asynchronously
            let note_id = payload.note_id;
            let state_clone = appstate.clone();
            tokio::spawn(async move {
                if let Err(e) = media_reference_manager::update_media_references_note_state(
                    &state_clone,
                    note_id,
                )
                .await
                {
                    tracing::warn!(error = ?e, note_id = note_id, "Failed to update media references for note");
                }
            });

            Ok(Json(structs::BatchFieldSuggestionResponse {
                success: true,
                updated_count,
                created_count,
                fields: field_results,
            }))
        }
        Err(error) => {
            tracing::warn!(error = %error, note_id = payload.note_id, "Failed to batch update field suggestions");
            let _ = tx.rollback().await;
            Ok(Json(structs::BatchFieldSuggestionResponse {
                success: false,
                updated_count: 0,
                created_count: 0,
                fields: vec![],
            }))
        }
    }
}

/// Add a tag suggestion (addition or removal) to a note within a commit
pub(crate) async fn add_tag_suggestion(
    State(appstate): State<Arc<AppState>>,
    ClientIp(client_ip): ClientIp,
    user: User,
    Json(payload): Json<structs::AddTagSuggestionRequest>,
) -> Result<impl IntoResponse, Error> {
    // Get the deck ID for this note to check authorization
    let client = database::client(&appstate).await?;
    let deck_row = client
        .query_opt(
            "SELECT deck FROM notes WHERE id = $1 AND deleted = false",
            &[&payload.note_id],
        )
        .await?;

    let deck_id: i64 = match deck_row {
        Some(row) => row.get(0),
        None => {
            return Ok(Json(structs::AddTagSuggestionResponse {
                success: false,
                tag_id: None,
            }));
        }
    };

    if !access_check(&appstate, deck_id, &user).await? {
        return Ok(Json(structs::AddTagSuggestionResponse {
            success: false,
            tag_id: None,
        }));
    }

    let cleaned_content = cleanser::clean(&payload.content);
    if cleaned_content.is_empty() {
        return Ok(Json(structs::AddTagSuggestionResponse {
            success: false,
            tag_id: None,
        }));
    }

    let mut db_client = database::client(&appstate).await?;
    let tx = db_client.transaction().await?;

    let ip_str = client_ip.to_string();
    match suggestion_manager::create_tag_suggestion(
        &tx,
        payload.note_id,
        payload.commit_id,
        &cleaned_content,
        payload.action,
        user.id(),
        &ip_str,
    )
    .await
    {
        Ok(tag_id) => {
            tx.commit().await?;
            Ok(Json(structs::AddTagSuggestionResponse {
                success: true,
                tag_id: Some(tag_id),
            }))
        }
        Err(error) => {
            tracing::warn!(error = %error, note_id = payload.note_id, "Failed to add tag suggestion");
            let _ = tx.rollback().await;
            Ok(Json(structs::AddTagSuggestionResponse {
                success: false,
                tag_id: None,
            }))
        }
    }
}
