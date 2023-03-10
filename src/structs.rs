use rocket::serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    pub deck: String
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
    pub fields:Vec<NoteModelFieldInfo>,
    pub name: String,
}

#[derive(Deserialize, Serialize)]
pub struct EditDecksData {
  pub items: HashMap<i64, bool>,
  pub description: String,
  pub media_url: String,
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