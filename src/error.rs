use rocket::{
    http::Status,
    response::{self, status::*, Responder},
    serde::Serialize,
};
// use sentry::ClientInitGuard;
use thiserror::Error;

use crate::*;

#[derive(Serialize, Debug)]
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

#[derive(Debug, Error)]
pub enum NoteNotFoundReason {
    #[error("Tag Approve")]
    TagApprove,
    #[error("Tag Denied")]
    TagDenied,
    #[error("Field Approve")]
    FieldApprove,
    #[error("Field Denied")]
    FieldDenied,
    #[error("Mark Note Deleted")]
    MarkNoteDeleted,
    #[error("Approve Card")]
    ApproveCard,
    #[error("Invalid Data")]
    InvalidData,
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("Database error: {0}")]
    DB(#[from] tokio_postgres::Error),
    #[error("Unauthorized")]
    Unauthorized,
    #[error("Error while authenticating: {0}")]
    AuthenticationError(#[from] rocket_auth::Error),
    #[error("Redirecting to {0}")]
    Redirect(&'static str),
    #[error("Template rendering error")]
    Template(#[from] tera::Error),
    #[error("BB8 error: {0}")]
    BB8(#[from] bb8_postgres::bb8::RunError<tokio_postgres::Error>),
    #[error("Tab already exists")]
    TabAlreadyExists,
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
    #[error("Note not found. Reason:{0}")]
    NoteNotFound(NoteNotFoundReason),
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
}

impl<'r> Responder<'r, 'static> for Error {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'static> {
        let e = Some(Json(ErrorResponse::new(&self)));
        println!("{:?}", &self);
        // let uuid = sentry::capture_error(&self);
        // dbg!(uuid);
        match self {
            Self::Unauthorized => Unauthorized(e).respond_to(req),
            Self::AuthenticationError(_) => Unauthorized(e).respond_to(req),
            Self::Redirect(url) => Redirect::to(url).respond_to(req),
            Self::TabAlreadyExists => BadRequest(e).respond_to(req),
            Self::UserNotFound => NotFound(e).respond_to(req),
            Self::UserIsAlreadyMaintainer => BadRequest(e).respond_to(req),
            // Self::EmailNotFound => Unauthorized(e).respond_to(req),
            // Self::NotSubscribed => Forbidden(e).respond_to(req),
            // Self::NoContent => BadRequest(e).respond_to(req),
            // Self::DocumentNotFound => NotFound(e).respond_to(req),
            // Self::UnsupportedFileFormat => BadRequest(e).respond_to(req),
            // Self::EmailDuplicate => BadRequest(e).respond_to(req),
            // Self::NotEnoughCredits(_, _) => Forbidden(e).respond_to(req),
            // Inspect DB Error for duplicate email error code
            // Self::DB(e)
            //     if e.as_database_error().is_some()
            //         && e.as_database_error().unwrap().code() == Some(Cow::Borrowed("23505")) =>
            // {
            //     let e = Some(Json(ErrorResponse::new(&Error::EmailDuplicate)));
            //     BadRequest(e).respond_to(req)
            // }
            _ => {
                dbg!(&self);
                Status::InternalServerError.respond_to(req)
            }
        }
    }
}

// pub struct Reporter {
//     _sentry: ClientInitGuard,
// }
