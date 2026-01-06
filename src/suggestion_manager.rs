use std::sync::Arc;

use crate::cleanser;
use crate::error::Error::{
    AmbiguousFields, CommitDeckNotFound, InvalidNote, NoteNotFound, Unauthorized,
};
use crate::error::NoteNotFoundContext;
use crate::media_reference_manager;
use crate::note_history::{self, EventType};
use crate::user::User;
use crate::{database, note_manager, Return};
use sentry::Level;

pub async fn update_note_timestamp(
    tx: &tokio_postgres::Transaction<'_>,
    note_id: i64,
) -> Return<()> {
    // Delegate to bulk implementation for single note
    update_notes_timestamps(tx, &[note_id]).await
}

// Bulk version to reduce round trips when updating many notes (and their ancestor decks)
pub async fn update_notes_timestamps(
    tx: &tokio_postgres::Transaction<'_>,
    note_ids: &[i64],
) -> Return<()> {
    if note_ids.is_empty() {
        return Ok(());
    }

    // Update all ancestor decks (recursive) for all affected notes in one shot
    let query1 = "
        WITH RECURSIVE target_notes AS (
            SELECT UNNEST($1::bigint[]) AS nid
        ), note_decks AS (
            SELECT DISTINCT deck AS id
            FROM notes n
            JOIN target_notes t ON n.id = t.nid
        ), tree AS (
            SELECT id, parent FROM decks WHERE id IN (SELECT id FROM note_decks)
            UNION ALL
            SELECT d.id, d.parent FROM decks d
            JOIN tree t ON d.id = t.parent
        )
        UPDATE decks
        SET last_update = NOW()
        WHERE id IN (SELECT id FROM tree)";

    let query2 = "UPDATE notes SET last_update = NOW() WHERE id = ANY($1)";

    tx.query(query1, &[&note_ids]).await?;
    tx.query(query2, &[&note_ids]).await?;
    Ok(())
}

async fn derive_commit_id(
    tx: &tokio_postgres::Transaction<'_>,
    note_id: i64,
) -> Return<Option<i32>> {
    const QUERIES: [&str; 4] = [
        "SELECT commit FROM fields WHERE note = $1 AND commit IS NOT NULL LIMIT 1",
        "SELECT commit FROM tags WHERE note = $1 AND commit IS NOT NULL LIMIT 1",
        "SELECT commit FROM note_move_suggestions WHERE note = $1 AND commit IS NOT NULL LIMIT 1",
        "SELECT commit FROM card_deletion_suggestions WHERE note = $1 AND commit IS NOT NULL LIMIT 1",
    ];

    for query in QUERIES {
        if let Some(row) = tx.query_opt(query, &[&note_id]).await? {
            let commit_id: i32 = row.get(0);
            return Ok(Some(commit_id));
        }
    }

    Ok(None)
}

pub async fn is_authorized(
    db_state: &Arc<database::AppState>,
    user: &User,
    deck: i64,
) -> Return<bool> {
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
        let query = r"
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
        ";
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
pub async fn delete_card(
    db_state: &Arc<database::AppState>,
    note_id: i64,
    user: User,
) -> Return<String> {
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
        .query("DELETE FROM notes WHERE id = $1", &[&note_id])
        .await?;

    // Clean up media references should be unnecessary since the card is deleted with cascade and we have a oreign key

    Ok(guid)
}

// If bulk is true, we skip a few steps that have already been handled by the caller
pub async fn approve_card(
    tx: &tokio_postgres::Transaction<'_>,
    db_state: &Arc<database::AppState>,
    note_id: i64,
    user: &User,
    bulk: bool,
) -> Return<String> {
    let q_guid = tx
        .query(
            "select deck, reviewed from notes where id = $1",
            &[&note_id],
        )
        .await?;
    if q_guid.is_empty() {
        return Err(NoteNotFound(NoteNotFoundContext::ApproveCard));
    }
    let deck_id: i64 = q_guid[0].get(0);
    let was_reviewed: bool = q_guid[0].get(1);

    if !bulk {
        let access = is_authorized(db_state, user, deck_id).await?;
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
            )",
            &[&note_id],
        )
        .await?;
    if unique_fields_row.is_empty() {
        return Err(InvalidNote);
    }

    if !unique_fields_row[0].get::<_, bool>(0) {
        return Err(AmbiguousFields(note_id));
    }

    // Validate that the note has at least one field at position 0 with non-empty content
    // This check runs before marking as reviewed to prevent orphaned notes
    let has_valid_field_zero = tx
        .query_one(
            "SELECT EXISTS(
                SELECT 1 FROM fields 
                WHERE note = $1 AND position = 0 AND content IS NOT NULL AND content <> ''
            )",
            &[&note_id],
        )
        .await?
        .get::<_, bool>(0);
    if !has_valid_field_zero {
        return Err(InvalidNote);
    }

    // Always mark all fields and tags as reviewed when approving a card
    // This ensures fields not in the current commit are also marked reviewed
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

    if !was_reviewed {
        // Only emit baseline NoteCreated if the note has no prior events (fresh approval)
        let prior = tx
            .query(
                "SELECT 1 FROM note_events WHERE note_id = $1 LIMIT 1",
                &[&note_id],
            )
            .await?;
        if prior.is_empty() {
            let commit_id = derive_commit_id(tx, note_id).await?;
            // Build snapshot of reviewed fields/tags after approval
            let field_rows = tx.query(
                "SELECT position, content FROM fields WHERE note = $1 AND reviewed = true ORDER BY position",
                &[&note_id]
            ).await?;
            let mut fields_json = Vec::with_capacity(field_rows.len());
            for fr in field_rows {
                let pos: u32 = fr.get(0);
                let content: String = fr.get(1);
                fields_json.push(
                    serde_json::json!({"position": pos, "content": cleanser::clean(&content)}),
                );
            }
            let tag_rows = tx.query(
                "SELECT content FROM tags WHERE note = $1 AND reviewed = true AND action = true",
                &[&note_id]
            ).await?;
            let mut tags_json = Vec::with_capacity(tag_rows.len());
            for tr in tag_rows {
                if let Some(c) = tr.get::<_, Option<String>>(0) {
                    tags_json.push(cleanser::clean(&c));
                }
            }
            let snapshot = serde_json::json!({
                "reviewed": true,
                "fields": fields_json,
                "tags": tags_json
            });
            let _ = note_history::log_event(
                tx,
                note_id,
                EventType::NoteCreated,
                None,
                Some(&snapshot),
                Some(user.id()),
                commit_id,
                Some(true),
            )
            .await?;
        }
    }

    if !bulk {
        // Collect timestamps to update: the note plus any subscribers
        let mut to_bump: Vec<i64> = vec![note_id];
        // Bump linked subscriber notes' timestamps if this note is a base for others (bubble to decks)
        let subs = tx
            .query(
                "SELECT subscriber_note_id FROM note_inheritance WHERE base_note_id = $1",
                &[&note_id],
            )
            .await?;
        for r in &subs {
            let sid: i64 = r.get(0);
            to_bump.push(sid);
        }
        update_notes_timestamps(tx, &to_bump).await?;
    } else {
        // bulk path still needs to handle subscriber timestamp bump outside (caller responsibility)
    }

    Ok(note_id.to_string())
}

pub async fn deny_note_move_request(
    tx: &tokio_postgres::Transaction<'_>,
    move_id: i32,
    actor_user_id: i32,
) -> Return<String> {
    let rows = tx
        .query(
            "SELECT note, target_deck, commit FROM note_move_suggestions WHERE id = $1",
            &[&move_id],
        )
        .await?;

    if rows.is_empty() {
        return Err(NoteNotFound(NoteNotFoundContext::NoteMovalRequest));
    }

    let note_id: i64 = rows[0].get(0);
    let target: i64 = rows[0].get(1);
    let commit_id: Option<i32> = rows[0].get(2);
    tx.query(
        "DELETE FROM note_move_suggestions WHERE id = $1",
        &[&move_id],
    )
    .await?;
    let _ = note_history::log_event(
        tx,
        note_id,
        EventType::SuggestionDenied,
        Some(&serde_json::json!({"type":"move","target_deck": target})),
        None,
        Some(actor_user_id),
        commit_id,
        Some(false),
    )
    .await?;
    Ok(note_id.to_string())
}

pub async fn deny_tag_change(
    tx: &tokio_postgres::Transaction<'_>,
    tag_id: i64,
    actor_user_id: i32,
) -> Return<String> {
    let rows = tx
        .query(
            "SELECT note, content, action, commit FROM tags WHERE id = $1",
            &[&tag_id],
        )
        .await?;

    if rows.is_empty() {
        return Err(NoteNotFound(NoteNotFoundContext::TagDenied));
    }
    let note_id: i64 = rows[0].get(0);
    let content: Option<String> = rows[0].get(1);
    let action: bool = rows[0].get(2); // addition if true
    let commit_id: Option<i32> = rows[0].get(3);

    tx.query("DELETE FROM tags WHERE id = $1", &[&tag_id])
        .await?;

    let old_json = content.map(|c| {
        serde_json::json!({
            "content": c,
            "action": action,
            "suggestion": true
        })
    });
    let _ = note_history::log_event(
        tx,
        note_id,
        EventType::TagChangeDenied,
        old_json.as_ref(),
        None,
        Some(actor_user_id),
        commit_id,
        Some(false),
    )
    .await?;

    Ok(note_id.to_string())
}

pub async fn deny_field_change(
    tx: &tokio_postgres::Transaction<'_>,
    field_id: i64,
    actor_user_id: i32,
) -> Return<String> {
    // Load target field details
    let row_opt = tx
        .query_opt(
            "SELECT note, position, reviewed, content, commit FROM fields WHERE id = $1",
            &[&field_id],
        )
        .await?;

    if row_opt.is_none() {
        return Err(NoteNotFound(NoteNotFoundContext::FieldDenied));
    }

    let row = row_opt.unwrap();
    let note_id: i64 = row.get(0);
    let position: u32 = row.get(1);
    let reviewed: bool = row.get(2);
    let denied_content: String = row.get(3);
    let commit_id: Option<i32> = row.get(4);

    // Fetch the current reviewed field content at the same position (if it exists)
    let current_content_opt = tx
        .query_opt(
            "SELECT content FROM fields WHERE note = $1 AND position = $2 AND reviewed = true AND id <> $3",
            &[&note_id, &position, &field_id],
        )
        .await?;
    
    let current_content = current_content_opt.map(|r| r.get::<_, String>(0));

    // Determine whether the parent note is already reviewed
    let note_reviewed: bool = tx
        .query_one("SELECT reviewed FROM notes WHERE id = $1", &[&note_id])
        .await?
        .get(0);

    // Never allow deletion of the reviewed base field (position 0)
    if reviewed && position == 0 {
        // Keep invariants: field 0 must remain non-empty and present
        // Rollback implicit by dropping tx on error
        return Err(InvalidNote);
    }

    // VALIDATE BEFORE DELETION: Check if deletion would violate invariants
    if note_reviewed {
        // For reviewed notes: ensure field 0 will remain present and non-empty after deletion
        // If deleting a field at position 0 (unreviewed suggestion), check if there's another reviewed field 0
        if position == 0 {
            let exists_other_pos0 = tx
                .query_one(
                    "SELECT EXISTS(SELECT 1 FROM fields WHERE note = $1 AND position = 0 AND reviewed = true AND content <> '' AND id <> $2)",
                    &[&note_id, &field_id],
                )
                .await?
                .get::<_, bool>(0);
            if !exists_other_pos0 {
                return Err(InvalidNote);
            }
        }
    } else {
        // For unreviewed notes: ensure at least one field will remain after deletion
        let field_count: i64 = tx
            .query_one("SELECT COUNT(*) FROM fields WHERE note = $1", &[&note_id])
            .await?
            .get(0);
        if field_count <= 1 {
            return Err(InvalidNote);
        }
    }

    // Perform the deletion (now safe - invariants validated above)
    tx.execute("DELETE FROM fields WHERE id = $1", &[&field_id])
        .await?;

    // Log the denial with both current and denied content
    let old_value = serde_json::json!({
        "position": position,
        "current_content": current_content.as_ref().map(|c| cleanser::clean(c)).unwrap_or_default(),
        "denied_content": cleanser::clean(&denied_content),
        "had_current": current_content.is_some()
    });

    let _ = note_history::log_event(
        tx,
        note_id,
        EventType::FieldChangeDenied,
        Some(&old_value),
        None,
        Some(actor_user_id),
        commit_id,
        Some(false),
    )
    .await?;
    Ok(note_id.to_string())
}

pub async fn approve_move_note_request_by_moveid(
    tx: &tokio_postgres::Transaction<'_>,
    move_id: i32,
    actor_user_id: i32,
) -> Return<String> {
    let rows = tx
        .query(
            "SELECT note, target_deck, commit FROM note_move_suggestions WHERE id = $1",
            &[&move_id],
        )
        .await?;

    if rows.is_empty() {
        return Err(NoteNotFound(NoteNotFoundContext::TagApprove));
    }
    let note_id: i64 = rows[0].get(0);
    let target_id: i64 = rows[0].get(1);
    let commit_id: Option<i32> = rows[0].get(2);

    approve_move_note_request(tx, note_id, target_id, true, commit_id, actor_user_id).await?;

    Ok(note_id.to_string())
}

pub async fn approve_move_note_request(
    tx: &tokio_postgres::Transaction<'_>,
    note_id: i64,
    target_deck: i64,
    update_timestamp: bool,
    commit_id: Option<i32>,
    actor_user_id: i32,
) -> Return<String> {
    // Capture old deck and get deck paths for both old and new
    let old_deck_row = tx
        .query_one("SELECT deck FROM notes WHERE id = $1", &[&note_id])
        .await?;
    let old_deck: i64 = old_deck_row.get(0);
    
    // Get deck names (consider paths instead?) for human-readable event logging
    let old_deck_path_row = tx
        .query_one("SELECT name FROM decks WHERE id = $1", &[&old_deck])
        .await?;
    let old_deck_path: String = old_deck_path_row.get(0);
    
    let new_deck_path_row = tx
        .query_one("SELECT name FROM decks WHERE id = $1", &[&target_deck])
        .await?;
    let new_deck_path: String = new_deck_path_row.get(0);
    
    tx.execute(
        "UPDATE notes SET deck = $1 WHERE id = $2",
        &[&target_deck, &note_id],
    )
    .await?;
    tx.execute(
        "DELETE FROM note_move_suggestions WHERE note = $1 AND target_deck = $2",
        &[&note_id, &target_deck],
    )
    .await?;

    if update_timestamp {
        update_notes_timestamps(tx, &[note_id]).await?;
    }

    let _ = note_history::log_event(
        tx,
        note_id,
        EventType::NoteMoved,
        Some(&serde_json::json!({"from": old_deck_path})),
        Some(&serde_json::json!({"to": new_deck_path})),
        Some(actor_user_id),
        commit_id,
        Some(true),
    )
    .await?;

    Ok(note_id.to_string())
}

pub async fn approve_tag_change(
    tx: &tokio_postgres::Transaction<'_>,
    tag_id: i64,
    update_timestamp: bool,
    actor_user_id: i32,
) -> Return<String> {
    approve_tag_change_with_commit(tx, tag_id, update_timestamp, None, actor_user_id).await
}

pub async fn approve_tag_change_with_commit(
    tx: &tokio_postgres::Transaction<'_>,
    tag_id: i64,
    update_timestamp: bool,
    commit_id: Option<i32>,
    actor_user_id: i32,
) -> Return<String> {
    // Fetch suggestion row
    let row_opt = tx
        .query_opt(
            "SELECT note, content, action, commit FROM tags WHERE id = $1",
            &[&tag_id],
        )
        .await?;
    if row_opt.is_none() {
        // Capture unexpected absence to help debug TagApprove reports
        // Note: Only use sentry::with_scope here, not error!() macro,
        // because the sentry_layer auto-captures ERROR events causing double-capture
        sentry::with_scope(
            |scope| {
                scope.set_tag("component", "suggestion_manager");
                scope.set_tag("operation", "approve_tag_change_with_commit");
                scope.set_extra("tag_id", tag_id.into());
                scope.set_extra("commit_id_param", commit_id.map(|c| c.into()).unwrap_or_else(|| "none".into()));
                scope.set_extra("actor_user_id", actor_user_id.into());
            },
            || {
                sentry::capture_message(
                    "TagApprove failure: missing tag suggestion row",
                    Level::Error,
                );
            },
        );
        return Err(NoteNotFound(NoteNotFoundContext::TagApprove));
    }
    let row = row_opt.unwrap();
    let note_id: i64 = row.get(0);
    let content: String = row.get(1);
    let action: bool = row.get(2); // true = addition, false = removal
    let suggestion_commit: Option<i32> = row.get(3);
    let effective_commit_id = commit_id.or(suggestion_commit);

    // Determine inheritance role of this note
    let subscriber_inh = tx.query(
        "SELECT base_note_id, removed_base_tags FROM note_inheritance WHERE subscriber_note_id = $1",
        &[&note_id]
    ).await?;
    let is_subscriber = !subscriber_inh.is_empty();
    let (base_note_id, removed_base_tags): (Option<i64>, Vec<String>) = if is_subscriber {
        let b: i64 = subscriber_inh[0].get(0);
        let arr: Vec<String> = subscriber_inh[0].get(1);
        (Some(b), arr)
    } else {
        (None, vec![])
    };

    // Helper checks (only executed when relevant)
    let mut base_has_tag = false;
    if let Some(bid) = base_note_id {
        // subscriber scenario
        let res = tx.query(
            "SELECT 1 FROM tags WHERE note = $1 AND content = $2 AND reviewed = true AND action = true LIMIT 1",
            &[&bid, &content]
        ).await?;
        base_has_tag = !res.is_empty();
    }

    let existing_local_reviewed = tx.query(
        "SELECT id FROM tags WHERE note = $1 AND content = $2 AND reviewed = true AND action = true",
        &[&note_id, &content]
    ).await?;
    let has_local_reviewed = !existing_local_reviewed.is_empty();

    if is_subscriber {
        // Subscriber-specific logic
        if !action {
            // removal request
            if has_local_reviewed {
                // Remove the local tag(s)
                tx.execute("DELETE FROM tags WHERE note = $1 AND content = $2 AND reviewed = true AND action = true", &[&note_id, &content]).await?;
            }
            if base_has_tag {
                // hide base tag
                tx.execute(
                    "UPDATE note_inheritance SET removed_base_tags = CASE WHEN $2 = ANY(removed_base_tags) THEN removed_base_tags ELSE array_append(removed_base_tags, $2) END WHERE subscriber_note_id = $1",
                    &[&note_id, &content]
                ).await?;
            } else if !has_local_reviewed { // stale removal suggestion (neither base nor local)
                 // nothing to track
            }
            // Delete the removal suggestion row itself
            tx.execute("DELETE FROM tags WHERE id = $1", &[&tag_id])
                .await?;
        } else {
            // addition request
            if base_has_tag {
                // re-enable hidden base tag or duplicate of base
                // Ensure it's not hidden anymore
                tx.execute(
                    "UPDATE note_inheritance SET removed_base_tags = array_remove(removed_base_tags, $2) WHERE subscriber_note_id = $1 AND $2 = ANY(removed_base_tags)",
                    &[&note_id, &content]
                ).await?;
                // Drop the suggestion (no local duplicate wanted)
                tx.execute("DELETE FROM tags WHERE id = $1", &[&tag_id])
                    .await?;
            } else {
                // Base does not have tag
                if has_local_reviewed {
                    // duplicate addition -> discard suggestion
                    tx.execute("DELETE FROM tags WHERE id = $1", &[&tag_id])
                        .await?;
                } else {
                    // Promote suggestion to local tag
                    tx.execute("UPDATE tags SET reviewed = true WHERE id = $1", &[&tag_id])
                        .await?;
                }
                // Remove stale hidden marker if somehow present (base no longer has tag but array retained)
                if removed_base_tags.iter().any(|t| t == &content) {
                    tx.execute(
                        "UPDATE note_inheritance SET removed_base_tags = array_remove(removed_base_tags, $2) WHERE subscriber_note_id = $1",
                        &[&note_id, &content]
                    ).await?;
                }
            }
        }
    } else {
        // Base (or standalone) note logic
        if action {
            // approve addition on base or standalone
            if has_local_reviewed {
                // duplicate local already exists -> just delete suggestion
                tx.execute("DELETE FROM tags WHERE id = $1", &[&tag_id])
                    .await?;
            } else {
                tx.execute("UPDATE tags SET reviewed = true WHERE id = $1", &[&tag_id])
                    .await?;
            }
        } else {
            // removal on base or standalone
            // Delete both the suggestion row and any existing reviewed tag rows of same content
            tx.execute(
                "WITH hit AS (SELECT content, note FROM tags WHERE id = $1 AND action = false) DELETE FROM tags WHERE (note, content) IN (SELECT note, content FROM hit)",
                &[&tag_id]
            ).await?;
        }
    }

    // Update timestamp for the note itself if requested
    let mut bump: Vec<i64> = Vec::new();
    if update_timestamp {
        bump.push(note_id);
    }

    // Propagation for base note changes (both additions and removals) to subscribers
    if !is_subscriber {
        // Check if this note has subscribers
        let subs = tx.query(
            "SELECT subscriber_note_id, removed_base_tags FROM note_inheritance WHERE base_note_id = $1",
            &[&note_id]
        ).await?;
        if !subs.is_empty() {
            if action {
                // base tag addition
                for r in &subs {
                    let sid: i64 = r.get(0);
                    let rb: Vec<String> = r.get(1);
                    // If subscriber is not hiding this tag, remove duplicate local copies
                    if !rb.iter().any(|t| t == &content) {
                        tx.execute(
                            "DELETE FROM tags WHERE note = $1 AND content = $2 AND reviewed = true AND action = true",
                            &[&sid, &content]
                        ).await?;
                    }
                    bump.push(sid);
                }
            } else {
                // base tag removal
                // Clean removed_base_tags arrays and bump timestamps
                tx.execute(
                    "UPDATE note_inheritance SET removed_base_tags = array_remove(removed_base_tags, $2) WHERE base_note_id = $1 AND $2 = ANY(removed_base_tags)",
                    &[&note_id, &content]
                ).await?;
                for r in &subs {
                    let sid: i64 = r.get(0);
                    bump.push(sid);
                }
            }
        }
    }

    if !bump.is_empty() {
        update_notes_timestamps(tx, &bump).await?;
    }
    // Basic event(s): we log one event representing the resolved suggestion outcome.
    // For simplicity we just capture final intent rather than every branch nuance.
    let _ = note_history::log_event(
        tx,
        note_id,
        if action {
            EventType::TagAdded
        } else {
            EventType::TagRemoved
        },
        None,
        Some(&serde_json::json!({"content": content, "action": action, "reviewed": true })),
        Some(actor_user_id),
        effective_commit_id,
        Some(true),
    )
    .await?;
    Ok(note_id.to_string())
}

pub async fn update_field_suggestion(
    tx: &tokio_postgres::Transaction<'_>,
    field_id: i64,
    new_content_r: &str,
) -> Return<()> {
    let rows = tx
        .query("SELECT content FROM fields WHERE id = $1", &[&field_id])
        .await?;

    if rows.is_empty() {
        return Err(NoteNotFound(NoteNotFoundContext::FieldUpdate));
    }

    let old_content_r: String = rows[0].get(0);
    let old_content = cleanser::clean(&old_content_r);
    // Remove zero-width spaces from the frontend spaghetti fix
    let cleaned_new_content_r = new_content_r.replace('\u{200B}', "");
    let new_content = cleanser::clean(&cleaned_new_content_r);
    if !new_content.is_empty() && new_content != old_content {
        tx.execute(
            "UPDATE fields SET content = $1 WHERE id = $2 ",
            &[&new_content, &field_id],
        )
        .await?;
    }

    Ok(())
}

/// Fetches all fields for a note with their reviewed content and any pending suggestions for a specific commit.
/// Used by the "Edit All Fields" panel to show maintainers all fields at once.
pub async fn get_all_fields_for_edit(
    db_state: &Arc<database::AppState>,
    note_id: i64,
    commit_id: i32,
) -> Return<crate::structs::AllFieldsForEditResponse> {
    use crate::structs::{AllFieldsForEditResponse, EditableFieldInfo};
    
    let client = database::client(db_state).await?;
    
    // Get note info and notetype
    let note_row = client
        .query_opt(
            "SELECT notetype, reviewed FROM notes WHERE id = $1 AND deleted = false",
            &[&note_id],
        )
        .await?;
    
    let (notetype_id, note_reviewed): (i64, bool) = match note_row {
        Some(row) => (row.get(0), row.get(1)),
        None => return Err(NoteNotFound(NoteNotFoundContext::FieldUpdate)),
    };
    
    // Get all field definitions from the notetype
    let field_defs = client
        .query(
            "SELECT position, name FROM notetype_field WHERE notetype = $1 ORDER BY position",
            &[&notetype_id],
        )
        .await?;
    
    // Get reviewed fields for this note
    let reviewed_fields = client
        .query(
            "SELECT position, content FROM fields WHERE note = $1 AND reviewed = true",
            &[&note_id],
        )
        .await?;
    let mut reviewed_map: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    for row in reviewed_fields {
        let pos: u32 = row.get(0);
        let content: String = row.get(1);
        reviewed_map.insert(pos, cleanser::clean(&content));
    }
    
    // Get suggestions for THIS commit
    let suggestions_this_commit = client
        .query(
            "SELECT id, position, content FROM fields WHERE note = $1 AND reviewed = false AND commit = $2",
            &[&note_id, &commit_id],
        )
        .await?;
    let mut suggestion_map: std::collections::HashMap<u32, (i64, String)> = std::collections::HashMap::new();
    for row in suggestions_this_commit {
        let id: i64 = row.get(0);
        let pos: u32 = row.get(1);
        let content: String = row.get(2);
        suggestion_map.insert(pos, (id, cleanser::clean(&content)));
    }
    
    // Get suggestions from OTHER commits (to mark fields that have pending changes elsewhere)
    let suggestions_other_commits = client
        .query(
            "SELECT DISTINCT position FROM fields WHERE note = $1 AND reviewed = false AND commit != $2",
            &[&note_id, &commit_id],
        )
        .await?;
    let mut other_suggestions_positions: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for row in suggestions_other_commits {
        let pos: u32 = row.get(0);
        other_suggestions_positions.insert(pos);
    }
    
    // Check if this is an inherited note and get subscribed fields
    let inheritance_row = client
        .query_opt(
            "SELECT base_note_id, subscribed_fields FROM note_inheritance WHERE subscriber_note_id = $1",
            &[&note_id],
        )
        .await?;
    
    let (base_note_id, subscribed_fields): (Option<i64>, Option<Vec<i32>>) = match inheritance_row {
        Some(row) => (Some(row.get(0)), row.get(1)),
        None => (None, None),
    };
    
    // If inherited, fetch base note reviewed fields and overlay
    let mut inherited_positions: std::collections::HashSet<u32> = std::collections::HashSet::new();
    if let Some(base_id) = base_note_id {
        let base_fields = client
            .query(
                "SELECT position, content FROM fields WHERE note = $1 AND reviewed = true",
                &[&base_id],
            )
            .await?;
        
        let is_subscribed = |pos: i32| -> bool {
            match &subscribed_fields {
                None => true, // Subscribe all
                Some(v) => v.contains(&pos),
            }
        };
        
        for row in base_fields {
            let pos: u32 = row.get(0);
            let content: String = row.get(1);
            if is_subscribed(pos as i32) {
                reviewed_map.insert(pos, cleanser::clean(&content));
                inherited_positions.insert(pos);
            }
        }
    }
    
    // Build the response
    let mut fields: Vec<EditableFieldInfo> = Vec::with_capacity(field_defs.len());
    for row in field_defs {
        let position: u32 = row.get(0);
        let name: String = row.get(1);
        
        let reviewed_content = reviewed_map.get(&position).cloned().unwrap_or_default();
        let (suggestion_id, suggestion_content) = match suggestion_map.get(&position) {
            Some((id, content)) => (Some(*id), Some(content.clone())),
            None => (None, None),
        };
        let inherited = inherited_positions.contains(&position);
        let has_other_suggestions = other_suggestions_positions.contains(&position);
        
        fields.push(EditableFieldInfo {
            position,
            name,
            reviewed_content,
            suggestion_content,
            suggestion_id,
            inherited,
            has_other_suggestions,
        });
    }
    
    Ok(AllFieldsForEditResponse {
        note_id,
        commit_id,
        note_reviewed,
        fields,
    })
}

/// Result of creating or updating a single field suggestion
pub struct FieldSuggestionResult {
    pub position: u32,
    pub field_id: i64,
    pub action: String,
    pub old_content: Option<String>,
    pub new_content: String,
}

/// Batch create or update field suggestions for a note.
/// This handles the "Save Changes" action from the edit all fields panel.
/// Skips protected field validation (maintainer override).
/// Returns results for each field processed.
pub async fn batch_create_or_update_field_suggestions(
    tx: &tokio_postgres::Transaction<'_>,
    note_id: i64,
    commit_id: i32,
    fields: &[crate::structs::FieldSuggestionUpdate],
    actor_user_id: i32,
    client_ip: &str,
) -> Return<Vec<FieldSuggestionResult>> {
    use crate::note_history::{self, EventType};
    
    // Get note info
    let note_row = tx
        .query_opt(
            "SELECT notetype, reviewed FROM notes WHERE id = $1 AND deleted = false",
            &[&note_id],
        )
        .await?;
    
    let (notetype_id, _note_reviewed): (i64, bool) = match note_row {
        Some(row) => (row.get(0), row.get(1)),
        None => return Err(NoteNotFound(NoteNotFoundContext::FieldUpdate)),
    };
    
    // Get all field positions from notetype (for validation)
    let valid_positions: std::collections::HashSet<u32> = tx
        .query(
            "SELECT position FROM notetype_field WHERE notetype = $1",
            &[&notetype_id],
        )
        .await?
        .into_iter()
        .map(|r| r.get::<_, u32>(0))
        .collect();
    
    // Get current reviewed content for each position
    let reviewed_rows = tx
        .query(
            "SELECT position, content FROM fields WHERE note = $1 AND reviewed = true",
            &[&note_id],
        )
        .await?;
    let mut reviewed_map: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    for row in reviewed_rows {
        let pos: u32 = row.get(0);
        let content: String = row.get(1);
        reviewed_map.insert(pos, content);
    }
    
    // Get existing unreviewed suggestions for this commit
    let existing_suggestions = tx
        .query(
            "SELECT id, position, content FROM fields WHERE note = $1 AND reviewed = false AND commit = $2",
            &[&note_id, &commit_id],
        )
        .await?;
    let mut suggestion_map: std::collections::HashMap<u32, (i64, String)> = std::collections::HashMap::new();
    for row in existing_suggestions {
        let id: i64 = row.get(0);
        let pos: u32 = row.get(1);
        let content: String = row.get(2);
        suggestion_map.insert(pos, (id, content));
    }
    
    // Check for inheritance to determine which fields are inherited (read-only)
    let inheritance_row = tx
        .query_opt(
            "SELECT base_note_id, subscribed_fields FROM note_inheritance WHERE subscriber_note_id = $1",
            &[&note_id],
        )
        .await?;
    
    let inherited_positions: std::collections::HashSet<u32> = if let Some(row) = inheritance_row {
        let subscribed_fields: Option<Vec<i32>> = row.get(1);
        match subscribed_fields {
            None => valid_positions.clone(), // All fields inherited
            Some(v) => v.into_iter().filter(|&p| p >= 0).map(|p| p as u32).collect(),
        }
    } else {
        std::collections::HashSet::new()
    };
    
    let mut results: Vec<FieldSuggestionResult> = Vec::with_capacity(fields.len());
    
    for field_update in fields {
        let position = field_update.position;
        
        // Validate position exists in notetype
        if !valid_positions.contains(&position) {
            continue; // Skip invalid positions
        }
        
        // Skip inherited fields (read-only)
        if inherited_positions.contains(&position) {
            continue;
        }
        
        // Clean and sanitize content
        let cleaned_content_r = field_update.content.replace('\u{200B}', "");
        let new_content = cleanser::clean(&cleaned_content_r);
        
        // Get the baseline (reviewed content for this position)
        let reviewed_content = reviewed_map.get(&position).cloned().unwrap_or_default();
        
        // Check if there's an existing suggestion for this commit
        if let Some((existing_id, existing_content)) = suggestion_map.get(&position) {
            // Existing suggestion found
            let old_clean = cleanser::clean(existing_content);
            
            if new_content == old_clean {
                // No change
                results.push(FieldSuggestionResult {
                    position,
                    field_id: *existing_id,
                    action: "unchanged".to_string(),
                    old_content: Some(old_clean),
                    new_content,
                });
            } else if new_content == reviewed_content {
                // Content matches reviewed - delete the suggestion
                tx.execute("DELETE FROM fields WHERE id = $1", &[existing_id])
                    .await?;
                
                // Log the removal
                let _ = note_history::log_event(
                    tx,
                    note_id,
                    EventType::FieldRemoved,
                    Some(&serde_json::json!({
                        "position": position,
                        "content": old_clean,
                        "suggestion_removed": true
                    })),
                    None,
                    Some(actor_user_id),
                    Some(commit_id),
                    None,
                )
                .await;
                
                results.push(FieldSuggestionResult {
                    position,
                    field_id: *existing_id,
                    action: "removed".to_string(),
                    old_content: Some(old_clean),
                    new_content,
                });
            } else {
                // Update existing suggestion
                tx.execute(
                    "UPDATE fields SET content = $1 WHERE id = $2",
                    &[&new_content, existing_id],
                )
                .await?;
                
                // Log the update
                let _ = note_history::log_event(
                    tx,
                    note_id,
                    EventType::FieldUpdated,
                    Some(&serde_json::json!({
                        "position": position,
                        "content": old_clean,
                        "suggestion_update": true
                    })),
                    Some(&serde_json::json!({
                        "position": position,
                        "content": new_content,
                        "suggestion_update": true
                    })),
                    Some(actor_user_id),
                    Some(commit_id),
                    None,
                )
                .await;
                
                results.push(FieldSuggestionResult {
                    position,
                    field_id: *existing_id,
                    action: "updated".to_string(),
                    old_content: Some(old_clean),
                    new_content,
                });
            }
        } else {
            // No existing suggestion for this commit at this position
            if new_content == reviewed_content {
                // No change from reviewed content - skip
                continue;
            }
            
            if new_content.is_empty() && reviewed_content.is_empty() {
                // Both empty - skip
                continue;
            }
            
            // Create new suggestion
            let new_id: i64 = tx
                .query_one(
                    "INSERT INTO fields (note, position, content, creator_ip, commit, reviewed) VALUES ($1, $2, $3, $4, $5, false) RETURNING id",
                    &[&note_id, &position, &new_content, &client_ip, &commit_id],
                )
                .await?
                .get(0);
            
            // Log the creation
            let _ = note_history::log_event(
                tx,
                note_id,
                EventType::FieldUpdated,
                Some(&serde_json::json!({
                    "position": position,
                    "reviewed_content": reviewed_content,
                    "suggestion_created": true
                })),
                Some(&serde_json::json!({
                    "position": position,
                    "content": new_content,
                    "suggestion_created": true
                })),
                Some(actor_user_id),
                Some(commit_id),
                None,
            )
            .await;
            
            results.push(FieldSuggestionResult {
                position,
                field_id: new_id,
                action: "created".to_string(),
                old_content: if reviewed_content.is_empty() { None } else { Some(reviewed_content) },
                new_content,
            });
        }
    }
    
    Ok(results)
}

pub async fn approve_field_change(
    tx: &tokio_postgres::Transaction<'_>,
    field_id: i64,
    update_timestamp: bool,
    actor_user_id: i32,
) -> Return<String> {
    approve_field_change_with_commit(tx, field_id, update_timestamp, None, actor_user_id).await
}

pub async fn approve_field_change_with_commit(
    tx: &tokio_postgres::Transaction<'_>,
    field_id: i64,
    update_timestamp: bool,
    commit_id: Option<i32>,
    actor_user_id: i32,
) -> Return<String> {
    let field_info_row = tx
        .query_opt(
            "SELECT note, content, position, commit FROM fields WHERE id = $1",
            &[&field_id],
        )
        .await?;

    let (note_id, field_content, field_position, suggestion_commit): (
        i64,
        String,
        u32,
        Option<i32>,
    ) = match field_info_row {
        Some(row) => (row.get(0), row.get(1), row.get(2), row.get(3)),
        None => {
            tracing::warn!(field_id = field_id, "Field not found during approval");
            return Err(NoteNotFound(NoteNotFoundContext::FieldApprove));
        }
    };

    let effective_commit_id = commit_id.or(suggestion_commit);

    // Enforce invariants around field 0 and empty content approvals
    let is_empty = field_content.trim().is_empty();

    if is_empty && field_position == 0 {
        // Cannot approve an empty first field
        return Err(InvalidNote);
    }

    // Determine note review state to tailor invariants for new vs reviewed notes
    let note_reviewed: bool = tx
        .query_one("SELECT reviewed FROM notes WHERE id = $1", &[&note_id])
        .await?
        .get(0);

    // We'll capture old reviewed field (if any) before mutation for event logging
    let prior_reviewed_same_pos = tx.query(
        "SELECT id, content, reviewed FROM fields WHERE note = $1 AND position = $2 AND reviewed = true AND id <> $3",
        &[&note_id, &field_position, &field_id]
    ).await?;
    let old_field_json = if prior_reviewed_same_pos.is_empty() {
        None
    } else {
        let c: String = prior_reviewed_same_pos[0].get(1);
        Some(serde_json::json!({
            "position": field_position,
            "content": cleanser::clean(&c),
            "reviewed": true
        }))
    };

    if is_empty {
        // VALIDATE BEFORE DELETION: Check all invariants first
        if note_reviewed {
            // For reviewed notes: ensure field 0 will remain present and non-empty
            let exists_pos0 = tx
                .query_one(
                    "SELECT EXISTS(SELECT 1 FROM fields WHERE note = $1 AND position = 0 AND reviewed = true AND content <> '')",
                    &[&note_id],
                )
                .await?
                .get::<_, bool>(0);
            if !exists_pos0 {
                return Err(InvalidNote);
            }
        } else {
            // For unreviewed notes: count how many fields will remain after deletion
            // We're deleting: 1) reviewed fields at this position, 2) the suggestion itself
            let reviewed_at_position: i64 = tx
                .query_one(
                    "SELECT COUNT(*) FROM fields WHERE note = $1 AND position = $2 AND reviewed = true AND id <> $3",
                    &[&note_id, &field_position, &field_id],
                )
                .await?
                .get(0);
            let total_fields: i64 = tx
                .query_one("SELECT COUNT(*) FROM fields WHERE note = $1", &[&note_id])
                .await?
                .get(0);
            // After deletion: total - reviewed_at_position - 1 (the suggestion)
            let remaining_after = total_fields - reviewed_at_position - 1;
            if remaining_after <= 0 {
                return Err(InvalidNote);
            }
        }

        // Now safe to perform deletions - invariants validated above
        // Remove existing reviewed field(s) at that position and drop the empty suggestion
        tx.execute(
            "DELETE FROM fields
             WHERE reviewed = true
               AND note = $1
               AND position = $2
               AND id <> $3",
            &[&note_id, &field_position, &field_id],
        )
        .await?;

        tx.execute("DELETE FROM fields WHERE id = $1", &[&field_id])
            .await?;
    } else {
        // Non-empty: replace any reviewed at same position, then approve this one
        tx.execute(
            "DELETE FROM fields
             WHERE reviewed = true
               AND note = $1
               AND position = $2
               AND id <> $3",
            &[&note_id, &field_position, &field_id],
        )
        .await?;

        tx.execute(
            "UPDATE fields SET reviewed = true WHERE id = $1",
            &[&field_id],
        )
        .await?;
    }

    // Final invariant check before commit (only for reviewed notes): field 0 must exist, be reviewed and non-empty
    if note_reviewed {
        let pos0_ok = tx
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM fields WHERE note = $1 AND position = 0 AND reviewed = true AND content <> '')",
                &[&note_id],
            )
            .await?
            .get::<_, bool>(0);

        if !pos0_ok {
            return Err(InvalidNote);
        }
    }

    // Decide event type & construct JSON payloads now that DB state changed
    // Determine if this approval created, updated, or removed a field
    // Removal path already executed when is_empty and we deleted the suggestion and any reviewed field
    if is_empty {
        // Emit FieldRemoved if there was a prior reviewed field at that position
        if old_field_json.is_some() {
            let _ = note_history::log_event(
                tx,
                note_id,
                EventType::FieldRemoved,
                old_field_json.as_ref(),
                None,
                Some(actor_user_id),
                effective_commit_id,
                Some(true),
            )
            .await?;
        } else {
            // Removing an unreviewed suggestion that had no prior reviewed field -> treat as SuggestionDenied (approved=false)
            let _ = note_history::log_event(
                tx,
                note_id,
                EventType::SuggestionDenied,
                Some(&serde_json::json!({"type":"field","position": field_position,"reviewed": false})),
                None,
            Some(actor_user_id),
                effective_commit_id,
                Some(false),
            ).await?;
        }
    } else {
        let new_json = serde_json::json!({
            "position": field_position,
            "content": cleanser::clean(&field_content),
            "reviewed": true
        });
        let event_type = if old_field_json.is_none() {
            EventType::FieldAdded
        } else {
            EventType::FieldUpdated
        };
        let _ = note_history::log_event(
            tx,
            note_id,
            event_type,
            old_field_json.as_ref(),
            Some(&new_json),
            Some(actor_user_id),
            effective_commit_id,
            Some(true),
        )
        .await?;
    }

    // Timestamp bumps (note + subscribers) collected then updated in bulk
    let mut bump: Vec<i64> = Vec::new();
    if update_timestamp {
        bump.push(note_id);
    }
    let subs = tx
        .query(
            "SELECT subscriber_note_id FROM note_inheritance WHERE base_note_id = $1",
            &[&note_id],
        )
        .await?;
    for r in subs {
        bump.push(r.get(0));
    }
    if !bump.is_empty() {
        let _ = update_notes_timestamps(&tx, &bump).await;
    }

    Ok(note_id.to_string())
}

/// Result for a single note in the bulk merge operation
pub struct BulkNoteResult {
    pub note_id: i64,
    pub success: bool,
    pub reason: Option<String>,
}

/// Merge or deny a specific set of notes within a commit.
/// This function processes each note independently, allowing partial success.
/// Returns a list of results indicating success/failure for each note.
pub async fn merge_by_note_ids(
    db_state: &Arc<database::AppState>,
    commit_id: i32,
    note_ids: &[i64],
    approve: bool,
    user: &User,
) -> Return<Vec<BulkNoteResult>> {
    if note_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut client = database::client(db_state).await?;

    // Verify commit exists and get deck
    let q_guid = client
        .query(
            "SELECT deck FROM commits WHERE commit_id = $1",
            &[&commit_id],
        )
        .await?;
    if q_guid.is_empty() {
        return Err(CommitDeckNotFound);
    }
    let deck_id: i64 = q_guid[0].get(0);

    // Verify user authorization
    let access = is_authorized(db_state, user, deck_id).await?;
    if !access {
        return Err(Unauthorized);
    }

    // Query all notes affected by this commit to verify the provided note_ids are valid
    let affected_notes_query = r"
        SELECT notes.id, notes.reviewed FROM notes
        JOIN (
            SELECT note FROM fields WHERE commit = $1 AND reviewed = false
            UNION
            SELECT note FROM tags WHERE commit = $1 AND reviewed = false
            UNION
            SELECT note FROM card_deletion_suggestions WHERE commit = $1
            UNION
            SELECT note FROM note_move_suggestions WHERE commit = $1
        ) AS n ON notes.id = n.note
        WHERE notes.id = ANY($2)
        GROUP BY notes.id
    ";
    let valid_notes = client
        .query(affected_notes_query, &[&commit_id, &note_ids])
        .await?;

    use std::collections::{HashMap, HashSet};
    let valid_note_map: HashMap<i64, bool> = valid_notes
        .iter()
        .map(|row| (row.get::<_, i64>(0), row.get::<_, bool>(1)))
        .collect();

    let valid_note_ids: HashSet<i64> = valid_note_map.keys().copied().collect();

    let mut results: Vec<BulkNoteResult> = Vec::with_capacity(note_ids.len());

    // Mark notes not part of this commit as failed
    for &note_id in note_ids {
        if !valid_note_ids.contains(&note_id) {
            results.push(BulkNoteResult {
                note_id,
                success: false,
                reason: Some("Note not part of this commit or already processed".to_string()),
            });
        }
    }

    // Process each valid note independently in its own transaction
    let notes_to_process: Vec<i64> = note_ids
        .iter()
        .copied()
        .filter(|id| valid_note_ids.contains(id))
        .collect();

    for note_id in notes_to_process {
        let is_reviewed = valid_note_map.get(&note_id).copied().unwrap_or(false);

        let tx = client.transaction().await?;
        let result = process_single_note_merge(
            &tx,
            db_state,
            commit_id,
            note_id,
            is_reviewed,
            approve,
            user,
        )
        .await;

        match result {
            Ok(()) => {
                if let Err(e) = tx.commit().await {
                    results.push(BulkNoteResult {
                        note_id,
                        success: false,
                        reason: Some(format!("Transaction commit failed: {e}")),
                    });
                } else {
                    results.push(BulkNoteResult {
                        note_id,
                        success: true,
                        reason: None,
                    });
                }
            }
            Err(e) => {
                let _ = tx.rollback().await;
                results.push(BulkNoteResult {
                    note_id,
                    success: false,
                    reason: Some(e.to_string()),
                });
            }
        }
    }

    // Background task to update media references for successful notes
    let succeeded_ids: Vec<i64> = results
        .iter()
        .filter(|r| r.success)
        .map(|r| r.note_id)
        .collect();
    if !succeeded_ids.is_empty() {
        let state_clone = db_state.clone();
        tokio::spawn(async move {
            if let Err(e) = media_reference_manager::update_media_references_for_commit(
                &state_clone,
                &succeeded_ids,
            )
            .await
            {
                tracing::warn!(error = ?e, "Failed to update media references for bulk merge");
            }
        });
    }

    Ok(results)
}

/// Process a single note for merge/deny. Extracted from merge_by_commit for reuse.
/// This function operates within the provided transaction and handles all aspects
/// of merging or denying a single note.
async fn process_single_note_merge(
    tx: &tokio_postgres::Transaction<'_>,
    db_state: &Arc<database::AppState>,
    commit_id: i32,
    note_id: i64,
    is_reviewed: bool,
    approve: bool,
    user: &User,
) -> Return<()> {
    // Get tags for this note in this commit
    let affected_tags: Vec<i64> = tx
        .query(
            "SELECT id FROM tags WHERE commit = $1 AND note = $2 AND reviewed = false",
            &[&commit_id, &note_id],
        )
        .await?
        .into_iter()
        .map(|row| row.get(0))
        .collect();

    // Get fields for this note in this commit
    let affected_fields: Vec<i64> = tx
        .query(
            "SELECT id FROM fields WHERE commit = $1 AND note = $2 AND reviewed = false",
            &[&commit_id, &note_id],
        )
        .await?
        .into_iter()
        .map(|row| row.get(0))
        .collect();

    // Check for deletion suggestion
    let deletion_row = tx
        .query(
            "SELECT note FROM card_deletion_suggestions WHERE commit = $1 AND note = $2",
            &[&commit_id, &note_id],
        )
        .await?;
    let has_deletion = !deletion_row.is_empty();

    // Check for move suggestion
    let move_row = tx
        .query(
            "SELECT note, target_deck FROM note_move_suggestions WHERE commit = $1 AND note = $2",
            &[&commit_id, &note_id],
        )
        .await?;
    let move_suggestion: Option<(i64, i64)> = move_row
        .first()
        .map(|row| (row.get(0), row.get(1)));

    if approve {
        // Process tags
        for tag_id in &affected_tags {
            approve_tag_change_with_commit(tx, *tag_id, false, Some(commit_id), user.id()).await?;
        }

        // Process fields
        for field_id in &affected_fields {
            approve_field_change_with_commit(tx, *field_id, false, Some(commit_id), user.id())
                .await?;
        }

        // Process deletion
        if has_deletion {
            note_manager::mark_note_deleted(tx, db_state, note_id, user.clone(), true).await?;
        }

        // Process move
        if let Some((_, target_deck)) = move_suggestion {
            approve_move_note_request(tx, note_id, target_deck, false, Some(commit_id), user.id())
                .await?;
        }

        // If note is unreviewed, approve it
        if !is_reviewed {
            approve_card(tx, db_state, note_id, user, true).await?;
        }

        // Collect and update timestamps
        let mut bump: Vec<i64> = vec![note_id];
        let subs = tx
            .query(
                "SELECT subscriber_note_id FROM note_inheritance WHERE base_note_id = $1",
                &[&note_id],
            )
            .await?;
        for r in subs {
            bump.push(r.get(0));
        }
        update_notes_timestamps(tx, &bump).await?;

        // Log approval event
        let _ = note_history::log_event(
            tx,
            note_id,
            EventType::CommitApprovedEffect,
            Some(&serde_json::json!({"commit_state": "pending"})),
            Some(&serde_json::json!({"commit_state": "approved"})),
            Some(user.id()),
            Some(commit_id),
            Some(true),
        )
        .await;
    } else {
        // Deny path
        if is_reviewed {
            // For reviewed notes, deny individual suggestions
            for tag_id in &affected_tags {
                deny_tag_change(tx, *tag_id, user.id()).await?;
            }
            for field_id in &affected_fields {
                deny_field_change(tx, *field_id, user.id()).await?;
            }
        } else {
            // For unreviewed notes, delete them entirely
            tx.execute("DELETE FROM notes WHERE id = $1", &[&note_id])
                .await?;
        }

        // Clean up deletion suggestions
        if has_deletion {
            tx.execute(
                "DELETE FROM card_deletion_suggestions WHERE commit = $1 AND note = $2",
                &[&commit_id, &note_id],
            )
            .await?;
        }

        // Clean up move suggestions
        if let Some((_, target_deck)) = move_suggestion {
            tx.execute(
                "DELETE FROM note_move_suggestions WHERE note = $1 AND target_deck = $2",
                &[&note_id, &target_deck],
            )
            .await?;
        }

        // Log denial event
        let _ = note_history::log_event(
            tx,
            note_id,
            EventType::CommitDeniedEffect,
            Some(&serde_json::json!({"commit_state": "pending"})),
            Some(&serde_json::json!({"commit_state": "denied"})),
            Some(user.id()),
            Some(commit_id),
            Some(false),
        )
        .await;
    }

    Ok(())
}

pub async fn merge_by_commit(
    db_state: &Arc<database::AppState>,
    commit_id: i32,
    approve: bool,
    user: User,
) -> Return<Option<i32>> {
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

    // Map tag->note for deny path decisions
    let tag_note_pairs = client
        .query(
            "SELECT id, note FROM tags WHERE commit = $1 AND reviewed = false",
            &[&commit_id],
        )
        .await?
        .into_iter()
        .map(|row| (row.get::<_, i64>(0), row.get::<_, i64>(1)))
        .collect::<Vec<(i64, i64)>>();

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

    // Map field->note for deny path decisions
    let field_note_pairs = client
        .query(
            "SELECT id, note FROM fields WHERE commit = $1 AND reviewed = false",
            &[&commit_id],
        )
        .await?
        .into_iter()
        .map(|row| (row.get::<_, i64>(0), row.get::<_, i64>(1)))
        .collect::<Vec<(i64, i64)>>();

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

    let affected_note_ids = affected_notes
        .iter()
        .map(|row| row.get(0))
        .collect::<Vec<i64>>();
    use std::collections::HashSet;
    let reviewed_notes: HashSet<i64> = affected_notes
        .iter()
        .filter(|row| row.get::<usize, bool>(1) == true)
        .map(|row| row.get::<usize, i64>(0))
        .collect();

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
    let next_review_query = r"
        WITH RECURSIVE accessible AS (
            SELECT id FROM decks WHERE id IN (
                SELECT deck FROM maintainers WHERE user_id = $1
                UNION
                SELECT id FROM decks WHERE owner = $1
            )
            UNION
            SELECT d.id
            FROM decks d
            INNER JOIN accessible a ON d.parent = a.id
        ),
        unreviewed_changes AS (
            SELECT commit_id, rationale, timestamp, deck
            FROM commits c
            WHERE EXISTS (
                SELECT 1 FROM fields f
                WHERE f.reviewed = false AND f.commit = c.commit_id
            )
            UNION
            SELECT commit_id, rationale, timestamp, deck
            FROM commits c
            WHERE EXISTS (
                SELECT 1 FROM tags t
                WHERE t.reviewed = false AND t.commit = c.commit_id
            )
            UNION
            SELECT commit_id, rationale, timestamp, deck
            FROM commits c
            WHERE EXISTS (
                SELECT 1 FROM card_deletion_suggestions cds
                WHERE cds.commit = c.commit_id
            )
            UNION
            SELECT commit_id, rationale, timestamp, deck
            FROM commits c
            WHERE EXISTS (
                SELECT 1 FROM note_move_suggestions nms
                WHERE nms.commit = c.commit_id
            )
        ),
        indexed_unreviewed AS (
            SELECT commit_id, ROW_NUMBER() OVER (ORDER BY timestamp) AS row_num
            FROM unreviewed_changes
            WHERE deck IN (SELECT id FROM accessible)
        )
        SELECT commit_id
        FROM indexed_unreviewed
        WHERE row_num = (
            SELECT CASE
                WHEN EXISTS (
                    SELECT 1
                    FROM indexed_unreviewed o
                    JOIN indexed_unreviewed cur ON cur.commit_id = $2
                    WHERE o.row_num > cur.row_num
                ) THEN (
                    SELECT MIN(o.row_num)
                    FROM indexed_unreviewed o
                    JOIN indexed_unreviewed cur ON cur.commit_id = $2
                    WHERE o.row_num > cur.row_num
                )
                ELSE (
                    SELECT MAX(o.row_num)
                    FROM indexed_unreviewed o
                    JOIN indexed_unreviewed cur ON cur.commit_id = $2
                    WHERE o.row_num < cur.row_num
                )
            END
        )
        ORDER BY commit_id
        LIMIT 1
    ";
    let next_review = client
        .query(next_review_query, &[&user.id(), &commit_id])
        .await?;

    // Slightly less performant to do it in single queries than doing a bigger query here, but for readability and easier code maintenance, we keep it that way.
    // The performance difference is not relevant in this case
    // Bulk processing in a single transaction; rollback on any error.
    let tx = client.transaction().await?;
    let tx_res: Return<()> = async {
        if approve {
            for tag in &affected_tags {
                let note_for_tag = tag_note_pairs
                    .iter()
                    .find(|(tid, _)| tid == tag)
                    .map(|(_, n)| *n);

                if let Err(err) = approve_tag_change_with_commit(
                    &tx,
                    *tag,
                    false,
                    Some(commit_id),
                    user.id(),
                )
                .await
                {
                    // Note: Only use sentry::with_scope, not error!() macro,
                    // because the sentry_layer auto-captures ERROR events causing double-capture
                    sentry::with_scope(
                        |scope| {
                            scope.set_tag("component", "suggestion_manager");
                            scope.set_tag("operation", "merge_by_commit_approve_tag");
                            scope.set_extra("commit_id", commit_id.into());
                            scope.set_extra("tag_id", (*tag).into());
                            scope.set_extra(
                                "note_id",
                                note_for_tag
                                    .map(|n| n.into())
                                    .unwrap_or_else(|| "unknown".into()),
                            );
                            scope.set_extra("user_id", user.id().into());
                        },
                        || {
                            sentry::capture_message(
                                "merge_by_commit: approve_tag_change failed",
                                Level::Error,
                            );
                        },
                    );
                    return Err(err);
                }
            }
            for field in &affected_fields {
                approve_field_change_with_commit(&tx, *field, false, Some(commit_id), user.id())
                    .await?;
            }
            for note in &deleted_notes {
                note_manager::mark_note_deleted(&tx, db_state, *note, user.clone(), true).await?;
            }
            for (note_id, target_deck) in &moved_deck_suggestion {
                approve_move_note_request(
                    &tx,
                    *note_id,
                    *target_deck,
                    false,
                    Some(commit_id),
                    user.id(),
                )
                .await?;
            }

            // Collect note IDs to bump timestamps (notes + their subscribers)
            let mut bump: Vec<i64> = Vec::new();
            for row in &affected_notes {
                let note_id: i64 = row.get(0);
                let reviewed: bool = row.get(1);
                if !reviewed {
                    let _ = approve_card(&tx, db_state, note_id, &user, true).await?;
                }
                bump.push(note_id);
            }
            if !affected_note_ids.is_empty() {
                let subs = tx.query(
                    "SELECT subscriber_note_id FROM note_inheritance WHERE base_note_id = ANY($1)",
                    &[&affected_note_ids]
                ).await?;
                for r in subs {
                    bump.push(r.get(0));
                }
            }
            if !bump.is_empty() {
                update_notes_timestamps(&tx, &bump).await?;
            }
            // Commit approved effect events per affected reviewed note (after operations)
            for nid in &affected_note_ids {
                let _ = note_history::log_event(
                    &tx,
                    *nid,
                    EventType::CommitApprovedEffect,
                    Some(&serde_json::json!({"commit_state":"pending"})),
                    Some(&serde_json::json!({"commit_state":"approved"})),
                    Some(user.id()),
                    Some(commit_id),
                    Some(true),
                )
                .await;
            }
        } else {
            // Only deny individual suggestions for reviewed notes; unreviewed notes are removed in bulk below
            for (tag_id, note_id) in &tag_note_pairs {
                if reviewed_notes.contains(note_id) {
                    let _ = deny_tag_change(&tx, *tag_id, user.id()).await?;
                }
            }
            for (field_id, note_id) in &field_note_pairs {
                if reviewed_notes.contains(note_id) {
                    let _ = deny_field_change(&tx, *field_id, user.id()).await?;
                }
            }

            // Remove unreviewed notes and any pending suggestions tied to them (cascade handles suggestions)
            let unreviewed_note_ids: Vec<i64> = affected_notes
                .iter()
                .filter(|row| !row.get::<usize, bool>(1))
                .map(|row| row.get::<usize, i64>(0))
                .collect();
            if !unreviewed_note_ids.is_empty() {
                tx.execute(
                    "DELETE FROM notes WHERE id = ANY($1)",
                    &[&unreviewed_note_ids],
                )
                .await?;
            }
            // Clean up commit-level suggestions not tied to unreviewed notes, in batches
            if !deleted_notes.is_empty() {
                tx.execute(
                    "DELETE FROM card_deletion_suggestions WHERE note = ANY($1)",
                    &[&deleted_notes],
                )
                .await?;
            }
            if !moved_deck_suggestion.is_empty() {
                let move_notes: Vec<i64> = moved_deck_suggestion.iter().map(|(n, _)| *n).collect();
                let move_targets: Vec<i64> =
                    moved_deck_suggestion.iter().map(|(_, d)| *d).collect();
                tx.execute(
                    "DELETE FROM note_move_suggestions nms
                     USING unnest($1::bigint[], $2::bigint[]) AS t(note_id, target_deck)
                     WHERE nms.note = t.note_id AND nms.target_deck = t.target_deck",
                    &[&move_notes, &move_targets],
                )
                .await?;
            }
            // Denial commit events
            for nid in &affected_note_ids {
                let _ = note_history::log_event(
                    &tx,
                    *nid,
                    EventType::CommitDeniedEffect,
                    Some(&serde_json::json!({"commit_state":"pending"})),
                    Some(&serde_json::json!({"commit_state":"denied"})),
                    Some(user.id()),
                    Some(commit_id),
                    Some(false),
                )
                .await;
            }
        }
        Ok(())
    }
    .await;

    match tx_res {
        Ok(()) => {
            tx.commit().await?;
        }
        Err(e) => {
            // Explicit rollback for clarity; drop would also rollback
            let _ = tx.rollback().await;
            return Err(e);
        }
    }

    let state_clone = db_state.clone();
    tokio::spawn(async move {
        if let Err(e) = media_reference_manager::update_media_references_for_commit(
            &state_clone,
            &affected_note_ids,
        )
        .await
        {
            tracing::warn!(error = ?e, "Failed to update media references for commit");
        }
    });

    // Get next outstanding commit id and return it (if any)
    if next_review.is_empty() {
        return Ok(None);
    }
    Ok(Some(next_review[0].get(0)))
}
