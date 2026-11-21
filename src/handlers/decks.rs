use std::{env, sync::Arc};

use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse, Redirect},
    Json,
};

use crate::database::{self, owned_deck_id, AppState};
use crate::error::{self, Error};
use crate::handlers::common::{check_login, error_page, SharedConn};
use crate::structs::{BasicDeckInfo, DeckOverview};
use crate::user::User;
use crate::{changelog_manager, cleanser, note_manager, notetype_manager, structs};

fn prepare_edit_deck_description(raw_desc: &str) -> (String, String) {
    (
        cleanser::clean(raw_desc),
        cleanser::extract_sanitized_css(raw_desc),
    )
}

fn build_deck_description_for_save(raw_html: &str, raw_custom_css: Option<&str>) -> String {
    match raw_custom_css {
        Some(css) => {
            let clean_html = cleanser::clean(raw_html);
            let trimmed_html = clean_html.trim();
            let clean_style = cleanser::sanitize_style_input(css);
            match (trimmed_html.is_empty(), clean_style.is_empty()) {
                (true, true) => String::new(),
                (false, true) => trimmed_html.to_string(),
                (true, false) => clean_style,
                (false, false) => format!("{trimmed_html}\n{clean_style}"),
            }
        }
        None => cleanser::clean_with_styles(raw_html),
    }
}

pub(crate) async fn edit_deck(
    State(appstate): State<Arc<AppState>>,
    user: Option<User>,
    Path(deck_hash): Path<String>,
) -> Result<impl IntoResponse, Error> {
    let user = check_login(user)?;
    let client = database::client(&appstate).await?;
    let owned_info = client
        .query(
            "Select owner, description, private, restrict_subdecks, restrict_notetypes from decks where human_hash = $1",
            &[&deck_hash],
        )
        .await
        .expect("Error preparing edit deck statement");
    if owned_info.is_empty() {
        return Ok(Html("Deck not found.".to_string()));
    }
    let owner: i32 = owned_info[0].get(0);

    let mut context = tera::Context::new();

    if owner != user.id() {
        return error_page(&appstate, &error::Error::Unauthorized.to_string());
    }

    let raw_desc: String = owned_info[0]
        .get::<_, Option<String>>(1)
        .unwrap_or_default();
    let (desc, custom_css) = prepare_edit_deck_description(&raw_desc);
    let is_private: bool = owned_info[0].get(2);
    let prevent_subdecks: bool = owned_info[0].get(3);
    let restrict_notetypes: bool = owned_info[0].get(4);

    let changelogs = changelog_manager::get_changelogs(&appstate, &deck_hash).await?;

    // Load existing base subscriptions for this deck (as subscriber)
    let subs_rows = client
        .query(
            "SELECT b.name, b.human_hash FROM deck_subscriptions ds JOIN decks s ON ds.subscriber_deck_id = s.id JOIN decks b ON ds.base_deck_id = b.id WHERE s.human_hash = $1",
            &[&deck_hash],
        )
        .await
        .unwrap_or_default();
    let mut base_links: Vec<BasicDeckInfo> = Vec::new();
    for r in subs_rows {
        base_links.push(BasicDeckInfo {
            name: r.get(0),
            human_hash: r.get(1),
        });
    }

    context.insert("user", &user);
    context.insert("hash", &deck_hash);
    context.insert("description", &desc);
    context.insert("custom_css", &custom_css);
    context.insert("private", &is_private);
    context.insert("prevent_subdecks", &prevent_subdecks);
    context.insert("restrict_notetypes", &restrict_notetypes);
    context.insert("changelogs", &changelogs);
    context.insert("base_links", &base_links);

    let rendered_template = appstate
        .tera
        .render("edit_deck.html", &context)
        .expect("Failed to render template");
    Ok(Html(rendered_template))
}

pub(crate) async fn post_edit_deck(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Json(edit_deck_data): Json<structs::EditDecksData>,
) -> Result<impl IntoResponse, Error> {
    let client = database::client(&appstate).await?;
    let data = edit_deck_data;

    owned_deck_id(&appstate, &data.hash, user.id()).await?; // only for checking if user owns the deck

    let cleaned_desc =
        build_deck_description_for_save(&data.description, data.custom_css.as_deref());
    client
        .query(
            "
        UPDATE decks 
        SET description = $1, private = $2, restrict_subdecks = $3, restrict_notetypes = $4
        WHERE human_hash = $5
        AND owner = $6",
            &[
                &cleaned_desc,
                &data.is_private,
                &data.prevent_subdecks,
                &data.restrict_notetypes,
                &data.hash,
                &user.id(),
            ],
        )
        .await?;

    if !data.changelog.is_empty() {
        changelog_manager::insert_new_changelog(&appstate, &data.hash, &data.changelog).await?;
    }

    Ok(())
}

pub(crate) async fn delete_changelog(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Path(changelog_id): Path<i64>,
) -> Result<impl IntoResponse, Error> {
    match changelog_manager::delete_changelog(&appstate, changelog_id, user.id()).await {
        Ok(hash) => Ok(Redirect::to(format!("/EditDeck/{hash}").as_str())),
        Err(_err) => Ok(Redirect::to("/")),
    }
}

pub(crate) async fn delete_deck(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Path(deck_hash): Path<String>,
) -> Result<impl IntoResponse, Error> {
    let db_state_clone = Arc::clone(&appstate);

    let client: SharedConn = match db_state_clone.db_pool.get_owned().await {
        Ok(pool) => pool,
        Err(err) => {
            tracing::error!(error = %err, "Failed to get database connection pool");
            return Err(error::Error::DatabaseConnection);
        }
    };
    let _ = owned_deck_id(&appstate, &deck_hash, user.id()).await?; // only for checking if user owns the deck

    client
        .query(
            "UPDATE decks SET deleted_at = NOW() WHERE id IN (
                WITH RECURSIVE subdecks AS (
                    SELECT id FROM decks WHERE human_hash = $1
                    UNION ALL
                    SELECT d.id FROM decks d JOIN subdecks s ON s.id = d.parent
                ) SELECT id FROM subdecks
            )",
            &[&deck_hash],
        )
        .await?;

    // Run on the Tokio runtime
    tokio::spawn(async move {
        // chunked background delete
        let client: SharedConn = match db_state_clone.db_pool.get_owned().await {
            Ok(pool) => pool,
            Err(err) => {
                tracing::error!(error = %err, "Failed to get database pool in background task");
                return;
            }
        };

        // Find all deck ids in this tree
        let rows = client
            .query(
                "WITH RECURSIVE subdecks AS (
            SELECT id, 1 as depth FROM decks WHERE human_hash = $1
            UNION ALL
            SELECT d.id, s.depth + 1 FROM decks d JOIN subdecks s ON s.id = d.parent
        ) SELECT id FROM subdecks ORDER BY depth DESC",
                &[&deck_hash],
            )
            .await;

        if let Ok(rows) = rows {
            for row in rows {
                let deck_id: i64 = row.get(0);

                // Delete the subdeck on its own, allowing CASCADE to clean up its notes,
                // commits, and other dependencies in a smaller transaction.
                let _ = client
                    .query("DELETE FROM decks WHERE id = $1", &[&deck_id])
                    .await;
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }

        // This query cleans up unused notetypes
        client
            .query(
                "DELETE FROM notetype WHERE id NOT IN (SELECT DISTINCT notetype FROM notes)",
                &[],
            )
            .await
            .unwrap();

        if let Err(err) = purge_s3_deck_assets(&db_state_clone, &deck_hash).await {
            tracing::warn!(error = %err, deck_hash = %deck_hash, "Failed to purge S3 assets for deck");
        }
    });

    Ok(Redirect::to("/"))
}

// Remove any deck-specific assets stored under the S3 prefix for this deck.
#[allow(clippy::result_large_err)]
async fn purge_s3_deck_assets(
    appstate: &Arc<AppState>,
    deck_hash: &str,
) -> Result<(), aws_sdk_s3::Error> {
    let bucket = match env::var("S3_MEDIA_BUCKET") {
        Ok(bucket) if !bucket.trim().is_empty() => bucket.trim().to_owned(),
        _ => return Ok(()),
    };

    let prefix = format!("decks/{deck_hash}/");
    let client = &appstate.s3_client;
    let mut continuation_token: Option<String> = None;

    loop {
        let mut request = client.list_objects_v2().bucket(&bucket).prefix(&prefix);

        if let Some(ref token) = continuation_token {
            request = request.continuation_token(token);
        }

        let response = request.send().await?;

        let keys: Vec<String> = response
            .contents()
            .iter()
            .filter_map(|object| object.key().map(str::to_owned))
            .collect();

        for key in keys {
            client
                .delete_object()
                .bucket(&bucket)
                .key(key)
                .send()
                .await?;
        }

        if response.is_truncated().unwrap_or(false) {
            continuation_token = response
                .next_continuation_token()
                .map(std::borrow::ToOwned::to_owned);
        } else {
            break;
        }
    }

    let marker_key = format!("decks/{deck_hash}");
    let _ = client
        .delete_object()
        .bucket(&bucket)
        .key(marker_key)
        .send()
        .await;

    Ok(())
}

pub(crate) async fn get_notes_from_deck(
    State(appstate): State<Arc<AppState>>,
    Path(deck_hash): Path<String>,
    user: Option<User>,
) -> Result<impl IntoResponse, Error> {
    let mut context = tera::Context::new();

    if user.is_none() {
        return Ok(Redirect::to("/login").into_response());
    }

    // let deck_name = decks::get_name_by_hash(&deck_hash).await;
    // if deck_name.is_err() {
    //     return Html(format!("Deck not found."))
    // }

    let notes = note_manager::retrieve_notes(&appstate, &deck_hash).await?;

    let client = database::client(&appstate).await?;
    let deck_info = client.query("Select id, name, description, human_hash, owner, TO_CHAR(last_update, 'MM/DD/YYYY') AS last_update from decks where human_hash = $1 Limit 1", &[&deck_hash]).await.expect("Error preparing deck notes statement");
    if deck_info.is_empty() {
        return error_page(&appstate, &error::Error::DeckNotFound.to_string())
            .map(axum::response::IntoResponse::into_response);
    }

    let id: i64 = deck_info[0].get(0);

    let children_rows = client
        .query(
            "Select name, human_hash from decks where parent = $1",
            &[&id],
        )
        .await
        .expect("Error getting children from decks");
    let mut childr = vec![];
    for row in children_rows {
        childr.push(BasicDeckInfo {
            name: row.get(0),
            human_hash: row.get(1),
        });
    }

    let deck = DeckOverview {
        owner: deck_info[0].get(4),
        id,
        name: deck_info[0].get(1),
        desc: cleanser::clean(
            deck_info[0]
                .get::<_, Option<String>>(2)
                .as_deref()
                .unwrap_or(""),
        ),
        hash: deck_info[0].get(3),
        last_update: deck_info[0].get(5),
        notes: "0".to_string(),
        children: childr,
        subscriptions: 0,
        stats_enabled: false, // We don't care about this here
    };

    context.insert("notes", &notes);
    context.insert("user", &user);
    context.insert("deck", &deck);

    let rendered_template = appstate
        .tera
        .render("notes.html", &context)
        .expect("Failed to render template");

    // Return the rendered HTML as the response
    Ok(Html(rendered_template).into_response())
}

pub(crate) async fn deck_overview(
    State(appstate): State<Arc<AppState>>,
    user: Option<User>,
) -> Result<impl IntoResponse, Error> {
    let mut decks: Vec<DeckOverview> = vec![];
    let user_id: i32 = user.clone().map_or(1, |u| u.id());
    let client = database::client(&appstate).await?;
    let stmt = client
        .prepare(
            "
        SELECT 
            deck_stats.id, 
            deck_stats.name, 
            deck_stats.description, 
            deck_stats.human_hash, 
            deck_stats.owner,
            TO_CHAR(deck_stats.last_update, 'MM/DD/YYYY') AS last_update,
            (SELECT COUNT(*) FROM subscriptions WHERE deck_id = deck_stats.id) AS subs,
            deck_stats.note_count
        FROM deck_stats 
        JOIN decks ON decks.id = deck_stats.id
        WHERE (deck_stats.private = false OR deck_stats.owner = $1) AND decks.deleted_at IS NULL
        ",
        )
        .await
        .expect("Error preparing decks overview statement");

    let rows = client
        .query(&stmt, &[&user_id])
        .await
        .expect("Error executing decks overview statement");

    for row in rows {
        decks.push(DeckOverview {
            owner: row.get(4),
            id: row.get(0),
            name: row.get(1),
            desc: cleanser::clean(row.get::<_, Option<String>>(2).as_deref().unwrap_or("")),
            hash: row.get(3),
            last_update: row.get(5),
            notes: row.get(7),
            children: vec![],
            subscriptions: row.get(6),
            stats_enabled: false, // We don't care about this here
        });
    }

    // decks.sort_by(|a, b| {
    //     if a.owner == user_id {
    //         return std::cmp::Ordering::Less;
    //     } else if b.owner == user_id {
    //         return std::cmp::Ordering::Greater;
    //     }

    //     b.notes.cmp(&a.notes)
    // });

    let mut context = tera::Context::new();
    context.insert("decks", &decks);
    context.insert("user", &user);
    let rendered_template = appstate
        .tera
        .render("decks.html", &context)
        .expect("Failed to render template");

    // Return the rendered HTML as the response
    Ok(Html(rendered_template))
}

pub(crate) async fn manage_decks(
    user: Option<User>,
    State(appstate): State<Arc<AppState>>,
) -> Result<impl IntoResponse, Error> {
    let user = check_login(user)?;
    let mut decks: Vec<DeckOverview> = vec![];

    let client = database::client(&appstate).await?;
    let stmt = client
        .prepare(
            "
        SELECT 
            id, 
            name, 
            description, 
            human_hash, 
            owner, 
            TO_CHAR(last_update, 'MM/DD/YYYY') AS last_update,
            (SELECT COUNT(*) FROM subscriptions WHERE deck_id = decks.id) AS subs,
            stats_enabled
        FROM decks 
        WHERE parent IS NULL and owner = $1 and deleted_at IS NULL
    ",
        )
        .await
        .expect("Error preparing decks overview statement");

    let mut context = tera::Context::new();

    let rows = client
        .query(&stmt, &[&user.id()])
        .await
        .expect("Error executing decks overview statement");

    for row in rows {
        decks.push(DeckOverview {
            owner: row.get(4),
            id: row.get(0),
            name: row.get(1),
            desc: cleanser::clean(row.get::<_, Option<String>>(2).as_deref().unwrap_or("")),
            hash: row.get(3),
            last_update: row.get(5),
            notes: note_manager::get_notes_count_in_deck(&appstate, row.get(0))
                .await
                .unwrap()
                .to_string(),
            children: vec![],
            subscriptions: row.get(6),
            stats_enabled: row.get(7),
        });
    }

    let notetypes = match notetype_manager::get_notetype_overview(&appstate, &user).await {
        Ok(cl) => cl,
        Err(error) => {
            tracing::warn!(error = %error, "Failed to get notetype overview");
            return Ok(Html("Error managing your decks.".to_string()));
        }
    };

    context.insert("decks", &decks);
    context.insert("user", &user);
    context.insert("notetypes", &notetypes);

    let rendered_template = appstate
        .tera
        .render("manage_decks.html", &context)
        .expect("Failed to render template");

    Ok(Html(rendered_template))
}
