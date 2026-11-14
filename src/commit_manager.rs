use crate::cleanser;
use crate::database;
use crate::error::Error::NoNotesAffected;
use crate::structs::{
    CommitData, CommitNotesPage, CommitsOverview, FieldsInfo, FieldsReviewInfo, NoteMoveReq,
    TagsInfo,
};
use crate::Return;

use std::cmp::min;
use std::collections::{HashMap, HashSet};
extern crate htmldiff;

#[must_use]
pub const fn get_string_from_rationale(input: i32) -> &'static str {
    match input {
        0 => "None",
        1 => "Deck Creation",
        2 => "Updated content",
        3 => "New content",
        4 => "Content error",
        5 => "Spelling/Grammar",
        6 => "New card",
        7 => "Updated Tags",
        8 => "New Tags",
        9 => "Bulk Suggestion",
        10 => "Other",
        11 => "Card Deletion",
        12 => "Changed Deck",
        _ => "Unknown Rationale",
    }
}

fn deck_leaf(deck_path: &str) -> String {
    deck_path
        .rsplit("::")
        .next()
        .unwrap_or(deck_path)
        .to_string()
}

pub async fn get_commit_info(
    db_state: &database::AppState,
    commit_id: i32,
) -> Return<CommitsOverview> {
    let query = r"    
        SELECT c.commit_id, c.rationale, c.info,
        TO_CHAR(c.timestamp, 'MM/DD/YYYY HH24:MI:SS') AS last_update,
        d.name,
        COALESCE(u.username, 'Unknown') as username
        FROM commits c
        JOIN decks d on d.id = c.deck
        LEFT JOIN users u on u.id = c.user_id
        WHERE c.commit_id = $1
    ";
    let client = database::client(db_state).await?;
    let row = client.query_one(query, &[&commit_id]).await?;
    let commit = CommitsOverview {
        id: row.get(0),
        rationale: get_string_from_rationale(row.get(1)).into(),
        commit_info: row.get(2),
        timestamp: row.get(3),
        deck: row.get(4),
        user: row.get(5),
    };
    Ok(commit)
}

fn find_common_prefix(paths: &[&str]) -> String {
    if paths.is_empty() {
        return String::new();
    }

    let mut prefix_parts = paths[0].split("::").collect::<Vec<_>>();

    for path in paths.iter().skip(1) {
        let parts = path.split("::").collect::<Vec<_>>();
        let mut i = 0;
        while i < min(prefix_parts.len(), parts.len()) && prefix_parts[i] == parts[i] {
            i += 1;
        }
        prefix_parts.truncate(i);
    }

    prefix_parts.join("::")
}

pub async fn commits_review(
    db_state: &database::AppState,
    uid: i32,
) -> Result<Vec<CommitsOverview>, Box<dyn std::error::Error>> {
    let client = database::client(db_state).await?;

    let best_query = r#"
        WITH RECURSIVE accessible AS MATERIALIZED (
            SELECT id FROM decks
            WHERE id IN (
                SELECT deck FROM maintainers WHERE user_id = $1
                UNION
                SELECT id FROM decks WHERE owner = $1
            )
            UNION ALL
            SELECT d.id
            FROM decks d
            JOIN accessible a ON d.parent = a.id
        ),

        relevant_commits AS MATERIALIZED (
            SELECT c.commit_id
            FROM commits c
            WHERE c.deck IN (SELECT id FROM accessible)
              AND (
                  EXISTS (SELECT 1 FROM fields f WHERE f.commit = c.commit_id AND f.reviewed = false)
                  OR EXISTS (SELECT 1 FROM tags t WHERE t.commit = c.commit_id AND t.reviewed = false)
                  OR EXISTS (SELECT 1 FROM card_deletion_suggestions cds WHERE cds.commit = c.commit_id)
                  OR EXISTS (SELECT 1 FROM note_move_suggestions nms WHERE nms.commit = c.commit_id)
              )
        ),

        distinct_decks AS (
            SELECT DISTINCT src.commit, n.deck
            FROM (
                SELECT DISTINCT f.commit, f.note
                FROM fields f
                WHERE f.reviewed = false
                  AND f.commit IN (SELECT commit_id FROM relevant_commits)
                UNION
                SELECT DISTINCT t.commit, t.note
                FROM tags t
                WHERE t.reviewed = false
                  AND t.commit IN (SELECT commit_id FROM relevant_commits)
                UNION
                SELECT DISTINCT cds.commit, cds.note
                FROM card_deletion_suggestions cds
                WHERE cds.commit IN (SELECT commit_id FROM relevant_commits)
                UNION
                SELECT DISTINCT nms.commit, nms.note
                FROM note_move_suggestions nms
                WHERE nms.commit IN (SELECT commit_id FROM relevant_commits)
            ) src
            JOIN notes n ON n.id = src.note
        ),

        deck_paths_agg AS (
            SELECT dd.commit,
                array_agg(d.full_path) AS deck_paths
            FROM distinct_decks dd
            JOIN decks d ON d.id = dd.deck
            GROUP BY dd.commit
        )

        SELECT
            c.commit_id,
            c.rationale,
            c.info,
            TO_CHAR(c."timestamp", 'MM/DD/YYYY') AS formatted_timestamp,
            dpa.deck_paths,
            COALESCE(u.username, 'Unknown') AS username
        FROM commits c
        JOIN relevant_commits rc ON c.commit_id = rc.commit_id
        LEFT JOIN users u ON u.id = c.user_id
        LEFT JOIN deck_paths_agg dpa ON dpa.commit = c.commit_id
        ORDER BY c.commit_id DESC
    "#;

    let rows = client.query(best_query, &[&uid]).await?;

    let result: Vec<CommitsOverview> = rows
        .into_iter()
        .map(|row| {
            let deck_paths_opt: Option<Vec<String>> = row.get(4);

            let deck_string = deck_paths_opt.map_or(String::new(), |paths_vec| {
                let paths_ref: Vec<&str> = paths_vec.iter().map(String::as_str).collect();
                find_common_prefix(&paths_ref)
            });

            CommitsOverview {
                id: row.get(0),
                rationale: get_string_from_rationale(row.get(1)).into(),
                commit_info: row.get(2),
                timestamp: row.get(3),
                deck: deck_string,
                user: row.get(5),
            }
        })
        .collect();

    Ok(result)
}

pub async fn get_field_diff(db_state: &database::AppState, field_id: i64) -> Return<String> {
    let client = database::client(db_state).await?;
    let new_content_row = client
        .query_one(
            "SELECT note, content, position::int AS position FROM fields WHERE id = $1",
            &[&field_id],
        )
        .await?;
    if new_content_row.is_empty() {
        return Err(NoNotesAffected);
    }
    let note_id: i64 = new_content_row.get(0);
    let new_content: String = new_content_row.get(1);
    let position: u32 = u32::try_from(new_content_row.get::<_, i32>(2)).unwrap_or(0);
    let og_content_row = client
        .query_one(
            "SELECT content FROM fields WHERE note = $1 AND position = $2 ORDER BY reviewed DESC LIMIT 1", 
            &[&note_id, &position],
        )
        .await?;
    if og_content_row.is_empty() {
        return Err(NoNotesAffected);
    }
    let current_content: String = og_content_row.get(0);

    let clean_new_content = cleanser::clean(&new_content);
    let clean_content = cleanser::clean(&current_content);
    let diff = htmldiff::htmldiff(&clean_content, &clean_new_content);
    Ok(diff)
}

/// Parses unreviewed fields JSON array from Postgres and calculates HTML diffs.
pub fn parse_unreviewed_fields_json(json: &serde_json::Value) -> Vec<FieldsReviewInfo> {
    let mut fields = Vec::new();
    if let Some(fields_array) = json.as_array() {
        for field_data in fields_array {
            let content = field_data
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let reviewed_content = field_data
                .get("reviewed_content")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let clean_content = cleanser::clean(content);
            let clean_reviewed = cleanser::clean(reviewed_content);
            let diff_string = htmldiff::htmldiff(&clean_reviewed, &clean_content);

            fields.push(FieldsReviewInfo {
                id: field_data
                    .get("id")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0),
                position: field_data
                    .get("position")
                    .and_then(serde_json::Value::as_i64)
                    .and_then(|v| u32::try_from(v).ok())
                    .unwrap_or(0),
                content: clean_content,
                reviewed_content: clean_reviewed,
                diff: diff_string,
            });
        }
    }
    fields
}

/// Parses first fields JSON array (used for deleted notes representation).
pub fn parse_first_fields_json(json: &serde_json::Value) -> Vec<FieldsReviewInfo> {
    let mut fields = Vec::new();
    if let Some(fields_array) = json.as_array() {
        for field_data in fields_array {
            let content = field_data
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let clean_content = cleanser::clean(content);
            fields.push(FieldsReviewInfo {
                id: field_data
                    .get("id")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0),
                position: field_data
                    .get("position")
                    .and_then(serde_json::Value::as_i64)
                    .and_then(|v| u32::try_from(v).ok())
                    .unwrap_or(0),
                content: clean_content.clone(),
                reviewed_content: clean_content.clone(),
                diff: clean_content,
            });
        }
    }
    fields
}

/// Parses tags changes JSON array from Postgres into (`new_tags`, `removed_tags`).
pub fn parse_tags_changes_json(
    json: &serde_json::Value,
    commit_id: i32,
) -> (Vec<TagsInfo>, Vec<TagsInfo>) {
    let mut new_tags = Vec::new();
    let mut removed_tags = Vec::new();
    if let Some(tags_array) = json.as_array() {
        for tag_data in tags_array {
            if let (Some(id), Some(content), Some(action)) = (
                tag_data.get("id").and_then(serde_json::Value::as_i64),
                tag_data.get("content").and_then(|v| v.as_str()),
                tag_data.get("action").and_then(serde_json::Value::as_bool),
            ) {
                let tag_info = TagsInfo {
                    id,
                    content: cleanser::clean(content),
                    inherited: false,
                    commit_id,
                };
                if action {
                    new_tags.push(tag_info);
                } else {
                    removed_tags.push(tag_info);
                }
            }
        }
    }
    (new_tags, removed_tags)
}

/// Parses move request JSON object from Postgres.
pub fn parse_move_req_json(json: Option<&serde_json::Value>) -> Option<NoteMoveReq> {
    let move_data = json?;
    let id = move_data.get("id").and_then(serde_json::Value::as_i64)?;
    let path = move_data.get("path").and_then(|v| v.as_str())?;
    Some(NoteMoveReq {
        id: i32::try_from(id).unwrap_or(0),
        path: path.to_string(),
    })
}

/// Determines whether a note should be included in the review / preview results.
#[must_use]
pub fn should_include_note(note: &CommitData) -> bool {
    note.delete_req
        || !note.fields.is_empty()
        || !note.new_tags.is_empty()
        || !note.removed_tags.is_empty()
        || note.move_req.is_some()
}

pub async fn notes_by_commit(
    db_state: &database::AppState,
    commit_id: i32,
    offset: i64,
    limit: i64,
) -> Return<CommitNotesPage> {
    let client = database::client(db_state).await?;

    let sanitized_offset = offset.max(0);
    let sanitized_limit = limit.clamp(1, 200);
    let upper_bound = sanitized_offset.saturating_add(sanitized_limit);

    let comprehensive_query = r"
        WITH affected_notes AS MATERIALIZED (
            SELECT note FROM (
                SELECT note FROM fields WHERE commit = $1 AND reviewed = false
                UNION ALL
                SELECT note FROM tags WHERE commit = $1 AND reviewed = false
                UNION ALL
                SELECT note FROM card_deletion_suggestions WHERE commit = $1
                UNION ALL
                SELECT note FROM note_move_suggestions WHERE commit = $1
            ) AS n
            GROUP BY note
        ),
        ordered_notes AS MATERIALIZED (
            SELECT
                note,
                ROW_NUMBER() OVER (ORDER BY note) AS rn
            FROM affected_notes
        ),
        page_notes AS (
            SELECT note, rn
            FROM ordered_notes
            WHERE rn > $2 AND rn <= $3
        ),
        note_data AS (
            SELECT 
                n.id,
                n.guid,
                TO_CHAR(n.last_update, 'MM/DD/YYYY HH12:MI AM') AS last_update,
                n.reviewed,
                d.owner,
                d.full_path,
                n.notetype,
                (cds.note IS NOT NULL) AS delete_req,
                pn.rn
            FROM notes n
            JOIN page_notes pn ON n.id = pn.note
            JOIN decks d ON d.id = n.deck
            LEFT JOIN card_deletion_suggestions cds ON cds.note = n.id AND cds.commit = $1
        ),
        fields_data AS (
            SELECT 
                f1.note,
                json_agg(json_build_object(
                    'id', f1.id,
                    'position', f1.position::int,
                    'content', f1.content,
                    'reviewed_content', COALESCE(f2.content, '')
                ) ORDER BY f1.position) AS unreviewed_fields
            FROM fields f1
            LEFT JOIN fields f2 ON f1.note = f2.note AND f1.position = f2.position AND f2.reviewed = true
            WHERE f1.reviewed = false AND f1.commit = $1 AND f1.note IN (SELECT note FROM page_notes)
            GROUP BY f1.note
        ),
        first_fields_data AS (
            WITH numbered_fields AS (
                SELECT
                    f.id,
                    f.note,
                    f.position::int AS position,
                    f.content,
                    ROW_NUMBER() OVER (PARTITION BY f.note ORDER BY f.position) AS rn
                FROM fields f
                WHERE f.note IN (SELECT note FROM page_notes)
            )
            SELECT
                nf.note,
                json_agg(json_build_object('id', nf.id, 'position', nf.position::int, 'content', nf.content) ORDER BY nf.position) AS first_fields
            FROM numbered_fields nf
            WHERE nf.rn <= 3
            GROUP BY nf.note
        ),
        tags_data AS (
            SELECT 
                t.note,
                json_agg(json_build_object('id', t.id, 'content', t.content, 'action', t.action)) AS tags_changes
            FROM tags t
            WHERE t.commit = $1 AND t.note IN (SELECT note FROM page_notes) AND t.reviewed = false
            GROUP BY t.note
        ),
        reviewed_fields_data AS (
            SELECT 
                f.note,
                json_agg(json_build_object(
                    'id', f.id,
                    'position', f.position::int,
                    'content', f.content
                ) ORDER BY f.position) AS reviewed_fields
            FROM fields f
            WHERE f.note IN (SELECT note FROM page_notes) AND f.reviewed = true
            GROUP BY f.note
        ),
        reviewed_tags_data AS (
            SELECT
                t.note,
                json_agg(json_build_object('id', t.id, 'content', t.content) ORDER BY t.content) AS reviewed_tags
            FROM tags t
            WHERE t.note IN (SELECT note FROM page_notes) AND t.reviewed = true AND t.content IS NOT NULL
            GROUP BY t.note
        ),
        inheritance_meta AS (
            SELECT 
                ni.subscriber_note_id AS note,
                ni.base_note_id,
                ni.subscribed_fields,
                COALESCE(ni.removed_base_tags, '{}') AS removed_base_tags
            FROM note_inheritance ni
            WHERE ni.subscriber_note_id IN (SELECT note FROM page_notes)
        ),
        base_reviewed_fields AS (
            SELECT 
                ni.subscriber_note_id AS note,
                json_agg(json_build_object(
                    'position', f.position::int,
                    'content', f.content
                ) ORDER BY f.position) AS base_fields
            FROM note_inheritance ni
            JOIN fields f ON f.note = ni.base_note_id AND f.reviewed = true
            WHERE ni.subscriber_note_id IN (SELECT note FROM page_notes)
            GROUP BY ni.subscriber_note_id
        ),
        base_reviewed_tags AS (
            SELECT 
                ni.subscriber_note_id AS note,
                json_agg(json_build_object('id', t.id, 'content', t.content) ORDER BY t.content) AS base_tags
            FROM note_inheritance ni
            JOIN tags t ON t.note = ni.base_note_id AND t.reviewed = true AND t.content IS NOT NULL
            WHERE ni.subscriber_note_id IN (SELECT note FROM page_notes)
            GROUP BY ni.subscriber_note_id
        ),
        move_data AS (
            SELECT 
                nms.note,
                json_build_object('id', nms.id, 'path', d.full_path) AS move_req
            FROM note_move_suggestions nms
            JOIN decks d ON d.id = nms.target_deck
            WHERE nms.note IN (SELECT note FROM page_notes) AND nms.commit = $1
        ),
        total AS (
            SELECT COUNT(*)::BIGINT AS total_count FROM affected_notes
        )
        SELECT 
            total.total_count,
            nd.rn,
            nd.id,
            nd.guid,
            nd.last_update,
            nd.reviewed,
            nd.owner,
            nd.full_path,
            nd.notetype,
            nd.delete_req,
            COALESCE(fd.unreviewed_fields, '[]'::json) AS unreviewed_fields,
            COALESCE(ffd.first_fields, '[]'::json) AS first_fields,
            COALESCE(td.tags_changes, '[]'::json) AS tags_changes,
            md.move_req,
            COALESCE(rfd.reviewed_fields, '[]'::json) AS reviewed_fields,
            COALESCE(rtd.reviewed_tags, '[]'::json) AS reviewed_tags,
            im.base_note_id,
            im.subscribed_fields,
            COALESCE(im.removed_base_tags, '{}') AS removed_base_tags,
            COALESCE(brf.base_fields, '[]'::json) AS base_reviewed_fields,
            COALESCE(brt.base_tags, '[]'::json) AS base_reviewed_tags
        FROM total
        LEFT JOIN note_data nd ON TRUE
        LEFT JOIN fields_data fd ON nd.id = fd.note
        LEFT JOIN first_fields_data ffd ON nd.id = ffd.note
        LEFT JOIN tags_data td ON nd.id = td.note
        LEFT JOIN reviewed_fields_data rfd ON nd.id = rfd.note
        LEFT JOIN reviewed_tags_data rtd ON nd.id = rtd.note
        LEFT JOIN inheritance_meta im ON nd.id = im.note
        LEFT JOIN base_reviewed_fields brf ON nd.id = brf.note
        LEFT JOIN base_reviewed_tags brt ON nd.id = brt.note
        LEFT JOIN move_data md ON nd.id = md.note
        ORDER BY nd.rn;
    ";

    let rows = client
        .query(
            comprehensive_query,
            &[&commit_id, &sanitized_offset, &upper_bound],
        )
        .await?;

    let mut total_count: i64 = 0;
    let mut commit_info: Vec<CommitData> = Vec::new();

    for row in rows {
        let row_total: i64 = row.get(0);
        if total_count == 0 {
            total_count = row_total;
        }

        let note_id: Option<i64> = row.get(2);
        if note_id.is_none() {
            continue;
        }

        let delete_req = row.get::<_, Option<bool>>(9).unwrap_or(false);

        let mut current_note = CommitData {
            commit_id,
            id: note_id.unwrap(),
            guid: row.get::<_, Option<String>>(3).unwrap_or_default(),
            last_update: row.get::<_, Option<String>>(4).unwrap_or_default(),
            reviewed: row.get::<_, Option<bool>>(5).unwrap_or(false),
            owner: row.get::<_, Option<i32>>(6).unwrap_or_default(),
            deck: row
                .get::<_, Option<String>>(7)
                .map_or_default(|name| deck_leaf(&name)),
            note_model: row.get::<_, Option<i64>>(8).unwrap_or_default(),
            delete_req,
            move_req: None,
            fields: Vec::new(),
            new_tags: Vec::new(),
            removed_tags: Vec::new(),
            reviewed_fields: Vec::new(),
            reviewed_tags: Vec::new(),
        };

        if delete_req {
            let first_fields_json: serde_json::Value = row.get(11);
            current_note.fields = parse_first_fields_json(&first_fields_json);
        } else {
            let unreviewed_fields_json: serde_json::Value = row.get(10);
            current_note.fields = parse_unreviewed_fields_json(&unreviewed_fields_json);

            let tags_changes_json: serde_json::Value = row.get(12);
            let (new_tags, removed_tags) = parse_tags_changes_json(&tags_changes_json, commit_id);
            current_note.new_tags = new_tags;
            current_note.removed_tags = removed_tags;
        }

        let move_req_json: Option<serde_json::Value> = row.get(13);
        current_note.move_req = parse_move_req_json(move_req_json.as_ref());

        let reviewed_fields_json: serde_json::Value = row.get(14);
        if let Some(fields_array) = reviewed_fields_json.as_array() {
            for field_data in fields_array {
                let clean_content = field_data
                    .get("content")
                    .and_then(|v| v.as_str())
                    .map_or_default(cleanser::clean);
                current_note.reviewed_fields.push(FieldsInfo {
                    id: field_data
                        .get("id")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0),
                    position: field_data
                        .get("position")
                        .and_then(serde_json::Value::as_i64)
                        .and_then(|v| u32::try_from(v).ok())
                        .unwrap_or(0),
                    content: clean_content,
                    inherited: false,
                });
            }
        }

        let reviewed_tags_json: serde_json::Value = row.get(15);
        if let Some(tags_array) = reviewed_tags_json.as_array() {
            for tag_data in tags_array {
                if let (Some(id), Some(content)) = (
                    tag_data.get("id").and_then(serde_json::Value::as_i64),
                    tag_data.get("content").and_then(|v| v.as_str()),
                ) {
                    current_note.reviewed_tags.push(TagsInfo {
                        id,
                        content: cleanser::clean(content),
                        inherited: false,
                        commit_id,
                    });
                }
            }
        }

        let base_note_id: Option<i64> = row.get(16);
        let subscribed_fields: Option<Vec<i32>> = row.get(17);
        let removed_base_tags: Vec<String> = row.get(18);
        let base_fields_json: serde_json::Value = row.get(19);
        let base_tags_json: serde_json::Value = row.get(20);

        if base_note_id.is_some() {
            // Overlay inherited fields
            let subscribed_set: Option<HashSet<u32>> = subscribed_fields.map(|positions| {
                positions
                    .into_iter()
                    .map(|p| u32::try_from(p.max(0)).unwrap_or(0))
                    .collect()
            });

            let mut position_index: HashMap<u32, usize> = HashMap::new();
            for (idx, field) in current_note.reviewed_fields.iter().enumerate() {
                position_index.insert(field.position, idx);
            }

            if let Some(base_fields) = base_fields_json.as_array() {
                for field_data in base_fields {
                    let position = field_data
                        .get("position")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0);
                    let position_u32 = u32::try_from(position.max(0)).unwrap_or(0);
                    let allowed = match &subscribed_set {
                        None => true,
                        Some(set) => set.contains(&position_u32),
                    };
                    if !allowed {
                        continue;
                    }

                    let clean_content = field_data
                        .get("content")
                        .and_then(|v| v.as_str())
                        .map_or_default(cleanser::clean);

                    if let Some(idx) = position_index.get(&position_u32).copied() {
                        current_note.reviewed_fields[idx]
                            .content
                            .clone_from(&clean_content);
                        current_note.reviewed_fields[idx].inherited = true;
                    } else {
                        current_note.reviewed_fields.push(FieldsInfo {
                            id: 0,
                            position: position_u32,
                            content: clean_content.clone(),
                            inherited: true,
                        });
                        position_index.insert(position_u32, current_note.reviewed_fields.len() - 1);
                    }
                }
            }

            // Merge inherited tags
            let local_tags: HashSet<String> = current_note
                .reviewed_tags
                .iter()
                .map(|t| t.content.clone())
                .collect();

            let removed_set: HashSet<String> = removed_base_tags
                .into_iter()
                .map(|t| cleanser::clean(&t))
                .collect();

            let mut effective_base: HashSet<String> = HashSet::new();
            if let Some(base_tags) = base_tags_json.as_array() {
                for tag_data in base_tags {
                    if let Some(content) = tag_data.get("content").and_then(|v| v.as_str()) {
                        let clean_content = cleanser::clean(content);
                        if !removed_set.contains(&clean_content) {
                            effective_base.insert(clean_content);
                        }
                    }
                }
            }

            let mut combined: Vec<String> = local_tags.union(&effective_base).cloned().collect();
            combined.sort();

            current_note.reviewed_tags = combined
                .into_iter()
                .map(|tag_content| TagsInfo {
                    id: 0,
                    content: tag_content.clone(),
                    inherited: !local_tags.contains(&tag_content),
                    commit_id,
                })
                .collect();
        }

        current_note
            .reviewed_fields
            .sort_by_key(|field| field.position);

        if should_include_note(&current_note) {
            commit_info.push(current_note);
        }
    }

    if total_count == 0 {
        return Err(NoNotesAffected);
    }

    let increment = sanitized_limit;
    let candidate_next = sanitized_offset.saturating_add(increment);
    let next_offset = if candidate_next < total_count {
        Some(candidate_next)
    } else {
        None
    };

    Ok(CommitNotesPage {
        total: total_count,
        offset: sanitized_offset,
        limit: sanitized_limit,
        next_offset,
        notes: commit_info,
    })
}

/// Lightweight version of `notes_by_commit` tailored for the preview drawer.
///
/// Skips `first_fields_data`, `reviewed_fields_data`, `reviewed_tags_data`,
/// `inheritance_meta`, `base_reviewed_fields`, and `base_reviewed_tags`, while
/// preserving total count, `delete_req` notes, fields diffs, tags changes,
/// and move requests filtering.
pub async fn preview_notes_by_commit(
    db_state: &database::AppState,
    commit_id: i32,
    offset: i64,
    limit: i64,
) -> Return<CommitNotesPage> {
    let client = database::client(db_state).await?;

    let sanitized_offset = offset.max(0);
    let sanitized_limit = limit.clamp(1, 200);
    let upper_bound = sanitized_offset.saturating_add(sanitized_limit);

    let preview_query = r"
        WITH affected_notes AS MATERIALIZED (
            SELECT note FROM (
                SELECT note FROM fields WHERE commit = $1 AND reviewed = false
                UNION ALL
                SELECT note FROM tags WHERE commit = $1 AND reviewed = false
                UNION ALL
                SELECT note FROM card_deletion_suggestions WHERE commit = $1
                UNION ALL
                SELECT note FROM note_move_suggestions WHERE commit = $1
            ) AS n
            GROUP BY note
        ),
        ordered_notes AS MATERIALIZED (
            SELECT
                note,
                ROW_NUMBER() OVER (ORDER BY note) AS rn
            FROM affected_notes
        ),
        page_notes AS (
            SELECT note, rn
            FROM ordered_notes
            WHERE rn > $2 AND rn <= $3
        ),
        note_data AS (
            SELECT 
                n.id,
                n.guid,
                TO_CHAR(n.last_update, 'MM/DD/YYYY HH12:MI AM') AS last_update,
                n.reviewed,
                d.owner,
                d.full_path,
                n.notetype,
                (cds.note IS NOT NULL) AS delete_req,
                pn.rn
            FROM notes n
            JOIN page_notes pn ON n.id = pn.note
            JOIN decks d ON d.id = n.deck
            LEFT JOIN card_deletion_suggestions cds ON cds.note = n.id AND cds.commit = $1
        ),
        fields_data AS (
            SELECT 
                f1.note,
                json_agg(json_build_object(
                    'id', f1.id,
                    'position', f1.position::int,
                    'content', f1.content,
                    'reviewed_content', COALESCE(f2.content, '')
                ) ORDER BY f1.position) AS unreviewed_fields
            FROM fields f1
            LEFT JOIN fields f2 ON f1.note = f2.note AND f1.position = f2.position AND f2.reviewed = true
            WHERE f1.reviewed = false AND f1.commit = $1 AND f1.note IN (SELECT note FROM page_notes)
            GROUP BY f1.note
        ),
        tags_data AS (
            SELECT 
                t.note,
                json_agg(json_build_object('id', t.id, 'content', t.content, 'action', t.action)) AS tags_changes
            FROM tags t
            WHERE t.commit = $1 AND t.note IN (SELECT note FROM page_notes) AND t.reviewed = false
            GROUP BY t.note
        ),
        move_data AS (
            SELECT 
                nms.note,
                json_build_object('id', nms.id, 'path', d.full_path) AS move_req
            FROM note_move_suggestions nms
            JOIN decks d ON d.id = nms.target_deck
            WHERE nms.note IN (SELECT note FROM page_notes) AND nms.commit = $1
        ),
        total AS (
            SELECT COUNT(*)::BIGINT AS total_count FROM affected_notes
        )
        SELECT 
            total.total_count,
            nd.rn,
            nd.id,
            nd.guid,
            nd.last_update,
            nd.reviewed,
            nd.owner,
            nd.full_path,
            nd.notetype,
            nd.delete_req,
            COALESCE(fd.unreviewed_fields, '[]'::json) AS unreviewed_fields,
            COALESCE(td.tags_changes, '[]'::json) AS tags_changes,
            md.move_req
        FROM total
        LEFT JOIN note_data nd ON TRUE
        LEFT JOIN fields_data fd ON nd.id = fd.note
        LEFT JOIN tags_data td ON nd.id = td.note
        LEFT JOIN move_data md ON nd.id = md.note
        ORDER BY nd.rn;
    ";

    let rows = client
        .query(
            preview_query,
            &[&commit_id, &sanitized_offset, &upper_bound],
        )
        .await?;

    let mut total_count: i64 = 0;
    let mut commit_info: Vec<CommitData> = Vec::new();

    for row in rows {
        let row_total: i64 = row.get(0);
        if total_count == 0 {
            total_count = row_total;
        }

        let note_id: Option<i64> = row.get(2);
        if note_id.is_none() {
            continue;
        }

        let delete_req = row.get::<_, Option<bool>>(9).unwrap_or(false);

        let mut current_note = CommitData {
            commit_id,
            id: note_id.unwrap(),
            guid: row.get::<_, Option<String>>(3).unwrap_or_default(),
            last_update: row.get::<_, Option<String>>(4).unwrap_or_default(),
            reviewed: row.get::<_, Option<bool>>(5).unwrap_or(false),
            owner: row.get::<_, Option<i32>>(6).unwrap_or_default(),
            deck: row
                .get::<_, Option<String>>(7)
                .map_or_default(|name| deck_leaf(&name)),
            note_model: row.get::<_, Option<i64>>(8).unwrap_or_default(),
            delete_req,
            move_req: None,
            fields: Vec::new(),
            new_tags: Vec::new(),
            removed_tags: Vec::new(),
            reviewed_fields: Vec::new(),
            reviewed_tags: Vec::new(),
        };

        if !delete_req {
            let unreviewed_fields_json: serde_json::Value = row.get(10);
            current_note.fields = parse_unreviewed_fields_json(&unreviewed_fields_json);

            let tags_changes_json: serde_json::Value = row.get(11);
            let (new_tags, removed_tags) = parse_tags_changes_json(&tags_changes_json, commit_id);
            current_note.new_tags = new_tags;
            current_note.removed_tags = removed_tags;
        }

        let move_req_json: Option<serde_json::Value> = row.get(12);
        current_note.move_req = parse_move_req_json(move_req_json.as_ref());

        if should_include_note(&current_note) {
            commit_info.push(current_note);
        }
    }

    if total_count == 0 {
        return Err(NoNotesAffected);
    }

    let increment = sanitized_limit;
    let candidate_next = sanitized_offset.saturating_add(increment);
    let next_offset = if candidate_next < total_count {
        Some(candidate_next)
    } else {
        None
    };

    Ok(CommitNotesPage {
        total: total_count,
        offset: sanitized_offset,
        limit: sanitized_limit,
        next_offset,
        notes: commit_info,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_unreviewed_fields_json() {
        let json = serde_json::json!([
            {
                "id": 10,
                "position": 0,
                "content": "Hello World",
                "reviewed_content": "Hello"
            },
            {
                "id": 11,
                "position": 1,
                "content": "<script>alert(1)</script>Safe",
                "reviewed_content": ""
            }
        ]);
        let fields = parse_unreviewed_fields_json(&json);
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].id, 10);
        assert_eq!(fields[0].position, 0);
        assert_eq!(fields[0].content, "Hello World");
        assert_eq!(fields[0].reviewed_content, "Hello");
        assert!(fields[0].diff.contains("World"));

        assert_eq!(fields[1].id, 11);
        assert_eq!(fields[1].position, 1);
        assert!(!fields[1].content.contains("<script>"));
    }

    #[test]
    fn test_parse_first_fields_json() {
        let json = serde_json::json!([
            {
                "id": 20,
                "position": 0,
                "content": "Front Content"
            }
        ]);
        let fields = parse_first_fields_json(&json);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].id, 20);
        assert_eq!(fields[0].content, "Front Content");
        assert_eq!(fields[0].reviewed_content, "Front Content");
        assert_eq!(fields[0].diff, "Front Content");
    }

    #[test]
    fn test_parse_tags_changes_json() {
        let json = serde_json::json!([
            {
                "id": 1,
                "content": "tag_new",
                "action": true
            },
            {
                "id": 2,
                "content": "tag_removed",
                "action": false
            }
        ]);
        let (new_tags, removed_tags) = parse_tags_changes_json(&json, 123);
        assert_eq!(new_tags.len(), 1);
        assert_eq!(new_tags[0].id, 1);
        assert_eq!(new_tags[0].content, "tag_new");
        assert_eq!(new_tags[0].commit_id, 123);

        assert_eq!(removed_tags.len(), 1);
        assert_eq!(removed_tags[0].id, 2);
        assert_eq!(removed_tags[0].content, "tag_removed");
        assert_eq!(removed_tags[0].commit_id, 123);
    }

    #[test]
    fn test_parse_move_req_json() {
        let json = serde_json::json!({
            "id": 5,
            "path": "Deck::Subdeck"
        });
        let move_req = parse_move_req_json(Some(&json)).expect("should parse");
        assert_eq!(move_req.id, 5);
        assert_eq!(move_req.path, "Deck::Subdeck");

        assert!(parse_move_req_json(None).is_none());
    }

    #[test]
    fn test_should_include_note() {
        let mut note = CommitData {
            commit_id: 1,
            id: 10,
            guid: String::new(),
            last_update: String::new(),
            reviewed: false,
            owner: 0,
            deck: String::new(),
            note_model: 0,
            delete_req: false,
            move_req: None,
            fields: Vec::new(),
            new_tags: Vec::new(),
            removed_tags: Vec::new(),
            reviewed_fields: Vec::new(),
            reviewed_tags: Vec::new(),
        };

        // Initially no changes
        assert!(!should_include_note(&note));

        // Delete request note must be included even if fields empty
        note.delete_req = true;
        assert!(should_include_note(&note));
        note.delete_req = false;

        // Note with fields
        note.fields.push(FieldsReviewInfo {
            id: 1,
            position: 0,
            content: "a".into(),
            reviewed_content: String::new(),
            diff: "a".into(),
        });
        assert!(should_include_note(&note));
        note.fields.clear();

        // Note with new tags
        note.new_tags.push(TagsInfo {
            id: 1,
            content: "tag".into(),
            inherited: false,
            commit_id: 1,
        });
        assert!(should_include_note(&note));
        note.new_tags.clear();

        // Note with removed tags
        note.removed_tags.push(TagsInfo {
            id: 1,
            content: "tag".into(),
            inherited: false,
            commit_id: 1,
        });
        assert!(should_include_note(&note));
        note.removed_tags.clear();

        // Note with move req
        note.move_req = Some(NoteMoveReq {
            id: 1,
            path: "p".into(),
        });
        assert!(should_include_note(&note));
    }
}
