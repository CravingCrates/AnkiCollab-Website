
use rocket_auth::User;
use crate::database;

async fn update_note_timestamp(note_id: i64)  -> Result<(), Box<dyn std::error::Error>> { 
    let client = database::TOKIO_POSTGRES_POOL.get().unwrap().get().await.unwrap();
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

    let query2 = "UPDATE notes SET last_update = NOW() WHERE id = $1";

    client.query(query1, &[&note_id]).await?;
    client.query(query2, &[&note_id]).await?;

    Ok(())
}

pub async fn is_authorized(user: &User, deck: i64) -> Result<bool, Box<dyn std::error::Error>> {
    let client = database::TOKIO_POSTGRES_POOL.get().unwrap().get().await.unwrap();
    let rows = client.query("SELECT 1 FROM decks WHERE (owner = $1 AND id = $3) OR $2 LIMIT 1", &[&user.id(), &user.is_admin, &deck]).await?;
    let access = !rows.is_empty();

    // Check if its a maintainer
    if !access {
        // Get the topmost parent deck
        let query = r#"
            WITH RECURSIVE parent_decks AS (
                SELECT id, parent
                FROM decks
                WHERE id = $1
                UNION ALL
                SELECT decks.id, decks.parent
                FROM decks
                JOIN parent_decks ON decks.id = parent_decks.parent
            )
            SELECT id
            FROM parent_decks
            WHERE parent IS NULL          
        "#;
        let parent_deck = client.query(query, &[&deck]).await?;
        if parent_deck.is_empty() {
            return Ok(false);
        }
        let parent_deck: i64 = parent_deck[0].get(0);
        let rows = client.query("SELECT 1 FROM maintainers WHERE user_id = $1 AND deck = $2 LIMIT 1", &[&user.id(), &parent_deck]).await?;
        return Ok(!rows.is_empty());
    }

    Ok(access)
}

pub async fn delete_card(note_id: i64, user: User) -> Result<String, Box<dyn std::error::Error>> {
    let client = database::TOKIO_POSTGRES_POOL.get().unwrap().get().await.unwrap();
    
    let q_guid = client.query("Select human_hash, id from decks where id = (select deck from notes where id = $1)", &[&note_id]).await?;
    if q_guid.is_empty() {
        return Err("Note not found (Delete Card).".into());
    }
    let guid: String = q_guid[0].get(0);
    let deck_id: i64 = q_guid[0].get(1);

    let access = is_authorized(&user, deck_id).await?;
    if !access {
        return Err("Unauthorized.".into());
    }

    client.query("DELETE FROM notes CASCADE WHERE id = $1", &[&note_id]).await?;

    Ok(guid)
}

pub async fn approve_card(note_id: i64, user: User) -> Result<String, Box<dyn std::error::Error>> {
    let client = database::TOKIO_POSTGRES_POOL.get().unwrap().get().await.unwrap();
    
    let q_guid = client.query("select deck from notes where id = $1", &[&note_id]).await?;
    if q_guid.is_empty() {
        return Err("Note not found (Approve Card).".into());
    }
    let deck_id: i64 = q_guid[0].get(0);

    let access = is_authorized(&user, deck_id).await?;
    if !access {
        return Err("Unauthorized.".into());
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
    client.query("UPDATE tags SET reviewed = true WHERE note = $1", &[&note_id]).await?;
    client.query("UPDATE notes SET reviewed = true WHERE id = $1", &[&note_id]).await?;

    update_note_timestamp(note_id).await?;
    
    Ok(note_id.to_string())
}

pub async fn deny_tag_change(tag_id: i64) -> Result<String, Box<dyn std::error::Error>>  {
    let client = database::TOKIO_POSTGRES_POOL.get().unwrap().get().await.unwrap();
    
    let rows = client.query("SELECT note FROM tags WHERE id = $1", &[&tag_id]).await?;

    if rows.is_empty() {
        return Err("Note not found (Tag denied).".into());
    }

    client.query("DELETE FROM tags WHERE id = $1", &[&tag_id]).await?;
    
    let note_id: i64 = rows[0].get(0);
    Ok(note_id.to_string())
}

pub async fn deny_field_change(field_id: i64) -> Result<String, Box<dyn std::error::Error>>  {
    let client = database::TOKIO_POSTGRES_POOL.get().unwrap().get().await.unwrap();
    
    let rows = client.query("SELECT note FROM fields WHERE id = $1", &[&field_id]).await?;

    if rows.is_empty() {
        return Err("Note not found (Field Denied).".into());
    }

    client.query("DELETE FROM fields WHERE id = $1", &[&field_id]).await?;
    
    let note_id: i64 = rows[0].get(0);
    Ok(note_id.to_string())
}

pub async fn approve_tag_change(tag_id: i64, update_timestamp: bool) -> Result<String, Box<dyn std::error::Error>> {
    let client = database::TOKIO_POSTGRES_POOL.get().unwrap().get().await.unwrap();
    
    let rows = client.query("SELECT note FROM tags WHERE id = $1", &[&tag_id]).await?;
    if rows.is_empty() {
        return Err("Note not found (Tag Approve).".into());
    }
    let note_id: i64 = rows[0].get(0);

    let update_query = "UPDATE tags SET reviewed = true WHERE id = $1 AND action = true";    
    let delete_query = "
    WITH hit AS (
        SELECT content, note 
        FROM tags WHERE id = $1 AND action = false
    )
    DELETE FROM tags WHERE note in (select note from hit) and content in (select content from hit)";

    client.query(update_query, &[&tag_id]).await?;
    client.query(delete_query, &[&tag_id]).await?;

    if update_timestamp {
        update_note_timestamp(note_id).await?;
    }
    
    Ok(note_id.to_string())
}

pub async fn approve_field_change(field_id: i64, update_timestamp: bool) -> Result<String, Box<dyn std::error::Error>>  {
    let client = database::TOKIO_POSTGRES_POOL.get().unwrap().get().await.unwrap();

    let rows = client.query("SELECT note FROM fields WHERE id = $1", &[&field_id]).await?;

    if rows.is_empty() {
        return Err("Note not found (Field Approve).".into());
    }

    let note_id: i64 = rows[0].get(0);

    let query1 = "
        DELETE FROM fields
        WHERE reviewed = true
        AND position = (SELECT position FROM fields WHERE id = $1)
        AND id <> $1
        AND note = $2
    ";
    let query2 = "
        UPDATE fields
        SET reviewed = true
        WHERE id = $1
    ";

    client.query(query1, &[&field_id, &note_id]).await?;
    client.query(query2, &[&field_id]).await?;

    if update_timestamp {
        update_note_timestamp(note_id).await?;
    }
    
    Ok(note_id.to_string())
}

pub async fn merge_by_commit(commit_id: i32, approve: bool, user: User) -> Result<String, Box<dyn std::error::Error>> {
    let client = database::TOKIO_POSTGRES_POOL.get().unwrap().get().await.unwrap();

    let q_guid = client.query("Select deck from commits where commit_id = $1", &[&commit_id]).await?;
    if q_guid.is_empty() {
        return Err("Deck in Commit not found (Merge Commit).".into());
    }
    let deck_id: i64 = q_guid[0].get(0);

    let access = is_authorized(&user, deck_id).await?;
    if !access {
        return Err("Unauthorized.".into());
    }

    let affected_tags = client.query("
        SELECT id FROM tags WHERE commit = $1 and reviewed = false
    ", &[&commit_id])
    .await?.into_iter().map(|row| row.get::<_, i64>("id")).collect::<Vec<i64>>();

    let affected_fields = client.query("
        SELECT id FROM fields WHERE commit = $1 and reviewed = false
    ", &[&commit_id])
    .await?.into_iter().map(|row| row.get::<_, i64>("id")).collect::<Vec<i64>>();

    let new_notes = client.query("
        SELECT notes.id FROM notes
        JOIN (
            SELECT note FROM fields WHERE commit = $1 and reviewed = false
            UNION
            SELECT note FROM tags WHERE commit = $1 and reviewed = false
        ) AS n ON notes.id = n.note
        WHERE reviewed = false
    ", &[&commit_id])
    .await?.into_iter().map(|row| row.get::<_, i64>("id")).collect::<Vec<i64>>();

    let changed_notes = client.query("
        SELECT note FROM (
            SELECT note FROM fields WHERE commit = $1 and reviewed = false
            UNION ALL
            SELECT note FROM tags WHERE commit = $1 and reviewed = false
        ) AS n
        GROUP BY note
    ", &[&commit_id])
    .await?.into_iter().map(|row| row.get::<_, i64>("note")).collect::<Vec<i64>>();

    // Slightly less performant to do it in single queries than doing a bigger query here, but for readability and easier code maintenance, we keep it that way. 
    // The performance difference is not relevant in this case
    if approve {
        let note_counter = changed_notes.len();
        if note_counter > 100 {
            println!("Trying to approve large commit {}.", commit_id);
        }

        for tag in affected_tags {
            approve_tag_change(tag, false).await?;
        }

        for field in affected_fields {
            approve_field_change(field, false).await?;
        }

        for note in new_notes {
            client.query("UPDATE notes SET reviewed = true WHERE id = $1", &[&note]).await?;                
        }

        for note in changed_notes {            
            update_note_timestamp(note).await?;      
        }

        if note_counter > 100 {
            println!("Merge commit approved.");
        }

    } else {
        for tag in affected_tags {
            deny_tag_change(tag).await?;
        }

        for field in affected_fields {
            deny_field_change(field).await?;
        }

        for note in new_notes {
            client.query("DELETE FROM notes cascade WHERE id = $1", &[&note]).await?;        
        }
    }

    Ok("Success".into())
}