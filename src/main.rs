extern crate rocket;

#[macro_use(lazy_static)]
extern crate lazy_static;

extern crate ammonia;
extern crate html5ever;

pub mod changelog_manager;
pub mod commit_manager;
pub mod database;
pub mod gdrive_manager;
pub mod maintainer_manager;
pub mod note_manager;
pub mod notetype_manager;
pub mod optional_tags_manager;
pub mod structs;
pub mod suggestion_manager;

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

fn is_user_logged_in(user: Option<User>) -> Result<Option<User>, Status> {
    match user {
        Some(user) => Ok(Some(user)),
        None => Err(Status::SeeOther),
    }
}
fn handle_login_check(user: Option<User>) -> Result<Option<User>, Redirect> {
    match is_user_logged_in(user) {
        Ok(user) => Ok(user),
        Err(_redirect_status) => Err(Redirect::to("/login")),
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
async fn error_page(message: String) -> content::RawHtml<String> {
    let mut context = tera::Context::new();
    context.insert("message", &message);
    let rendered_template = TEMPLATES
        .render("error.html", &context)
        .expect("Failed to render template");
    content::RawHtml(rendered_template)
}

#[get("/signup")]
async fn get_signup() -> content::RawHtml<String> {
    let context = tera::Context::new();
    let rendered_template = TEMPLATES
        .render("signup.html", &context)
        .expect("Failed to render template");
    content::RawHtml(rendered_template)
}

#[get("/")]
async fn index(user: Option<User>) -> content::RawHtml<String> {
    let mut context = tera::Context::new();
    context.insert("user", &user);
    let rendered_template = TEMPLATES
        .render("index.html", &context)
        .expect("Failed to render template");
    content::RawHtml(rendered_template)
}
#[get("/terms")]
async fn terms() -> content::RawHtml<String> {
    let context = tera::Context::new();
    let rendered_template = TEMPLATES
        .render("terms.html", &context)
        .expect("Failed to render template");
    content::RawHtml(rendered_template)
}
#[get("/privacy")]
async fn privacy() -> content::RawHtml<String> {
    let context = tera::Context::new();
    let rendered_template = TEMPLATES
        .render("privacy.html", &context)
        .expect("Failed to render template");
    content::RawHtml(rendered_template)
}

#[get("/logout")]
fn logout(auth: Auth<'_>) -> Result<content::RawHtml<String>, Error> {
    auth.logout()?;

    let context = tera::Context::new();
    let rendered_template = TEMPLATES
        .render("logout.html", &context)
        .expect("Failed to render template");
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
            return content::RawHtml(format!("Error retrieving optional tags. Please notify us."));
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
) -> String {
    let client = database::client().await;
    let data = edit_optional_tag.into_inner();

    let owned_info = client
        .query(
            "Select id from decks where human_hash = $1 and owner = $2",
            &[&data.deck, &user.id()],
        )
        .await
        .expect("Error preparing edit deck statement");
    if owned_info.is_empty() {
        return "Unauthorized.".into();
    }
    let deck_id: i64 = owned_info[0].get(0);

    // Add new tag
    if data.action == 1 {
        match optional_tags_manager::add_tag(deck_id, data.taggroup).await {
            Ok(_res) => "added".to_owned(),
            Err(e) => e.to_string(),
        }
    } else {
        // Delete existing optional_tag
        match optional_tags_manager::remove_tag(deck_id, data.taggroup).await {
            Ok(_res) => "removed".to_owned(),
            Err(e) => e.to_string(),
        }
    }
}

#[get("/OptionalTags/<deck_hash>")]
async fn show_optional_tags(user: User, deck_hash: String) -> content::RawHtml<String> {
    let client = database::client().await;

    let owned_info = client
        .query(
            "Select id from decks where human_hash = $1 and owner = $2",
            &[&deck_hash, &user.id()],
        )
        .await
        .expect("Error preparing edit deck statement");
    if owned_info.is_empty() {
        return content::RawHtml(format!("Unauthorized."));
    }
    let deck_id: i64 = owned_info[0].get(0);
    render_optional_tags(&deck_hash, deck_id, user).await
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
            return content::RawHtml(format!("Error getting maintainers."));
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
async fn post_maintainers(user: User, edit_maintainer: Json<structs::UpdateMaintainer>) -> String {
    let client = database::client().await;
    let data = edit_maintainer.into_inner();

    let owned_info = client
        .query(
            "Select id from decks where human_hash = $1 and owner = $2",
            &[&data.deck, &user.id()],
        )
        .await
        .expect("Error preparing edit deck statement");
    if owned_info.is_empty() {
        return "Unauthorized.".into();
    }
    let deck_id: i64 = owned_info[0].get(0);

    // Add new maintainer
    if data.action == 1 {
        match maintainer_manager::add_maintainer(deck_id, data.email).await {
            Ok(_res) => "added".to_owned(),
            Err(e) => e.to_string(),
        }
    } else {
        // Delete existing maintainer
        match maintainer_manager::remove_maintainer(deck_id, data.email).await {
            Ok(_res) => "removed".to_owned(),
            Err(e) => e.to_string(),
        }
    }
}

fn translate_error(e: Box<dyn std::error::Error>) -> String {
    if e.to_string() == "db error: ERROR: value too long for type character varying(33)" {
        return String::from("Your folder ID is too long. Please double check it and try again.");
    } else {
        return e.to_string();
    }
}

#[post("/MediaManager", format = "application/json", data = "<update_media>")]
async fn post_media_manager(user: User, update_media: Json<structs::GDriveInfo>) -> String {
    let client = database::client().await;
    let data = update_media.into_inner();

    let owned_info = client
        .query(
            "Select id from decks where human_hash = $1 and owner = $2",
            &[&data.deck, &user.id()],
        )
        .await
        .expect("Error preparing edit deck statement");
    if owned_info.is_empty() {
        return "Unauthorized.".into();
    }
    let deck_id: i64 = owned_info[0].get(0);

    let status = match gdrive_manager::update_media(deck_id, data).await {
        Ok(res) => res,
        Err(e) => {
            println!("Error: {}", e);
            translate_error(e)
        }
    };
    return status;
}

#[get("/MediaManager/<deck_hash>")]
async fn media_manager(user: User, deck_hash: String) -> content::RawHtml<String> {
    let client = database::client().await;

    let owned_info = client
        .query(
            "Select id from decks where human_hash = $1 and owner = $2",
            &[&deck_hash, &user.id()],
        )
        .await
        .expect("Error preparing edit deck statement");
    if owned_info.is_empty() {
        return content::RawHtml(format!("Unauthorized."));
    }

    let mut context = tera::Context::new();
    context.insert("hash", &deck_hash);
    context.insert("user", &user);

    let rendered_template = TEMPLATES
        .render("media_manager.html", &context)
        .expect("Failed to render template");
    content::RawHtml(rendered_template)
}

#[get("/Maintainers/<deck_hash>")]
async fn show_maintainers(user: User, deck_hash: String) -> content::RawHtml<String> {
    let client = database::client().await;

    let owned_info = client
        .query(
            "Select id from decks where human_hash = $1 and owner = $2",
            &[&deck_hash, &user.id()],
        )
        .await
        .expect("Error preparing edit deck statement");
    if owned_info.is_empty() {
        return content::RawHtml(format!("Unauthorized."));
    }
    let deck_id: i64 = owned_info[0].get(0);
    render_maintainers(&deck_hash, deck_id, user).await
}

#[get("/EditNotetype/<notetype_id>")]
async fn edit_notetype(user: User, notetype_id: i64) -> content::RawHtml<String> {
    let client = database::client().await;

    let owned_info = client
        .query(
            "SELECT 1 FROM notetype WHERE (owner = $1 AND id = $3) OR $2 LIMIT 1",
            &[&user.id(), &user.is_admin, &notetype_id],
        )
        .await
        .expect("Error preparing edit notetype statement");
    if owned_info.is_empty() {
        return content::RawHtml(format!("Unauthorized."));
    }

    let notetype_info = client
        .query(
            "Select name, css from notetype where id = $1",
            &[&notetype_id],
        )
        .await
        .expect("Error preparing edit notetype statement");
    let notetype_template_info = client.query("Select id, qfmt, afmt from notetype_template where notetype = $1 and position = 0 LIMIT 1", &[&notetype_id]).await.expect("Error preparing edit notetype statement");

    let protected_fields = match notetype_manager::get_protected_fields(notetype_id).await {
        Ok(note) => note,
        Err(error) => return content::RawHtml(format!("Error: {}", error)),
    };

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
    content::RawHtml(rendered_template)
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
async fn edit_deck(
    user: Option<User>,
    deck_hash: String,
) -> Result<content::RawHtml<String>, Redirect> {
    let user = handle_login_check(user)?;
    let client = database::client().await;
    let owned_info = client
        .query(
            "Select owner, description, private, id from decks where human_hash = $1",
            &[&deck_hash],
        )
        .await
        .expect("Error preparing edit deck statement");
    if owned_info.is_empty() {
        return Ok(content::RawHtml(format!("Deck not found.")));
    }
    let owner: i32 = owned_info[0].get(0);

    let mut context = tera::Context::new();
    if let Some(user) = &user {
        if owner != user.id() {
            return Ok(content::RawHtml(format!("Unauthorized.")));
        }

        let desc: String = owned_info[0].get(1);
        let is_private: bool = owned_info[0].get(2);

        let changelogs = match changelog_manager::get_changelogs(&deck_hash).await {
            Ok(cl) => cl,
            Err(error) => return Ok(content::RawHtml(format!("Error: {}", error))),
        };

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
) -> Result<Redirect, Error> {
    let client = database::client().await;
    let data = edit_deck_data.into_inner();

    let owned_info = client
        .query(
            "Select id from decks where human_hash = $1 and owner = $2",
            &[&data.hash, &user.id()],
        )
        .await
        .expect("Error preparing edit deck statement");
    if owned_info.is_empty() {
        return Ok(Redirect::to("/"));
    }

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

    if data.changelog != "" {
        changelog_manager::insert_new_changelog(&data.hash, &data.changelog)
            .await
            .expect("Failed to insert new changelog");
    }

    return Ok(Redirect::to(format!("/EditDeck/{}", data.hash)));
}

#[get("/DeleteChangelog/<changelog_id>")]
async fn delete_changelog(user: User, changelog_id: i64) -> Result<Redirect, Error> {
    match changelog_manager::delete_changelog(changelog_id, user.id()).await {
        Ok(hash) => return Ok(Redirect::to(format!("/EditDeck/{}", hash))),
        Err(_err) => return Ok(Redirect::to("/")),
    }
}

#[get("/DeleteDeck/<deck_hash>")]
async fn delete_deck(user: User, deck_hash: String) -> Result<Redirect, Error> {
    let client = database::client().await;
    let owned_info = client
        .query(
            "Select owner from decks where human_hash = $1 and owner = $2",
            &[&deck_hash, &user.id()],
        )
        .await
        .expect("Error preparing deck deletion statement");
    if owned_info.is_empty() {
        return Ok(Redirect::to("/"));
    }

    client
        .query("Select delete_deck($1)", &[&deck_hash])
        .await
        .expect("Error deleting deck");

    Ok(Redirect::to("/"))
}

#[get("/AsyncApproveCommit/<commit_id>")]
async fn async_approve_commit(commit_id: i32, user: User) -> Result<Redirect, Error> {
    tokio::spawn(async move {
        match suggestion_manager::merge_by_commit(commit_id, true, user).await {
            Ok(_res) => println!("Async approved commit {}", commit_id),
            Err(error) => println!("Async approve commit Error: {}", error),
        };
    });
    Ok(Redirect::to("/"))
}

#[get("/ApproveCommit/<commit_id>")]
async fn approve_commit(commit_id: i32, user: User) -> Result<Redirect, Error> {
    match suggestion_manager::merge_by_commit(commit_id, true, user).await {
        Ok(res) => {
            if res.is_none() {
                return Ok(Redirect::to(format!("/reviews")));
            } else {
                return Ok(Redirect::to(format!("/commit/{}", res.unwrap())));
            }
        }
        Err(error) => {
            println!("Error: {}", error);
            return Ok(Redirect::to("/"));
        }
    };
}

#[get("/DenyCommit/<commit_id>")]
async fn deny_commit(commit_id: i32, user: User) -> Result<Redirect, Error> {
    match suggestion_manager::merge_by_commit(commit_id, false, user).await {
        Ok(res) => {
            if res.is_none() {
                return Ok(Redirect::to(format!("/reviews")));
            } else {
                return Ok(Redirect::to(format!("/commit/{}", res.unwrap())));
            }
        }
        Err(error) => {
            println!("Error: {}", error);
            return Ok(Redirect::to("/"));
        }
    };
}

#[get("/commit/<commit_id>")]
async fn review_commit(commit_id: i32, user: User) -> content::RawHtml<String> {
    let mut context = tera::Context::new();

    let notes = match commit_manager::notes_by_commit(commit_id).await {
        Ok(notes) => notes,
        Err(error) => return content::RawHtml(format!("Error: {}", error)),
    };

    let commit = match commit_manager::get_commit_info(commit_id).await {
        Ok(commit) => commit,
        Err(error) => return content::RawHtml(format!("Error: {}", error)),
    };

    let client = database::client().await;
    let q_guid = match client
        .query(
            "Select deck from commits where commit_id = $1",
            &[&commit_id],
        )
        .await
    {
        Ok(q_guid) => q_guid,
        Err(error) => return content::RawHtml(format!("Error: {}", error)),
    };
    if q_guid.is_empty() {
        return content::RawHtml("Error: Commit not found".into());
    }
    let deck_id: i64 = q_guid[0].get(0);

    let access = match suggestion_manager::is_authorized(&user, deck_id).await {
        Ok(access) => access,
        Err(error) => return content::RawHtml(format!("Error: {}", error)),
    };

    let notemodels = match notetype_manager::notetypes_by_commit(commit_id).await {
        Ok(notemodels) => notemodels,
        Err(error) => return content::RawHtml(format!("Error: {}", error)),
    };

    context.insert("notes", &notes);
    context.insert("commit", &commit);
    context.insert("user", &user);
    context.insert("owned", &access);
    context.insert("notemodels", &notemodels);

    let rendered_template = TEMPLATES
        .render("commit.html", &context)
        .expect("Failed to render template");

    // Return the rendered HTML as the response
    content::RawHtml(rendered_template)
}

#[get("/review/<note_id>")]
async fn review_note(note_id: i64, user: Option<User>) -> content::RawHtml<String> {
    let mut context = tera::Context::new();

    let note = match note_manager::get_note_data(note_id).await {
        Ok(note) => note,
        Err(error) => return content::RawHtml(format!("Error: {}", error)),
    };
    if note.id == 0 {
        // Invalid data // No note found!
        return content::RawHtml(format!("Error: Note not found."));
    }

    let mut access = false;

    if let Some(ref user) = user {
        let client = database::TOKIO_POSTGRES_POOL
            .get()
            .unwrap()
            .get()
            .await
            .unwrap();
        let q_guid = match client
            .query("Select deck from notes where id = $1", &[&note_id])
            .await
        {
            Ok(q_guid) => q_guid,
            Err(error) => return content::RawHtml(format!("Error: {}", error)),
        };
        if q_guid.is_empty() {
            return content::RawHtml("Error: Note not found".into());
        }
        let deck_id: i64 = q_guid[0].get(0);

        access = match suggestion_manager::is_authorized(&user, deck_id).await {
            Ok(access) => access,
            Err(error) => return content::RawHtml(format!("Error: {}", error)),
        };
    }

    context.insert("note", &note);
    context.insert("access", &access);
    context.insert("user", &user);
    let rendered_template = TEMPLATES
        .render("review.html", &context)
        .expect("Failed to render template");

    // Return the rendered HTML as the response
    content::RawHtml(rendered_template)
}

async fn access_check(deck_id: i64, user: &User) -> Result<bool, Error> {
    let access = match suggestion_manager::is_authorized(&user, deck_id).await {
        Ok(access) => access,
        Err(_error) => return Ok(false),
    };

    if !access {
        return Ok(false);
    }

    Ok(true)
}

async fn get_deck_by_tag_id(tag_id: i64) -> Result<i64, Error> {
    let client = database::client().await;
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

async fn get_deck_by_field_id(field_id: i64) -> Result<i64, Error> {
    let client = database::client().await;
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
    let deck_id: i64 = q_guid[0].get(0);

    Ok(deck_id)
}

#[get("/DenyTag/<tag_id>")]
async fn deny_tag(tag_id: i64, user: User) -> Result<Redirect, Error> {
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
        Ok(res) => return Ok(Redirect::to(format!("/review/{}", res))),
        Err(error) => {
            println!("Error: {}", error);
            return Ok(Redirect::to("/"));
        }
    };
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
        Ok(res) => return Ok(Redirect::to(format!("/review/{}", res))),
        Err(error) => {
            println!("Error: {}", error);
            return Ok(Redirect::to("/"));
        }
    };
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
        Ok(res) => return Ok(Redirect::to(format!("/review/{}", res))),
        Err(error) => {
            println!("Error: {}", error);
            return Ok(Redirect::to("/"));
        }
    };
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
        Ok(res) => return Ok(Redirect::to(format!("/review/{}", res))),
        Err(error) => {
            println!("Error: {}", error);
            return Ok(Redirect::to("/"));
        }
    };
}

#[get("/AcceptNote/<note_id>")]
async fn accept_note(note_id: i64, user: User) -> Result<Redirect, Error> {
    match suggestion_manager::approve_card(note_id, user, false).await {
        Ok(res) => return Ok(Redirect::to(format!("/review/{}", res))),
        Err(error) => {
            println!("Error: {}", error);
            return Ok(Redirect::to("/"));
        }
    };
}

// This actually removes the note from the database (Only used for notes that are not approved yet)
#[get("/DeleteNote/<note_id>")]
async fn deny_note(note_id: i64, user: User) -> Result<Redirect, Error> {
    match suggestion_manager::delete_card(note_id, user).await {
        Ok(res) => return Ok(Redirect::to(format!("/notes/{}", res))),
        Err(error) => {
            println!("Error: {}", error);
            return Ok(Redirect::to("/"));
        }
    };
}

// This marks the note as deleted, but does not remove them (Used for existing notes that are approved)
#[get("/AcceptNoteRemoval/<note_id>")]
async fn remove_note_from_deck(note_id: i64, user: User) -> Result<Redirect, Error> {
    match note_manager::mark_note_deleted(note_id, user, false).await {
        Ok(res) => return Ok(Redirect::to(format!("/notes/{}", res))),
        Err(error) => {
            println!("Error: {}", error);
            return Ok(Redirect::to("/"));
        }
    };
}

#[get("/DenyNoteRemoval/<note_id>")]
async fn deny_note_removal(note_id: i64, user: User) -> Result<Redirect, Error> {
    match note_manager::deny_note_removal_request(note_id, user).await {
        Ok(res) => return Ok(Redirect::to(format!("/review/{}", res))),
        Err(error) => {
            println!("Error: {}", error);
            return Ok(Redirect::to("/"));
        }
    };
}

#[get("/notes/<deck_hash>")]
async fn get_notes_from_deck(deck_hash: String, user: Option<User>) -> content::RawHtml<String> {
    let mut context = tera::Context::new();

    // let deck_name = decks::get_name_by_hash(&deck_hash).await;
    // if deck_name.is_err() {
    //     return content::RawHtml(format!("Deck not found."))
    // }

    let notes = match note_manager::retrieve_notes(&deck_hash).await {
        Ok(notes) => notes,
        Err(error) => return content::RawHtml(format!("Error: {}", error)),
    };

    let client = database::client().await;
    let deck_info = client.query("Select id, name, description, human_hash, owner, TO_CHAR(last_update, 'MM/DD/YYYY') AS last_update from decks where human_hash = $1 Limit 1", &[&deck_hash]).await.expect("Error preparing deck notes statement");
    if deck_info.is_empty() {
        return content::RawHtml(format!("Deck not found."));
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
        id: id,
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
    content::RawHtml(rendered_template)
}

#[get("/reviews")]
async fn all_reviews(user: Option<User>) -> Result<content::RawHtml<String>, Redirect> {
    let user = handle_login_check(user)?;
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
async fn deck_overview(user: Option<User>) -> content::RawHtml<String> {
    let mut decks: Vec<DeckOverview> = vec![];
    let mut user_id: i32 = 1;
    if user.is_some() {
        user_id = user.as_ref().unwrap().id();
    }
    let client = database::client().await;
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
    content::RawHtml(rendered_template)
}

#[get("/ManageDecks")]
async fn manage_decks(user: Option<User>) -> Result<content::RawHtml<String>, Redirect> {
    let user = handle_login_check(user)?;
    let mut decks: Vec<DeckOverview> = vec![];

    let client = database::client().await;
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

        let notetypes = match notetype_manager::get_notetype_overview(&user).await {
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
                async_approve_commit,
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
