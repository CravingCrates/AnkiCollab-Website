use axum::{
    extract::Request,
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
};
use sentry::{protocol::Event, ClientInitGuard, Level};
use serde::Serialize;
use tera::Context;
use thiserror::Error;

use tokio_postgres::Error as PgError;

use crate::{format, option_env, str, usize, AppState, Html, Next, NoteId, State};

use std::sync::Arc;

/// Categorizes errors for structured Sentry reporting without exposing PII.
#[derive(Debug, Clone, Copy)]
pub enum ErrorCategory {
    Database,
    Authentication,
    Authorization,
    NotFound,
    Validation,
    External,
    Internal,
}

impl std::fmt::Display for ErrorCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Database => write!(f, "database"),
            Self::Authentication => write!(f, "authentication"),
            Self::Authorization => write!(f, "authorization"),
            Self::NotFound => write!(f, "not_found"),
            Self::Validation => write!(f, "validation"),
            Self::External => write!(f, "external_service"),
            Self::Internal => write!(f, "internal"),
        }
    }
}

pub struct Reporter {
    _sentry: ClientInitGuard,
}

impl Default for Reporter {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if an error message represents an expected client error that should not be sent to Sentry.
/// These are user-facing errors that are properly handled, not bugs.
fn is_expected_client_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    // Authentication/authorization errors - expected user mistakes
    lower.contains("invalid credentials")
        || lower.contains("invalid username or password")
        || lower.contains("username already in use")
        || lower.contains("password is too weak")
        || lower.contains("not authenticated")
        || lower.contains("please log in")
        || lower.contains("session has expired")
        || lower.contains("unauthorized")
        || lower.contains("invalid token")
        // Validation errors - expected user input issues
        || lower.contains("tag already exists")
        || lower.contains("user is already a maintainer")
        || lower.contains("folder id is too long")
        || lower.contains("not found") // 404s are client errors
        || lower.contains("ambiguous fields")
        || lower.contains("account has been deleted")
}

impl Reporter {
    pub fn new() -> Self {
        let endpoint = std::env::var("SENTRY_URL").unwrap();
        let before_send = Some(Arc::new(|mut event: Event<'static>| {
            // Drop expected client errors - these are properly handled, not bugs
            if event.message.as_ref().is_some_and(|m| is_expected_client_error(m)) {
                return None;
            }
            if event.exception.iter().any(|exc| {
                is_expected_client_error(exc.value.as_deref().unwrap_or(""))
            }) {
                return None;
            }

            // Drop known benign database constraint violations to avoid noise and quota burn.
            if event
                .exception
                .iter()
                .any(|exc| is_benign_db_error(exc.value.as_deref().unwrap_or("")))
            {
                return None;
            }

            // Also drop transaction or connection pool timeouts that are benign under load
            if event.exception.iter().any(|exc| {
                let msg = exc.value.as_deref().unwrap_or("").to_ascii_lowercase();
                msg.contains("timed out waiting for connection")
                    || msg.contains("connection reset")
            }) {
                return None;
            }

            // Strip request/PII-like data to stay GDPR-friendly.
            if let Some(req) = event.request.as_mut() {
                req.headers.clear();
                req.cookies = None;
                req.query_string = None;
                req.env.clear();
                req.data = None;
                // Keep sanitized path for debugging (strip IDs from paths)
                if let Some(url) = &req.url {
                    let sanitized = sanitize_path(url.as_str());
                    // Parse back to URL, falling back to clearing on error
                    req.url = sentry::protocol::Url::parse(&sanitized).ok();
                }
            }
            event.user = None;
            // Keep structured tags but clear potentially sensitive extra data
            event.extra.retain(|k, _| {
                matches!(
                    k.as_str(),
                    "error.category" | "error.code" | "http.status" | "http.method"
                )
            });

            Some(event)
        }) as Arc<dyn Fn(Event<'static>) -> Option<Event<'static>> + Send + Sync>);

        Self {
            _sentry: sentry::init((
                endpoint.as_str(),
                sentry::ClientOptions {
                    release: sentry::release_name!(),
                    traces_sample_rate: 0.2, // Performance monitoring, 0.0 to disable
                    sample_rate: 0.35,       // Error event sampling
                    send_default_pii: false,
                    before_send,
                    ..Default::default()
                },
            )),
        }
    }
}

use once_cell::sync::Lazy;

/// Pre-compiled regexes for path sanitization (compiled once at startup)
static RE_NUMERIC_ID: Lazy<regex::Regex> =
    Lazy::new(|| regex::Regex::new(r"/\d+(?:/|$)").expect("Invalid numeric ID regex"));
static RE_DECK_HASH: Lazy<regex::Regex> = Lazy::new(|| {
    regex::Regex::new(
        r"/(EditDeck|notes|Statistics|Maintainers|OptionalTags|ToggleStats|DeckSubscriptionPolicy)/[a-zA-Z0-9_-]+",
    )
    .expect("Invalid deck hash regex")
});

/// Sanitize URL paths by replacing numeric IDs and hashes with placeholders.
/// This helps with GDPR compliance and Sentry grouping.
fn sanitize_path(path: &str) -> String {
    // Replace numeric IDs in paths (e.g., /review/12345 -> /review/{id}/)
    let result = RE_NUMERIC_ID.replace_all(path, "/{id}/");
    
    // Replace deck hashes in known routes (e.g., /EditDeck/abc123 -> /EditDeck/{hash})
    let result = RE_DECK_HASH.replace_all(&result, "/$1/{hash}");
    
    result.to_string()
}

#[derive(Serialize, Debug, Clone)]
pub struct ErrorResponse {
    pub error: String,
    #[serde(skip)]
    pub category: Option<ErrorCategory>,
}

impl ErrorResponse {
    pub fn new(err: impl std::fmt::Display) -> Self {
        Self {
            error: err.to_string(),
            category: None,
        }
    }

    pub fn with_category(err: impl std::fmt::Display, category: ErrorCategory) -> Self {
        Self {
            error: err.to_string(),
            category: Some(category),
        }
    }
}

/// Captures server errors to Sentry with structured context.
/// Only captures 5xx errors and filters out benign/expected errors.
fn capture_server_error(
    status: StatusCode,
    method: &Method,
    path: &str,
    message: &str,
    category: Option<ErrorCategory>,
) {
    // Only capture actual server errors (5xx), not client errors (4xx)
    if !status.is_server_error() {
        return;
    }

    // Skip benign errors that would clutter Sentry
    if is_benign_db_error(message) {
        return;
    }

    // Skip expected client errors that shouldn't be in Sentry
    if is_expected_client_error(message) {
        return;
    }

    let sanitized_path = sanitize_path(path);
    let category_str = category.map_or("unknown".to_string(), |c| c.to_string());

    // Note: We only use sentry::capture_message here, NOT tracing::error!
    // The sentry_layer in main.rs auto-captures ERROR level tracing events,
    // so using tracing::error! here would cause double-capture.
    // For local logging of server errors, the Sentry capture is sufficient.

    // Capture to Sentry with structured tags
    sentry::with_scope(
        |scope| {
            scope.set_tag("http.status", status.as_str());
            scope.set_tag("http.method", method.as_str());
            scope.set_tag("http.target", &sanitized_path);
            scope.set_tag("error.category", &category_str);
            scope.set_level(Some(Level::Error));
        },
        || {
            sentry::capture_message(message, Level::Error);
        },
    );
}

/// Log client errors (4xx) for debugging without sending to Sentry.
/// These are typically user errors and don't need alerting.
/// Expected client errors (auth failures, validation, etc.) are silenced entirely.
fn log_client_error(status: StatusCode, method: &Method, path: &str, message: &str) {
    if status.is_client_error() && !is_expected_client_error(message) {
        let sanitized_path = sanitize_path(path);
        tracing::warn!(
            http.status = %status,
            http.method = %method,
            http.path = %sanitized_path,
            "Client error: {}", message
        );
    }
}

fn is_benign_db_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("duplicate key value")
        || lower.contains("sqlstate 23505")
        || lower.contains("notes_deck_guid_unique")
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
    #[error("Log Event")]
    NoteLogEvent,

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
            Self::NoteLogEvent => StatusCode::NOT_FOUND,
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
    #[error("Bad request: {0}")]
    BadRequest(String),
}

impl Error {
    /// Returns the error category for structured logging and Sentry tagging.
    pub const fn category(&self) -> ErrorCategory {
        match self {
            Self::DB(_) | Self::BB8(_) | Self::Database(_) | Self::DatabaseConnection => {
                ErrorCategory::Database
            }
            Self::Unauthorized | Self::Auth(_) => ErrorCategory::Authorization,
            Self::UserNotFound | Self::CommitNotFound | Self::CommitDeckNotFound
            | Self::NoteNotFound(_) | Self::DeckNotFound | Self::NoNotesAffected
            | Self::NoNoteTypesAffected => ErrorCategory::NotFound,
            Self::TagAlreadyExists | Self::UserIsAlreadyMaintainer | Self::FolderIdTooLong
            | Self::InvalidNote | Self::AmbiguousFields(_) | Self::Serialization(_)
            | Self::BadRequest(_) => {
                ErrorCategory::Validation
            }
            Self::Template(_) => ErrorCategory::Internal,
            Self::Redirect(_) | Self::Unknown => ErrorCategory::Internal,
        }
    }
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
            Self::NoNoteTypesAffected => StatusCode::BAD_REQUEST,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            // Database and BB8 errors should return 503 Service Unavailable, not 500
            Self::DB(_) | Self::BB8(_) | Self::Database(_) | Self::DatabaseConnection => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            Self::Template(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Serialization(_) => StatusCode::BAD_REQUEST,
            Self::Auth(_) => StatusCode::UNAUTHORIZED,
            Self::Unknown => StatusCode::INTERNAL_SERVER_ERROR,
        };

        if let Self::Redirect(path) = &self {
            return axum::response::Redirect::to(path).into_response();
        }

        let error_message = self.to_string();
        let category = self.category();
        let mut response = Response::new(axum::body::Body::empty());
        *response.status_mut() = status_code;
        response
            .extensions_mut()
            .insert(ErrorResponse::with_category(&error_message, category));
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
    UserNotFound,
    #[error("Account has been deleted")]
    AccountDeleted,
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
            Self::AccountDeleted => Self::AccountDeleted,
            Self::Database(_error) => {
                // tokio_postgres::Error doesn't implement Clone, so we degrade gracefully.
                Self::PasswordHash("Database Error".to_string())
            }
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
            Self::UserNotFound => (StatusCode::NOT_FOUND, "User not found"),
            Self::InvalidCredentials => (StatusCode::UNAUTHORIZED, "Invalid username or password"),
            Self::Database(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Internal Error 23110"),
            Self::PasswordHash(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "An error occurred while hashing the password",
            ),
            Self::Jwt(_) => (StatusCode::INTERNAL_SERVER_ERROR, "Internal Error 23810"),
            Self::Redirect(_) => (StatusCode::FOUND, ""),
            Self::UsernameAlreadyExists => (StatusCode::BAD_REQUEST, "Username already in use"),
            Self::PasswordWeak => (StatusCode::BAD_REQUEST, "Password is too weak"),
            Self::AccountDeleted => (StatusCode::FORBIDDEN, "This account has been deleted"),
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
            )
                .into_response(),
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
            )
                .into_response(),
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
            )
                .into_response(),
        }
    }
}

// Middleware layer that wraps the handlers
pub async fn pretty_error_middleware(
    State(app_state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();

    // Process the request
    let response = next.run(request).await;
    let status_code = response.status();

    // Handle AuthError
    if let Some(auth_error) = response.extensions().get::<AuthError>() {
        let (status, _) = auth_error.get_status_and_message();
        capture_server_error(
            status,
            &method,
            &path,
            &auth_error.to_string(),
            Some(ErrorCategory::Authentication),
        );
        log_client_error(status, &method, &path, &auth_error.to_string());
        return auth_error.render_error_template(&app_state);
    }

    // Handle Error enum
    if let Some(error_response) = response.extensions().get::<ErrorResponse>() {
        let category = error_response.category;
        if status_code.is_server_error() {
            capture_server_error(
                status_code,
                &method,
                &path,
                &error_response.error,
                category,
            );
        } else {
            log_client_error(status_code, &method, &path, &error_response.error);
        }
        return error_response.render_error_template(&app_state);
    }

    // Handle NoteNotFoundContext
    if let Some(note_not_found_context) = response.extensions().get::<NoteNotFoundContext>() {
        // These are client errors (404/403), not server errors
        log_client_error(
            status_code,
            &method,
            &path,
            &note_not_found_context.to_string(),
        );
        return note_not_found_context.render_error_template(&app_state);
    }

    // Handle 404 - don't send to Sentry, just log locally
    if status_code == axum::http::StatusCode::NOT_FOUND {
        tracing::debug!(http.path = %path, "Page not found");
        let mut context = tera::Context::new();
        context.insert("message", "Page not found");
        if let Ok(html) = app_state.tera.render("error.html", &context) {
            return (axum::http::StatusCode::NOT_FOUND, Html(html)).into_response();
        }
    }

    // Capture uncategorized server errors so they are visible in Sentry.
    if status_code.is_server_error() {
        capture_server_error(
            status_code,
            &method,
            &path,
            "Unhandled server error - response produced without error context",
            Some(ErrorCategory::Internal),
        );
    }

    response
}
