use crate::database;
use crate::structs::*;
use async_recursion::async_recursion;

pub async fn update_stats() -> Result<(), Box<dyn std::error::Error>> {

    // Refresh the note calculated_stats
    calculate_note_stats().await?;

    // Update the decks retention rates
    update_all_decks().await?;

    Ok(())
}

pub async fn calculate_note_stats() -> Result<(), Box<dyn std::error::Error>> {
    let client = database::client().await?;
    let query = "
        INSERT INTO calculated_stats (note_id, sample_size, retention, lapses, reps)
        SELECT 
            note_id,
            COUNT(DISTINCT user_hash) as sample_size,
            ROUND(AVG(retention), 1) as retention,
            ROUND(AVG(lapses), 1) as lapses,
            ROUND(AVG(reps), 1) as reps
        FROM note_stats
        GROUP BY note_id
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
            WHERE n.deck = decks.id
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

pub async fn get_leaf_decks() -> Result<Vec<i64>, Box<dyn std::error::Error>> {
    let client = database::client().await?;
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
pub async fn calculate_average_retention(deck: i64) -> Result<Option<f32>, Box<dyn std::error::Error>> {
    let client = database::client().await?;
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
pub async fn update_deck_and_parent_retention(deck: i64) -> Result<(), Box<dyn std::error::Error>> {
    let client = database::client().await?;
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
            WHERE n.deck = $1
        )
        WHERE id = $1
    ";
    client.execute(update_note_count_query, &[&deck]).await?;

    let retention = calculate_average_retention(deck).await?;
    if let Some(retention) = retention {
        let parent_query = "SELECT parent FROM decks WHERE id = $1";
        let rows = client.query(parent_query, &[&deck]).await?;
        
        let query = "UPDATE decks SET retention = $2 WHERE id = $1";
        client.execute(query, &[&deck, &retention]).await?;

        if let Some(parent_deck) = rows.get(0).and_then(|row| row.get::<_, Option<i64>>(0)) {
            update_deck_and_parent_retention(parent_deck).await?;
        }
    }

    Ok(())
}

pub async fn update_all_decks() -> Result<(), Box<dyn std::error::Error>> {
    let leaf_decks = get_leaf_decks().await?;

    for deck in leaf_decks {
        update_deck_and_parent_retention(deck).await?;
    }

    Ok(())
}

pub async fn get_base_deck_info(deck_hash: &String) -> Result<DeckBaseStatsInfo, Box<dyn std::error::Error>> {
    let client = database::client().await?;

    // Query to get note_count and retention_avg
    let query1 = "
        SELECT COALESCE(notes_with_stats_count, 0), COALESCE(retention, 0.0)
        FROM decks
        WHERE human_hash = $1
    ";
    let rows = client.query(query1, &[&deck_hash]).await?;
    let (note_count, retention_avg) = if let Some(row) = rows.get(0) {
        (row.get(0), row.get(1))
    } else {
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "No deck found with the given hash")));
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
        JOIN notes n ON cte.id = n.deck
        JOIN calculated_stats cs ON n.id = cs.note_id
    ";
    let rows = client.query(query2, &[&deck_hash]).await?;
    let (lapses_avg, reps_avg) = if let Some(row) = rows.get(0) {
        (row.get(0), row.get(1))
    } else {
        return Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, "No calculated stats found for the given deck")));
    };

    Ok(DeckBaseStatsInfo {
        note_count,
        retention_avg,
        lapses_avg,
        reps_avg,
    })
}

pub async fn get_deck_stat_info(deck_hash: &String) -> Result<Vec<DeckStatsInfo>, Box<dyn std::error::Error>> {
    let client = database::client().await?;
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

    let res = rows.into_iter().map(|row| {
        let hash: String = row.get(1);
        let path: String = row.get(3);
        let retention: f32 = row.get(4);
        DeckStatsInfo {
            hash,
            path,
            retention,
        }
    }).collect::<Vec<DeckStatsInfo>>();

    Ok(res)
}

pub async fn get_worst_notes_info(deck_hash: &String) -> Result<Vec<NoteStatsInfo>, Box<dyn std::error::Error>> {
    let client = database::client().await?;
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
            WHERE n.deck IN (SELECT id FROM cte)
            ORDER BY cs.retention ASC, cs.lapses DESC
            LIMIT 100
        )
        SELECT wn.id, 
            (SELECT coalesce(f.content, '') FROM fields AS f WHERE f.note = wn.id AND f.position = 0 LIMIT 1) AS content,
            wn.lapses, wn.reps, wn.retention, wn.sample_size
        FROM worst_notes wn
    ";
    let rows = client.query(query, &[&deck_hash]).await?;

    let res = rows.into_iter().map(|row| {
        NoteStatsInfo {
            id: row.get(0),
            fields: row.get::<usize, Option<String>>(1).unwrap(),
            lapses: row.get(2),
            reps: row.get(3),
            retention: row.get(4),
            sample_size: row.get(5),
        }
    }).collect::<Vec<NoteStatsInfo>>();

    Ok(res)
}

pub async fn toggle_stats(deck_id: i64) -> Result<(), Box<dyn std::error::Error>> {
    let client = database::client().await?;
    let query = "
        UPDATE decks
        SET stats_enabled = NOT stats_enabled
        WHERE id = $1
    ";
    client.execute(query, &[&deck_id]).await?;
    Ok(())
}