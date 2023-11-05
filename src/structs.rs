use rocket::serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub type Return<T> = Result<T, crate::error::Error>;
pub type DeckHash = String;
pub type UserId = i32;
pub type DeckId = i64;
pub type NoteId = i64;
pub type FieldId = i64;

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

#[derive(Serialize)]
pub struct CommitsOverview {
    pub id: i32,
    pub rationale: String,
    pub timestamp: String,
    pub deck: String,
}

#[derive(Serialize)]
pub struct FieldsReviewInfo {
    pub id: i64,
    pub position: u32,
    pub content: String,
    pub reviewed_content: String,
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
    pub fields: Vec<FieldsReviewInfo>,
    pub new_tags: Vec<TagsInfo>,
    pub removed_tags: Vec<TagsInfo>,
}

#[derive(Serialize)]
pub struct FieldsInfo {
    pub id: i64,
    pub position: u32,
    pub content: String,
}

#[derive(Serialize)]
pub struct TagsInfo {
    pub id: i64,
    pub content: String,
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
    pub reviewed_fields: Vec<FieldsInfo>,
    pub reviewed_tags: Vec<TagsInfo>,
    pub unconfirmed_fields: Vec<FieldsInfo>,
    pub new_tags: Vec<TagsInfo>,
    pub removed_tags: Vec<TagsInfo>,
    pub note_model_fields: Vec<String>,
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
    pub notes: i64,
    pub children: Vec<BasicDeckInfo>,
    pub subscriptions: i64,
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
    pub email: String,
    pub action: i32, // 1 = add, 0 = remove
}

#[derive(Deserialize, Serialize)]
pub struct UpdateOptionalTag {
    pub deck: String,
    pub taggroup: String,
    pub action: i32, // 1 = add, 0 = remove
}

#[derive(Deserialize, Serialize)]
pub struct UpdateNotetype {
    pub items: HashMap<i64, bool>,
    pub front: String,
    pub back: String,
    pub styling: String,
    pub notetype_id: i64,
    pub template_id: i64,
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
