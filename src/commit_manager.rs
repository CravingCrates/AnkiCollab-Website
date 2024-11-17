use crate::database;
use crate::error::Error::*;
use crate::structs::*;
use crate::Return;

use std::cmp::min;
use std::collections::HashMap;
use std::sync::Arc;

fn get_string_from_rationale(input: i32) -> &'static str {
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

pub async fn get_commit_info(db_state: &Arc<database::AppState>, commit_id: i32) -> Return<CommitsOverview> {
    let query = r#"    
        SELECT c.commit_id, c.rationale, c.info,
        TO_CHAR(c.timestamp, 'MM/DD/YYYY') AS last_update,
        d.name
        FROM commits c
        JOIN decks d on d.id = c.deck
        WHERE c.commit_id = $1
    "#;
    let client = database::client(db_state).await?;
    let row = client.query_one(query, &[&commit_id]).await?;
    let commit = CommitsOverview {
        id: row.get(0),
        rationale: get_string_from_rationale(row.get(1)).into(),
        commit_info: row.get(2),
        timestamp: row.get(3),
        deck: row.get(4),
    };
    Ok(commit)
}

// Helper function to find the shortest common prefix among a vector of strings
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

pub async fn commits_review(db_state: &Arc<database::AppState>, uid: i32) -> Result<Vec<CommitsOverview>, Box<dyn std::error::Error>> {
    let client = database::client(db_state).await?;

    let accessible_query = r#"
        WITH RECURSIVE accessible AS (
            SELECT id FROM decks WHERE id IN (
                SELECT deck FROM maintainers WHERE user_id = $1
                UNION
                SELECT id FROM decks WHERE owner = $1
            )
            UNION
            SELECT decks.id FROM decks
            INNER JOIN accessible ON decks.parent = accessible.id
        )
        SELECT id FROM accessible
    "#;
    let accessible_decks: Vec<i64> = client
        .query(accessible_query, &[&uid])
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();

    let changes_query = r#"
        WITH unreviewed_changes AS (
            SELECT commit, note FROM fields WHERE reviewed = false
            UNION ALL
            SELECT commit, note FROM tags WHERE reviewed = false
            UNION ALL 
            SELECT commit, note FROM card_deletion_suggestions
            UNION ALL
            SELECT commit, note FROM note_move_suggestions
        ),
        distinct_commits AS (
            SELECT DISTINCT commit,
                FIRST_VALUE(note) OVER (PARTITION BY commit ORDER BY note) as note
            FROM unreviewed_changes
        )
        SELECT 
            c.commit_id,
            c.rationale,
            c.info,
            TO_CHAR(c."timestamp", 'MM/DD/YYYY') AS formatted_timestamp,
            c.deck,
            dc.note
        FROM commits c
        INNER JOIN distinct_commits dc ON c.commit_id = dc.commit
        WHERE (c.deck = ANY($1) OR (SELECT is_admin FROM users WHERE id = $2))
    "#;
    let changes_rows = client
        .query(changes_query, &[&accessible_decks, &uid])
        .await?;

    // This is new and kinda bad, bc its super slow and inefficient. But it works for now. gottaa think of a better way to do this tho
    let deck_names_query = r#"
        SELECT
            n.id AS note_id,
            d.full_path
        FROM notes n
        JOIN decks d ON d.id = n.deck
        WHERE n.id = ANY($1)
    "#;
    let note_ids: Vec<i64> = changes_rows.iter().filter_map(|row| row.get(5)).collect();
    let deck_names_rows = client.query(deck_names_query, &[&note_ids]).await?;

    // Process results
    let mut commit_map: HashMap<i32, (CommitsOverview, Vec<String>)> = HashMap::new();

    for row in changes_rows {
        let commit_id: i32 = row.get(0);
        let note_id: Option<i64> = row.get(5);
        
        commit_map.entry(commit_id).or_insert_with(|| (
            CommitsOverview {
                id: commit_id,
                rationale: get_string_from_rationale(row.get(1)).into(),
                commit_info: row.get(2),
                timestamp: row.get(3),
                deck: String::new(),
            },
            Vec::new()
        ));

        if let Some(note_id) = note_id {
            commit_map.get_mut(&commit_id).unwrap().1.push(note_id.to_string());
        }
    }

    let mut deck_paths: HashMap<i64, String> = HashMap::new();
    for row in deck_names_rows {
        let note_id: i64 = row.get(0);
        let full_path: String = row.get(1);
        deck_paths.insert(note_id, full_path);
    }

    // We could do all that in just 1 sql query, but to break it down and make it more readable, we do it here
    let result: Vec<CommitsOverview> = commit_map
        .into_iter()
        .map(|(_, (mut overview, note_ids))| {
            let paths: Vec<&str> = note_ids
                .iter()
                .filter_map(|note_id| deck_paths.get(&note_id.parse::<i64>().unwrap()).map(|s| s.as_str()))
                .collect();
            
            if !paths.is_empty() {
                overview.deck = find_common_prefix(paths);
            }
            overview
        })
        .collect();

    Ok(result)
}

pub async fn notes_by_commit(db_state: &Arc<database::AppState>, commit_id: i32) -> Return<Vec<CommitData>> {
    let client = database::client(db_state).await?;

    let get_notes = "
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
    ";
    let affected_notes = client
        .query(get_notes, &[&commit_id])
        .await?
        .into_iter()
        .map(|row| row.get::<_, i64>("note"))
        .collect::<Vec<i64>>();

    if affected_notes.is_empty() {
        return Err(NoNotesAffected);
    }

    let note_info_query = client
        .prepare("
            SELECT id, guid, TO_CHAR(last_update, 'MM/DD/YYYY HH12:MI AM') AS last_update, reviewed, 
            (Select owner from decks where id = notes.deck), (select full_path from decks where id = notes.deck) as full_path, notetype
            FROM notes
            WHERE id = $1
        ").await?;

    let fields_query = client
        .prepare(
            "
            SELECT f1.id, f1.position, f1.content, COALESCE(f2.content, '') AS reviewed_content 
            FROM fields f1 
            LEFT JOIN fields f2 
            ON f1.note = f2.note AND f1.position = f2.position AND f2.reviewed = true 
            WHERE f1.reviewed = false AND f1.commit = $1 AND f1.note = $2
            ORDER BY position
        ",
        )
        .await?;

    let tags_query = client
        .prepare(
            "
            SELECT id, content, action
            FROM tags
            WHERE commit = $1 and note = $2 and reviewed = false
        ",
        )
        .await?;

    let delete_req_query = client
        .prepare(
            "
            SELECT 1
            FROM card_deletion_suggestions
            WHERE note = $1
        ",
        )
        .await?;

    let move_req_query = client
    .prepare(
            "
            SELECT nms.id, d.full_path FROM note_move_suggestions nms
            JOIN decks d ON d.id = nms.target_deck
            WHERE nms.note = $1 and nms.commit = $2
            LIMIT 1
        ",
        )
        .await?;

    let first_field_query = client
        .prepare(
            "
            SELECT id, position, content
            FROM fields 
            WHERE note = $1
            ORDER BY position
            LIMIT 3
        ",
        )
        .await?;

    let mut commit_info = Vec::with_capacity(affected_notes.len());

    for note_id in affected_notes {
        let mut current_note = CommitData {
            commit_id,
            id: 0,
            guid: String::new(),
            deck: String::new(),
            owner: 0,
            note_model: 0,
            last_update: String::new(),
            reviewed: false,
            delete_req: false,
            move_req: None,
            fields: Vec::new(),
            new_tags: Vec::new(),
            removed_tags: Vec::new(),
        };

        // Fill generic note info
        let note_res = client.query_one(&note_info_query, &[&note_id]).await?;
        let note_guid: String = note_res.get(1);
        let note_last_update: String = note_res.get(2);
        let note_reviewed: bool = note_res.get(3);
        let note_owner: i32 = note_res.get(4);
        let note_deck: String = note_res.get(5);
        let note_model: i64 = note_res.get(6);

        current_note.id = note_id;
        current_note.guid = note_guid;
        current_note.last_update = note_last_update;
        current_note.reviewed = note_reviewed;
        current_note.owner = note_owner;
        current_note.note_model = note_model;
        current_note.deck = note_deck;

        let delete_req_rows = client.query(&delete_req_query, &[&note_id]).await?;
        current_note.delete_req = !delete_req_rows.is_empty();

        if current_note.delete_req {
            let fields_rows = client.query(&first_field_query, &[&note_id]).await?;

            for row in fields_rows {
                let id = row.get(0);
                let position = row.get(1);
                let content = row.get(2);

                if let Some(content) = content {
                    current_note.fields.push(FieldsReviewInfo {
                        id,
                        position,
                        content: ammonia::clean(content),
                        reviewed_content: ammonia::clean(content),
                    });
                }
            }
        } else {
            // Now get to the actual good bits (unreviewed material!)
            let fields_rows = client.query(&fields_query, &[&commit_id, &note_id]).await?;
            for row in fields_rows {
                let id = row.get(0);
                let position = row.get(1);
                let content = row.get(2);
                let reviewed = row.get(3);
                if let Some(content) = content {
                    current_note.fields.push(FieldsReviewInfo {
                        id,
                        position,
                        content: ammonia::clean(content),
                        reviewed_content: ammonia::clean(reviewed),
                    });
                }
            }
            let tags_rows = client.query(&tags_query, &[&commit_id, &note_id]).await?;
            for row in tags_rows {
                let id = row.get(0);
                let content = row.get(1);
                let action = row.get(2);
                if let Some(content) = content {
                    if action {
                        // New suggested tag
                        current_note.new_tags.push(TagsInfo { id, content });
                    } else {
                        // Tag got removed
                        current_note.removed_tags.push(TagsInfo { id, content });
                    }
                }
            }

            let move_req_rows = client.query(&move_req_query, &[&note_id, &commit_id]).await?;
            if !move_req_rows.is_empty() {
                let move_req_row = NoteMoveReq {
                    id: move_req_rows[0].get(0),
                    path: move_req_rows[0].get(1),
                };
                current_note.move_req = Some(move_req_row);
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
