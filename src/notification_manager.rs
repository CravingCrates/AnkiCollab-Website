use std::collections::BTreeMap;
use std::sync::Arc;

use crate::database;
use crate::structs::{
    NotificationDeckGroup, NotificationHistoryResponse, NotificationItem, NotificationUnreadResponse,
};
use crate::Return;

fn deck_display_name(full_path: &str) -> String {
    let name = full_path
        .rsplit("::")
        .next()
        .unwrap_or(full_path)
        .trim()
        .to_string();
    if name.is_empty() {
        "[Unnamed Deck]".to_string()
    } else {
        name
    }
}

pub async fn create_commit_notification(
    db_state: &Arc<database::AppState>,
    commit_id: i32,
    status: &str,
    reason: Option<&str>,
    actor_user_id: i32,
) -> Return<()> {
    let client = database::client(db_state).await?;

    let row_opt = client
        .query_opt(
            "SELECT user_id, deck FROM commits WHERE commit_id = $1",
            &[&commit_id],
        )
        .await?;

    let Some(row) = row_opt else {
        return Ok(());
    };

    let target_user_id: Option<i32> = row.get(0);
    let deck_id: i64 = row.get(1);

    let Some(user_id) = target_user_id else {
        return Ok(());
    };

    if user_id == actor_user_id {
        return Ok(());
    }

    client
        .execute(
            "INSERT INTO notifications (user_id, commit_id, deck_id, status, reason)
             VALUES ($1, $2, $3, $4, $5)",
            &[&user_id, &commit_id, &deck_id, &status, &reason],
        )
        .await?;

    Ok(())
}

pub async fn get_unread_grouped(
    db_state: &Arc<database::AppState>,
    user_id: i32,
) -> Return<NotificationUnreadResponse> {
    let client = database::client(db_state).await?;

    let rows = client
        .query(
            "SELECT n.id,
                    n.commit_id,
                    n.deck_id,
                    d.full_path,
                    n.status,
                    n.reason,
                    TO_CHAR(n.created_at, 'YYYY-MM-DD\"T\"HH24:MI:SSTZH:TZM')
             FROM notifications n
             JOIN decks d ON d.id = n.deck_id
             WHERE n.user_id = $1 AND n.is_read = false
             ORDER BY n.created_at DESC
             LIMIT 500",
            &[&user_id],
        )
        .await?;

    let mut groups: BTreeMap<i64, NotificationDeckGroup> = BTreeMap::new();

    for row in rows {
        let deck_id: i64 = row.get(2);
        let full_path: String = row.get(3);
        let status: String = row.get(4);

        let item = NotificationItem {
            id: row.get(0),
            commit_id: row.get(1),
            deck_id,
            deck_name: deck_display_name(&full_path),
            status: status.clone(),
            reason: row.get(5),
            created_at: row.get(6),
            is_read: false,
        };

        let entry = groups.entry(deck_id).or_insert_with(|| NotificationDeckGroup {
            deck_id,
            deck_name: deck_display_name(&full_path),
            approved_count: 0,
            denied_count: 0,
            notifications: Vec::new(),
        });

        if status == "approved" {
            entry.approved_count += 1;
        } else if status == "denied" {
            entry.denied_count += 1;
        }

        entry.notifications.push(item);
    }

    let mut group_list: Vec<NotificationDeckGroup> = groups.into_values().collect();

    // Sort deck groups by the most recent notification's created_at (newest first)
    group_list.sort_by(|a, b| {
        let a_ts = a.notifications.first().map(|n| &n.created_at);
        let b_ts = b.notifications.first().map(|n| &n.created_at);
        // We want newest first, so compare b to a
        b_ts.cmp(&a_ts)
    });

    let unread_count = group_list
        .iter()
        .map(|g| g.notifications.len() as i64)
        .sum::<i64>();

    Ok(NotificationUnreadResponse {
        unread_count,
        groups: group_list,
    })
}

pub async fn get_history(
    db_state: &Arc<database::AppState>,
    user_id: i32,
    offset: i64,
    limit: i64,
) -> Return<NotificationHistoryResponse> {
    let client = database::client(db_state).await?;

    let rows = client
        .query(
            "SELECT n.id,
                    n.commit_id,
                    n.deck_id,
                    d.full_path,
                    n.status,
                    n.reason,
                    TO_CHAR(n.created_at, 'YYYY-MM-DD\"T\"HH24:MI:SSTZH:TZM'),
                    n.is_read
             FROM notifications n
             JOIN decks d ON d.id = n.deck_id
             WHERE n.user_id = $1
             ORDER BY n.created_at DESC
             OFFSET $2
             LIMIT $3",
            &[&user_id, &offset, &limit],
        )
        .await?;

    let total_row = client
        .query_one(
            "SELECT COUNT(*) FROM notifications WHERE user_id = $1",
            &[&user_id],
        )
        .await?;

    let items: Vec<NotificationItem> = rows
        .into_iter()
        .map(|row| {
            let full_path: String = row.get(3);
            NotificationItem {
                id: row.get(0),
                commit_id: row.get(1),
                deck_id: row.get(2),
                deck_name: deck_display_name(&full_path),
                status: row.get(4),
                reason: row.get(5),
                created_at: row.get(6),
                is_read: row.get(7),
            }
        })
        .collect();

    Ok(NotificationHistoryResponse {
        total: total_row.get(0),
        offset,
        limit,
        items,
    })
}

/// Maximum number of notification IDs accepted in a single mark-read request.
/// Validated in the handler to return 400; enforced here as defence-in-depth.
pub const MAX_MARK_READ_IDS: usize = 1_000;

pub async fn mark_read(
    db_state: &Arc<database::AppState>,
    user_id: i32,
    notification_ids: &[i32],
) -> Return<u64> {
    if notification_ids.is_empty() {
        return Ok(0);
    }

    debug_assert!(
        notification_ids.len() <= MAX_MARK_READ_IDS,
        "mark_read called with {} IDs; handler must validate before calling",
        notification_ids.len()
    );

    let client = database::client(db_state).await?;

    let updated = client
        .execute(
            "UPDATE notifications
             SET is_read = true
             WHERE user_id = $1
               AND id = ANY($2)",
            &[&user_id, &notification_ids],
        )
        .await?;

    Ok(updated)
}
