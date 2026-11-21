use std::sync::Arc;

use axum::{
    extract::{Path, State},
    response::{Html, IntoResponse},
    Json,
};

use crate::database::{self, AppState};
use crate::error::{self, Error};
use crate::handlers::common::{access_check, check_login, error_page, resolve_deck_id_by_hash};
use crate::structs::{
    SubscriptionPolicyGetResponse, SubscriptionPolicyItem, SubscriptionPolicyPostRequest,
};
use crate::user::User;

pub(crate) async fn api_get_subscription_policy(
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

pub(crate) async fn api_post_subscription_policy(
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

                let vp_set: std::collections::HashSet<i32> =
                    valid_positions.iter().copied().collect();
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

pub(crate) async fn page_subscription_policy(
    State(appstate): State<Arc<AppState>>,
    user: Option<User>,
    Path((subscriber_hash, base_hash)): Path<(String, String)>,
) -> Result<impl IntoResponse, Error> {
    let user = check_login(user)?;
    // Authorization: must be owner/maintainer of subscriber deck
    let sub_id = resolve_deck_id_by_hash(&appstate, &subscriber_hash).await?;
    if sub_id == 0 || !access_check(&appstate, sub_id, &user).await? {
        return error_page(&appstate, &error::Error::Unauthorized.to_string())
            .map(IntoResponse::into_response);
    }
    // Ensure the subscription link exists
    let base_id = resolve_deck_id_by_hash(&appstate, &base_hash).await?;
    if base_id == 0 {
        return error_page(&appstate, "Base deck not found.").map(IntoResponse::into_response);
    }
    let client_check = database::client(&appstate).await?;
    let exists = client_check.query(
        "SELECT 1 FROM deck_subscriptions WHERE subscriber_deck_id = $1 AND base_deck_id = $2 LIMIT 1",
        &[&sub_id, &base_id]
    ).await?;
    if exists.is_empty() {
        return error_page(
            &appstate,
            "No deck subscription link exists for these decks.",
        )
        .map(IntoResponse::into_response);
    }

    // Build notetype metadata for the subtree
    let client = database::client(&appstate).await?;
    let nt_rows = client
        .query(
            r"
        WITH RECURSIVE subtree AS (
            SELECT id FROM decks WHERE human_hash = $1
            UNION ALL
            SELECT d.id FROM decks d JOIN subtree s ON d.parent = s.id
        )
        SELECT DISTINCT nt.id, nt.name
        FROM notes n
        JOIN notetype nt ON nt.id = n.notetype
        WHERE n.deck IN (SELECT id FROM subtree) AND n.deleted = false
        ",
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
