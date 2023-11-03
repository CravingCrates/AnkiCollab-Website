extern crate rocket;

#[macro_use(lazy_static)]
extern crate lazy_static;

extern crate ammonia;
extern crate html5ever;

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

use database::owned_deck_id;
use error::NoteNotFoundReason;
use rocket::fs::FileServer;
use rocket::http::Status;
use rocket::response::content;
use rocket::serde::json::Json;
use rocket::Request;
use rocket::*;
use rocket::{form::*, get, post, response::Redirect};

use rocket_auth::{Auth, Error, Login, Signup, User, Users};

use structs::*;
use tera::Tera;

use std::result::Result;
use std::*;

use tokio_postgres::connect;

pub type Return<T> = Result<T, error::Error>;
pub type DeckHash = String;
pub type UserId = i32;
pub type DeckId = i64;
pub type NoteId = i64;
pub type FieldId = i64;

lazy_static! {
    pub static ref TEMPLATES: Tera = {
        let mut tera = match Tera::new("src/templates/**/*") {
            Ok(t) => t,
            Err(e) => {
                println!("Parsing error(s): {}", e);
                ::std::process::exit(1);
            }
        };
        tera.autoescape_on(vec![".html", ".sql"]);
        tera
    };
}

// Review: This does not make much sense, the option can only ever be Some(_), never None
fn check_login(user: Option<User>) -> Return<Option<User>> {
    match user {
        Some(user) => Ok(Some(user)),
        None => Err(error::Error::Redirect("/login")),
    }
}

#[catch(500)]
fn internal_error() -> content::RawHtml<String> {
    let mut context = tera::Context::new();
    context.insert(
        "message",
        "Whoops! Looks like we messed up. Can you inform us on Discord please?",
    );
    let rendered_template = TEMPLATES
        .render("error.html", &context)
        .expect("Failed to render template");
    content::RawHtml(rendered_template)
}

#[catch(404)]
fn not_found(req: &Request) -> content::RawHtml<String> {
    let mut context = tera::Context::new();
    context.insert(
        "message",
        &format!("I couldn't find '{}'. Try something else?", req.uri()),
    );
    let rendered_template = TEMPLATES
        .render("error.html", &context)
        .expect("Failed to render template");
    content::RawHtml(rendered_template)
}

#[catch(default)]
fn default(status: Status, req: &Request) -> String {
    format!("{} ({})", status, req.uri())
}

#[get("/donate")]
fn forward_donation() -> Redirect {
    Redirect::to("https://ankiweb.net/shared/review/1957538407")
}

#[get("/login")]
fn get_login() -> content::RawHtml<String> {
    let context = tera::Context::new();
    let rendered_template = TEMPLATES
        .render("login.html", &context)
        .expect("Failed to render template");
    content::RawHtml(rendered_template)
}

#[post("/login", data = "<form>")]
async fn post_login(auth: Auth<'_>, form: Form<Login>) -> Result<Redirect, Redirect> {
    let result = auth.login(&form).await;
    match result {
        Ok(_) => Ok(Redirect::to("/")),
        Err(e) => Err(Redirect::to(uri!(error_page(e.to_string())))),
    }
}

#[post("/signup", data = "<form>")]
async fn post_signup(auth: Auth<'_>, form: Form<Signup>) -> Result<Redirect, Redirect> {
    let result = auth.signup(&form).await;
    match result {
        Ok(_) => {
            let login_form: Login = form.into();
            match auth.login(&login_form).await {
                Ok(_) => Ok(Redirect::to("/")),
                Err(e) => Err(Redirect::to(uri!(error_page(e.to_string())))),
            }
        }
        Err(Error::FormValidationErrors(source)) => {
            let mut error_message = String::new();
            for (field, errors) in source.field_errors() {
                error_message.push_str(&format!("{}:\n", field));
                for error in errors {
                    error_message.push_str(&format!("\t{}\n", error.code));
                }
            }
            Err(Redirect::to(uri!(error_page(error_message))))
        }
        Err(e) => Err(Redirect::to(uri!(error_page(format!("{}", e))))),
    }
}

#[get("/error?<message>")]
async fn error_page(message: String) -> Return<content::RawHtml<String>> {
    let mut context = tera::Context::new();
    context.insert("message", &message);
    let rendered_template = TEMPLATES.render("error.html", &context)?;
    Ok(content::RawHtml(rendered_template))
}

#[get("/signup")]
async fn get_signup() -> Return<content::RawHtml<String>> {
    let context = tera::Context::new();
    let rendered_template = TEMPLATES.render("signup.html", &context)?;

    Ok(content::RawHtml(rendered_template))
}

#[get("/")]
async fn index(user: Option<User>) -> Return<content::RawHtml<String>> {
    let mut context = tera::Context::new();
    context.insert("user", &user);
    let rendered_template = TEMPLATES.render("index.html", &context)?;

    Ok(content::RawHtml(rendered_template))
}
#[get("/terms")]
async fn terms() -> Return<content::RawHtml<String>> {
    let context = tera::Context::new();
    let rendered_template = TEMPLATES.render("terms.html", &context)?;

    Ok(content::RawHtml(rendered_template))
}
#[get("/privacy")]
async fn privacy() -> Return<content::RawHtml<String>> {
    let context = tera::Context::new();
    let rendered_template = TEMPLATES.render("privacy.html", &context)?;

    Ok(content::RawHtml(rendered_template))
}

#[get("/logout")]
fn logout(auth: Auth<'_>) -> Return<content::RawHtml<String>> {
    auth.logout()?;

    let context = tera::Context::new();
    let rendered_template = TEMPLATES.render("logout.html", &context)?;
    Ok(content::RawHtml(rendered_template))
}

async fn render_optional_tags(
    deck_hash: &String,
    deck_id: i64,
    user: User,
) -> content::RawHtml<String> {
    // Get Tags by deck id
    let tags = match optional_tags_manager::get_tags(deck_id).await {
        Ok(tags) => tags,
        Err(e) => {
            println!("Error retrieving opt tags: {}", e);
            return content::RawHtml(
                "Error retrieving optional tags. Please notify us.".to_string(),
            );
        }
    };

    let mut context = tera::Context::new();
    context.insert("optional_tags", &tags);
    context.insert("hash", &deck_hash);
    context.insert("user", &user);

    let rendered_template = TEMPLATES
        .render("OptionalTags.html", &context)
        .expect("Failed to render template");
    content::RawHtml(rendered_template)
}

#[post(
    "/OptionalTags",
    format = "application/json",
    data = "<edit_optional_tag>"
)]
async fn post_optional_tags(
    user: User,
    edit_optional_tag: Json<structs::UpdateOptionalTag>,
) -> Return<String> {
    let data = edit_optional_tag.into_inner();

    let deck_id: i64 = owned_deck_id(&data.deck, user.id()).await?;

    // Add new tag
    if data.action == 1 {
        optional_tags_manager::add_tag(deck_id, data.taggroup).await
    } else {
        // Delete existing optional_tag
        optional_tags_manager::remove_tag(deck_id, data.taggroup).await
    }
}

#[get("/OptionalTags/<deck_hash>")]
async fn show_optional_tags(user: User, deck_hash: DeckHash) -> Return<content::RawHtml<String>> {
    let deck_id: i64 = owned_deck_id(&deck_hash, user.id()).await?;

    Ok(render_optional_tags(&deck_hash, deck_id, user).await)
}

async fn render_maintainers(
    deck_hash: &String,
    deck_id: i64,
    user: User,
) -> content::RawHtml<String> {
    // Get Maintainers by deck id
    let maintainers = match maintainer_manager::get_maintainers(deck_id).await {
        Ok(maintainers) => maintainers,
        Err(e) => {
            println!("Error getting maintainers: {}", e);
            return content::RawHtml("Error getting maintainers.".to_string());
        }
    };

    let mut context = tera::Context::new();
    context.insert("maintainers", &maintainers);
    context.insert("hash", &deck_hash);
    context.insert("user", &user);

    let rendered_template = TEMPLATES
        .render("maintainers.html", &context)
        .expect("Failed to render template");
    content::RawHtml(rendered_template)
}

#[post(
    "/Maintainers",
    format = "application/json",
    data = "<edit_maintainer>"
)]
async fn post_maintainers(
    user: User,
    edit_maintainer: Json<structs::UpdateMaintainer>,
) -> Return<String> {
    let data = edit_maintainer.into_inner();

    let deck_id: i64 = owned_deck_id(&data.deck, user.id()).await?;

    // Add new maintainer
    if data.action == 1 {
        maintainer_manager::add_maintainer(deck_id, data.email).await
    } else {
        // Delete existing maintainer
        maintainer_manager::remove_maintainer(deck_id, data.email).await
    }
}

#[post("/MediaManager", format = "application/json", data = "<update_media>")]
async fn post_media_manager(user: User, update_media: Json<structs::GDriveInfo>) -> Return<String> {
    let data = update_media.into_inner();

    let deck_id: i64 = owned_deck_id(&data.deck, user.id()).await?;

    gdrive_manager::update_media(deck_id, data).await
}

#[get("/MediaManager/<deck_hash>")]
async fn media_manager(user: User, deck_hash: String) -> Return<content::RawHtml<String>> {
    let mut context = tera::Context::new();
    context.insert("hash", &deck_hash);
    context.insert("user", &user);

    let rendered_template = TEMPLATES
        .render("media_manager.html", &context)
        .expect("Failed to render template");
    Ok(content::RawHtml(rendered_template))
}

#[get("/Maintainers/<deck_hash>")]
async fn show_maintainers(user: User, deck_hash: String) -> Return<content::RawHtml<String>> {
    let deck_id: i64 = owned_deck_id(&deck_hash, user.id()).await?;

    Ok(render_maintainers(&deck_hash, deck_id, user).await)
}

#[get("/EditNotetype/<notetype_id>")]
async fn edit_notetype(user: User, notetype_id: i64) -> Return<content::RawHtml<String>> {
    let client = database::client().await?;

    let owned_info = client
        .query(
            "SELECT 1 FROM notetype WHERE (owner = $1 AND id = $3) OR $2 LIMIT 1",
            &[&user.id(), &user.is_admin, &notetype_id],
        )
        .await
        .expect("Error preparing edit notetype statement");
    if owned_info.is_empty() {
        return Err(error::Error::Unauthorized);
    }

    let notetype_info = client
        .query(
            "Select name, css from notetype where id = $1",
            &[&notetype_id],
        )
        .await
        .expect("Error preparing edit notetype statement");
    let notetype_template_info = client.query("Select id, qfmt, afmt from notetype_template where notetype = $1 and position = 0 LIMIT 1", &[&notetype_id]).await.expect("Error preparing edit notetype statement");

    let protected_fields = notetype_manager::get_protected_fields(notetype_id).await?;

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

    let rendered_template = TEMPLATES
        .render("edit_notetype.html", &context)
        .expect("Failed to render template");
    Ok(content::RawHtml(rendered_template))
}

#[post("/EditNotetype", format = "application/json", data = "<edit_notetype>")]
async fn post_edit_notetype(user: User, edit_notetype: Json<structs::UpdateNotetype>) -> String {
    let data = edit_notetype.into_inner();

    match notetype_manager::update_notetype(&user, &data).await {
        Ok(_res) => "updated".to_owned(),
        Err(e) => e.to_string(),
    }
}

#[get("/EditDeck/<deck_hash>")]
async fn edit_deck(user: Option<User>, deck_hash: String) -> Return<content::RawHtml<String>> {
    let user = check_login(user)?;
    let client = database::client().await?;
    let owned_info = client
        .query(
            "Select owner, description, private, id from decks where human_hash = $1",
            &[&deck_hash],
        )
        .await
        .expect("Error preparing edit deck statement");
    if owned_info.is_empty() {
        return Ok(content::RawHtml("Deck not found.".to_string()));
    }
    let owner: i32 = owned_info[0].get(0);

    let mut context = tera::Context::new();
    if let Some(user) = &user {
        if owner != user.id() {
            return Err(error::Error::Unauthorized);
        }

        let desc: String = owned_info[0].get(1);
        let is_private: bool = owned_info[0].get(2);

        let changelogs = changelog_manager::get_changelogs(&deck_hash).await?;

        context.insert("user", &user);
        context.insert("hash", &deck_hash);
        context.insert("description", &desc);
        context.insert("private", &is_private);
        context.insert("changelogs", &changelogs);
    }

    let rendered_template = TEMPLATES
        .render("edit_deck.html", &context)
        .expect("Failed to render template");
    Ok(content::RawHtml(rendered_template))
}

#[post("/EditDeck", format = "application/json", data = "<edit_deck_data>")]
async fn post_edit_deck(
    user: User,
    edit_deck_data: Json<structs::EditDecksData>,
) -> Return<Redirect> {
    let client = database::client().await?;
    let data = edit_deck_data.into_inner();

    let _ = owned_deck_id(&data.hash, user.id()).await?; // only for checking if user owns the deck

    client
        .query(
            "
        UPDATE decks 
        SET description = $1, private = $2 
        WHERE human_hash = $3
        AND owner = $4",
            &[&data.description, &data.is_private, &data.hash, &user.id()],
        )
        .await?;

    if !data.changelog.is_empty() {
        changelog_manager::insert_new_changelog(&data.hash, &data.changelog).await?;
    }

    Ok(Redirect::to(format!("/EditDeck/{}", data.hash)))
}

#[get("/DeleteChangelog/<changelog_id>")]
async fn delete_changelog(user: User, changelog_id: i64) -> Return<Redirect> {
    match changelog_manager::delete_changelog(changelog_id, user.id()).await {
        Ok(hash) => Ok(Redirect::to(format!("/EditDeck/{}", hash))),
        Err(_err) => Ok(Redirect::to("/")),
    }
}

#[get("/DeleteDeck/<deck_hash>")]
async fn delete_deck(user: User, deck_hash: String) -> Return<Redirect> {
    let client = database::client().await?;
    let _ = owned_deck_id(&deck_hash, user.id()).await?; // only for checking if user owns the deck

    client
        .query("Select delete_deck($1)", &[&deck_hash])
        .await?;

    Ok(Redirect::to("/"))
}

// REVIEW: You don't seem to be using this function anywhere. Is it still needed?

// #[get("/AsyncApproveCommit/<commit_id>")]
// async fn async_approve_commit(commit_id: i32, user: User) -> Result<Redirect, Error> {
//     tokio::spawn(async move {
//         match suggestion_manager::merge_by_commit(commit_id, true, user).await {
//             Ok(_res) => println!("Async approved commit {}", commit_id),
//             Err(error) => println!("Async approve commit Error: {}", error),
//         };
//     });
//     Ok(Redirect::to("/"))
// }

#[get("/ApproveCommit/<commit_id>")]
async fn approve_commit(commit_id: i32, user: User) -> Return<Redirect> {
    let res = suggestion_manager::merge_by_commit(commit_id, true, user).await?;

    if res.is_none() {
        Ok(Redirect::to("/reviews".to_string()))
    } else {
        Ok(Redirect::to(format!("/commit/{}", res.unwrap())))
    }
}

#[get("/DenyCommit/<commit_id>")]
async fn deny_commit(commit_id: i32, user: User) -> Result<Redirect, Error> {
    match suggestion_manager::merge_by_commit(commit_id, false, user).await {
        Ok(res) => {
            if res.is_none() {
                Ok(Redirect::to("/reviews".to_string()))
            } else {
                Ok(Redirect::to(format!("/commit/{}", res.unwrap())))
            }
        }
        Err(error) => {
            println!("Error: {}", error);
            Ok(Redirect::to("/"))
        }
    }
}

#[get("/commit/<commit_id>")]
async fn review_commit(commit_id: i32, user: User) -> Return<content::RawHtml<String>> {
    let mut context = tera::Context::new();

    let notes = commit_manager::notes_by_commit(commit_id).await?;

    let commit = commit_manager::get_commit_info(commit_id).await?;

    let client = database::client().await?;
    let q_guid = client
        .query(
            "Select deck from commits where commit_id = $1",
            &[&commit_id],
        )
        .await?;
    if q_guid.is_empty() {
        return Err(error::Error::CommitNotFound);
    }
    let deck_id: i64 = q_guid[0].get(0);

    let access = suggestion_manager::is_authorized(&user, deck_id).await?;
    let notemodels = notetype_manager::notetypes_by_commit(commit_id).await?;

    context.insert("notes", &notes);
    context.insert("commit", &commit);
    context.insert("user", &user);
    context.insert("owned", &access);
    context.insert("notemodels", &notemodels);

    let rendered_template = TEMPLATES
        .render("commit.html", &context)
        .expect("Failed to render template");

    // Return the rendered HTML as the response
    Ok(content::RawHtml(rendered_template))
}

#[get("/review/<note_id>")]
async fn review_note(note_id: i64, user: Option<User>) -> Return<content::RawHtml<String>> {
    let mut context = tera::Context::new();

    let note = note_manager::get_note_data(note_id).await?;
    if note.id == 0 {
        // Invalid data // No note found!
        return Err(error::Error::NoteNotFound(NoteNotFoundReason::InvalidData));
    }

    let mut access = false;

    if let Some(ref user) = user {
        let client = database::client().await?;
        let q_guid = client
            .query("Select deck from notes where id = $1", &[&note_id])
            .await?;
        if q_guid.is_empty() {
            return Err(error::Error::NoteNotFound(NoteNotFoundReason::InvalidData));
        }
        let deck_id: i64 = q_guid[0].get(0);

        access = suggestion_manager::is_authorized(user, deck_id).await?;
    }

    context.insert("note", &note);
    context.insert("access", &access);
    context.insert("user", &user);
    let rendered_template = TEMPLATES
        .render("review.html", &context)
        .expect("Failed to render template");

    // Return the rendered HTML as the response
    Ok(content::RawHtml(rendered_template))
}

async fn access_check(deck_id: i64, user: &User) -> Result<bool, Error> {
    let access = match suggestion_manager::is_authorized(user, deck_id).await {
        Ok(access) => access,
        Err(_error) => return Ok(false),
    };

    if !access {
        return Ok(false);
    }

    Ok(true)
}

async fn get_deck_by_tag_id(tag_id: i64) -> Return<i64> {
    let client = database::client().await?;
    let q_guid = match client
        .query(
            "Select deck from notes where id = (select note from tags where id = $1)",
            &[&tag_id],
        )
        .await
    {
        Ok(q_guid) => q_guid,
        Err(_error) => return Ok(0),
    };
    if q_guid.is_empty() {
        return Ok(0);
    }
    let deck_id: i64 = q_guid[0].get(0);

    Ok(deck_id)
}

async fn get_deck_by_field_id(field_id: FieldId) -> Return<DeckId> {
    let client = database::client().await?;
    let q_guid = match client
        .query(
            "Select deck from notes where id = (select note from fields where id = $1)",
            &[&field_id],
        )
        .await
    {
        Ok(q_guid) => q_guid,
        Err(_error) => return Ok(0),
    };
    if q_guid.is_empty() {
        return Ok(0);
    }
    let deck_id: DeckId = q_guid[0].get(0);

    Ok(deck_id)
}

#[get("/DenyTag/<tag_id>")]
async fn deny_tag(tag_id: i64, user: User) -> Return<Redirect> {
    let deck_id = match get_deck_by_tag_id(tag_id).await {
        Ok(deck_id) => deck_id,
        Err(error) => {
            println!("Error: {}", error);
            return Ok(Redirect::to("/"));
        }
    };

    if !access_check(deck_id, &user).await? {
        return Ok(Redirect::to("/"));
    }

    match suggestion_manager::deny_tag_change(tag_id).await {
        Ok(res) => Ok(Redirect::to(format!("/review/{}", res))),
        Err(error) => {
            println!("Error: {}", error);
            Ok(Redirect::to("/"))
        }
    }
}

#[get("/AcceptTag/<tag_id>")]
async fn accept_tag(tag_id: i64, user: User) -> Result<Redirect, Error> {
    let deck_id = match get_deck_by_tag_id(tag_id).await {
        Ok(deck_id) => deck_id,
        Err(error) => {
            println!("Error: {}", error);
            return Ok(Redirect::to("/"));
        }
    };

    if !access_check(deck_id, &user).await? {
        return Ok(Redirect::to("/"));
    }

    match suggestion_manager::approve_tag_change(tag_id, true).await {
        Ok(res) => Ok(Redirect::to(format!("/review/{}", res))),
        Err(error) => {
            println!("Error: {}", error);
            Ok(Redirect::to("/"))
        }
    }
}

#[get("/DenyField/<field_id>")]
async fn deny_field(field_id: i64, user: User) -> Result<Redirect, Error> {
    let deck_id = match get_deck_by_field_id(field_id).await {
        Ok(deck_id) => deck_id,
        Err(error) => {
            println!("Error: {}", error);
            return Ok(Redirect::to("/"));
        }
    };

    if !access_check(deck_id, &user).await? {
        return Ok(Redirect::to("/"));
    }

    match suggestion_manager::deny_field_change(field_id).await {
        Ok(res) => Ok(Redirect::to(format!("/review/{}", res))),
        Err(error) => {
            println!("Error: {}", error);
            Ok(Redirect::to("/"))
        }
    }
}

#[get("/AcceptField/<field_id>")]
async fn accept_field(field_id: i64, user: User) -> Result<Redirect, Error> {
    let deck_id = match get_deck_by_field_id(field_id).await {
        Ok(deck_id) => deck_id,
        Err(error) => {
            println!("Error: {}", error);
            return Ok(Redirect::to("/"));
        }
    };

    if !access_check(deck_id, &user).await? {
        return Ok(Redirect::to("/"));
    }

    match suggestion_manager::approve_field_change(field_id, true).await {
        Ok(res) => Ok(Redirect::to(format!("/review/{}", res))),
        Err(error) => {
            println!("Error: {}", error);
            Ok(Redirect::to("/"))
        }
    }
}

#[get("/AcceptNote/<note_id>")]
async fn accept_note(note_id: i64, user: User) -> Result<Redirect, Error> {
    match suggestion_manager::approve_card(note_id, user, false).await {
        Ok(res) => Ok(Redirect::to(format!("/review/{}", res))),
        Err(error) => {
            println!("Error: {}", error);
            Ok(Redirect::to("/"))
        }
    }
}

// This actually removes the note from the database (Only used for notes that are not approved yet)
#[get("/DeleteNote/<note_id>")]
async fn deny_note(note_id: i64, user: User) -> Result<Redirect, Error> {
    match suggestion_manager::delete_card(note_id, user).await {
        Ok(res) => Ok(Redirect::to(format!("/notes/{}", res))),
        Err(error) => {
            println!("Error: {}", error);
            Ok(Redirect::to("/"))
        }
    }
}

// This marks the note as deleted, but does not remove them (Used for existing notes that are approved)
#[get("/AcceptNoteRemoval/<note_id>")]
async fn remove_note_from_deck(note_id: i64, user: User) -> Result<Redirect, Error> {
    match note_manager::mark_note_deleted(note_id, user, false).await {
        Ok(res) => Ok(Redirect::to(format!("/notes/{}", res))),
        Err(error) => {
            println!("Error: {}", error);
            Ok(Redirect::to("/"))
        }
    }
}

#[get("/DenyNoteRemoval/<note_id>")]
async fn deny_note_removal(note_id: i64, user: User) -> Result<Redirect, Error> {
    match note_manager::deny_note_removal_request(note_id, user).await {
        Ok(res) => Ok(Redirect::to(format!("/review/{}", res))),
        Err(error) => {
            println!("Error: {}", error);
            Ok(Redirect::to("/"))
        }
    }
}

#[get("/notes/<deck_hash>")]
async fn get_notes_from_deck(
    deck_hash: String,
    user: Option<User>,
) -> Return<content::RawHtml<String>> {
    let mut context = tera::Context::new();

    // let deck_name = decks::get_name_by_hash(&deck_hash).await;
    // if deck_name.is_err() {
    //     return content::RawHtml(format!("Deck not found."))
    // }

    let notes = note_manager::retrieve_notes(&deck_hash).await?;

    let client = database::client().await?;
    let deck_info = client.query("Select id, name, description, human_hash, owner, TO_CHAR(last_update, 'MM/DD/YYYY') AS last_update from decks where human_hash = $1 Limit 1", &[&deck_hash]).await.expect("Error preparing deck notes statement");
    if deck_info.is_empty() {
        return Err(error::Error::DeckNotFound);
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
        desc: ammonia::clean(deck_info[0].get(2)),
        hash: deck_info[0].get(3),
        last_update: deck_info[0].get(5),
        notes: 0,
        children: childr,
        subscriptions: 0,
    };

    context.insert("notes", &notes);
    context.insert("user", &user);
    context.insert("deck", &deck);

    let rendered_template = TEMPLATES
        .render("notes.html", &context)
        .expect("Failed to render template");

    // Return the rendered HTML as the response
    Ok(content::RawHtml(rendered_template))
}

#[get("/reviews")]
async fn all_reviews(user: Option<User>) -> Return<content::RawHtml<String>> {
    let user = check_login(user)?;
    let mut context = tera::Context::new();
    if let Some(user) = &user {
        let commits = match commit_manager::commits_review(user.id()).await {
            Ok(commits) => commits,
            Err(error) => return Ok(content::RawHtml(format!("Error: {}", error))),
        };

        context.insert("commits", &commits);
        //context.insert("notes", &notes);
        context.insert("user", &user);
    }

    let rendered_template = TEMPLATES
        .render("reviews.html", &context)
        .expect("Failed to render template");
    Ok(content::RawHtml(rendered_template))
}

#[get("/decks")]
async fn deck_overview(user: Option<User>) -> Return<content::RawHtml<String>> {
    let mut decks: Vec<DeckOverview> = vec![];
    let mut user_id: i32 = 1;
    if user.is_some() {
        user_id = user.as_ref().unwrap().id();
    }
    let client = database::client().await?;
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
            (SELECT COUNT(*) FROM subscriptions WHERE deck_id = decks.id) AS subs
        FROM decks 
        WHERE parent IS NULL and (private = false or owner = $1)
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
            desc: ammonia::clean(row.get(2)),
            hash: row.get(3),
            last_update: row.get(5),
            notes: note_manager::get_notes_count_in_deck(row.get(0))
                .await
                .unwrap(),
            children: vec![],
            subscriptions: row.get(6),
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
    let rendered_template = TEMPLATES
        .render("decks.html", &context)
        .expect("Failed to render template");

    // Return the rendered HTML as the response
    Ok(content::RawHtml(rendered_template))
}

#[get("/ManageDecks")]
async fn manage_decks(user: Option<User>) -> Return<content::RawHtml<String>> {
    let user = check_login(user)?;
    let mut decks: Vec<DeckOverview> = vec![];

    let client = database::client().await?;
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
            (SELECT COUNT(*) FROM subscriptions WHERE deck_id = decks.id) AS subs
        FROM decks 
        WHERE parent IS NULL and owner = $1
    ",
        )
        .await
        .expect("Error preparing decks overview statement");

    let mut context = tera::Context::new();
    if let Some(user) = &user {
        let rows = client
            .query(&stmt, &[&user.id()])
            .await
            .expect("Error executing decks overview statement");

        for row in rows {
            decks.push(DeckOverview {
                owner: row.get(4),
                id: row.get(0),
                name: row.get(1),
                desc: ammonia::clean(row.get(2)),
                hash: row.get(3),
                last_update: row.get(5),
                notes: note_manager::get_notes_count_in_deck(row.get(0))
                    .await
                    .unwrap(),
                children: vec![],
                subscriptions: row.get(6),
            });
        }

        let notetypes = match notetype_manager::get_notetype_overview(user).await {
            Ok(cl) => cl,
            Err(error) => return Ok(content::RawHtml(format!("Error: {}", error))),
        };

        context.insert("decks", &decks);
        context.insert("user", &user);
        context.insert("notetypes", &notetypes);
    }
    let rendered_template = TEMPLATES
        .render("manage_decks.html", &context)
        .expect("Failed to render template");

    Ok(content::RawHtml(rendered_template))
}

use rocket::data::{Limits, ToByteUnit};

#[rocket::main]
async fn main() {
    dotenvy::dotenv().expect(
        "Expected .env file in the root directory containing the database connection string",
    );
    let pool = database::establish_connection()
        .await
        .expect("Failed to establish database connection");
    database::TOKIO_POSTGRES_POOL
        .set(pool)
        .expect("Failed to store database connection pool in static variable");

    use tokio_postgres::NoTls;
    let (client, conn) = connect(
        &env::var("DATABASE_URL").expect("Expected DATABASE_URL to exist in the environment"),
        NoTls,
    )
    .await
    .unwrap();
    let client = sync::Arc::new(client);
    let users: Users = client.clone().into();

    tokio::spawn(async move {
        if let Err(e) = conn.await {
            eprintln!("TokioPostgresError: {}", e);
        }
    });

    let figment = rocket::Config::figment()
        .merge(("port", 1337))
        .merge((
            "secret_key",
            env::var("ROCKET_SECRET_KEY")
                .expect("Expected ROCKET_SECRET_KEY to exist in the environment"),
        ))
        .merge(("limits", Limits::new().limit("json", 10.mebibytes())));

    if let Err(err) = rocket::custom(figment)
        .mount("/", FileServer::from("src/templates/static/"))
        .mount(
            "/",
            rocket::routes![
                deck_overview,
                get_notes_from_deck,
                manage_decks,
                review_note,
                accept_note,
                deny_note,
                all_reviews,
                review_commit,
                approve_commit,
                deny_commit,
                accept_field,
                deny_field,
                accept_tag,
                deny_tag,
                edit_deck,
                post_edit_deck,
                delete_deck,
                delete_changelog,
                index,
                terms,
                privacy,
                get_login,
                post_login,
                logout,
                post_signup,
                get_signup,
                post_maintainers,
                show_maintainers,
                post_optional_tags,
                show_optional_tags,
                edit_notetype,
                post_edit_notetype,
                media_manager,
                post_media_manager,
                remove_note_from_deck,
                deny_note_removal,
                forward_donation,
                error_page
            ],
        )
        .register("/", catchers![internal_error, not_found, default])
        .manage(client)
        .manage(users)
        .launch()
        .await
    {
        println!("Rocket Rust couldn't take off successfully!");
        drop(err); // Drop initiates Rocket-formatted panic
    }
}
