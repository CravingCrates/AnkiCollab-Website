use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Redirect, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::database::{self, AppState};
use crate::error::{self, Error};
use crate::handlers::common::{check_login, error_page};
use crate::structs::{
    BulkNoteActionFailure, BulkNoteActionRequest, BulkNoteActionResponse, CommitDecisionRequest,
};
use crate::user::User;
use crate::{
    commit_manager, note_history, notetype_manager, notification_manager, suggestion_manager,
    webhook_manager,
};

pub(crate) async fn approve_commit(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Path(commit_id): Path<i32>,
) -> Result<impl IntoResponse, Error> {
    let actor_user_id = user.id();
    let reviewer_username = user.username();
    let pending_notes = webhook_manager::count_pending_commit_notes(&appstate, commit_id).await;
    let res = suggestion_manager::merge_by_commit(&appstate, commit_id, true, user).await?;

    notification_manager::create_commit_notification(
        &appstate,
        commit_id,
        "approved",
        None,
        actor_user_id,
    )
    .await?;

    if pending_notes > 0 {
        webhook_manager::spawn_commit_decision_webhook(
            Arc::clone(&appstate),
            commit_id,
            true,
            reviewer_username,
            None,
            false,
            pending_notes,
            false,
        );
    }

    Ok(match res {
        None => Redirect::to("/reviews"),
        Some(commit_id) => Redirect::to(&format!("/commit/{commit_id}")),
    })
}

pub(crate) async fn deny_commit(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Path(commit_id): Path<i32>,
    payload: Option<Json<CommitDecisionRequest>>,
) -> Result<impl IntoResponse, Error> {
    let actor_user_id = user.id();
    let reviewer_username = user.username();
    let decision = payload.map_or_default(|Json(p)| p);
    let silent = decision.silent.unwrap_or(false);
    let sanitized_reason = decision
        .reason
        .as_deref()
        .map(ammonia::clean)
        .map(|r| r.trim().chars().take(2000).collect::<String>())
        .filter(|r| !r.is_empty());
    let pending_notes = webhook_manager::count_pending_commit_notes(&appstate, commit_id).await;

    match suggestion_manager::merge_by_commit(&appstate, commit_id, false, user).await {
        Ok(res) => {
            if !silent {
                notification_manager::create_commit_notification(
                    &appstate,
                    commit_id,
                    "denied",
                    sanitized_reason.as_deref(),
                    actor_user_id,
                )
                .await?;
            }

            if pending_notes > 0 {
                webhook_manager::spawn_commit_decision_webhook(
                    Arc::clone(&appstate),
                    commit_id,
                    false,
                    reviewer_username,
                    if silent { None } else { sanitized_reason },
                    silent,
                    pending_notes,
                    false,
                );
            }

            match res {
                Some(commit_id) => Ok(Redirect::to(&format!("/commit/{commit_id}"))),
                None => Ok(Redirect::to("/reviews")),
            }
        }
        Err(error) => {
            tracing::warn!(error = %error, commit_id = commit_id, "Failed to deny commit");
            Err(error)
        }
    }
}

pub(crate) async fn bulk_note_action(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Path(commit_id): Path<i32>,
    Json(payload): Json<BulkNoteActionRequest>,
) -> Result<impl IntoResponse, Error> {
    let silent = payload.silent.unwrap_or(false);
    let sanitized_reason = payload
        .reason
        .as_deref()
        .map(ammonia::clean)
        .map(|r| r.trim().chars().take(2000).collect::<String>())
        .filter(|r| !r.is_empty());

    // Validate action
    let approve = match payload.action.as_str() {
        "approve" => true,
        "deny" => false,
        _ => {
            return Ok(Json(BulkNoteActionResponse {
                succeeded: vec![],
                failed: vec![BulkNoteActionFailure {
                    id: 0,
                    reason: format!(
                        "Invalid action '{}'. Expected 'approve' or 'deny'.",
                        payload.action
                    ),
                }],
            }));
        }
    };

    // Validate note_ids is not empty
    if payload.note_ids.is_empty() {
        return Ok(Json(BulkNoteActionResponse {
            succeeded: vec![],
            failed: vec![BulkNoteActionFailure {
                id: 0,
                reason: "No note IDs provided".to_string(),
            }],
        }));
    }

    // Call the merge function
    let results = suggestion_manager::merge_by_note_ids(
        &appstate,
        commit_id,
        &payload.note_ids,
        approve,
        &user,
    )
    .await?;

    // Convert results to response format
    let succeeded: Vec<i64> = results
        .iter()
        .filter(|r| r.success)
        .map(|r| r.note_id)
        .collect();

    let failed: Vec<BulkNoteActionFailure> = results
        .iter()
        .filter(|r| !r.success)
        .map(|r| BulkNoteActionFailure {
            id: r.note_id,
            reason: r
                .reason
                .clone()
                .unwrap_or_else(|| "Unknown error".to_string()),
        })
        .collect();

    if !succeeded.is_empty() {
        if approve {
            notification_manager::create_commit_notification(
                &appstate,
                commit_id,
                "approved",
                None,
                user.id(),
            )
            .await?;
        } else if !silent {
            notification_manager::create_commit_notification(
                &appstate,
                commit_id,
                "denied",
                sanitized_reason.as_deref(),
                user.id(),
            )
            .await?;
        }

        let remaining_pending =
            webhook_manager::count_pending_commit_notes(&appstate, commit_id).await;
        webhook_manager::spawn_commit_decision_webhook(
            Arc::clone(&appstate),
            commit_id,
            approve,
            user.username(),
            if approve || silent {
                None
            } else {
                sanitized_reason
            },
            if approve { false } else { silent },
            succeeded.len(),
            remaining_pending > 0 || !failed.is_empty(),
        );
    }

    Ok(Json(BulkNoteActionResponse { succeeded, failed }))
}

#[derive(Default, Deserialize)]
pub(crate) struct ReviewCommitQuery {
    offset: Option<i64>,
    limit: Option<i64>,
    format: Option<String>,
}

#[derive(Serialize)]
struct NotesPageResponse {
    html: String,
    total: i64,
    offset: i64,
    limit: i64,
    loaded: usize,
    next_offset: Option<i64>,
}

pub(crate) async fn review_commit(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Path(commit_id): Path<i32>,
    Query(params): Query<ReviewCommitQuery>,
) -> Result<Response, Error> {
    const PAGE_INCREMENT: i64 = 50;

    let wants_json = params
        .format
        .as_deref()
        .is_some_and(|f| f.eq_ignore_ascii_case("json"));

    let raw_offset = params.offset.unwrap_or(0).max(0);
    let sanitized_offset = raw_offset - (raw_offset % PAGE_INCREMENT);

    let default_limit = if wants_json { PAGE_INCREMENT } else { 100 };
    let requested_limit = params.limit.unwrap_or(default_limit);
    let sanitized_limit = match requested_limit {
        100 if !wants_json => 100,
        50 => PAGE_INCREMENT,
        _ => default_limit,
    };

    let notes_page =
        commit_manager::notes_by_commit(&appstate, commit_id, sanitized_offset, sanitized_limit)
            .await?;
    let notes_loaded = notes_page.notes.len();

    let commit = commit_manager::get_commit_info(&appstate, commit_id).await?;

    let client = database::client(&appstate).await?;
    let q_guid = client
        .query(
            "Select deck from commits where commit_id = $1",
            &[&commit_id],
        )
        .await?;
    if q_guid.is_empty() {
        return error_page(&appstate, &error::Error::CommitNotFound.to_string())
            .map(IntoResponse::into_response);
    }
    let deck_id: i64 = q_guid[0].get(0);

    let access = suggestion_manager::is_authorized(&appstate, &user, deck_id).await?;
    let notemodels = notetype_manager::notetypes_by_commit(&appstate, commit_id).await?;

    if wants_json {
        let mut fragment_context = tera::Context::new();
        fragment_context.insert("notes", &notes_page.notes);
        fragment_context.insert("notes_total", &notes_page.total);
        fragment_context.insert("user", &user);
        fragment_context.insert("owned", &access);
        fragment_context.insert("notemodels", &notemodels);
        fragment_context.insert("commit", &commit);

        let notes_html = appstate
            .tera
            .render("partials/commit_notes.html", &fragment_context)
            .expect("Failed to render notes fragment");

        let response = NotesPageResponse {
            html: notes_html,
            total: notes_page.total,
            offset: notes_page.offset,
            limit: notes_page.limit,
            loaded: notes_loaded,
            next_offset: notes_page.next_offset,
        };

        return Ok(Json(response).into_response());
    }

    let mut context = tera::Context::new();
    context.insert("notes", &notes_page.notes);
    context.insert("notes_total", &notes_page.total);
    context.insert("notes_loaded", &notes_loaded);
    context.insert("notes_next_offset", &notes_page.next_offset);
    context.insert("notes_offset", &notes_page.offset);
    context.insert("notes_limit", &notes_page.limit);
    context.insert("commit", &commit);
    context.insert("user", &user);
    context.insert("owned", &access);
    context.insert("notemodels", &notemodels);

    let rendered_template = appstate
        .tera
        .render("commit.html", &context)
        .expect("Failed to render template");

    Ok(Html(rendered_template).into_response())
}

pub(crate) async fn review_commit_preview(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Path(commit_id): Path<i32>,
) -> Result<Response, Error> {
    let deck_id: i64 = {
        let client = database::client(&appstate).await?;
        let q_guid = client
            .query(
                "Select deck from commits where commit_id = $1",
                &[&commit_id],
            )
            .await?;
        if q_guid.is_empty() {
            return error_page(&appstate, &error::Error::CommitNotFound.to_string())
                .map(IntoResponse::into_response);
        }
        q_guid[0].get(0)
    };

    let access = suggestion_manager::is_authorized(&appstate, &user, deck_id).await?;
    if !access {
        return error_page(&appstate, &error::Error::Unauthorized.to_string())
            .map(IntoResponse::into_response);
    }

    let (commit, notes_page, notemodels) = tokio::try_join!(
        commit_manager::get_commit_info(&appstate, commit_id),
        commit_manager::preview_notes_by_commit(&appstate, commit_id, 0, 5),
        notetype_manager::notetypes_by_commit(&appstate, commit_id),
    )?;

    let mut context = tera::Context::new();
    context.insert("commit", &commit);
    context.insert("notes", &notes_page.notes);
    context.insert("notes_total", &notes_page.total);
    context.insert("notemodels", &notemodels);
    context.insert("user", &user);
    context.insert("owned", &access);

    let rendered_template = appstate
        .tera
        .render("partials/commit_preview_drawer.html", &context)
        .expect("Failed to render template");

    Ok(Html(rendered_template).into_response())
}

// Show all notes impacted by a commit via events aggregation
pub(crate) async fn commit_history_page(
    State(appstate): State<Arc<AppState>>,
    Path(commit_id): Path<i32>,
    user: Option<User>,
) -> Result<impl IntoResponse, Error> {
    if user.is_none() {
        return Ok(Redirect::to("/login").into_response());
    }
    let client = database::client(&appstate).await?;
    let notes = note_history::fetch_commit_history(&client, commit_id).await?;
    let mut context = tera::Context::new();
    context.insert("commit_id", &commit_id);
    context.insert("notes", &notes);
    context.insert("user", &user);
    let rendered_template = appstate.tera.render("commit_history.html", &context)?;
    Ok(Html(rendered_template).into_response())
}

pub(crate) async fn all_reviews(
    State(appstate): State<Arc<AppState>>,
    user: Option<User>,
) -> Result<impl IntoResponse, Error> {
    let user = check_login(user)?;
    let mut context = tera::Context::new();

    let commits = match commit_manager::commits_review(&appstate, user.id()).await {
        Ok(commits) => commits,
        Err(error) => {
            tracing::warn!(error = %error, "Failed to get commits for review");
            return Ok(Html("Error getting the reviews.".to_string()));
        }
    };

    context.insert("commits", &commits);
    //context.insert("notes", &notes);
    context.insert("user", &user);

    let rendered_template = appstate
        .tera
        .render("reviews.html", &context)
        .expect("Failed to render template");
    Ok(Html(rendered_template))
}
