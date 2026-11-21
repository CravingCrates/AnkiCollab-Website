use std::sync::Arc;

use axum::{
    extract::{Query, State},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

use crate::database::AppState;
use crate::error::Error;
use crate::notification_manager;
use crate::structs::{
    NotificationHistoryResponse, NotificationMarkReadRequest, NotificationMarkReadResponse,
    NotificationUnreadResponse,
};
use crate::user::User;

#[derive(Default, Deserialize)]
pub(crate) struct NotificationHistoryQuery {
    offset: Option<i64>,
    limit: Option<i64>,
}

pub(crate) async fn get_notifications(
    State(appstate): State<Arc<AppState>>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let response: NotificationUnreadResponse =
        notification_manager::get_unread_grouped(&appstate, user.id()).await?;
    Ok(Json(response))
}

pub(crate) async fn get_notifications_history(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Query(params): Query<NotificationHistoryQuery>,
) -> Result<impl IntoResponse, Error> {
    let offset = params.offset.unwrap_or(0).max(0);
    let limit = params.limit.unwrap_or(50).clamp(1, 200);
    let response: NotificationHistoryResponse =
        notification_manager::get_history(&appstate, user.id(), offset, limit).await?;
    Ok(Json(response))
}

pub(crate) async fn mark_notifications_read(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Json(payload): Json<NotificationMarkReadRequest>,
) -> Result<impl IntoResponse, Error> {
    if payload.ids.len() > notification_manager::MAX_MARK_READ_IDS {
        return Err(Error::BadRequest(format!(
            "Too many IDs (max {})",
            notification_manager::MAX_MARK_READ_IDS
        )));
    }
    let updated = notification_manager::mark_read(&appstate, user.id(), &payload.ids).await?;
    Ok(Json(NotificationMarkReadResponse { updated }))
}
