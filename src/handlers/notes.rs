use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect},
    Json,
};

use crate::database::{self, AppState};
use crate::error::{self, Error, NoteNotFoundContext};
use crate::handlers::common::error_page;
use crate::user::User;
use crate::{
    media_reference_manager, note_history, note_manager, structs, suggestion_manager,
    webhook_manager,
};

pub(crate) async fn review_note(
    State(appstate): State<Arc<AppState>>,
    Path(note_id): Path<i64>,
    user: Option<User>,
) -> Result<impl IntoResponse, Error> {
    let mut context = tera::Context::new();

    // Require a logged-in user for any note review view (no anonymous browsing)
    if user.is_none() {
        return Ok(Redirect::to("/login").into_response());
    }

    let note = match note_manager::get_note_data(&appstate, note_id).await {
        Ok(note) => note,
        Err(_error) => {
            return error_page(
                &appstate,
                &error::Error::NoteNotFound(NoteNotFoundContext::InvalidData).to_string(),
            )
            .map(axum::response::IntoResponse::into_response);
        }
    };

    if note.id == 0 {
        // Invalid data // No note found!
        return error_page(
            &appstate,
            &error::Error::NoteNotFound(NoteNotFoundContext::InvalidData).to_string(),
        )
        .map(axum::response::IntoResponse::into_response);
    }

    // access boolean previously used for template conditions; removed as unused

    // Safe unwrap: we early-returned if user.is_none()
    let current_user = user.as_ref().unwrap();
    let client = database::client(&appstate).await?;
    let q_guid = client
        .query("Select deck from notes where id = $1", &[&note_id])
        .await?;
    if q_guid.is_empty() {
        return error_page(
            &appstate,
            &error::Error::NoteNotFound(NoteNotFoundContext::InvalidData).to_string(),
        )
        .map(axum::response::IntoResponse::into_response);
    }
    let deck_id: i64 = q_guid[0].get(0);
    let access = suggestion_manager::is_authorized(&appstate, current_user, deck_id).await?;

    context.insert("note", &note);
    context.insert("access", &access);
    context.insert("user", &user);
    let rendered_template = appstate
        .tera
        .render("review.html", &context)
        .expect("Failed to render template");

    // Return the rendered HTML as the response
    Ok(Html(rendered_template).into_response())
}

// Fetch recent history events for a note (newest first)
pub(crate) async fn note_history_page(
    State(appstate): State<Arc<AppState>>,
    Path(note_id): Path<i64>,
    user: Option<User>,
) -> Result<impl IntoResponse, Error> {
    if user.is_none() {
        return Ok(Redirect::to("/login").into_response());
    }

    let client = database::client(&appstate).await?;
    let row_opt = client
        .query_opt("SELECT deck FROM notes WHERE id = $1", &[&note_id])
        .await?;
    if row_opt.is_none() {
        return error_page(
            &appstate,
            &error::Error::NoteNotFound(NoteNotFoundContext::InvalidData).to_string(),
        )
        .map(axum::response::IntoResponse::into_response);
    }
    let deck_id: i64 = row_opt.unwrap().get(0);
    let u = user.as_ref().unwrap();
    let _ = suggestion_manager::is_authorized(&appstate, u, deck_id).await?; // we still render even if not owner; access boolean not used here yet
    let history = note_history::fetch_note_history(&client, note_id).await?;
    let mut context = tera::Context::new();
    context.insert("note_id", &note_id);
    context.insert("events", &history.events);
    context.insert("groups", &history.groups);
    context.insert("actors", &history.actors);
    context.insert("user", &user);
    let rendered_template = appstate.tera.render("note_history.html", &context)?;
    Ok(Html(rendered_template).into_response())
}

pub(crate) async fn accept_note(
    State(appstate): State<Arc<AppState>>,
    Path(note_id): Path<i64>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let pending_commit = webhook_manager::pending_commit_for_note(&appstate, note_id).await;
    let reviewer_username = user.username();
    let mut client = database::client(&appstate).await?;
    let tx = client.transaction().await?;
    match suggestion_manager::approve_card(&tx, &appstate, note_id, &user, false).await {
        Ok(res) => {
            tx.commit().await?;
            if let Some(cid) = pending_commit {
                webhook_manager::spawn_single_note_review_webhook(
                    Arc::clone(&appstate),
                    cid,
                    note_id,
                    true,
                    reviewer_username,
                    false,
                );
            }
            // Update media references post-commit for the approved note
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
            tracing::warn!(error = %error, note_id = note_id, "Failed to approve note");
            let _ = tx.rollback().await;
            Err(error)
        }
    }
}

// This actually removes the note from the database (Only used for notes that are not approved yet)
pub(crate) async fn deny_note(
    State(appstate): State<Arc<AppState>>,
    Path(note_id): Path<i64>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let pending_commit = webhook_manager::pending_commit_for_note(&appstate, note_id).await;
    let reviewer_username = user.username();
    match suggestion_manager::delete_card(&appstate, note_id, user).await {
        Ok(res) => {
            if let Some(cid) = pending_commit {
                webhook_manager::spawn_single_note_review_webhook(
                    Arc::clone(&appstate),
                    cid,
                    note_id,
                    false,
                    reviewer_username,
                    false,
                );
            }
            Ok(Redirect::to(&format!("/notes/{res}")))
        }
        Err(error) => {
            tracing::warn!(error = %error, note_id = note_id, "Failed to delete note");
            Err(error)
        }
    }
}

// This marks the note as deleted, but does not remove them (Used for existing notes that are approved)
pub(crate) async fn remove_note_from_deck(
    State(appstate): State<Arc<AppState>>,
    Path(note_id): Path<i64>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let reviewer_username = user.username();
    let mut client = database::client(&appstate).await?;
    let deletion_commit: Option<i32> = client
        .query_opt(
            "SELECT commit FROM card_deletion_suggestions WHERE note = $1 AND commit IS NOT NULL LIMIT 1",
            &[&note_id],
        )
        .await
        .ok()
        .flatten()
        .map(|r| r.get(0));
    let tx = client.transaction().await?;
    match note_manager::mark_note_deleted(&tx, &appstate, note_id, user, false).await {
        Ok(res) => {
            tx.commit().await?;
            if let Some(cid) = deletion_commit {
                webhook_manager::spawn_single_note_review_webhook(
                    Arc::clone(&appstate),
                    cid,
                    note_id,
                    true,
                    reviewer_username,
                    false,
                );
            }
            // Post-commit cleanup of media references for denied note
            if let Ok(nid) = res.parse::<i64>() {
                let state_clone = appstate.clone();
                tokio::spawn(async move {
                    if let Err(e) =
                        media_reference_manager::cleanup_media_for_denied_note(&state_clone, nid)
                            .await
                    {
                        tracing::warn!(error = ?e, note_id = nid, "Failed to cleanup media references");
                    }
                });
            }
            Ok(Redirect::to(&format!("/notes/{res}")))
        }
        Err(error) => {
            tracing::warn!(error = %error, note_id = note_id, "Failed to mark note as deleted");
            let _ = tx.rollback().await;
            Err(error)
        }
    }
}

pub(crate) async fn deny_note_removal(
    State(appstate): State<Arc<AppState>>,
    Path(note_id): Path<i64>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let reviewer_username = user.username();
    let deletion_commit: Option<i32> = match database::client(&appstate).await {
        Ok(client) => client
            .query_opt(
                "SELECT commit FROM card_deletion_suggestions WHERE note = $1 AND commit IS NOT NULL LIMIT 1",
                &[&note_id],
            )
            .await
            .ok()
            .flatten()
            .map(|r| r.get(0)),
        Err(_) => None,
    };
    match note_manager::deny_note_removal_request(&appstate, note_id, user).await {
        Ok(res) => {
            if let Some(cid) = deletion_commit {
                webhook_manager::spawn_single_note_review_webhook(
                    Arc::clone(&appstate),
                    cid,
                    note_id,
                    false,
                    reviewer_username,
                    true,
                );
            }
            Ok(Redirect::to(&format!("/review/{res}")))
        }
        Err(error) => {
            tracing::warn!(error = %error, note_id = note_id, "Failed to deny note removal request");
            // Convert Box<dyn Error> to our Error type
            Err(error::Error::Unknown)
        }
    }
}

pub(crate) async fn get_presigned_url(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Json(data): Json<structs::PresignedURLRequest>,
) -> Result<impl IntoResponse, Error> {
    let mut response: structs::PresignedURLResponse = structs::PresignedURLResponse {
        success: false,
        presigned_url: String::new(),
    };

    if data.filename.is_empty() || data.context_type != "note" {
        return Ok(Json(response));
    }

    let parsed_nid = data.context_id.parse::<i64>().unwrap_or(0);
    if parsed_nid == 0 {
        return Ok(Json(response));
    }
    let presigned_url = match media_reference_manager::get_presigned_url(
        &appstate,
        &data.filename,
        parsed_nid,
        user.id(),
    )
    .await
    {
        Ok(presigned_url) => presigned_url,
        Err(_error) => return Ok(Json(response)),
    };

    response.success = true;
    response.presigned_url = presigned_url;

    Ok(Json(response))
}
