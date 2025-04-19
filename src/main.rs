#![warn(
    clippy::all,
    clippy::pedantic,
    clippy::nursery,
)]

pub mod changelog_manager;
pub mod commit_manager;
pub mod database;
pub mod error;
pub mod gdrive_manager;
pub mod maintainer_manager;
pub mod note_manager;
pub mod notetype_manager;
pub mod optional_tags_manager;
pub mod structs;
pub mod suggestion_manager;
pub mod stats_manager;
pub mod user;
pub mod cleanser;
pub mod media_reference_manager;

use database::owned_deck_id;
use database::AppState;
use net::SocketAddr;
use sync::Arc;
use tokio::signal;
use tower::ServiceBuilder;
use user::{Auth, Credentials, User};
use crate::error::Error;
use crate::error::NoteNotFoundContext;

use tower_http::trace::TraceLayer;
use tower_http::services::ServeDir;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use axum::{
    extract::{Path, State}, 
    http::{header, HeaderValue}, 
    middleware::{self, Next}, 
    response::{Html, IntoResponse, Redirect, Response}, 
    routing::{get, post}, 
    Extension, Json, Router
};

use structs::{BasicDeckInfo, DeckHash, DeckId, DeckOverview, FieldId, NoteId, Return, UpdateNotetype, UserId};
use tera::Tera;

use std::result::Result;
use std::{cfg, env, eprintln, format, i32, i64, net, option_env, panic, println, str, sync, u32, unreachable, usize, vec};
use aws_sdk_s3::Client as S3Client;



fn check_login(user: Option<User>) -> Result<User, Error> {
    match user {
        Some(user) => Ok(user),
        None => Err(error::Error::Redirect("/login".to_string())),
    }
}

async fn forward_donation() -> impl IntoResponse {
    Redirect::permanent("https://ankiweb.net/shared/review/1957538407")
}

async fn get_login(State(appstate): State<Arc<AppState>>,) -> Result<impl IntoResponse, Error> {
    let context = tera::Context::new();
    let rendered_template = appstate.tera.render("login.html", &context)?;
    Ok(Html(rendered_template))
}
async fn post_login(
    Extension(auth): Extension<Arc<Auth>>,
    axum::Form(form): axum::Form<Credentials>,
) -> Result<impl IntoResponse, Error> {
    let res = auth.login(form).await?;

    let mut response = axum::response::Redirect::to("/").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        header::HeaderValue::from_str(&res).unwrap(),
    );

    Ok(response)
}

async fn post_signup(
    Extension(auth): Extension<Arc<Auth>>,
    axum::Form(form): axum::Form<Credentials>,
) -> Result<impl IntoResponse, Error> {
    auth.signup(form.clone()).await?;
    post_login(Extension(auth), axum::Form(form)).await
}

async fn error_page(
    appstate: &Arc<AppState>,
    message: String,
) -> Result<Html<String>, Error> {
    let mut context = tera::Context::new();
    context.insert("message", &message);
    let rendered_template = appstate.tera.render("error.html", &context)?;
    Ok(Html(rendered_template))
}

async fn get_signup(State(appstate): State<Arc<AppState>>,) -> Result<impl IntoResponse, Error> {
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

async fn terms(State(appstate): State<Arc<AppState>>,) -> Result<impl IntoResponse, Error> {
    let context = tera::Context::new();
    let rendered_template = appstate.tera.render("terms.html", &context)?;
    Ok(Html(rendered_template))
}

async fn privacy(State(appstate): State<Arc<AppState>>,) -> Result<impl IntoResponse, Error> {
    let context = tera::Context::new();
    let rendered_template = appstate.tera.render("privacy.html", &context)?;
    Ok(Html(rendered_template))
}

async fn imprint(State(appstate): State<Arc<AppState>>,) -> Result<impl IntoResponse, Error> {
    let context = tera::Context::new();
    let rendered_template = appstate.tera.render("imprint.html", &context)?;
    Ok(Html(rendered_template))
}

async fn logout(Extension(auth): Extension<Arc<Auth>>,) -> Result<impl IntoResponse, Error> {
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

    let rendered_template = appstate.tera
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

    let rendered_template = appstate.tera
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
    let notetype_template_info = client.query("Select id, qfmt, afmt from notetype_template where notetype = $1 and position = 0 LIMIT 1", &[&notetype_id]).await.expect("Error preparing edit notetype statement");

    let protected_fields = notetype_manager::get_protected_fields(&appstate, notetype_id).await?;

    let name: String = notetype_info[0].get(0);
    let styling: String = notetype_info[0].get(1);
    let template_id: i64 = notetype_template_info[0].get(0);
    let front: String = notetype_template_info[0].get(1);
    let back: String = notetype_template_info[0].get(2);

    let mut context = tera::Context::new();
    context.insert("name", &name);
    context.insert("front", &front);
    context.insert("back", &back);
    context.insert("styling", &styling);
    context.insert("template_id", &template_id);
    context.insert("notetype_id", &notetype_id);
    context.insert("user", &user);
    context.insert("protected_fields", &protected_fields);

    let rendered_template = appstate.tera.render("edit_notetype.html", &context)?;
    Ok(Html(rendered_template))
}

async fn post_edit_notetype(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Json(edit_notetype): Json<UpdateNotetype>
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

    context.insert("user", &user);
    context.insert("hash", &deck_hash);
    context.insert("description", &desc);
    context.insert("private", &is_private);
    context.insert("prevent_subdecks", &prevent_subdecks);
    context.insert("restrict_notetypes", &restrict_notetypes);
    context.insert("changelogs", &changelogs);

    let rendered_template = appstate.tera
        .render("edit_deck.html", &context)
        .expect("Failed to render template");
    Ok(Html(rendered_template))
}

async fn post_edit_deck(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Json(edit_deck_data): Json<structs::EditDecksData>
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
            &[&cleaned_desc, &data.is_private, &data.prevent_subdecks, &data.restrict_notetypes, &data.hash, &user.id()],
        )
        .await?;

    if !data.changelog.is_empty() {
        changelog_manager::insert_new_changelog(&appstate, &data.hash, &data.changelog).await?;
    }

    Ok(Redirect::permanent(&format!("/EditDeck/{}", data.hash)))
}

async fn delete_changelog(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Path(changelog_id): Path<i64>
) -> Result<impl IntoResponse, Error> {
    match changelog_manager::delete_changelog(&appstate, changelog_id, user.id()).await {
        Ok(hash) => Ok(Redirect::permanent(format!("/EditDeck/{hash}").as_str())),
        Err(_err) => Ok(Redirect::permanent("/")),
    }
}

async fn delete_deck(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Path(deck_hash): Path<String>
) -> Result<impl IntoResponse, Error> {
    let client = database::client(&appstate).await?;
    let _ = owned_deck_id(&appstate, &deck_hash, user.id()).await?; // only for checking if user owns the deck

    client
        .query("Select delete_deck($1)", &[&deck_hash])
        .await?;

    // This query is quite expensive, but it is only used when deleting a deck, so it should be fine. I use it to trigger a cleanup
    client
        .query("DELETE FROM notetype CASCADE WHERE id NOT IN (SELECT notetype FROM notes)", &[]).await?;

    Ok(Redirect::permanent("/"))
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

    let rendered_template = appstate.tera
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

    let note = match note_manager::get_note_data(&appstate, note_id).await {
        Ok(note) => note,
        Err(_error) => {
            return error_page(
                &appstate,
                error::Error::NoteNotFound(NoteNotFoundContext::InvalidData).to_string()
            )
            .await;
        }
    };

    if note.id == 0 {
        // Invalid data // No note found!
        return error_page(
            &appstate,
            error::Error::NoteNotFound(NoteNotFoundContext::InvalidData).to_string()
        )
        .await;
    }

    let mut access = false;

    if let Some(ref user) = user {
        let client = database::client(&appstate).await?;
        let q_guid = client
            .query("Select deck from notes where id = $1", &[&note_id])
            .await?;
        if q_guid.is_empty() {
            return error_page(
                &appstate,
                error::Error::NoteNotFound(NoteNotFoundContext::InvalidData).to_string()
            )
            .await;
        }
        let deck_id: i64 = q_guid[0].get(0);

        access = suggestion_manager::is_authorized(&appstate, user, deck_id).await?;
    }

    context.insert("note", &note);
    context.insert("access", &access);
    context.insert("user", &user);
    let rendered_template = appstate.tera
        .render("review.html", &context)
        .expect("Failed to render template");

    // Return the rendered HTML as the response
    Ok(Html(rendered_template))
}

async fn access_check(
    appstate: &Arc<AppState>,
    deck_id: i64, 
    user: &User
) -> Result<bool, Error> {
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

async fn get_deck_by_tag_id(
    appstate: &Arc<AppState>, 
    tag_id: i64
) -> Return<DeckId> {
    let query = "Select deck from notes where id = (select note from tags where id = $1)";
    get_deck_id(appstate, query, &tag_id).await
}

async fn get_deck_by_field_id(
    appstate: &Arc<AppState>,
    field_id: FieldId
) -> Return<DeckId> {
    let query = "Select deck from notes where id = (select note from fields where id = $1)";
    get_deck_id(appstate, query, &field_id).await
}

async fn get_deck_by_move_id(
    appstate: &Arc<AppState>,
    move_id: i32
) -> Return<DeckId> {
    let query = "Select original_deck from note_move_suggestions where id = $1";
    get_deck_id(appstate, query, &move_id).await
}

async fn deny_tag(
    State(appstate): State<Arc<AppState>>,
    Path(tag_id): Path<i64>,
    user: User
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

    match suggestion_manager::deny_tag_change(&appstate, tag_id).await {
        Ok(res) => Ok(Redirect::to(&format!("/review/{res}"))),
        Err(error) => {
            println!("Error: {error}");
            Ok(Redirect::to("/"))
        }
    }
}

async fn deny_note_move(
    State(appstate): State<Arc<AppState>>,
    Path(move_id): Path<i32>,
    user: User
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

    match suggestion_manager::deny_note_move_request(&appstate, move_id).await {
        Ok(res) => Ok(Redirect::to(&format!("/review/{res}"))),
        Err(error) => {
            println!("Error: {error}");
            Ok(Redirect::to("/"))
        }
    }
}

async fn accept_note_move(
    State(appstate): State<Arc<AppState>>,
    Path(move_id): Path<i32>,
    user: User
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

    match suggestion_manager::approve_move_note_request_by_moveid(&appstate, move_id).await {
        Ok(res) => Ok(Redirect::to(&format!("/review/{res}"))),
        Err(error) => {
            println!("Error: {error}");
            Ok(Redirect::to("/"))
        }
    }
}

async fn accept_tag(
    State(appstate): State<Arc<AppState>>,
    Path(tag_id): Path<i64>,
    user: User
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

    match suggestion_manager::approve_tag_change(&appstate, tag_id, true).await {
        Ok(res) => Ok(Redirect::to(&format!("/review/{res}"))),
        Err(error) => {
            println!("Error: {error}");
            Ok(Redirect::to("/"))
        }
    }
}

async fn deny_field(
    State(appstate): State<Arc<AppState>>,
    Path(field_id): Path<i64>,
    user: User
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

    match suggestion_manager::deny_field_change(&appstate, field_id, true).await {
        Ok(res) => Ok(Redirect::to(&format!("/review/{res}"))),
        Err(error) => {
            println!("Error: {error}");
            Ok(Redirect::to("/"))
        }
    }
}

async fn accept_field(
    State(appstate): State<Arc<AppState>>,
    Path(field_id): Path<i64>,
    user: User
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

    match suggestion_manager::approve_field_change(&appstate, field_id, true).await {
        Ok(res) => Ok(Redirect::to(&format!("/review/{res}"))),
        Err(error) => {
            println!("Error: {error}");
            Ok(Redirect::to("/"))
        }
    }
}

async fn update_field(
    State(appstate): State<Arc<AppState>>,
    user: User,
    Json(edit_optional_tag): Json<structs::UpdateFieldSuggestion>
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

    match suggestion_manager::update_field_suggestion(&appstate, data.field_id, &data.content).await {
        Ok(_res) => {
            match commit_manager::get_field_diff(&appstate, data.field_id).await {
                Ok(diff) => {
                    Ok(diff)
                },
                Err(error) => {
                    println!("Error: {error}");
                    Ok(String::new())
                }
            }            
        },
        Err(error) => {
            println!("Error: {error}");
            Ok(String::new())
        }
    }

}

async fn accept_note(
    State(appstate): State<Arc<AppState>>,
    Path(note_id): Path<i64>,
    user: User
) -> Result<impl IntoResponse, Error> {
    match suggestion_manager::approve_card(&appstate, note_id, user, false).await {
        Ok(res) => Ok(Redirect::to(&format!("/review/{res}"))),
        Err(error) => {
            println!("Error: {error}");
            Ok(Redirect::to("/"))
        }
    }
}

// This actually removes the note from the database (Only used for notes that are not approved yet)
async fn deny_note(
    State(appstate): State<Arc<AppState>>,
    Path(note_id): Path<i64>,
    user: User
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
    user: User
) -> Result<impl IntoResponse, Error> {
    match note_manager::mark_note_deleted(&appstate, note_id, user, false).await {
        Ok(res) => Ok(Redirect::to(&format!("/notes/{res}"))),
        Err(error) => {
            println!("Error: {error}");
            Ok(Redirect::to("/"))
        }
    }
}

async fn deny_note_removal(
    State(appstate): State<Arc<AppState>>,
    Path(note_id): Path<i64>,
    user: User
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

static STATS_CACHE_KEY: Lazy<String> = Lazy::new(|| {
    std::env::var("STATS_CACHE_KEY").expect("STATS_CACHE_KEY must be set")
});

async fn refresh_stats_cache(
    State(appstate): State<Arc<AppState>>,
    Path(secret): Path<String>
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
    user: User
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

    stats_manager::toggle_stats(&appstate, deck_id).await.unwrap();

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
        .query(
            "Select id from decks where human_hash = $1",
            &[&deck_hash],
        )
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
        let rendered_template = appstate.tera
        .render("empty_stats.html", &context)
        .expect("Failed to render template");
        return Ok(Html(rendered_template));
    }
    
    let deck_info = match stats_manager::get_deck_stat_info(&appstate, &deck_hash).await {
        Ok(deck_info) => deck_info,
        Err(error) => {
            println!("Error get_deck_stat_info: {error}");
            return Ok(Html("Error showing the statistics.".to_string()));
        },
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

    let rendered_template = appstate.tera
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

    // let deck_name = decks::get_name_by_hash(&deck_hash).await;
    // if deck_name.is_err() {
    //     return Html(format!("Deck not found."))
    // }

    let notes = note_manager::retrieve_notes(&appstate, &deck_hash).await?;

    let client = database::client(&appstate).await?;
    let deck_info = client.query("Select id, name, description, human_hash, owner, TO_CHAR(last_update, 'MM/DD/YYYY') AS last_update from decks where human_hash = $1 Limit 1", &[&deck_hash]).await.expect("Error preparing deck notes statement");
    if deck_info.is_empty() {
        return error_page(&appstate, error::Error::DeckNotFound.to_string()).await;
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

    let rendered_template = appstate.tera
        .render("notes.html", &context)
        .expect("Failed to render template");

    // Return the rendered HTML as the response
    Ok(Html(rendered_template))
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

    let rendered_template = appstate.tera
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
        .prepare("
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
    let rendered_template = appstate.tera
        .render("decks.html", &context)
        .expect("Failed to render template");

    // Return the rendered HTML as the response
    Ok(Html(rendered_template))
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
                .unwrap().to_string(),
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

    let rendered_template = appstate.tera
        .render("manage_decks.html", &context)
        .expect("Failed to render template");

    Ok(Html(rendered_template))
}

async fn get_presigned_url(
    State(appstate): State<Arc<AppState>>,
    _user: User,
    Json(data): Json<structs::PresignedURLRequest>
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
    let presigned_url = match media_reference_manager::get_presigned_url(&appstate, &data.filename, parsed_nid).await {
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
    let _guard = sentry::init((env::var("SENTRY_URL").expect("SENTRY_URL must be set"), sentry::ClientOptions {
        release: sentry::release_name!(),
        traces_sample_rate: 0.2,
        ..Default::default()
        }));

    let mut tera = match Tera::new("src/templates/**/*.html") {
        Ok(t) => t,
        Err(e) => {
            println!("Parsing error(s): {e}");
            ::std::process::exit(1);
        }
    };
    tera.autoescape_on(vec![".html", ".sql", ".htm", ".xml"]);

    let pool = database::establish_pool_connection().await.expect("Failed to establish database connection pool");
    
    let s3_access_key_id = std::env::var("S3_ACCESS_KEY_ID").expect("S3_ACCESS_KEY_ID must be set");
    let s3_secret_access_key = std::env::var("S3_SECRET_ACCESS_KEY").expect("S3_SECRET_ACCESS_KEY must be set");
    let s3_domain = std::env::var("S3_DOMAIN").expect("S3_DOMAIN must be set");

    let credentials = aws_sdk_s3::config::Credentials::new(
        s3_access_key_id,
        s3_secret_access_key,
        None, None, "s3-credentials");
    
    let region_provider = aws_config::meta::region::RegionProviderChain::default_provider().or_else("eu-central-1"); // Europe (Frankfurt)
    let s3_config = aws_config::from_env()
        .region(region_provider)
        .credentials_provider(aws_sdk_s3::config::SharedCredentialsProvider::new(credentials))
        .endpoint_url(&s3_domain)
        .load()
        .await;
    
    let s3_service_config = aws_sdk_s3::config::Builder::from(&s3_config)
    .force_path_style(true) // Contabo is <special>
    .build();
    
    let s3_client = S3Client::from_conf(s3_service_config);


    let state = Arc::new(database::AppState {
        db_pool: Arc::new(pool),
        tera: Arc::new(tera),
        s3_client,
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
    ).await.expect("Failed to connect to database");
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
        .route("/logout", get(logout))
        .route("/OptionalTags", post(post_optional_tags))
        .route("/OptionalTags/{deck_hash}", get(show_optional_tags))
        .route("/Maintainers/{deck_hash}", get(show_maintainers))
        .route("/Maintainers", post(post_maintainers))
        // .route("/MediaManager/:deck_hash", get(media_manager))
        // .route("/MediaManager", post(post_media_manager))
        .route("/EditNotetype/{notetype_id}", get(edit_notetype))
        .route("/EditNotetype", post(post_edit_notetype))
        .route("/EditDeck/{deck_hash}", get(edit_deck))
        .route("/EditDeck", post(post_edit_deck))
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
        .layer(Extension(auth));

    // run it
    let listener = tokio::net::TcpListener::bind("localhost:1337").await.unwrap();
    println!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
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
