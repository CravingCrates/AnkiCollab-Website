use crate::error::Error::UserNotFound;
use crate::{database, Return};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DemocracyConfig {
    pub deck_id: i64,
    pub enabled: bool,
    pub mode: String,
    pub approval_threshold: i32,
    pub require_unanimity: bool,
    pub weekly_vote_limit: i32,
    pub min_days_subscribed: i32,
    pub min_cards_reviewed: i32,
    pub notifications_enabled: bool,
}

impl Default for DemocracyConfig {
    fn default() -> Self {
        Self {
            deck_id: 0,
            enabled: false,
            mode: "consensus_strict".to_string(),
            approval_threshold: 5,
            require_unanimity: true,
            weekly_vote_limit: 20,
            min_days_subscribed: 14,
            min_cards_reviewed: 200,
            notifications_enabled: true,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateDemocracyConfig {
    pub deck: String,
    pub enabled: bool,
    pub mode: String,
    pub approval_threshold: i32,
    pub require_unanimity: bool,
    pub weekly_vote_limit: i32,
    pub min_days_subscribed: i32,
    pub min_cards_reviewed: i32,
    pub notifications_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RosterMember {
    pub user_id: i32,
    pub username: String,
    pub status: String,
    pub allowed_tags: Option<Vec<String>>,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRosterRequest {
    pub deck: String,
    pub username: String,
    pub status: String,
    pub action: i32, // 1 = add, 2 = remove
    pub allowed_tags: Option<String>,
    pub reason: Option<String>,
}

pub async fn get_democracy_config(
    db_state: &database::AppState,
    deck_id: i64,
) -> Result<DemocracyConfig, Box<dyn std::error::Error>> {
    let client = database::client(db_state).await?;
    let row = client
        .query_opt(
            "SELECT deck_id, enabled, mode, approval_threshold, require_unanimity, \
             weekly_vote_limit, min_days_subscribed, min_cards_reviewed, notifications_enabled \
             FROM anki.deck_democracy_settings WHERE deck_id = $1",
            &[&deck_id],
        )
        .await?;

    if let Some(r) = row {
        Ok(DemocracyConfig {
            deck_id: r.get(0),
            enabled: r.get(1),
            mode: r.get(2),
            approval_threshold: r.get(3),
            require_unanimity: r.get(4),
            weekly_vote_limit: r.get(5),
            min_days_subscribed: r.get(6),
            min_cards_reviewed: r.get(7),
            notifications_enabled: r.get(8),
        })
    } else {
        let mut def = DemocracyConfig::default();
        def.deck_id = deck_id;
        Ok(def)
    }
}

pub async fn update_democracy_config(
    db_state: &database::AppState,
    deck_id: i64,
    cfg: &UpdateDemocracyConfig,
) -> Return<()> {
    let client = database::client(db_state).await?;
    let clamped_threshold = cfg.approval_threshold.clamp(1, 100);
    let clamped_weekly_limit = cfg.weekly_vote_limit.clamp(1, 5000);
    let clamped_min_days = cfg.min_days_subscribed.max(0);
    let clamped_min_cards = cfg.min_cards_reviewed.max(0);
    let valid_mode = match cfg.mode.as_str() {
        "consensus_strict" | "qualified_majority" | "lazy_consensus" => cfg.mode.as_str(),
        _ => "consensus_strict",
    };

    client
        .execute(
            "INSERT INTO anki.deck_democracy_settings \
             (deck_id, enabled, mode, approval_threshold, require_unanimity, weekly_vote_limit, \
              min_days_subscribed, min_cards_reviewed, notifications_enabled, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NOW()) \
             ON CONFLICT (deck_id) DO UPDATE SET \
                enabled = EXCLUDED.enabled, \
                mode = EXCLUDED.mode, \
                approval_threshold = EXCLUDED.approval_threshold, \
                require_unanimity = EXCLUDED.require_unanimity, \
                weekly_vote_limit = EXCLUDED.weekly_vote_limit, \
                min_days_subscribed = EXCLUDED.min_days_subscribed, \
                min_cards_reviewed = EXCLUDED.min_cards_reviewed, \
                notifications_enabled = EXCLUDED.notifications_enabled, \
                updated_at = NOW()",
            &[
                &deck_id,
                &cfg.enabled,
                &valid_mode,
                &clamped_threshold,
                &cfg.require_unanimity,
                &clamped_weekly_limit,
                &clamped_min_days,
                &clamped_min_cards,
                &cfg.notifications_enabled,
            ],
        )
        .await?;
    Ok(())
}

pub async fn get_roster(
    db_state: &database::AppState,
    deck_id: i64,
) -> Result<Vec<RosterMember>, Box<dyn std::error::Error>> {
    let client = database::client(db_state).await?;
    let rows = client
        .query(
            "SELECT r.user_id, u.username, r.status, r.allowed_tags, r.reason \
             FROM anki.deck_moderator_roster r \
             JOIN public.users u ON r.user_id = u.id \
             WHERE r.deck_id = $1 \
             ORDER BY r.status ASC, u.username ASC",
            &[&deck_id],
        )
        .await?;

    let mut members = Vec::new();
    for row in rows {
        members.push(RosterMember {
            user_id: row.get(0),
            username: row.get(1),
            status: row.get(2),
            allowed_tags: row.get(3),
            reason: row.get(4),
        });
    }
    Ok(members)
}

pub async fn update_roster(
    db_state: &database::AppState,
    deck_id: i64,
    req: &UpdateRosterRequest,
) -> Return<()> {
    let client = database::client(db_state).await?;
    let normalized_username = req.username.trim().to_lowercase();
    let user_row = client
        .query_opt(
            "SELECT id FROM public.users WHERE LOWER(username) = $1",
            &[&normalized_username],
        )
        .await?;

    let user_id: i32 = match user_row {
        Some(u) => u.get(0),
        None => return Err(UserNotFound),
    };

    if req.action == 1 {
        let valid_status = match req.status.as_str() {
            "whitelist" | "blacklist" | "active" | "auto_qualified" => req.status.as_str(),
            _ => "whitelist",
        };
        let tags: Option<Vec<String>> = req.allowed_tags.as_ref().map(|s| {
            s.split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        });

        client
            .execute(
                "INSERT INTO anki.deck_moderator_roster \
                 (deck_id, user_id, status, allowed_tags, reason, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, NOW()) \
                 ON CONFLICT (deck_id, user_id) DO UPDATE SET \
                    status = EXCLUDED.status, \
                    allowed_tags = EXCLUDED.allowed_tags, \
                    reason = EXCLUDED.reason, \
                    updated_at = NOW()",
                &[&deck_id, &user_id, &valid_status, &tags, &req.reason],
            )
            .await?;
    } else {
        client
            .execute(
                "DELETE FROM anki.deck_moderator_roster WHERE deck_id = $1 AND user_id = $2",
                &[&deck_id, &user_id],
            )
            .await?;
    }

    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditVoteEntry {
    pub user_id: i32,
    pub username: String,
    pub vote: String, // "approve", "reject", "flag"
    pub weight: f64,
    pub early_adopted: bool,
    pub voted_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditItem {
    pub suggestion_type: String,
    pub suggestion_id: i64,
    pub deck_id: i64,
    pub status: String,
    pub approve_count: i32,
    pub reject_count: i32,
    pub note_id: Option<i64>,
    pub note_guid: Option<String>,
    pub field_position: Option<u32>,
    pub field_name: Option<String>,
    pub original_content: Option<String>,
    pub new_content: Option<String>,
    pub diff_html: Option<String>,
    pub tag_name: Option<String>,
    pub tag_action: Option<bool>,
    pub resolved_at: Option<String>,
    pub reverted_at: Option<String>,
    pub reverted_by_username: Option<String>,
    pub override_reason: Option<String>,
    pub votes: Vec<AuditVoteEntry>,
}

#[derive(Debug, Deserialize)]
pub struct AdminOverrideRequest {
    pub deck: String,
    pub suggestion_type: String,
    pub suggestion_id: i64,
    pub action: String, // "revert", "confirm_approval", "admin_reject"
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AdminBlacklistRequest {
    pub deck: String,
    pub username: String,
    pub reason: Option<String>,
}

pub async fn get_audit_items(
    db_state: &database::AppState,
    deck_id: i64,
) -> Result<Vec<AuditItem>, Box<dyn std::error::Error>> {
    let client = database::client(db_state).await?;
    let rows = client
        .query(
            "SELECT s.suggestion_type, s.suggestion_id, s.deck_id, s.status, s.approve_count, s.reject_count, \
                    s.note_id, n.guid, s.field_position, s.field_name, s.original_content, s.new_content, \
                    s.tag_name, s.tag_action, \
                    to_char(s.resolved_at, 'YYYY-MM-DD HH24:MI:SS'), \
                    to_char(s.reverted_at, 'YYYY-MM-DD HH24:MI:SS'), \
                    u.username, s.override_reason \
             FROM anki.suggestion_moderation_status s \
             LEFT JOIN anki.notes n ON s.note_id = n.id \
             LEFT JOIN public.users u ON s.reverted_by = u.id \
             WHERE s.deck_id = $1 \
             ORDER BY COALESCE(s.reverted_at, s.resolved_at, NOW()) DESC \
             LIMIT 150",
            &[&deck_id],
        )
        .await?;

    let mut field_ids = Vec::new();
    let mut tag_ids = Vec::new();
    for row in &rows {
        let st: String = row.get(0);
        let sid: i64 = row.get(1);
        if st == "field" {
            field_ids.push(sid);
        } else {
            tag_ids.push(sid);
        }
    }

    let mut votes_map: std::collections::HashMap<(String, i64), Vec<AuditVoteEntry>> =
        std::collections::HashMap::new();

    if !field_ids.is_empty() || !tag_ids.is_empty() {
        let all_votes = client
            .query(
                "SELECT v.suggestion_type, v.suggestion_id, v.user_id, u.username, v.vote, v.weight::float8, v.early_adopted, \
                        to_char(v.voted_at, 'YYYY-MM-DD HH24:MI:SS') \
                 FROM anki.deck_moderator_votes v \
                 JOIN public.users u ON v.user_id = u.id \
                 WHERE (v.suggestion_type = 'field' AND v.suggestion_id = ANY($1)) \
                    OR (v.suggestion_type = 'tag' AND v.suggestion_id = ANY($2)) \
                 ORDER BY v.voted_at ASC",
                &[&field_ids, &tag_ids],
            )
            .await?;

        for vr in all_votes {
            let st: String = vr.get(0);
            let sid: i64 = vr.get(1);
            let entry = AuditVoteEntry {
                user_id: vr.get(2),
                username: vr.get(3),
                vote: vr.get(4),
                weight: vr.get(5),
                early_adopted: vr.get(6),
                voted_at: vr.get(7),
            };
            votes_map.entry((st, sid)).or_default().push(entry);
        }
    }

    let mut items = Vec::new();
    for row in rows {
        let suggestion_type: String = row.get(0);
        let suggestion_id: i64 = row.get(1);
        let votes = votes_map
            .remove(&(suggestion_type.clone(), suggestion_id))
            .unwrap_or_default();

        let orig: Option<String> = row.get(10);
        let new_c: Option<String> = row.get(11);
        let clean_orig = orig.as_ref().map(|o| crate::cleanser::clean(o));
        let clean_new = new_c.as_ref().map(|n| crate::cleanser::clean(n));
        let diff_html = match (&clean_orig, &clean_new) {
            (Some(o), Some(n)) => Some(htmldiff::htmldiff(o, n)),
            (None, Some(n)) => Some(format!("<ins>{}</ins>", n)),
            (Some(o), None) => Some(format!("<del>{}</del>", o)),
            _ => None,
        };

        let field_pos_i32: Option<i32> = row.get(8);
        items.push(AuditItem {
            suggestion_type,
            suggestion_id,
            deck_id: row.get(2),
            status: row.get(3),
            approve_count: row.get(4),
            reject_count: row.get(5),
            note_id: row.get(6),
            note_guid: row.get(7),
            field_position: field_pos_i32.map(|p| p as u32),
            field_name: row.get(9),
            original_content: orig,
            new_content: new_c,
            diff_html,
            tag_name: row.get(12),
            tag_action: row.get(13),
            resolved_at: row.get(14),
            reverted_at: row.get(15),
            reverted_by_username: row.get(16),
            override_reason: row.get(17),
            votes,
        });
    }

    Ok(items)
}

pub async fn execute_admin_override(
    db_state: &database::AppState,
    deck_id: i64,
    admin_user_id: i32,
    req: &AdminOverrideRequest,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut client = database::client(db_state).await?;
    let tx = client.transaction().await?;

    let status_row = tx
        .query_opt(
            "SELECT status, note_id, field_position, original_content, new_content, tag_name, tag_action \
             FROM anki.suggestion_moderation_status \
             WHERE suggestion_type = $1 AND suggestion_id = $2 AND deck_id = $3",
            &[&req.suggestion_type, &req.suggestion_id, &deck_id],
        )
        .await?;

    let (
        current_status,
        note_id_opt,
        field_pos_opt,
        original_content,
        _new_content,
        tag_name_opt,
        tag_action_opt,
    ) = match status_row {
        Some(r) => (
            r.get::<_, String>(0),
            r.get::<_, Option<i64>>(1),
            r.get::<_, Option<i32>>(2),
            r.get::<_, Option<String>>(3),
            r.get::<_, Option<String>>(4),
            r.get::<_, Option<String>>(5),
            r.get::<_, Option<bool>>(6),
        ),
        None => return Err("Suggestion moderation entry not found".into()),
    };

    let new_status: String;
    match req.action.as_str() {
        "revert" => {
            if current_status != "approved" && current_status != "approved_by_admin" {
                return Err("Only approved suggestions can be reverted".into());
            }

            if req.suggestion_type == "field" {
                if let (Some(note_id), Some(pos)) = (note_id_opt, field_pos_opt) {
                    let pos_u32 = pos as u32;
                    tx.execute(
                        "DELETE FROM anki.fields WHERE id = $1 OR (note = $2 AND position = $3 AND reviewed = true)",
                        &[&req.suggestion_id, &note_id, &pos_u32],
                    )
                    .await?;

                    if let Some(ref orig) = original_content {
                        tx.execute(
                            "INSERT INTO anki.fields (note, position, content, reviewed, creator_ip) \
                             VALUES ($1, $2, $3, true, 'retro_revert')",
                            &[&note_id, &pos_u32, orig],
                        )
                        .await?;
                    }

                    tx.execute(
                        "UPDATE anki.notes SET last_update = NOW() WHERE id = $1",
                        &[&note_id],
                    )
                    .await?;
                }
            } else if req.suggestion_type == "tag" {
                if let (Some(note_id), Some(tag_name)) = (note_id_opt, tag_name_opt) {
                    if tag_action_opt == Some(true) {
                        tx.execute(
                            "DELETE FROM anki.tags WHERE note = $1 AND content = $2",
                            &[&note_id, &tag_name],
                        )
                        .await?;
                    } else if tag_action_opt == Some(false) {
                        tx.execute(
                            "INSERT INTO anki.tags (note, content, reviewed, action, creator_ip) \
                             SELECT $1, $2, true, true, 'retro_revert' \
                             WHERE NOT EXISTS ( \
                                 SELECT 1 FROM anki.tags WHERE note = $1 AND content = $2 AND reviewed = true \
                             )",
                            &[&note_id, &tag_name],
                        )
                        .await?;
                    }

                    tx.execute(
                        "UPDATE anki.notes SET last_update = NOW() WHERE id = $1",
                        &[&note_id],
                    )
                    .await?;
                }
            }

            new_status = "reverted_by_admin".to_string();
            tx.execute(
                "UPDATE anki.suggestion_moderation_status \
                 SET status = $1, reverted_at = NOW(), reverted_by = $2, override_reason = $3 \
                 WHERE suggestion_type = $4 AND suggestion_id = $5",
                &[
                    &new_status,
                    &admin_user_id,
                    &req.reason,
                    &req.suggestion_type,
                    &req.suggestion_id,
                ],
            )
            .await?;
        }
        "confirm_approval" => {
            if req.suggestion_type == "field" {
                if let (Some(note_id), Some(pos)) = (note_id_opt, field_pos_opt) {
                    let pos_u32 = pos as u32;
                    if pos == 0 {
                        let field_row = tx
                            .query_opt("SELECT content FROM anki.fields WHERE id = $1", &[&req.suggestion_id])
                            .await?;
                        let content = field_row.and_then(|r| r.get::<_, Option<String>>(0)).unwrap_or_default();
                        if content.trim().is_empty() {
                            return Err("Cannot approve empty content for field 0 (sort field / primary field).".into());
                        }
                    }
                    tx.execute(
                        "DELETE FROM anki.fields WHERE note = $1 AND position = $2 AND reviewed = true AND id != $3",
                        &[&note_id, &pos_u32, &req.suggestion_id],
                    )
                    .await?;

                    tx.execute(
                        "UPDATE anki.fields SET reviewed = true WHERE id = $1",
                        &[&req.suggestion_id],
                    )
                    .await?;

                    tx.execute(
                        "UPDATE anki.notes SET last_update = NOW() WHERE id = $1",
                        &[&note_id],
                    )
                    .await?;
                }
            } else if req.suggestion_type == "tag" {
                if let (Some(note_id), Some(tag_name)) = (note_id_opt, tag_name_opt) {
                    if tag_action_opt == Some(false) {
                        tx.execute(
                            "DELETE FROM anki.tags WHERE note = $1 AND content = $2",
                            &[&note_id, &tag_name],
                        )
                        .await?;
                    } else {
                        tx.execute(
                            "UPDATE anki.tags SET reviewed = true WHERE id = $1",
                            &[&req.suggestion_id],
                        )
                        .await?;
                    }

                    tx.execute(
                        "UPDATE anki.notes SET last_update = NOW() WHERE id = $1",
                        &[&note_id],
                    )
                    .await?;
                }
            }

            new_status = "approved_by_admin".to_string();
            tx.execute(
                "UPDATE anki.suggestion_moderation_status \
                 SET status = $1, resolved_at = NOW(), override_reason = $2 \
                 WHERE suggestion_type = $3 AND suggestion_id = $4",
                &[
                    &new_status,
                    &req.reason,
                    &req.suggestion_type,
                    &req.suggestion_id,
                ],
            )
            .await?;
        }
        "admin_reject" => {
            if req.suggestion_type == "field" {
                tx.execute(
                    "DELETE FROM anki.fields WHERE id = $1 AND reviewed = false",
                    &[&req.suggestion_id],
                )
                .await?;
            } else if req.suggestion_type == "tag" {
                tx.execute(
                    "DELETE FROM anki.tags WHERE id = $1 AND reviewed = false",
                    &[&req.suggestion_id],
                )
                .await?;
            }

            new_status = "rejected_by_admin".to_string();
            tx.execute(
                "UPDATE anki.suggestion_moderation_status \
                 SET status = $1, resolved_at = NOW(), override_reason = $2 \
                 WHERE suggestion_type = $3 AND suggestion_id = $4",
                &[
                    &new_status,
                    &req.reason,
                    &req.suggestion_type,
                    &req.suggestion_id,
                ],
            )
            .await?;
        }
        _ => {
            return Err("Invalid action. Supported: revert, confirm_approval, admin_reject".into())
        }
    }

    if req.action == "revert" || req.action == "confirm_approval" {
        if let Some(note_id) = note_id_opt {
            let updated_note = tx
                .query_opt(
                    "UPDATE anki.notes SET last_update = NOW(), version = version + 1 WHERE id = $1 RETURNING version",
                    &[&note_id],
                )
                .await?;

            if let Some(nr) = updated_note {
                let new_ver: i64 = nr.get(0);
                let event_type = if req.action == "revert" {
                    "maintainer_revert"
                } else {
                    "maintainer_confirm_approval"
                };
                let meta = serde_json::json!({
                    "suggestion_type": req.suggestion_type,
                    "suggestion_id": req.suggestion_id,
                    "reason": req.reason
                });
                let _ = tx
                    .execute(
                        "INSERT INTO anki.note_events (note_id, version, event_type, actor_user_id, approved, new_value) \
                         VALUES ($1, $2, $3, $4, true, $5)",
                        &[&note_id, &new_ver, &event_type, &admin_user_id, &meta],
                    )
                    .await;
            }

            let _ = tx
                .execute(
                    "WITH RECURSIVE dt AS ( \
                         SELECT id, parent FROM anki.decks WHERE id = (SELECT deck FROM anki.notes WHERE id = $1) \
                         UNION ALL \
                         SELECT d.id, d.parent FROM anki.decks d INNER JOIN dt ON dt.parent = d.id \
                     ) UPDATE anki.decks SET last_update = NOW() WHERE id IN (SELECT id FROM dt)",
                    &[&note_id],
                )
                .await?;

            // Bump subscriber notes and their ancestor decks
            if let Ok(subs) = tx
                .query(
                    "SELECT subscriber_note_id FROM anki.note_inheritance WHERE base_note_id = $1",
                    &[&note_id],
                )
                .await
            {
                if !subs.is_empty() {
                    let sub_ids: Vec<i64> = subs.into_iter().map(|r| r.get(0)).collect();
                    let _ = tx
                        .execute(
                            "UPDATE anki.notes SET last_update = NOW() WHERE id = ANY($1::bigint[])",
                            &[&sub_ids],
                        )
                        .await;
                    let _ = tx
                        .execute(
                            "WITH RECURSIVE roots AS ( \
                                 SELECT DISTINCT n.deck AS id FROM anki.notes n WHERE n.id = ANY($1::bigint[]) \
                             ), up AS ( \
                                 SELECT d.id, d.parent FROM anki.decks d JOIN roots r ON d.id = r.id \
                                 UNION ALL \
                                 SELECT d.id, d.parent FROM anki.decks d JOIN up ON d.id = up.parent \
                             ) UPDATE anki.decks SET last_update = NOW() WHERE id IN (SELECT id FROM up)",
                            &[&sub_ids],
                        )
                        .await;
                }
            }

            if req.action == "confirm_approval" {
                if let Ok(rules) =
                    crate::deck_proposal_rules_manager::get_rules_with_client(&tx, deck_id).await
                {
                    let _ = crate::deck_proposal_rules_manager::apply_tagging_rules_on_approval(
                        &tx, note_id, false, &rules, None,
                    )
                    .await;
                }
            }
        }
    }

    tx.commit().await?;
    Ok(new_status)
}

pub async fn blacklist_voter_from_audit(
    db_state: &database::AppState,
    deck_id: i64,
    username: &str,
    reason: Option<&str>,
) -> Return<()> {
    let client = database::client(db_state).await?;
    let normalized = username.trim().to_lowercase();
    let user_row = client
        .query_opt(
            "SELECT id FROM public.users WHERE LOWER(username) = $1",
            &[&normalized],
        )
        .await?;

    let user_id: i32 = match user_row {
        Some(u) => u.get(0),
        None => return Err(UserNotFound),
    };

    client
        .execute(
            "INSERT INTO anki.deck_moderator_roster \
             (deck_id, user_id, status, allowed_tags, reason, updated_at) \
             VALUES ($1, $2, 'blacklist', NULL, $3, NOW()) \
             ON CONFLICT (deck_id, user_id) DO UPDATE SET \
                status = 'blacklist', \
                reason = EXCLUDED.reason, \
                updated_at = NOW()",
            &[&deck_id, &user_id, &reason],
        )
        .await?;

    Ok(())
}
