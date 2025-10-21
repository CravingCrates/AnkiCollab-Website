use crate::{database, AppState};
use regex::Regex;
use std::collections::HashSet;
use std::sync::Arc;

use bb8_postgres::bb8::PooledConnection;
use bb8_postgres::PostgresConnectionManager;
use tokio_postgres::Error as PgError;

use crate::media_tokens::DownloadTokenParams;

type SharedConn = PooledConnection<'static, PostgresConnectionManager<tokio_postgres::NoTls>>;

/// Extract all media references from a field content string as anki does
#[must_use]
pub fn extract_media_references(field_content: &str) -> HashSet<String> {
    let mut references = HashSet::new();

    // Sound references [sound:filename.mp3]
    let sound_regex = Regex::new(r"\[sound:(.*?)\]").unwrap();
    for cap in sound_regex.captures_iter(field_content) {
        if let Some(filename) = cap.get(1) {
            references.insert(filename.as_str().to_string());
        }
    }

    // HTML img src
    let img_regex = Regex::new(r#"<img[^>]*src=["']([^"']*)["'][^>]*>"#).unwrap();
    for cap in img_regex.captures_iter(field_content) {
        if let Some(filename) = cap.get(1) {
            let src = filename.as_str();
            // Only consider local media files (not URLs)
            if !src.starts_with("http://")
                && !src.starts_with("https://")
                && !src.starts_with("data:")
            {
                references.insert(src.to_string());
            }
        }
    }

    // CSS url() references
    let css_regex = Regex::new(r#"url\(["']?([^"')]+)["']?\)"#).unwrap();
    for cap in css_regex.captures_iter(field_content) {
        if let Some(filename) = cap.get(1) {
            let src = filename.as_str();
            if !src.starts_with("http://")
                && !src.starts_with("https://")
                && !src.starts_with("data:")
            {
                references.insert(src.to_string());
            }
        }
    }

    // Other HTML elements with src attribute
    let src_regex = Regex::new(r#"(?i)(?:src|xlink:href)=["']([^"']+)["']"#).unwrap();
    for cap in src_regex.captures_iter(field_content) {
        if let Some(filename) = cap.get(1) {
            let src = filename.as_str();
            if !src.starts_with("http://")
                && !src.starts_with("https://")
                && !src.starts_with("data:")
            {
                references.insert(src.to_string());
            }
        }
    }

    // LaTeX image references
    let latex_regex = Regex::new(r"latex-image-\w+\.png").unwrap();
    for cap in latex_regex.captures_iter(field_content) {
        references.insert(cap.get(0).unwrap().as_str().to_string());
    }

    references
}

/// Get all fields of a note
pub async fn get_note_fields(client: &SharedConn, note_id: i64) -> Result<Vec<String>, PgError> {
    let rows = client
        .query("SELECT content FROM fields WHERE note = $1", &[&note_id])
        .await?;

    let fields = rows.iter().map(|row| row.get::<_, String>(0)).collect();

    Ok(fields)
}

/// Get all media references currently saved for a note
pub async fn get_existing_references(
    client: &SharedConn,
    note_id: i64,
) -> Result<HashSet<String>, PgError> {
    let rows = client
        .query(
            "SELECT file_name FROM media_references WHERE note_id = $1",
            &[&note_id],
        )
        .await?;

    let refs = rows.iter().map(|row| row.get::<_, String>(0)).collect();

    Ok(refs)
}

/// Update media references for a single note
pub async fn update_media_references_for_note(
    client: &mut SharedConn,
    note_id: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    // Get all field content for the note
    let fields = get_note_fields(client, note_id).await?;

    // Extract all media references from fields
    let mut all_references = HashSet::new();
    for field in &fields {
        let refs = extract_media_references(field);
        all_references.extend(refs);
    }

    // Get existing references
    let existing_refs = get_existing_references(client, note_id).await?;

    // Calculate differences
    let to_add: HashSet<_> = all_references.difference(&existing_refs).cloned().collect();
    let to_remove: HashSet<_> = existing_refs.difference(&all_references).cloned().collect();

    if to_add.is_empty() && to_remove.is_empty() {
        return Ok(());
    }
    let tx = client.transaction().await?;

    // Remove old references
    for filename in &to_remove {
        tx.execute(
            "DELETE FROM media_references 
            WHERE note_id = $1 AND file_name = $2",
            &[&note_id, &filename],
        )
        .await?;
    }

    tx.commit().await?;

    Ok(())
}

pub async fn update_media_references_note_state(
    state: &Arc<AppState>,
    note_id: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut client: SharedConn = match state.db_pool.get_owned().await {
        Ok(pool) => pool,
        Err(err) => {
            println!("Error getting pool: {err}");
            return Err("Internal Error".into());
        }
    };
    update_media_references_for_note(&mut client, note_id).await?;
    Ok(())
}

/// Update media references for an approved note
pub async fn update_media_references_for_approved_note(
    state: &Arc<AppState>,
    note_id: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut client: SharedConn = match state.db_pool.get_owned().await {
        Ok(pool) => pool,
        Err(err) => {
            println!("Error getting pool: {err}");
            return Err("Internal Error".into());
        }
    };

    update_media_references_for_note(&mut client, note_id).await?;
    Ok(())
}

/// Clean up media references for a denied note. deleting notes completely should be handled by postgres itself
pub async fn cleanup_media_for_denied_note(
    state: &Arc<AppState>,
    note_id: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut client = database::client(state).await?;

    // Remove all references for this note
    let tx = client.transaction().await?;

    let _ = tx
        .execute(
            "DELETE FROM media_references WHERE note_id = $1",
            &[&note_id],
        )
        .await?;

    tx.commit().await?;

    Ok(())
}

/// Update media references for all notes affected by a commit
pub async fn update_media_references_for_commit(
    state: &Arc<AppState>,
    affected_notes: &Vec<i64>,
) -> Result<(), Box<dyn std::error::Error>> {
    if affected_notes.is_empty() {
        return Ok(());
    }

    let mut client: SharedConn = match state.db_pool.get_owned().await {
        Ok(pool) => pool,
        Err(err) => {
            println!("Error getting pool: {err}");
            return Err("Internal Error".into());
        }
    };

    for note_id in affected_notes {
        // Slightly less optimal, bc we could theoretically skip notes that only had their tags affected, but oh well
        update_media_references_for_note(&mut client, *note_id).await?;
    }

    Ok(())
}

pub async fn get_presigned_url(
    state: &Arc<AppState>,
    filename: &str,
    note_id: i64,
    user_id: i32,
) -> Result<String, Box<dyn std::error::Error>> {
    let client: SharedConn = match state.db_pool.get_owned().await {
        Ok(pool) => pool,
        Err(err) => {
            println!("Error getting pool: {err}");
            return Err("Internal Error".into());
        }
    };

    let clean_filename = crate::cleanser::clean(filename);
    
    // Query to get hash and deck_hash in one go
    let query = "
        SELECT mf.hash, d.human_hash 
        FROM media_files mf
        JOIN media_references mr ON mr.media_id = mf.id
        JOIN notes n ON n.id = mr.note_id
        JOIN decks d ON d.id = n.deck
        WHERE mr.file_name = $1 AND mr.note_id = $2
    ";
    
    let row = client
        .query_one(query, &[&clean_filename, &note_id])
        .await?;

    let hash: String = row.get(0);
    let deck_hash: String = row.get(1);

    // Generate download token
    let token_params = DownloadTokenParams {
        hash: hash.clone(),
        user_id,
        deck_hash,
        filename: Some(clean_filename),
    };

    let token = state
        .media_token_service
        .generate_download_token(token_params)
        .map_err(|err| format!("Failed to generate download token: {err}"))?;

    // Get media proxy URL from environment
    let media_proxy_url = std::env::var("MEDIA_PROXY_URL")
        .unwrap_or_else(|_| "https://media.ankicollab.com".to_string());

    // Construct proxy URL
    let proxy_url = format!("{}/v1/media/{}?token={}", media_proxy_url, hash, token);

    Ok(proxy_url)
}