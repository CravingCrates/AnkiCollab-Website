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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contribution_manager::{
        CommunityHealth, ContributorRationale, ContributorRow, ContributorSummary, Funnel,
        MilestoneSummary, RationaleRow, ReviewSpeed, SubdeckBacklogRow, SubscriberSummary,
        SuggestionActivityBucket, SuggestionRow, TeamRow, TimelineBucket, TurnaroundBucket,
    };
    use crate::structs::{
        DeckBaseStatsInfo, DeckStatsInfo, NoteStatsInfo, NotetypeStatsInfo, RetentionBucket,
        RetentionTrendPoint, TagStatsInfo,
    };

    #[test]
    fn statistics_and_contributions_templates_compile_and_render() {
        let tera = build_tera();

        let base = DeckBaseStatsInfo {
            note_count: 120,
            total_reviewed_notes: 300,
            coverage_pct: 40.0,
            lapses_avg: 1.8,
            reps_avg: 7.4,
            retention_avg: 84.5,
            contributor_count: 18,
            median_sample_size: 9.0,
            leech_count: 4,
            recently_revised_struggling: 2,
            trend_delta_30d: Some(1.6),
        };
        let decks = vec![
            DeckStatsInfo {
                hash: "abc".to_string(),
                path: "Med::Anatomy".to_string(),
                prefix_path: "Med::".to_string(),
                leaf_name: "Anatomy".to_string(),
                depth: 1,
                retention: Some(86.2),
                tracked_notes: 80,
                total_notes: 120,
                coverage_pct: 66.7,
                lapses_avg: Some(1.5),
                reps_avg: Some(6.8),
                leech_count: 2,
            },
            DeckStatsInfo {
                hash: "def".to_string(),
                path: "Med::Embryology".to_string(),
                prefix_path: "Med::".to_string(),
                leaf_name: "Embryology".to_string(),
                depth: 1,
                retention: None,
                tracked_notes: 0,
                total_notes: 50,
                coverage_pct: 0.0,
                lapses_avg: None,
                reps_avg: None,
                leech_count: 0,
            },
        ];
        let notes = vec![NoteStatsInfo {
            id: 42,
            fields: "<b>Front</b> question".to_string(),
            lapses: 4.5,
            reps: 14.0,
            retention: 62.5,
            sample_size: 8,
            retention_stddev: Some(14.2),
            subdeck_path: "Med::Anatomy".to_string(),
            subdeck_prefix: "Med::".to_string(),
            subdeck_leaf: "Anatomy".to_string(),
            has_pending_suggestion: true,
            last_updated: Some("2026-11-10".to_string()),
            days_since_update: Some(11),
            revised_recently: true,
            is_leech: true,
        }];
        let buckets = vec![RetentionBucket {
            label: "85–90%".to_string(),
            range_key: "85_90".to_string(),
            all_count: 50,
            confident_count: 35,
            tone: "success".to_string(),
        }];
        let trend = vec![
            RetentionTrendPoint {
                captured_on: "2026-10-20".to_string(),
                avg_retention: 82.9,
                avg_lapses: 2.0,
                tracked_notes: 100,
            },
            RetentionTrendPoint {
                captured_on: "2026-11-20".to_string(),
                avg_retention: 84.5,
                avg_lapses: 1.8,
                tracked_notes: 120,
            },
        ];
        let tags = vec![TagStatsInfo {
            tag: "Med::Anatomy::Thorax".to_string(),
            prefix: "Med::Anatomy::".to_string(),
            leaf: "Thorax".to_string(),
            note_count: 15,
            avg_retention: 74.2,
            avg_lapses: 2.4,
            avg_reps: 8.1,
            leech_count: 3,
        }];
        let notetypes = vec![
            NotetypeStatsInfo {
                name: "Cloze".to_string(),
                note_count: 90,
                avg_retention: 85.0,
                avg_lapses: 1.6,
                avg_reps: 7.0,
            },
            NotetypeStatsInfo {
                name: "Basic".to_string(),
                note_count: 30,
                avg_retention: 82.1,
                avg_lapses: 2.1,
                avg_reps: 8.0,
            },
        ];

        let mut stats_ctx = tera::Context::new();
        stats_ctx.insert("user", &serde_json::json!({ "username": "alice" }));
        stats_ctx.insert("deck_hash", "abc");
        stats_ctx.insert("deck_name", "Anatomy");
        stats_ctx.insert("stats_enabled", &true);
        stats_ctx.insert("active_tab", "statistics");
        stats_ctx.insert("base", &base);
        stats_ctx.insert("decks", &decks);
        stats_ctx.insert("notes", &notes);
        stats_ctx.insert("retention_buckets", &buckets);
        stats_ctx.insert("confident_bucket_total", &35_i64);
        stats_ctx.insert("retention_trend", &trend);
        stats_ctx.insert("tag_stats", &tags);
        stats_ctx.insert("notetype_stats", &notetypes);
        stats_ctx.insert("count_all", &1_usize);
        stats_ctx.insert("count_attention", &0_usize);
        stats_ctx.insert("count_confident", &1_usize);
        stats_ctx.insert("count_leech", &1_usize);
        stats_ctx.insert("count_revised", &1_usize);
        stats_ctx.insert("hist_labels", &vec!["85–90%"]);
        stats_ctx.insert("hist_all", &vec![50_i64]);
        stats_ctx.insert("hist_confident", &vec![35_i64]);
        stats_ctx.insert("trend_labels", &vec!["2026-10-20", "2026-11-20"]);
        stats_ctx.insert("trend_retention", &vec![82.9_f32, 84.5_f32]);
        stats_ctx.insert("trend_lapses", &vec![2.0_f32, 1.8_f32]);

        let rendered_stats = tera.render("statistics.html", &stats_ctx).unwrap();
        assert!(rendered_stats.contains("Anatomy"));
        assert!(!rendered_stats.contains("style=\""));

        let rendered_empty = tera.render("empty_stats.html", &stats_ctx).unwrap();
        assert!(rendered_empty.contains("No review statistics yet"));

        let mut contrib_ctx = tera::Context::new();
        contrib_ctx.insert("user", &serde_json::json!({ "username": "alice" }));
        contrib_ctx.insert("deck_hash", "abc");
        contrib_ctx.insert("deck_name", "Anatomy");
        contrib_ctx.insert("stats_enabled", &true);
        contrib_ctx.insert("active_tab", "contributions");
        contrib_ctx.insert("digest", &["Line 1", "Line 2", "Line 3"]);
        contrib_ctx.insert(
            "funnel",
            &Funnel {
                submitted: 20,
                accepted: 12,
                declined: 4,
                pending: 3,
                unrecorded: 1,
                acceptance_ratio: Some(0.75),
            },
        );
        contrib_ctx.insert(
            "speed",
            &ReviewSpeed {
                median_hours: Some(12.5),
                p90_hours: Some(54.0),
                pending: 3,
                oldest_pending_days: Some(4),
            },
        );
        contrib_ctx.insert(
            "team",
            &vec![TeamRow {
                name: Some("alice".to_string()),
                reviewed: 16,
                reviewed_30d: 6,
                accepted: 12,
                declined: 4,
                share_pct: 100.0,
            }],
        );
        contrib_ctx.insert(
            "contributors",
            &vec![ContributorRow {
                username: "bob".to_string(),
                username_url: "bob".to_string(),
                submitted: 7,
                accepted: 5,
                declined: 1,
                pending: 1,
                last_contribution: Some("2026-11-18".to_string()),
            }],
        );
        contrib_ctx.insert("top_contributors", &Vec::<ContributorRow>::new());
        contrib_ctx.insert("distinct_contributors", &6_i64);
        contrib_ctx.insert("contributors_cap", &100_i64);
        contrib_ctx.insert("breakdown_visible", &true);
        contrib_ctx.insert("tracking_since", &Some("2026-10-01"));
        contrib_ctx.insert("activity_tracked", &true);
        contrib_ctx.insert(
            "timeline",
            &vec![TimelineBucket {
                label: "2026-11-20".to_string(),
                total: 240,
                active_30d: 180,
                active_90d: 210,
                new: 4,
                unsubscribed: 1,
                net: 3,
            }],
        );
        contrib_ctx.insert(
            "subscriber_summary",
            &SubscriberSummary {
                total: 240,
                net_30d: Some(18),
                active_30d: Some(180),
                active_90d: Some(210),
                stickiness_30d_pct: Some(75.0),
                stickiness_90d_pct: Some(87.5),
            },
        );
        contrib_ctx.insert(
            "rationales",
            &vec![RationaleRow {
                rationale: "Content error".to_string(),
                submitted: 10,
                accepted: 8,
                declined: 1,
                pending: 1,
                share_pct: 50.0,
                acceptance_ratio: Some(8.0 / 9.0),
            }],
        );
        contrib_ctx.insert(
            "turnaround_buckets",
            &vec![TurnaroundBucket {
                label: "< 24 hours".to_string(),
                count: 10,
                share_pct: 62.5,
                tone: "success".to_string(),
            }],
        );
        contrib_ctx.insert("turnaround_total", &16_i64);
        contrib_ctx.insert(
            "subdeck_backlog",
            &vec![SubdeckBacklogRow {
                path: "Med::Anatomy".to_string(),
                prefix: "Med::".to_string(),
                leaf: "Anatomy".to_string(),
                pending: 2,
                oldest_days: Some(4),
            }],
        );
        contrib_ctx.insert(
            "community_health",
            &CommunityHealth {
                active_contributors_30d: 4,
                returning_contributors: 3,
                notes_improved: 42,
            },
        );
        contrib_ctx.insert(
            "suggestion_timeline",
            &vec![SuggestionActivityBucket {
                label: "2026-11-20".to_string(),
                submitted: 5,
                accepted: 4,
                declined: 1,
            }],
        );
        contrib_ctx.insert("bucket", "daily");
        contrib_ctx.insert("active_filter", "all");
        contrib_ctx.insert("window_days", &365_i64);
        contrib_ctx.insert("chart_labels", &vec!["2026-11-20"]);
        contrib_ctx.insert("chart_total", &vec![240_i64]);
        contrib_ctx.insert("chart_active_30d", &vec![180_i64]);
        contrib_ctx.insert("chart_active_90d", &vec![210_i64]);
        contrib_ctx.insert("chart_new", &vec![4_i64]);
        contrib_ctx.insert("chart_unsubscribed", &vec![1_i64]);
        contrib_ctx.insert("sugg_labels", &vec!["2026-11-20"]);
        contrib_ctx.insert("sugg_submitted", &vec![5_i64]);
        contrib_ctx.insert("sugg_accepted", &vec![4_i64]);
        contrib_ctx.insert("sugg_declined", &vec![1_i64]);

        let rendered_contrib = tera.render("contributions.html", &contrib_ctx).unwrap();
        assert!(rendered_contrib.contains("Suggestion Activity"));
        assert!(!rendered_contrib.contains("style=\""));

        let mut drill_ctx = tera::Context::new();
        drill_ctx.insert("user", &serde_json::json!({ "username": "alice" }));
        drill_ctx.insert("deck_hash", "abc");
        drill_ctx.insert("deck_name", "Anatomy");
        drill_ctx.insert("username", "bob");
        drill_ctx.insert("username_url", "bob");
        drill_ctx.insert("state", "open");
        drill_ctx.insert(
            "summary",
            &ContributorSummary {
                submitted: 7,
                accepted: 5,
                declined: 1,
                pending: 1,
                acceptance_ratio: Some(5.0 / 6.0),
                first_contribution: Some("2026-09-01".to_string()),
                last_contribution: Some("2026-11-18".to_string()),
                milestone: MilestoneSummary {
                    current_name: Some("Regular Contributor"),
                    next_name: Some("Trusted Contributor"),
                    remaining_to_next: Some(5),
                    progress_pct: 0,
                },
                top_rationales: vec![ContributorRationale {
                    rationale: "Content error".to_string(),
                    count: 5,
                }],
            },
        );
        drill_ctx.insert(
            "suggestions",
            &vec![SuggestionRow {
                commit_id: 99,
                info: Some("Fix diagram".to_string()),
                submitted_at: "2026-11-18 14:20".to_string(),
                rationale: "Content error".to_string(),
                open: true,
                accepted: false,
                declined: false,
            }],
        );
        drill_ctx.insert("suggestions_truncated", &false);

        let rendered_drill = tera
            .render("contributions_contributor.html", &drill_ctx)
            .unwrap();
        assert!(rendered_drill.contains("Regular Contributor"));
        assert!(!rendered_drill.contains("style=\""));
    }
}
