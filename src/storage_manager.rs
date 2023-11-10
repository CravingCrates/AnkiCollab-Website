use std::path::Path;

use s3::bucket::Bucket;
use s3::creds::Credentials;
use s3::error::S3Error;
use s3::request_trait::ResponseData;
use s3::Region;

pub struct Storage(Bucket);

pub type ContentId<'a> = &'a str;

impl Storage {
    pub async fn get(id: ContentId<'_>) -> Result<ResponseData, S3Error> {
        Self::default().0.get_object(id).await
    }

    pub async fn put(id: ContentId<'_>, file: &Path) -> Result<ResponseData, S3Error> {
        let content = std::fs::read(file).unwrap();
        Self::default().0.put_object(id, &content).await
    }

    pub async fn delete(id: ContentId<'_>) -> Result<ResponseData, S3Error> {
        Self::default().0.delete_object(id).await
    }

    pub async fn get_to_text(id: ContentId<'_>) -> Result<String, S3Error> {
        let res = Self::get(id).await?;
        let bytes = res.bytes().to_vec();
        let text = String::from_utf8(bytes).unwrap(); // is this ok?
        Ok(text)
    }
}

impl Default for Storage {
    fn default() -> Self {
        let r2_url = std::env::var("R2_URL").unwrap();
        let credentials = Credentials::new(
            std::env::var("R2_ACCESS_KEY").ok().as_deref(), // access_key
            std::env::var("R2_SECRET_KEY").ok().as_deref(), // secret_key
            None,
            None,
            None,
        )
        .unwrap();
        let region = Region::Custom {
            region: "auto".to_string(),
            endpoint: r2_url,
        };

        let bucket = Bucket::new("ankinator", region, credentials)
            .unwrap()
            .with_path_style();

        Storage(bucket)
    }
}
