use std::sync::Arc;

use axum::{
    extract::State,
    response::{Html, IntoResponse, Redirect},
};

use crate::database::AppState;
use crate::error::Error;
use crate::passkey;
use crate::user::User;

pub(crate) async fn forward_donation() -> impl IntoResponse {
    Redirect::permanent("https://ankiweb.net/shared/review/1957538407")
}

pub(crate) async fn index(
    State(appstate): State<Arc<AppState>>,
    user: Option<User>,
) -> Result<impl IntoResponse, Error> {
    let mut context = tera::Context::new();

    let show_passkey_prompt = match &user {
        Some(signed_in) => passkey::passkey_count(&appstate, signed_in.id()).await? == 0,
        None => false,
    };

    context.insert("user", &user);
    context.insert("show_passkey_prompt", &show_passkey_prompt);
    let rendered_template = appstate.tera.render("index.html", &context)?;
    Ok(Html(rendered_template))
}

pub(crate) async fn terms(
    State(appstate): State<Arc<AppState>>,
) -> Result<impl IntoResponse, Error> {
    let context = tera::Context::new();
    let rendered_template = appstate.tera.render("terms.html", &context)?;
    Ok(Html(rendered_template))
}

pub(crate) async fn privacy(
    State(appstate): State<Arc<AppState>>,
) -> Result<impl IntoResponse, Error> {
    let context = tera::Context::new();
    let rendered_template = appstate.tera.render("privacy.html", &context)?;
    Ok(Html(rendered_template))
}

pub(crate) async fn imprint(
    State(appstate): State<Arc<AppState>>,
) -> Result<impl IntoResponse, Error> {
    let context = tera::Context::new();
    let rendered_template = appstate.tera.render("imprint.html", &context)?;
    Ok(Html(rendered_template))
}

pub(crate) async fn datenschutz(
    State(appstate): State<Arc<AppState>>,
) -> Result<impl IntoResponse, Error> {
    let context = tera::Context::new();
    let rendered_template = appstate.tera.render("datenschutz.html", &context)?;
    Ok(Html(rendered_template))
}

pub(crate) async fn accessibility_page(
    State(appstate): State<Arc<AppState>>,
) -> Result<impl IntoResponse, Error> {
    let context = tera::Context::new();
    let rendered_template = appstate
        .tera
        .render("accessibility_statement.html", &context)?;
    Ok(Html(rendered_template))
}
