use crate::database;
use crate::structs::*;

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

pub async fn get_commit_info(commit_id: i32) -> Result<CommitsOverview, Box<dyn std::error::Error>> {
    let query = r#"    
        SELECT c.commit_id, c.rationale,
        TO_CHAR(c.timestamp, 'MM/DD/YYYY') AS last_update,
        d.name
        FROM commits c
        JOIN decks d on d.id = c.deck
        WHERE c.commit_id = $1
    "#; 
    let client = database::TOKIO_POSTGRES_POOL.get().unwrap().get().await.unwrap();
    let row = client.query_one(query, &[&commit_id]).await?;
    let commit = CommitsOverview {
        id: row.get(0),
        rationale: get_string_from_rationale(row.get(1)).into(),
        timestamp: row.get(2),
        deck: row.get(3)
    };
    Ok(commit)
}

pub async fn commits_review(uid: i32) -> Result<Vec<CommitsOverview>, Box<dyn std::error::Error>> {    
    let query = r#"
        WITH accessable AS (
            SELECT id FROM decks WHERE id IN (
                SELECT deck FROM maintainers WHERE user_id = $1
                UNION
                SELECT id FROM decks WHERE owner = $1
            )
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
        )
        SELECT commit_id, rationale, TO_CHAR(timestamp, 'MM/DD/YYYY') AS last_update
        FROM unreviewed_changes WHERE deck IN (SELECT id FROM accessable) OR (select is_admin from users where id = $1)
    "#;
    let client = database::TOKIO_POSTGRES_POOL.get().unwrap().get().await.unwrap();

    let rows = client.query(query, &[&uid])
    .await?
    .into_iter()
    .map(|row| CommitsOverview {
    id: row.get(0),
    rationale: get_string_from_rationale(row.get(1)).into(),
    timestamp: row.get(2),
    deck: String::new()
    })
    .collect::<Vec<_>>();

    Ok(rows)
}

pub async fn notes_by_commit(commit_id: i32) -> Result<Vec<CommitData>, Box<dyn std::error::Error>> {
    let client = database::TOKIO_POSTGRES_POOL.get().unwrap().get().await.unwrap();

    let get_notes = "
        SELECT note FROM (
            SELECT note FROM fields WHERE commit = $1 and reviewed = false
            UNION ALL
            SELECT note FROM tags WHERE commit = $1 and reviewed = false
        ) AS n
        GROUP BY note
        LIMIT 100
    ";
    let affected_notes = client.query(get_notes, &[&commit_id])
    .await?
    .into_iter()
    .map(|row| row.get::<_, i64>("note"))
    .collect::<Vec<i64>>();

    if affected_notes.is_empty() {
        return Err("No notes affected by this commit.".into());
    }


    let note_info_query = "
        SELECT id, guid, TO_CHAR(last_update, 'MM/DD/YYYY HH12:MI AM') AS last_update, reviewed, 
        (Select owner from decks where id = notes.deck), (select full_path from decks where id = notes.deck) as full_path, notetype
        FROM notes
        WHERE id = $1
    ";

    let fields_query = "
        SELECT f1.id, f1.position, f1.content, COALESCE(f2.content, '') AS reviewed_content 
        FROM fields f1 
        LEFT JOIN fields f2 
        ON f1.note = f2.note AND f1.position = f2.position AND f2.reviewed = true 
        WHERE f1.reviewed = false AND f1.commit = $1 AND f1.note = $2
        ORDER BY position
    ";

    let tags_query = "
        SELECT id, content, action
        FROM tags
        WHERE commit = $1 and note = $2 and reviewed = false
    ";
   
    let mut commit_info = vec![];
    commit_info.reserve(affected_notes.len());

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
            fields: Vec::new(),
            new_tags: Vec::new(),
            removed_tags: Vec::new(),
        };
    
        // Fill generic note info
        let note_res = client.query_one(note_info_query, &[&note_id]).await?;
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

        // Now get to the actual good bits (unreviewed material!)
        let fields_rows = client.query(fields_query, &[&commit_id, &note_id]).await?;
        for row in fields_rows {
            let id = row.get(0);
            let position = row.get(1);
            let content = row.get(2);
            let reviewed = row.get(3);
            if let Some(content) = content {
                current_note.fields.push(FieldsReviewInfo { id, position, content: ammonia::clean(content), reviewed_content: ammonia::clean(reviewed) });
            }
        
        }
        let tags_rows = client.query(tags_query, &[&commit_id, &note_id]).await?;
        for row in tags_rows {
            let id = row.get(0);
            let content = row.get(1);
            let action = row.get(2);
            if let Some(content) = content {
                if action { // New suggested tag
                    current_note.new_tags.push(TagsInfo {id, content});
                } else { // Tag got removed                    
                    current_note.removed_tags.push(TagsInfo {id, content});
                }
            }
        }
        if current_note.fields.len() > 0 || current_note.new_tags.len() > 0 || current_note.removed_tags.len() > 0 {
            commit_info.push(current_note);
        }
    }
    Ok::<Vec<CommitData>, Box<dyn std::error::Error>>(commit_info)
}