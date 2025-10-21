use crate::cleanser;
use crate::database;
use crate::error::Error::NoNotesAffected;
use crate::structs::{CommitData, CommitsOverview, FieldsReviewInfo, NoteMoveReq, TagsInfo};
use crate::Return;

use std::cmp::min;
use std::sync::Arc;

extern crate htmldiff;

const fn get_string_from_rationale(input: i32) -> &'static str {
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

pub async fn get_commit_info(
    db_state: &Arc<database::AppState>,
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

fn find_common_prefix(paths: Vec<&str>) -> String {
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
    db_state: &Arc<database::AppState>,
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
            SELECT DISTINCT commit_id
            FROM (
                SELECT c.commit_id
                FROM fields f
                JOIN commits c ON c.commit_id = f.commit
                WHERE f.reviewed = false
                AND c.deck IN (SELECT id FROM accessible)

                UNION ALL

                SELECT c.commit_id
                FROM tags t
                JOIN commits c ON c.commit_id = t.commit
                WHERE t.reviewed = false
                AND c.deck IN (SELECT id FROM accessible)

                UNION ALL

                SELECT c.commit_id
                FROM card_deletion_suggestions cds
                JOIN commits c ON c.commit_id = cds.commit
                WHERE c.deck IN (SELECT id FROM accessible)

                UNION ALL

                SELECT c.commit_id
                FROM note_move_suggestions nms
                JOIN commits c ON c.commit_id = nms.commit
                WHERE c.deck IN (SELECT id FROM accessible)
            ) s
        ),

        distinct_decks AS (
            SELECT DISTINCT src.commit, n.deck
            FROM (
                SELECT f.commit, f.note FROM fields f WHERE f.reviewed = false AND f.commit IN (SELECT commit_id FROM relevant_commits)
                UNION ALL
                SELECT t.commit, t.note FROM tags t WHERE t.reviewed = false AND t.commit IN (SELECT commit_id FROM relevant_commits)
                UNION ALL
                SELECT cds.commit, cds.note FROM card_deletion_suggestions cds WHERE cds.commit IN (SELECT commit_id FROM relevant_commits)
                UNION ALL
                SELECT nms.commit, nms.note FROM note_move_suggestions nms WHERE nms.commit IN (SELECT commit_id FROM relevant_commits)
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
                find_common_prefix(paths_ref)
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

pub async fn get_field_diff(db_state: &Arc<database::AppState>, field_id: i64) -> Return<String> {
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
    let position: u32 = new_content_row.get::<_, i32>(2) as u32;
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

pub async fn notes_by_commit(
    db_state: &Arc<database::AppState>,
    commit_id: i32,
) -> Return<Vec<CommitData>> {
    let client = database::client(db_state).await?;

    let comprehensive_query = r#"
        WITH affected_notes AS (
            SELECT note FROM (
                SELECT note FROM fields WHERE commit = $1 and reviewed = false
                UNION ALL
                SELECT note FROM tags WHERE commit = $1 and reviewed = false
                UNION ALL
                SELECT note FROM card_deletion_suggestions WHERE commit = $1
                UNION ALL
                SELECT note FROM note_move_suggestions WHERE commit = $1
            ) AS n
            GROUP BY note
            LIMIT 100
        ),
        note_data AS (
            SELECT 
                n.id, n.guid, TO_CHAR(n.last_update, 'MM/DD/YYYY HH12:MI AM') AS last_update,
                n.reviewed, d.owner, d.full_path, n.notetype,
                (cds.note IS NOT NULL) as delete_req
            FROM notes n
            JOIN affected_notes an ON n.id = an.note
            JOIN decks d ON d.id = n.deck
            LEFT JOIN card_deletion_suggestions cds ON cds.note = n.id AND cds.commit = $1
        ),
        fields_data AS (
            SELECT 
                f1.note,
                json_agg(json_build_object('id', f1.id, 'position', f1.position::int, 'content', f1.content, 'reviewed_content', COALESCE(f2.content, '')) ORDER BY f1.position) as unreviewed_fields
            FROM fields f1
            LEFT JOIN fields f2 ON f1.note = f2.note AND f1.position = f2.position AND f2.reviewed = true
            WHERE f1.reviewed = false AND f1.commit = $1 AND f1.note IN (SELECT note FROM affected_notes)
            GROUP BY f1.note
        ),
        first_fields_data AS (
            WITH numbered_fields AS (
                SELECT f.id, f.note, f.position::int AS position, f.content,
                    ROW_NUMBER() OVER(PARTITION BY f.note ORDER BY f.position) as rn
                FROM fields f
                WHERE f.note IN (SELECT note FROM affected_notes)
            )
            SELECT
                nf.note,
                json_agg(json_build_object('id', nf.id, 'position', nf.position::int, 'content', nf.content) ORDER BY nf.position) as first_fields
            FROM numbered_fields nf
            WHERE nf.rn <= 3
            GROUP BY nf.note
        ),
        tags_data AS (
            SELECT 
                t.note,
                json_agg(json_build_object('id', t.id, 'content', t.content, 'action', t.action)) as tags_changes
            FROM tags t
            WHERE t.commit = $1 AND t.note IN (SELECT note FROM affected_notes) AND t.reviewed = false
            GROUP BY t.note
        ),
        move_data AS (
            SELECT 
                nms.note,
                json_build_object('id', nms.id, 'path', d.full_path) as move_req
            FROM note_move_suggestions nms
            JOIN decks d ON d.id = nms.target_deck
            WHERE nms.note IN (SELECT note FROM affected_notes) AND nms.commit = $1
        )
        SELECT 
            nd.id, nd.guid, nd.last_update, nd.reviewed, nd.owner, nd.full_path,
            nd.notetype, nd.delete_req,
            COALESCE(fd.unreviewed_fields, '[]'::json) as unreviewed_fields,
            COALESCE(ffd.first_fields, '[]'::json) as first_fields,
            COALESCE(td.tags_changes, '[]'::json) as tags_changes,
            md.move_req
        FROM note_data nd
        LEFT JOIN fields_data fd ON nd.id = fd.note
        LEFT JOIN first_fields_data ffd ON nd.id = ffd.note
        LEFT JOIN tags_data td ON nd.id = td.note
        LEFT JOIN move_data md ON nd.id = md.note
    "#;

    let rows = client.query(comprehensive_query, &[&commit_id]).await?;

    if rows.is_empty() {
        return Err(NoNotesAffected);
    }

    let mut commit_info = Vec::with_capacity(rows.len());

    for row in rows {
        let delete_req: bool = row.get(7);

        let mut current_note = CommitData {
            commit_id,
            id: row.get(0),
            guid: row.get(1),
            last_update: row.get(2),
            reviewed: row.get(3),
            owner: row.get(4),
            deck: row.get(5),
            note_model: row.get(6),
            delete_req,
            move_req: None,
            fields: Vec::new(),
            new_tags: Vec::new(),
            removed_tags: Vec::new(),
        };

        // Process fields based on whether it's a delete request
        if delete_req {
            let first_fields_json: serde_json::Value = row.get(9);
            if let Some(fields_array) = first_fields_json.as_array() {
                for field_data in fields_array {
                    let content = field_data
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let clean_content = cleanser::clean(content);
                    current_note.fields.push(FieldsReviewInfo {
                        id: field_data.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
                        position: field_data
                            .get("position")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0) as u32,
                        content: clean_content.clone(),
                        reviewed_content: clean_content.clone(),
                        diff: clean_content,
                    });
                }
            }
        } else {
            // Process unreviewed fields
            let unreviewed_fields_json: serde_json::Value = row.get(8);
            if let Some(fields_array) = unreviewed_fields_json.as_array() {
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

                    current_note.fields.push(FieldsReviewInfo {
                        id: field_data.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
                        position: field_data
                            .get("position")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0) as u32,
                        content: clean_content,
                        reviewed_content: clean_reviewed,
                        diff: diff_string,
                    });
                }
            }
            // Process tags
            let tags_changes_json: serde_json::Value = row.get(10);
            if let Some(tags_array) = tags_changes_json.as_array() {
                for tag_data in tags_array {
                    if let (Some(id), Some(content), Some(action)) = (
                        tag_data.get("id").and_then(|v| v.as_i64()),
                        tag_data.get("content").and_then(|v| v.as_str()),
                        tag_data.get("action").and_then(|v| v.as_bool()),
                    ) {
                        let tag_info = TagsInfo {
                            id,
                            content: cleanser::clean(content),
                            inherited: false,
                            commit_id,
                        };
                        if action {
                            current_note.new_tags.push(tag_info);
                        } else {
                            current_note.removed_tags.push(tag_info);
                        }
                    }
                }
            }
        }

        let move_req_json: Option<serde_json::Value> = row.get(11);
        if let Some(move_data) = move_req_json {
            if let (Some(id), Some(path)) = (
                move_data.get("id").and_then(|v| v.as_i64()),
                move_data.get("path").and_then(|v| v.as_str()),
            ) {
                current_note.move_req = Some(NoteMoveReq {
                    id: id as i32,
                    path: path.to_string(),
                });
            }
        }

        if !current_note.fields.is_empty()
            || !current_note.new_tags.is_empty()
            || !current_note.removed_tags.is_empty()
            || current_note.move_req.is_some()
        {
            commit_info.push(current_note);
        }
    }

    Ok(commit_info)
}
