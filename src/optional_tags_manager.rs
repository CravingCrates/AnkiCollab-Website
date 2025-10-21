use std::sync::Arc;

use crate::error::Error::TagAlreadyExists;
use crate::{database, Return};

pub async fn get_tags(
    db_state: &Arc<database::AppState>,
    deck: i64,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let query = "SELECT tag_group from optional_tags WHERE deck = $1";
    let client = database::client(db_state).await?;
    let tags = client
        .query(query, &[&deck])
        .await?
        .into_iter()
        .map(|row| row.get::<_, String>("tag_group"))
        .collect::<Vec<String>>();

    Ok(tags)
}

pub async fn add_tag(
    db_state: &Arc<database::AppState>,
    deck: i64,
    tag_group: String,
) -> Return<String> {
    // Replace whitespaces with underscores in tag_group
    let tag_group_fixed = tag_group.replace(' ', "_");

    let client = database::client(db_state).await?;
    match client
        .query_one(
            "SELECT id FROM optional_tags WHERE deck = $1 AND tag_group = $2",
            &[&deck, &tag_group_fixed],
        )
        .await
    {
        Ok(_no) => return Err(TagAlreadyExists),
        Err(e) => e,
    };

    client
        .execute(
            "INSERT INTO optional_tags (deck, tag_group) VALUES ($1, $2)",
            &[&deck, &tag_group_fixed],
        )
        .await?;
    Ok("added".to_string())
}

pub async fn remove_tag(
    db_state: &Arc<database::AppState>,
    deck: i64,
    tag_group: String,
) -> Return<String> {
    let client = database::client(db_state).await?;
    client
        .execute(
            "DELETE FROM optional_tags WHERE deck = $1 AND tag_group = $2",
            &[&deck, &tag_group],
        )
        .await?;

    // This should remove all tags from the tags table that follow the layout AnkiCollab_Optional::tag_group::*
    client.execute("
        WITH RECURSIVE cte AS (
            SELECT $1::bigint as id
            UNION ALL
            SELECT d.id
            FROM cte JOIN decks d ON d.parent = cte.id
        )
        DELETE FROM tags WHERE content LIKE $2 AND note IN (SELECT id FROM notes WHERE deck IN (SELECT id FROM cte))",
    &[&deck, &format!("AnkiCollab_Optional::{}::%", &tag_group)]).await?;
    Ok("removed".to_string())
}
