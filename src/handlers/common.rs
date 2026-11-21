use std::sync::Arc;

use axum::{
    http::header,
    response::{Html, IntoResponse},
};

use crate::database::{self, AppState};
use crate::error::Error;
use crate::structs::Return;
use crate::suggestion_manager;
use crate::user::User;

pub(crate) type SharedConn = bb8_postgres::bb8::PooledConnection<
    'static,
    bb8_postgres::PostgresConnectionManager<tokio_postgres::NoTls>,
>;

pub(crate) fn check_login(user: Option<User>) -> Result<User, Error> {
    match user {
        Some(user) => Ok(user),
        None => Err(Error::Redirect("/login".to_string())),
    }
}

pub(crate) fn safe_next(raw: Option<&str>) -> String {
    let Some(value) = raw else {
        return "/".to_string();
    };

    let is_local_path = value.starts_with('/')
        && !value.starts_with("//")
        && !value.contains('\\')
        && value.len() <= 512
        && !value.chars().any(char::is_control);

    if is_local_path {
        value.to_string()
    } else {
        "/".to_string()
    }
}

pub(crate) fn redirect_with_cookie(location: &str, cookie: &str) -> axum::response::Response {
    let mut response = axum::response::Redirect::to(location).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        header::HeaderValue::from_str(cookie).unwrap(),
    );
    response
}

pub(crate) fn error_page(appstate: &Arc<AppState>, message: &str) -> Result<Html<String>, Error> {
    let mut context = tera::Context::new();
    context.insert("message", message);
    let rendered_template = appstate.tera.render("error.html", &context)?;
    Ok(Html(rendered_template))
}

/// Build a logout redirect that clears auth cookies and origin DOM storage.
pub(crate) fn clear_session_response(
    redirect_to: &str,
    exp_cookie: &str,
) -> axum::response::Response {
    let mut response = axum::response::Redirect::to(redirect_to).into_response();
    if let Ok(cookie_header) = header::HeaderValue::from_str(exp_cookie) {
        response
            .headers_mut()
            .insert(header::SET_COOKIE, cookie_header);
    }
    response.headers_mut().insert(
        header::HeaderName::from_static("clear-site-data"),
        header::HeaderValue::from_static("\"cookies\", \"storage\""),
    );
    response
}

pub(crate) async fn access_check(
    appstate: &Arc<AppState>,
    deck_id: i64,
    user: &User,
) -> Result<bool, Error> {
    let access = match suggestion_manager::is_authorized(appstate, user, deck_id).await {
        Ok(access) => access,
        Err(_error) => return Ok(false),
    };

    Ok(access)
}

pub(crate) async fn resolve_deck_id_by_hash(appstate: &Arc<AppState>, hash: &str) -> Return<i64> {
    let client = database::client(appstate).await?;
    let rows = client
        .query("SELECT id FROM decks WHERE human_hash = $1", &[&hash])
        .await?;
    if rows.is_empty() {
        return Ok(0);
    }
    Ok(rows[0].get(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_session_response_clears_cookies_and_storage() {
        let resp = clear_session_response("/login", "session=; Max-Age=0; Path=/");
        assert!(resp.status().is_redirection());
        assert_eq!(resp.headers().get(header::LOCATION).unwrap(), "/login");
        assert_eq!(
            resp.headers().get(header::SET_COOKIE).unwrap(),
            "session=; Max-Age=0; Path=/"
        );
        assert_eq!(
            resp.headers().get("clear-site-data").unwrap(),
            "\"cookies\", \"storage\""
        );
    }
}
