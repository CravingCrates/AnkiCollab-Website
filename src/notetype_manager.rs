use crate::user::User;

use crate::database;
use crate::error::Error::NoNoteTypesAffected;
use crate::structs::{NoteModelFieldInfo, NotetypeOverview, UpdateNotetype};
use crate::Return;

use std::collections::HashMap;
use std::sync::Arc;

pub async fn get_protected_fields(
    db_state: &Arc<database::AppState>,
    notetype_id: i64,
) -> Return<Vec<NoteModelFieldInfo>> {
    let client = database::client(db_state).await?;
    let query = "SELECT id, name, protected, position FROM notetype_field WHERE notetype = $1 ORDER BY position";

    let rows = client
        .query(query, &[&notetype_id])
        .await?
        .into_iter()
        .map(|row| NoteModelFieldInfo {
            id: row.get(0),
            name: row.get(1),
            protected: row.get(2),
        })
        .collect::<Vec<_>>();

    Ok(rows)
}

pub async fn notetypes_by_commit(
    db_state: &Arc<database::AppState>,
    commit_id: i32,
) -> Return<HashMap<i64, Vec<String>>> {
    // Returns a map of notetypes with a vector of the field names of that notetype
    let client = database::client(db_state).await?;
    let get_notetypes = "
        SELECT DISTINCT notetype FROM notes
        WHERE notes.id IN 
        (
            SELECT fields.note FROM fields WHERE fields.commit = $1 
            UNION 
            SELECT tags.note FROM tags WHERE tags.commit = $1
            UNION
            SELECT card_deletion_suggestions.note FROM card_deletion_suggestions WHERE card_deletion_suggestions.commit = $1
            UNION
            SELECT note_move_suggestions.note FROM note_move_suggestions WHERE note_move_suggestions.commit = $1
        )
    ";
    let affected_notetypes = client
        .query(get_notetypes, &[&commit_id])
        .await?
        .into_iter()
        .map(|row| row.get::<_, i64>("notetype"))
        .collect::<Vec<i64>>();

    if affected_notetypes.is_empty() {
        return Err(NoNoteTypesAffected);
    }

    let mut notetype_map = HashMap::new();

    for notetype in affected_notetypes {
        let get_fields = "
            SELECT name FROM notetype_field
            WHERE notetype = $1 order by position
        ";
        let fields = client
            .query(get_fields, &[&notetype])
            .await?
            .into_iter()
            .map(|row| row.get::<_, String>("name"))
            .collect::<Vec<String>>();
        notetype_map.insert(notetype, fields);
    }

    Ok(notetype_map)
}

pub async fn update_notetype(
    db_state: &Arc<database::AppState>,
    user: &User,
    notetype: &UpdateNotetype,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut client = database::client(db_state).await?;
    let rows = client
        .query(
            "SELECT 1 FROM notetype WHERE (owner = $1 AND id = $3) OR $2 LIMIT 1",
            &[&user.id(), &user.is_admin, &notetype.notetype_id],
        )
        .await?;
    if rows.is_empty() {
        return Err("Unauthorized".into());
    }

    let tx = client.transaction().await?;

    // Batch update field protection flags only where changed, returning changed rows.
    // Collect ids & new statuses for UNNEST arrays.
    let mut field_ids: Vec<i64> = Vec::new();
    let mut new_statuses: Vec<bool> = Vec::new();
    for (id, status) in &notetype.items {
        field_ids.push(*id);
        new_statuses.push(*status);
    }

    // Only run the heavy logic if there are any candidate fields.
    let mut newly_protected_positions: Vec<i32> = Vec::new();
    if !field_ids.is_empty() {
        let changed_rows = tx
            .query(
                r#"
                WITH incoming AS (
                    SELECT unnest($1::bigint[]) AS id, unnest($2::bool[]) AS new_protected
                ), updated AS (
                    UPDATE notetype_field nf
                    SET protected = incoming.new_protected
                    FROM incoming
                    WHERE nf.id = incoming.id
                      AND nf.notetype = $3
                      AND nf.protected IS DISTINCT FROM incoming.new_protected
                    RETURNING nf.id, nf.position::int AS position, nf.protected AS new_protected
                )
                SELECT position, new_protected FROM updated
                "#,
                &[&field_ids, &new_statuses, &notetype.notetype_id],
            )
            .await?;

        for row in changed_rows {
            let pos: i32 = row.get(0);
            let new_protected: bool = row.get(1);
            if new_protected {
                newly_protected_positions.push(pos);
            }
        }
    }

    if !newly_protected_positions.is_empty() {
        newly_protected_positions.sort_unstable();
        newly_protected_positions.dedup();

        // Remove newly protected field positions from explicit subscription_field_policy arrays.
        tx.execute(
            r#"
            UPDATE subscription_field_policy
            SET subscribed_fields = (
                SELECT COALESCE(array_agg(elem ORDER BY elem), '{}')
                FROM unnest(subscribed_fields) elem
                WHERE NOT (elem = ANY($1))
            )
            WHERE notetype_id = $2
              AND subscribed_fields IS NOT NULL
              AND EXISTS (
                  SELECT 1 FROM unnest(subscribed_fields) e WHERE e = ANY($1)
              )
            "#,
            &[&newly_protected_positions, &notetype.notetype_id],
        )
        .await?;

        // Materialize NULL policies to explicit list of unprotected positions (after protection changes).
        let unprotected_rows = tx
            .query(
                "SELECT position::int FROM notetype_field WHERE notetype = $1 AND protected = false ORDER BY position",
                &[&notetype.notetype_id],
            )
            .await?;
        let unprotected_positions: Vec<i32> =
            unprotected_rows.into_iter().map(|r| r.get(0)).collect();
        tx.execute(
            "UPDATE subscription_field_policy SET subscribed_fields = $2 WHERE notetype_id = $1 AND subscribed_fields IS NULL",
            &[&notetype.notetype_id, &unprotected_positions],
        )
        .await?;
    }

    tx.execute(
        "UPDATE notetype SET css = $1 WHERE id = $2",
        &[&notetype.styling, &notetype.notetype_id],
    )
    .await?;

    for template in &notetype.templates {
        tx.execute(
            "UPDATE notetype_template SET qfmt = $1, afmt = $2 WHERE id = $3 AND notetype = $4",
            &[
                &template.front,
                &template.back,
                &template.template_id,
                &notetype.notetype_id,
            ],
        )
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

pub async fn get_notetype_overview(
    db_state: &Arc<database::AppState>,
    user: &User,
) -> Result<Vec<NotetypeOverview>, Box<dyn std::error::Error>> {
    let client = database::client(db_state).await?;

    let query = client
        .prepare(
            "
        SELECT nt.id, nt.name, count(notes.*) AS note_count
        FROM notetype nt
        LEFT JOIN notes ON notes.notetype = nt.id
        WHERE nt.owner = $1
        GROUP BY nt.id
    ",
        )
        .await?;

    let rows = client
        .query(&query, &[&user.id()])
        .await?
        .into_iter()
        .map(|row| NotetypeOverview {
            id: row.get(0),
            name: row.get(1),
            notecount: row.get(2),
        })
        .collect::<Vec<_>>();

    Ok(rows)
}
