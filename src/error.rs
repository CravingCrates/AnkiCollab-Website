use axum::{
    extract::Request, http::StatusCode, response::{IntoResponse, Response}
};
use sentry::ClientInitGuard;
use serde::Serialize;
use thiserror::Error;
use tera::Context;

use tokio_postgres::Error as PgError;

use crate::{AppState, Html, Next, NoteId, State, format, option_env, str, usize};

use std::sync::Arc;

pub struct Reporter {
    _sentry: ClientInitGuard,
}

impl Default for Reporter {
    fn default() -> Self {
        Self::new()
    }
}

impl Reporter {
    pub fn new() -> Self {
        let endpoint = std::env::var("SENTRY_URL").unwrap();
        Self {
            _sentry: sentry::init((
                endpoint.as_str(),
                sentry::ClientOptions {
                    release: sentry::release_name!(),
                    traces_sample_rate: 0.2, // Performance monitoring, 0.0 to disable
                    ..Default::default()
                },
            )),
        }
    }
}

#[derive(Serialize, Debug, Clone)]
pub struct ErrorResponse {
    pub error: String,
}

impl ErrorResponse {
    pub fn new(err: impl std::fmt::Display) -> Self {
        Self {
            error: err.to_string(),
        }
    }
}

// Why is this a custom enum and not just built into error
#[derive(Debug, Error, Clone)]
pub enum NoteNotFoundContext {
    #[error("Tag Approve")]
    TagApprove,
    #[error("Tag Denied")]
    TagDenied,
    #[error("Field Approve")]
    FieldApprove,
    #[error("Field Denied")]
    FieldDenied,
    #[error("Field Update")]
    FieldUpdate,
    #[error("Mark Note Deleted")]
    MarkNoteDeleted,
    #[error("Approve Card")]
    ApproveCard,
    #[error("Invalid Data")]
    InvalidData,
    #[error("Delete Card")]
    DeleteCard,
    #[error("Note Moval Request")]
    NoteMovalRequest,
}

impl IntoResponse for NoteNotFoundContext {
    fn into_response(self) -> Response {
        let status_code = match self {
            Self::TagApprove => StatusCode::NOT_FOUND,
            Self::TagDenied => StatusCode::FORBIDDEN,
            Self::FieldApprove => StatusCode::NOT_FOUND,
            Self::FieldDenied => StatusCode::FORBIDDEN,
            Self::FieldUpdate => StatusCode::NOT_FOUND,
            Self::MarkNoteDeleted => StatusCode::NOT_FOUND,
            Self::ApproveCard => StatusCode::FORBIDDEN,
            Self::InvalidData => StatusCode::BAD_REQUEST,
            Self::DeleteCard => StatusCode::FORBIDDEN,
            Self::NoteMovalRequest => StatusCode::NOT_FOUND,
        };

        let mut response = Response::new(axum::body::Body::empty());
        *response.status_mut() = status_code;
        response.extensions_mut().insert(self);
        response
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Database error: {0}")]
    DB(#[from] tokio_postgres::Error),
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Template rendering error")]
    Template(#[from] tera::Error),
    #[error("BB8 error: {0}")]
    BB8(#[from] bb8_postgres::bb8::RunError<tokio_postgres::Error>),
    #[error("Tag already exists")]
    TagAlreadyExists,
    #[error("User not found")]
    UserNotFound,
    #[error("User is already a maintainer")]
    UserIsAlreadyMaintainer,
    #[error("Your folder ID is too long. Please double check it and try again.")]
    FolderIdTooLong,
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Commit not found")]
    CommitNotFound,
    #[error("Deck in Commit not found (Merge Commit).")]
    CommitDeckNotFound,
    #[error("Note not found. Context: {0}")]
    NoteNotFound(NoteNotFoundContext),
    #[error("Note is invalid")]
    InvalidNote,
    #[error("Fields are ambiguous. Please handle manually. Note: {0}")]
    AmbiguousFields(NoteId),
    #[error("No notes affected by this commit")]
    NoNotesAffected,
    #[error("No notetypes affected by this commit.")]
    NoNoteTypesAffected,
    #[error("Deck not found")]
    DeckNotFound,
    #[error("Error while authenticating: {0}")]
    Auth(AuthError),
    #[error("Database error: {0}")]
    Database(tokio_postgres::Error),
    #[error("Redirect: {0}")]
    Redirect(String),
    #[error("Unknown error")]
    Unknown,
    #[error("Database connection error")]
    DatabaseConnection,
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status_code = match &self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Redirect(_) => StatusCode::FOUND,
            Self::TagAlreadyExists => StatusCode::BAD_REQUEST,
            Self::UserIsAlreadyMaintainer => StatusCode::BAD_REQUEST,
            Self::NoNotesAffected => StatusCode::BAD_REQUEST,
            Self::FolderIdTooLong => StatusCode::BAD_REQUEST,
            Self::UserNotFound => StatusCode::NOT_FOUND,
            Self::CommitNotFound => StatusCode::NOT_FOUND,
            Self::CommitDeckNotFound => StatusCode::NOT_FOUND,
            Self::NoteNotFound(_) => StatusCode::NOT_FOUND,
            Self::DeckNotFound => StatusCode::NOT_FOUND,
            Self::AmbiguousFields(_) => StatusCode::BAD_REQUEST,
            Self::InvalidNote => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };

        if let Self::Redirect(path) = &self {
            return axum::response::Redirect::to(path).into_response();
        }

        let error_message = self.to_string();
        let mut response = Response::new(axum::body::Body::empty());
        *response.status_mut() = status_code;
        response.extensions_mut().insert(ErrorResponse::new(error_message));
        response
    }
}

impl From<AuthError> for Error {
    fn from(err: AuthError) -> Self {
        match err {
            AuthError::Redirect(path) => Self::Redirect(path),
            _ => Self::Auth(err),
        }
    }
}


#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Invalid credentials")]
    InvalidCredentials,
    #[error("Database error: {0}")]
    Database(#[from] PgError),
    #[error("Password hashing error: {0}")]
    PasswordHash(String),
    #[error("JWT error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    #[error("Not authenticated")]
    NotAuthenticated,
    #[error("Redirect to {0}")]
    Redirect(String),
    #[error("Username already in use")]
    UsernameAlreadyExists,
    #[error("Weak password")]
    PasswordWeak,
    #[error("Internal server error")]
    InternalError,
    #[error("Invalid token")]
    InvalidToken,
    #[error("User not found")]
    UserNotFound
}


impl Clone for AuthError {
    fn clone(&self) -> Self {
        match self {
            Self::InvalidCredentials => Self::InvalidCredentials,
            Self::PasswordHash(e) => Self::PasswordHash(e.clone()),
            Self::Jwt(e) => Self::Jwt(e.clone()),
            Self::NotAuthenticated => Self::NotAuthenticated,
            Self::Redirect(e) => Self::Redirect(e.clone()),
            Self::UsernameAlreadyExists => Self::UsernameAlreadyExists,
            Self::PasswordWeak => Self::PasswordWeak,
            Self::InternalError => Self::InternalError,
            Self::InvalidToken => Self::InvalidToken,
            Self::UserNotFound => Self::UserNotFound,
            Self::Database(_error) => Self::PasswordHash("Database Error".to_string()) // tokio_posgres::Error doesn't implement clone() so i'm kinda fucked and its 2am so i'm just gonna do this for now
            ,
        }
    }
}

impl AuthError {
    const fn get_status_and_message(&self) -> (StatusCode, &'static str) {
        match self {
            Self::NotAuthenticated => (
                StatusCode::UNAUTHORIZED,
                "Please log in to access this page",
            ),
            Self::InternalError => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal error occurred",
            ),
            Self::InvalidToken => (
                StatusCode::UNAUTHORIZED,
                "Your session has expired. Please log in again",
            ),
            Self::UserNotFound => (
                StatusCode::NOT_FOUND,
                "User not found",
            ),
            Self::InvalidCredentials => (
                StatusCode::UNAUTHORIZED,
                "Invalid username or password",
            ),
            Self::Database(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal Error 23110",
            ),
            Self::PasswordHash(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "An error occurred while hashing the password",
            ),
            Self::Jwt(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal Error 23810",
            ),
            Self::Redirect(_) => (StatusCode::FOUND, ""),
            Self::UsernameAlreadyExists => (
                StatusCode::BAD_REQUEST,
                "Username already in use",
            ),
            Self::PasswordWeak => (
                StatusCode::BAD_REQUEST,
                "Password is too weak",
            ),
        }
    }
}

// Create a wrapper middleware for pretty error pages
pub struct PrettyErrorHandler<S>(pub S);

// Extension trait to handle errors with templates
pub trait ErrorTemplate {
    fn render_error_template(&self, app_state: &Arc<AppState>) -> Response;
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status_code, error_msg) = self.get_status_and_message();
        let mut response = (status_code, error_msg).into_response();
        response.extensions_mut().insert(self);
        response
    }
}

impl ErrorTemplate for AuthError {
    fn render_error_template(&self, app_state: &Arc<AppState>) -> Response {
        let (status_code, error_msg) = self.get_status_and_message();
        let mut context = Context::new();
        context.insert("message", error_msg);
        
        match app_state.tera.render("error.html", &context) {
            Ok(html) => (status_code, Html(html)).into_response(),
            Err(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to render error template",
            ).into_response(),
        }
    }
}

impl ErrorTemplate for ErrorResponse {
    fn render_error_template(&self, app_state: &Arc<AppState>) -> Response {
        let status_code = StatusCode::BAD_REQUEST;
        let error_msg = self.error.clone();
        let mut context = Context::new();
        context.insert("message", &error_msg);

        match app_state.tera.render("error.html", &context) {
            Ok(html) => (status_code, Html(html)).into_response(),
            Err(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to render error template",
            ).into_response(),
        }
    }
}

impl ErrorTemplate for NoteNotFoundContext {
    fn render_error_template(&self, app_state: &Arc<AppState>) -> Response {
        let error_msg = self.to_string();
        let status_code = StatusCode::BAD_REQUEST;
        let mut context = Context::new();
        context.insert("message", &error_msg);
        match app_state.tera.render("error.html", &context) {
            Ok(html) => (status_code, Html(html)).into_response(),
            Err(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to render error template",
            ).into_response(),
        }
    }
}

// Middleware layer that wraps the handlers
pub async fn pretty_error_middleware(
    State(app_state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    // Process the request
    let response = next.run(request).await;
    
    // Handle AuthError
    if let Some(auth_error) = response.extensions().get::<AuthError>() {
        return auth_error.render_error_template(&app_state);
    }
    
    // Handle Error enum
    if let Some(error_respoonse) = response.extensions().get::<ErrorResponse>() {
        return error_respoonse.render_error_template(&app_state);
    }

    // Handle NoteNotFoundContext
    if let Some(note_not_found_context) = response.extensions().get::<NoteNotFoundContext>() {
        return note_not_found_context.render_error_template(&app_state);
    }

    // Handle 404
    if response.status() == axum::http::StatusCode::NOT_FOUND {
        let mut context = tera::Context::new();
        context.insert("message", "Page not found");
        if let Ok(html) = app_state.tera.render("error.html", &context) {
            return (axum::http::StatusCode::NOT_FOUND, Html(html)).into_response();
        }
    }
    
    response
}