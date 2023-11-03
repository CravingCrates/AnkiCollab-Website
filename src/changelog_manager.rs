use crate::database;
use crate::structs::*;

pub async fn insert_new_changelog(
    deck_hash: &String,
    message: &String,
) -> Result<(), Box<dyn std::error::Error>> {
    let query = r#"
        INSERT INTO changelogs (deck, message, timestamp)
        VALUES ((SELECT id FROM decks WHERE human_hash = $1), $2, NOW())
    "#;
    let client = database::client().await;
    client.execute(query, &[&deck_hash, &message]).await?;
    Ok(())
}

pub async fn get_changelogs(
    deck_hash: &String,
) -> Result<Vec<ChangelogInfo>, Box<dyn std::error::Error>> {
    let client = database::client().await;

    let query = "SELECT id, message, TO_CHAR(timestamp, 'MM/DD/YYYY HH24:MI:SS') AS timestamp FROM changelogs WHERE deck = (SELECT id FROM decks WHERE human_hash = $1) ORDER BY timestamp DESC LIMIT 5";

    let rows = client
        .query(query, &[&deck_hash])
        .await?
        .into_iter()
        .map(|row| ChangelogInfo {
            id: row.get(0),
            message: row.get(1),
            timestamp: row.get(2),
        })
        .collect::<Vec<_>>();

    Ok(rows)
}

pub async fn delete_changelog(id: i64, user_id: i32) -> Result<String, Box<dyn std::error::Error>> {
    let query = r#"
        DELETE FROM changelogs
        WHERE id = $1 AND deck IN (SELECT id FROM decks WHERE owner = $2)
        RETURNING deck
    "#;
    let client = database::client().await;
    let row = match client.query_opt(query, &[&id, &user_id]).await? {
        Some(row) => row,
        None => return Err("Deck not found".into()),
    };
    let deck_id: i64 = row.get(0);
    let deck_hash_query = "SELECT human_hash FROM decks WHERE id = $1";
    let deck_hash_row = client.query_one(deck_hash_query, &[&deck_id]).await?;
    let deck_hash: String = deck_hash_row.get(0);
    Ok(deck_hash)
}
