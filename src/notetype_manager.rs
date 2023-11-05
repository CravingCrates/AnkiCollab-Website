use rocket_auth::User;

use crate::database;
use crate::error::Error::*;
use crate::structs::*;
use crate::Return;

use std::collections::HashMap;

pub async fn get_protected_fields(notetype_id: i64) -> Return<Vec<NoteModelFieldInfo>> {
    let client = database::client().await?;
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

pub async fn notetypes_by_commit(commit_id: i32) -> Return<HashMap<i64, Vec<String>>> {
    // Returns a map of notetypes with a vector of the field names of that notetype
    let client = database::client().await?;
    let get_notetypes = "
        SELECT DISTINCT notetype FROM notes
        WHERE notes.id IN 
        (
            SELECT fields.note FROM fields WHERE fields.commit = $1 
            UNION 
            SELECT tags.note FROM tags WHERE tags.commit = $1
            UNION
            SELECT card_deletion_suggestions.note FROM card_deletion_suggestions WHERE card_deletion_suggestions.commit = $1
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
    user: &User,
    notetype: &UpdateNotetype,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut client = database::client().await?;
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

    for (field_id, checked) in notetype.items.iter() {
        tx.execute(
            "
            UPDATE notetype_field 
            SET protected = $1 
            WHERE id = $2 AND notetype = $3
        ",
            &[&checked, &field_id, &notetype.notetype_id],
        )
        .await?;
    }

    tx.execute(
        "UPDATE notetype SET css = $1 WHERE id = $2",
        &[&notetype.styling, &notetype.notetype_id],
    )
    .await?;
    tx.execute(
        "UPDATE notetype_template SET qfmt = $1, afmt = $2 WHERE id = $3 AND notetype = $4",
        &[
            &notetype.front,
            &notetype.back,
            &notetype.template_id,
            &notetype.notetype_id,
        ],
    )
    .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn get_notetype_overview(
    user: &User,
) -> Result<Vec<NotetypeOverview>, Box<dyn std::error::Error>> {
    let client = database::client().await?;

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
