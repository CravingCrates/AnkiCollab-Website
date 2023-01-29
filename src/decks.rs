
use std::collections::HashSet;

use rocket_auth::User;

use crate::database;
use crate::structs::*;

async fn update_note_timestamp(note_id: i64)  -> Result<(), Box<dyn std::error::Error>> { 
    let client = unsafe { database::TOKIO_POSTGRES_CLIENT.as_mut().unwrap() };
    let query1 = "
    WITH RECURSIVE tree AS (
        SELECT id, last_update, parent FROM decks
        WHERE id = (SELECT deck FROM notes WHERE id = $1)
        UNION ALL
        SELECT d.id, d.last_update, d.parent FROM decks d
        JOIN tree t ON d.id = t.parent
    )
    UPDATE decks
    SET last_update = NOW()
    WHERE id IN (SELECT id FROM tree)";
    client.query(query1, &[&note_id]).await?;

    let query2 = "UPDATE notes SET last_update = NOW() WHERE id = $1";
    client.query(query2, &[&note_id]).await?;
    Ok(())
}

pub async fn get_note_model_info(user_id: i32) -> Result<Vec<NoteModel>, Box<dyn std::error::Error>> {
    let client = unsafe { database::TOKIO_POSTGRES_CLIENT.as_mut().unwrap() };
    let rows = client.query(
        "WITH decks AS (
           SELECT id, owner, parent
           FROM decks
           WHERE owner = $1
         ), notes AS (
           SELECT n.id, n.notetype, n.deck
           FROM notes n
           INNER JOIN decks d ON n.deck = d.id
         ), notetype AS (
           SELECT nt.id, nt.name
           FROM notetype nt
           INNER JOIN notes n ON nt.id = n.notetype
         )
         SELECT DISTINCT nt.id, nt.name, ntf.id, ntf.name, ntf.protected, ntf.position
         FROM notetype nt
         LEFT JOIN notetype_field ntf ON nt.id = ntf.notetype
         ORDER BY nt.id, ntf.position",
        &[&user_id],
    ).await?;

    let mut note_models = Vec::new();
    let mut current_note_model = NoteModel {
        id: 0,
        fields: Vec::new(),
        name: String::new(),
    };
    for row in rows {
        let notetype_id: i64 = row.get(0);
        let notetype_name: &str = row.get(1);
        let field_id: i64 = row.get(2);
        let field_name: &str = row.get(3);
        let field_protected: bool = row.get(4);

        let current_note_model_id = current_note_model.id;
        if current_note_model_id == 0 || current_note_model_id != notetype_id {
            if !current_note_model.fields.is_empty() {
                note_models.push(current_note_model);
            }
            current_note_model = NoteModel {
                id: notetype_id,
                fields: Vec::new(),
                name: notetype_name.to_owned(),
            };
        }

        current_note_model.fields.push(NoteModelFieldInfo {
            id: field_id,
            name: field_name.to_owned(),
            protected: field_protected,
        });
    }

    if !current_note_model.fields.is_empty() {
        note_models.push(current_note_model);
    }

    Ok(note_models)
}

pub async fn approve_tag_change(tag_id: i64, user: User) -> Result<String, Box<dyn std::error::Error>> {
    let client = unsafe { database::TOKIO_POSTGRES_CLIENT.as_mut().unwrap() };
    
    let rows = client.query("SELECT id from notes where id = (Select note from tags where id = $1)", &[&tag_id]).await?;
    if rows.is_empty() {
        return Err("Note not found.".into());
    }
    let note_id: i64 = rows[0].get(0);

    let update_query = "
    UPDATE tags SET reviewed = true WHERE id = $1 AND id IN (
        SELECT id FROM tags WHERE id = $1 AND note IN (
            SELECT n.id FROM tags t JOIN notes n ON t.note = n.id WHERE t.id = $1 AND (n.deck IN (SELECT id FROM decks WHERE owner = $2) OR $3)
        ) AND action = true
    )";
    
    let delete_query = "
    WITH hit AS (
        SELECT content, note 
        FROM tags WHERE id = $1 AND note IN (
            SELECT n.id FROM tags t 
            JOIN notes n ON t.note = n.id WHERE t.id = $1 AND (n.deck IN (SELECT id FROM decks WHERE owner = $2) OR $3)
        ) AND action = false
    )
    DELETE FROM tags WHERE note in (select note from hit) and content in (select content from hit)        
    ";

    let trans = client.transaction().await?;
    trans.execute(update_query, &[&tag_id, &user.id(), &user.is_admin]).await?;
    trans.execute(delete_query, &[&tag_id, &user.id(), &user.is_admin]).await?;
    trans.commit().await?;

   
    update_note_timestamp(note_id).await?;
    Ok(note_id.to_string())
}

pub async fn delete_card(note_id: i64, user: User) -> Result<String, Box<dyn std::error::Error>> {
    let client = unsafe { database::TOKIO_POSTGRES_CLIENT.as_mut().unwrap() };
    
    let owner_check_row = client.query("SELECT 1 FROM decks WHERE (owner = $1 OR $2) AND id = (Select deck from notes where id = $3)", &[&user.id(), &user.is_admin, &note_id]).await?;
    if owner_check_row.is_empty() {
        println!("Access denied");
        return Err("Access denied.".into());
    }
    
    let q_guid = client.query("Select human_hash from decks where id = (select deck from notes where id = $1)", &[&note_id]).await?;
    let guid: String = q_guid[0].get(0);

    client.query(
    "DELETE FROM fields
     WHERE note = $1
     AND note IN (SELECT n.id FROM fields f JOIN notes n ON f.note = n.id
                   WHERE (n.deck IN (SELECT id FROM decks WHERE owner = $2) OR $3)
                 )
    ", &[&note_id, &user.id(), &user.is_admin]).await?;
    
    client.query(
        "DELETE FROM tags
         WHERE note = $1
         AND note IN (SELECT n.id FROM fields f JOIN notes n ON f.note = n.id
                       WHERE (n.deck IN (SELECT id FROM decks WHERE owner = $2) OR $3)
                     )
        ", &[&note_id, &user.id(), &user.is_admin]).await?;
    
    client.query(
        "DELETE FROM notes WHERE id = $1 AND (deck IN (SELECT id FROM decks WHERE owner = $2) OR $3)
        ", &[&note_id, &user.id(), &user.is_admin]).await?;

    Ok(guid)
}

pub async fn approve_card(note_id: i64, user: User) -> Result<String, Box<dyn std::error::Error>> {
    let client = unsafe { database::TOKIO_POSTGRES_CLIENT.as_mut().unwrap() };
    
    let owner_check_row = client.query("SELECT 1 FROM decks WHERE (owner = $1 OR $2) AND id = (Select deck from notes where id = $3)", &[&user.id(), &user.is_admin, &note_id]).await?;
    if owner_check_row.is_empty() {
        println!("Access denied");
        return Err("Access denied.".into());
    }

    // Check if the fields are valid
    let unique_fields_row = client.query(
        "
        SELECT (
            (
              SELECT COUNT(*)
              FROM notetype_field
              WHERE notetype = (SELECT notetype FROM notes WHERE id = $1) and protected = false
            ) = (
              SELECT COUNT(*)
              FROM fields
              WHERE note = $1
            ) AND (
              SELECT NOT EXISTS (
                SELECT 1
                FROM fields
                WHERE note = $1
                GROUP BY position
                HAVING COUNT(*) > 1
              )
            )
          ) AS result;
        ", &[&note_id]).await?;
    if unique_fields_row.is_empty() {
        println!("Note invalid");
        return Err("Note is invalid.".into());
    }

    if !unique_fields_row[0].get::<_, bool>(0) {
        println!("Field ambiguous");
        return Err("Fields are ambiguous. Please handle manually.".into());
    }

    client.query("UPDATE fields SET reviewed = true WHERE note = $1", &[&note_id]).await?;
    client.query("UPDATE notes SET reviewed = true WHERE id = $1", &[&note_id]).await?;
    client.query("UPDATE tags SET reviewed = true WHERE note = $1", &[&note_id]).await?;

    update_note_timestamp(note_id).await?;
    Ok(note_id.to_string())
}

pub async fn deny_tag_change(tag_id: i64, user: User) -> Result<String, Box<dyn std::error::Error>>  {
    let client = unsafe { database::TOKIO_POSTGRES_CLIENT.as_mut().unwrap() };
    
    let rows = client.query("SELECT id from notes where id = (Select note from tags where id = $1)", &[&tag_id]).await?;

    if rows.is_empty() {
        return Err("Note not found.".into());
    }

    let query = "
    DELETE FROM tags
    WHERE id = $1
      AND note IN (SELECT n.id
                   FROM tags t
                   JOIN notes n ON t.note = n.id
                   WHERE t.id = $1
                     AND (n.deck IN (SELECT id FROM decks WHERE owner = $2) OR $3)
                  )
    ";
    client.query(query, &[&tag_id, &user.id(), &user.is_admin]).await?;
    
    let note_id: i64 = rows[0].get(0);
    Ok(note_id.to_string())
}

pub async fn deny_field_change(field_id: i64, user: User) -> Result<String, Box<dyn std::error::Error>>  {
    let client = unsafe { database::TOKIO_POSTGRES_CLIENT.as_mut().unwrap() };
    
    let rows = client.query("SELECT id from notes where id = (Select note from fields where id = $1)", &[&field_id]).await?;

    if rows.is_empty() {
        return Err("Note not found.".into());
    }

    let query = "
    DELETE FROM fields
    WHERE id = $1
      AND note IN (SELECT n.id
                   FROM fields f
                   JOIN notes n ON f.note = n.id
                   WHERE f.id = $1
                     AND (n.deck IN (SELECT id FROM decks WHERE owner = $2) OR $3)
                  )
    ";
    client.query(query, &[&field_id, &user.id(), &user.is_admin]).await?;
    
    let note_id: i64 = rows[0].get(0);
    Ok(note_id.to_string())
}


pub async fn approve_field_change(field_id: i64, user: User) -> Result<String, Box<dyn std::error::Error>>  {
    let client = unsafe { database::TOKIO_POSTGRES_CLIENT.as_mut().unwrap() };
    let query1 = "
    DELETE FROM fields
    WHERE reviewed = true
      AND position = (SELECT position FROM fields WHERE id = $1)
      AND note IN (SELECT n.id
                   FROM fields f
                   JOIN notes n ON f.note = n.id
                   WHERE f.id = $1
                     AND (n.deck IN (SELECT id FROM decks WHERE owner = $2) OR $3)
                  )
    ";
    let query2 = "
    UPDATE fields
    SET reviewed = true
    WHERE id = $1
    AND note IN (SELECT n.id
        FROM fields f
        JOIN notes n ON f.note = n.id
        WHERE f.id = $1
          AND (n.deck IN (SELECT id FROM decks WHERE owner = $2) OR $3)
       )
    ";
    client.query(query1, &[&field_id, &user.id(), &user.is_admin]).await?;
    client.query(query2, &[&field_id, &user.id(), &user.is_admin]).await?;

    let rows = client.query("SELECT id from notes where id = (Select note from fields where id = $1)", &[&field_id]).await?;

    if rows.is_empty() {
        return Err("Note not found.".into());
    }

    let note_id: i64 = rows[0].get(0);
    update_note_timestamp(note_id).await?;
    Ok(note_id.to_string())
}

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
        _ => "Unknown Rationale",
    }
}

pub async fn commits_review(uid: i32) -> Result<Vec<CommitsOverview>, Box<dyn std::error::Error>> {
    
    let query = r#"
        WITH owned AS (
            Select id, name from decks where owner = $1
        )
        SELECT c.commit_id, c.rationale,
        TO_CHAR(c.timestamp, 'MM/DD/YYYY') AS last_update,
        d.name
        FROM commits AS c
        LEFT JOIN owned d ON d.id = c.deck
        WHERE
            c.deck in (select id from owned)            
        GROUP BY c.commit_id, c.rationale, d.name
        ORDER BY c.commit_id ASC
    "#;
    let client = unsafe { database::TOKIO_POSTGRES_CLIENT.as_mut().unwrap() };

    let rows = client.query(query, &[&uid])
    .await?
    .into_iter()
    .map(|row| CommitsOverview {
    id: row.get(0),
    rationale: get_string_from_rationale(row.get(1)).into(),
    timestamp: row.get(2),
    deck: row.get(3)
    })
    .collect::<Vec<_>>();

    Ok(rows)
}

pub async fn under_review(uid: i32) -> Result<Vec<ReviewOverview>, Box<dyn std::error::Error>> {
    let query = r#"
        WITH owned AS (
            Select id, full_path from decks where owner = $1
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
        ORDER BY n.id ASC
    "#;
    let client = unsafe { database::TOKIO_POSTGRES_CLIENT.as_mut().unwrap() };

    let rows = client.query(query, &[&uid])
    .await?
    .into_iter()
    .map(|row| ReviewOverview {
    id: row.get(0),
    guid: row.get(1),
    full_path: row.get(2),
    status: row.get(3),
    last_update: row.get(4),
    fields: row.get(5)
    })
    .collect::<Vec<_>>();

    Ok(rows)
}

pub async fn get_notes_count_in_deck(deck: i64) -> Result<i64, Box<dyn std::error::Error>> {
    let client = unsafe { database::TOKIO_POSTGRES_CLIENT.as_mut().unwrap() };
    let query = "
        WITH RECURSIVE cte AS (
            SELECT $1::bigint as id
            UNION ALL
            SELECT d.id
            FROM cte JOIN decks d ON d.parent = cte.id
        )
        SELECT COUNT(*) as num FROM notes WHERE deck IN (SELECT id FROM cte)
    ";
    let rows = client.query(query, &[&deck]).await?;

    let count: i64 = rows[0].get(0);
    Ok(count)
}

pub async fn get_name_by_hash(deck: &String) -> Result<Option<String>, Box<dyn std::error::Error>> {
    
    let client = unsafe { database::TOKIO_POSTGRES_CLIENT.as_mut().unwrap() };

    let query = "SELECT name FROM decks WHERE human_hash = $1";
    let rows = client.query(query, &[&deck]).await?;

    if rows.is_empty() {
        return Err("Deck not found.".into());
    }

    let name: String = rows[0].get(0);
    Ok(Some(name))
}

pub async fn get_note_data(note_id: i64) -> Result<NoteData, Box<dyn std::error::Error>> {
    let client = unsafe { database::TOKIO_POSTGRES_CLIENT.as_mut().unwrap() };

    let note_query = "
        SELECT id, guid, TO_CHAR(last_update, 'MM/DD/YYYY HH12:MI AM') AS last_update, reviewed, 
        (Select owner from decks where id = notes.deck), (select full_path from decks where id = notes.deck) as full_path
        FROM notes
        WHERE id = $1
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

    let mut current_note = NoteData {
        id: 0,
        guid: String::new(),
        owner: 0,
        deck: String::new(),
        last_update: String::new(),
        reviewed: false,
        reviewed_fields: Vec::new(),
        reviewed_tags: Vec::new(),
        unconfirmed_fields: Vec::new(),
        new_tags: Vec::new(),
        removed_tags: Vec::new(),
    };

    let note_res = client.query_one(note_query, &[&note_id]).await?;
    let note_guid: String = note_res.get(1);
    let note_last_update: String = note_res.get(2);
    let note_reviewed: bool = note_res.get(3);
    let note_owner: i32 = note_res.get(4);
    let note_deck: String = note_res.get(5);

    current_note.id = note_id;
    current_note.guid = note_guid;
    current_note.last_update = note_last_update;
    current_note.reviewed = note_reviewed;
    current_note.owner = note_owner;
    current_note.deck = note_deck;

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
                current_note.reviewed_fields.retain(|info| info.position != position);
                current_note.reviewed_fields.push(FieldsInfo { id, position, content: ammonia::clean(content) });
            } else {
                // For the html diff we need to make sure that every reviewed field index exists for a suggestion. Right now those can be NULL from the database (for unreviewed cards), so we need to fill it with dummies
                if !current_note.reviewed_fields.iter().any(|info| info.position == position)
                {
                    current_note.reviewed_fields.push(FieldsInfo { id:0, position, content:"".to_string() });
                }
                current_note.unconfirmed_fields.push(FieldsInfo { id, position, content: content.to_owned() });
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
                current_note.reviewed_tags.push(TagsInfo {id, content});
            } else {
                if action { // New suggested tag
                    current_note.new_tags.push(TagsInfo {id, content});
                } else { // Tag got removed                    
                    current_note.removed_tags.push(TagsInfo {id, content});
                }
            }
        }
    }
    Ok::<NoteData, Box<dyn std::error::Error>>(current_note)
}

pub async fn retrieve_notes(deck: &String) -> std::result::Result<Vec<Note>, Box<dyn std::error::Error>> {
    let query = r#"
                SELECT n.id, n.guid,
                (CASE
                    WHEN n.reviewed = false THEN 0
                    WHEN n.reviewed = true AND EXISTS (SELECT 1 FROM fields WHERE fields.note = n.id AND fields.reviewed = false) THEN 1
                    WHEN n.reviewed = true AND EXISTS (SELECT 1 FROM tags WHERE tags.note = n.id AND tags.reviewed = false) THEN 1
                    ELSE 2
                END) AS status,
                TO_CHAR(n.last_update, 'MM/DD/YYYY') AS last_update,
                coalesce(string_agg(f.content, ','), '') AS content
                FROM notes AS n                
                LEFT JOIN notetype AS nt ON n.notetype = nt.id
                LEFT JOIN fields AS f ON n.id = f.note
		WHERE n.deck = (Select id from decks where human_hash = $1)
                GROUP BY n.id, n.guid, n.reviewed
                ORDER BY n.id ASC
        "#;
    let client = unsafe { database::TOKIO_POSTGRES_CLIENT.as_mut().unwrap() };
    
    let rows = client.query(query, &[&deck])
    .await?
    .into_iter()
    .map(|row| Note {
        id: row.get(0),
        guid: row.get(1),
        status: row.get(2),
        last_update: row.get(3),
        fields: row.get(4)
    })
    .collect::<Vec<_>>();

    Ok(rows)

}
