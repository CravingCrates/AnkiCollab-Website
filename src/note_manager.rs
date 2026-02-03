use std::sync::Arc;

use crate::cleanser;
use crate::database;
use crate::error::Error::{NoteNotFound, Unauthorized};
use crate::error::NoteNotFoundContext;
use crate::note_history::{self, EventType};
use crate::structs::{
    FieldSuggestionInfo, FieldsInfo, Note, NoteData, NoteMoveReq, ReviewOverview, TagsInfo,
};
use crate::suggestion_manager;
use crate::user;
use crate::NoteId;
use crate::Return;

extern crate htmldiff;

pub async fn under_review(
    db_state: &Arc<database::AppState>,
    uid: i32,
) -> Result<Vec<ReviewOverview>, Box<dyn std::error::Error>> {
    let query = r"
        WITH owned AS (
            SELECT id, full_path FROM decks WHERE id IN (
                SELECT deck FROM maintainers WHERE user_id = $1
                UNION
                SELECT id FROM decks WHERE owner = $1
            )
        )
        SELECT n.id, n.guid, d.full_path,
        (CASE
            WHEN n.reviewed = false THEN 0 ELSE 1
        END) AS status,
        TO_CHAR(n.last_update, 'MM/DD/YYYY') AS last_update,
        coalesce(string_agg(f.content, ','), '') AS content
        FROM notes AS n
        LEFT JOIN fields AS f ON n.id = f.note
        LEFT JOIN owned AS d ON d.id = n.deck
        WHERE
            n.deck in (select id from owned) AND
            (n.reviewed = false OR 
            (n.reviewed = true AND EXISTS (SELECT 1 FROM fields WHERE fields.note = n.id AND fields.reviewed = false)) OR
            (n.reviewed = true AND EXISTS (SELECT 1 FROM tags WHERE tags.note = n.id AND tags.reviewed = false)))
        GROUP BY n.id, n.guid, n.reviewed, d.full_path
    ";
    let client = database::client(db_state).await?;

    let rows = client
        .query(query, &[&uid])
        .await?
        .into_iter()
        .map(|row| ReviewOverview {
            id: row.get(0),
            guid: row.get(1),
            full_path: row.get(2),
            status: row.get(3),
            last_update: row.get(4),
            fields: row.get(5),
        })
        .collect::<Vec<_>>();

    Ok(rows)
}

pub async fn get_notes_count_in_deck(
    db_state: &Arc<database::AppState>,
    deck: i64,
) -> Result<i64, Box<dyn std::error::Error>> {
    let client = database::client(db_state).await?;
    let query = "
        WITH RECURSIVE cte AS (
            SELECT $1::bigint as id
            UNION ALL
            SELECT d.id
            FROM cte JOIN decks d ON d.parent = cte.id
        )
        SELECT COUNT(*) as num FROM notes WHERE deck IN (SELECT id FROM cte) AND deleted = false
    ";
    let rows = client.query(query, &[&deck]).await?;

    let count: i64 = rows[0].get(0);
    Ok(count)
}

pub async fn get_name_by_hash(
    db_state: &Arc<database::AppState>,
    deck: &String,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let client = database::client(db_state).await?;

    let query = "SELECT name FROM decks WHERE human_hash = $1";
    let rows = client.query(query, &[&deck]).await?;

    if rows.is_empty() {
        return Err("Deck not found.".into());
    }

    let name: String = rows[0].get(0);
    Ok(Some(name))
}

pub async fn get_note_data(
    db_state: &Arc<database::AppState>,
    note_id: NoteId,
) -> Return<NoteData> {
    let client = database::client(db_state).await?;

    let note_query = "
        SELECT n.id, n.guid,
               TO_CHAR(n.last_update, 'MM/DD/YYYY HH12:MI AM') AS last_update,
               n.reviewed,
               (SELECT owner FROM decks WHERE id = n.deck) AS owner,
               (SELECT full_path FROM decks WHERE id = n.deck) AS full_path,
               n.notetype,
               EXISTS (SELECT 1 FROM note_inheritance ni WHERE ni.subscriber_note_id = n.id) AS is_inherited
        FROM notes n
        WHERE n.id = $1 AND n.deleted = false
    ";
    let fields_query = "
        SELECT id, position, content, reviewed, commit
        FROM fields
        WHERE note = $1
        ORDER BY position, reviewed DESC NULLS LAST
    ";
    let tags_query = "
        SELECT id, content, reviewed, action, commit
        FROM tags
        WHERE note = $1
    ";

    let notetype_query = "
        SELECT name FROM notetype_field
        WHERE notetype = $1 order by position
    ";

    let delete_req_query = "
        SELECT 1
        FROM card_deletion_suggestions
        WHERE note = $1
    ";

    let move_req_query = "
        SELECT DISTINCT ON (d.full_path) d.full_path, nms.id
        FROM decks d
        JOIN note_move_suggestions nms ON d.id = nms.target_deck
        WHERE nms.note = $1
    ";

    let mut current_note = NoteData {
        id: 0,
        guid: String::new(),
        owner: 0,
        deck: String::new(),
        last_update: String::new(),
        reviewed: false,
        delete_req: false,
        is_inherited: false,
        reviewed_fields: Vec::new(),
        reviewed_tags: Vec::new(),
        unconfirmed_fields: Vec::new(),
        new_tags: Vec::new(),
        removed_tags: Vec::new(),
        note_model_fields: Vec::new(),
        note_move_decks: Vec::new(),
    };

    let note_res = client.query_one(note_query, &[&note_id]).await?;
    let note_guid: String = note_res.get(1);
    let note_last_update: String = note_res.get(2);
    let note_reviewed: bool = note_res.get(3);
    let note_owner: i32 = note_res.get(4);
    let note_deck: String = note_res.get(5);
    let notetype: i64 = note_res.get(6);
    let is_inherited_row: bool = note_res.get(7);

    current_note.id = note_id;
    current_note.guid = note_guid;
    current_note.last_update = note_last_update;
    current_note.reviewed = note_reviewed;
    current_note.owner = note_owner;
    current_note.deck = note_deck;

    // Determine if this is a subscriber note using the merged query
    current_note.is_inherited = is_inherited_row;

    let notetype_fields = client
        .query(notetype_query, &[&notetype])
        .await?
        .into_iter()
        .map(|row| row.get::<_, String>("name"))
        .collect::<Vec<String>>();

    let move_suggestions = client
        .query(move_req_query, &[&note_id])
        .await?
        .into_iter()
        .map(|row| NoteMoveReq {
            id: row.get("id"),
            path: row.get("full_path"),
        })
        .collect::<Vec<NoteMoveReq>>();
    current_note.note_move_decks = move_suggestions;

    current_note.note_model_fields = notetype_fields;

    let delete_req = client.query(delete_req_query, &[&note_id]).await?;
    current_note.delete_req = !delete_req.is_empty();

    let fields_rows = client.query(fields_query, &[&current_note.id]).await?;
    let tags_rows = client.query(tags_query, &[&current_note.id]).await?;

    // Fill reviewed_fields and unconfirmed_fields with dummy elements, set the position to the index of the field in the notetype
    for (index, _field) in current_note.note_model_fields.iter().enumerate() {
        current_note.reviewed_fields.push(FieldsInfo {
            id: 0,
            position: index as u32,
            content: String::new(),
            inherited: false,
        });
    }

    for row in fields_rows {
        let id = row.get(0);
        let position = row.get(1);
        let content = row.get(2);
        let reviewed = row.get(3);
        let commit_id = row.get(4);
        let clean_content = cleanser::clean(content);

        if reviewed {
            // Overwrite the dummy element with the actual data
            current_note.reviewed_fields[position as usize] = FieldsInfo {
                id,
                position,
                content: clean_content,
                inherited: false,
            };
        } else {
            let reviewed_cont = current_note.reviewed_fields[position as usize]
                .content
                .clone(); // This should work bc we sort by position and reviewed fields are first
            let diff = htmldiff::htmldiff(&reviewed_cont, &clean_content);
            current_note.unconfirmed_fields.push(FieldSuggestionInfo {
                id,
                position,
                content: clean_content,
                commit_id,
                diff,
            });
        }
    }

    for row in tags_rows {
        let id = row.get(0);
        let content = row.get(1);
        let reviewed = row.get(2);
        let action = row.get(3);
        let commit_id: i32 = row.get(4);
        if let Some(c) = content {
            let content = cleanser::clean(c);
            if reviewed {
                current_note.reviewed_tags.push(TagsInfo {
                    id,
                    content,
                    inherited: false,
                    commit_id,
                });
            } else if action {
                // New suggested tag
                current_note.new_tags.push(TagsInfo {
                    id,
                    content,
                    inherited: false,
                    commit_id,
                });
            } else {
                // Tag got removed
                current_note.removed_tags.push(TagsInfo {
                    id,
                    content,
                    inherited: false,
                    commit_id,
                });
            }
        }
    }

    // If this is an inherited note, overlay reviewed fields from the base note according to subscribed_fields
    if current_note.is_inherited {
        let inh_rows = client
            .query(
                "SELECT base_note_id, subscribed_fields, COALESCE(removed_base_tags, '{}') FROM note_inheritance WHERE subscriber_note_id = $1",
                &[&note_id],
            )
            .await?;
        if !inh_rows.is_empty() {
            let base_id: i64 = inh_rows[0].get(0);
            let subscribed_fields_opt: Option<Vec<i32>> = inh_rows[0].get(1);
            let removed_base_tags: Vec<String> = inh_rows[0].get(2);
            // Fetch base note reviewed fields
            let base_fields_rows = client
                .query(
                    "SELECT position::int, content FROM fields WHERE note = $1 AND reviewed = true",
                    &[&base_id],
                )
                .await?;
            let mut base_pos_map: std::collections::HashMap<i32, String> =
                std::collections::HashMap::new();
            for r in base_fields_rows {
                let pos: i32 = r.get(0);
                let content: String = r.get(1);
                base_pos_map.insert(pos, cleanser::clean(&content));
            }

            // Determine which positions to overwrite
            match subscribed_fields_opt {
                None => {
                    // Inherit all: overwrite any positions present in base_pos_map
                    for (pos, val) in base_pos_map.iter() {
                        if *pos >= 0 {
                            let idx = *pos as usize;
                            if idx < current_note.reviewed_fields.len() {
                                current_note.reviewed_fields[idx].content = val.clone();
                                current_note.reviewed_fields[idx].inherited = true;
                            }
                        }
                    }
                }
                Some(ords) => {
                    for ord in ords {
                        if ord >= 0 {
                            let idx = ord as usize;
                            if idx < current_note.reviewed_fields.len() {
                                if let Some(val) = base_pos_map.get(&ord) {
                                    current_note.reviewed_fields[idx].content = val.clone();
                                    current_note.reviewed_fields[idx].inherited = true;
                                }
                            }
                        }
                    }
                }
            }

            // Merge tags: base minus removed_base_tags, then union local reviewed tags
            // Build a set of local reviewed tags
            let mut local_set: std::collections::HashSet<String> = std::collections::HashSet::new();
            for t in &current_note.reviewed_tags {
                local_set.insert(t.content.clone());
            }
            // Load base reviewed tags
            let base_tags_rows = client
                .query(
                    "SELECT content FROM tags WHERE note = $1 AND reviewed = true",
                    &[&base_id],
                )
                .await?;
            let mut base_tags: std::collections::HashSet<String> = std::collections::HashSet::new();
            for r in base_tags_rows {
                let c: Option<String> = r.get(0);
                if let Some(cc) = c {
                    base_tags.insert(cleanser::clean(&cc));
                }
            }
            let removed_set: std::collections::HashSet<String> = removed_base_tags
                .into_iter()
                .map(|t| cleanser::clean(&t))
                .collect();
            let effective_base: std::collections::HashSet<String> =
                base_tags.difference(&removed_set).cloned().collect();
            let mut final_tags: std::collections::HashSet<String> =
                effective_base.union(&local_set).cloned().collect();

            // Replace reviewed_tags with merged, marking inherited ones
            current_note.reviewed_tags.clear();
            for tag in final_tags.drain() {
                let inherited = !local_set.contains(&tag);
                current_note.reviewed_tags.push(TagsInfo {
                    id: 0,
                    content: tag,
                    inherited,
                    commit_id: 0,
                });
            }

            // Recompute diffs of unconfirmed fields against the effective reviewed content (after overlay)
            for uf in &mut current_note.unconfirmed_fields {
                let reviewed_cont = if (uf.position as usize) < current_note.reviewed_fields.len() {
                    current_note.reviewed_fields[uf.position as usize]
                        .content
                        .clone()
                } else {
                    String::new()
                };
                uf.diff = htmldiff::htmldiff(&reviewed_cont, &uf.content);
            }
        }
    }
    Ok(current_note)
}

// Only show at most 1k cards. everything else is too much for the website to load. TODO Later: add incremental loading instead
pub async fn retrieve_notes(
    db_state: &Arc<database::AppState>,
    deck: &String,
) -> Return<Vec<Note>> {
    let query = r"
        SELECT n.id, n.guid,
            CASE
                WHEN n.reviewed = false THEN 0
                WHEN EXISTS (SELECT 1 FROM card_deletion_suggestions WHERE card_deletion_suggestions.note = n.id) THEN 1
                ELSE 2
            END AS status,
            TO_CHAR(n.last_update, 'MM/DD/YYYY') AS last_update,
            (SELECT coalesce(f.content, '') FROM fields AS f WHERE f.note = n.id AND f.position = 0 LIMIT 1) AS content
        FROM notes AS n
        INNER JOIN decks AS d ON n.deck = d.id
        WHERE d.human_hash = $1 AND n.deleted = false
        ORDER BY n.id ASC
        LIMIT 200;
    ";
    let client = database::client(db_state).await?;

    // Phase 1: load raw rows and build initial notes vector + id list
    let raw_rows = client.query(query, &[&deck]).await?;
    let mut notes: Vec<Note> = Vec::new();
    let mut note_ids: Vec<i64> = Vec::new();
    for row in raw_rows.iter() {
        if let Some(content) = row.get::<usize, Option<String>>(4) {
            let id: i64 = row.get(0);
            note_ids.push(id);
            notes.push(Note {
                id,
                guid: row.get(1),
                status: row.get(2),
                last_update: row.get(3),
                // Clean local content; may be overwritten below if inherited
                fields: cleanser::clean(&content),
            });
        }
    }

    // Phase 2: overlay inherited base content for field position 0, in batch
    if !note_ids.is_empty() {
        // Fetch inheritance links for visible notes
        let inh_rows = client
            .query(
                "SELECT subscriber_note_id, base_note_id, subscribed_fields FROM note_inheritance WHERE subscriber_note_id = ANY($1)",
                &[&note_ids],
            )
            .await?;

        if !inh_rows.is_empty() {
            use std::collections::HashMap;
            let mut inheritance_map: HashMap<i64, (i64, Option<Vec<i32>>)> = HashMap::new();
            let mut base_ids: Vec<i64> = Vec::new();
            for r in inh_rows {
                let sub_id: i64 = r.get(0);
                let base_id: i64 = r.get(1);
                let subs: Option<Vec<i32>> = r.get(2);
                if !inheritance_map.contains_key(&sub_id) {
                    inheritance_map.insert(sub_id, (base_id, subs));
                    base_ids.push(base_id);
                }
            }

            // Fetch base note reviewed content for field position 0 only
            let mut base_front_map: HashMap<i64, String> = HashMap::new();
            if !base_ids.is_empty() {
                let base_fields_rows = client
                    .query(
                        "SELECT note, content FROM fields WHERE note = ANY($1) AND position = 0 AND reviewed = true",
                        &[&base_ids],
                    )
                    .await?;
                for row in base_fields_rows {
                    let note_id: i64 = row.get(0);
                    let content: String = row.get(1);
                    base_front_map.insert(note_id, cleanser::clean(&content));
                }
            }

            // Apply overlay per note when subscribed to field 0 (or subscribe-all)
            for n in &mut notes {
                if let Some((base_id, subs_opt)) = inheritance_map.get(&n.id) {
                    let subscribe_all = subs_opt.is_none();
                    let subscribed_to_zero =
                        subs_opt.as_ref().map(|v| v.contains(&0)).unwrap_or(true);
                    if subscribe_all || subscribed_to_zero {
                        if let Some(base_content) = base_front_map.get(base_id) {
                            n.fields = base_content.clone();
                        }
                    }
                }
            }
        }
    }

    Ok(notes)
}

pub async fn deny_note_removal_request(
    db_state: &Arc<database::AppState>,
    note_id: i64,
    user: user::User,
) -> Result<String, Box<dyn std::error::Error>> {
    let client = database::client(db_state).await?;

    let q_guid = client
        .query("Select deck from notes where id = $1", &[&note_id])
        .await?;
    if q_guid.is_empty() {
        return Err("Note not found (Deny Note Removal Request).".into());
    }
    let deck_id: i64 = q_guid[0].get(0);

    let access = suggestion_manager::is_authorized(db_state, &user, deck_id).await?;
    if !access {
        return Err("Unauthorized.".into());
    }

    client
        .execute(
            "DELETE FROM card_deletion_suggestions WHERE note = $1",
            &[&note_id],
        )
        .await?;

    Ok(note_id.to_string())
}

// We skip a few steps if the caller is a bulk approve since they handle some stuff
pub async fn mark_note_deleted(
    tx: &tokio_postgres::Transaction<'_>,
    db_state: &Arc<database::AppState>,
    note_id: i64,
    user: user::User,
    bulk: bool,
) -> Return<String> {
    let q_guid = tx
        .query(
            "Select human_hash, id from decks where id = (select deck from notes where id = $1)",
            &[&note_id],
        )
        .await?;
    if q_guid.is_empty() {
        return Err(NoteNotFound(NoteNotFoundContext::MarkNoteDeleted));
    }
    let guid: String = q_guid[0].get(0);
    let deck_id: i64 = q_guid[0].get(1);

    if !bulk {
        let access = suggestion_manager::is_authorized(db_state, &user, deck_id).await?;
        if !access {
            return Err(Unauthorized);
        }
    }

    // Convert subscribers to local if this is a base note
    let inh_rows = tx.query(
        "SELECT subscriber_note_id, subscribed_fields, COALESCE(removed_base_tags, '{}') FROM note_inheritance WHERE base_note_id = $1",
        &[&note_id]
    ).await?;
    if !inh_rows.is_empty() {
        // Load base reviewed fields and tags
        let base_fields = tx
            .query(
                "SELECT position::int, content FROM fields WHERE note = $1 AND reviewed = true",
                &[&note_id],
            )
            .await?;
        let mut base_fields_map: std::collections::HashMap<i32, String> =
            std::collections::HashMap::new();
        for r in base_fields {
            let s: String = r.get(1);
            base_fields_map.insert(r.get(0), cleanser::clean(&s));
        }
        let base_tags_rows = tx
            .query(
                "SELECT content FROM tags WHERE note = $1 AND reviewed = true",
                &[&note_id],
            )
            .await?;
        let mut base_tags: std::collections::HashSet<String> = std::collections::HashSet::new();
        for r in base_tags_rows {
            let c: Option<String> = r.get(0);
            if let Some(cc) = c {
                base_tags.insert(cleanser::clean(&cc));
            }
        }

        for r in inh_rows {
            let sub_note_id: i64 = r.get(0);
            let subscribed_fields: Option<Vec<i32>> = r.get(1);
            let removed_base_tags: Vec<String> = r.get(2);

            // Determine positions to copy
            let positions: Vec<i32> = match subscribed_fields {
                None => base_fields_map.keys().cloned().collect(), // inherit all
                Some(arr) => arr
                    .into_iter()
                    .filter(|p| base_fields_map.contains_key(p))
                    .collect(),
            };

            // Replace subscriber reviewed fields for those positions
            if !positions.is_empty() {
                tx.execute(
                    "DELETE FROM fields WHERE reviewed = true AND note = $1 AND position::int = ANY($2)",
                    &[&sub_note_id, &positions],
                )
                .await?;
                for pos in &positions {
                    if let Some(content) = base_fields_map.get(pos) {
                        tx.execute(
                            "INSERT INTO fields (note, position, content, reviewed) VALUES ($1, $2, $3, true)",
                            &[&sub_note_id, &(*pos as u32), content]
                        ).await?;
                    }
                }
            }

            // Merge tags: (base_tags - removed_base_tags) U local_tags
            let local_tags_rows = tx
                .query(
                    "SELECT content FROM tags WHERE note = $1 AND reviewed = true",
                    &[&sub_note_id],
                )
                .await?;
            let mut local_tags: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            for rt in local_tags_rows {
                let c: Option<String> = rt.get(0);
                if let Some(cc) = c {
                    local_tags.insert(cleanser::clean(&cc));
                }
            }
            let removed_set: std::collections::HashSet<String> = removed_base_tags
                .into_iter()
                .map(|t| cleanser::clean(&t))
                .collect();
            let effective_base: std::collections::HashSet<String> =
                base_tags.difference(&removed_set).cloned().collect();
            let mut merged: std::collections::HashSet<String> =
                local_tags.union(&effective_base).cloned().collect();
            merged.insert("AnkiCollab::Base_note_deleted".to_string());

            // Replace subscriber reviewed tags with merged
            tx.execute(
                "DELETE FROM tags WHERE note = $1 AND reviewed = true",
                &[&sub_note_id],
            )
            .await?;
            for tag in merged {
                tx.execute(
                    "INSERT INTO tags (note, content, reviewed, action) VALUES ($1, $2, true, true)",
                    &[&sub_note_id, &tag]
                ).await?;
            }

            // Remove inheritance row and bump timestamp
            tx.execute(
                "DELETE FROM note_inheritance WHERE subscriber_note_id = $1",
                &[&sub_note_id],
            )
            .await?;
            suggestion_manager::update_note_timestamp(tx, sub_note_id).await?;
        }
    }

    // Update note flag
    let query = "UPDATE notes SET deleted = true WHERE id = $1";

    // Remove outstanding suggestions
    let query2 = "DELETE FROM fields WHERE note = $1 AND reviewed = false";
    let query3 = "DELETE FROM tags WHERE note = $1 AND reviewed = false";

    // Remove note from deletion_suggestions table
    let query4 = "DELETE FROM card_deletion_suggestions WHERE note = $1";

    // Remove note from move_suggestions table
    let query5 = "DELETE FROM note_move_suggestions WHERE note = $1";

    tx.execute(query, &[&note_id]).await?;
    tx.execute(query2, &[&note_id]).await?;
    tx.execute(query3, &[&note_id]).await?;
    tx.execute(query4, &[&note_id]).await?;
    tx.execute(query5, &[&note_id]).await?;

    if !bulk {
        // Update timestamp (media cleanup is caller's responsibility post-commit)
        suggestion_manager::update_note_timestamp(tx, note_id).await?;
    }
    // Log deletion event (always) - we treat this as a content change.
    let _ = note_history::log_event(
        tx,
        note_id,
        EventType::NoteDeleted,
        Some(&serde_json::json!({"deleted": false})),
        Some(&serde_json::json!({"deleted": true})),
        Some(user.id()),
        None,
        Some(true),
    )
    .await;
    Ok(guid)
}
