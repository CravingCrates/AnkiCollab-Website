use std::sync::Arc;

use axum::{
    http::{header, HeaderValue},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Extension, Router,
};
use axum_client_ip::ClientIpSource;
use governor::{clock::QuantaInstant, middleware::NoOpMiddleware};
use tower::ServiceBuilder;
use tower_governor::{
    governor::{GovernorConfig, GovernorConfigBuilder},
    key_extractor::PeerIpKeyExtractor,
    GovernorLayer,
};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use crate::database::AppState;
use crate::handlers::{
    auth as auth_handlers, commits as commit_handlers, contributions as contribution_handlers,
    decks as deck_handlers, maintainers as maintainer_handlers, notes as note_handlers,
    notetypes as notetype_handlers, notifications as notification_handlers, pages as page_handlers,
    stats as stats_handlers, subscription_policy as subscription_policy_handlers,
    suggestions as suggestion_handlers,
};
use crate::user::Auth;
use crate::{error, pairing, passkey};

async fn set_static_cache_control(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    response
}

pub(crate) type WebsiteGovernorConfig =
    Arc<GovernorConfig<PeerIpKeyExtractor, NoOpMiddleware<QuantaInstant>>>;

pub(crate) fn init_governor() -> WebsiteGovernorConfig {
    let governor_conf = Arc::new(GovernorConfigBuilder::default().finish().unwrap());

    let governor_limiter = governor_conf.limiter().clone();
    let interval = std::time::Duration::from_mins(1);
    // a separate background task to clean up
    std::thread::spawn(move || loop {
        std::thread::sleep(interval);
        governor_limiter.retain_recent();
    });

    governor_conf
}

pub(crate) fn build_router(
    state: Arc<AppState>,
    auth: Arc<Auth>,
    governor_conf: &WebsiteGovernorConfig,
) -> Router {
    let passkey_routes = Router::new()
        .route(
            "/profile/passkeys/register/start",
            post(passkey::post_register_start),
        )
        .route(
            "/profile/passkeys/register/finish",
            post(passkey::post_register_finish),
        )
        .route(
            "/profile/passkeys/delete",
            post(passkey::post_delete_passkey),
        )
        .route("/login/passkey/start", post(passkey::post_login_start))
        .route("/login/passkey/finish", post(passkey::post_login_finish))
        .layer(GovernorLayer::new(governor_conf.clone()))
        .layer(axum::middleware::map_response(passkey::no_store));

    let auth_routes = Router::new()
        .route(
            "/login",
            get(auth_handlers::get_login).post(auth_handlers::post_login),
        )
        .route(
            "/signup",
            get(auth_handlers::get_signup).post(auth_handlers::post_signup),
        )
        .route(
            "/profile/change-password",
            post(auth_handlers::post_change_password),
        )
        .route(
            "/profile/delete-account",
            post(auth_handlers::delete_account),
        )
        .route("/pair", get(pairing::get_pair))
        .route("/pair/approve", post(pairing::post_pair_approve))
        .layer(GovernorLayer::new(governor_conf.clone()));

    Router::new()
        .merge(auth_routes)
        .merge(passkey_routes)
        .route("/", get(page_handlers::index))
        .route("/terms", get(page_handlers::terms))
        .route("/privacy", get(page_handlers::privacy))
        .route("/imprint", get(page_handlers::imprint))
        .route("/datenschutz", get(page_handlers::datenschutz))
        .route("/accessibility", get(page_handlers::accessibility_page))
        .route("/logout", get(auth_handlers::logout))
        .route("/welcome", get(auth_handlers::get_welcome))
        .route("/profile", get(auth_handlers::get_profile))
        .route(
            "/Maintainers/{deck_hash}",
            get(maintainer_handlers::show_maintainers),
        )
        .route("/Maintainers", post(maintainer_handlers::post_maintainers))
        .route(
            "/EditNotetype/{notetype_id}",
            get(notetype_handlers::edit_notetype),
        )
        .route(
            "/EditNotetype",
            post(notetype_handlers::post_edit_notetype)
                .layer(axum::extract::DefaultBodyLimit::max(5 * 1024 * 1024)), // 5MB limit for notetype updates (to allow large CSS/templates
        )
        .route("/EditDeck/{deck_hash}", get(deck_handlers::edit_deck))
        .route("/EditDeck", post(deck_handlers::post_edit_deck))
        .route(
            "/DeckSubscriptionPolicy/{subscriber_hash}/{base_hash}",
            get(subscription_policy_handlers::page_subscription_policy),
        )
        .route(
            "/api/subscription-field-policy",
            get(subscription_policy_handlers::api_get_subscription_policy)
                .post(subscription_policy_handlers::api_post_subscription_policy),
        )
        .route(
            "/DeleteChangelog/{changelog_id}",
            post(deck_handlers::delete_changelog),
        )
        .route("/DeleteDeck/{deck_hash}", post(deck_handlers::delete_deck))
        .route("/leavereview", get(page_handlers::forward_donation))
        .route("/decks", get(deck_handlers::deck_overview))
        .route(
            "/notes/{deck_hash}",
            get(deck_handlers::get_notes_from_deck),
        )
        .route("/ManageDecks", get(deck_handlers::manage_decks))
        .route("/review/{note_id}", get(note_handlers::review_note))
        .route(
            "/ToggleStats/{deck_hash}",
            post(stats_handlers::toggle_stats),
        )
        .route(
            "/Statistics/{deck_hash}",
            get(stats_handlers::show_statistics),
        )
        .route(
            "/Statistics/{deck_hash}/sections",
            get(stats_handlers::show_statistics_sections),
        )
        .route(
            "/Contributions/{deck_hash}",
            get(contribution_handlers::show_contributions),
        )
        .route(
            "/Contributions/{deck_hash}/contributor/{username}",
            get(contribution_handlers::show_contributor_suggestions),
        )
        .route(
            "/UpdateStatsPages/{secret}",
            get(stats_handlers::refresh_stats_cache),
        )
        .route(
            "/DenyNoteRemoval/{note_id}",
            post(note_handlers::deny_note_removal),
        )
        .route(
            "/AcceptNoteRemoval/{note_id}",
            post(note_handlers::remove_note_from_deck),
        )
        .route("/DenyTag/{tag_id}", post(suggestion_handlers::deny_tag))
        .route("/AcceptTag/{tag_id}", post(suggestion_handlers::accept_tag))
        .route(
            "/DenyNoteMove/{move_id}",
            post(suggestion_handlers::deny_note_move),
        )
        .route(
            "/AcceptNoteMove/{move_id}",
            post(suggestion_handlers::accept_note_move),
        )
        .route(
            "/DenyField/{field_id}",
            post(suggestion_handlers::deny_field),
        )
        .route(
            "/AcceptField/{field_id}",
            post(suggestion_handlers::accept_field),
        )
        //.route("/UpdateFieldSuggestion", post(update_field))
        .route(
            "/GetAllFieldsForEdit/{note_id}/{commit_id}",
            get(suggestion_handlers::get_all_fields_for_edit),
        )
        .route(
            "/BatchUpdateFieldSuggestions",
            post(suggestion_handlers::batch_update_field_suggestions),
        )
        .route(
            "/AddTagSuggestion",
            post(suggestion_handlers::add_tag_suggestion),
        )
        .route(
            "/DenyCommit/{commit_id}",
            post(commit_handlers::deny_commit),
        )
        .route(
            "/ApproveCommit/{commit_id}",
            post(commit_handlers::approve_commit),
        )
        .route(
            "/BulkNoteAction/{commit_id}",
            post(commit_handlers::bulk_note_action),
        )
        .route(
            "/GetNotifications",
            get(notification_handlers::get_notifications),
        )
        .route(
            "/GetNotificationsHistory",
            get(notification_handlers::get_notifications_history),
        )
        .route(
            "/MarkNotificationsRead",
            post(notification_handlers::mark_notifications_read),
        )
        .route("/commit/{commit_id}", get(commit_handlers::review_commit))
        .route(
            "/commit_preview/{commit_id}",
            get(commit_handlers::review_commit_preview),
        )
        .route(
            "/note_history/{note_id}",
            get(note_handlers::note_history_page),
        )
        .route(
            "/commit_history/{commit_id}",
            get(commit_handlers::commit_history_page),
        )
        .route("/reviews", get(commit_handlers::all_reviews))
        .route("/DeleteNote/{note_id}", post(note_handlers::deny_note))
        .route("/AcceptNote/{note_id}", post(note_handlers::accept_note))
        .route("/GetImageFile", post(note_handlers::get_presigned_url))
        .nest_service(
            "/static",
            ServiceBuilder::new()
                .layer(middleware::from_fn(set_static_cache_control))
                .service(
                    ServeDir::new("src/templates/static")
                        .precompressed_br()
                        .precompressed_gzip(),
                ),
        )
        .layer((
            TraceLayer::new_for_http(),
            // Graceful shutdown will wait for outstanding requests to complete. Add a timeout so
            // requests don't hang forever. Causes issues for streaming large decks that take more than 10secs to generate. hence i disabled it
            //TimeoutLayer::new(Duration::from_secs(10)),
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            error::pretty_error_middleware,
        ))
        .with_state(state)
        .layer(Extension(auth))
        .layer(ClientIpSource::CfConnectingIp.into_extension())
    //.layer(ClientIpSource::ConnectInfo.into_extension());
}
