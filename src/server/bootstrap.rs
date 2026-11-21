use std::{env, sync::Arc};

use aws_sdk_s3::Client as S3Client;
use sentry::integrations::tracing::BreadcrumbLayer;
use tera::Tera;
use tracing_subscriber::{
    filter::LevelFilter, layer::SubscriberExt, util::SubscriberInitExt, Layer,
};

use crate::database::{self, AppState};
use crate::user::Auth;
use crate::{media_tokens, pairing, passkey};

fn build_tera() -> Tera {
    let mut tera = Tera::default();
    // Register filters that were moved from Tera core to tera-contrib in v2
    tera.register_filter("striptags", tera_contrib::regex::striptags);
    tera.register_filter("json_encode", tera_contrib::json::json_encode);
    match tera.load_from_glob("src/templates/**/*.html") {
        Ok(t) => t,
        Err(e) => {
            eprintln!("FATAL: Template parsing error(s): {e}");
            ::std::process::exit(1);
        }
    }

    tera.autoescape_on(vec![".html", ".sql", ".htm", ".xml"]);
    tera
}

async fn build_s3_client() -> S3Client {
    let s3_access_key_id = std::env::var("S3_ACCESS_KEY_ID").expect("S3_ACCESS_KEY_ID must be set");
    let s3_secret_access_key =
        std::env::var("S3_SECRET_ACCESS_KEY").expect("S3_SECRET_ACCESS_KEY must be set");
    let s3_domain = std::env::var("S3_DOMAIN").expect("S3_DOMAIN must be set");

    let credentials = aws_sdk_s3::config::Credentials::new(
        s3_access_key_id,
        s3_secret_access_key,
        None,
        None,
        "s3-credentials",
    );

    let region_provider =
        aws_config::meta::region::RegionProviderChain::default_provider().or_else("eu-central-1"); // Europe (Frankfurt)
    let s3_config = aws_config::from_env()
        .region(region_provider)
        .credentials_provider(aws_sdk_s3::config::SharedCredentialsProvider::new(
            credentials,
        ))
        .endpoint_url(&s3_domain)
        .load()
        .await;

    let s3_service_config = aws_sdk_s3::config::Builder::from(&s3_config)
        .force_path_style(true) // Contabo is <special>
        .build();

    S3Client::from_conf(s3_service_config)
}

pub(crate) async fn build_app_state() -> Arc<AppState> {
    let tera = build_tera();

    let pool = database::establish_pool_connection()
        .await
        .expect("Failed to establish database connection pool");

    let s3_client = build_s3_client().await;

    // Initialize media token service
    let media_token_secret =
        std::env::var("MEDIA_TOKEN_SECRET").expect("MEDIA_TOKEN_SECRET must be set");
    let media_token_service = media_tokens::MediaTokenService::new(
        media_token_secret.into_bytes(),
        std::time::Duration::from_mins(5), // 5 minutes
    )
    .expect("Failed to initialize media token service");

    // Fails the process rather than falling back to a default: an RP_ID/RP_ORIGIN
    // mismatch fails silently in the browser, so a misconfigured deployment must
    // not start. The resolved pair is logged once by webauthn_from_env.
    let (webauthn, site_origin) = passkey::webauthn_from_env();
    let webauthn = Arc::new(webauthn);

    // Per-address rate limiting for the unauthenticated passkey sign-in endpoints.
    let passkey_guard = Arc::new(passkey::PasskeyGuard::new());

    Arc::new(database::AppState {
        db_pool: Arc::new(pool),
        tera: Arc::new(tera),
        s3_client,
        media_token_service,
        webauthn,
        site_origin,
        passkey_challenges: Arc::new(passkey::ChallengeStore::new()),
        passkey_guard,
        pairing_attempts: Arc::new(pairing::CodeGuessTracker::new()),
    })
}

pub(crate) fn init_tracing() {
    // Enable tracing.
    let env_filter = if cfg!(debug_assertions) {
        // Debug build
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            format!(
                "{}=debug,tower_http=debug,axum=trace",
                env!("CARGO_CRATE_NAME")
            )
            .into()
        })
    } else {
        // Release build
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            format!(
                "{}=info,tower_http=info,axum=info",
                env!("CARGO_CRATE_NAME")
            )
            .into()
        })
    };

    // Only capture breadcrumbs for context, not events.
    // Real errors are captured explicitly via sentry::capture_message in error.rs
    let sentry_layer = BreadcrumbLayer::new().with_filter(LevelFilter::WARN);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(sentry_layer)
        .with(tracing_subscriber::fmt::layer().without_time())
        .init();
}

pub(crate) fn spawn_background_sweeps(state: &Arc<AppState>) {
    let challenge_store = state.passkey_challenges.clone();
    let passkey_guard = state.passkey_guard.clone();
    let pairing_attempts = state.pairing_attempts.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_mins(1));
        challenge_store.retain_recent();
        passkey_guard.sweep();
        pairing_attempts.retain_recent();
        if !challenge_store.is_empty() {
            tracing::debug!(
                outstanding = challenge_store.len(),
                "pruned expired WebAuthn challenges"
            );
        }
    });
}

pub(crate) async fn build_auth() -> Arc<Auth> {
    // Second db connection for the auth. idk.. should prolly use the pool for this too
    let (client, connection) = tokio_postgres::connect(
        &env::var("DATABASE_URL").expect("DATABASE_URL must be set"),
        tokio_postgres::NoTls,
    )
    .await
    .expect("Failed to connect to database");
    // Spawn connection handling
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            tracing::error!(error = %e, "Database connection error");
        }
    });
    let db = Arc::new(client);
    // Create Auth instance
    let jwt_secret = env::var("JWT_SECRET").expect("JWT_SECRET must be set");
    Arc::new(Auth::new(
        db.clone(),
        jwt_secret,
        env::var("COOKIE_SECURE").unwrap_or("false".to_string()) == "true",
    ))
}
