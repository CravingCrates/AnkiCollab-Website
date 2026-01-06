use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use tokio_postgres::Client;

use crate::{
    Return, cleanser, structs::{CommitHistoryEvent, CommitHistoryNote, NoteHistoryEvent, NoteHistoryGroup, NoteId}
};

use crate::Error::NoteNotFound;
use crate::NoteNotFoundContext;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventType {
    NoteCreated,
    FieldAdded,
    FieldUpdated,
    FieldRemoved,
    TagAdded,
    TagRemoved,
    TagHidden,
    TagUnhidden,
    NoteMoved,
    NoteDeleted,
    CommitApprovedEffect,
    CommitDeniedEffect,
    SuggestionDenied,
    FieldChangeDenied,
    TagChangeDenied,
}

impl EventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventType::NoteCreated => "note_created",
            EventType::FieldAdded => "field_added",
            EventType::FieldUpdated => "field_updated",
            EventType::FieldRemoved => "field_removed",
            EventType::TagAdded => "tag_added",
            EventType::TagRemoved => "tag_removed",
            EventType::TagHidden => "tag_hidden",
            EventType::TagUnhidden => "tag_unhidden",
            EventType::NoteMoved => "note_moved",
            EventType::NoteDeleted => "note_deleted",
            EventType::CommitApprovedEffect => "commit_approved_effect",
            EventType::CommitDeniedEffect => "commit_denied_effect",
            EventType::SuggestionDenied => "suggestion_denied",
            EventType::FieldChangeDenied => "field_change_denied",
            EventType::TagChangeDenied => "tag_change_denied",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NoteEvent {
    pub id: i64,
    pub note_id: i64,
    pub version: i64,
    pub event_type: String,
    pub actor_user_id: Option<i32>,
    pub commit_id: Option<i32>,
    pub approved: Option<bool>,
    pub old_value: Option<JsonValue>,
    pub new_value: Option<JsonValue>,
    pub created_at: String,
}

pub struct NoteHistoryData {
    pub events: Vec<NoteHistoryEvent>,
    pub groups: Vec<NoteHistoryGroup>,
    pub actors: Vec<String>,
}

pub async fn fetch_note_history(client: &Client, note_id: NoteId) -> Return<NoteHistoryData> {
    let rows = client
        .query(
            "SELECT e.id, e.version, e.event_type, e.actor_user_id, u.username, e.commit_id, e.approved, e.old_value, e.new_value, to_char(e.created_at,'YYYY-MM-DD HH24:MI:SS')
             FROM note_events e
             LEFT JOIN users u ON e.actor_user_id = u.id
             WHERE e.note_id = $1
             ORDER BY e.version DESC
             LIMIT 100",
            &[&note_id],
        )
        .await?;

    let notetype_row = client
        .query_opt("SELECT notetype FROM notes WHERE id = $1", &[&note_id])
        .await?;
    let mut field_map: HashMap<u32, String> = HashMap::new();

    if let Some(row) = notetype_row {
        let notetype_id: i64 = row.get(0);
        let fields = client
            .query(
                "SELECT position, name FROM notetype_field WHERE notetype = $1",
                &[&notetype_id],
            )
            .await?;
        for f in fields {
            let pos: u32 = f.get(0);
            let name: String = f.get(1);
            field_map.insert(pos, name);
        }
    }

    let mut events: Vec<NoteHistoryEvent> = Vec::with_capacity(rows.len());
    for row in rows.iter() {
        let event_type: String = row.get(2);
        let old_value: Option<JsonValue> = row.get(7);
        let new_value: Option<JsonValue> = row.get(8);

        let mut field_name = None;
        if event_type.contains("field") {
            let pos_val = new_value
                .as_ref()
                .and_then(|v| v.get("position"))
                .or_else(|| old_value.as_ref().and_then(|v| v.get("position")));

            if let Some(pos_v) = pos_val {
                if let Some(pos_i64) = pos_v.as_i64() {
                    field_name = field_map.get(&(pos_i64 as u32)).cloned();
                }
            }
        }

        let (snapshot_field_count, snapshot_tags) = snapshot_meta(&event_type, &new_value);
        let old_human = summarize_event(&event_type, &old_value, "old");
        let new_human = summarize_event(&event_type, &new_value, "new");
        let diff_html = compute_diff_html(&event_type, &old_value, &new_value);
        events.push(NoteHistoryEvent {
            id: row.get(0),
            version: row.get(1),
            event_type: event_type.clone(),
            actor_user_id: row.get(3),
            actor_username: row.get(4),
            commit_id: row.get(5),
            approved: row.get(6),
            old_human,
            new_human,
            old_value,
            new_value,
            created_at: row.get(9),
            snapshot_field_count,
            snapshot_tags,
            diff_html,
            field_name,
        });
    }

    let mut groups = group_note_history_events(&events);
    auto_approve_created_only_groups(&mut groups);
    let actors = collect_actors(&events);

    Ok(NoteHistoryData {
        events,
        groups,
        actors,
    })
}

// Inserts an event and returns its id. Increments note version atomically.
pub async fn log_event(
    tx: &tokio_postgres::Transaction<'_>,
    note_id: i64,
    event_type: EventType,
    old_value: Option<&JsonValue>,
    new_value: Option<&JsonValue>,
    actor_user_id: Option<i32>,
    commit_id: Option<i32>,
    approved: Option<bool>,
) -> Return<i64> {
    let row = tx
        .query(
            "UPDATE notes SET version = version + 1 WHERE id = $1 RETURNING version",
            &[&note_id],
        )
        .await?;

    if row.is_empty() {
        return Err(NoteNotFound(NoteNotFoundContext::NoteLogEvent));
    }
    let new_version: i64 = row[0].get(0);

    let id_row = tx
        .query(
            "INSERT INTO note_events (note_id, version, event_type, actor_user_id, commit_id, approved, old_value, new_value) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id",
            &[
                &note_id,
                &new_version,
                &event_type.as_str(),
                &actor_user_id,
                &commit_id,
                &approved,
                &old_value,
                &new_value,
            ],
        )
        .await?;

    if id_row.is_empty() {
        return Err(NoteNotFound(NoteNotFoundContext::NoteLogEvent));
    }
    Ok(id_row[0].get(0))
}

pub async fn fetch_commit_history(
    client: &Client,
    commit_id: i32,
) -> Return<Vec<CommitHistoryNote>> {
    let rows = client
        .query(
            "SELECT e.note_id, e.id, e.version, e.event_type, e.old_value, e.new_value, e.actor_user_id, u.username, to_char(e.created_at,'YYYY-MM-DD HH24:MI:SS'), n.notetype
             FROM note_events e
             LEFT JOIN users u ON e.actor_user_id = u.id
             LEFT JOIN notes n ON e.note_id = n.id
             WHERE e.commit_id = $1
             ORDER BY e.note_id, e.version",
            &[&commit_id],
        )
        .await?;

    let mut notetypes = BTreeSet::new();
    for row in &rows {
        if let Some(nt) = row.get::<_, Option<i64>>(9) {
            notetypes.insert(nt);
        }
    }

    let mut field_map: HashMap<(i64, u32), String> = HashMap::new();
    if !notetypes.is_empty() {
        let nt_vec: Vec<i64> = notetypes.into_iter().collect();
        let fields = client
            .query(
                "SELECT notetype, position, name FROM notetype_field WHERE notetype = ANY($1)",
                &[&nt_vec],
            )
            .await?;
        for f in fields {
            let nt: i64 = f.get(0);
            let pos: u32 = f.get(1);
            let name: String = f.get(2);
            field_map.insert((nt, pos), name);
        }
    }

    let mut notes: BTreeMap<NoteId, CommitHistoryNote> = BTreeMap::new();
    for row in rows.iter() {
        let note_id: NoteId = row.get(0);
        let version: i64 = row.get(2);
        let event_type: String = row.get(3);
        let old_value: Option<JsonValue> = row.get(4);
        let new_value: Option<JsonValue> = row.get(5);
        let notetype_id: Option<i64> = row.get(9);

        let mut field_name = None;
        if let Some(nt) = notetype_id {
            if event_type.contains("field") {
                let pos_val = new_value
                    .as_ref()
                    .and_then(|v| v.get("position"))
                    .or_else(|| old_value.as_ref().and_then(|v| v.get("position")));

                if let Some(pos_v) = pos_val {
                    if let Some(pos_i64) = pos_v.as_i64() {
                        field_name = field_map.get(&(nt, pos_i64 as u32)).cloned();
                    }
                }
            }
        }

        let old_human = summarize_event(&event_type, &old_value, "old");
        let new_human = summarize_event(&event_type, &new_value, "new");
        let diff_html = compute_diff_html(&event_type, &old_value, &new_value);

        let event = CommitHistoryEvent {
            id: row.get(1),
            version,
            event_type: event_type.clone(),
            actor_username: row.get(7),
            created_at: row.get(8),
            old_human,
            new_human,
            diff_html,
            field_name,
        };

        let entry = notes.entry(note_id).or_insert_with(|| CommitHistoryNote {
            note_id,
            min_version: version,
            max_version: version,
            event_types: vec![event_type.clone()],
            events: Vec::new(),
            field_added: 0,
            field_updated: 0,
            field_removed: 0,
            tag_added: 0,
            tag_removed: 0,
            moved: false,
            deleted: false,
        });

        entry.min_version = entry.min_version.min(version);
        entry.max_version = entry.max_version.max(version);
        if !entry.event_types.contains(&event_type) {
            entry.event_types.push(event_type.clone());
        }
        apply_event_counters(entry, &event_type);
        entry.events.push(event);
    }

    Ok(notes.into_values().collect())
}

fn compute_diff_html(
    event_type: &str,
    old_value: &Option<JsonValue>,
    new_value: &Option<JsonValue>,
) -> Option<String> {
    if event_type != "field_updated" {
        return None;
    }
    let old_content = old_value
        .as_ref()
        .and_then(|v| v.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");
    let new_content = new_value
        .as_ref()
        .and_then(|v| v.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");
    if old_content.is_empty() && new_content.is_empty() {
        return None;
    }
    let clean_old = cleanser::clean(old_content);
    let clean_new = cleanser::clean(new_content);
    Some(htmldiff::htmldiff(&clean_old, &clean_new))
}

fn snapshot_meta(
    event_type: &str,
    new_value: &Option<JsonValue>,
) -> (Option<usize>, Option<Vec<String>>) {
    if event_type != "note_created" {
        return (None, None);
    }

    let value = match new_value {
        Some(v) => v,
        None => return (None, None),
    };

    let field_count = value
        .get("fields")
        .and_then(|f| f.as_array())
        .map(|arr| arr.len());
    let tags = value.get("tags").and_then(|t| t.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|e| e.as_str().map(|s| s.to_string()))
            .collect::<Vec<_>>()
    });
    (field_count, tags)
}

fn summarize_event(event_type: &str, json: &Option<JsonValue>, side: &str) -> Option<String> {
    let v = json.as_ref()?;
    match event_type {
        "field_added" | "field_removed" | "field_updated" => v
            .get("content")
            .and_then(|c| c.as_str())
            .map(|s| s.to_string())
            .or_else(|| {
                v.get("value")
                    .and_then(|c| c.as_str())
                    .map(|s| s.to_string())
            }),
        "tag_added" | "tag_removed" => v
            .get("content")
            .and_then(|c| c.as_str())
            .map(|s| format!("#{}", s)),
        "note_moved" => {
            let to = v.get("to").and_then(|x| x.as_str()).unwrap_or("");
            let from = v.get("from").and_then(|x| x.as_str()).unwrap_or("");
            if side == "new" && !to.is_empty() {
                Some(format!("to deck {}", to))
            } else if side == "old" && !from.is_empty() {
                Some(format!("from deck {}", from))
            } else {
                Some("moved".to_string())
            }
        }
        "note_deleted" => Some("note deleted".to_string()),
        "commit_approved_effect" => Some("commit approved".to_string()),
        "commit_denied_effect" => Some("commit denied".to_string()),
        "suggestion_denied" => Some("suggestion denied".to_string()),
        "field_change_denied" => {
            if side == "old" {
                // Current reviewed content
                v.get("current_content")
                    .and_then(|c| c.as_str())
                    .map(|s| truncate(s, 80))
            } else {
                // Denied suggestion content
                v.get("denied_content")
                    .and_then(|c| c.as_str())
                    .map(|s| truncate(s, 80))
            }
        }
        "tag_change_denied" => {
            let content = v.get("content").and_then(|c| c.as_str()).unwrap_or("");
            let action = v.get("action").and_then(|a| a.as_bool()).unwrap_or(true);
            if action {
                Some(format!("denied addition: #{}", content))
            } else {
                Some(format!("denied removal: #{}", content))
            }
        }
        _ => None,
    }
}

fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return if s.is_empty() {
            String::new()
        } else {
            "…".to_string()
        };
    }

    // Walk char boundaries so we never split a multi-byte codepoint mid-slice.
    let mut end = None;
    let mut count = 0;
    for (idx, _) in s.char_indices() {
        if count == max {
            end = Some(idx);
            break;
        }
        count += 1;
    }

    match end {
        None => s.to_string(),
        Some(idx) => format!("{}…", &s[..idx]),
    }
}

fn group_note_history_events(events: &[NoteHistoryEvent]) -> Vec<NoteHistoryGroup> {
    let mut ordered: Vec<NoteHistoryGroup> = Vec::new();
    let mut by_commit: BTreeMap<i32, usize> = BTreeMap::new();

    for event in events {
        if let Some(commit_id) = event.commit_id {
            if let Some(&idx) = by_commit.get(&commit_id) {
                if event.event_type == "commit_approved_effect" {
                    ordered[idx].approved = true;
                    continue;
                }
                if event.event_type == "commit_denied_effect" {
                    ordered[idx].denied = true;
                    continue;
                }
                ordered[idx].events.push(event.clone());
            } else {
                let mut group = NoteHistoryGroup {
                    commit_id: Some(commit_id),
                    ..Default::default()
                };
                if event.event_type == "commit_approved_effect" {
                    group.approved = true;
                } else if event.event_type == "commit_denied_effect" {
                    group.denied = true;
                } else {
                    group.events.push(event.clone());
                }
                by_commit.insert(commit_id, ordered.len());
                ordered.push(group);
            }
        } else {
            let mut group = NoteHistoryGroup::default();
            group.events.push(event.clone());
            ordered.push(group);
        }
    }

    ordered
}

fn auto_approve_created_only_groups(groups: &mut [NoteHistoryGroup]) {
    for group in groups.iter_mut() {
        if group.commit_id.is_some() && !group.approved && !group.denied {
            let all_note_created = group
                .events
                .iter()
                .all(|event| event.event_type == "note_created");
            if all_note_created && !group.events.is_empty() {
                group.approved = true;
            }
        }
    }
}

fn collect_actors(events: &[NoteHistoryEvent]) -> Vec<String> {
    let mut actors = BTreeSet::new();
    for event in events {
        actors.insert(
            event
                .actor_username
                .clone()
                .unwrap_or_else(|| "Anonymous".to_string()),
        );
    }
    actors.into_iter().collect()
}

fn apply_event_counters(note: &mut CommitHistoryNote, event_type: &str) {
    match event_type {
        "field_added" => note.field_added += 1,
        "field_updated" => note.field_updated += 1,
        "field_removed" => note.field_removed += 1,
        "tag_added" => note.tag_added += 1,
        "tag_removed" => note.tag_removed += 1,
        "note_moved" => note.moved = true,
        "note_deleted" => note.deleted = true,
        _ => {}
    }
}
