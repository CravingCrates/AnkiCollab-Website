use crate::database;

pub async fn get_maintainers(deck: i64) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let query = "SELECT email from users WHERE id IN (SELECT user_id FROM maintainers WHERE deck = $1)";
    let client = database::TOKIO_POSTGRES_POOL.get().unwrap().get().await.unwrap();
    let users = client.query(query, &[&deck])
        .await?
        .into_iter()
        .map(|row| row.get::<_, String>("email"))
        .collect::<Vec<String>>();

    Ok(users)
}

pub async fn add_maintainer(deck: i64, email: String) -> Result<String, Box<dyn std::error::Error>> {
    let client = database::TOKIO_POSTGRES_POOL.get().unwrap().get().await.unwrap();
    let user = match client.query_one("SELECT id FROM users WHERE email = $1", &[&email]).await {
        Ok(user) => user,
        Err(_e) => return Err("User not found".into()),
    };
    let user_id: i32 = user.get(0);

    match client.query_one("SELECT id FROM maintainers WHERE deck = $1 AND user_id = $2", &[&deck, &user_id]).await {
        Ok(_no) => return Err("User is already a maintainer".into()),
        Err(e) => e,
    };

    client.execute("INSERT INTO maintainers (deck, user_id) VALUES ($1, $2)", &[&deck, &user_id]).await?;
    Ok(email)
}

pub async fn remove_maintainer(deck: i64, email: String) -> Result<String, Box<dyn std::error::Error>> {
    let client = database::TOKIO_POSTGRES_POOL.get().unwrap().get().await.unwrap();
    let user = match client.query_one("SELECT id FROM users WHERE email = $1", &[&email]).await {
        Ok(user) => user,
        Err(_e) => return Err("User not found".into()),
    };
    let user_id: i32 = user.get(0);

    client.execute("DELETE FROM maintainers WHERE deck = $1 AND user_id = $2", &[&deck, &user_id]).await?;
    Ok(email)
}