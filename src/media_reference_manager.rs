use crate::{database, AppState};
use bb8_postgres::bb8::PooledConnection;
use bb8_postgres::PostgresConnectionManager;
use regex::Regex;
use std::collections::HashSet;
use tokio_postgres::Error as PgError;

use crate::media_tokens::DownloadTokenParams;

type SharedConn = PooledConnection<'static, PostgresConnectionManager<tokio_postgres::NoTls>>;

const ALLOWED_NOTETYPE_EXTENSIONS: [&str; 17] = [
    "jpg", "jpeg", "png", "gif", "webp", "svg", "bmp", "tif", "tiff", "mp3", "ogg", "js", "css",
    "ttf", "otf", "woff", "woff2",
];

static UNDERSCORED_CSS_IMPORTS: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(
        r#"(?xi)(?:@import\s+"(_[^"]*\.css)"|'(_[^']*\.css)')|(?:url\(\s*"(_[^"]+)"|'(_[^']+)'|(_.+?)\s*\))"#,
    )
    .expect("invalid UNDERSCORED_CSS_IMPORTS regex")
});

static UNDERSCORED_REFERENCES: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r#"(?x)\[sound:(_[^]]+)\]|"(_[^"]+)"|'(_[^']+)'|\b(?:src|data)=(_[^ >]+)"#)
        .expect("invalid UNDERSCORED_REFERENCES regex")
});

/// Validate candidate filename against notetype static media rules.
/// Must start with '_', length 4..=255, no path traversal or trailing dot/space,
/// allowed characters matching backend `check_filename_validity`, and allowed extension.
#[must_use]
pub fn is_allowed_notetype_filename(filename: &str) -> bool {
    if !filename.starts_with('_') {
        return false;
    }

    if !(4..=255).contains(&filename.len()) {
        return false;
    }

    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return false;
    }

    if filename.ends_with('.') || filename.ends_with(' ') {
        return false;
    }

    for c in filename.chars() {
        if c.is_control() {
            return false;
        }
        if !(c.is_alphanumeric()
            || c == '.'
            || c == '-'
            || c == '_'
            || c == ' '
            || c == '('
            || c == ')'
            || c == '+'
            || c == ','
            || c == '%'
            || c == '&')
        {
            return false;
        }
    }

    if !filename.chars().any(char::is_alphanumeric) {
        return false;
    }

    let path = std::path::Path::new(filename);
    let ext = match path.extension().and_then(|s| s.to_str()) {
        Some(ext) => ext.to_lowercase(),
        None => return false,
    };

    ALLOWED_NOTETYPE_EXTENSIONS.contains(&ext.as_str())
}

/// Extract static media references (`_*`) from notetype styling and card templates.
/// Matches Anki's exact regex logic, filtering extracted candidates through
/// `is_allowed_notetype_filename`.
#[must_use]
pub fn extract_static_media_references(
    css: &str,
    templates: &[(String, String)],
) -> HashSet<String> {
    let mut references = HashSet::new();

    let mut scan_text = |text: &str| {
        for cap in UNDERSCORED_CSS_IMPORTS.captures_iter(text) {
            if let Some(matched) = cap.iter().skip(1).find_map(|g| g).map(|m| m.as_str()) {
                let trimmed = matched.trim();
                if is_allowed_notetype_filename(trimmed) {
                    references.insert(trimmed.to_string());
                }
            }
        }
        for cap in UNDERSCORED_REFERENCES.captures_iter(text) {
            if let Some(matched) = cap.iter().skip(1).find_map(|g| g).map(|m| m.as_str()) {
                let trimmed = matched.trim();
                if is_allowed_notetype_filename(trimmed) {
                    references.insert(trimmed.to_string());
                }
            }
        }
    };

    scan_text(css);
    for (qfmt, afmt) in templates {
        scan_text(qfmt);
        scan_text(afmt);
    }

    references
}

/// Update static media references for a notetype within an existing transaction.
/// Queries the updated `css` and `notetype_template (qfmt, afmt)`, computes current
/// static references, diffs against existing `notetype_media_references`, and deletes
/// removed references.
pub async fn update_media_references_for_notetype_tx(
    tx: &tokio_postgres::Transaction<'_>,
    notetype_id: i64,
) -> Result<(), PgError> {
    let notetype_row = tx
        .query_opt("SELECT css FROM notetype WHERE id = $1", &[&notetype_id])
        .await?;

    let css = match notetype_row {
        Some(row) => row.get::<_, Option<String>>(0).unwrap_or_default(),
        None => return Ok(()),
    };

    let template_rows = tx
        .query(
            "SELECT qfmt, afmt FROM notetype_template WHERE notetype = $1",
            &[&notetype_id],
        )
        .await?;

    let templates: Vec<(String, String)> = template_rows
        .into_iter()
        .map(|row| {
            let qfmt: String = row.get::<_, Option<String>>(0).unwrap_or_default();
            let afmt: String = row.get::<_, Option<String>>(1).unwrap_or_default();
            (qfmt, afmt)
        })
        .collect();

    let current_refs = extract_static_media_references(&css, &templates);

    let existing_rows = tx
        .query(
            "SELECT file_name FROM notetype_media_references WHERE notetype_id = $1",
            &[&notetype_id],
        )
        .await?;

    let existing_refs: HashSet<String> = existing_rows
        .into_iter()
        .map(|row| row.get::<_, String>(0))
        .collect();

    let to_remove: HashSet<_> = existing_refs.difference(&current_refs).collect();

    for filename in to_remove {
        tx.execute(
            "DELETE FROM notetype_media_references WHERE notetype_id = $1 AND file_name = $2",
            &[&notetype_id, filename],
        )
        .await?;
    }

    Ok(())
}

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
    state: &AppState,
    note_id: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut client: SharedConn = match state.db_pool.get_owned().await {
        Ok(pool) => pool,
        Err(err) => {
            tracing::error!(error = %err, note_id = note_id, "Failed to get database pool for media reference update");
            return Err("Internal Error".into());
        }
    };
    update_media_references_for_note(&mut client, note_id).await?;
    Ok(())
}

/// Update media references for an approved note
pub async fn update_media_references_for_approved_note(
    state: &AppState,
    note_id: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut client: SharedConn = match state.db_pool.get_owned().await {
        Ok(pool) => pool,
        Err(err) => {
            tracing::error!(error = %err, note_id = note_id, "Failed to get database pool for approved note");
            return Err("Internal Error".into());
        }
    };

    update_media_references_for_note(&mut client, note_id).await?;
    Ok(())
}

/// Clean up media references for a denied note. deleting notes completely should be handled by postgres itself
pub async fn cleanup_media_for_denied_note(
    state: &AppState,
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
    state: &AppState,
    affected_notes: &Vec<i64>,
) -> Result<(), Box<dyn std::error::Error>> {
    if affected_notes.is_empty() {
        return Ok(());
    }

    let mut client: SharedConn = match state.db_pool.get_owned().await {
        Ok(pool) => pool,
        Err(err) => {
            tracing::error!(error = %err, "Failed to get database pool for commit media references");
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
    state: &AppState,
    filename: &str,
    note_id: i64,
    user_id: i32,
) -> Result<String, Box<dyn std::error::Error>> {
    let client: SharedConn = match state.db_pool.get_owned().await {
        Ok(pool) => pool,
        Err(err) => {
            tracing::error!(error = %err, note_id = note_id, "Failed to get database pool for presigned URL");
            return Err("Internal Error".into());
        }
    };

    let clean_filename = crate::cleanser::clean(filename);

    // Skip svg files since we don't sanitize them atm
    // if clean_filename.to_lowercase().ends_with(".svg") {
    //     return Err("SVG files are not supported".into());
    // }

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
    let proxy_url = format!("{media_proxy_url}/v1/media/{hash}?token={token}");

    Ok(proxy_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_allowed_notetype_filename_valid_extensions() {
        let extensions = [
            "jpg", "jpeg", "png", "gif", "webp", "svg", "bmp", "tif", "tiff", "mp3", "ogg", "js",
            "css", "ttf", "otf", "woff", "woff2",
        ];

        for ext in extensions {
            let name = format!("_test.{ext}");
            assert!(
                is_allowed_notetype_filename(&name),
                "Expected {name} to be allowed"
            );

            let upper_name = format!("_TEST.{}", ext.to_uppercase());
            assert!(
                is_allowed_notetype_filename(&upper_name),
                "Expected {upper_name} to be allowed"
            );
        }

        // Test valid characters: - _ ( ) + , % & and spaces
        assert!(is_allowed_notetype_filename(
            "_my-font (v1)+extra,alt%20&new.woff2"
        ));
    }

    #[test]
    fn test_is_allowed_notetype_filename_rejections() {
        // Must start with '_'
        assert!(!is_allowed_notetype_filename("normal.png"));
        assert!(!is_allowed_notetype_filename("image.jpg"));

        // No extension / non-filename candidates
        assert!(!is_allowed_notetype_filename("_blank"));
        assert!(!is_allowed_notetype_filename("_self"));
        assert!(!is_allowed_notetype_filename("_top"));
        assert!(!is_allowed_notetype_filename("_parent"));

        // Disallowed extensions
        assert!(!is_allowed_notetype_filename("_script.exe"));
        assert!(!is_allowed_notetype_filename("_doc.pdf"));
        assert!(!is_allowed_notetype_filename("_page.html"));
        assert!(!is_allowed_notetype_filename("_archive.zip"));

        // Length limits
        assert!(!is_allowed_notetype_filename(""));
        assert!(!is_allowed_notetype_filename("_"));
        assert!(!is_allowed_notetype_filename("_a."));
        let too_long = format!("_{}.png", "a".repeat(255));
        assert!(!is_allowed_notetype_filename(&too_long));

        // Path traversal
        assert!(!is_allowed_notetype_filename("_dir/file.png"));
        assert!(!is_allowed_notetype_filename(r"_dir\file.png"));
        assert!(!is_allowed_notetype_filename("_../file.png"));
        assert!(!is_allowed_notetype_filename("_a..b.png"));

        // Trailing dot or space
        assert!(!is_allowed_notetype_filename("_file.png."));
        assert!(!is_allowed_notetype_filename("_file.png "));

        // Disallowed characters
        assert!(!is_allowed_notetype_filename("_file?.png"));
        assert!(!is_allowed_notetype_filename("_file*.png"));
        assert!(!is_allowed_notetype_filename("_file<>.png"));

        // No alphanumeric
        assert!(!is_allowed_notetype_filename("_..."));
    }

    #[test]
    fn test_extract_static_media_css_imports() {
        let css = r#"
            @import "_main.css";
            @import '_theme.css';
            @import "normal.css";
            @import "_disallowed.exe";
        "#;

        let refs = extract_static_media_references(css, &[]);
        assert!(refs.contains("_main.css"));
        assert!(refs.contains("_theme.css"));
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn test_extract_static_media_css_urls() {
        let css = r#"
            body {
                background: url("_bg.png");
                background-image: url('_bg2.webp');
                border-image: url(_bg3.jpg);
                cursor: url("normal.png");
                list-style: url("https://example.com/_remote.png");
            }
            @font-face {
                font-family: "MyFont";
                src: url("_font.woff2") format("woff2"),
                     url('_font.ttf') format("truetype"),
                     url(_font.otf) format("opentype");
            }
        "#;

        let refs = extract_static_media_references(css, &[]);
        assert!(refs.contains("_bg.png"));
        assert!(refs.contains("_bg2.webp"));
        assert!(refs.contains("_bg3.jpg"));
        assert!(refs.contains("_font.woff2"));
        assert!(refs.contains("_font.ttf"));
        assert!(refs.contains("_font.otf"));
        assert_eq!(refs.len(), 6);
    }

    #[test]
    fn test_extract_static_media_html_templates() {
        let qfmt = r#"
            <img src=_unquoted.png>
            <img src="_quoted.png">
            <img src='_single_quoted.jpg'>
            <img src="normal.png">
            <object data=_vector.svg></object>
            <object data="_vector2.svg"></object>
            <object data='_vector3.svg'></object>
            <object data="normal.svg"></object>
        "#;

        let afmt = r#"
            <script src="_math.js"></script>
            <script>
                var helper = "_util.js";
                var style = '_custom.css';
                var target = "_blank";
                var selfTarget = '_self';
            </script>
            [sound:_audio.mp3]
            [sound:_pronounce.ogg]
            [sound:normal.mp3]
            <a href="https://example.com" target="_blank">External</a>
        "#;

        let templates = vec![(qfmt.to_string(), afmt.to_string())];
        let refs = extract_static_media_references("", &templates);

        // HTML src
        assert!(refs.contains("_unquoted.png"));
        assert!(refs.contains("_quoted.png"));
        assert!(refs.contains("_single_quoted.jpg"));

        // HTML data
        assert!(refs.contains("_vector.svg"));
        assert!(refs.contains("_vector2.svg"));
        assert!(refs.contains("_vector3.svg"));

        // Quoted script / style
        assert!(refs.contains("_math.js"));
        assert!(refs.contains("_util.js"));
        assert!(refs.contains("_custom.css"));

        // Sound references
        assert!(refs.contains("_audio.mp3"));
        assert!(refs.contains("_pronounce.ogg"));

        // Excluded non-filenames / non-underscored
        assert!(!refs.contains("normal.png"));
        assert!(!refs.contains("normal.svg"));
        assert!(!refs.contains("normal.mp3"));
        assert!(!refs.contains("_blank"));
        assert!(!refs.contains("_self"));

        assert_eq!(refs.len(), 11);
    }

    #[test]
    fn test_extract_static_media_combined() {
        let css = r#"
            @import "_base.css";
            .card { background: url("_bg.png"); }
            @font-face { src: url("_custom-font.woff2"); }
        "#;

        let templates = vec![
            (
                r#"<img src="_bg.png"><div>{{Front}}</div><script src="_card.js"></script>"#
                    .to_string(),
                r#"{{FrontSide}}<hr><div target="_blank">{{Back}}</div>[sound:_reveal.mp3]"#
                    .to_string(),
            ),
            (
                "<div>Card 2 Front</div>".to_string(),
                r#"<div>Card 2 Back</div><link rel="stylesheet" href="_extra.css">"#.to_string(),
            ),
        ];

        let refs = extract_static_media_references(css, &templates);
        let expected = HashSet::from([
            "_base.css".to_string(),
            "_bg.png".to_string(),
            "_custom-font.woff2".to_string(),
            "_card.js".to_string(),
            "_reveal.mp3".to_string(),
            "_extra.css".to_string(),
        ]);

        assert_eq!(refs, expected);
    }
}
