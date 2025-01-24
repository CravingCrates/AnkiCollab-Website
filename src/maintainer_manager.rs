use std::sync::Arc;

use crate::error::Error::*;
use crate::{database, Return};

pub async fn get_maintainers(db_state: &Arc<database::AppState>, deck: i64) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let query =
        "SELECT username from users WHERE id IN (SELECT user_id FROM maintainers WHERE deck = $1)";
    let client = database::client(db_state).await?;
    let users = client
        .query(query, &[&deck])
        .await?
        .into_iter()
        .map(|row| row.get::<_, String>("username"))
        .collect::<Vec<String>>();

    Ok(users)
}

pub async fn add_maintainer(db_state: &Arc<database::AppState>, deck: i64, username: String) -> Return<String> {
    let normalized_username = username.to_lowercase();
    let client = database::client(db_state).await?;
    let user = match client
        .query_one("SELECT id FROM users WHERE username = $1", &[&normalized_username])
        .await
    {
        Ok(user) => user,
        Err(_e) => return Err(UserNotFound),
    };
    let user_id: i32 = user.get(0);

    match client
        .query_one(
            "SELECT id FROM maintainers WHERE deck = $1 AND user_id = $2",
            &[&deck, &user_id],
        )
        .await
    {
        Ok(_no) => return Err(UserIsAlreadyMaintainer),
        Err(e) => e,
    };

    client
        .execute(
            "INSERT INTO maintainers (deck, user_id) VALUES ($1, $2)",
            &[&deck, &user_id],
        )
        .await?;
    Ok("added".to_string())
}

pub async fn remove_maintainer(db_state: &Arc<database::AppState>, deck: i64, username: String) -> Return<String> {
    let normalized_username = username.to_lowercase();
    let client = database::client(db_state).await?;
    let user = match client
        .query_one("SELECT id FROM users WHERE username = $1", &[&normalized_username])
        .await
    {
        Ok(user) => user,
        Err(_e) => return Err(UserNotFound),
    };
    let user_id: i32 = user.get(0);

    client
        .execute(
            "DELETE FROM maintainers WHERE deck = $1 AND user_id = $2",
            &[&deck, &user_id],
        )
        .await?;
    Ok("removed".to_string())
}
