use crate::database;
use crate::structs::*;
use crate::suggestion_manager;

pub async fn under_review(uid: i32) -> Result<Vec<ReviewOverview>, Box<dyn std::error::Error>> {
    let query = r#"
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
    "#;
    let client = database::client().await;

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

pub async fn get_notes_count_in_deck(deck: i64) -> Result<i64, Box<dyn std::error::Error>> {
    let client = database::client().await;
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

pub async fn get_name_by_hash(deck: &String) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let client = database::client().await;

    let query = "SELECT name FROM decks WHERE human_hash = $1";
    let rows = client.query(query, &[&deck]).await?;

    if rows.is_empty() {
        return Err("Deck not found.".into());
    }

    let name: String = rows[0].get(0);
    Ok(Some(name))
}

pub async fn get_note_data(note_id: i64) -> Result<NoteData, Box<dyn std::error::Error>> {
    let client = database::client().await;

    let note_query = "
        SELECT id, guid, TO_CHAR(last_update, 'MM/DD/YYYY HH12:MI AM') AS last_update, reviewed, 
        (Select owner from decks where id = notes.deck), (select full_path from decks where id = notes.deck) as full_path, notetype
        FROM notes
        WHERE id = $1 AND deleted = false
    ";
    let fields_query = "
        SELECT id, position, content, reviewed
        FROM fields
        WHERE note = $1
        ORDER BY position
    ";
    let tags_query = "
        SELECT id, content, reviewed, action
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

    let mut current_note = NoteData {
        id: 0,
        guid: String::new(),
        owner: 0,
        deck: String::new(),
        last_update: String::new(),
        reviewed: false,
        delete_req: false,
        reviewed_fields: Vec::new(),
        reviewed_tags: Vec::new(),
        unconfirmed_fields: Vec::new(),
        new_tags: Vec::new(),
        removed_tags: Vec::new(),
        note_model_fields: Vec::new(),
    };

    let note_res = client.query_one(note_query, &[&note_id]).await?;
    let note_guid: String = note_res.get(1);
    let note_last_update: String = note_res.get(2);
    let note_reviewed: bool = note_res.get(3);
    let note_owner: i32 = note_res.get(4);
    let note_deck: String = note_res.get(5);
    let notetype: i64 = note_res.get(6);

    current_note.id = note_id;
    current_note.guid = note_guid;
    current_note.last_update = note_last_update;
    current_note.reviewed = note_reviewed;
    current_note.owner = note_owner;
    current_note.deck = note_deck;

    let notetype_fields = client
        .query(notetype_query, &[&notetype])
        .await?
        .into_iter()
        .map(|row| row.get::<_, String>("name"))
        .collect::<Vec<String>>();

    current_note.note_model_fields = notetype_fields;

    let delete_req = client.query(delete_req_query, &[&note_id]).await?;
    current_note.delete_req = !delete_req.is_empty();

    let fields_rows = client.query(fields_query, &[&current_note.id]).await?;
    let tags_rows = client.query(tags_query, &[&current_note.id]).await?;
    for row in fields_rows {
        let id = row.get(0);
        let position = row.get(1);
        let content = row.get(2);
        let reviewed = row.get(3);
        if let Some(content) = content {
            if reviewed {
                // make sure no dummy element already exists for this position (happens when the unconfirmed field gets evaluated BEFORE the reviewed one)
                current_note
                    .reviewed_fields
                    .retain(|info| info.position != position);
                current_note.reviewed_fields.push(FieldsInfo {
                    id,
                    position,
                    content: ammonia::clean(content),
                });
            } else {
                // For the html diff we need to make sure that every reviewed field index exists for a suggestion. Right now those can be NULL from the database (for unreviewed cards), so we need to fill it with dummies
                if !current_note
                    .reviewed_fields
                    .iter()
                    .any(|info| info.position == position)
                {
                    current_note.reviewed_fields.push(FieldsInfo {
                        id: 0,
                        position,
                        content: "".to_string(),
                    });
                }
                current_note.unconfirmed_fields.push(FieldsInfo {
                    id,
                    position,
                    content: content.to_owned(),
                });
            }
        }
    }
    for row in tags_rows {
        let id = row.get(0);
        let content = row.get(1);
        let reviewed = row.get(2);
        let action = row.get(3);
        if let Some(content) = content {
            if reviewed {
                current_note.reviewed_tags.push(TagsInfo { id, content });
            } else if action {
                // New suggested tag
                current_note.new_tags.push(TagsInfo { id, content });
            } else {
                // Tag got removed
                current_note.removed_tags.push(TagsInfo { id, content });
            }
        }
    }
    Ok::<NoteData, Box<dyn std::error::Error>>(current_note)
}

// Only show at most 1k cards. everything else is too much for the website to load. TODO Later: add incremental loading instead
pub async fn retrieve_notes(
    deck: &String,
) -> std::result::Result<Vec<Note>, Box<dyn std::error::Error>> {
    let query = r#"
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
    "#;
    let client = database::client().await;

    let rows = client
        .query(query, &[&deck])
        .await?
        .into_iter()
        .filter(|row| row.get::<usize, Option<String>>(4).is_some())
        .map(|row| Note {
            id: row.get(0),
            guid: row.get(1),
            status: row.get(2),
            last_update: row.get(3),
            fields: row.get::<usize, Option<String>>(4).unwrap(),
        })
        .collect::<Vec<Note>>(); // Collect into Vec<Note>

    Ok(rows)
}

pub async fn deny_note_removal_request(
    note_id: i64,
    user: rocket_auth::User,
) -> Result<String, Box<dyn std::error::Error>> {
    let client = database::client().await;

    let q_guid = client
        .query("Select deck from notes where id = $1", &[&note_id])
        .await?;
    if q_guid.is_empty() {
        return Err("Note not found (Deny Note Removal Request).".into());
    }
    let deck_id: i64 = q_guid[0].get(0);

    let access = suggestion_manager::is_authorized(&user, deck_id).await?;
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
    note_id: i64,
    user: rocket_auth::User,
    bulk: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut client = database::client().await;

    let q_guid = client
        .query(
            "Select human_hash, id from decks where id = (select deck from notes where id = $1)",
            &[&note_id],
        )
        .await?;
    if q_guid.is_empty() {
        return Err("Note not found (Mark Note Deleted).".into());
    }
    let guid: String = q_guid[0].get(0);
    let deck_id: i64 = q_guid[0].get(1);

    if !bulk {
        let access = suggestion_manager::is_authorized(&user, deck_id).await?;
        if !access {
            return Err("Unauthorized.".into());
        }
    }

    let tx = client.transaction().await?;

    // Update note flag
    let query = "UPDATE notes SET deleted = true WHERE id = $1";

    // Remove outstanding suggestions
    let query2 = "DELETE FROM fields WHERE note = $1 AND reviewed = false";
    let query3 = "DELETE FROM tags WHERE note = $1 AND reviewed = false";

    // Remove note from deletion_suggestions table
    let query4 = "DELETE FROM card_deletion_suggestions WHERE note = $1";

    tx.execute(query, &[&note_id]).await?;
    tx.execute(query2, &[&note_id]).await?;
    tx.execute(query3, &[&note_id]).await?;
    tx.execute(query4, &[&note_id]).await?;

    if !bulk {
        // Update timestamp
        suggestion_manager::update_note_timestamp(&tx, note_id).await?;
    }

    tx.commit().await?;
    Ok(guid)
}
