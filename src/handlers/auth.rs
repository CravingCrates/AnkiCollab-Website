use std::sync::Arc;

use axum::{
    extract::State,
    response::{Html, IntoResponse},
    Extension,
};
use axum_client_ip::ClientIp;
use serde::Deserialize;

use crate::database::AppState;
use crate::error::Error;
use crate::handlers::common::{
    check_login, clear_session_response, redirect_with_cookie, safe_next,
};
use crate::passkey;
use crate::user::{
    hash_user_id, notify_offsite_deletion, purge_deleted_account_data, Auth, ChangePasswordRequest,
    Credentials, SessionAge, User,
};

static DISCORD_WEBHOOK_URL: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    std::env::var("DISCORD_WEBHOOK_URL").expect("DISCORD_WEBHOOK_URL must be set")
});

static OFF_SITE_SECRET_SALT: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    std::env::var("OFF_SITE_SECRET_SALT").expect("OFF_SITE_SECRET_SALT must be set")
});

#[derive(Deserialize)]
pub(crate) struct NextQuery {
    #[serde(default)]
    next: Option<String>,
}

pub(crate) async fn get_login(
    State(appstate): State<Arc<AppState>>,
    axum::extract::Query(query): axum::extract::Query<NextQuery>,
) -> Result<impl IntoResponse, Error> {
    let mut context = tera::Context::new();
    context.insert("next", &safe_next(query.next.as_deref()));
    let rendered_template = appstate.tera.render("login.html", &context)?;
    Ok(Html(rendered_template))
}

pub(crate) async fn post_login(
    ClientIp(ip): ClientIp,
    Extension(auth): Extension<Arc<Auth>>,
    axum::Form(form): axum::Form<Credentials>,
) -> Result<impl IntoResponse, Error> {
    let destination = safe_next(form.next.as_deref());
    let res = auth.login(form, ip).await?;

    Ok(redirect_with_cookie(&destination, &res))
}

pub(crate) async fn post_signup(
    ClientIp(ip): ClientIp,
    Extension(auth): Extension<Arc<Auth>>,
    axum::Form(form): axum::Form<Credentials>,
) -> Result<impl IntoResponse, Error> {
    let user = auth.signup(form, ip).await?;
    let cookie = auth.complete_login(user.id(), ip, false).await?;
    Ok(redirect_with_cookie("/welcome", &cookie))
}

pub(crate) async fn get_signup(
    State(appstate): State<Arc<AppState>>,
) -> Result<impl IntoResponse, Error> {
    let context = tera::Context::new();
    let rendered_template = appstate.tera.render("signup.html", &context)?;
    Ok(Html(rendered_template))
}

pub(crate) async fn get_welcome(
    State(appstate): State<Arc<AppState>>,
    session: Option<SessionAge>,
    user: Option<User>,
) -> Result<axum::response::Response, Error> {
    let user = check_login(user)?;
    if passkey::passkey_count(&appstate, user.id()).await? > 0 {
        return Ok(axum::response::Redirect::to("/").into_response());
    }

    let show_passkey_password = !session.is_some_and(|age| age.is_within(passkey::STEP_UP_WINDOW));

    let mut context = tera::Context::new();
    context.insert("user", &user);
    context.insert("show_passkey_password", &show_passkey_password);
    let rendered_template = appstate.tera.render("welcome_passkey.html", &context)?;
    Ok(Html(rendered_template).into_response())
}

pub(crate) async fn logout(
    Extension(auth): Extension<Arc<Auth>>,
) -> Result<impl IntoResponse, Error> {
    let exp_cookie = auth.logout();
    Ok(clear_session_response("/", &exp_cookie))
}

pub(crate) async fn get_profile(
    State(appstate): State<Arc<AppState>>,
    user: Option<User>,
) -> Result<impl IntoResponse, Error> {
    let user = check_login(user)?;
    let mut context = tera::Context::new();
    context.insert("user", &user);
    context.insert(
        "passkeys",
        &passkey::passkeys_for_profile(&appstate, user.id()).await?,
    );
    let rendered_template = appstate.tera.render("profile.html", &context)?;
    Ok(Html(rendered_template))
}

pub(crate) async fn post_change_password(
    Extension(auth): Extension<Arc<Auth>>,
    user: Option<User>,
    axum::Form(form): axum::Form<ChangePasswordRequest>,
) -> Result<impl IntoResponse, Error> {
    let user = check_login(user)?;

    // Verify passwords match
    if form.new_password != form.confirm_password {
        return Err(Error::BadRequest("Passwords do not match".to_string()));
    }

    // Change password (this validates the current password and new password strength)
    auth.change_password(user.id, &form.current_password, &form.new_password)
        .await?;

    // Log the user out so they need to re-authenticate with new password
    let exp_cookie = auth.logout();
    Ok(clear_session_response("/login", &exp_cookie))
}

pub(crate) async fn delete_account(
    State(appstate): State<Arc<AppState>>,
    Extension(auth): Extension<Arc<Auth>>,
    user: Option<User>,
) -> Result<impl IntoResponse, Error> {
    let user = check_login(user)?;

    // Fast soft-delete: invalidates password + sets deleted_at (instant)
    let username = auth.soft_delete_account(user.id).await?;

    let hashed_id = hash_user_id(user.id, &OFF_SITE_SECRET_SALT);

    tokio::spawn(async move {
        notify_offsite_deletion(&DISCORD_WEBHOOK_URL, hashed_id).await;
    });

    // Spawn the heavy cleanup work in the background so the user isn't waiting
    let pool = appstate.db_pool.clone();
    let user_id = user.id;
    tokio::spawn(async move {
        match pool.get().await {
            Ok(conn) => {
                purge_deleted_account_data(&conn, user_id, &username).await;
            }
            Err(e) => {
                tracing::error!(user_id, error = %e, "Failed to get DB connection for account purge");
            }
        }
    });

    // Log the user out immediately
    let exp_cookie = auth.logout();
    Ok(clear_session_response("/", &exp_cookie))
}
