use crate::database;
use crate::structs;

pub async fn update_media(deck: i64, data: structs::GDriveInfo) -> Result<String, Box<dyn std::error::Error>> {
    let client = database::TOKIO_POSTGRES_POOL.get().unwrap().get().await.unwrap();
    let google_json = serde_json::to_value(&data.service_account)?;
    client.execute("
        INSERT INTO service_accounts (google_data, folder_id, deck)
        VALUES ($1, $2, $3)
        ON CONFLICT (deck)
        DO UPDATE SET
            google_data = EXCLUDED.google_data,
            folder_id = EXCLUDED.folder_id
    ", &[&google_json, &data.folder_id, &deck]).await?;

    Ok("All set! You're ready to use media now :)".to_string())
}
