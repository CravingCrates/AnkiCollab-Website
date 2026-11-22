use crate::database;
use crate::stats_cache::DeckStatsPayload;
use crate::structs::{
    DeckBaseStatsInfo, DeckStatsInfo, NoteStatsInfo, NotetypeStatsInfo, RetentionBucket,
    RetentionTrendPoint, TagStatsInfo,
};
use async_recursion::async_recursion;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use time::{Date, Duration};

pub type DynError = Box<dyn std::error::Error + Send + Sync>;

/// Lightweight metadata read directly from `anki.decks` for instant shell rendering.
#[derive(Debug, Clone)]
pub struct DeckStatsShell {
    pub deck_id: i64,
    pub deck_name: String,
    pub stats_enabled: bool,
    pub note_count: i32,
    pub retention_avg: f32,
    pub stats_learner_count: Option<i32>,
}

/// Minimum distinct reporting learners before learner-spread or group rollups are shown.
pub const MIN_LEARNER_SAMPLE: i64 = 5;

/// Minimum tracked notes before a tag or notetype aggregate is surfaced.
pub const MIN_TAG_NOTES: i64 = 5;

/// Window (in days) within which a note edit is flagged as recently revised (review-interval lag).
pub const RECENT_REVISION_DAYS: i64 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagRollupPlan {
    pub rollup_depth: Option<usize>,
    pub shared_root: Option<String>,
}

/// Split an Anki `::`-delimited path or tag into `(prefix_with_sep, leaf, depth)`.
/// Keeping the full text across both spans lets CSS truncate the prefix while preserving
/// the full string for screen readers, copy, and find-in-page.
#[must_use]
pub fn split_hierarchy_path(path: &str) -> (String, String, usize) {
    let trimmed = path.trim().trim_end_matches("::").trim();
    if trimmed.is_empty() {
        return (String::new(), "[Unnamed]".to_string(), 0);
    }

    let depth = trimmed.matches("::").count();
    match trimmed.rsplit_once("::") {
        Some((prefix, leaf)) => {
            let leaf_trimmed = leaf.trim();
            if leaf_trimmed.is_empty() {
                (String::new(), trimmed.to_string(), 0)
            } else {
                (format!("{prefix}::"), leaf_trimmed.to_string(), depth)
            }
        }
        None => (String::new(), trimmed.to_string(), 0),
    }
}

/// Identify Anki / `AnkiCollab` housekeeping tags that would pollute topic performance breakdowns.
#[must_use]
pub fn is_housekeeping_tag(tag: &str) -> bool {
    let trimmed = tag.trim();
    if trimmed.is_empty() {
        return true;
    }

    let mut segments = trimmed
        .split("::")
        .map(str::trim)
        .filter(|segment| !segment.is_empty());

    let Some(first) = segments.next() else {
        return true;
    };

    let first_norm = first.trim_start_matches('#').to_ascii_lowercase();
    if matches!(
        first_norm.as_str(),
        "leech" | "marked" | "suspended" | "buried" | "flag"
    ) {
        return true;
    }

    std::iter::once(first).chain(segments).any(|segment| {
        let norm = segment.trim_start_matches('#').to_ascii_lowercase();
        norm.starts_with("ankicollab")
            || norm.starts_with("anki_collab")
            || norm.starts_with("crowdanki")
    })
}

/// Decide whether tags should stay verbatim (shallow study groups) or roll up to a readable
/// branch depth (large nested decks like `Ankizin`).
#[must_use]
pub fn plan_tag_rollup(tags: &[String]) -> TagRollupPlan {
    let cleaned: Vec<&str> = tags
        .iter()
        .map(|tag| tag.trim())
        .filter(|tag| !is_housekeeping_tag(tag))
        .collect();

    if cleaned.is_empty() {
        return TagRollupPlan {
            rollup_depth: None,
            shared_root: None,
        };
    }

    let mut deep_roots: Vec<&str> = Vec::new();
    for tag in &cleaned {
        let mut parts = tag.split("::").map(str::trim).filter(|s| !s.is_empty());
        let first = parts.next();
        let second = parts.next();
        let third = parts.next();
        if let (Some(root), Some(_), Some(_)) = (first, second, third) {
            deep_roots.push(root);
        }
    }

    // Less than 40% deeply nested (>= 3 levels): keep exact tags for shallow study-group decks.
    if deep_roots.len() * 10 < cleaned.len() * 4 {
        return TagRollupPlan {
            rollup_depth: None,
            shared_root: None,
        };
    }

    let mut root_counts: HashMap<String, (usize, &str)> = HashMap::new();
    for root in &deep_roots {
        let key = root.to_ascii_lowercase();
        let entry = root_counts.entry(key).or_insert((0, root));
        entry.0 += 1;
    }

    if let Some((_, (top_count, canonical_root))) =
        root_counts.into_iter().max_by_key(|(_, (count, _))| *count)
    {
        // When >= 80% of nested tags share one root (e.g. `#Ankizin::...`),
        // rolling up to depth 3 yields `Root::Subject::Topic` instead of collapsing everything
        // into just `Root::Subject`.
        if top_count * 10 >= deep_roots.len() * 8 {
            return TagRollupPlan {
                rollup_depth: Some(3),
                shared_root: Some(canonical_root.to_string()),
            };
        }
    }

    TagRollupPlan {
        rollup_depth: Some(2),
        shared_root: None,
    }
}

#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn coverage_percentage(tracked: i64, total: i64) -> f64 {
    if tracked <= 0 || total <= 0 {
        return 0.0;
    }
    ((tracked as f64 / total as f64) * 100.0).clamp(0.0, 100.0)
}

/// Compute retention change between the latest snapshot and the snapshot closest to 30 days prior.
/// Returns `None` when fewer than 2 snapshots exist or the span is under 7 days.
#[must_use]
pub fn compute_trend_delta_30d(points: &[(Date, f32)]) -> Option<f32> {
    if points.len() < 2 {
        return None;
    }
    let first = points.first()?;
    let last = points.last()?;
    if (last.0 - first.0).whole_days() < 7 {
        return None;
    }

    let target = last.0 - Duration::days(30);
    let baseline = points[..points.len() - 1]
        .iter()
        .min_by_key(|point| ((point.0 - target).whole_days().abs(), point.0))?;

    let delta = ((last.1 - baseline.1) * 10.0).round() / 10.0;
    Some(delta)
}

pub async fn update_stats(db_state: &database::AppState) -> Result<(), DynError> {
    // Refresh the note calculated_stats
    calculate_note_stats(db_state).await?;

    // Update the decks retention rates
    update_all_decks(db_state).await?;

    // Capture daily deck retention snapshot for trend tracking (k-anonymity >= 5 learners)
    snapshot_deck_retention(db_state).await?;

    db_state.stats_cache.invalidate_all();

    Ok(())
}

fn is_undefined_schema_error(err: &tokio_postgres::Error) -> bool {
    err.code().is_some_and(|code| {
        *code == tokio_postgres::error::SqlState::UNDEFINED_COLUMN
            || *code == tokio_postgres::error::SqlState::UNDEFINED_TABLE
    })
}

fn calculate_note_stats_sql(include_stddev: bool) -> String {
    let (extra_col, extra_expr, extra_update) = if include_stddev {
        (
            ", retention_stddev",
            ",
            CASE
                WHEN COUNT(DISTINCT ns.user_hash) >= 5
                THEN CAST(ROUND(STDDEV_POP(ns.retention)::numeric, 1) AS REAL)
                ELSE NULL
            END as retention_stddev",
            ",
            retention_stddev = EXCLUDED.retention_stddev",
        )
    } else {
        ("", "", "")
    };

    format!(
        "
        WITH non_deleted_notes AS (
            SELECT id FROM notes WHERE NOT deleted
        )
        INSERT INTO calculated_stats (note_id, sample_size, retention, lapses, reps{extra_col})
        SELECT
            ns.note_id,
            COUNT(DISTINCT ns.user_hash) as sample_size,
            ROUND(AVG(ns.retention), 1) as retention,
            ROUND(AVG(ns.lapses), 1) as lapses,
            ROUND(AVG(ns.reps), 1) as reps{extra_expr}
        FROM
            note_stats ns
        JOIN
            non_deleted_notes n ON ns.note_id = n.id
        GROUP BY
            ns.note_id
        ON CONFLICT (note_id) DO UPDATE
        SET
            sample_size = EXCLUDED.sample_size,
            retention = EXCLUDED.retention,
            lapses = EXCLUDED.lapses,
            reps = EXCLUDED.reps{extra_update}
        "
    )
}

pub async fn calculate_note_stats(db_state: &database::AppState) -> Result<(), DynError> {
    let client = database::client(db_state).await?;
    let query = calculate_note_stats_sql(true);
    if let Err(err) = client.execute(&query, &[]).await {
        if is_undefined_schema_error(&err) {
            tracing::warn!(
                error = %err,
                "calculated_stats.retention_stddev missing; falling back to legacy 5-column stats refresh"
            );
            let legacy_query = calculate_note_stats_sql(false);
            client.execute(&legacy_query, &[]).await?;
        } else {
            return Err(Box::new(err));
        }
    }

    let update_query = "
        UPDATE decks
        SET notes_with_stats_count = (
            SELECT COUNT(*)
            FROM notes n JOIN calculated_stats cs ON n.id = cs.note_id
            WHERE n.deck = decks.id AND NOT n.deleted
        )
        WHERE id IN (
            SELECT deck
            FROM notes
            WHERE id IN (
                SELECT note_id
                FROM calculated_stats
            )
        )
    ";
    client.execute(update_query, &[]).await?;
    Ok(())
}

fn snapshot_deck_retention_sql(include_learner_count_update: bool) -> String {
    let update_cte = if include_learner_count_update {
        ",
        updated_learner_counts AS (
            UPDATE anki.decks d
            SET stats_learner_count = asl.learner_count::int
            FROM all_subtree_learners asl
            WHERE d.id = asl.deck_id
              AND d.stats_learner_count IS DISTINCT FROM asl.learner_count::int
        )"
    } else {
        ""
    };

    format!(
        "
        WITH RECURSIVE enabled_decks AS (
            SELECT id, parent
            FROM anki.decks
            WHERE stats_enabled = true AND deleted_at IS NULL
            UNION
            SELECT d.id, d.parent
            FROM anki.decks d
            JOIN enabled_decks e ON d.parent = e.id
            WHERE d.deleted_at IS NULL
        ),
        descendants AS (
            SELECT id AS ancestor_id, id AS descendant_id
            FROM enabled_decks
            UNION
            SELECT des.ancestor_id, d.id
            FROM descendants des
            JOIN anki.decks d ON d.parent = des.descendant_id
            WHERE d.deleted_at IS NULL
        ),
        per_deck_learners AS (
            SELECT n.deck AS deck_id, ns.user_hash
            FROM anki.notes n
            JOIN anki.note_stats ns ON ns.note_id = n.id
            WHERE n.deck IN (SELECT id FROM enabled_decks)
              AND NOT n.deleted
            GROUP BY n.deck, ns.user_hash
        ),
        all_subtree_learners AS (
            SELECT e.id AS deck_id,
                   COUNT(DISTINCT pdl.user_hash)::bigint AS learner_count
            FROM enabled_decks e
            LEFT JOIN descendants des ON des.ancestor_id = e.id
            LEFT JOIN per_deck_learners pdl ON pdl.deck_id = des.descendant_id
            GROUP BY e.id
        ),
        direct_calc AS (
            SELECT n.deck AS deck_id,
                   COUNT(cs.note_id)::bigint AS tracked_notes,
                   COALESCE(SUM(cs.retention) FILTER (WHERE cs.retention IS NOT NULL), 0)::float8 AS sum_retention,
                   COUNT(cs.retention)::bigint AS cnt_retention,
                   COALESCE(SUM(cs.lapses), 0)::float8 AS sum_lapses
            FROM anki.notes n
            JOIN anki.calculated_stats cs ON cs.note_id = n.id
            WHERE n.deck IN (SELECT id FROM enabled_decks)
              AND NOT n.deleted
            GROUP BY n.deck
        ),
        subtree_calc AS (
            SELECT des.ancestor_id AS deck_id,
                   SUM(dc.tracked_notes)::int AS tracked_notes,
                   CAST(ROUND((SUM(dc.sum_retention) / NULLIF(SUM(dc.cnt_retention), 0))::numeric, 1) AS REAL) AS avg_retention,
                   CAST(ROUND((SUM(dc.sum_lapses) / NULLIF(SUM(dc.tracked_notes), 0))::numeric, 1) AS REAL) AS avg_lapses
            FROM descendants des
            JOIN direct_calc dc ON dc.deck_id = des.descendant_id
            GROUP BY des.ancestor_id
            HAVING SUM(dc.cnt_retention) > 0 AND SUM(dc.tracked_notes) > 0
        ){update_cte}
        INSERT INTO anki.deck_retention_daily (deck_id, captured_on, avg_retention, avg_lapses, tracked_notes)
        SELECT sc.deck_id, CURRENT_DATE, sc.avg_retention, sc.avg_lapses, sc.tracked_notes
        FROM subtree_calc sc
        JOIN all_subtree_learners asl ON asl.deck_id = sc.deck_id
        WHERE asl.learner_count >= $1::bigint
        ON CONFLICT (deck_id, captured_on) DO UPDATE
           SET avg_retention = EXCLUDED.avg_retention,
               avg_lapses    = EXCLUDED.avg_lapses,
               tracked_notes = EXCLUDED.tracked_notes
        "
    )
}

pub async fn snapshot_deck_retention(db_state: &database::AppState) -> Result<(), DynError> {
    let client = database::client(db_state).await?;
    let query = snapshot_deck_retention_sql(true);
    match client.execute(&query, &[&MIN_LEARNER_SAMPLE]).await {
        Ok(_) => Ok(()),
        Err(err) if is_undefined_schema_error(&err) => {
            tracing::warn!(
                error = %err,
                "decks.stats_learner_count or anki.deck_retention_daily missing; falling back to snapshot without cached learner count"
            );
            let fallback_query = snapshot_deck_retention_sql(false);
            if let Err(fallback_err) = client
                .execute(&fallback_query, &[&MIN_LEARNER_SAMPLE])
                .await
            {
                if is_undefined_schema_error(&fallback_err) {
                    tracing::warn!(
                        error = %fallback_err,
                        "anki.deck_retention_daily missing; skipping daily retention snapshot until migration is applied"
                    );
                    return Ok(());
                }
                return Err(Box::new(fallback_err));
            }
            Ok(())
        }
        Err(err) => Err(Box::new(err)),
    }
}

pub async fn get_leaf_decks(db_state: &database::AppState) -> Result<Vec<i64>, DynError> {
    let client = database::client(db_state).await?;
    let query = "
        WITH RECURSIVE cte AS (
            SELECT id, parent FROM decks WHERE stats_enabled = true AND deleted_at IS NULL
            UNION ALL
            SELECT d.id, d.parent
            FROM cte JOIN decks d ON d.parent = cte.id
            WHERE d.deleted_at IS NULL
        )
        SELECT id FROM cte d
        WHERE NOT EXISTS (
            SELECT 1 FROM decks WHERE parent = d.id AND deleted_at IS NULL
        )
    ";
    let rows = client.query(query, &[]).await?;

    let leaf_decks = rows.into_iter().map(|row| row.get(0)).collect::<Vec<i64>>();
    Ok(leaf_decks)
}

#[allow(clippy::double_must_use)]
#[async_recursion]
pub async fn calculate_average_retention(
    db_state: &database::AppState,
    deck: i64,
) -> Result<Option<f32>, DynError> {
    let client = database::client(db_state).await?;
    let query = "
        WITH notes_with_stats AS (
            SELECT n.id, cs.retention, 1 as note_count
            FROM notes n JOIN calculated_stats cs ON n.id = cs.note_id
            WHERE n.deck = $1 and n.deleted = false and cs.retention IS NOT NULL
        ),
        decks_with_retention AS (
            SELECT d.id, d.retention, d.notes_with_stats_count as note_count
            FROM decks d
            WHERE d.parent = $1 AND d.retention IS NOT NULL AND d.deleted_at IS NULL
        ),
        combined AS (
            SELECT retention, note_count FROM notes_with_stats
            UNION ALL
            SELECT retention, note_count FROM decks_with_retention
        )
        SELECT 
            CASE 
                WHEN SUM(note_count) = 0 THEN NULL
                ELSE CAST(ROUND((SUM(retention * note_count) / SUM(note_count))::numeric, 1) AS REAL)
            END as average_retention 
        FROM combined
    ";
    let rows = client.query(query, &[&deck]).await?;

    if rows.is_empty() {
        return Ok(None);
    }

    let average_retention: Option<f32> = rows[0].get(0);
    Ok(average_retention)
}

#[allow(clippy::double_must_use)]
#[async_recursion]
pub async fn update_deck_and_parent_retention(
    db_state: &database::AppState,
    deck: i64,
) -> Result<(), DynError> {
    let client = database::client(db_state).await?;
    // Get the note count for the current deck and its subdecks
    let update_note_count_query = "
        UPDATE decks
        SET notes_with_stats_count = (
            SELECT COALESCE(SUM(notes_with_stats_count), 0)
            FROM decks
            WHERE parent = $1 AND deleted_at IS NULL
        ) + (
            SELECT COUNT(*)
            FROM notes n JOIN calculated_stats cs ON n.id = cs.note_id
            WHERE n.deck = $1 AND NOT n.deleted
        )
        WHERE id = $1
    ";
    client.execute(update_note_count_query, &[&deck]).await?;

    let retention = calculate_average_retention(db_state, deck).await?;
    if let Some(retention) = retention {
        let parent_query = "SELECT parent FROM decks WHERE id = $1";
        let rows = client.query(parent_query, &[&deck]).await?;

        let query = "UPDATE decks SET retention = $2 WHERE id = $1";
        client.execute(query, &[&deck, &retention]).await?;

        if let Some(parent_deck) = rows.first().and_then(|row| row.get::<_, Option<i64>>(0)) {
            update_deck_and_parent_retention(db_state, parent_deck).await?;
        }
    }

    Ok(())
}

pub async fn update_all_decks(db_state: &database::AppState) -> Result<(), DynError> {
    let leaf_decks = get_leaf_decks(db_state).await?;

    for deck in leaf_decks {
        update_deck_and_parent_retention(db_state, deck).await?;
    }

    Ok(())
}

const BASE_AND_DIST_COLS: usize = 6;

const RETENTION_BUCKET_SPECS: [(&str, &str, &str); 7] = [
    ("< 60%", "lt60", "danger"),
    ("60–70%", "60_70", "danger"),
    ("70–80%", "70_80", "warning"),
    ("80–85%", "80_85", "accent"),
    ("85–90%", "85_90", "success"),
    ("90–95%", "90_95", "success"),
    ("95–100%", "95_100", "accent"),
];

fn deck_stats_shell_sql(include_learner_count: bool) -> &'static str {
    if include_learner_count {
        "SELECT id,
                COALESCE(NULLIF(BTRIM(full_path), ''), NULLIF(BTRIM(name), ''), ''),
                COALESCE(stats_enabled, false),
                COALESCE(notes_with_stats_count, 0),
                COALESCE(retention, 0.0),
                stats_learner_count
           FROM anki.decks
          WHERE human_hash = $1 AND deleted_at IS NULL"
    } else {
        "SELECT id,
                COALESCE(NULLIF(BTRIM(full_path), ''), NULLIF(BTRIM(name), ''), ''),
                COALESCE(stats_enabled, false),
                COALESCE(notes_with_stats_count, 0),
                COALESCE(retention, 0.0),
                NULL::integer AS stats_learner_count
           FROM anki.decks
          WHERE human_hash = $1 AND deleted_at IS NULL"
    }
}

pub async fn get_deck_stats_shell(
    db_state: &database::AppState,
    deck_hash: &str,
) -> crate::Return<Option<DeckStatsShell>> {
    let client = database::client(db_state).await?;
    let row = match client
        .query_opt(deck_stats_shell_sql(true), &[&deck_hash])
        .await
    {
        Ok(row) => row,
        Err(err) if is_undefined_schema_error(&err) => {
            tracing::warn!(
                error = %err,
                "decks.stats_learner_count missing; falling back to legacy shell metadata query"
            );
            client
                .query_opt(deck_stats_shell_sql(false), &[&deck_hash])
                .await?
        }
        Err(err) => return Err(err.into()),
    };

    Ok(row.map(|row| {
        let deck_path: String = row.get(1);
        DeckStatsShell {
            deck_id: row.get(0),
            deck_name: crate::contribution_manager::deck_display_name_from_path(&deck_path),
            stats_enabled: row.get(2),
            note_count: row.get(3),
            retention_avg: row.get(4),
            stats_learner_count: row.get(5),
        }
    }))
}

pub async fn get_total_reviewed_notes(
    db_state: &database::AppState,
    deck_hash: &str,
) -> crate::Return<i64> {
    let client = database::client(db_state).await?;
    let row = client
        .query_one(
            "WITH RECURSIVE cte AS (
                SELECT id
                FROM anki.decks
                WHERE human_hash = $1 AND deleted_at IS NULL
                UNION ALL
                SELECT d.id
                FROM cte JOIN anki.decks d ON d.parent = cte.id
                WHERE d.deleted_at IS NULL
            )
            SELECT COUNT(*)::bigint
            FROM anki.notes n
            WHERE n.deck IN (SELECT id FROM cte)
              AND NOT n.deleted
              AND n.reviewed",
            &[&deck_hash],
        )
        .await?;
    Ok(row.get(0))
}

const fn base_and_distribution_sql() -> &'static str {
    "WITH RECURSIVE cte AS (
        SELECT id
        FROM anki.decks
        WHERE human_hash = $1 AND deleted_at IS NULL
        UNION ALL
        SELECT d.id
        FROM cte JOIN anki.decks d ON d.parent = cte.id
        WHERE d.deleted_at IS NULL
    ),
    active_notes AS (
        SELECT n.reviewed,
               n.last_update,
               cs.lapses,
               cs.reps,
               cs.retention,
               cs.sample_size
        FROM anki.notes n
        LEFT JOIN anki.calculated_stats cs ON cs.note_id = n.id
        WHERE n.deck IN (SELECT id FROM cte)
          AND NOT n.deleted
    )
    SELECT
        COUNT(*) FILTER (WHERE reviewed)::bigint,
        COALESCE(AVG(lapses), 0)::float8,
        COALESCE(AVG(reps), 0)::float8,
        COALESCE(PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY sample_size), 0)::float8,
        COUNT(*) FILTER (WHERE sample_size >= 5 AND (retention < 70.0 OR lapses >= 4.0))::bigint,
        COUNT(*) FILTER (WHERE sample_size >= 5 AND (retention < 70.0 OR lapses >= 4.0) AND last_update >= now() - INTERVAL '60 days')::bigint,
        COUNT(*) FILTER (WHERE retention IS NOT NULL AND retention < 60)::bigint,
        COUNT(*) FILTER (WHERE retention IS NOT NULL AND retention < 60 AND sample_size >= 5)::bigint,
        COUNT(*) FILTER (WHERE retention IS NOT NULL AND retention >= 60 AND retention < 70)::bigint,
        COUNT(*) FILTER (WHERE retention IS NOT NULL AND retention >= 60 AND retention < 70 AND sample_size >= 5)::bigint,
        COUNT(*) FILTER (WHERE retention IS NOT NULL AND retention >= 70 AND retention < 80)::bigint,
        COUNT(*) FILTER (WHERE retention IS NOT NULL AND retention >= 70 AND retention < 80 AND sample_size >= 5)::bigint,
        COUNT(*) FILTER (WHERE retention IS NOT NULL AND retention >= 80 AND retention < 85)::bigint,
        COUNT(*) FILTER (WHERE retention IS NOT NULL AND retention >= 80 AND retention < 85 AND sample_size >= 5)::bigint,
        COUNT(*) FILTER (WHERE retention IS NOT NULL AND retention >= 85 AND retention < 90)::bigint,
        COUNT(*) FILTER (WHERE retention IS NOT NULL AND retention >= 85 AND retention < 90 AND sample_size >= 5)::bigint,
        COUNT(*) FILTER (WHERE retention IS NOT NULL AND retention >= 90 AND retention < 95)::bigint,
        COUNT(*) FILTER (WHERE retention IS NOT NULL AND retention >= 90 AND retention < 95 AND sample_size >= 5)::bigint,
        COUNT(*) FILTER (WHERE retention IS NOT NULL AND retention >= 95)::bigint,
        COUNT(*) FILTER (WHERE retention IS NOT NULL AND retention >= 95 AND sample_size >= 5)::bigint
    FROM active_notes"
}

fn retention_buckets_from_row(row: &tokio_postgres::Row) -> Vec<RetentionBucket> {
    RETENTION_BUCKET_SPECS
        .into_iter()
        .enumerate()
        .map(|(idx, (label, range_key, tone))| {
            let all_idx = BASE_AND_DIST_COLS + idx * 2;
            let conf_idx = all_idx + 1;
            RetentionBucket {
                label: label.to_string(),
                range_key: range_key.to_string(),
                all_count: row.get(all_idx),
                confident_count: row.get(conf_idx),
                tone: tone.to_string(),
            }
        })
        .collect()
}

pub async fn get_base_and_distribution(
    db_state: &database::AppState,
    shell: &DeckStatsShell,
    deck_hash: &str,
) -> Result<(DeckBaseStatsInfo, Vec<RetentionBucket>), DynError> {
    let client = database::client(db_state).await?;
    let row = client
        .query_one(base_and_distribution_sql(), &[&deck_hash])
        .await?;

    let total_reviewed_notes: i64 = row.get(0);
    let lapses_avg: f64 = row.get(1);
    let reps_avg: f64 = row.get(2);
    let median_sample_size: f64 = row.get(3);
    let leech_count: i64 = row.get(4);
    let recently_revised_struggling: i64 = row.get(5);
    let buckets = retention_buckets_from_row(&row);
    drop(client);

    let contributor_count = match shell.stats_learner_count {
        Some(count) => i64::from(count.max(0)),
        None => get_contributor_count(db_state, deck_hash).await?,
    };
    let coverage_pct = coverage_percentage(i64::from(shell.note_count), total_reviewed_notes);

    let base = DeckBaseStatsInfo {
        note_count: shell.note_count,
        total_reviewed_notes,
        coverage_pct,
        lapses_avg,
        reps_avg,
        retention_avg: shell.retention_avg,
        contributor_count,
        median_sample_size,
        leech_count,
        recently_revised_struggling,
        trend_delta_30d: None,
    };

    Ok((base, buckets))
}

pub async fn get_deck_stat_info(
    db_state: &database::AppState,
    deck_hash: &str,
) -> Result<Vec<DeckStatsInfo>, DynError> {
    let client = database::client(db_state).await?;
    let query = "
        WITH RECURSIVE subtree AS (
            SELECT id, human_hash, parent,
                   COALESCE(NULLIF(BTRIM(full_path), ''), NULLIF(BTRIM(name), ''), '[Unnamed Deck]') AS path
            FROM anki.decks
            WHERE human_hash = $1 AND deleted_at IS NULL
            UNION ALL
            SELECT d.id, d.human_hash, d.parent,
                   COALESCE(NULLIF(BTRIM(d.full_path), ''), NULLIF(BTRIM(d.name), ''), '[Unnamed Deck]')
            FROM anki.decks d
            JOIN subtree s ON d.parent = s.id
            WHERE d.deleted_at IS NULL
        ),
        descendants AS (
            SELECT id AS ancestor_id, id AS descendant_id FROM subtree
            UNION
            SELECT des.ancestor_id, s.id
            FROM descendants des
            JOIN subtree s ON s.parent = des.descendant_id
        ),
        direct_notes AS (
            SELECT n.deck AS deck_id,
                   COUNT(*) FILTER (WHERE n.reviewed)::bigint AS total_notes,
                   COUNT(cs.note_id)::bigint AS tracked_notes,
                   COALESCE(SUM(cs.retention) FILTER (WHERE cs.retention IS NOT NULL), 0)::float8 AS sum_retention,
                   COUNT(cs.retention)::bigint AS cnt_retention,
                   COALESCE(SUM(cs.lapses) FILTER (WHERE cs.note_id IS NOT NULL), 0)::float8 AS sum_lapses,
                   COALESCE(SUM(cs.reps) FILTER (WHERE cs.note_id IS NOT NULL), 0)::float8 AS sum_reps,
                   COUNT(*) FILTER (WHERE cs.sample_size >= 5 AND (cs.retention < 70.0 OR cs.lapses >= 4.0))::bigint AS leech_count
            FROM anki.notes n
            LEFT JOIN anki.calculated_stats cs ON cs.note_id = n.id
            WHERE n.deck IN (SELECT id FROM subtree)
              AND NOT n.deleted
            GROUP BY n.deck
        )
        SELECT s.human_hash,
               s.path,
               CASE
                   WHEN COALESCE(SUM(dn.cnt_retention), 0) = 0 THEN NULL
                   ELSE CAST(ROUND((SUM(dn.sum_retention) / SUM(dn.cnt_retention))::numeric, 1) AS REAL)
               END AS retention,
               COALESCE(SUM(dn.tracked_notes), 0)::bigint AS tracked_notes,
               COALESCE(SUM(dn.total_notes), 0)::bigint AS total_notes,
               CASE
                   WHEN COALESCE(SUM(dn.tracked_notes), 0) = 0 THEN NULL
                   ELSE ROUND((SUM(dn.sum_lapses) / NULLIF(SUM(dn.tracked_notes), 0))::numeric, 1)::float8
               END AS lapses_avg,
               CASE
                   WHEN COALESCE(SUM(dn.tracked_notes), 0) = 0 THEN NULL
                   ELSE ROUND((SUM(dn.sum_reps) / NULLIF(SUM(dn.tracked_notes), 0))::numeric, 1)::float8
               END AS reps_avg,
               COALESCE(SUM(dn.leech_count), 0)::bigint AS leech_count
        FROM subtree s
        LEFT JOIN descendants des ON des.ancestor_id = s.id
        LEFT JOIN direct_notes dn ON dn.deck_id = des.descendant_id
        GROUP BY s.id, s.human_hash, s.path
        ORDER BY s.path ASC
    ";
    let rows = client.query(query, &[&deck_hash]).await?;

    let res = rows
        .into_iter()
        .map(|row| {
            let hash: String = row.get(0);
            let path: String = row.get(1);
            let retention: Option<f32> = row.get(2);
            let tracked_notes: i64 = row.get(3);
            let total_notes: i64 = row.get(4);
            let lapses_avg: Option<f64> = row.get(5);
            let reps_avg: Option<f64> = row.get(6);
            let leech_count: i64 = row.get(7);
            let (prefix_path, leaf_name, depth) = split_hierarchy_path(&path);
            let coverage_pct = coverage_percentage(tracked_notes, total_notes);
            DeckStatsInfo {
                hash,
                path,
                prefix_path,
                leaf_name,
                depth,
                retention,
                tracked_notes,
                total_notes,
                coverage_pct,
                lapses_avg,
                reps_avg,
                leech_count,
            }
        })
        .collect::<Vec<DeckStatsInfo>>();

    Ok(res)
}

pub async fn get_retention_trend(
    db_state: &database::AppState,
    deck_id: i64,
) -> Result<(Vec<RetentionTrendPoint>, Option<f32>), DynError> {
    let client = database::client(db_state).await?;
    let rows = client
        .query(
            "SELECT captured_on, avg_retention, avg_lapses, tracked_notes
             FROM anki.deck_retention_daily
             WHERE deck_id = $1
               AND captured_on >= CURRENT_DATE - INTERVAL '180 days'
             ORDER BY captured_on ASC",
            &[&deck_id],
        )
        .await?;

    let mut date_pairs: Vec<(Date, f32)> = Vec::with_capacity(rows.len());
    let mut points: Vec<RetentionTrendPoint> = Vec::with_capacity(rows.len());

    for row in rows {
        let on: Date = row.get(0);
        let avg_retention: f32 = row.get(1);
        let avg_lapses: f32 = row.get(2);
        let tracked_notes: i32 = row.get(3);
        date_pairs.push((on, avg_retention));
        points.push(RetentionTrendPoint {
            captured_on: on.to_string(),
            avg_retention,
            avg_lapses,
            tracked_notes,
        });
    }

    let delta = compute_trend_delta_30d(&date_pairs);
    Ok((points, delta))
}

pub async fn get_tag_stats(
    db_state: &database::AppState,
    deck_hash: &str,
) -> Result<Vec<TagStatsInfo>, DynError> {
    let client = database::client(db_state).await?;
    let raw_tag_rows = client
        .query(
            "WITH RECURSIVE cte AS (
                SELECT id FROM anki.decks WHERE human_hash = $1 AND deleted_at IS NULL
                UNION ALL
                SELECT d.id FROM anki.decks d JOIN cte ON d.parent = cte.id WHERE d.deleted_at IS NULL
            ),
            sample_tags AS (
                SELECT BTRIM(t.content) AS tag
                FROM anki.notes n
                JOIN anki.calculated_stats cs ON cs.note_id = n.id
                JOIN anki.tags t ON t.note = n.id
                WHERE n.deck IN (SELECT id FROM cte)
                  AND NOT n.deleted
                  AND cs.retention IS NOT NULL
                  AND t.reviewed = true
                  AND t.content IS NOT NULL
                  AND BTRIM(t.content) <> ''
                LIMIT 2000
            )
            SELECT DISTINCT tag FROM sample_tags",
            &[&deck_hash],
        )
        .await?;

    let raw_tags: Vec<String> = raw_tag_rows
        .into_iter()
        .map(|row| row.get::<_, String>(0))
        .collect();
    if raw_tags.is_empty() {
        return Ok(Vec::new());
    }

    let plan = plan_tag_rollup(&raw_tags);
    let rollup_depth_i32 = plan.rollup_depth.and_then(|d| i32::try_from(d).ok());

    let query = "
        WITH RECURSIVE cte AS (
            SELECT id FROM anki.decks WHERE human_hash = $1 AND deleted_at IS NULL
            UNION ALL
            SELECT d.id FROM anki.decks d JOIN cte ON d.parent = cte.id WHERE d.deleted_at IS NULL
        ),
        tracked_notes AS (
            SELECT n.id, cs.retention, cs.lapses, cs.reps, cs.sample_size
            FROM anki.notes n
            JOIN anki.calculated_stats cs ON cs.note_id = n.id
            WHERE n.deck IN (SELECT id FROM cte)
              AND NOT n.deleted
              AND cs.retention IS NOT NULL
        ),
        note_topics AS (
            SELECT DISTINCT
                tn.id AS note_id,
                CASE
                    WHEN $2::int IS NULL THEN BTRIM(t.content)
                    ELSE array_to_string((string_to_array(BTRIM(t.content), '::'))[1:$2::int], '::')
                END AS topic_key,
                tn.retention,
                tn.lapses,
                tn.reps,
                tn.sample_size
            FROM tracked_notes tn
            JOIN anki.tags t ON t.note = tn.id
            WHERE t.reviewed = true
              AND t.content IS NOT NULL
              AND BTRIM(t.content) <> ''
              AND LOWER(SPLIT_PART(BTRIM(t.content), '::', 1)) NOT IN (
                  'leech', 'marked', 'suspended', 'buried', 'flag',
                  '#leech', '#marked', '#suspended', '#buried', '#flag'
              )
              AND (
                  (
                      LOWER(t.content) NOT LIKE '%ankicollab%'
                      AND LOWER(t.content) NOT LIKE '%anki_collab%'
                      AND LOWER(t.content) NOT LIKE '%crowdanki%'
                  )
                  OR BTRIM(t.content) !~* '(^|::)#?(ankicollab|anki_collab|crowdanki)'
              )
        )
        SELECT topic_key,
               COUNT(*)::bigint AS note_count,
               ROUND(AVG(retention)::numeric, 1)::float8 AS avg_retention,
               ROUND(AVG(lapses)::numeric, 1)::float8 AS avg_lapses,
               ROUND(AVG(reps)::numeric, 1)::float8 AS avg_reps,
               COUNT(*) FILTER (WHERE sample_size >= 5 AND (retention < 70.0 OR lapses >= 4.0))::bigint AS leech_count
        FROM note_topics
        GROUP BY topic_key
        HAVING COUNT(*) >= $3::bigint AND MAX(sample_size) >= $4::int
        ORDER BY avg_retention ASC, avg_lapses DESC, note_count DESC
        LIMIT 50
    ";

    let min_sample_i32 = i32::try_from(MIN_LEARNER_SAMPLE).unwrap_or(5);
    let rows = client
        .query(
            query,
            &[
                &deck_hash,
                &rollup_depth_i32,
                &MIN_TAG_NOTES,
                &min_sample_i32,
            ],
        )
        .await?;

    let mut seen: HashSet<String> = HashSet::new();
    let mut results = Vec::with_capacity(rows.len());
    for row in rows {
        let tag: String = row.get(0);
        if is_housekeeping_tag(&tag) || !seen.insert(tag.clone()) {
            continue;
        }
        let (prefix, leaf, _) = split_hierarchy_path(&tag);
        results.push(TagStatsInfo {
            tag,
            prefix,
            leaf,
            note_count: row.get(1),
            avg_retention: row.get(2),
            avg_lapses: row.get(3),
            avg_reps: row.get(4),
            leech_count: row.get(5),
        });
    }

    Ok(results)
}

pub async fn get_notetype_stats(
    db_state: &database::AppState,
    deck_hash: &str,
) -> Result<Vec<NotetypeStatsInfo>, DynError> {
    let client = database::client(db_state).await?;
    let min_sample_i32 = i32::try_from(MIN_LEARNER_SAMPLE).unwrap_or(5);
    let query = "
        WITH RECURSIVE cte AS (
            SELECT id FROM anki.decks WHERE human_hash = $1 AND deleted_at IS NULL
            UNION ALL
            SELECT d.id FROM anki.decks d JOIN cte ON d.parent = cte.id WHERE d.deleted_at IS NULL
        )
        SELECT COALESCE(NULLIF(BTRIM(nt.name), ''), 'Unnamed Notetype') AS name,
               COUNT(*)::bigint AS note_count,
               ROUND(AVG(cs.retention)::numeric, 1)::float8 AS avg_retention,
               ROUND(AVG(cs.lapses)::numeric, 1)::float8 AS avg_lapses,
               ROUND(AVG(cs.reps)::numeric, 1)::float8 AS avg_reps
        FROM anki.notes n
        JOIN anki.calculated_stats cs ON cs.note_id = n.id
        JOIN anki.notetype nt ON nt.id = n.notetype
        WHERE n.deck IN (SELECT id FROM cte)
          AND NOT n.deleted
          AND cs.retention IS NOT NULL
        GROUP BY nt.id, nt.name
        HAVING COUNT(*) >= $2::bigint AND MAX(cs.sample_size) >= $3::int
        ORDER BY avg_retention ASC, note_count DESC
    ";
    let rows = client
        .query(query, &[&deck_hash, &MIN_TAG_NOTES, &min_sample_i32])
        .await?;

    Ok(rows
        .into_iter()
        .map(|row| NotetypeStatsInfo {
            name: row.get(0),
            note_count: row.get(1),
            avg_retention: row.get(2),
            avg_lapses: row.get(3),
            avg_reps: row.get(4),
        })
        .collect())
}

fn worst_notes_sql(include_stddev_col: bool) -> String {
    let stddev_col = if include_stddev_col {
        "cs.retention_stddev"
    } else {
        "NULL::real AS retention_stddev"
    };
    format!(
        "
        WITH RECURSIVE cte AS (
            SELECT id, human_hash, parent, full_path
            FROM decks
            WHERE human_hash = $1 AND deleted_at IS NULL
            UNION ALL
            SELECT d.id, d.human_hash, d.parent, d.full_path
            FROM cte JOIN decks d ON d.parent = cte.id
            WHERE d.deleted_at IS NULL
        ), candidates AS (
            SELECT n.id, n.deck AS deck_id, n.last_update,
                   cs.lapses, cs.reps, cs.retention, cs.sample_size, {stddev_col}
            FROM notes n
            JOIN calculated_stats cs ON n.id = cs.note_id
            WHERE n.deck IN (SELECT id FROM cte) AND NOT n.deleted
              AND cs.retention IS NOT NULL
        ), pool_stats AS (
            SELECT
                AVG(retention) AS c,
                PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY sample_size) AS m
            FROM candidates
        ), worst_notes AS (
            SELECT
                c.id, c.deck_id, c.last_update,
                c.lapses, c.reps, c.retention, c.sample_size, c.retention_stddev,
                (c.sample_size::float8 / (c.sample_size + GREATEST(s.m, 1)))
                    * c.retention
                +
                (GREATEST(s.m, 1) / (c.sample_size + GREATEST(s.m, 1)))
                    * s.c
                AS weighted_retention
            FROM candidates c
            CROSS JOIN pool_stats s
            ORDER BY weighted_retention ASC, c.lapses DESC
            LIMIT 100
        )
        SELECT wn.id,
            (SELECT coalesce(f.content, '')
             FROM fields AS f
             WHERE f.note = wn.id AND BTRIM(COALESCE(f.content, '')) <> ''
             ORDER BY f.reviewed DESC, f.position ASC
             LIMIT 1) AS content,
            wn.lapses, wn.reps, wn.retention, wn.sample_size,
            CASE
                WHEN wn.sample_size >= 5 THEN COALESCE(
                    wn.retention_stddev,
                    (SELECT CAST(ROUND(STDDEV_POP(ns.retention)::numeric, 1) AS REAL)
                     FROM anki.note_stats ns
                     WHERE ns.note_id = wn.id)
                )
                ELSE NULL
            END AS retention_stddev,
            COALESCE(NULLIF(BTRIM(d.full_path), ''), NULLIF(BTRIM(d.name), ''), '[Unnamed Deck]') AS subdeck_path,
            (
                EXISTS (SELECT 1 FROM anki.fields f WHERE f.note = wn.id AND f.reviewed = false)
                OR EXISTS (SELECT 1 FROM anki.tags t WHERE t.note = wn.id AND t.reviewed = false)
                OR EXISTS (SELECT 1 FROM anki.card_deletion_suggestions cds WHERE cds.note = wn.id)
                OR EXISTS (SELECT 1 FROM anki.note_move_suggestions nms WHERE nms.note = wn.id)
            ) AS has_pending_suggestion,
            TO_CHAR(wn.last_update, 'YYYY-MM-DD') AS last_updated,
            CASE
                WHEN wn.last_update IS NULL THEN NULL
                ELSE GREATEST(0, FLOOR(EXTRACT(EPOCH FROM (now() - wn.last_update)) / 86400))::bigint
            END AS days_since_update
        FROM worst_notes wn
        JOIN anki.decks d ON d.id = wn.deck_id
        ORDER BY wn.weighted_retention ASC, wn.lapses DESC
        "
    )
}

pub async fn get_worst_notes_info(
    db_state: &database::AppState,
    deck_hash: &str,
) -> Result<Vec<NoteStatsInfo>, DynError> {
    let client = database::client(db_state).await?;
    let query = worst_notes_sql(true);
    let rows = match client.query(&query, &[&deck_hash]).await {
        Ok(rows) => rows,
        Err(err) if is_undefined_schema_error(&err) => {
            tracing::warn!(
                error = %err,
                "calculated_stats.retention_stddev missing; computing learner spread inline from anki.note_stats"
            );
            let legacy_query = worst_notes_sql(false);
            client.query(&legacy_query, &[&deck_hash]).await?
        }
        Err(err) => return Err(Box::new(err)),
    };

    let res = rows
        .into_iter()
        .map(|row| {
            let lapses: f32 = row.get(2);
            let reps: f32 = row.get(3);
            let retention: f32 = row.get(4);
            let sample_size: i32 = row.get(5);
            let retention_stddev: Option<f32> = row.get(6);
            let subdeck_path: String = row.get(7);
            let (subdeck_prefix, subdeck_leaf, _) = split_hierarchy_path(&subdeck_path);
            let has_pending_suggestion: bool = row.get(8);
            let last_updated: Option<String> = row.get(9);
            let days_since_update: Option<i64> = row.get(10);
            let revised_recently =
                days_since_update.is_some_and(|days| days <= RECENT_REVISION_DAYS);
            let low_confidence = i64::from(sample_size) < MIN_LEARNER_SAMPLE;
            let is_leech = !low_confidence && (retention < 70.0 || lapses >= 4.0);

            NoteStatsInfo {
                id: row.get(0),
                fields: row.get::<usize, Option<String>>(1).unwrap_or_default(),
                lapses,
                reps,
                retention,
                sample_size,
                retention_stddev,
                subdeck_path,
                subdeck_prefix,
                subdeck_leaf,
                has_pending_suggestion,
                last_updated,
                days_since_update,
                revised_recently,
                is_leech,
            }
        })
        .collect::<Vec<NoteStatsInfo>>();

    Ok(res)
}

pub async fn toggle_stats(db_state: &database::AppState, deck_id: i64) -> Result<(), DynError> {
    let client = database::client(db_state).await?;
    let query = "
        UPDATE decks
        SET stats_enabled = NOT stats_enabled,
            stats_learner_count = NULL
        WHERE id = $1
    ";
    if let Err(err) = client.execute(query, &[&deck_id]).await {
        if is_undefined_schema_error(&err) {
            client
                .execute(
                    "UPDATE decks SET stats_enabled = NOT stats_enabled WHERE id = $1",
                    &[&deck_id],
                )
                .await?;
        } else {
            return Err(Box::new(err));
        }
    }
    db_state.stats_cache.invalidate_deck(deck_id);
    Ok(())
}

pub async fn get_contributor_count(
    db_state: &database::AppState,
    deck_hash: &str,
) -> Result<i64, DynError> {
    let client = database::client(db_state).await?;
    let query = "
        WITH RECURSIVE cte AS (
            SELECT id
            FROM decks
            WHERE human_hash = $1 AND deleted_at IS NULL
            UNION ALL
            SELECT d.id
            FROM cte JOIN decks d ON d.parent = cte.id
            WHERE d.deleted_at IS NULL
        )
        SELECT COUNT(DISTINCT ns.user_hash)
        FROM cte
        JOIN notes n ON cte.id = n.deck AND NOT n.deleted
        JOIN note_stats ns ON ns.note_id = n.id
    ";

    let row = client.query_one(query, &[&deck_hash]).await?;
    let contributor_count: i64 = row.get(0);

    Ok(contributor_count)
}

/// Execute the statistics queries in small batches (max 2 pooled DB connections at once)
/// so heavy decks never exhaust the `bb8` pool while still cutting sequential latency.
pub async fn load_deck_stats_payload(
    db_state: &database::AppState,
    shell: &DeckStatsShell,
    deck_hash: &str,
) -> Result<DeckStatsPayload, String> {
    let (base_dist_res, deck_info_res) = tokio::join!(
        get_base_and_distribution(db_state, shell, deck_hash),
        get_deck_stat_info(db_state, deck_hash),
    );
    let (mut base, retention_buckets) = base_dist_res.map_err(|err| {
        tracing::warn!(error = %err, "Failed to get base deck info for statistics");
        err.to_string()
    })?;
    let decks = deck_info_res.map_err(|err| {
        tracing::warn!(error = %err, "Failed to get deck stat info");
        err.to_string()
    })?;

    let (notes_res, tags_res) = tokio::join!(
        get_worst_notes_info(db_state, deck_hash),
        get_tag_stats(db_state, deck_hash),
    );
    let notes = notes_res.map_err(|err| {
        tracing::warn!(error = %err, "Failed to get worst notes info");
        err.to_string()
    })?;
    let tag_stats = match tags_res {
        Ok(tags) => tags,
        Err(err) => {
            tracing::warn!(error = %err, "Failed to get tag stats");
            Vec::new()
        }
    };

    let (trend_res, notetypes_res) = tokio::join!(
        get_retention_trend(db_state, shell.deck_id),
        get_notetype_stats(db_state, deck_hash),
    );
    let (retention_trend, trend_delta_30d) = match trend_res {
        Ok(trend) => trend,
        Err(err) => {
            tracing::warn!(error = %err, "Failed to get retention trend");
            (Vec::new(), None)
        }
    };
    base.trend_delta_30d = trend_delta_30d;

    let notetype_stats = match notetypes_res {
        Ok(notetypes) => notetypes,
        Err(err) => {
            tracing::warn!(error = %err, "Failed to get notetype stats");
            Vec::new()
        }
    };

    Ok(DeckStatsPayload {
        base,
        decks,
        notes,
        retention_buckets,
        retention_trend,
        tag_stats,
        notetype_stats,
    })
}

pub async fn get_or_load_deck_stats(
    db_state: &database::AppState,
    shell: &DeckStatsShell,
    deck_hash: &str,
) -> Result<Arc<DeckStatsPayload>, String> {
    db_state
        .stats_cache
        .get_or_load(shell.deck_id, || {
            load_deck_stats_payload(db_state, shell, deck_hash)
        })
        .await
}

pub fn warm_deck_stats_in_background(
    db_state: Arc<database::AppState>,
    shell: DeckStatsShell,
    deck_hash: String,
) {
    tokio::spawn(async move {
        if let Err(err) = get_or_load_deck_stats(&db_state, &shell, &deck_hash).await {
            tracing::warn!(
                deck_id = shell.deck_id,
                error = %err,
                "Background deck statistics warmup failed"
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Month;

    #[test]
    fn housekeeping_tags_are_filtered_out() {
        for tag in [
            "",
            "   ",
            "leech",
            "Leech",
            "leech::auto",
            "marked",
            "Marked",
            "suspended",
            "buried",
            "flag",
            "AnkiCollab_Optional::HighYield::Card",
            "AnkiCollab_Personal",
            "AnkiCollab_Personal::Mine",
            "AnkiCollab_Protect::Keep",
            "#AnkiCollab_Optional::Extra",
            "anki_collab::internal",
            "crowdanki_note",
        ] {
            assert!(is_housekeeping_tag(tag), "expected housekeeping: {tag:?}");
        }
    }

    #[test]
    fn subject_tags_are_preserved() {
        for tag in [
            "Pharmacology",
            "Biology::Annelida::Leech",
            "German::B2::Vocabulary",
            "Neuroanatomy::Marked_Tracts",
        ] {
            assert!(!is_housekeeping_tag(tag), "expected subject tag: {tag:?}");
        }
    }

    #[test]
    fn shallow_study_group_tags_stay_unrolled() {
        let tags = vec![
            "Week1".to_string(),
            "Week2".to_string(),
            "Exam::Midterm".to_string(),
            "Lecture::Slides".to_string(),
            "leech".to_string(),
            "AnkiCollab_Optional::Bonus::Card".to_string(),
        ];
        let plan = plan_tag_rollup(&tags);
        assert_eq!(plan.rollup_depth, None);
        assert_eq!(plan.shared_root, None);
    }

    #[test]
    fn deeply_nested_single_root_rolls_up_to_depth_three() {
        let tags = vec![
            "#Malleus::Cardiology::Arrhythmias::AFib".to_string(),
            "#Malleus::Cardiology::HeartFailure::HFrEF".to_string(),
            "#Malleus::Renal::Glomerular::Nephritic".to_string(),
            "#Malleus::Pharmacology::Antibiotics::BetaLactams".to_string(),
            "AnkiCollab_Optional::Extra::Note".to_string(),
        ];
        let plan = plan_tag_rollup(&tags);
        assert_eq!(plan.rollup_depth, Some(3));
        assert_eq!(plan.shared_root.as_deref(), Some("#Malleus"));
    }

    #[test]
    fn deeply_nested_multi_root_rolls_up_to_depth_two() {
        let tags = vec![
            "Anatomy::UpperLimb::BrachialPlexus::Roots".to_string(),
            "Physiology::Renal::Filtration::GFR".to_string(),
            "Biochem::Metabolism::Glycolysis::Enzymes".to_string(),
            "Pathology::GI::Liver::Cirrhosis".to_string(),
        ];
        let plan = plan_tag_rollup(&tags);
        assert_eq!(plan.rollup_depth, Some(2));
        assert_eq!(plan.shared_root, None);
    }

    #[test]
    fn split_hierarchy_preserves_leaf_and_depth() {
        assert_eq!(
            split_hierarchy_path("Med::Anatomy::Upper Limb"),
            ("Med::Anatomy::".to_string(), "Upper Limb".to_string(), 2)
        );
        assert_eq!(
            split_hierarchy_path("Genetics"),
            (String::new(), "Genetics".to_string(), 0)
        );
        assert_eq!(
            split_hierarchy_path("   "),
            (String::new(), "[Unnamed]".to_string(), 0)
        );
    }

    #[test]
    fn trend_delta_requires_at_least_seven_day_span() {
        let d1 = Date::from_calendar_date(2026, Month::October, 1).unwrap();
        let d2 = Date::from_calendar_date(2026, Month::October, 5).unwrap();
        assert_eq!(compute_trend_delta_30d(&[(d1, 82.0)]), None);
        assert_eq!(compute_trend_delta_30d(&[(d1, 82.0), (d2, 84.0)]), None);

        let d3 = Date::from_calendar_date(2026, Month::October, 31).unwrap();
        let d4 = Date::from_calendar_date(2026, Month::November, 15).unwrap();
        let delta = compute_trend_delta_30d(&[(d1, 78.4), (d3, 81.2), (d4, 83.7)]);
        assert_eq!(delta, Some(5.3));
    }

    #[test]
    fn coverage_percentage_handles_zero_and_bounds() {
        assert!((coverage_percentage(0, 100) - 0.0).abs() < f64::EPSILON);
        assert!((coverage_percentage(10, 0) - 0.0).abs() < f64::EPSILON);
        assert!((coverage_percentage(120, 100) - 100.0).abs() < f64::EPSILON);
        assert!((coverage_percentage(25, 100) - 25.0).abs() < f64::EPSILON);
    }

    #[test]
    fn calculate_note_stats_sql_supports_pre_and_post_migration_schemas() {
        let with_col = calculate_note_stats_sql(true);
        assert!(with_col.contains("retention_stddev"));
        assert!(with_col.contains("retention_stddev = EXCLUDED.retention_stddev"));

        let legacy = calculate_note_stats_sql(false);
        assert!(!legacy.contains("retention_stddev"));
        assert!(legacy.contains(
            "INSERT INTO calculated_stats (note_id, sample_size, retention, lapses, reps)"
        ));
    }

    #[test]
    fn worst_notes_sql_falls_back_when_stddev_column_is_absent() {
        let with_col = worst_notes_sql(true);
        assert!(with_col.contains("cs.retention_stddev"));
        assert!(with_col.contains("ORDER BY f.reviewed DESC, f.position ASC"));
        assert!(with_col.contains("FROM anki.note_stats ns"));

        let legacy = worst_notes_sql(false);
        assert!(!legacy.contains("cs.retention_stddev"));
        assert!(legacy.contains("NULL::real AS retention_stddev"));
        assert!(legacy.contains("FROM anki.note_stats ns"));
    }

    #[test]
    fn snapshot_deck_retention_sql_supports_with_and_without_learner_count() {
        let with_update = snapshot_deck_retention_sql(true);
        assert!(with_update.contains("updated_learner_counts"));
        assert!(with_update.contains("SET stats_learner_count = asl.learner_count::int"));
        assert!(with_update.contains("WHERE asl.learner_count >= $1::bigint"));

        let legacy = snapshot_deck_retention_sql(false);
        assert!(!legacy.contains("updated_learner_counts"));
        assert!(!legacy.contains("SET stats_learner_count"));
        assert!(legacy.contains("WHERE asl.learner_count >= $1::bigint"));
    }

    #[test]
    fn shell_and_merged_base_distribution_sql_have_expected_shapes() {
        assert!(deck_stats_shell_sql(true).contains("stats_learner_count"));
        assert!(deck_stats_shell_sql(false).contains("NULL::integer AS stats_learner_count"));

        let merged = base_and_distribution_sql();
        assert!(merged.contains("COUNT(*) FILTER (WHERE reviewed)::bigint"));
        assert!(merged.contains("PERCENTILE_CONT(0.5)"));
        assert!(merged.contains("retention < 60 AND sample_size >= 5"));
        assert!(merged.contains("retention >= 95 AND sample_size >= 5"));
    }
}
