use std::sync::Arc;

use crate::cleanser;
use crate::error::Error::*;
use crate::error::NoteNotFoundContext;
use crate::{database, note_manager, Return};
use crate::user::User;
use crate::media_reference_manager;


pub async fn update_note_timestamp(
    tx: &tokio_postgres::Transaction<'_>,
    note_id: i64,
) -> Return<()> {
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

    tx.query(query1, &[&note_id]).await?;
    tx.query(query2, &[&note_id]).await?;

    Ok(())
}

pub async fn is_authorized(db_state: &Arc<database::AppState>,user: &User, deck: i64) -> Return<bool> {
    let client = database::client(db_state).await?;
    let rows = client
        .query(
            "SELECT 1 FROM decks WHERE (owner = $1 AND id = $3) OR $2 LIMIT 1",
            &[&user.id(), &user.is_admin, &deck],
        )
        .await?;
    let access = !rows.is_empty();

    // Check if it's a maintainer
    if !access {
        // Get all parent decks including the current one
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
        "#;
        let parent_decks = client.query(query, &[&deck]).await?;
        if parent_decks.is_empty() {
            return Ok(false);
        }
        // Check if the user is a maintainer for any of the parent decks
        for row in parent_decks {
            let parent_deck_id: i64 = row.get(0);
            let rows = client
                .query(
                    "SELECT 1 FROM maintainers WHERE user_id = $1 AND deck = $2 LIMIT 1",
                    &[&user.id(), &parent_deck_id],
                )
                .await?;
            if !rows.is_empty() {
                // User is a maintainer for this deck or one of its parents
                return Ok(true);
            }
        }
        // User is not a maintainer for any of the decks in the hierarchy
        return Ok(false);
    }

    Ok(access)
}

// Only used for unreviewed cards to prevent them from being added to the deck. Existing cards should use mark_note_deleted instead
pub async fn delete_card(db_state: &Arc<database::AppState>,note_id: i64, user: User) -> Return<String> {
    let client = database::client(db_state).await?;

    let q_guid = client
        .query(
            "Select human_hash, id from decks where id = (select deck from notes where id = $1)",
            &[&note_id],
        )
        .await?;
    if q_guid.is_empty() {
        return Err(NoteNotFound(NoteNotFoundContext::DeleteCard));
    }
    let guid: String = q_guid[0].get(0);
    let deck_id: i64 = q_guid[0].get(1);

    let access = is_authorized(db_state, &user, deck_id).await?;
    if !access {
        return Err(Unauthorized);
    }

    client
        .query("DELETE FROM notes CASCADE WHERE id = $1", &[&note_id])
        .await?;

    // Clean up media references should be unnecessary since the card is deleted with cascade and we have a oreign key

    Ok(guid)
}

// If bulk is true, we skip a few steps that have already been handled by the caller
pub async fn approve_card(db_state: &Arc<database::AppState>,note_id: i64, user: User, bulk: bool) -> Return<String> {
    let mut client = database::client(db_state).await?;
    let tx = client.transaction().await?;

    let q_guid = tx
        .query("select deck from notes where id = $1", &[&note_id])
        .await?;
    if q_guid.is_empty() {
        return Err(NoteNotFound(NoteNotFoundContext::ApproveCard));
    }
    let deck_id: i64 = q_guid[0].get(0);

    if !bulk {
        let access = is_authorized(db_state, &user, deck_id).await?;
        if !access {
            return Err(Unauthorized);
        }
    }

    // Check if the fields are valid
    let unique_fields_row = tx
        .query(
            "SELECT NOT EXISTS (
                SELECT 1
                FROM fields
                WHERE note = $1
                GROUP BY position
                HAVING COUNT(*) > 1
            )",&[&note_id],
        )
        .await?;
    if unique_fields_row.is_empty() {
        return Err(InvalidNote);
    }

    if !unique_fields_row[0].get::<_, bool>(0) {
        return Err(AmbiguousFields(note_id));
    }

    if !bulk {
        tx.query(
            "UPDATE fields SET reviewed = true WHERE note = $1",
            &[&note_id],
        )
        .await?;
        tx.query(
            "UPDATE tags SET reviewed = true WHERE note = $1",
            &[&note_id],
        )
        .await?;
    }

    tx.query(
        "UPDATE notes SET reviewed = true WHERE id = $1",
        &[&note_id],
    )
    .await?;

    if !bulk {
        update_note_timestamp(&tx, note_id).await?;

        // Update media references after approval
        let state_clone = db_state.clone();
        tokio::spawn(async move {
            if let Err(e) = media_reference_manager::update_media_references_for_approved_note(&state_clone, note_id).await {
                println!("Error updating media references: {:?}", e);
                // Continue anyway since the card has been approved
            }
        });

    }

    tx.commit().await?;

    Ok(note_id.to_string())
}

pub async fn deny_note_move_request(db_state: &Arc<database::AppState>, move_id: i32) -> Return<String> {
    let client = database::client(db_state).await?;

    let rows = client
        .query("SELECT note FROM note_move_suggestions WHERE id = $1", &[&move_id])
        .await?;

    if rows.is_empty() {
        return Err(NoteNotFound(NoteNotFoundContext::NoteMovalRequest));
    }

    client
        .query("DELETE FROM note_move_suggestions WHERE id = $1", &[&move_id])
        .await?;

    let note_id: i64 = rows[0].get(0);
    Ok(note_id.to_string())
}

pub async fn deny_tag_change(db_state: &Arc<database::AppState>,tag_id: i64) -> Return<String> {
    let client = database::client(db_state).await?;

    let rows = client
        .query("SELECT note FROM tags WHERE id = $1", &[&tag_id])
        .await?;

    if rows.is_empty() {
        return Err(NoteNotFound(NoteNotFoundContext::TagDenied));
    }

    client
        .query("DELETE FROM tags WHERE id = $1", &[&tag_id])
        .await?;

    let note_id: i64 = rows[0].get(0);
    Ok(note_id.to_string())
}

pub async fn deny_field_change(db_state: &Arc<database::AppState>,field_id: i64, update_media_references: bool) -> Return<String> {
    let client = database::client(db_state).await?;

    let rows = client
        .query("SELECT note FROM fields WHERE id = $1", &[&field_id])
        .await?;

    if rows.is_empty() {
        return Err(NoteNotFound(NoteNotFoundContext::FieldDenied));
    }

    client
        .query("DELETE FROM fields WHERE id = $1", &[&field_id])
        .await?;

    let note_id: i64 = rows[0].get(0);

    if update_media_references {    
        let state_clone = db_state.clone();
        tokio::spawn(async move {
            if let Err(e) = media_reference_manager::update_media_references_note_state(&state_clone, note_id).await {
                println!("Error updating media references (3): {:?}", e);
            }
        });
    }
    Ok(note_id.to_string())
}

pub async fn approve_move_note_request_by_moveid(db_state: &Arc<database::AppState>, move_id: i32) -> Return<String> {
    let client = database::client(db_state).await?;
    let rows = client
        .query("SELECT note, target_deck FROM note_move_suggestions WHERE id = $1", &[&move_id])
        .await?;

    if rows.is_empty() {
        return Err(NoteNotFound(NoteNotFoundContext::TagApprove));
    }
    let note_id: i64 = rows[0].get(0);
    let target_id: i64 = rows[0].get(1);

    approve_move_note_request(db_state, note_id, target_id, true).await?;

    Ok(note_id.to_string())
}

pub async fn approve_move_note_request(db_state: &Arc<database::AppState>, note_id: i64, target_deck: i64, update_timestamp: bool) -> Return<String> {
    let mut client = database::client(db_state).await?;
    let tx = client.transaction().await?;

    tx.execute("UPDATE notes SET deck = $1 WHERE id = $2", &[&target_deck, &note_id]).await?;
    tx.execute("DELETE FROM note_move_suggestions WHERE note = $1 AND target_deck = $2", &[&note_id, &target_deck]).await?;

    if update_timestamp {
        update_note_timestamp(&tx, note_id).await?;
    }

    tx.commit().await?;
    Ok(note_id.to_string())
}


pub async fn approve_tag_change(db_state: &Arc<database::AppState>,tag_id: i64, update_timestamp: bool) -> Return<String> {
    let mut client = database::client(db_state).await?;
    let tx = client.transaction().await?;

    let rows = tx
        .query("SELECT note, content FROM tags WHERE id = $1", &[&tag_id])
        .await?;
    
    if rows.is_empty() {
        return Err(NoteNotFound(NoteNotFoundContext::TagApprove));
    }
    let note_id: i64 = rows[0].get(0);
    let content: String = rows[0].get(1);
    
    // Only approve new tags if they don't already exist to prevent duplicates
    let existing_tag_check = tx.query(
        "SELECT 1 FROM tags WHERE content = $1 AND note = $2 AND reviewed = true",
        &[&content, &note_id],
    ).await?;
    
    if !existing_tag_check.is_empty() { // Tag already exists, delete the new one
        tx.execute(
            "DELETE FROM tags WHERE id = $1 AND action = true",
            &[&tag_id],
        ).await?;
    } else { // Tag doesn't exist, approve it
        tx.execute(
            "UPDATE tags SET reviewed = true WHERE id = $1 AND action = true",
            &[&tag_id],
        ).await?;
    }
    
    let delete_query = "
    WITH hit AS (
        SELECT content, note 
        FROM tags WHERE id = $1 AND action = false
    )
    DELETE FROM tags WHERE note in (select note from hit) and content in (select content from hit)";
    
    tx.execute(delete_query, &[&tag_id]).await?;

    if update_timestamp {
        update_note_timestamp(&tx, note_id).await?;
    }

    tx.commit().await?;
    Ok(note_id.to_string())
}

pub async fn update_field_suggestion(db_state: &Arc<database::AppState>, field_id: i64, new_content_r: &str) -> Return<()> {
    let mut client = database::client(db_state).await?;
    let tx = client.transaction().await?;
    
    let rows = tx
        .query("SELECT content FROM fields WHERE id = $1", &[&field_id])
        .await?;

    if rows.is_empty() {
        return Err(NoteNotFound(NoteNotFoundContext::FieldUpdate));
    }

    let old_content_r: String = rows[0].get(0);
    let old_content = cleanser::clean(&old_content_r);
    let new_content = cleanser::clean(new_content_r);
    if !new_content.is_empty() && new_content != old_content {
        tx.execute("UPDATE fields SET content = $1 WHERE id = $2 ", &[&new_content, &field_id]).await?;
    }

    tx.commit().await?;

    Ok(())
}

pub async fn approve_field_change(db_state: &Arc<database::AppState>,field_id: i64, update_timestamp: bool) -> Return<String> {
    let mut client = database::client(db_state).await?;
    let tx = client.transaction().await?;

    let rows = tx
        .query("SELECT note FROM fields WHERE id = $1", &[&field_id])
        .await?;

    if rows.is_empty() {
        return Err(NoteNotFound(NoteNotFoundContext::FieldApprove));
    }

    let note_id: i64 = rows[0].get(0);

    let del_cur_field_q = "
        DELETE FROM fields
        WHERE reviewed = true
        AND position = (SELECT position FROM fields WHERE id = $1)
        AND id <> $1
        AND note = $2
    ";
    let appr_new_field_q = "
        UPDATE fields
        SET reviewed = true
        WHERE id = $1
    ";

    let content = tx
        .query("Select content from fields where id = $1", &[&field_id])
        .await?
        [0].get::<_, String>(0);

    tx.execute(del_cur_field_q, &[&field_id, &note_id]).await?;

    if !content.is_empty() {
        tx.execute(appr_new_field_q, &[&field_id]).await?;
    } else {
        tx.execute("DELETE FROM fields WHERE id = $1",&[&field_id]).await?;
    }

    if update_timestamp {
        update_note_timestamp(&tx, note_id).await?;
    }

    tx.commit().await?;
    
    if update_timestamp {
        // we use update_timestamp as a proxy for whether the note was bulk updated. Only if they updated it manually on the website, we spawn. otherwise it egts handled by the ulk
        let state_clone = db_state.clone();
        tokio::spawn(async move {
            if let Err(e) = media_reference_manager::update_media_references_note_state(&state_clone, note_id).await {
                println!("Error updating media references (3): {:?}", e);
            }
        });
    }

    Ok(note_id.to_string())
}

pub async fn merge_by_commit(db_state: &Arc<database::AppState>,commit_id: i32, approve: bool, user: User) -> Return<Option<i32>> {
    let mut client = database::client(db_state).await?;

    let q_guid = client
        .query(
            "Select deck from commits where commit_id = $1",
            &[&commit_id],
        )
        .await?;
    if q_guid.is_empty() {
        return Err(CommitDeckNotFound);
    }
    let deck_id: i64 = q_guid[0].get(0);

    let access = is_authorized(db_state, &user, deck_id).await?;
    if !access {
        return Err(Unauthorized);
    }

    let affected_tags = client
        .query(
            "
        SELECT id FROM tags WHERE commit = $1 and reviewed = false
    ",
            &[&commit_id],
        )
        .await?
        .into_iter()
        .map(|row| row.get::<_, i64>("id"))
        .collect::<Vec<i64>>();

    let affected_fields = client
        .query(
            "
        SELECT id FROM fields WHERE commit = $1 and reviewed = false
    ",
            &[&commit_id],
        )
        .await?
        .into_iter()
        .map(|row| row.get::<_, i64>("id"))
        .collect::<Vec<i64>>();

    let affected_notes = client
        .query(
            "
        SELECT notes.id, notes.reviewed FROM notes
        JOIN (
            SELECT note FROM fields WHERE commit = $1 and reviewed = false
            UNION
            SELECT note FROM tags WHERE commit = $1 and reviewed = false
            UNION
            SELECT note from card_deletion_suggestions WHERE commit = $1
            UNION
            SELECT note FROM note_move_suggestions WHERE commit = $1
        ) AS n ON notes.id = n.note
        GROUP BY notes.id
    ",
            &[&commit_id],
        )
        .await?;

    let affected_note_ids = affected_notes.iter().map(|row| row.get(0)).collect::<Vec<i64>>();

    let deleted_notes = client
        .query(
            "
        SELECT note FROM card_deletion_suggestions WHERE commit = $1
    ",
            &[&commit_id],
        )
        .await?
        .into_iter()
        .map(|row| row.get::<_, i64>("note"))
        .collect::<Vec<i64>>();

    let moved_deck_suggestion = client
        .query(
        "
            SELECT note, target_deck FROM note_move_suggestions WHERE commit = $1
        ",
        &[&commit_id],
        )
        .await?
        .into_iter()
        .map(|row| (row.get::<_, i64>("note"), row.get::<_, i64>("target_deck")))
        .collect::<Vec<(i64, i64)>>();


    // The query is very similar to the one /reviews uses
    let next_review_query = r#"
    WITH RECURSIVE accessible AS (
        SELECT id FROM decks WHERE id IN (
            SELECT deck FROM maintainers WHERE user_id = $1
            UNION
            SELECT id FROM decks WHERE owner = $1
        )
        UNION
        SELECT decks.id
        FROM decks
        INNER JOIN accessible ON decks.parent = accessible.id
    ),
    unreviewed_changes AS (
        SELECT commit_id, rationale, timestamp, deck
        FROM commits
        WHERE EXISTS (
            SELECT 1 FROM fields
            WHERE fields.reviewed = false AND fields.commit = commits.commit_id
        )
        UNION
        SELECT commit_id, rationale, timestamp, deck
        FROM commits
        WHERE EXISTS (
            SELECT 1 FROM tags
            WHERE tags.reviewed = false AND tags.commit = commits.commit_id
        )
        UNION
        SELECT commit_id, rationale, timestamp, deck
        FROM commits
        WHERE EXISTS (
            SELECT 1 FROM card_deletion_suggestions
            WHERE card_deletion_suggestions.commit = commits.commit_id
        )
        UNION
        SELECT commit_id, rationale, timestamp, deck
        FROM commits
        WHERE EXISTS (
            SELECT 1 FROM note_move_suggestions
            WHERE note_move_suggestions.commit = commits.commit_id
        )
    ),
    indexed_unreviewed AS (
        SELECT commit_id, ROW_NUMBER() OVER (ORDER BY timestamp) AS row_num
        FROM unreviewed_changes
        WHERE deck IN (SELECT id FROM accessible) OR (SELECT is_admin FROM users WHERE id = $1)
    )
    SELECT commit_id
    FROM indexed_unreviewed
    WHERE row_num = (
        SELECT CASE
            WHEN EXISTS (
                SELECT 1
                FROM indexed_unreviewed
                WHERE row_num > (
                    SELECT row_num
                    FROM indexed_unreviewed
                    WHERE commit_id = $2
                )
            ) THEN (
                SELECT MIN(row_num)
                FROM indexed_unreviewed
                WHERE row_num > (
                    SELECT row_num
                    FROM indexed_unreviewed
                    WHERE commit_id = $2
                )
            )
            ELSE (
                SELECT MAX(row_num)
                FROM indexed_unreviewed
                WHERE row_num < (
                    SELECT row_num
                    FROM indexed_unreviewed
                    WHERE commit_id = $2
                )
            )
        END
    )
    ORDER BY commit_id
    LIMIT 1
    "#;
    let next_review = client
        .query(next_review_query, &[&user.id(), &commit_id])
        .await?;

    // Slightly less performant to do it in single queries than doing a bigger query here, but for readability and easier code maintenance, we keep it that way.
    // The performance difference is not relevant in this case
    if approve {
        for tag in affected_tags {
            approve_tag_change(db_state, tag, false).await?;
        }

        for field in affected_fields {
            approve_field_change(db_state, field, false).await?;
        }

        for note in deleted_notes {
            note_manager::mark_note_deleted(db_state, note, user.clone(), true).await?;
        }

        for note in moved_deck_suggestion {
            let note_id = note.0;
            let target_deck = note.1;
            approve_move_note_request(db_state, note_id, target_deck, false).await?;
        }

        let tx = client.transaction().await?;

        for row in affected_notes {
            let note_id: i64 = row.get(0);
            let reviewed: bool = row.get(1);
            if !reviewed {
                approve_card(db_state, note_id, user.clone(), true).await?;
            }
            update_note_timestamp(&tx, note_id).await?;
        }

        tx.commit().await?;
    } else {
        for tag in affected_tags {
            deny_tag_change(db_state, tag).await?;
        }

        for field in affected_fields {
            deny_field_change(db_state, field, false).await?;
        }
        
        let tx = client.transaction().await?;

        for row in affected_notes {
            let note_id: i64 = row.get(0);
            let reviewed: bool = row.get(1);
            if !reviewed {
                tx.execute("DELETE FROM notes cascade WHERE id = $1", &[&note_id])
                    .await?;
                // Should handle media reference automatically
            }
        }

        for note_id in deleted_notes {
            tx.execute(
                "DELETE FROM card_deletion_suggestions WHERE note = $1",
                &[&note_id],
            )
            .await?;
        }

        for note in moved_deck_suggestion {
            let note_id = note.0;
            let target_deck = note.1;
            tx.execute("DELETE FROM note_move_suggestions WHERE note = $1 AND target_deck = $2", &[&note_id, &target_deck])
            .await?;
        }


        tx.commit().await?;
    }

    let state_clone = db_state.clone();
    tokio::spawn(async move {
        if let Err(e) = media_reference_manager::update_media_references_for_commit(&state_clone, &affected_note_ids).await {
            println!("Error updating media references (4) for commit: {:?}", e);
        }
    });

    // Get next outstanding commit id and return it (if any)
    if next_review.is_empty() {
        return Ok(None);
    }
    Ok(Some(next_review[0].get(0)))
}
