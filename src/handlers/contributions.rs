use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse},
};
use serde::Deserialize;

use crate::contribution_manager;
use crate::database::{self, AppState};
use crate::error::{self, Error};
use crate::handlers::common::{access_check, check_login, error_page};
use crate::structs::Return;
use crate::user::User;

#[derive(Deserialize)]
pub(crate) struct ContributionsQuery {
    bucket: Option<String>,
    active: Option<String>,
    days: Option<i64>,
}

impl ContributionsQuery {
    fn granularity(&self) -> contribution_manager::Granularity {
        match self.bucket.as_deref() {
            Some("weekly") => contribution_manager::Granularity::Weekly,
            _ => contribution_manager::Granularity::Daily,
        }
    }

    fn active_filter(&self) -> &'static str {
        match self.active.as_deref() {
            Some("30") => "30",
            Some("90") => "90",
            _ => "all",
        }
    }

    fn window_days(&self) -> i64 {
        self.days.unwrap_or(365).clamp(7, 1095)
    }
}

async fn contributions_deck(
    appstate: &Arc<AppState>,
    deck_hash: &str,
) -> Return<Option<(i64, String, bool)>> {
    let client = database::client(appstate).await?;
    let row = client
        .query_opt(
            "SELECT id,
                    COALESCE(NULLIF(BTRIM(full_path), ''), NULLIF(BTRIM(name), ''), ''),
                    COALESCE(stats_enabled, false)
               FROM anki.decks
              WHERE human_hash = $1 AND deleted_at IS NULL",
            &[&deck_hash],
        )
        .await?;
    Ok(row.map(|row| {
        let path: String = row.get(1);
        let display = contribution_manager::deck_display_name_from_path(&path);
        (row.get(0), display, row.get(2))
    }))
}

pub(crate) async fn show_contributions(
    State(appstate): State<Arc<AppState>>,
    Path(deck_hash): Path<String>,
    Query(params): Query<ContributionsQuery>,
    user: Option<User>,
) -> Result<impl IntoResponse, Error> {
    let user = check_login(user)?;

    let Some((deck_id, deck_name, stats_enabled)) =
        contributions_deck(&appstate, &deck_hash).await?
    else {
        return error_page(&appstate, "Deck not found");
    };

    if !access_check(&appstate, deck_id, &user).await? {
        return Err(error::Error::Unauthorized);
    }

    let window_days = params.window_days();
    let granularity = params.granularity();
    let active_filter = params.active_filter();

    let funnel = contribution_manager::get_funnel(&appstate, deck_id).await?;
    let speed = contribution_manager::get_review_speed(&appstate, deck_id).await?;
    let team = contribution_manager::get_team(&appstate, deck_id).await?;
    let contributors = contribution_manager::get_contributors(&appstate, deck_id).await?;
    let top_contributors =
        contribution_manager::get_top_contributors(&appstate, deck_id, 30).await?;
    let month_impact = contribution_manager::get_month_impact(&appstate, deck_id).await?;
    let distinct_contributors =
        contribution_manager::count_contributors(&appstate, deck_id).await?;
    let subscribers = contribution_manager::count_subscribers(&appstate, deck_id).await?;
    let tracking_since = contribution_manager::tracking_since(&appstate, deck_id).await?;
    let activity_tracked = contribution_manager::activity_tracked(&appstate, deck_id).await?;
    let (snapshots, new_by_date) =
        contribution_manager::get_timeline(&appstate, deck_id, window_days).await?;

    let timeline = contribution_manager::build_timeline(&snapshots, &new_by_date, granularity);
    let subscriber_summary =
        contribution_manager::summarize_subscribers(subscribers, &snapshots, activity_tracked);

    let suggestion_timeline =
        contribution_manager::get_suggestion_timeline(&appstate, deck_id, window_days, granularity)
            .await?;
    let rationales = contribution_manager::get_rationale_breakdown(&appstate, deck_id).await?;
    let turnaround_buckets =
        contribution_manager::get_review_turnaround_buckets(&appstate, deck_id).await?;
    let turnaround_total: i64 = turnaround_buckets.iter().map(|b| b.count).sum();
    let subdeck_backlog = contribution_manager::get_pending_by_subdeck(&appstate, deck_id).await?;
    let community_health = contribution_manager::get_community_health(&appstate, deck_id).await?;

    let breakdown_visible = contribution_manager::breakdown_visible(distinct_contributors);

    let digest = contribution_manager::digest_sentences(&contribution_manager::DigestInput {
        people_this_month: month_impact.people,
        accepted_this_month: month_impact.accepted,
        all_time_contributors: distinct_contributors,
        subscribers,
        median_review_hours: speed.median_hours,
        pending: speed.pending,
        oldest_pending_days: speed.oldest_pending_days,
    });

    let mut context = tera::Context::new();
    context.insert("user", &user);
    context.insert("deck_hash", &deck_hash);
    context.insert("deck_name", &deck_name);
    context.insert("stats_enabled", &stats_enabled);
    context.insert("active_tab", "contributions");
    context.insert("digest", &digest);
    context.insert("funnel", &funnel);
    context.insert("speed", &speed);
    context.insert("team", &team);
    context.insert("contributors", &contributors);
    context.insert("top_contributors", &top_contributors);
    context.insert("distinct_contributors", &distinct_contributors);
    context.insert("contributors_cap", &contribution_manager::MAX_CONTRIBUTORS);
    context.insert("breakdown_visible", &breakdown_visible);
    context.insert(
        "tracking_since",
        &tracking_since.as_ref().map(ToString::to_string),
    );
    context.insert("activity_tracked", &activity_tracked);
    context.insert("timeline", &timeline);
    context.insert("subscriber_summary", &subscriber_summary);
    context.insert("rationales", &rationales);
    context.insert("turnaround_buckets", &turnaround_buckets);
    context.insert("turnaround_total", &turnaround_total);
    context.insert("subdeck_backlog", &subdeck_backlog);
    context.insert("community_health", &community_health);
    context.insert("suggestion_timeline", &suggestion_timeline);
    context.insert(
        "bucket",
        &match granularity {
            contribution_manager::Granularity::Daily => "daily",
            contribution_manager::Granularity::Weekly => "weekly",
        },
    );
    context.insert("active_filter", &active_filter);
    context.insert("window_days", &window_days);
    // Chart inputs, kept flat so the page needs no templating gymnastics.
    context.insert(
        "chart_labels",
        &timeline.iter().map(|b| b.label.clone()).collect::<Vec<_>>(),
    );
    context.insert(
        "chart_total",
        &timeline.iter().map(|b| b.total).collect::<Vec<_>>(),
    );
    context.insert(
        "chart_active_30d",
        &timeline.iter().map(|b| b.active_30d).collect::<Vec<_>>(),
    );
    context.insert(
        "chart_active_90d",
        &timeline.iter().map(|b| b.active_90d).collect::<Vec<_>>(),
    );
    context.insert(
        "chart_new",
        &timeline.iter().map(|b| b.new).collect::<Vec<_>>(),
    );
    context.insert(
        "chart_unsubscribed",
        &timeline.iter().map(|b| b.unsubscribed).collect::<Vec<_>>(),
    );
    context.insert(
        "sugg_labels",
        &suggestion_timeline
            .iter()
            .map(|b| b.label.clone())
            .collect::<Vec<_>>(),
    );
    context.insert(
        "sugg_submitted",
        &suggestion_timeline
            .iter()
            .map(|b| b.submitted)
            .collect::<Vec<_>>(),
    );
    context.insert(
        "sugg_accepted",
        &suggestion_timeline
            .iter()
            .map(|b| b.accepted)
            .collect::<Vec<_>>(),
    );
    context.insert(
        "sugg_declined",
        &suggestion_timeline
            .iter()
            .map(|b| b.declined)
            .collect::<Vec<_>>(),
    );

    let rendered_template = appstate.tera.render("contributions.html", &context)?;
    Ok(Html(rendered_template))
}

/// Which slice of a contributor's suggestions to list.
#[derive(Deserialize)]
pub(crate) struct ContributorSuggestionsQuery {
    state: Option<String>,
}

pub(crate) async fn show_contributor_suggestions(
    State(appstate): State<Arc<AppState>>,
    Path((deck_hash, username)): Path<(String, String)>,
    Query(params): Query<ContributorSuggestionsQuery>,
    user: Option<User>,
) -> Result<impl IntoResponse, Error> {
    let user = check_login(user)?;

    let Some((deck_id, deck_name, stats_enabled)) =
        contributions_deck(&appstate, &deck_hash).await?
    else {
        return error_page(&appstate, "Deck not found");
    };

    if !access_check(&appstate, deck_id, &user).await? {
        return Err(error::Error::Unauthorized);
    }

    // The same k-anonymity floor the listing applies.
    if !contribution_manager::breakdown_visible(
        contribution_manager::count_contributors(&appstate, deck_id).await?,
    ) {
        return Err(error::Error::Unauthorized);
    }

    let state = match params.state.as_deref() {
        Some("archived") => "archived",
        _ => "open",
    };

    let summary =
        contribution_manager::get_contributor_summary(&appstate, deck_id, &username).await?;
    let suggestions =
        contribution_manager::get_contributor_suggestions(&appstate, deck_id, &username, state)
            .await?;

    let mut context = tera::Context::new();
    context.insert("user", &user);
    context.insert("deck_hash", &deck_hash);
    context.insert("deck_name", &deck_name);
    context.insert("stats_enabled", &stats_enabled);
    context.insert("active_tab", "contributions");
    context.insert("username", &username);
    context.insert(
        "username_url",
        &contribution_manager::url_path_segment(&username),
    );
    context.insert("state", &state);
    context.insert("summary", &summary);
    context.insert("suggestions", &suggestions);
    context.insert(
        "suggestions_truncated",
        &(i64::try_from(suggestions.len()).unwrap_or(i64::MAX)
            == contribution_manager::MAX_CONTRIBUTOR_SUGGESTIONS),
    );

    let rendered_template = appstate
        .tera
        .render("contributions_contributor.html", &context)?;
    Ok(Html(rendered_template))
}
