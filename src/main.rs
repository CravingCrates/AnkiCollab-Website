#![warn(clippy::all)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::too_many_lines,
    clippy::manual_let_else,
    clippy::implicit_hasher,
    clippy::similar_names,
    clippy::case_sensitive_file_extension_comparisons
)]

pub mod changelog_manager;
pub mod cleanser;
pub mod commit_manager;
pub mod contribution_manager;
pub mod database;
pub mod error;
mod handlers;
pub mod maintainer_manager;
pub mod media_reference_manager;
pub mod media_tokens;
pub mod note_history;
pub mod note_manager;
pub mod notetype_manager;
pub mod notification_manager;
pub mod optional_tags_manager;
pub mod pairing;
pub mod passkey;
mod server;
pub mod stats_cache;
pub mod stats_manager;
pub mod structs;
pub mod suggestion_manager;
pub mod user;

use std::net::SocketAddr;

pub(crate) use axum::{extract::State, middleware::Next, response::Html};
pub(crate) use database::AppState;
pub(crate) use error::{Error, NoteNotFoundContext, Reporter};
pub(crate) use std::{format, option_env, str};
pub(crate) use structs::{DeckHash, DeckId, NoteId, Return, UserId};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().expect(
        "Expected .env file in the root directory containing the database connection string",
    );
    let _reporter = Reporter::new();

    let state = server::bootstrap::build_app_state().await;

    server::bootstrap::init_tracing();

    let governor_conf = server::router::init_governor();

    server::bootstrap::spawn_background_sweeps(&state);

    let auth = server::bootstrap::build_auth().await;

    let app = server::router::build_router(state, auth, &governor_conf);

    // run it
    let listener = tokio::net::TcpListener::bind(
        std::env::var("BIND_ADDRESS").unwrap_or_else(|_| "localhost:1337".to_string()),
    )
    .await
    .unwrap();
    tracing::info!("Server listening on {}", listener.local_addr().unwrap());
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(server::shutdown::shutdown_signal())
    .await
    .unwrap();
}
