use bb8_postgres::{tokio_postgres::NoTls, PostgresConnectionManager};
use bb8_postgres::bb8::Pool;
use once_cell::sync::OnceCell;

pub(crate) static TOKIO_POSTGRES_POOL: OnceCell<Pool<PostgresConnectionManager<NoTls>>> = OnceCell::new();

pub async fn establish_connection() -> Result<Pool<PostgresConnectionManager<NoTls>>, Box<dyn std::error::Error + Send + Sync + 'static>> {
    let conn_manager = PostgresConnectionManager::new_from_stringlike(
        "postgresql://postgres:password@localhost/anki",
        NoTls,
    ).unwrap();

    let pool = Pool::builder().max_size(15).build(conn_manager).await?;
    Ok(pool)
}