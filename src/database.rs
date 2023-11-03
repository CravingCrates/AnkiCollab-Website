use std::env;

use bb8_postgres::bb8::{Pool, PooledConnection};
use bb8_postgres::{tokio_postgres::NoTls, PostgresConnectionManager};
use once_cell::sync::OnceCell;

use crate::error::Error;
use crate::{DeckHash, DeckId, Return, UserId};

pub(crate) static TOKIO_POSTGRES_POOL: OnceCell<Pool<PostgresConnectionManager<NoTls>>> =
    OnceCell::new();

pub async fn establish_connection() -> Result<
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

pub async fn client() -> Return<PooledConnection<'static, PostgresConnectionManager<NoTls>>> {
    Ok(TOKIO_POSTGRES_POOL.get().unwrap().get().await?)
}

pub async fn owned_deck_id(deck_hash: &DeckHash, user_id: UserId) -> Return<DeckId> {
    let owned_info = client()
        .await?
        .query(
            "Select id from decks where human_hash = $1 and owner = $2",
            &[&deck_hash, &user_id],
        )
        .await?;

    match owned_info.is_empty() {
        true => Err(Error::Unauthorized),
        false => Ok(owned_info[0].get(0)),
    }
}
