use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type Return<T> = Result<T, crate::error::Error>;
pub type DeckHash = String;
pub type UserId = i32;
pub type DeckId = i64;
pub type NoteId = i64;
pub type FieldId = i64;

/// The `Login` form is used along with the [`Auth`] guard to authenticate users.
#[derive(Deserialize)]
pub struct BetterLogin {
    pub username: String,
    pub password: String,
    pub cookie: bool,
}

/* Notes */
#[derive(Serialize)]
pub struct Note {
    pub id: i64,
    pub guid: String,
    pub status: i32,
    pub last_update: String,
    pub fields: String,
}

#[derive(Serialize)]
pub struct ReviewOverview {
    pub id: i64,
    pub guid: String,
    pub full_path: String,
    pub status: i32, // 0 = new card, 1 = published, but suggestions
    pub last_update: String,
    pub fields: String,
}

#[derive(Clone, Serialize)]
pub struct NoteHistoryEvent {
    pub id: i64,
    pub version: i64,
    pub event_type: String,
    pub actor_user_id: Option<i32>,
    pub actor_username: Option<String>,
    pub commit_id: Option<i32>,
    pub approved: Option<bool>,
    pub old_value: Option<serde_json::Value>,
    pub new_value: Option<serde_json::Value>,
    pub created_at: String,
    pub old_human: Option<String>,
    pub new_human: Option<String>,
    pub snapshot_field_count: Option<usize>,
    pub snapshot_tags: Option<Vec<String>>,
    pub diff_html: Option<String>,
}

#[derive(Clone, Serialize, Default)]
pub struct NoteHistoryGroup {
    pub commit_id: Option<i32>,
    pub approved: bool,
    pub denied: bool,
    pub events: Vec<NoteHistoryEvent>,
}

#[derive(Serialize)]
pub struct CommitsOverview {
    pub id: i32,
    pub rationale: String,
    pub commit_info: String,
    pub timestamp: String,
    pub deck: String,
    pub user: String,
}

#[derive(Serialize)]
pub struct FieldsReviewInfo {
    pub id: i64,
    pub position: u32,
    pub content: String,
    pub reviewed_content: String,
    pub diff: String,
}

#[derive(Serialize)]
pub struct CommitData {
    pub commit_id: i32,
    // all these fields are note_x fields.
    pub id: i64,
    pub guid: String,
    pub deck: String,
    pub owner: i32,
    pub note_model: i64,
    pub last_update: String,
    pub reviewed: bool,
    pub delete_req: bool,
    pub move_req: Option<NoteMoveReq>,
    pub fields: Vec<FieldsReviewInfo>,
    pub new_tags: Vec<TagsInfo>,
    pub removed_tags: Vec<TagsInfo>,
}

#[derive(Serialize)]
pub struct FieldsInfo {
    pub id: i64,
    pub position: u32,
    pub content: String,
    pub inherited: bool,
}

#[derive(Serialize)]
pub struct TagsInfo {
    pub id: i64,
    pub content: String,
    pub inherited: bool,
    pub commit_id: i32,
}

#[derive(Serialize)]
pub struct NoteData {
    pub id: i64,
    pub guid: String,
    pub owner: i32,
    pub deck: String,
    pub last_update: String,
    pub reviewed: bool,
    pub delete_req: bool,
    pub is_inherited: bool,
    pub reviewed_fields: Vec<FieldsInfo>,
    pub reviewed_tags: Vec<TagsInfo>,
    pub unconfirmed_fields: Vec<FieldSuggestionInfo>,
    pub new_tags: Vec<TagsInfo>,
    pub removed_tags: Vec<TagsInfo>,
    pub note_model_fields: Vec<String>,
    pub note_move_decks: Vec<NoteMoveReq>,
}

#[derive(Clone, Serialize)]
pub struct CommitHistoryEvent {
    pub id: i64,
    pub version: i64,
    pub event_type: String,
    pub actor_username: Option<String>,
    pub created_at: String,
    pub old_human: Option<String>,
    pub new_human: Option<String>,
    pub diff_html: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct CommitHistoryNote {
    pub note_id: i64,
    pub min_version: i64,
    pub max_version: i64,
    pub event_types: Vec<String>,
    pub events: Vec<CommitHistoryEvent>,
    pub field_added: usize,
    pub field_updated: usize,
    pub field_removed: usize,
    pub tag_added: usize,
    pub tag_removed: usize,
    pub moved: bool,
    pub deleted: bool,
}

#[derive(Serialize)]
pub struct FieldSuggestionInfo {
    pub id: i64,
    pub position: u32,
    pub commit_id: i32,
    pub content: String,
    pub diff: String,
}

#[derive(Serialize)]
pub struct NoteMoveReq {
    pub id: i32,
    pub path: String,
}

/* Decks */
#[derive(Serialize)]
pub struct BasicDeckInfo {
    pub name: String,
    pub human_hash: String,
}

#[derive(Serialize)]
pub struct DeckOverview {
    pub owner: i32,
    pub desc: String,
    pub name: String,
    pub hash: String,
    pub last_update: String,
    pub id: i64,
    pub notes: String,
    pub children: Vec<BasicDeckInfo>,
    pub subscriptions: i64,
    pub stats_enabled: bool,
}

#[derive(Serialize)]
pub struct NoteModelFieldInfo {
    pub id: i64,
    pub name: String,
    pub protected: bool,
}

#[derive(Deserialize, Serialize)]
pub struct ErrorPayload {
    pub status: String,
    pub message: String,
}

#[derive(Serialize)]
pub struct NoteModel {
    pub id: i64,
    pub fields: Vec<NoteModelFieldInfo>,
    pub name: String,
}

#[derive(Deserialize, Serialize)]
pub struct EditDecksData {
    pub description: String,
    pub hash: String,
    pub is_private: bool,
    pub prevent_subdecks: bool,
    pub restrict_notetypes: bool,
    pub changelog: String,
}

#[derive(Deserialize, Serialize)]
pub struct ChangelogInfo {
    pub id: i64,
    pub message: String,
    pub timestamp: String,
}

#[derive(Deserialize, Serialize)]
pub struct UpdateMaintainer {
    pub deck: String,
    pub username: String,
    pub action: i32, // 1 = add, 0 = remove
}

#[derive(Deserialize, Serialize)]
pub struct UpdateOptionalTag {
    pub deck: String,
    pub taggroup: String,
    pub action: i32, // 1 = add, 0 = remove
}

#[derive(Deserialize, Serialize)]
pub struct UpdateFieldSuggestion {
    pub field_id: i64,
    pub content: String,
}

#[derive(Deserialize, Serialize)]
pub struct UpdateNotetype {
    pub items: HashMap<i64, bool>,
    pub styling: String,
    pub notetype_id: i64,
    pub templates: Vec<UpdateNotetypeTemplate>,
}

#[derive(Deserialize, Serialize)]
pub struct UpdateNotetypeTemplate {
    pub front: String,
    pub back: String,
    pub template_id: i64,
    pub name: String,
}

#[derive(Deserialize, Serialize)]
pub struct NotetypeOverview {
    pub id: i64,
    pub name: String,
    pub notecount: i64,
}

#[derive(Serialize, Deserialize)]
pub struct GoogleServiceAccount {
    pub r#type: String,
    pub project_id: String,
    pub private_key_id: String,
    pub private_key: String,
    pub client_email: String,
    pub client_id: String,
    pub auth_uri: String,
    pub token_uri: String,
    pub auth_provider_x509_cert_url: String,
    pub client_x509_cert_url: String,
}

#[derive(Serialize, Deserialize)]
pub struct GDriveInfo {
    pub deck: String,
    pub service_account: GoogleServiceAccount,
    pub folder_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct DeckStatsInfo {
    pub hash: String,
    pub path: String,
    pub retention: f32,
}

#[derive(Serialize, Deserialize)]
pub struct NoteStatsInfo {
    pub id: i64,
    pub fields: String,
    pub lapses: f32,
    pub reps: f32,
    pub retention: f32,
    pub sample_size: i32,
}

#[derive(Serialize, Deserialize)]
pub struct DeckBaseStatsInfo {
    pub note_count: i32,
    pub lapses_avg: f64,
    pub reps_avg: f64,
    pub retention_avg: f32,
}

#[derive(Serialize, Deserialize)]
pub struct PresignedURLRequest {
    pub filename: String,
    pub context_type: String,
    pub context_id: String, // Note id
}

#[derive(Serialize, Deserialize)]
pub struct PresignedURLResponse {
    pub success: bool,
    pub presigned_url: String,
}

// Subscription policy API
#[derive(Serialize, Deserialize, Clone)]
pub struct SubscriptionPolicyItem {
    pub notetype_id: i64,
    pub subscribed_fields: Option<Vec<i32>>, // None = subscribe all; Some(vec) = only these positions; no row = local-only
}

#[derive(Serialize, Deserialize)]
pub struct SubscriptionPolicyGetResponse {
    pub policies: Vec<SubscriptionPolicyItem>,
}

#[derive(Serialize, Deserialize)]
pub struct SubscriptionPolicyPostRequest {
    pub subscriber_deck_hash: String,
    pub base_deck_hash: String,
    pub policies: Vec<SubscriptionPolicyItem>,
}
