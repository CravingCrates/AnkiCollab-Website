use std::ops::Deref;

use rocket::{get, FromFormField};
use rocket_auth::User;
use s3::bucket::Bucket;
use s3::creds::Credentials;
use s3::error::S3Error;
use s3::Region;

use crate::database;
use crate::structs::Return;

pub struct Storage(Bucket);

pub type ContentId<'a> = &'a str;

impl Deref for Storage {
    type Target = Bucket;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Storage {
    pub fn presigned_get(id: ContentId<'_>) -> Result<String, S3Error> {
        Self::default().presign_get(id, 3600, None)
    }

    pub fn presigned_put(id: ContentId<'_>) -> Result<String, S3Error> {
        Self::default().presign_put(id, 3600, None)
    }

    pub fn presigned_delete(id: ContentId<'_>) -> Result<String, S3Error> {
        Self::default().presign_delete(id, 3600)
    }
}

impl Default for Storage {
    fn default() -> Self {
        let account_id = std::env::var("R2_ACCOUNT_ID").unwrap();
        let credentials = Credentials::new(
            std::env::var("R2_ACCESS_KEY").ok().as_deref(), // access_key
            std::env::var("R2_SECRET_KEY").ok().as_deref(), // secret_key
            None,
            None,
            None,
        )
        .unwrap();
        let region = Region::R2 { account_id };

        let bucket = Bucket::new("ankicollab", region, credentials)
            .unwrap()
            .with_path_style();

        Storage(bucket)
    }
}

#[derive(Debug, PartialEq, FromFormField)]
pub enum Method {
    Get,
    Put,
    Delete,
}

/// Create table media (
/// id SERIAL PRIMARY KEY,
/// user_id INTEGER REFERENCES users(id),
/// public_id UUID DEFAULT uuid_generate_v4()
/// );
///
#[get("/media/presigned?<method>")]
pub async fn get_presigned_url(user: User, method: Method) -> Return<String> {
    let client = database::client().await?;
    let user_id = user.id();

    let id = client
        .query(
            "insert into anki.media VALUES (user_id) returning public_id",
            &[&user_id],
        )
        .await?;

    let id = id[0].get(0);

    let url = match method {
        Method::Get => Storage::presigned_get(id),
        Method::Put => Storage::presigned_put(id),
        Method::Delete => Storage::presigned_delete(id),
    };

    Ok(url?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn up_and_download_presigned() {
        dotenvy::dotenv().unwrap();
        let original = "Storage works!";

        let up_url = Storage::presigned_put("/test.txt").unwrap();

        let client = reqwest::blocking::Client::new();

        client.put(up_url).body(original).send().unwrap();

        let down_url = Storage::presigned_get("/test.txt").unwrap();

        let content = client.get(down_url).send().unwrap().text().unwrap();

        assert_eq!(&content, original);

        let del_url = Storage::presigned_delete("/test.txt").unwrap();
        client.delete(del_url).send().unwrap();
    }
}
