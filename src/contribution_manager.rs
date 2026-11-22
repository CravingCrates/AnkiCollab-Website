//! Contribution ("HUMANS") statistics for a deck.

use crate::{database, stats_manager, Return};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use time::{Date, Duration};

/// Per-contributor rows are withheld below this many distinct contributors (k-anonymity).
pub const MIN_CONTRIBUTORS_FOR_BREAKDOWN: i64 = 5;

/// Reviewed-suggestion floor for an acceptance rate. Mirrors the backend constant; change together.
pub const MIN_REVIEWED_FOR_RATE: i64 = 5;

/// Row caps. No paging: the pages say so rather than pretending to show everything.
pub const MAX_CONTRIBUTORS: i64 = 100;
pub const MAX_CONTRIBUTOR_SUGGESTIONS: i64 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Granularity {
    Daily,
    Weekly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotPoint {
    pub on: Date,
    pub total: i64,
    pub active_30d: i64,
    pub active_90d: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimelineBucket {
    pub label: String,
    pub total: i64,
    pub active_30d: i64,
    pub active_90d: i64,
    pub new: i64,
    /// Derived from the drop in `total`.
    pub unsubscribed: i64,
    pub net: i64,
}

#[derive(Debug, Clone, Default)]
pub struct DigestInput {
    pub people_this_month: i64,
    pub accepted_this_month: i64,
    pub all_time_contributors: i64,
    pub subscribers: i64,
    pub median_review_hours: Option<f64>,
    pub pending: i64,
    pub oldest_pending_days: Option<i64>,
}

#[must_use]
pub fn breakdown_visible(distinct_contributors: i64) -> bool {
    distinct_contributors >= MIN_CONTRIBUTORS_FOR_BREAKDOWN
}

/// Unsubscribes are derived, not stored: a departure only shows as a drop in `total`, so
/// `unsubscribed = new - net`, clamped at zero for rows with a NULL `created_at`.
#[must_use]
pub fn build_timeline(
    snapshots: &[SnapshotPoint],
    new_by_date: &BTreeMap<Date, i64>,
    granularity: Granularity,
) -> Vec<TimelineBucket> {
    let mut buckets: Vec<TimelineBucket> = Vec::new();
    let mut previous_total: Option<i64> = None;

    for snapshot in snapshots {
        // The first day has no previous total, so all three flows are zero. Deriving
        // `unsubscribed = new - net` with `net` forced to 0 would invent departures for arrivals.
        let is_baseline = previous_total.is_none();
        let net = if is_baseline {
            0
        } else {
            previous_total.map_or(0, |previous| snapshot.total - previous)
        };
        let new = if is_baseline {
            0
        } else {
            new_by_date.get(&snapshot.on).copied().unwrap_or(0)
        };
        let unsubscribed = if is_baseline { 0 } else { (new - net).max(0) };
        let label = bucket_label(snapshot.on, granularity);

        match buckets.last_mut() {
            Some(last) if last.label == label => {
                last.total = snapshot.total;
                last.active_30d = snapshot.active_30d;
                last.active_90d = snapshot.active_90d;
                last.new += new;
                last.unsubscribed += unsubscribed;
                last.net += net;
            }
            _ => buckets.push(TimelineBucket {
                label,
                total: snapshot.total,
                active_30d: snapshot.active_30d,
                active_90d: snapshot.active_90d,
                new,
                unsubscribed,
                net,
            }),
        }

        previous_total = Some(snapshot.total);
    }

    buckets
}

fn bucket_label(on: Date, granularity: Granularity) -> String {
    match granularity {
        Granularity::Daily => on.to_string(),
        Granularity::Weekly => {
            let (year, week, _) = on.to_iso_week_date();
            format!("{year}-W{week:02}")
        }
    }
}

/// English has one inflection, and this is all the pluralisation the digest needs.
#[must_use]
fn plural(count: i64) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

#[must_use]
pub fn digest_sentences(input: &DigestInput) -> [String; 3] {
    let impact = if input.people_this_month <= 0 || input.accepted_this_month <= 0 {
        "No suggestions have been accepted this month yet — sharing the deck is what brings the first ones in.".to_string()
    } else {
        let people = input.people_this_month;
        let accepted = input.accepted_this_month;
        format!(
            "This month {people} contributor{} improved this deck, with {accepted} suggestion{} accepted.",
            plural(people),
            plural(accepted)
        )
    };

    let gratitude = if input.all_time_contributors <= 0 {
        "No contributions yet. Be the first — every accepted fix helps every learner.".to_string()
    } else {
        let contributors = input.all_time_contributors;
        let learners = input.subscribers;
        let verb = if contributors == 1 { "has" } else { "have" };
        format!(
            "All time, {contributors} contributor{} {verb} shaped this deck for {learners} learner{}.",
            plural(contributors),
            plural(learners)
        )
    };

    let speed = match (input.oldest_pending_days, input.median_review_hours) {
        (Some(days), _) if days > 30 => {
            let pending = input.pending;
            format!(
                "{pending} suggestion{} waiting for review, the oldest for {days} day{}.",
                plural(pending),
                plural(days)
            )
        }
        (_, Some(hours)) => format!("Suggestions are usually reviewed within {hours:.0} hours."),
        _ => "Review speed will appear here once suggestions have been reviewed.".to_string(),
    };

    [impact, gratitude, speed]
}

/// Usernames are only trimmed and lower-cased on write, so encoding is what keeps a name
/// containing `/`, `?`, `#` or `%` from breaking a drilldown link.
#[must_use]
pub fn url_path_segment(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";

    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        let character = char::from(*byte);
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '.' | '_' | '~') {
            encoded.push(character);
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(*byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(*byte & 0x0F)]));
        }
    }
    encoded
}

#[must_use]
pub fn deck_display_name_from_path(full_path: &str) -> String {
    full_path
        .rsplit("::")
        .next()
        .map(str::trim)
        .filter(|leaf| !leaf.is_empty())
        .unwrap_or("[Unnamed Deck]")
        .to_string()
}

/// The four buckets partition `submitted`: `submitted = accepted + declined + pending + unrecorded`.
/// `pending` excludes any verdict; `unrecorded` is the remainder, because a per-field or per-tag
/// review writes no `notifications` row.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Funnel {
    pub submitted: i64,
    pub accepted: i64,
    pub declined: i64,
    pub pending: i64,
    pub unrecorded: i64,
    /// `accepted / (accepted + declined)`, withheld below [`MIN_REVIEWED_FOR_RATE`].
    pub acceptance_ratio: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ReviewSpeed {
    pub median_hours: Option<f64>,
    pub p90_hours: Option<f64>,
    pub pending: i64,
    pub oldest_pending_days: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamRow {
    /// `None` for reviews recorded before `notifications.actor_user_id` existed.
    pub name: Option<String>,
    pub reviewed: i64,
    pub reviewed_30d: i64,
    pub accepted: i64,
    pub declined: i64,
    pub share_pct: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MilestoneTier {
    pub threshold: i64,
    pub name: &'static str,
}

pub const MILESTONES: [MilestoneTier; 8] = [
    MilestoneTier {
        threshold: 1,
        name: "Contributor",
    },
    MilestoneTier {
        threshold: 5,
        name: "Regular Contributor",
    },
    MilestoneTier {
        threshold: 10,
        name: "Trusted Contributor",
    },
    MilestoneTier {
        threshold: 25,
        name: "Core Contributor",
    },
    MilestoneTier {
        threshold: 50,
        name: "Deck Steward",
    },
    MilestoneTier {
        threshold: 100,
        name: "Community Steward",
    },
    MilestoneTier {
        threshold: 250,
        name: "Community Pillar",
    },
    MilestoneTier {
        threshold: 500,
        name: "Cornerstone",
    },
];

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct MilestoneSummary {
    pub current_name: Option<&'static str>,
    pub next_name: Option<&'static str>,
    pub remaining_to_next: Option<i64>,
    pub progress_pct: i64,
}

#[must_use]
pub fn milestone_for_accepted(accepted: i64) -> MilestoneSummary {
    let accepted = accepted.max(0);
    let current = MILESTONES
        .iter()
        .rev()
        .find(|tier| tier.threshold <= accepted)
        .copied();
    let next = MILESTONES
        .iter()
        .find(|tier| tier.threshold > accepted)
        .copied();

    let (remaining_to_next, progress_pct) = match (current, next) {
        (Some(cur), Some(nxt)) => {
            let span = (nxt.threshold - cur.threshold).max(1);
            let pct = (((accepted - cur.threshold) * 100) / span).clamp(0, 100);
            (Some(nxt.threshold - accepted), pct)
        }
        (None, Some(nxt)) => (Some(nxt.threshold - accepted), 0),
        (Some(_), None) => (None, 100),
        (None, None) => (None, 0),
    };

    MilestoneSummary {
        current_name: current.map(|tier| tier.name),
        next_name: next.map(|tier| tier.name),
        remaining_to_next,
        progress_pct,
    }
}

#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct SubscriberSummary {
    pub total: i64,
    pub net_30d: Option<i64>,
    pub active_30d: Option<i64>,
    pub active_90d: Option<i64>,
    pub stickiness_30d_pct: Option<f64>,
    pub stickiness_90d_pct: Option<f64>,
}

#[must_use]
#[allow(clippy::cast_precision_loss)]
fn gated_acceptance_ratio(accepted: i64, reviewed: i64) -> Option<f64> {
    if reviewed < MIN_REVIEWED_FOR_RATE {
        None
    } else {
        Some((accepted as f64 / reviewed as f64).clamp(0.0, 1.0))
    }
}

#[must_use]
pub fn summarize_subscribers(
    total: i64,
    snapshots: &[SnapshotPoint],
    activity_tracked: bool,
) -> SubscriberSummary {
    let latest = snapshots.last().copied();

    let net_30d = if snapshots.len() >= 2 {
        let first = snapshots[0];
        let last = snapshots[snapshots.len() - 1];
        if (last.on - first.on).whole_days() >= 7 {
            let target = last.on - Duration::days(30);
            snapshots[..snapshots.len() - 1]
                .iter()
                .min_by_key(|point| ((point.on - target).whole_days().abs(), point.on))
                .map(|baseline| last.total - baseline.total)
        } else {
            None
        }
    } else {
        None
    };

    let (active_30d, active_90d, stickiness_30d_pct, stickiness_90d_pct) = if activity_tracked {
        match latest {
            Some(snap) if total > 0 => (
                Some(snap.active_30d),
                Some(snap.active_90d),
                Some(stats_manager::coverage_percentage(snap.active_30d, total)),
                Some(stats_manager::coverage_percentage(snap.active_90d, total)),
            ),
            Some(snap) => (Some(snap.active_30d), Some(snap.active_90d), None, None),
            None => (None, None, None, None),
        }
    } else {
        (None, None, None, None)
    };

    SubscriberSummary {
        total,
        net_30d,
        active_30d,
        active_90d,
        stickiness_30d_pct,
        stickiness_90d_pct,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SuggestionActivityBucket {
    pub label: String,
    pub submitted: i64,
    pub accepted: i64,
    pub declined: i64,
}

fn suggestion_bucket_key(on: Date, days: i64, granularity: Granularity) -> String {
    if days > 365 {
        format!("{}-{:02}", on.year(), u8::from(on.month()))
    } else if days > 90 || matches!(granularity, Granularity::Weekly) {
        bucket_label(on, Granularity::Weekly)
    } else {
        bucket_label(on, Granularity::Daily)
    }
}

#[must_use]
pub fn build_suggestion_timeline(
    submitted_by_date: &BTreeMap<Date, i64>,
    verdicts_by_date: &BTreeMap<Date, (i64, i64)>,
    days: i64,
    granularity: Granularity,
) -> Vec<SuggestionActivityBucket> {
    let mut grouped: BTreeMap<String, (i64, i64, i64)> = BTreeMap::new();

    for (&on, &count) in submitted_by_date {
        let key = suggestion_bucket_key(on, days, granularity);
        grouped.entry(key).or_insert((0, 0, 0)).0 += count;
    }

    for (&on, &(accepted, declined)) in verdicts_by_date {
        let key = suggestion_bucket_key(on, days, granularity);
        let entry = grouped.entry(key).or_insert((0, 0, 0));
        entry.1 += accepted;
        entry.2 += declined;
    }

    grouped
        .into_iter()
        .map(
            |(label, (submitted, accepted, declined))| SuggestionActivityBucket {
                label,
                submitted,
                accepted,
                declined,
            },
        )
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct RationaleRow {
    pub rationale: String,
    pub submitted: i64,
    pub accepted: i64,
    pub declined: i64,
    pub pending: i64,
    pub share_pct: f64,
    pub acceptance_ratio: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TurnaroundBucket {
    pub label: String,
    pub count: i64,
    pub share_pct: f64,
    pub tone: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SubdeckBacklogRow {
    pub path: String,
    pub prefix: String,
    pub leaf: String,
    pub pending: i64,
    pub oldest_days: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct CommunityHealth {
    pub active_contributors_30d: i64,
    pub returning_contributors: i64,
    pub notes_improved: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContributorRationale {
    pub rationale: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContributorSummary {
    pub submitted: i64,
    pub accepted: i64,
    pub declined: i64,
    pub pending: i64,
    pub acceptance_ratio: Option<f64>,
    pub first_contribution: Option<String>,
    pub last_contribution: Option<String>,
    pub milestone: MilestoneSummary,
    pub top_rationales: Vec<ContributorRationale>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContributorRow {
    pub username: String,
    pub username_url: String,
    pub submitted: i64,
    pub accepted: i64,
    pub declined: i64,
    pub pending: i64,
    pub last_contribution: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SuggestionRow {
    pub commit_id: i32,
    pub info: Option<String>,
    pub submitted_at: String,
    pub rationale: String,
    /// `true` while any part of the commit is unreviewed.
    pub open: bool,
    pub accepted: bool,
    pub declined: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MonthImpact {
    pub people: i64,
    pub accepted: i64,
}

const SUBTREE_CTE: &str = "
    WITH RECURSIVE tree AS (
        SELECT id FROM anki.decks WHERE id = $1 AND deleted_at IS NULL
        UNION ALL
        SELECT d.id FROM anki.decks d JOIN tree t ON d.parent = t.id WHERE d.deleted_at IS NULL
    )";

/// The "still unreviewed" predicate, copied from `suggestion_manager::next_review`.
const COMMIT_IS_PENDING: &str = "
        (EXISTS (SELECT 1 FROM anki.fields f
                  WHERE f.commit = c.commit_id AND f.reviewed = false)
      OR EXISTS (SELECT 1 FROM anki.tags t
                  WHERE t.commit = c.commit_id AND t.reviewed = false)
      OR EXISTS (SELECT 1 FROM anki.card_deletion_suggestions cds
                  WHERE cds.commit = c.commit_id)
      OR EXISTS (SELECT 1 FROM anki.note_move_suggestions nms
                  WHERE nms.commit = c.commit_id))";

pub async fn get_funnel(db_state: &database::AppState, deck_id: i64) -> Return<Funnel> {
    let client = database::client(db_state).await?;
    let query = format!(
        "{SUBTREE_CTE},
        sugg AS (
            SELECT c.commit_id, {COMMIT_IS_PENDING} AS pending
              FROM anki.commits c
             WHERE c.deck IN (SELECT id FROM tree)
        ),
        verdicts AS (
            SELECT n.commit_id,
                   bool_or(n.status = 'approved') AS approved,
                   bool_or(n.status = 'denied')   AS denied
              FROM anki.notifications n
             WHERE n.commit_id IN (SELECT commit_id FROM sugg)
             GROUP BY n.commit_id
        ),
        counted AS (
            SELECT COUNT(*)::bigint AS submitted,
                   COUNT(*) FILTER (WHERE COALESCE(v.approved, false))::bigint AS accepted,
                   COUNT(*) FILTER (WHERE COALESCE(v.denied, false)
                                      AND NOT COALESCE(v.approved, false))::bigint AS declined,
                   COUNT(*) FILTER (WHERE s.pending
                                      AND NOT COALESCE(v.approved, false)
                                      AND NOT COALESCE(v.denied, false))::bigint AS pending,
                   COUNT(*) FILTER (WHERE NOT s.pending
                                      AND NOT COALESCE(v.approved, false)
                                      AND NOT COALESCE(v.denied, false))::bigint AS unrecorded
              FROM sugg s
              LEFT JOIN verdicts v ON v.commit_id = s.commit_id
        )
        SELECT submitted, accepted, declined, pending, unrecorded,
               CASE WHEN accepted + declined >= $2::bigint
                    THEN accepted::float8 / (accepted + declined)::float8
               END AS acceptance_ratio
          FROM counted"
    );

    let row = client
        .query_one(&query, &[&deck_id, &MIN_REVIEWED_FOR_RATE])
        .await?;
    Ok(Funnel {
        submitted: row.get(0),
        accepted: row.get(1),
        declined: row.get(2),
        pending: row.get(3),
        unrecorded: row.get(4),
        acceptance_ratio: row.get(5),
    })
}

pub async fn get_timeline(
    db_state: &database::AppState,
    deck_id: i64,
    days: i64,
) -> Return<(Vec<SnapshotPoint>, BTreeMap<Date, i64>)> {
    let client = database::client(db_state).await?;

    let snapshot_rows = client
        .query(
            "SELECT captured_on, subscriber_count::bigint, active_30d::bigint, active_90d::bigint
               FROM anki.deck_stats_daily
              WHERE deck_id = $1
                AND captured_on >= CURRENT_DATE - ($2::bigint * INTERVAL '1 day')
              ORDER BY captured_on",
            &[&deck_id, &days],
        )
        .await?;
    let snapshots = snapshot_rows
        .iter()
        .map(|row| SnapshotPoint {
            on: row.get(0),
            total: row.get(1),
            active_30d: row.get(2),
            active_90d: row.get(3),
        })
        .collect();

    let new_query = format!(
        "{SUBTREE_CTE}
        SELECT s.created_at::date AS on_date, COUNT(*)::bigint
          FROM anki.subscriptions s
         WHERE s.deck_id IN (SELECT id FROM tree)
           AND s.created_at IS NOT NULL
           AND s.created_at::date >= CURRENT_DATE - ($2::bigint * INTERVAL '1 day')
         GROUP BY 1"
    );
    let new_rows = client.query(&new_query, &[&deck_id, &days]).await?;
    let new_by_date: BTreeMap<Date, i64> = new_rows
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();

    Ok((snapshots, new_by_date))
}

pub async fn get_review_speed(db_state: &database::AppState, deck_id: i64) -> Return<ReviewSpeed> {
    let client = database::client(db_state).await?;

    let speed_query = format!(
        "{SUBTREE_CTE}
        SELECT percentile_cont(0.5) WITHIN GROUP (
                   ORDER BY EXTRACT(EPOCH FROM (n.created_at - c.timestamp)) / 3600.0),
               percentile_cont(0.9) WITHIN GROUP (
                   ORDER BY EXTRACT(EPOCH FROM (n.created_at - c.timestamp)) / 3600.0)
          FROM anki.notifications n
          JOIN anki.commits c ON c.commit_id = n.commit_id
         WHERE n.deck_id IN (SELECT id FROM tree)
           AND c.timestamp IS NOT NULL
           AND n.created_at >= c.timestamp
           AND n.created_at >= now() - INTERVAL '90 days'"
    );
    let row = client.query_one(&speed_query, &[&deck_id]).await?;

    let backlog_query = format!(
        "{SUBTREE_CTE},
        pending AS (
            SELECT c.timestamp
              FROM anki.commits c
             WHERE c.deck IN (SELECT id FROM tree)
               AND {COMMIT_IS_PENDING}
        )
        SELECT COUNT(*)::bigint,
               -- FLOOR, not a ::bigint cast: a cast rounds the age up by a day.
               -- Matches how every other age on the page truncates.
               FLOOR(EXTRACT(EPOCH FROM (now() - MIN(timestamp))) / 86400)::bigint
          FROM pending"
    );
    let backlog = client.query_one(&backlog_query, &[&deck_id]).await?;

    Ok(ReviewSpeed {
        median_hours: row.get(0),
        p90_hours: row.get(1),
        pending: backlog.get(0),
        oldest_pending_days: backlog.get(1),
    })
}

pub async fn get_team(db_state: &database::AppState, deck_id: i64) -> Return<Vec<TeamRow>> {
    let client = database::client(db_state).await?;
    let query = format!(
        "{SUBTREE_CTE}
        SELECT u.username,
               COUNT(*)::bigint AS reviewed,
               COUNT(*) FILTER (WHERE n.created_at >= now() - INTERVAL '30 days')::bigint AS reviewed_30d,
               COUNT(*) FILTER (WHERE n.status = 'approved')::bigint AS accepted,
               COUNT(*) FILTER (WHERE n.status = 'denied')::bigint AS declined
          FROM anki.notifications n
          LEFT JOIN public.users u ON u.id = n.actor_user_id
         WHERE n.deck_id IN (SELECT id FROM tree)
         GROUP BY u.username
         ORDER BY reviewed DESC, u.username"
    );

    let rows = client.query(&query, &[&deck_id]).await?;
    let total_reviewed: i64 = rows.iter().map(|row| row.get::<_, i64>(1)).sum();
    Ok(rows
        .iter()
        .map(|row| {
            let reviewed: i64 = row.get(1);
            let share_pct = stats_manager::coverage_percentage(reviewed, total_reviewed);
            TeamRow {
                name: row.get(0),
                reviewed,
                reviewed_30d: row.get(2),
                accepted: row.get(3),
                declined: row.get(4),
                share_pct,
            }
        })
        .collect())
}

/// shortcut: at most 100 rows and no paging; the page says so. Upgrade if a deck exceeds it.
pub async fn get_contributors(
    db_state: &database::AppState,
    deck_id: i64,
) -> Return<Vec<ContributorRow>> {
    let client = database::client(db_state).await?;
    let query = format!(
        "{SUBTREE_CTE},
        sugg AS (
            SELECT c.commit_id, c.user_id, c.timestamp, {COMMIT_IS_PENDING} AS pending
              FROM anki.commits c
             WHERE c.deck IN (SELECT id FROM tree)
               AND c.user_id IS NOT NULL
        ),
        verdicts AS (
            SELECT n.commit_id,
                   bool_or(n.status = 'approved') AS approved,
                   bool_or(n.status = 'denied')   AS denied
              FROM anki.notifications n
             WHERE n.commit_id IN (SELECT commit_id FROM sugg)
             GROUP BY n.commit_id
        )
        SELECT COALESCE(u.username, 'Unknown') AS username,
               COUNT(*)::bigint AS submitted,
               COUNT(*) FILTER (WHERE COALESCE(v.approved, false))::bigint AS accepted,
               COUNT(*) FILTER (WHERE COALESCE(v.denied, false)
                                  AND NOT COALESCE(v.approved, false))::bigint AS declined,
               COUNT(*) FILTER (WHERE s.pending)::bigint AS pending,
               TO_CHAR(MAX(s.timestamp), 'YYYY-MM-DD') AS last_contribution
          FROM sugg s
          LEFT JOIN verdicts v ON v.commit_id = s.commit_id
          LEFT JOIN public.users u ON u.id = s.user_id
         GROUP BY s.user_id, u.username
         -- user_id last, so equal counts keep a stable order between page loads
         ORDER BY accepted DESC, submitted DESC, s.user_id
         LIMIT {MAX_CONTRIBUTORS}"
    );

    let rows = client.query(&query, &[&deck_id]).await?;
    Ok(rows
        .iter()
        .map(|row| {
            let username: String = row.get(0);
            ContributorRow {
                username_url: url_path_segment(&username),
                username,
                submitted: row.get(1),
                accepted: row.get(2),
                declined: row.get(3),
                pending: row.get(4),
                last_contribution: row.get(5),
            }
        })
        .collect())
}

pub async fn get_top_contributors(
    db_state: &database::AppState,
    deck_id: i64,
    days: i64,
) -> Return<Vec<ContributorRow>> {
    let client = database::client(db_state).await?;
    let query = format!(
        "{SUBTREE_CTE},
        approved AS (
            SELECT n.commit_id, c.user_id, n.created_at
              FROM anki.notifications n
              JOIN anki.commits c ON c.commit_id = n.commit_id
             WHERE n.deck_id IN (SELECT id FROM tree)
               AND n.status = 'approved'
               AND n.created_at >= now() - ($2::bigint * INTERVAL '1 day')
               AND c.user_id IS NOT NULL
        )
        SELECT COALESCE(u.username, 'Unknown') AS username,
               COUNT(DISTINCT a.commit_id)::bigint AS accepted,
               COUNT(DISTINCT a.commit_id)::bigint AS submitted,
               0::bigint AS declined,
               0::bigint AS pending,
               TO_CHAR(MAX(a.created_at), 'YYYY-MM-DD') AS last_contribution
          FROM approved a
          LEFT JOIN public.users u ON u.id = a.user_id
         GROUP BY a.user_id, u.username
         ORDER BY accepted DESC, u.username
         LIMIT 10"
    );

    let rows = client.query(&query, &[&deck_id, &days]).await?;
    Ok(rows
        .iter()
        .map(|row| {
            let username: String = row.get(0);
            ContributorRow {
                username_url: url_path_segment(&username),
                username,
                submitted: row.get(2),
                accepted: row.get(1),
                declined: row.get(3),
                pending: row.get(4),
                last_contribution: row.get(5),
            }
        })
        .collect())
}

pub async fn get_month_impact(db_state: &database::AppState, deck_id: i64) -> Return<MonthImpact> {
    let client = database::client(db_state).await?;
    let query = format!(
        "{SUBTREE_CTE}
        SELECT COUNT(DISTINCT c.user_id)::bigint,
               COUNT(DISTINCT n.commit_id)::bigint
          FROM anki.notifications n
          JOIN anki.commits c ON c.commit_id = n.commit_id
         WHERE n.deck_id IN (SELECT id FROM tree)
           AND n.status = 'approved'
           AND n.created_at >= date_trunc('month', now())"
    );

    let row = client.query_one(&query, &[&deck_id]).await?;
    Ok(MonthImpact {
        people: row.get(0),
        accepted: row.get(1),
    })
}

pub async fn count_contributors(db_state: &database::AppState, deck_id: i64) -> Return<i64> {
    let client = database::client(db_state).await?;
    let query = format!(
        "{SUBTREE_CTE}
        SELECT COUNT(DISTINCT c.user_id)::bigint
          FROM anki.commits c
         WHERE c.deck IN (SELECT id FROM tree)
           AND c.user_id IS NOT NULL"
    );
    let row = client.query_one(&query, &[&deck_id]).await?;
    Ok(row.get(0))
}

/// Scoped to the deck subtree so it agrees with the snapshot-based net figure, and so differs
/// from the direct count `/decks` shows.
pub async fn count_subscribers(db_state: &database::AppState, deck_id: i64) -> Return<i64> {
    let client = database::client(db_state).await?;
    let query = format!(
        "{SUBTREE_CTE}
        SELECT COUNT(*)::bigint FROM anki.subscriptions WHERE deck_id IN (SELECT id FROM tree)"
    );
    let row = client.query_one(&query, &[&deck_id]).await?;
    Ok(row.get(0))
}

/// Whether any subscriber of this deck's subtree has synced. Until a client build sends its
/// bearer token on `/pullChanges`, every `last_synced_at` is NULL and the snapshot's active
/// counters are all zero — without this flag the page would claim "0 active subscribers".
pub async fn activity_tracked(db_state: &database::AppState, deck_id: i64) -> Return<bool> {
    let client = database::client(db_state).await?;
    let query = format!(
        "{SUBTREE_CTE}
        SELECT EXISTS (
            SELECT 1 FROM anki.subscriptions s
             WHERE s.deck_id IN (SELECT id FROM tree)
               AND s.last_synced_at IS NOT NULL
        )"
    );
    let row = client.query_one(&query, &[&deck_id]).await?;
    Ok(row.get(0))
}

/// The earliest snapshot date, so the page can say "tracking since <date>" rather than pretend
/// history starts at zero.
pub async fn tracking_since(db_state: &database::AppState, deck_id: i64) -> Return<Option<Date>> {
    let client = database::client(db_state).await?;
    let row = client
        .query_one(
            "SELECT MIN(captured_on) FROM anki.deck_stats_daily WHERE deck_id = $1",
            &[&deck_id],
        )
        .await?;
    Ok(row.get(0))
}

/// `state` is `open` (still unreviewed) or `archived` (a decision was reached — reviewed, not
/// necessarily declined, since the schema has no separate closed state).
/// shortcut: at most 200 rows, most recent first; the page says so when it hits the cap.
pub async fn get_contributor_suggestions(
    db_state: &database::AppState,
    deck_id: i64,
    username: &str,
    state: &str,
) -> Return<Vec<SuggestionRow>> {
    let client = database::client(db_state).await?;
    let query = format!(
        "{SUBTREE_CTE},
        sugg AS (
            SELECT c.commit_id, c.info, c.timestamp, c.rationale,
                   {COMMIT_IS_PENDING} AS pending
              FROM anki.commits c
              JOIN public.users u ON u.id = c.user_id
             WHERE c.deck IN (SELECT id FROM tree)
               AND u.username = $2
        ),
        verdicts AS (
            SELECT n.commit_id,
                   bool_or(n.status = 'approved') AS approved,
                   bool_or(n.status = 'denied')   AS denied
              FROM anki.notifications n
             WHERE n.commit_id IN (SELECT commit_id FROM sugg)
             GROUP BY n.commit_id
        )
        SELECT s.commit_id,
               s.info,
               TO_CHAR(s.timestamp, 'YYYY-MM-DD HH24:MI') AS submitted_at,
               s.rationale,
               s.pending,
               COALESCE(v.approved, false),
               COALESCE(v.denied, false)
          FROM sugg s
          LEFT JOIN verdicts v ON v.commit_id = s.commit_id
         WHERE ($3 = 'open' AND s.pending)
            OR ($3 = 'archived' AND NOT s.pending)
         ORDER BY s.timestamp DESC NULLS LAST
         LIMIT {MAX_CONTRIBUTOR_SUGGESTIONS}"
    );

    let rows = client.query(&query, &[&deck_id, &username, &state]).await?;
    Ok(rows
        .iter()
        .map(|row| {
            let rationale: i32 = row.get(3);
            SuggestionRow {
                commit_id: row.get(0),
                info: row.get(1),
                submitted_at: row.get(2),
                rationale: crate::commit_manager::get_string_from_rationale(rationale).to_string(),
                open: row.get(4),
                accepted: row.get(5),
                declined: row.get(6),
            }
        })
        .collect())
}

pub async fn get_suggestion_timeline(
    db_state: &database::AppState,
    deck_id: i64,
    days: i64,
    granularity: Granularity,
) -> Return<Vec<SuggestionActivityBucket>> {
    let client = database::client(db_state).await?;

    let submitted_query = format!(
        "{SUBTREE_CTE}
        SELECT c.timestamp::date AS on_date, COUNT(*)::bigint
          FROM anki.commits c
         WHERE c.deck IN (SELECT id FROM tree)
           AND c.timestamp IS NOT NULL
           AND c.timestamp >= now() - ($2::bigint * INTERVAL '1 day')
         GROUP BY 1
         ORDER BY 1"
    );
    let submitted_rows = client.query(&submitted_query, &[&deck_id, &days]).await?;
    let submitted_by_date: BTreeMap<Date, i64> = submitted_rows
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();

    let verdict_query = format!(
        "{SUBTREE_CTE}
        SELECT n.created_at::date AS on_date,
               COUNT(DISTINCT n.commit_id) FILTER (WHERE n.status = 'approved')::bigint,
               COUNT(DISTINCT n.commit_id) FILTER (WHERE n.status = 'denied')::bigint
          FROM anki.notifications n
         WHERE n.deck_id IN (SELECT id FROM tree)
           AND n.created_at >= now() - ($2::bigint * INTERVAL '1 day')
         GROUP BY 1
         ORDER BY 1"
    );
    let verdict_rows = client.query(&verdict_query, &[&deck_id, &days]).await?;
    let verdicts_by_date: BTreeMap<Date, (i64, i64)> = verdict_rows
        .iter()
        .map(|row| (row.get(0), (row.get(1), row.get(2))))
        .collect();

    Ok(build_suggestion_timeline(
        &submitted_by_date,
        &verdicts_by_date,
        days,
        granularity,
    ))
}

pub async fn get_rationale_breakdown(
    db_state: &database::AppState,
    deck_id: i64,
) -> Return<Vec<RationaleRow>> {
    let client = database::client(db_state).await?;
    let query = format!(
        "{SUBTREE_CTE},
        sugg AS (
            SELECT c.commit_id, COALESCE(c.rationale, 0) AS rationale, {COMMIT_IS_PENDING} AS pending
              FROM anki.commits c
             WHERE c.deck IN (SELECT id FROM tree)
        ),
        verdicts AS (
            SELECT n.commit_id,
                   bool_or(n.status = 'approved') AS approved,
                   bool_or(n.status = 'denied')   AS denied
              FROM anki.notifications n
             WHERE n.commit_id IN (SELECT commit_id FROM sugg)
             GROUP BY n.commit_id
        )
        SELECT s.rationale,
               COUNT(*)::bigint AS submitted,
               COUNT(*) FILTER (WHERE COALESCE(v.approved, false))::bigint AS accepted,
               COUNT(*) FILTER (WHERE COALESCE(v.denied, false)
                                  AND NOT COALESCE(v.approved, false))::bigint AS declined,
               COUNT(*) FILTER (WHERE s.pending
                                  AND NOT COALESCE(v.approved, false)
                                  AND NOT COALESCE(v.denied, false))::bigint AS pending
          FROM sugg s
          LEFT JOIN verdicts v ON v.commit_id = s.commit_id
         GROUP BY s.rationale"
    );
    let rows = client.query(&query, &[&deck_id]).await?;

    let mut merged: HashMap<&'static str, (i64, i64, i64, i64)> = HashMap::new();
    for row in &rows {
        let code: i32 = row.get(0);
        let label = crate::commit_manager::get_string_from_rationale(code);
        let entry = merged.entry(label).or_insert((0, 0, 0, 0));
        entry.0 += row.get::<_, i64>(1);
        entry.1 += row.get::<_, i64>(2);
        entry.2 += row.get::<_, i64>(3);
        entry.3 += row.get::<_, i64>(4);
    }

    let total_submitted: i64 = merged.values().map(|v| v.0).sum();
    let mut results: Vec<RationaleRow> = merged
        .into_iter()
        .map(|(label, (submitted, accepted, declined, pending))| {
            let reviewed = accepted + declined;
            let acceptance_ratio = gated_acceptance_ratio(accepted, reviewed);
            let share_pct = stats_manager::coverage_percentage(submitted, total_submitted);
            RationaleRow {
                rationale: label.to_string(),
                submitted,
                accepted,
                declined,
                pending,
                share_pct,
                acceptance_ratio,
            }
        })
        .collect();

    results.sort_by(|a, b| {
        b.submitted
            .cmp(&a.submitted)
            .then_with(|| a.rationale.cmp(&b.rationale))
    });
    Ok(results)
}

pub async fn get_review_turnaround_buckets(
    db_state: &database::AppState,
    deck_id: i64,
) -> Return<Vec<TurnaroundBucket>> {
    let client = database::client(db_state).await?;
    let query = format!(
        "{SUBTREE_CTE},
        waits AS (
            SELECT EXTRACT(EPOCH FROM (n.created_at - c.timestamp)) / 3600.0 AS hours
              FROM anki.notifications n
              JOIN anki.commits c ON c.commit_id = n.commit_id
             WHERE n.deck_id IN (SELECT id FROM tree)
               AND c.timestamp IS NOT NULL
               AND n.created_at >= c.timestamp
               AND n.created_at >= now() - INTERVAL '90 days'
        )
        SELECT COUNT(*) FILTER (WHERE hours < 24)::bigint,
               COUNT(*) FILTER (WHERE hours >= 24 AND hours < 72)::bigint,
               COUNT(*) FILTER (WHERE hours >= 72 AND hours < 168)::bigint,
               COUNT(*) FILTER (WHERE hours >= 168 AND hours < 720)::bigint,
               COUNT(*) FILTER (WHERE hours >= 720)::bigint
          FROM waits"
    );
    let row = client.query_one(&query, &[&deck_id]).await?;

    let counts = [
        ("< 24 hours", "success", row.get::<_, i64>(0)),
        ("1–3 days", "success", row.get::<_, i64>(1)),
        ("3–7 days", "accent", row.get::<_, i64>(2)),
        ("7–30 days", "warning", row.get::<_, i64>(3)),
        ("> 30 days", "danger", row.get::<_, i64>(4)),
    ];
    let total: i64 = counts.iter().map(|(_, _, count)| *count).sum();

    Ok(counts
        .into_iter()
        .map(|(label, tone, count)| TurnaroundBucket {
            label: label.to_string(),
            count,
            share_pct: stats_manager::coverage_percentage(count, total),
            tone: tone.to_string(),
        })
        .collect())
}

pub async fn get_pending_by_subdeck(
    db_state: &database::AppState,
    deck_id: i64,
) -> Return<Vec<SubdeckBacklogRow>> {
    let client = database::client(db_state).await?;
    let query = format!(
        "{SUBTREE_CTE}
        SELECT COALESCE(NULLIF(BTRIM(d.full_path), ''), NULLIF(BTRIM(d.name), ''), '[Unnamed Deck]') AS path,
               COUNT(*)::bigint AS pending,
               FLOOR(EXTRACT(EPOCH FROM (now() - MIN(c.timestamp))) / 86400)::bigint AS oldest_days
          FROM anki.commits c
          JOIN anki.decks d ON d.id = c.deck
         WHERE c.deck IN (SELECT id FROM tree)
           AND {COMMIT_IS_PENDING}
         GROUP BY d.id, d.full_path, d.name
         ORDER BY pending DESC, oldest_days DESC NULLS LAST
         LIMIT 6"
    );
    let rows = client.query(&query, &[&deck_id]).await?;
    Ok(rows
        .iter()
        .map(|row| {
            let path: String = row.get(0);
            let (prefix, leaf, _) = stats_manager::split_hierarchy_path(&path);
            SubdeckBacklogRow {
                path,
                prefix,
                leaf,
                pending: row.get(1),
                oldest_days: row.get(2),
            }
        })
        .collect())
}

pub async fn get_community_health(
    db_state: &database::AppState,
    deck_id: i64,
) -> Return<CommunityHealth> {
    let client = database::client(db_state).await?;
    let active_query = format!(
        "{SUBTREE_CTE}
        SELECT
            (SELECT COUNT(DISTINCT c.user_id)::bigint
               FROM anki.commits c
              WHERE c.deck IN (SELECT id FROM tree)
                AND c.user_id IS NOT NULL
                AND c.timestamp >= now() - INTERVAL '30 days'),
            (SELECT COUNT(*)::bigint
               FROM (
                   SELECT c.user_id
                     FROM anki.commits c
                     JOIN anki.notifications n ON n.commit_id = c.commit_id
                    WHERE c.deck IN (SELECT id FROM tree)
                      AND n.status = 'approved'
                      AND c.user_id IS NOT NULL
                    GROUP BY c.user_id
                   HAVING COUNT(DISTINCT c.commit_id) >= 2
               ) returning_users)"
    );
    let active_row = client.query_one(&active_query, &[&deck_id]).await?;

    let notes_query = format!(
        "{SUBTREE_CTE}
        SELECT COUNT(DISTINCT ne.note_id)::bigint
          FROM anki.note_events ne
          JOIN anki.commits c ON c.commit_id = ne.commit_id
         WHERE c.deck IN (SELECT id FROM tree)
           AND ne.note_id IS NOT NULL
           AND EXISTS (
               SELECT 1 FROM anki.notifications n
                WHERE n.commit_id = c.commit_id AND n.status = 'approved'
           )"
    );
    let notes_row = client.query_one(&notes_query, &[&deck_id]).await?;

    Ok(CommunityHealth {
        active_contributors_30d: active_row.get(0),
        returning_contributors: active_row.get(1),
        notes_improved: notes_row.get(0),
    })
}

pub async fn get_contributor_summary(
    db_state: &database::AppState,
    deck_id: i64,
    username: &str,
) -> Return<ContributorSummary> {
    let client = database::client(db_state).await?;
    let summary_query = format!(
        "{SUBTREE_CTE},
        sugg AS (
            SELECT c.commit_id, c.timestamp, COALESCE(c.rationale, 0) AS rationale,
                   {COMMIT_IS_PENDING} AS pending
              FROM anki.commits c
              JOIN public.users u ON u.id = c.user_id
             WHERE c.deck IN (SELECT id FROM tree)
               AND u.username = $2
        ),
        verdicts AS (
            SELECT n.commit_id,
                   bool_or(n.status = 'approved') AS approved,
                   bool_or(n.status = 'denied')   AS denied
              FROM anki.notifications n
             WHERE n.commit_id IN (SELECT commit_id FROM sugg)
             GROUP BY n.commit_id
        )
        SELECT COUNT(*)::bigint AS submitted,
               COUNT(*) FILTER (WHERE COALESCE(v.approved, false))::bigint AS accepted,
               COUNT(*) FILTER (WHERE COALESCE(v.denied, false)
                                  AND NOT COALESCE(v.approved, false))::bigint AS declined,
               COUNT(*) FILTER (WHERE s.pending)::bigint AS pending,
               TO_CHAR(MIN(s.timestamp), 'YYYY-MM-DD') AS first_contribution,
               TO_CHAR(MAX(s.timestamp), 'YYYY-MM-DD') AS last_contribution
          FROM sugg s
          LEFT JOIN verdicts v ON v.commit_id = s.commit_id"
    );
    let row = client
        .query_one(&summary_query, &[&deck_id, &username])
        .await?;

    let submitted: i64 = row.get(0);
    let accepted: i64 = row.get(1);
    let declined: i64 = row.get(2);
    let pending: i64 = row.get(3);
    let reviewed = accepted + declined;
    let acceptance_ratio = gated_acceptance_ratio(accepted, reviewed);

    let rationale_query = format!(
        "{SUBTREE_CTE}
        SELECT COALESCE(c.rationale, 0) AS rationale, COUNT(*)::bigint AS cnt
          FROM anki.commits c
          JOIN public.users u ON u.id = c.user_id
         WHERE c.deck IN (SELECT id FROM tree)
           AND u.username = $2
         GROUP BY COALESCE(c.rationale, 0)
         ORDER BY cnt DESC
         LIMIT 3"
    );
    let rationale_rows = client
        .query(&rationale_query, &[&deck_id, &username])
        .await?;
    let top_rationales = rationale_rows
        .iter()
        .map(|r| {
            let code: i32 = r.get(0);
            ContributorRationale {
                rationale: crate::commit_manager::get_string_from_rationale(code).to_string(),
                count: r.get(1),
            }
        })
        .collect();

    Ok(ContributorSummary {
        submitted,
        accepted,
        declined,
        pending,
        acceptance_ratio,
        first_contribution: row.get(4),
        last_contribution: row.get(5),
        milestone: milestone_for_accepted(accepted),
        top_rationales,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::{Month, SignedDuration, Weekday};

    fn monday_of_week(year: i32, week: u8) -> Date {
        Date::from_iso_week_date(year, week, Weekday::Monday).expect("valid ISO week")
    }

    fn point(on: Date, total: i64) -> SnapshotPoint {
        SnapshotPoint {
            on,
            total,
            active_30d: total,
            active_90d: total,
        }
    }

    #[test]
    fn deck_name_uses_the_last_path_segment() {
        assert_eq!(
            deck_display_name_from_path("Med::Anatomy::Upper Limb"),
            "Upper Limb"
        );
        assert_eq!(deck_display_name_from_path("Genetics"), "Genetics");
    }

    #[test]
    fn deck_name_defaults_when_the_path_is_unusable() {
        assert_eq!(deck_display_name_from_path(""), "[Unnamed Deck]");
        assert_eq!(deck_display_name_from_path("Med::"), "[Unnamed Deck]");
        assert_eq!(deck_display_name_from_path("   "), "[Unnamed Deck]");
    }

    #[test]
    fn usernames_are_encoded_so_they_cannot_break_a_link() {
        assert_eq!(url_path_segment("alice"), "alice");
        assert_eq!(url_path_segment("a.b-c_d~e"), "a.b-c_d~e");
        assert_eq!(url_path_segment("a b"), "a%20b");
        assert_eq!(url_path_segment("a/b"), "a%2Fb");
        assert_eq!(url_path_segment("a?b#c&d%e"), "a%3Fb%23c%26d%25e");
        assert_eq!(url_path_segment("ü"), "%C3%BC");
    }

    #[test]
    fn breakdown_hidden_below_five_contributors() {
        assert!(!breakdown_visible(0));
        assert!(!breakdown_visible(4));
    }

    #[test]
    fn breakdown_visible_from_five_contributors() {
        assert!(breakdown_visible(5));
        assert!(breakdown_visible(40));
    }

    #[test]
    fn timeline_of_no_snapshots_is_empty() {
        let buckets = build_timeline(&[], &BTreeMap::new(), Granularity::Daily);
        assert_eq!(buckets.len(), 0);
    }

    #[test]
    fn first_snapshot_is_a_baseline_with_no_change() {
        let day = monday_of_week(2026, 41);
        let buckets = build_timeline(&[point(day, 210)], &BTreeMap::new(), Granularity::Daily);
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].total, 210);
        assert_eq!(buckets[0].new, 0);
        assert_eq!(buckets[0].unsubscribed, 0);
        assert_eq!(buckets[0].net, 0);
        assert_eq!(buckets[0].label, "2026-10-05");
    }

    #[test]
    fn first_day_arrivals_are_not_reported_as_unsubscribes() {
        let day = monday_of_week(2026, 41);
        let mut new_by_date = BTreeMap::new();
        new_by_date.insert(day, 4);

        let buckets = build_timeline(&[point(day, 210)], &new_by_date, Granularity::Daily);

        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].total, 210, "the cumulative total is real");
        assert_eq!(buckets[0].new, 0, "the baseline day claims no flows");
        assert_eq!(
            buckets[0].unsubscribed, 0,
            "and certainly not four departures"
        );
        assert_eq!(buckets[0].net, 0);
    }

    #[test]
    fn arrivals_and_departures_are_derived_from_the_totals() {
        let day1 = monday_of_week(2026, 41);
        let day2 = day1 + SignedDuration::days(1);
        let mut new_by_date = BTreeMap::new();
        new_by_date.insert(day2, 4);

        let buckets = build_timeline(
            &[point(day1, 100), point(day2, 103)],
            &new_by_date,
            Granularity::Daily,
        );

        assert_eq!(buckets.len(), 2);
        assert_eq!(buckets[1].total, 103);
        assert_eq!(buckets[1].new, 4);
        assert_eq!(buckets[1].unsubscribed, 1);
        assert_eq!(buckets[1].net, 3);
    }

    #[test]
    fn a_pure_departure_shows_as_unsubscribed() {
        let day1 = monday_of_week(2026, 41);
        let day2 = day1 + SignedDuration::days(1);
        let buckets = build_timeline(
            &[point(day1, 100), point(day2, 98)],
            &BTreeMap::new(),
            Granularity::Daily,
        );
        assert_eq!(buckets[1].new, 0);
        assert_eq!(buckets[1].unsubscribed, 2);
        assert_eq!(buckets[1].net, -2);
    }

    #[test]
    fn legacy_rows_leaving_never_produce_negative_unsubscribes() {
        let day1 = monday_of_week(2026, 41);
        let day2 = day1 + SignedDuration::days(1);
        let mut new_by_date = BTreeMap::new();
        new_by_date.insert(day2, 1);

        let buckets = build_timeline(
            &[point(day1, 50), point(day2, 48)],
            &new_by_date,
            Granularity::Daily,
        );
        assert_eq!(buckets[1].new, 1);
        assert_eq!(buckets[1].unsubscribed, 3);
        assert_eq!(buckets[1].net, -2);
    }

    #[test]
    fn weekly_buckets_collapse_a_week_and_sum_its_changes() {
        let monday = monday_of_week(2026, 41);
        let tuesday = monday + SignedDuration::days(1);
        let next_monday = monday + SignedDuration::days(7);
        let mut new_by_date = BTreeMap::new();
        new_by_date.insert(tuesday, 2);

        let buckets = build_timeline(
            &[
                point(monday, 100),
                point(tuesday, 102),
                point(next_monday, 105),
            ],
            &new_by_date,
            Granularity::Weekly,
        );

        assert_eq!(buckets.len(), 2, "two ISO weeks");
        assert_eq!(buckets[0].label, "2026-W41");
        assert_eq!(buckets[0].total, 102);
        assert_eq!(buckets[0].new, 2);
        assert_eq!(buckets[0].net, 2);
        assert_eq!(buckets[1].label, "2026-W42");
        assert_eq!(buckets[1].total, 105);
        assert_eq!(buckets[1].net, 3);
    }

    #[test]
    fn weekly_buckets_use_iso_week_year_across_year_boundaries() {
        let dec_30_2024 = Date::from_calendar_date(2024, Month::December, 30).unwrap();
        let jan_1_2021 = Date::from_calendar_date(2021, Month::January, 1).unwrap();
        assert_eq!(bucket_label(dec_30_2024, Granularity::Weekly), "2025-W01");
        assert_eq!(bucket_label(jan_1_2021, Granularity::Weekly), "2020-W53");
    }

    #[test]
    fn timeline_carries_the_active_counts_of_the_last_snapshot_in_a_bucket() {
        let monday = monday_of_week(2026, 41);
        let tuesday = monday + SignedDuration::days(1);
        let mut first = point(monday, 100);
        first.active_30d = 40;
        let mut second = point(tuesday, 101);
        second.active_30d = 42;

        let buckets = build_timeline(&[first, second], &BTreeMap::new(), Granularity::Weekly);
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].active_30d, 42);
    }

    #[test]
    fn digest_is_encouraging_when_nothing_has_happened_yet() {
        let sentences = digest_sentences(&DigestInput::default());
        assert!(
            sentences[0].contains("sharing the deck"),
            "expected an encouraging first line, got {:?}",
            sentences[0]
        );
        assert!(sentences[1].contains("first"), "got {:?}", sentences[1]);
    }

    #[test]
    fn digest_states_the_months_impact() {
        let sentences = digest_sentences(&DigestInput {
            people_this_month: 14,
            accepted_this_month: 37,
            all_time_contributors: 52,
            subscribers: 2310,
            median_review_hours: Some(6.0),
            pending: 0,
            oldest_pending_days: None,
        });
        assert!(sentences[0].contains("14"), "got {:?}", sentences[0]);
        assert!(sentences[0].contains("37"), "got {:?}", sentences[0]);
        assert!(sentences[1].contains("52"), "got {:?}", sentences[1]);
    }

    #[test]
    fn digest_reports_review_speed_when_there_is_no_backlog() {
        let sentences = digest_sentences(&DigestInput {
            people_this_month: 3,
            accepted_this_month: 5,
            all_time_contributors: 9,
            subscribers: 100,
            median_review_hours: Some(6.4),
            pending: 1,
            oldest_pending_days: Some(2),
        });
        assert!(sentences[2].contains("hours"), "got {:?}", sentences[2]);
    }

    #[test]
    fn digest_switches_to_a_backlog_nudge_after_a_month() {
        let sentences = digest_sentences(&DigestInput {
            people_this_month: 3,
            accepted_this_month: 5,
            all_time_contributors: 9,
            subscribers: 100,
            median_review_hours: Some(6.4),
            pending: 7,
            oldest_pending_days: Some(45),
        });
        assert!(sentences[2].contains("45"), "got {:?}", sentences[2]);
        assert!(sentences[2].contains('7'), "got {:?}", sentences[2]);
    }

    #[test]
    fn digest_never_shames_an_empty_month_with_a_zero_count() {
        let sentences = digest_sentences(&DigestInput {
            people_this_month: 0,
            accepted_this_month: 0,
            all_time_contributors: 12,
            subscribers: 400,
            median_review_hours: None,
            pending: 0,
            oldest_pending_days: None,
        });
        assert!(!sentences[0].contains('0'), "got {:?}", sentences[0]);
    }

    #[test]
    fn digest_agrees_with_itself_at_one() {
        // Every count in the digest is user-visible, and each one reads wrong at exactly 1.
        let sentences = digest_sentences(&DigestInput {
            people_this_month: 1,
            accepted_this_month: 1,
            all_time_contributors: 1,
            subscribers: 1,
            median_review_hours: None,
            pending: 1,
            oldest_pending_days: Some(45),
        });
        let joined = sentences.join(" ");
        assert!(joined.contains("1 contributor improved"), "got {joined}");
        assert!(joined.contains("1 suggestion accepted"), "got {joined}");
        assert!(joined.contains("1 contributor has shaped"), "got {joined}");
        assert!(joined.contains("for 1 learner."), "got {joined}");
        assert!(joined.contains("1 suggestion waiting"), "got {joined}");
        assert!(joined.contains("for 45 days."), "got {joined}");
        assert!(!joined.contains("contributors improved"), "got {joined}");
        assert!(!joined.contains("learners."), "got {joined}");
    }

    #[test]
    fn milestone_progression_covers_boundaries() {
        let zero = milestone_for_accepted(0);
        assert_eq!(zero.current_name, None);
        assert_eq!(zero.next_name, Some("Contributor"));
        assert_eq!(zero.remaining_to_next, Some(1));
        assert_eq!(zero.progress_pct, 0);

        let one = milestone_for_accepted(1);
        assert_eq!(one.current_name, Some("Contributor"));
        assert_eq!(one.next_name, Some("Regular Contributor"));
        assert_eq!(one.remaining_to_next, Some(4));
        assert_eq!(one.progress_pct, 0);

        let three = milestone_for_accepted(3);
        assert_eq!(three.current_name, Some("Contributor"));
        assert_eq!(three.progress_pct, 50);
        assert_eq!(three.remaining_to_next, Some(2));

        let steward = milestone_for_accepted(50);
        assert_eq!(steward.current_name, Some("Deck Steward"));
        assert_eq!(steward.next_name, Some("Community Steward"));
        assert_eq!(steward.remaining_to_next, Some(50));

        let top = milestone_for_accepted(640);
        assert_eq!(top.current_name, Some("Cornerstone"));
        assert_eq!(top.next_name, None);
        assert_eq!(top.remaining_to_next, None);
        assert_eq!(top.progress_pct, 100);
    }

    #[test]
    fn subscriber_summary_handles_empty_untracked_and_stickiness() {
        let empty = summarize_subscribers(0, &[], true);
        assert_eq!(empty.total, 0);
        assert_eq!(empty.net_30d, None);
        assert_eq!(empty.active_30d, None);
        assert_eq!(empty.stickiness_30d_pct, None);

        let day1 = monday_of_week(2026, 40);
        let day2 = day1 + SignedDuration::days(30);
        let p1 = SnapshotPoint {
            on: day1,
            total: 200,
            active_30d: 80,
            active_90d: 120,
        };
        let p2 = SnapshotPoint {
            on: day2,
            total: 250,
            active_30d: 100,
            active_90d: 150,
        };

        let untracked = summarize_subscribers(250, &[p1, p2], false);
        assert_eq!(untracked.net_30d, Some(50));
        assert_eq!(untracked.active_30d, None);
        assert_eq!(untracked.stickiness_30d_pct, None);

        let tracked = summarize_subscribers(250, &[p1, p2], true);
        assert_eq!(tracked.net_30d, Some(50));
        assert_eq!(tracked.active_30d, Some(100));
        assert!((tracked.stickiness_30d_pct.unwrap() - 40.0).abs() < 1e-6);
        assert!((tracked.stickiness_90d_pct.unwrap() - 60.0).abs() < 1e-6);
    }

    #[test]
    fn suggestion_timeline_buckets_daily_weekly_and_monthly() {
        let d1 = Date::from_calendar_date(2026, Month::October, 5).unwrap();
        let d2 = Date::from_calendar_date(2026, Month::October, 6).unwrap();
        let mut submitted = BTreeMap::new();
        submitted.insert(d1, 3);
        submitted.insert(d2, 2);
        let mut verdicts = BTreeMap::new();
        verdicts.insert(d2, (4, 1));

        let daily = build_suggestion_timeline(&submitted, &verdicts, 30, Granularity::Daily);
        assert_eq!(daily.len(), 2);
        assert_eq!(daily[0].label, "2026-10-05");
        assert_eq!(daily[1].accepted, 4);

        let weekly = build_suggestion_timeline(&submitted, &verdicts, 90, Granularity::Weekly);
        assert_eq!(weekly.len(), 1);
        assert_eq!(weekly[0].submitted, 5);
        assert_eq!(weekly[0].accepted, 4);
        assert_eq!(weekly[0].declined, 1);

        let monthly = build_suggestion_timeline(&submitted, &verdicts, 730, Granularity::Daily);
        assert_eq!(monthly.len(), 1);
        assert_eq!(monthly[0].label, "2026-10");
        assert_eq!(monthly[0].submitted, 5);
    }
}
