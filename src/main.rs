#![warn(clippy::all, clippy::pedantic, clippy::nursery)]

pub mod changelog_manager;
pub mod cleanser;
pub mod commit_manager;
pub mod database;
pub mod error;
pub mod gdrive_manager;
pub mod maintainer_manager;
pub mod media_reference_manager;
pub mod media_tokens;
pub mod note_history;
pub mod note_manager;
pub mod notetype_manager;
pub mod optional_tags_manager;
pub mod stats_manager;
pub mod structs;
pub mod suggestion_manager;
pub mod user;

use crate::error::Error;
use crate::error::NoteNotFoundContext;
use database::owned_deck_id;
use database::AppState;
use net::SocketAddr;
use sync::Arc;
use tokio::signal;
use tower::ServiceBuilder;
use user::{Auth, Credentials, User};

use axum_client_ip::{ClientIp, ClientIpSource};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use axum::{
    extract::{Path, State},
    http::{header, HeaderValue},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Extension, Json, Router,
};

use structs::{
    BasicDeckInfo, DeckHash, DeckId, DeckOverview, FieldId, NoteId, Return, UpdateNotetype,
    UpdateNotetypeTemplate, UserId,
};
use structs::{
    SubscriptionPolicyGetResponse, SubscriptionPolicyItem, SubscriptionPolicyPostRequest,
};
use tera::Tera;

use aws_sdk_s3::Client as S3Client;
use std::result::Result;
use std::{
    cfg, env, eprintln, format, i32, i64, net, option_env, panic, println, str, sync, u32,
    unreachable, usize, vec,
};

type SharedConn = bb8_postgres::bb8::PooledConnection<'static, bb8_postgres::PostgresConnectionManager<tokio_postgres::NoTls>>;

fn check_login(user: Option<User>) -> Result<User, Error> {
    match user {
        Some(user) => Ok(user),
        None => Err(error::Error::Redirect("/login".to_string())),
    }
}

async fn forward_donation() -> impl IntoResponse {
    Redirect::permanent("https://ankiweb.net/shared/review/1957538407")
}

async fn get_login(State(appstate): State<Arc<AppState>>) -> Result<impl IntoResponse, Error> {
    let context = tera::Context::new();
    let rendered_template = appstate.tera.render("login.html", &context)?;
    Ok(Html(rendered_template))
}
async fn post_login(
    ClientIp(ip): ClientIp,
    Extension(auth): Extension<Arc<Auth>>,
    axum::Form(form): axum::Form<Credentials>,
) -> Result<impl IntoResponse, Error> {
    let res = auth.login(form, ip).await?;

    let mut response = axum::response::Redirect::to("/").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        header::HeaderValue::from_str(&res).unwrap(),
    );

    Ok(response)
}

async fn post_signup(
    ClientIp(ip): ClientIp,
    Extension(auth): Extension<Arc<Auth>>,
    axum::Form(form): axum::Form<Credentials>,
) -> Result<impl IntoResponse, Error> {
    auth.signup(form.clone(), ip).await?;
    // Reuse login flow to set the cookie header
    post_login(ClientIp(ip), Extension(auth), axum::Form(form)).await
}

async fn error_page(appstate: &Arc<AppState>, message: String) -> Result<Html<String>, Error> {
    let mut context = tera::Context::new();
    context.insert("message", &message);
    let rendered_template = appstate.tera.render("error.html", &context)?;
    Ok(Html(rendered_template))
}

async fn get_signup(State(appstate): State<Arc<AppState>>) -> Result<impl IntoResponse, Error> {
    let context = tera::Context::new();
    let rendered_template = appstate.tera.render("signup.html", &context)?;
    Ok(Html(rendered_template))
}

async fn index(
    State(appstate): State<Arc<AppState>>,
    user: Option<User>,
) -> Result<impl IntoResponse, Error> {
    let mut context = tera::Context::new();
    context.insert("user", &user);
    let rendered_template = appstate.tera.render("index.html", &context)?;
    Ok(Html(rendered_template))
}

async fn terms(State(appstate): State<Arc<AppState>>) -> Result<impl IntoResponse, Error> {
    let context = tera::Context::new();
    let rendered_template = appstate.tera.render("terms.html", &context)?;
    Ok(Html(rendered_template))
}

async fn privacy(State(appstate): State<Arc<AppState>>) -> Result<impl IntoResponse, Error> {
    let context = tera::Context::new();
    let rendered_template = appstate.tera.render("privacy.html", &context)?;
    Ok(Html(rendered_template))
}

async fn imprint(State(appstate): State<Arc<AppState>>) -> Result<impl IntoResponse, Error> {
    let context = tera::Context::new();
    let rendered_template = appstate.tera.render("imprint.html", &context)?;
    Ok(Html(rendered_template))
}

async fn datenschutz(State(appstate): State<Arc<AppState>>) -> Result<impl IntoResponse, Error> {
    let context = tera::Context::new();
    let rendered_template = appstate.tera.render("datenschutz.html", &context)?;
    Ok(Html(rendered_template))
}

async fn logout(Extension(auth): Extension<Arc<Auth>>) -> Result<impl IntoResponse, Error> {
    let exp_cookie = auth.logout().await;
    let mut response = axum::response::Redirect::to("/").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        header::HeaderValue::from_str(&exp_cookie).unwrap(),
    );
    // add a Clear-Site-Data header for complete cleanup
    response.headers_mut().insert(
        header::HeaderName::from_static("clear-site-data"),
        header::HeaderValue::from_static("\"cookies\""),
    );

    Ok(response)
}

async fn render_optional_tags(
    appstate: &Arc<AppState>,
    deck_hash: &String,
    deck_id: i64,
    user: User,
) -> Result<impl IntoResponse, Error> {
    // Get Tags by deck id
    let tags = match optional_tags_manager::get_tags(appstate, deck_id).await {
        Ok(tags) => tags,
        Err(e) => {
            println!("Error retrieving opt tags: {e}");
            return Ok(Html(
                "Error retrieving optional tags. Please notify us.".to_string(),
            ));
        }
    };

    let mut context = tera::Context::new();
    context.insert("optional_tags", &tags);
    context.insert("hash", &deck_hash);
    context.insert("user", &user);

    let rendered_template = appstate
        .tera
        .render("optional_tags.html", &context)
        .expect("Failed to render template");
    Ok(Html(rendered_template))
}

async fn post_optional_tags(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Json(edit_optional_tag): Json<structs::UpdateOptionalTag>,
) -> Result<impl IntoResponse, Error> {
    let data = edit_optional_tag;

    let deck_id: i64 = owned_deck_id(&appstate, &data.deck, user.id()).await?;

    // Add new tag
    if data.action == 1 {
        optional_tags_manager::add_tag(&appstate, deck_id, cleanser::clean(&data.taggroup)).await
    } else {
        // Delete existing optional_tag
        optional_tags_manager::remove_tag(&appstate, deck_id, cleanser::clean(&data.taggroup)).await
    }
}

async fn show_optional_tags(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Path(deck_hash): Path<DeckHash>,
) -> Result<impl IntoResponse, Error> {
    let deck_id: i64 = owned_deck_id(&appstate, &deck_hash, user.id()).await?;

    Ok(render_optional_tags(&appstate, &deck_hash, deck_id, user).await)
}

async fn render_maintainers(
    appstate: &Arc<AppState>,
    deck_hash: &String,
    deck_id: i64,
    user: User,
) -> impl IntoResponse {
    // Get Maintainers by deck id
    let maintainers = match maintainer_manager::get_maintainers(appstate, deck_id).await {
        Ok(maintainers) => maintainers,
        Err(e) => {
            println!("Error getting maintainers: {e}");
            return Html("Error getting maintainers.".to_string());
        }
    };

    let mut context = tera::Context::new();
    context.insert("maintainers", &maintainers);
    context.insert("hash", &deck_hash);
    context.insert("user", &user);

    let rendered_template = appstate
        .tera
        .render("maintainers.html", &context)
        .expect("Failed to render template");
    Html(rendered_template)
}

async fn post_maintainers(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Json(edit_maintainer): Json<structs::UpdateMaintainer>,
) -> Result<impl IntoResponse, Error> {
    let data = edit_maintainer;

    let deck_id: i64 = owned_deck_id(&appstate, &data.deck, user.id()).await?;

    // Add new maintainer
    if data.action == 1 {
        maintainer_manager::add_maintainer(&appstate, deck_id, data.username).await
    } else {
        // Delete existing maintainer
        maintainer_manager::remove_maintainer(&appstate, deck_id, data.username).await
    }
}

// async fn post_media_manager(
//     State(appstate): State<Arc<AppState>>,
//     user: User,
//     Json(update_media): Json<structs::GDriveInfo>
// ) -> Result<impl IntoResponse, Error> {
//     let data = update_media;

//     let deck_id: i64 = owned_deck_id(&appstate, &data.deck, user.id()).await?;

//     gdrive_manager::update_media(&appstate, deck_id, data).await
// }

// async fn media_manager(
//     State(appstate): State<Arc<AppState>>,
//     user: User,
//     Path(deck_hash): Path<String>,
// ) -> Result<impl IntoResponse, Error> {
//     let _ = owned_deck_id(&appstate, &deck_hash, user.id()).await?;
//     let mut context = tera::Context::new();
//     context.insert("hash", &deck_hash);
//     context.insert("user", &user);

//     let rendered_template = appstate.tera
//         .render("media_manager.html", &context)
//         .expect("Failed to render template");
//     Ok(Html(rendered_template))
// }

async fn show_maintainers(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Path(deck_hash): Path<String>,
) -> Result<impl IntoResponse, Error> {
    let deck_id: i64 = owned_deck_id(&appstate, &deck_hash, user.id()).await?;

    Ok(render_maintainers(&appstate, &deck_hash, deck_id, user).await)
}

async fn edit_notetype(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Path(notetype_id): Path<i64>,
) -> Result<impl IntoResponse, Error> {
    let client = database::client(&appstate).await?;

    let owned_info = client
        .query(
            "SELECT 1 FROM notetype WHERE (owner = $1 AND id = $3) OR $2 LIMIT 1",
            &[&user.id(), &user.is_admin, &notetype_id],
        )
        .await
        .expect("Error preparing edit notetype statement");
    if owned_info.is_empty() {
        return error_page(&appstate, error::Error::Unauthorized.to_string()).await;
    }

    let notetype_info = client
        .query(
            "Select name, css from notetype where id = $1",
            &[&notetype_id],
        )
        .await
        .expect("Error preparing edit notetype statement");
    let notetype_template_info = client
        .query(
            "Select id, qfmt, afmt, name from notetype_template where notetype = $1",
            &[&notetype_id],
        )
        .await
        .expect("Error preparing edit notetype statement");

    let protected_fields = notetype_manager::get_protected_fields(&appstate, notetype_id).await?;

    let name: String = notetype_info[0].get(0);
    let styling: String = notetype_info[0].get(1);

    let mut templates: Vec<UpdateNotetypeTemplate> = Vec::new();
    for row in notetype_template_info {
        templates.push(UpdateNotetypeTemplate {
            front: row.get(1),
            back: row.get(2),
            template_id: row.get(0),
            name: row.get(3),
        });
    }

    let mut context = tera::Context::new();
    context.insert("name", &name);
    context.insert("styling", &styling);
    context.insert("notetype_id", &notetype_id);
    context.insert("user", &user);
    context.insert("protected_fields", &protected_fields);
    context.insert("templates", &templates);

    let rendered_template = appstate.tera.render("edit_notetype.html", &context)?;
    Ok(Html(rendered_template))
}

async fn post_edit_notetype(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Json(edit_notetype): Json<UpdateNotetype>,
) -> impl IntoResponse {
    let data = edit_notetype;

    match notetype_manager::update_notetype(&appstate, &user, &data).await {
        Ok(_res) => "updated".to_owned(),
        Err(e) => e.to_string(),
    }
}

async fn edit_deck(
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
        return error_page(&appstate, error::Error::Unauthorized.to_string()).await;
    }

    let desc: String = owned_info[0].get(1);
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

async fn post_edit_deck(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Json(edit_deck_data): Json<structs::EditDecksData>,
) -> Result<impl IntoResponse, Error> {
    let client = database::client(&appstate).await?;
    let data = edit_deck_data;

    owned_deck_id(&appstate, &data.hash, user.id()).await?; // only for checking if user owns the deck

    let cleaned_desc = cleanser::clean(&data.description);
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

async fn delete_changelog(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Path(changelog_id): Path<i64>,
) -> Result<impl IntoResponse, Error> {
    match changelog_manager::delete_changelog(&appstate, changelog_id, user.id()).await {
        Ok(hash) => Ok(Redirect::permanent(format!("/EditDeck/{hash}").as_str())),
        Err(_err) => Ok(Redirect::permanent("/")),
    }
}

async fn delete_deck(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Path(deck_hash): Path<String>,
) -> Result<impl IntoResponse, Error> {
    let db_state_clone = Arc::clone(&appstate);

    let client: SharedConn = match db_state_clone.db_pool.get_owned().await {
            Ok(pool) => pool,
            Err(err) => {
                println!("Error getting pool: {err}");
                return Ok(Redirect::permanent("/"));
            }
        };
    let _ = owned_deck_id(&appstate, &deck_hash, user.id()).await?; // only for checking if user owns the deck

    client
        .query("Select delete_deck($1)", &[&deck_hash])
        .await?;


    // Run on the Tokio runtime
    tokio::spawn(async move {
        if let Err(e) = purge_s3_deck_assets(&db_state_clone, &deck_hash).await {
            eprintln!("Error purging S3 assets for deck {deck_hash}: {e}");
        }

        let client: SharedConn = match db_state_clone.db_pool.get_owned().await {
            Ok(pool) => pool,
            Err(err) => {
                println!("Error getting pool: {err}");
                return;
            }
        };
        // This query is quite expensive, but it is only used when deleting a deck, so it should be fine. I use it to trigger a cleanup
        client
            .query(
                "DELETE FROM notetype WHERE id NOT IN (SELECT DISTINCT notetype FROM notes)",
                &[],
            )
            .await.unwrap();

        if let Err(err) = purge_s3_deck_assets(&appstate, &deck_hash).await {
            println!(
                "Failed to delete S3 assets for deck {deck_hash}: {err}",
            );
        }
    });

    Ok(Redirect::permanent("/"))
}

// Remove any deck-specific assets stored under the S3 prefix for this deck.
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
        let mut request = client
            .list_objects_v2()
            .bucket(&bucket)
            .prefix(&prefix);

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

async fn approve_commit(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Path(commit_id): Path<i32>,
) -> Result<impl IntoResponse, Error> {
    let res = suggestion_manager::merge_by_commit(&appstate, commit_id, true, user).await?;

    Ok(if res.is_none() {
        Redirect::to("/reviews")
    } else {
        Redirect::to(&format!("/commit/{}", res.unwrap()))
    })
}

async fn deny_commit(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Path(commit_id): Path<i32>,
) -> Result<impl IntoResponse, Error> {
    match suggestion_manager::merge_by_commit(&appstate, commit_id, false, user).await {
        Ok(res) => {
            if res.is_none() {
                Ok(Redirect::to("/reviews"))
            } else {
                Ok(Redirect::to(&format!("/commit/{}", res.unwrap())))
            }
        }
        Err(error) => {
            println!("Error: {error}");
            Ok(Redirect::to("/"))
        }
    }
}

async fn review_commit(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Path(commit_id): Path<i32>,
) -> Result<impl IntoResponse, Error> {
    let mut context = tera::Context::new();

    let notes = commit_manager::notes_by_commit(&appstate, commit_id).await?;

    let commit = commit_manager::get_commit_info(&appstate, commit_id).await?;

    let client = database::client(&appstate).await?;
    let q_guid = client
        .query(
            "Select deck from commits where commit_id = $1",
            &[&commit_id],
        )
        .await?;
    if q_guid.is_empty() {
        return error_page(&appstate, error::Error::CommitNotFound.to_string()).await;
    }
    let deck_id: i64 = q_guid[0].get(0);

    let access = suggestion_manager::is_authorized(&appstate, &user, deck_id).await?;
    let notemodels = notetype_manager::notetypes_by_commit(&appstate, commit_id).await?;

    context.insert("notes", &notes);
    context.insert("commit", &commit);
    context.insert("user", &user);
    context.insert("owned", &access);
    context.insert("notemodels", &notemodels);

    let rendered_template = appstate
        .tera
        .render("commit.html", &context)
        .expect("Failed to render template");

    // Return the rendered HTML as the response
    Ok(Html(rendered_template))
}

async fn review_note(
    State(appstate): State<Arc<AppState>>,
    Path(note_id): Path<i64>,
    user: Option<User>,
) -> Result<impl IntoResponse, Error> {
    let mut context = tera::Context::new();

    // Require a logged-in user for any note review view (no anonymous browsing)
    if user.is_none() {
        return Ok(Redirect::to("/login").into_response());
    }

    let note = match note_manager::get_note_data(&appstate, note_id).await {
        Ok(note) => note,
        Err(_error) => {
            return error_page(
                &appstate,
                error::Error::NoteNotFound(NoteNotFoundContext::InvalidData).to_string(),
            )
            .await
            .map(|h| h.into_response());
        }
    };

    if note.id == 0 {
        // Invalid data // No note found!
        return error_page(
            &appstate,
            error::Error::NoteNotFound(NoteNotFoundContext::InvalidData).to_string(),
        )
        .await
        .map(|h| h.into_response());
    }

    // access boolean previously used for template conditions; removed as unused

    // Safe unwrap: we early-returned if user.is_none()
    let current_user = user.as_ref().unwrap();
    let client = database::client(&appstate).await?;
    let q_guid = client
        .query("Select deck from notes where id = $1", &[&note_id])
        .await?;
    if q_guid.is_empty() {
        return error_page(
            &appstate,
            error::Error::NoteNotFound(NoteNotFoundContext::InvalidData).to_string(),
        )
        .await
        .map(|h| h.into_response());
    }
    let deck_id: i64 = q_guid[0].get(0);
    let access = suggestion_manager::is_authorized(&appstate, current_user, deck_id).await?;

    context.insert("note", &note);
    context.insert("access", &access);
    context.insert("user", &user);
    let rendered_template = appstate
        .tera
        .render("review.html", &context)
        .expect("Failed to render template");

    // Return the rendered HTML as the response
    Ok(Html(rendered_template).into_response())
}

// Fetch recent history events for a note (newest first)
async fn note_history_page(
    State(appstate): State<Arc<AppState>>,
    Path(note_id): Path<i64>,
    user: Option<User>,
) -> Result<impl IntoResponse, Error> {
    if user.is_none() {
        return Ok(Redirect::to("/login").into_response());
    }

    let client = database::client(&appstate).await?;
    let row_opt = client
        .query_opt("SELECT deck FROM notes WHERE id = $1", &[&note_id])
        .await?;
    if row_opt.is_none() {
        return error_page(
            &appstate,
            error::Error::NoteNotFound(NoteNotFoundContext::InvalidData).to_string(),
        )
        .await
        .map(|h| h.into_response());
    }
    let deck_id: i64 = row_opt.unwrap().get(0);
    let u = user.as_ref().unwrap();
    let _ = suggestion_manager::is_authorized(&appstate, u, deck_id).await?; // we still render even if not owner; access boolean not used here yet
    let history = note_history::fetch_note_history(&client, note_id).await?;
    let mut context = tera::Context::new();
    context.insert("note_id", &note_id);
    context.insert("events", &history.events);
    context.insert("groups", &history.groups);
    context.insert("actors", &history.actors);
    context.insert("user", &user);
    let rendered_template = appstate.tera.render("note_history.html", &context)?;
    Ok(Html(rendered_template).into_response())
}

// Show all notes impacted by a commit via events aggregation
async fn commit_history_page(
    State(appstate): State<Arc<AppState>>,
    Path(commit_id): Path<i32>,
    user: Option<User>,
) -> Result<impl IntoResponse, Error> {
    if user.is_none() {
        return Ok(Redirect::to("/login").into_response());
    }
    let client = database::client(&appstate).await?;
    let notes = note_history::fetch_commit_history(&client, commit_id).await?;
    let mut context = tera::Context::new();
    context.insert("commit_id", &commit_id);
    context.insert("notes", &notes);
    context.insert("user", &user);
    let rendered_template = appstate.tera.render("commit_history.html", &context)?;
    Ok(Html(rendered_template).into_response())
}

async fn access_check(appstate: &Arc<AppState>, deck_id: i64, user: &User) -> Result<bool, Error> {
    let access = match suggestion_manager::is_authorized(appstate, user, deck_id).await {
        Ok(access) => access,
        Err(_error) => return Ok(false),
    };

    if !access {
        return Ok(false);
    }

    Ok(true)
}

async fn get_deck_id(
    appstate: &Arc<AppState>,
    query: &str,
    param: &(dyn tokio_postgres::types::ToSql + Sync),
) -> Return<DeckId> {
    let client = database::client(appstate).await?;
    let q_guid = match client.query(query, &[param]).await {
        Ok(q_guid) => q_guid,
        Err(_error) => return Ok(0),
    };
    if q_guid.is_empty() {
        return Ok(0);
    }
    let deck_id: DeckId = q_guid[0].get(0);

    Ok(deck_id)
}

async fn get_deck_by_tag_id(appstate: &Arc<AppState>, tag_id: i64) -> Return<DeckId> {
    let query = "Select deck from notes where id = (select note from tags where id = $1)";
    get_deck_id(appstate, query, &tag_id).await
}

async fn get_deck_by_field_id(appstate: &Arc<AppState>, field_id: FieldId) -> Return<DeckId> {
    let query = "Select deck from notes where id = (select note from fields where id = $1)";
    get_deck_id(appstate, query, &field_id).await
}

async fn get_deck_by_move_id(appstate: &Arc<AppState>, move_id: i32) -> Return<DeckId> {
    let query = "Select original_deck from note_move_suggestions where id = $1";
    get_deck_id(appstate, query, &move_id).await
}

async fn deny_tag(
    State(appstate): State<Arc<AppState>>,
    Path(tag_id): Path<i64>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let deck_id = match get_deck_by_tag_id(&appstate, tag_id).await {
        Ok(deck_id) => deck_id,
        Err(error) => {
            println!("Error: {error}");
            return Ok(Redirect::to("/"));
        }
    };

    if !access_check(&appstate, deck_id, &user).await? {
        return Ok(Redirect::to("/"));
    }

    let mut client = database::client(&appstate).await?; // needs mutable for transaction
    let tx = client.transaction().await?;
    match suggestion_manager::deny_tag_change(&tx, tag_id, user.id()).await {
        Ok(res) => {
            tx.commit().await?;
            Ok(Redirect::to(&format!("/review/{res}")))
        }
        Err(error) => {
            println!("Error: {error}");
            let _ = tx.rollback().await;
            Ok(Redirect::to("/"))
        }
    }
}

async fn deny_note_move(
    State(appstate): State<Arc<AppState>>,
    Path(move_id): Path<i32>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let deck_id = match get_deck_by_move_id(&appstate, move_id).await {
        Ok(deck_id) => deck_id,
        Err(error) => {
            println!("Error: {error}");
            return Ok(Redirect::to("/"));
        }
    };

    if !access_check(&appstate, deck_id, &user).await? {
        return Ok(Redirect::to("/"));
    }

    let mut client = database::client(&appstate).await?;
    let tx = client.transaction().await?;
    match suggestion_manager::deny_note_move_request(&tx, move_id, user.id()).await {
        Ok(res) => {
            tx.commit().await?;
            Ok(Redirect::to(&format!("/review/{res}")))
        }
        Err(error) => {
            println!("Error: {error}");
            let _ = tx.rollback().await;
            Ok(Redirect::to("/"))
        }
    }
}

async fn accept_note_move(
    State(appstate): State<Arc<AppState>>,
    Path(move_id): Path<i32>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let deck_id = match get_deck_by_move_id(&appstate, move_id).await {
        Ok(deck_id) => deck_id,
        Err(error) => {
            println!("Error: {error}");
            return Ok(Redirect::to("/"));
        }
    };

    if !access_check(&appstate, deck_id, &user).await? {
        return Ok(Redirect::to("/"));
    }

    let mut client = database::client(&appstate).await?;
    let tx = client.transaction().await?;
    match suggestion_manager::approve_move_note_request_by_moveid(&tx, move_id, user.id()).await {
        Ok(res) => {
            tx.commit().await?;
            Ok(Redirect::to(&format!("/review/{res}")))
        }
        Err(error) => {
            println!("Error: {error}");
            let _ = tx.rollback().await;
            Ok(Redirect::to("/"))
        }
    }
}

async fn accept_tag(
    State(appstate): State<Arc<AppState>>,
    Path(tag_id): Path<i64>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let deck_id = match get_deck_by_tag_id(&appstate, tag_id).await {
        Ok(deck_id) => deck_id,
        Err(error) => {
            println!("Error: {error}");
            return Ok(Redirect::to("/"));
        }
    };

    if !access_check(&appstate, deck_id, &user).await? {
        return Ok(Redirect::to("/"));
    }

    let mut client = database::client(&appstate).await?;
    let tx = client.transaction().await?;
    match suggestion_manager::approve_tag_change(&tx, tag_id, true, user.id()).await {
        Ok(res) => {
            tx.commit().await?;
            Ok(Redirect::to(&format!("/review/{res}")))
        }
        Err(error) => {
            println!("Error: {error}");
            let _ = tx.rollback().await;
            Ok(Redirect::to("/"))
        }
    }
}

async fn deny_field(
    State(appstate): State<Arc<AppState>>,
    Path(field_id): Path<i64>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let deck_id = match get_deck_by_field_id(&appstate, field_id).await {
        Ok(deck_id) => deck_id,
        Err(error) => {
            println!("Error: {error}");
            return Ok(Redirect::to("/"));
        }
    };

    if !access_check(&appstate, deck_id, &user).await? {
        return Ok(Redirect::to("/"));
    }

    let mut client = database::client(&appstate).await?;
    let tx = client.transaction().await?;
    match suggestion_manager::deny_field_change(&tx, field_id, user.id()).await {
        Ok(res) => {
            tx.commit().await?;
            Ok(Redirect::to(&format!("/review/{res}")))
        }
        Err(error) => {
            println!("Error: {error}");
            let _ = tx.rollback().await;
            Ok(Redirect::to("/"))
        }
    }
}

async fn accept_field(
    State(appstate): State<Arc<AppState>>,
    Path(field_id): Path<i64>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let deck_id = match get_deck_by_field_id(&appstate, field_id).await {
        Ok(deck_id) => deck_id,
        Err(error) => {
            println!("Error: {error}");
            return Ok(Redirect::to("/"));
        }
    };

    if !access_check(&appstate, deck_id, &user).await? {
        return Ok(Redirect::to("/"));
    }

    let mut client = database::client(&appstate).await?;
    let tx = client.transaction().await?;
    match suggestion_manager::approve_field_change(&tx, field_id, true, user.id()).await {
        Ok(res) => {
            tx.commit().await?;
            // Best-effort post-commit media reference refresh for this note
            if let Ok(nid) = res.parse::<i64>() {
                let state_clone = appstate.clone();
                tokio::spawn(async move {
                    if let Err(e) =
                        media_reference_manager::update_media_references_for_approved_note(
                            &state_clone,
                            nid,
                        )
                        .await
                    {
                        println!("Error updating media references: {e:?}");
                    }
                });
            }
            Ok(Redirect::to(&format!("/review/{res}")))
        }
        Err(error) => {
            println!("Error: {error}");
            let _ = tx.rollback().await;
            Ok(Redirect::to("/"))
        }
    }
}

async fn update_field(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Json(edit_optional_tag): Json<structs::UpdateFieldSuggestion>,
) -> Result<impl IntoResponse, Error> {
    let data = edit_optional_tag;
    let deck_id = match get_deck_by_field_id(&appstate, data.field_id).await {
        Ok(deck_id) => deck_id,
        Err(error) => {
            println!("Error: {error}");
            return Ok(String::new());
        }
    };

    if !access_check(&appstate, deck_id, &user).await? {
        return Ok(String::new());
    }

    let mut client = database::client(&appstate).await?;
    let tx = client.transaction().await?;
    match suggestion_manager::update_field_suggestion(&tx, data.field_id, &data.content).await {
        Ok(_res) => {
            tx.commit().await?;
            match commit_manager::get_field_diff(&appstate, data.field_id).await {
                Ok(diff) => Ok(diff),
                Err(error) => {
                    println!("Error: {error}");
                    Ok(String::new())
                }
            }
        }
        Err(error) => {
            println!("Error: {error}");
            let _ = tx.rollback().await;
            Ok(String::new())
        }
    }
}

async fn accept_note(
    State(appstate): State<Arc<AppState>>,
    Path(note_id): Path<i64>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let mut client = database::client(&appstate).await?;
    let tx = client.transaction().await?;
    match suggestion_manager::approve_card(&tx, &appstate, note_id, &user, false).await {
        Ok(res) => {
            tx.commit().await?;
            // Update media references post-commit for the approved note
            if let Ok(nid) = res.parse::<i64>() {
                let state_clone = appstate.clone();
                tokio::spawn(async move {
                    if let Err(e) =
                        media_reference_manager::update_media_references_for_approved_note(
                            &state_clone,
                            nid,
                        )
                        .await
                    {
                        println!("Error updating media references: {e:?}");
                    }
                });
            }
            Ok(Redirect::to(&format!("/review/{res}")))
        }
        Err(error) => {
            println!("Error: {error}");
            let _ = tx.rollback().await;
            Ok(Redirect::to("/"))
        }
    }
}

// This actually removes the note from the database (Only used for notes that are not approved yet)
async fn deny_note(
    State(appstate): State<Arc<AppState>>,
    Path(note_id): Path<i64>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    match suggestion_manager::delete_card(&appstate, note_id, user).await {
        Ok(res) => Ok(Redirect::to(&format!("/notes/{res}"))),
        Err(error) => {
            println!("Error: {error}");
            Ok(Redirect::to("/"))
        }
    }
}

// This marks the note as deleted, but does not remove them (Used for existing notes that are approved)
async fn remove_note_from_deck(
    State(appstate): State<Arc<AppState>>,
    Path(note_id): Path<i64>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let mut client = database::client(&appstate).await?;
    let tx = client.transaction().await?;
    match note_manager::mark_note_deleted(&tx, &appstate, note_id, user, false).await {
        Ok(res) => {
            tx.commit().await?;
            // Post-commit cleanup of media references for denied note
            if let Ok(nid) = res.parse::<i64>() {
                let state_clone = appstate.clone();
                tokio::spawn(async move {
                    if let Err(e) =
                        media_reference_manager::cleanup_media_for_denied_note(&state_clone, nid)
                            .await
                    {
                        println!("Error updating media references: {e:?}");
                    }
                });
            }
            Ok(Redirect::to(&format!("/notes/{res}")))
        }
        Err(error) => {
            println!("Error: {error}");
            let _ = tx.rollback().await;
            Ok(Redirect::to("/"))
        }
    }
}

async fn deny_note_removal(
    State(appstate): State<Arc<AppState>>,
    Path(note_id): Path<i64>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    match note_manager::deny_note_removal_request(&appstate, note_id, user).await {
        Ok(res) => Ok(Redirect::to(&format!("/review/{res}"))),
        Err(error) => {
            println!("Error: {error}");
            Ok(Redirect::to("/"))
        }
    }
}

use once_cell::sync::Lazy;

static STATS_CACHE_KEY: Lazy<String> =
    Lazy::new(|| std::env::var("STATS_CACHE_KEY").expect("STATS_CACHE_KEY must be set"));

async fn refresh_stats_cache(
    State(appstate): State<Arc<AppState>>,
    Path(secret): Path<String>,
) -> Result<impl IntoResponse, Error> {
    if secret != *STATS_CACHE_KEY {
        return Ok(Redirect::to("/"));
    }
    let db_state_clone = Arc::clone(&appstate);
    tokio::spawn(async move {
        stats_manager::update_stats(&db_state_clone).await.unwrap();
    });
    Ok(Redirect::to("/"))
}

async fn toggle_stats(
    State(appstate): State<Arc<AppState>>,
    Path(deck_hash): Path<String>,
    user: User,
) -> Result<impl IntoResponse, Error> {
    let client = database::client(&appstate).await?;
    let owned_info = client
        .query(
            "Select owner from decks where human_hash = $1",
            &[&deck_hash],
        )
        .await
        .expect("Error preparing edit deck statement");
    if owned_info.is_empty() {
        return Ok(Redirect::to("/"));
    }
    let owner: i32 = owned_info[0].get(0);

    if owner != user.id() {
        return Ok(Redirect::to("/"));
    }

    let deck_id = owned_deck_id(&appstate, &deck_hash, user.id()).await?;

    stats_manager::toggle_stats(&appstate, deck_id)
        .await
        .unwrap();

    Ok(Redirect::to("/ManageDecks"))
}

async fn show_statistics(
    State(appstate): State<Arc<AppState>>,
    Path(deck_hash): Path<String>,
    user: Option<User>,
) -> Result<impl IntoResponse, Error> {
    let user = check_login(user)?;
    let client = database::client(&appstate).await?;
    let owned_info = client
        .query("Select id from decks where human_hash = $1", &[&deck_hash])
        .await
        .expect("Error preparing edit deck statement");
    if owned_info.is_empty() {
        return Ok(Html("Deck not found.".to_string()));
    }
    let deck_id: i64 = owned_info[0].get(0);

    if !access_check(&appstate, deck_id, &user).await? {
        return Ok(Html("Unauthorized.".to_string()));
    }

    let mut context = tera::Context::new();

    let deck_base_info = match stats_manager::get_base_deck_info(&appstate, &deck_hash).await {
        Ok(deck_base_info) => deck_base_info,
        Err(error) => {
            println!("Error get_base_deck_info: {error}");
            return Ok(Html("Error showing the statistics.".to_string()));
        }
    };

    if deck_base_info.note_count == 0 {
        let rendered_template = appstate
            .tera
            .render("empty_stats.html", &context)
            .expect("Failed to render template");
        return Ok(Html(rendered_template));
    }

    let deck_info = match stats_manager::get_deck_stat_info(&appstate, &deck_hash).await {
        Ok(deck_info) => deck_info,
        Err(error) => {
            println!("Error get_deck_stat_info: {error}");
            return Ok(Html("Error showing the statistics.".to_string()));
        }
    };

    let notes_info = match stats_manager::get_worst_notes_info(&appstate, &deck_hash).await {
        Ok(notes_info) => notes_info,
        Err(error) => {
            println!("Error get_worst_notes_info: {error}");
            return Ok(Html("Error showing the statistics.".to_string()));
        }
    };

    context.insert("decks", &deck_info);
    context.insert("notes", &notes_info);
    context.insert("base", &deck_base_info);

    let rendered_template = appstate
        .tera
        .render("statistics.html", &context)
        .expect("Failed to render template");
    Ok(Html(rendered_template))
}

async fn get_notes_from_deck(
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
        return error_page(&appstate, error::Error::DeckNotFound.to_string())
            .await
            .map(|h| h.into_response());
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
        desc: cleanser::clean(deck_info[0].get(2)),
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

async fn all_reviews(
    State(appstate): State<Arc<AppState>>,
    user: Option<User>,
) -> Result<impl IntoResponse, Error> {
    let user = check_login(user)?;
    let mut context = tera::Context::new();

    let commits = match commit_manager::commits_review(&appstate, user.id()).await {
        Ok(commits) => commits,
        Err(error) => {
            println!("Error commits_review: {error}");
            return Ok(Html("Error getting the reviews.".to_string()));
        }
    };

    context.insert("commits", &commits);
    //context.insert("notes", &notes);
    context.insert("user", &user);

    let rendered_template = appstate
        .tera
        .render("reviews.html", &context)
        .expect("Failed to render template");
    Ok(Html(rendered_template))
}

async fn deck_overview(
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
            id, 
            name, 
            description, 
            human_hash, 
            owner,
            TO_CHAR(last_update, 'MM/DD/YYYY') AS last_update,
            (SELECT COUNT(*) FROM subscriptions WHERE deck_id = deck_stats.id) AS subs,
            note_count
        FROM deck_stats 
        WHERE private = false OR owner = $1
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
            desc: cleanser::clean(row.get(2)),
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

// Subscription policy helpers
async fn resolve_deck_id_by_hash(appstate: &Arc<AppState>, hash: &str) -> Return<i64> {
    let client = database::client(appstate).await?;
    let rows = client
        .query("SELECT id FROM decks WHERE human_hash = $1", &[&hash])
        .await?;
    if rows.is_empty() {
        return Ok(0);
    }
    Ok(rows[0].get(0))
}

async fn api_get_subscription_policy(
    State(appstate): State<Arc<AppState>>,
    user: User,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<impl IntoResponse, Error> {
    let sub_hash = params
        .get("subscriber_deck_hash")
        .cloned()
        .unwrap_or_default();
    let base_hash = params.get("base_deck_hash").cloned().unwrap_or_default();
    let sub_id = resolve_deck_id_by_hash(&appstate, &sub_hash).await?;
    let base_id = resolve_deck_id_by_hash(&appstate, &base_hash).await?;
    if sub_id == 0 || base_id == 0 {
        return Ok((axum::http::StatusCode::BAD_REQUEST, "").into_response());
    }
    if !access_check(&appstate, sub_id, &user).await? {
        return Ok((axum::http::StatusCode::FORBIDDEN, "").into_response());
    }

    let client = database::client(&appstate).await?;
    let rows = client.query(
        "SELECT notetype_id, subscribed_fields FROM subscription_field_policy WHERE subscriber_deck_id = $1 AND base_deck_id = $2",
        &[&sub_id, &base_id]
    ).await?;
    let mut policies = Vec::new();
    for r in rows {
        let notetype_id: i64 = r.get(0);
        let sf: Option<Vec<i32>> = r.get(1);
        policies.push(SubscriptionPolicyItem {
            notetype_id,
            subscribed_fields: sf,
        });
    }
    let resp = SubscriptionPolicyGetResponse { policies };
    Ok(Json(resp).into_response())
}

async fn api_post_subscription_policy(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Json(payload): Json<SubscriptionPolicyPostRequest>,
) -> Result<impl IntoResponse, Error> {
    let sub_id = resolve_deck_id_by_hash(&appstate, &payload.subscriber_deck_hash).await?;
    let base_id = resolve_deck_id_by_hash(&appstate, &payload.base_deck_hash).await?;
    if sub_id == 0 || base_id == 0 {
        return Ok((axum::http::StatusCode::BAD_REQUEST, "").into_response());
    }
    if !access_check(&appstate, sub_id, &user).await? {
        return Ok((axum::http::StatusCode::FORBIDDEN, "").into_response());
    }

    let mut client = database::client(&appstate).await?;
    let tx = client.transaction().await?;

    for p in payload.policies {
        match p.subscribed_fields {
            None => {
                // subscribe-all requested -> only allowed if there are NO protected fields for this notetype
                let protected_exists = tx.query(
                    "SELECT 1 FROM notetype_field WHERE notetype = $1 AND protected = true LIMIT 1",
                    &[&p.notetype_id]
                ).await?;
                if protected_exists.is_empty() {
                    tx.execute(
                        "INSERT INTO subscription_field_policy (subscriber_deck_id, base_deck_id, notetype_id, subscribed_fields) VALUES ($1,$2,$3,NULL)
                         ON CONFLICT (subscriber_deck_id, base_deck_id, notetype_id) DO UPDATE SET subscribed_fields = EXCLUDED.subscribed_fields",
                        &[&sub_id, &base_id, &p.notetype_id]
                    ).await?;
                } else {
                    // Fallback: treat as selecting all unprotected fields instead of rejecting outright.
                    let unprot_rows = tx.query(
                        "SELECT position::int FROM notetype_field WHERE notetype = $1 AND protected = false ORDER BY position",
                        &[&p.notetype_id]
                    ).await?;
                    let unprot: Vec<i32> = unprot_rows.iter().map(|r| r.get(0)).collect();
                    tx.execute(
                        "INSERT INTO subscription_field_policy (subscriber_deck_id, base_deck_id, notetype_id, subscribed_fields) VALUES ($1,$2,$3,$4)
                         ON CONFLICT (subscriber_deck_id, base_deck_id, notetype_id) DO UPDATE SET subscribed_fields = EXCLUDED.subscribed_fields",
                        &[&sub_id, &base_id, &p.notetype_id, &unprot]
                    ).await?;
                }
            }
            Some(ref arr) => {
                // Validate arr: must be unique, sorted, and only contain valid, UNPROTECTED field positions for this notetype
                let field_rows = tx
                    .query(
                        "SELECT position::int, protected FROM notetype_field WHERE notetype = $1",
                        &[&p.notetype_id],
                    )
                    .await?;
                let mut valid_positions: Vec<i32> =
                    field_rows.iter().map(|r| r.get::<_, i32>(0)).collect();
                let protected_set: std::collections::HashSet<i32> = field_rows
                    .iter()
                    .filter_map(|r| {
                        let pos: i32 = r.get(0);
                        let prot: bool = r.get(1);
                        if prot {
                            Some(pos)
                        } else {
                            None
                        }
                    })
                    .collect();
                valid_positions.sort_unstable();
                valid_positions.dedup();

                use std::collections::HashSet;
                let vp_set: HashSet<i32> = valid_positions.iter().copied().collect();
                let mut filtered: Vec<i32> = arr
                    .iter()
                    .copied()
                    .filter(|v| vp_set.contains(v) && !protected_set.contains(v))
                    .collect();
                filtered.sort_unstable();
                filtered.dedup();

                // If empty after filtering (e.g., client submitted only protected or invalid), store explicit empty array.
                tx.execute(
                    "INSERT INTO subscription_field_policy (subscriber_deck_id, base_deck_id, notetype_id, subscribed_fields) VALUES ($1,$2,$3,$4)
                     ON CONFLICT (subscriber_deck_id, base_deck_id, notetype_id) DO UPDATE SET subscribed_fields = EXCLUDED.subscribed_fields",
                    &[&sub_id, &base_id, &p.notetype_id, &filtered]
                ).await?;
            }
        }
    }

    tx.commit().await?;
    Ok(axum::http::StatusCode::NO_CONTENT.into_response())
}

async fn page_subscription_policy(
    State(appstate): State<Arc<AppState>>,
    user: Option<User>,
    Path((subscriber_hash, base_hash)): Path<(String, String)>,
) -> Result<impl IntoResponse, Error> {
    let user = check_login(user)?;
    // Authorization: must be owner/maintainer of subscriber deck
    let sub_id = resolve_deck_id_by_hash(&appstate, &subscriber_hash).await?;
    if sub_id == 0 || !access_check(&appstate, sub_id, &user).await? {
        return error_page(&appstate, error::Error::Unauthorized.to_string())
            .await
            .map(IntoResponse::into_response);
    }
    // Ensure the subscription link exists
    let base_id = resolve_deck_id_by_hash(&appstate, &base_hash).await?;
    if base_id == 0 {
        return error_page(&appstate, "Base deck not found.".to_string())
            .await
            .map(IntoResponse::into_response);
    }
    let client_check = database::client(&appstate).await?;
    let exists = client_check.query(
        "SELECT 1 FROM deck_subscriptions WHERE subscriber_deck_id = $1 AND base_deck_id = $2 LIMIT 1",
        &[&sub_id, &base_id]
    ).await?;
    if exists.is_empty() {
        return error_page(
            &appstate,
            "No deck subscription link exists for these decks.".to_string(),
        )
        .await
        .map(IntoResponse::into_response);
    }

    // Build notetype metadata for the subtree
    let client = database::client(&appstate).await?;
    let nt_rows = client
        .query(
            r#"
        WITH RECURSIVE subtree AS (
            SELECT id FROM decks WHERE human_hash = $1
            UNION ALL
            SELECT d.id FROM decks d JOIN subtree s ON d.parent = s.id
        )
        SELECT DISTINCT nt.id, nt.name
        FROM notes n
        JOIN notetype nt ON nt.id = n.notetype
        WHERE n.deck IN (SELECT id FROM subtree) AND n.deleted = false
        "#,
            &[&subscriber_hash],
        )
        .await?;

    let mut notetypes_meta: Vec<serde_json::Value> = Vec::new();
    for r in nt_rows {
        let nt_id: i64 = r.get(0);
        let nt_name: String = r.get(1);
        let fields = client.query(
            "SELECT position::int, name, protected FROM notetype_field WHERE notetype = $1 ORDER BY position",
            &[&nt_id]
        ).await?;
        let mut fs: Vec<serde_json::Value> = Vec::new();
        for f in fields {
            let pos: i32 = f.get(0);
            let fname: String = f.get(1);
            let prot: bool = f.get(2);
            fs.push(serde_json::json!({"position": pos, "name": fname, "protected": prot}));
        }
        notetypes_meta.push(serde_json::json!({"id": nt_id, "name": nt_name, "fields": fs}));
    }

    let mut context = tera::Context::new();
    context.insert("user", &user);
    context.insert("subscriber_hash", &subscriber_hash);
    context.insert("base_hash", &base_hash);
    context.insert(
        "notetypes",
        &serde_json::to_string(&notetypes_meta).unwrap(),
    );
    let rendered_template = appstate.tera.render("subscription_policy.html", &context)?;
    Ok(Html(rendered_template).into_response())
}

async fn manage_decks(
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
        WHERE parent IS NULL and owner = $1
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
            desc: cleanser::clean(row.get(2)),
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
            println!("Error get_notetype_overview: {error}");
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

async fn get_presigned_url(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Json(data): Json<structs::PresignedURLRequest>,
) -> Result<impl IntoResponse, Error> {
    let mut response: structs::PresignedURLResponse = structs::PresignedURLResponse {
        success: false,
        presigned_url: String::new(),
    };

    if data.filename.is_empty() || data.context_type != "note" {
        return Ok(Json(response));
    }

    let parsed_nid = data.context_id.parse::<i64>().unwrap_or(0);
    if parsed_nid == 0 {
        return Ok(Json(response));
    }
    let presigned_url =
        match media_reference_manager::get_presigned_url(&appstate, &data.filename, parsed_nid, user.id())
            .await
        {
            Ok(presigned_url) => presigned_url,
            Err(_error) => return Ok(Json(response)),
        };

    response.success = true;
    response.presigned_url = presigned_url;

    Ok(Json(response))
}

async fn set_static_cache_control(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    response
}

use crate::error::Reporter;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().expect(
        "Expected .env file in the root directory containing the database connection string",
    );
    let _reporter = Reporter::new();

    // Sentry setup
    let _guard = sentry::init((
        env::var("SENTRY_URL").expect("SENTRY_URL must be set"),
        sentry::ClientOptions {
            release: sentry::release_name!(),
            traces_sample_rate: 0.2,
            ..Default::default()
        },
    ));

    let mut tera = match Tera::new("src/templates/**/*.html") {
        Ok(t) => t,
        Err(e) => {
            println!("Parsing error(s): {e}");
            ::std::process::exit(1);
        }
    };
    tera.autoescape_on(vec![".html", ".sql", ".htm", ".xml"]);

    let pool = database::establish_pool_connection()
        .await
        .expect("Failed to establish database connection pool");

    let s3_access_key_id = std::env::var("S3_ACCESS_KEY_ID").expect("S3_ACCESS_KEY_ID must be set");
    let s3_secret_access_key =
        std::env::var("S3_SECRET_ACCESS_KEY").expect("S3_SECRET_ACCESS_KEY must be set");
    let s3_domain = std::env::var("S3_DOMAIN").expect("S3_DOMAIN must be set");

    let credentials = aws_sdk_s3::config::Credentials::new(
        s3_access_key_id,
        s3_secret_access_key,
        None,
        None,
        "s3-credentials",
    );

    let region_provider =
        aws_config::meta::region::RegionProviderChain::default_provider().or_else("eu-central-1"); // Europe (Frankfurt)
    let s3_config = aws_config::from_env()
        .region(region_provider)
        .credentials_provider(aws_sdk_s3::config::SharedCredentialsProvider::new(
            credentials,
        ))
        .endpoint_url(&s3_domain)
        .load()
        .await;

    let s3_service_config = aws_sdk_s3::config::Builder::from(&s3_config)
        .force_path_style(true) // Contabo is <special>
        .build();

    let s3_client = S3Client::from_conf(s3_service_config);

    // Initialize media token service
    let media_token_secret = std::env::var("MEDIA_TOKEN_SECRET")
        .expect("MEDIA_TOKEN_SECRET must be set");
    let media_token_service = media_tokens::MediaTokenService::new(
        media_token_secret.into_bytes(),
        std::time::Duration::from_secs(5 * 60), // 5 minutes
    )
    .expect("Failed to initialize media token service");

    let state = Arc::new(database::AppState {
        db_pool: Arc::new(pool),
        tera: Arc::new(tera),
        s3_client,
        media_token_service,
    });

    // Enable tracing.
    let env_filter = if cfg!(debug_assertions) {
        // Debug build
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            format!(
                "{}=debug,tower_http=debug,axum=trace",
                env!("CARGO_CRATE_NAME")
            )
            .into()
        })
    } else {
        // Release build
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            format!(
                "{}=info,tower_http=info,axum=info",
                env!("CARGO_CRATE_NAME")
            )
            .into()
        })
    };

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer().without_time())
        .init();

    // let governor_conf = Arc::new(
    //     GovernorConfigBuilder::default()
    //         .finish()
    //         .unwrap(),
    // );

    // let governor_limiter = governor_conf.limiter().clone();
    // let interval = std::time::Duration::from_secs(60);
    // // a separate background task to clean up
    // std::thread::spawn(move || {
    //     loop {
    //         std::thread::sleep(interval);
    //         governor_limiter.retain_recent();
    //     }
    // });

    // Second db connection for the auth. idk.. should prolly use the pool for this too
    let (client, connection) = tokio_postgres::connect(
        &env::var("DATABASE_URL").expect("DATABASE_URL must be set"),
        tokio_postgres::NoTls,
    )
    .await
    .expect("Failed to connect to database");
    // Spawn connection handling
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {e}");
        }
    });
    let db = Arc::new(client);
    // Create Auth instance
    let jwt_secret = env::var("JWT_SECRET").expect("JWT_SECRET must be set");
    let auth = Arc::new(Auth::new(
        db.clone(),
        jwt_secret,
        env::var("COOKIE_SECURE").unwrap_or("false".to_string()) == "true",
    ));

    let app = Router::new()
        .route("/login", get(get_login).post(post_login))
        .route("/signup", get(get_signup).post(post_signup))
        .route("/", get(index))
        .route("/terms", get(terms))
        .route("/privacy", get(privacy))
        .route("/imprint", get(imprint))
        .route("/datenschutz", get(datenschutz))
        .route("/logout", get(logout))
        .route("/OptionalTags", post(post_optional_tags))
        .route("/OptionalTags/{deck_hash}", get(show_optional_tags))
        .route("/Maintainers/{deck_hash}", get(show_maintainers))
        .route("/Maintainers", post(post_maintainers))
        // .route("/MediaManager/:deck_hash", get(media_manager))
        // .route("/MediaManager", post(post_media_manager))
        .route("/EditNotetype/{notetype_id}", get(edit_notetype))
        .route(
            "/EditNotetype", 
            post(post_edit_notetype)
                .layer(axum::extract::DefaultBodyLimit::max(5 * 1024 * 1024)) // 5MB limit for notetype updates (to allow large CSS/templates
        )
        .route("/EditDeck/{deck_hash}", get(edit_deck))
        .route("/EditDeck", post(post_edit_deck))
        .route(
            "/DeckSubscriptionPolicy/{subscriber_hash}/{base_hash}",
            get(page_subscription_policy),
        )
        .route(
            "/api/subscription-field-policy",
            get(api_get_subscription_policy).post(api_post_subscription_policy),
        )
        .route("/DeleteChangelog/{changelog_id}", get(delete_changelog))
        .route("/DeleteDeck/{deck_hash}", get(delete_deck))
        .route("/leavereview", get(forward_donation))
        .route("/decks", get(deck_overview))
        .route("/notes/{deck_hash}", get(get_notes_from_deck))
        .route("/ManageDecks", get(manage_decks))
        .route("/review/{note_id}", get(review_note))
        .route("/ToggleStats/{deck_hash}", get(toggle_stats))
        .route("/Statistics/{deck_hash}", get(show_statistics))
        .route("/UpdateStatsPages/{secret}", get(refresh_stats_cache))
        .route("/DenyNoteRemoval/{note_id}", get(deny_note_removal))
        .route("/AcceptNoteRemoval/{note_id}", get(remove_note_from_deck))
        .route("/DenyTag/{tag_id}", get(deny_tag))
        .route("/AcceptTag/{tag_id}", get(accept_tag))
        .route("/DenyNoteMove/{move_id}", get(deny_note_move))
        .route("/AcceptNoteMove/{move_id}", get(accept_note_move))
        .route("/DenyField/{field_id}", get(deny_field))
        .route("/AcceptField/{field_id}", get(accept_field))
        .route("/UpdateFieldSuggestion", post(update_field))
        .route("/DenyCommit/{commit_id}", get(deny_commit))
        .route("/ApproveCommit/{commit_id}", get(approve_commit))
        .route("/commit/{commit_id}", get(review_commit))
        .route("/note_history/{note_id}", get(note_history_page))
        .route("/commit_history/{commit_id}", get(commit_history_page))
        .route("/reviews", get(all_reviews))
        .route("/DeleteNote/{note_id}", get(deny_note))
        .route("/AcceptNote/{note_id}", get(accept_note))
        .route("/GetImageFile", post(get_presigned_url))
        .nest_service(
            "/static",
            ServiceBuilder::new()
                .layer(middleware::from_fn(set_static_cache_control))
                .service(
                    ServeDir::new("src/templates/static")
                        .precompressed_br()
                        .precompressed_gzip(),
                ),
        )
        .layer((
            TraceLayer::new_for_http(),
            // Graceful shutdown will wait for outstanding requests to complete. Add a timeout so
            // requests don't hang forever. Causes issues for streaming large decks that take more than 10secs to generate. hence i disabled it
            //TimeoutLayer::new(Duration::from_secs(10)),
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            error::pretty_error_middleware,
        ))
        // .layer(GovernorLayer {
        //     config: governor_conf,
        // })
        .with_state(state)
        .layer(Extension(auth))
        .layer(ClientIpSource::CfConnectingIp.into_extension());
        //.layer(ClientIpSource::ConnectInfo.into_extension());

    // run it
    let listener = tokio::net::TcpListener::bind("localhost:1337")
        .await
        .unwrap();
    println!("listening on {}", listener.local_addr().unwrap());
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .unwrap();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
