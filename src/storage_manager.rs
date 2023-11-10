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

#[derive(FromForm)]
pub struct Upload<'r> {
    file: TempFile<'r>,
}
#[derive(Serialize)]
pub struct UploadResponse {
    hash: String,
    id: i32,
}

#[get("/doc/<id>")]
pub async fn get_doc(user: Token, db: &DB, id: i32) -> Return<Vec<u8>> {
    let material = sqlx::query!(
        "SELECT title, digest FROM documents WHERE id = $1 and user_id = $2 and archived = False",
        id,
        user.id
    )
    .fetch_optional(&db.0)
    .await?;

    if material.is_none() {
        return Err(Error::DocumentNotFound);
    }

    let material = material.unwrap();

    let stor = Storage::new();
    let file = stor.get(&material.digest).await?;

    Ok(file.bytes().to_owned())
}

#[post("/doc", data = "<form>")]
pub async fn upload_doc(
    user: Token,
    db: &DB,

    mut form: Form<Upload<'_>>,
) -> Return<Json<UploadResponse>> {
    let file = &mut form.file;
    let name = file.name().unwrap();
    let file_type = file.content_type().unwrap().to_string();
    let extension = file
        .content_type()
        .unwrap()
        .extension()
        .unwrap_or("unknown".into())
        .to_string();

    let name = name.replace(' ', "_");
    let name = name.replace('.', "_");
    let name = format!("{}.{}", name, extension);

    if file.path().is_none() {
        file.persist_to(format!("./tmp/{}_{}", user.id, file.len()))
            .await?
    };

    let path = file.path().unwrap();
    let mut hasher = Sha256::new();
    let mut f = fs::File::open(path)?;
    std::io::copy(&mut f, &mut hasher)?;
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&hasher.finalize());

    let res = sqlx::query!(
        "INSERT INTO documents (user_id, title, type, size, digest)
                VALUES ($1, $2, $3, $4, $5) returning id ",
        user.id,
        name,
        file_type,
        file.len() as i32,
        &hash
    )
    .fetch_one(&db.0)
    .await?;

    let identical = sqlx::query!("SELECT Count(*) FROM documents WHERE digest = $1 ", &hash,)
        .fetch_one(&db.0)
        .await?;
    let hash_str = Storage::base_64(&hash);

    // If there is more than one file with the same hash, we don't need to upload it again
    if identical.count.unwrap() > 1 {
        return Ok(Json(UploadResponse {
            hash: hash_str.to_string(),
            id: res.id,
        }));
    }

    let upload_timing = Instant::now();
    let stor = Storage::new();

    stor.put(&hash, path).await?;
    fs::remove_file(path)?;

    info!("Uploaded file {} in {}", name, upload_timing.elapsed());

    Ok(Json(UploadResponse {
        hash: hash_str.to_string(),
        id: res.id,
    }))
}

#[delete("/doc/<id>")]
pub async fn archive_doc(user: Token, db: &DB, id: i32) -> Return<Json<IdResponse>> {
    let res = sqlx::query!(
        "UPDATE documents SET archived = True WHERE id = $1 and user_id = $2 returning digest",
        id,
        user.id
    )
    .fetch_optional(&db.0)
    .await?;

    if res.is_none() {
        return Err(Error::DocumentNotFound);
    }

    let res = res.unwrap();

    // turn bytearray digest into Base64 string

    let identical = sqlx::query!(
        "SELECT Count(*) FROM documents WHERE digest = $1 AND archived = FALSE",
        &res.digest,
    )
    .fetch_one(&db.0)
    .await?;

    dbg!(identical.count);

    // If we deleted the last file with this hash, we can delete it from the storage
    if identical.count == Some(0) {
        let stor = Storage::new();

        stor.delete(&res.digest).await?;

        sqlx::query!("DELETE FROM documents WHERE digest = $1", &res.digest)
            .execute(&db.0)
            .await?;
        p
    }

    Ok(Json(IdResponse { id }))
}
