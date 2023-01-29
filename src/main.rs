#[macro_use]
extern crate rocket;

#[macro_use(lazy_static)]
extern crate lazy_static;

extern crate html5ever;
extern crate ammonia;

pub mod database;
pub mod structs;
pub mod decks;

use rocket::*;
use rocket::fs::FileServer;
use rocket::response::content;
use rocket::{form::*, get, post, response::Redirect};
use rocket::serde::json::Json;
use rocket::Request;
use rocket::http::Status;

use rocket_auth::{Users, Error, Auth, Signup, Login, User};

use structs::*;
use tera::Tera;

use std::*;
use std::{result::Result};

use tokio_postgres::{connect};

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

#[catch(500)]
fn internal_error() -> &'static str {
    "Whoops! Looks like we messed up."
}

#[catch(404)]
fn not_found(req: &Request) -> String {
    format!("I couldn't find '{}'. Try something else?", req.uri())
}

#[catch(default)]
fn default(status: Status, req: &Request) -> String {
    format!("{} ({})", status, req.uri())
}

#[get("/login")]
fn get_login() -> content::RawHtml<String> {
    let context = tera::Context::new();
    let rendered_template = TEMPLATES.render("login.html", &context).expect("Failed to render template");
    content::RawHtml(rendered_template)
}

#[post("/login", data = "<form>")]
async fn post_login(auth: Auth<'_>, form: Form<Login>) -> Result<Redirect, Error> {
    let result = auth.login(&form).await;
    result?;
    Ok(Redirect::to("/"))
}

#[post("/signup", data = "<form>")]
async fn post_signup(auth: Auth<'_>, form: Form<Signup>) -> Result<Redirect, Error> {
    auth.signup(&form).await?;
    auth.login(&form.into()).await?;

    Ok(Redirect::to("/"))
}

#[get("/signup")]
async fn get_signup() -> content::RawHtml<String> {
    let context = tera::Context::new();
    let rendered_template = TEMPLATES.render("signup.html", &context).expect("Failed to render template");
    content::RawHtml(rendered_template)
}

#[get("/")]
async fn index(user: Option<User>) -> content::RawHtml<String> {
    let mut context = tera::Context::new();
    context.insert("user", &user);
    let rendered_template = TEMPLATES.render("index.html", &context).expect("Failed to render template");
    content::RawHtml(rendered_template)
}
#[get("/terms")]
async fn terms() -> content::RawHtml<String> {
    let context = tera::Context::new();
    let rendered_template = TEMPLATES.render("terms.html", &context).expect("Failed to render template");
    content::RawHtml(rendered_template)
}
#[get("/privacy")]
async fn privacy() -> content::RawHtml<String> {
    let context = tera::Context::new();
    let rendered_template = TEMPLATES.render("privacy.html", &context).expect("Failed to render template");
    content::RawHtml(rendered_template)
}

#[get("/logout")]
fn logout(auth: Auth<'_>) -> Result<content::RawHtml<String>, Error> {
    auth.logout()?;
    
    let context = tera::Context::new();
    let rendered_template = TEMPLATES.render("logout.html", &context).expect("Failed to render template");
    Ok(content::RawHtml(rendered_template))
}

#[get("/protected_fields")]
async fn get_prot_fields(user: User) -> content::RawHtml<String> {
    let notemodels = match decks::get_note_model_info(user.id()).await {
        Ok(note) => note,
        Err(error) => return content::RawHtml(format!("Error: {}", error)),
    };

    let mut context = tera::Context::new();
    context.insert("notetypes", &notemodels);    
    context.insert("user", &user);
    let rendered_template = TEMPLATES.render("protected_fields.html", &context).expect("Failed to render template");
    content::RawHtml(rendered_template)
}

#[post("/protected_fields", format = "application/json", data = "<ProtectedFieldsData>")]
async fn post_prot_fields(user: User, ProtectedFieldsData: Json<structs::ProtectedFieldsData>) -> Result<Redirect, Error> {

    let client = unsafe { database::TOKIO_POSTGRES_CLIENT.as_mut().unwrap() };
    let data = ProtectedFieldsData.into_inner();
    for (field_id, checked) in data.items.iter() {
        client.query("
        UPDATE notetype_field 
        SET protected = $1 
        WHERE id = $2 
        AND notetype IN (
            SELECT notetype 
            FROM notes 
            WHERE deck IN (
                SELECT id 
                FROM decks 
                WHERE owner = $3
            )
        )", &[&checked, &field_id, &user.id()]).await?;
    }
    return Ok(Redirect::to("/"));
}

#[get("/review/<note_id>")]
async fn review_note(note_id: i64, user: Option<User>) -> content::RawHtml<String> {
    
    let mut context = tera::Context::new();

    let note = match decks::get_note_data(note_id).await {
        Ok(note) => note,
        Err(error) => return content::RawHtml(format!("Error: {}", error)),
    };
    if note.id == 0 { // Invalid data // No note found!
        return content::RawHtml(format!("Error: Note not found."))
    }
    context.insert("note", &note);
    context.insert("user", &user);
    let rendered_template = TEMPLATES.render("review.html", &context).expect("Failed to render template");

    // Return the rendered HTML as the response
    content::RawHtml(rendered_template)
}

#[get("/DenyTag/<tag_id>")]
async fn deny_tag(tag_id: i64, user: User) -> Result<Redirect, Error> {    
    match decks::deny_tag_change(tag_id, user).await {
        Ok(res) => return Ok(Redirect::to(format!("/review/{}", res))),
        Err(error) => {println!("Error: {}", error); return Ok(Redirect::to("/")) },
    };
}

#[get("/AcceptTag/<tag_id>")]
async fn accept_tag(tag_id: i64, user: User) -> Result<Redirect, Error> {    
    match decks::approve_tag_change(tag_id, user).await {
        Ok(res) => return Ok(Redirect::to(format!("/review/{}", res))),
        Err(error) => {println!("Error: {}", error); return Ok(Redirect::to("/")) },
    };    
}

#[get("/DenyField/<field_id>")]
async fn deny_field(field_id: i64, user: User) -> Result<Redirect, Error> {    
    match decks::deny_field_change(field_id, user).await {
        Ok(res) => return Ok(Redirect::to(format!("/review/{}", res))),
        Err(error) => {println!("Error: {}", error); return Ok(Redirect::to("/")) },
    };
}

#[get("/AcceptField/<field_id>")]
async fn accept_field(field_id: i64, user: User) -> Result<Redirect, Error> {    
    match decks::approve_field_change(field_id, user).await {
        Ok(res) => return Ok(Redirect::to(format!("/review/{}", res))),
        Err(error) => {println!("Error: {}", error); return Ok(Redirect::to("/")) },
    };    
}

#[get("/AcceptNote/<note_id>")]
async fn accept_note(note_id: i64, user: User) -> Result<Redirect, Error> {
    match decks::approve_card(note_id, user).await {
        Ok(res) => return Ok(Redirect::to(format!("/review/{}", res))),
        Err(error) => {println!("Error: {}", error); return Ok(Redirect::to("/")) },
    };    
}

#[get("/DeleteNote/<note_id>")]
async fn deny_note(note_id: i64, user: User) -> Result<Redirect, Error> {    
    match decks::delete_card(note_id, user).await {
        Ok(res) => return Ok(Redirect::to(format!("/notes/{}", res))),
        Err(error) => {println!("Error: {}", error); return Ok(Redirect::to("/")) },
    };    
}

#[get("/notes/<deck_hash>")]
async fn get_notes_from_deck(deck_hash: String, user: Option<User>) -> content::RawHtml<String> {
    
    let mut context = tera::Context::new();

    // let deck_name = decks::get_name_by_hash(&deck_hash).await;
    // if deck_name.is_err() {
    //     return content::RawHtml(format!("Deck not found."))
    // }

    let notes = match decks::retrieve_notes(&deck_hash).await {
        Ok(notes) => notes,
        Err(error) => return content::RawHtml(format!("Error: {}", error)),
    };

    let client = unsafe { database::TOKIO_POSTGRES_CLIENT.as_mut().unwrap() };
    let deck_info = client.query("Select id, name, description, human_hash, owner, TO_CHAR(last_update, 'MM/DD/YYYY') AS last_update from decks where human_hash = $1 Limit 1", &[&deck_hash]).await.expect("Error preparing statement");
    if deck_info.is_empty() {
        return content::RawHtml(format!("Deck not found."))
    }

    let id: i64 = deck_info[0].get(0);

    let children_rows = client.query("Select name, human_hash from decks where parent = $1", &[&id]).await.expect("Error getting children from decks");
    let mut childr = vec![];
    for row in children_rows {
        childr.push( BasicDeckInfo {
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
    };

    context.insert("notes", &notes);
    context.insert("user", &user);    
    context.insert("deck", &deck);
    
    let rendered_template = TEMPLATES.render("notes.html", &context).expect("Failed to render template");

    // Return the rendered HTML as the response
    content::RawHtml(rendered_template)
}

#[get("/reviews")]
async fn all_reviews(user: User) -> content::RawHtml<String> {
    let mut context = tera::Context::new();

    let notes = match decks::under_review(user.id()).await {
        Ok(notes) => notes,
        Err(error) => return content::RawHtml(format!("Error: {}", error)),
    };

    context.insert("notes", &notes);
    context.insert("user", &user);
    
    let rendered_template = TEMPLATES.render("reviews.html", &context).expect("Failed to render template");
    content::RawHtml(rendered_template)
}

#[get("/commits")]
async fn commit_reviews(user: User) -> content::RawHtml<String> {
    let mut context = tera::Context::new();

    let commits = match decks::commits_review(user.id()).await {
        Ok(commits) => commits,
        Err(error) => return content::RawHtml(format!("Error: {}", error)),
    };

    context.insert("commits", &commits);
    context.insert("user", &user);
    
    let rendered_template = TEMPLATES.render("commits.html", &context).expect("Failed to render template");
    content::RawHtml(rendered_template)
}

#[get("/decks")]
async fn deck_overview(user: Option<User>) -> content::RawHtml<String> {
    let mut decks:Vec<DeckOverview> = vec![];
    let mut user_id: i32 = 1; // Dummy
    if user.is_some() {
        user_id = user.as_ref().unwrap().id();
    }
    let client = unsafe { database::TOKIO_POSTGRES_CLIENT.as_mut().unwrap() };
    let stmt = client
                .prepare("Select id, name, description, human_hash, owner, TO_CHAR(last_update, 'MM/DD/YYYY') AS last_update from decks where parent is NULL ORDER BY id")
                .await.expect("Error preparing statement");

    let rows = client
                .query(&stmt, &[])
                .await.expect("Error executing statement");

    for row in rows {
        decks.push(DeckOverview {
            owner: row.get(4),
            id: row.get(0),
            name: row.get(1),
            desc: ammonia::clean(row.get(2)),
            hash: row.get(3),
            last_update: row.get(5),
            notes: decks::get_notes_count_in_deck(row.get(0)).await.unwrap(),
            children: vec![],
        });
    }

    decks.sort_by(|a, b| {
        if a.owner == user_id {
            return std::cmp::Ordering::Less;
        } else if b.owner == user_id {
            return std::cmp::Ordering::Greater;
        }
    
        b.notes.cmp(&a.notes)
    });

    let mut context = tera::Context::new();
    context.insert("decks", &decks);
    context.insert("user", &user);
    let rendered_template = TEMPLATES.render("decks.html", &context).expect("Failed to render template");

    // Return the rendered HTML as the response
    content::RawHtml(rendered_template)
}

use rocket::data::{Limits, ToByteUnit};

#[rocket::main]
async fn main() {
    database::establish_connection().await.unwrap();

    use tokio_postgres::NoTls;
    let (client, conn) = connect("postgresql://postgres:password@localhost/anki", NoTls).await.unwrap();
    let client = sync::Arc::new(client);
    let users: Users = client.clone().into();

    tokio::spawn(async move {
        if let Err(e) = conn.await {
            eprintln!("TokioPostgresError: {}", e);
        }
    });
    
    let figment = rocket::Config::figment()
        .merge(("port", 1337))
        .merge(("secret_key", "RETRACTED"))
        .merge(("limits", Limits::new().limit("json", 10.mebibytes())));

    if let Err(err) = rocket::custom(figment)
        .mount("/", FileServer::from("src/templates/static/"))
        .mount("/", rocket::routes![
                deck_overview, get_notes_from_deck,
                review_note, accept_note, deny_note, all_reviews,
                commit_reviews,
                accept_field, deny_field, accept_tag, deny_tag,
                get_prot_fields, post_prot_fields,
                index, terms, privacy,
                get_login, post_login, logout,
                post_signup, get_signup
        ])
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
