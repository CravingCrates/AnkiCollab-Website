use std::env;
use std::sync::Arc;

use bb8_postgres::bb8::{Pool, PooledConnection};
use bb8_postgres::{tokio_postgres::NoTls, PostgresConnectionManager};

use crate::{DeckHash, DeckId, Return, UserId};
use crate::error::Error::*;

use aws_sdk_s3::Client as S3Client;
use tera::Tera;

#[derive(Debug)]
pub struct AppState {
    pub db_pool: Arc<Pool<PostgresConnectionManager<NoTls>>>,
    pub tera: Arc<Tera>,
    pub s3_client: S3Client,
}

pub async fn establish_pool_connection() -> Result<
    Pool<PostgresConnectionManager<NoTls>>,
    Box<dyn std::error::Error + Send + Sync + 'static>,
> {
    let conn_manager = PostgresConnectionManager::new_from_stringlike(
        env::var("DATABASE_URL").expect("Expected DATABASE_URL to exist in the environment"),
        NoTls,
    )
    .unwrap();

    let pool = Pool::builder().max_size(15).build(conn_manager).await?;
    Ok(pool)
}

pub async fn client(db_state: &Arc<AppState>) -> Return<PooledConnection<'_, PostgresConnectionManager<NoTls>>> {
    match db_state.db_pool.get().await {
        Ok(pool) => Ok(pool),
        Err(err) => {
            println!("Error getting pool: {}", err);
            Err(DatabaseConnection)
        },
    }
}

pub async fn owned_deck_id(db_state: &Arc<AppState>, deck_hash: &DeckHash, user_id: UserId) -> Return<DeckId> {
    let owned_info = client(db_state)
        .await?
        .query(
            "Select id from decks where human_hash = $1 and owner = $2",
            &[&deck_hash, &user_id],
        )
        .await?;

    match owned_info.is_empty() {
        true => Err(Unauthorized),
        false => Ok(owned_info[0].get(0)),
    }
}
