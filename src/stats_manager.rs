use std::sync::Arc;

use crate::database;
use crate::structs::{DeckBaseStatsInfo, DeckStatsInfo, NoteStatsInfo};
use async_recursion::async_recursion;

pub async fn update_stats(
    db_state: &Arc<database::AppState>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Refresh the note calculated_stats
    calculate_note_stats(db_state).await?;

    // Update the decks retention rates
    update_all_decks(db_state).await?;

    Ok(())
}

pub async fn calculate_note_stats(
    db_state: &Arc<database::AppState>,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = database::client(db_state).await?;
    let query = "
        WITH non_deleted_notes AS (
            SELECT id FROM notes WHERE NOT deleted
        )
        INSERT INTO calculated_stats (note_id, sample_size, retention, lapses, reps)
        SELECT
            ns.note_id,
            COUNT(DISTINCT ns.user_hash) as sample_size,
            ROUND(AVG(ns.retention), 1) as retention,
            ROUND(AVG(ns.lapses), 1) as lapses,
            ROUND(AVG(ns.reps), 1) as reps
        FROM
            note_stats ns
        JOIN
            non_deleted_notes n ON ns.note_id = n.id
        GROUP BY
            ns.note_id
        ON CONFLICT (note_id) DO UPDATE
        SET
            sample_size = EXCLUDED.sample_size,
            retention = EXCLUDED.retention,
            lapses = EXCLUDED.lapses,
            reps = EXCLUDED.reps
    ";
    client.execute(query, &[]).await?;

    let update_query = "
        UPDATE decks
        SET notes_with_stats_count = (
            SELECT COUNT(*)
            FROM notes n JOIN calculated_stats cs ON n.id = cs.note_id
            WHERE n.deck = decks.id AND NOT n.deleted
        )
        WHERE id IN (
            SELECT deck
            FROM notes
            WHERE id IN (
                SELECT note_id
                FROM calculated_stats
            )
        )
    ";
    client.execute(update_query, &[]).await?;
    Ok(())
}

pub async fn get_leaf_decks(
    db_state: &Arc<database::AppState>,
) -> Result<Vec<i64>, Box<dyn std::error::Error>> {
    let client = database::client(db_state).await?;
    let query = "
        WITH RECURSIVE cte AS (
            SELECT id, parent FROM decks WHERE stats_enabled = true
            UNION ALL
            SELECT d.id, d.parent
            FROM cte JOIN decks d ON d.parent = cte.id
        )
        SELECT id FROM cte d
        WHERE NOT EXISTS (
            SELECT 1 FROM decks WHERE parent = d.id
        )
    ";
    let rows = client.query(query, &[]).await?;

    let leaf_decks = rows.into_iter().map(|row| row.get(0)).collect::<Vec<i64>>();
    Ok(leaf_decks)
}

#[async_recursion]
pub async fn calculate_average_retention(
    db_state: &Arc<database::AppState>,
    deck: i64,
) -> Result<Option<f32>, Box<dyn std::error::Error>> {
    let client = database::client(db_state).await?;
    let query = "
        WITH notes_with_stats AS (
            SELECT n.id, cs.retention, 1 as note_count
            FROM notes n JOIN calculated_stats cs ON n.id = cs.note_id
            WHERE n.deck = $1 and n.deleted = false and cs.retention IS NOT NULL
        ),
        decks_with_retention AS (
            SELECT d.id, d.retention, d.notes_with_stats_count as note_count
            FROM decks d
            WHERE d.parent = $1 AND d.retention IS NOT NULL
        ),
        combined AS (
            SELECT retention, note_count FROM notes_with_stats
            UNION ALL
            SELECT retention, note_count FROM decks_with_retention
        )
        SELECT 
            CASE 
                WHEN SUM(note_count) = 0 THEN NULL
                ELSE CAST(ROUND((SUM(retention * note_count) / SUM(note_count))::numeric, 1) AS REAL)
            END as average_retention 
        FROM combined
    ";
    let rows = client.query(query, &[&deck]).await?;

    if rows.is_empty() {
        return Ok(None);
    }

    let average_retention: Option<f32> = rows[0].get(0);
    Ok(average_retention)
}

#[async_recursion]
pub async fn update_deck_and_parent_retention(
    db_state: &Arc<database::AppState>,
    deck: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = database::client(db_state).await?;
    // Get the note count for the current deck and its subdecks
    let update_note_count_query = "
        UPDATE decks
        SET notes_with_stats_count = (
            SELECT COALESCE(SUM(notes_with_stats_count), 0)
            FROM decks
            WHERE parent = $1
        ) + (
            SELECT COUNT(*)
            FROM notes n JOIN calculated_stats cs ON n.id = cs.note_id
            WHERE n.deck = $1 AND NOT n.deleted
        )
        WHERE id = $1
    ";
    client.execute(update_note_count_query, &[&deck]).await?;

    let retention = calculate_average_retention(db_state, deck).await?;
    if let Some(retention) = retention {
        let parent_query = "SELECT parent FROM decks WHERE id = $1";
        let rows = client.query(parent_query, &[&deck]).await?;

        let query = "UPDATE decks SET retention = $2 WHERE id = $1";
        client.execute(query, &[&deck, &retention]).await?;

        if let Some(parent_deck) = rows.first().and_then(|row| row.get::<_, Option<i64>>(0)) {
            update_deck_and_parent_retention(db_state, parent_deck).await?;
        }
    }

    Ok(())
}

pub async fn update_all_decks(
    db_state: &Arc<database::AppState>,
) -> Result<(), Box<dyn std::error::Error>> {
    let leaf_decks = get_leaf_decks(db_state).await?;

    for deck in leaf_decks {
        update_deck_and_parent_retention(db_state, deck).await?;
    }

    Ok(())
}

pub async fn get_base_deck_info(
    db_state: &Arc<database::AppState>,
    deck_hash: &String,
) -> Result<DeckBaseStatsInfo, Box<dyn std::error::Error>> {
    let client = database::client(db_state).await?;

    // Query to get note_count and retention_avg
    let query1 = "
        SELECT COALESCE(notes_with_stats_count, 0), COALESCE(retention, 0.0)
        FROM decks
        WHERE human_hash = $1
    ";
    let rows = client.query(query1, &[&deck_hash]).await?;
    let (note_count, retention_avg) = if let Some(row) = rows.first() {
        (row.get(0), row.get(1))
    } else {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "No deck found with the given hash",
        )));
    };

    // Query to get lapses_avg and reps_avg
    let query2 = "
        WITH RECURSIVE cte AS (
            SELECT id
            FROM decks
            WHERE human_hash = $1
            UNION ALL
            SELECT d.id
            FROM cte JOIN decks d ON d.parent = cte.id
        )
        SELECT COALESCE(AVG(cs.lapses), 0), COALESCE(AVG(cs.reps), 0)
        FROM cte
        JOIN notes n ON cte.id = n.deck AND NOT n.deleted
        JOIN calculated_stats cs ON n.id = cs.note_id
    ";
    let rows = client.query(query2, &[&deck_hash]).await?;
    let (lapses_avg, reps_avg) = if let Some(row) = rows.first() {
        (row.get(0), row.get(1))
    } else {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "No calculated stats found for the given deck",
        )));
    };

    Ok(DeckBaseStatsInfo {
        note_count,
        lapses_avg,
        reps_avg,
        retention_avg,
    })
}

pub async fn get_deck_stat_info(
    db_state: &Arc<database::AppState>,
    deck_hash: &String,
) -> Result<Vec<DeckStatsInfo>, Box<dyn std::error::Error>> {
    let client = database::client(db_state).await?;
    // Get all the stat infos on the deck and (recursively) all subdecks
    let query = "
        WITH RECURSIVE cte AS (
            SELECT id, human_hash, parent, full_path, retention
            FROM decks
            WHERE human_hash = $1
            UNION ALL
            SELECT d.id, d.human_hash, d.parent, d.full_path, d.retention
            FROM cte JOIN decks d ON d.parent = cte.id
        )
        SELECT id, human_hash, parent, full_path, COALESCE(retention, 0)
        FROM cte
    ";
    let rows = client.query(query, &[&deck_hash]).await?;

    let res = rows
        .into_iter()
        .map(|row| {
            let hash: String = row.get(1);
            let path: String = row.get(3);
            let retention: f32 = row.get(4);
            DeckStatsInfo {
                hash,
                path,
                retention,
            }
        })
        .collect::<Vec<DeckStatsInfo>>();

    Ok(res)
}

pub async fn get_worst_notes_info(
    db_state: &Arc<database::AppState>,
    deck_hash: &String,
) -> Result<Vec<NoteStatsInfo>, Box<dyn std::error::Error>> {
    let client = database::client(db_state).await?;
    let query = "
        WITH RECURSIVE cte AS (
            SELECT id, human_hash, parent, full_path
            FROM decks
            WHERE human_hash = $1
            UNION ALL
            SELECT d.id, d.human_hash, d.parent, d.full_path
            FROM cte JOIN decks d ON d.parent = cte.id
        ), worst_notes AS (
            SELECT n.id, cs.lapses, cs.reps, cs.retention, cs.sample_size
            FROM notes n 
            JOIN calculated_stats cs ON n.id = cs.note_id
            WHERE n.deck IN (SELECT id FROM cte) and NOT n.deleted
            ORDER BY cs.retention ASC, cs.lapses DESC
            LIMIT 100
        )
        SELECT wn.id, 
            (SELECT coalesce(f.content, '') FROM fields AS f WHERE f.note = wn.id AND f.position = 0 LIMIT 1) AS content,
            wn.lapses, wn.reps, wn.retention, wn.sample_size
        FROM worst_notes wn
    ";
    let rows = client.query(query, &[&deck_hash]).await?;

    let res = rows
        .into_iter()
        .map(|row| NoteStatsInfo {
            id: row.get(0),
            fields: row.get::<usize, Option<String>>(1).unwrap_or_default(),
            lapses: row.get(2),
            reps: row.get(3),
            retention: row.get(4),
            sample_size: row.get(5),
        })
        .collect::<Vec<NoteStatsInfo>>();

    Ok(res)
}

pub async fn toggle_stats(
    db_state: &Arc<database::AppState>,
    deck_id: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = database::client(db_state).await?;
    let query = "
        UPDATE decks
        SET stats_enabled = NOT stats_enabled
        WHERE id = $1
    ";
    client.execute(query, &[&deck_id]).await?;
    Ok(())
}
