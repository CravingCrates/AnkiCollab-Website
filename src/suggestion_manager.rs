use crate::error::Error::*;
use crate::error::NoteNotFoundContext;
use crate::{database, note_manager, Return};
use rocket_auth::User;

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

pub async fn is_authorized(user: &User, deck: i64) -> Return<bool> {
    let client = database::client().await?;
    let rows = client
        .query(
            "SELECT 1 FROM decks WHERE (owner = $1 AND id = $3) OR $2 LIMIT 1",
            &[&user.id(), &user.is_admin, &deck],
        )
        .await?;
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
        let rows = client
            .query(
                "SELECT 1 FROM maintainers WHERE user_id = $1 AND deck = $2 LIMIT 1",
                &[&user.id(), &parent_deck],
            )
            .await?;
        return Ok(!rows.is_empty());
    }

    Ok(access)
}

// Only used for unreviewed cards to prevent them from being added to the deck. Existing cards should use mark_note_deleted instead
pub async fn delete_card(note_id: i64, user: User) -> Return<String> {
    let client = database::client().await?;

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

    let access = is_authorized(&user, deck_id).await?;
    if !access {
        return Err(Unauthorized);
    }

    client
        .query("DELETE FROM notes CASCADE WHERE id = $1", &[&note_id])
        .await?;

    Ok(guid)
}

// If bulk is true, we skip a few steps that have already been handled by the caller
pub async fn approve_card(note_id: i64, user: User, bulk: bool) -> Return<String> {
    let mut client = database::client().await?;
    let tx = client.transaction().await?;

    let q_guid = tx
        .query("select deck from notes where id = $1", &[&note_id])
        .await?;
    if q_guid.is_empty() {
        return Err(NoteNotFound(NoteNotFoundContext::ApproveCard));
    }
    let deck_id: i64 = q_guid[0].get(0);

    if !bulk {
        let access = is_authorized(&user, deck_id).await?;
        if !access {
            return Err(Unauthorized);
        }
    }

    // Check if the fields are valid
    let unique_fields_row = tx
        .query(
            "
        SELECT (
            (
              SELECT COUNT(*)
              FROM notetype_field
              WHERE notetype = (SELECT notetype FROM notes WHERE id = $1)
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
        ",
            &[&note_id],
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
    }

    tx.commit().await?;

    Ok(note_id.to_string())
}

pub async fn deny_tag_change(tag_id: i64) -> Return<String> {
    let client = database::client().await?;

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

pub async fn deny_field_change(field_id: i64) -> Return<String> {
    let client = database::client().await?;

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
    Ok(note_id.to_string())
}

pub async fn approve_tag_change(tag_id: i64, update_timestamp: bool) -> Return<String> {
    let mut client = database::client().await?;
    let tx = client.transaction().await?;

    let rows = tx
        .query("SELECT note FROM tags WHERE id = $1", &[&tag_id])
        .await?;

    if rows.is_empty() {
        return Err(NoteNotFound(NoteNotFoundContext::TagApprove));
    }
    let note_id: i64 = rows[0].get(0);

    let update_query = "UPDATE tags SET reviewed = true WHERE id = $1 AND action = true";
    let delete_query = "
    WITH hit AS (
        SELECT content, note 
        FROM tags WHERE id = $1 AND action = false
    )
    DELETE FROM tags WHERE note in (select note from hit) and content in (select content from hit)";

    tx.query(update_query, &[&tag_id]).await?;
    tx.query(delete_query, &[&tag_id]).await?;

    if update_timestamp {
        update_note_timestamp(&tx, note_id).await?;
    }

    tx.commit().await?;
    Ok(note_id.to_string())
}

pub async fn approve_field_change(field_id: i64, update_timestamp: bool) -> Return<String> {
    let mut client = database::client().await?;
    let tx = client.transaction().await?;

    let rows = tx
        .query("SELECT note FROM fields WHERE id = $1", &[&field_id])
        .await?;

    if rows.is_empty() {
        return Err(NoteNotFound(NoteNotFoundContext::FieldApprove));
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

    tx.query(query1, &[&field_id, &note_id]).await?;
    tx.query(query2, &[&field_id]).await?;

    if update_timestamp {
        update_note_timestamp(&tx, note_id).await?;
    }

    tx.commit().await?;

    Ok(note_id.to_string())
}

pub async fn merge_by_commit(commit_id: i32, approve: bool, user: User) -> Return<Option<i32>> {
    let mut client = database::client().await?;

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

    let access = is_authorized(&user, deck_id).await?;
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
        ) AS n ON notes.id = n.note
        GROUP BY notes.id
    ",
            &[&commit_id],
        )
        .await?;

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
            approve_tag_change(tag, false).await?;
        }

        for field in affected_fields {
            approve_field_change(field, false).await?;
        }

        for note in deleted_notes {
            note_manager::mark_note_deleted(note, user.clone(), true).await?;
        }

        let tx = client.transaction().await?;

        for row in affected_notes {
            let note_id: i64 = row.get(0);
            let reviewed: bool = row.get(1);
            if !reviewed {
                approve_card(note_id, user.clone(), true).await?;
            }
            update_note_timestamp(&tx, note_id).await?;
        }

        tx.commit().await?;
    } else {
        for tag in affected_tags {
            deny_tag_change(tag).await?;
        }

        for field in affected_fields {
            deny_field_change(field).await?;
        }

        let tx = client.transaction().await?;

        for row in affected_notes {
            let note_id: i64 = row.get(0);
            let reviewed: bool = row.get(1);
            if !reviewed {
                tx.execute("DELETE FROM notes cascade WHERE id = $1", &[&note_id])
                    .await?;
            }
        }

        for note_id in deleted_notes {
            tx.execute(
                "DELETE FROM card_deletion_suggestions WHERE note = $1",
                &[&note_id],
            )
            .await?;
        }

        tx.commit().await?;
    }

    // Get next outstanding commit id and return it (if any)
    if next_review.is_empty() {
        return Ok(None);
    }
    Ok(Some(next_review[0].get(0)))
}
