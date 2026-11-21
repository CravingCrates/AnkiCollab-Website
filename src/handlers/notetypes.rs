use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse},
    Json,
};

use crate::database::{self, AppState};
use crate::error::{self, Error};
use crate::handlers::common::error_page;
use crate::notetype_manager;
use crate::structs::{UpdateNotetype, UpdateNotetypeTemplate};
use crate::user::User;

pub(crate) async fn edit_notetype(
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
        return error_page(&appstate, &error::Error::Unauthorized.to_string());
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

    let name: String = notetype_info[0]
        .get::<_, Option<String>>(0)
        .unwrap_or_default();
    let styling: String = notetype_info[0]
        .get::<_, Option<String>>(1)
        .unwrap_or_default();

    let mut templates: Vec<UpdateNotetypeTemplate> = Vec::new();
    for row in notetype_template_info {
        templates.push(UpdateNotetypeTemplate {
            front: row.get::<_, Option<String>>(1).unwrap_or_default(),
            back: row.get::<_, Option<String>>(2).unwrap_or_default(),
            template_id: row.get(0),
            name: row.get::<_, Option<String>>(3).unwrap_or_default(),
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

pub(crate) async fn post_edit_notetype(
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
