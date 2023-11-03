use crate::database;
use crate::structs;
use crate::Return;

pub async fn update_media(deck: i64, data: structs::GDriveInfo) -> Return<String> {
    let client = database::client().await?;
    let google_json = serde_json::to_value(&data.service_account)?;
    let fixed_folder = data.folder_id.trim();
    client
        .execute(
            "
        INSERT INTO service_accounts (google_data, folder_id, deck)
        VALUES ($1, $2, $3)
        ON CONFLICT (deck)
        DO UPDATE SET
            google_data = EXCLUDED.google_data,
            folder_id = EXCLUDED.folder_id
    ",
            &[&google_json, &fixed_folder, &deck],
        )
        .await?;

    Ok("All set! You're ready to use media now :)".to_string())
}
