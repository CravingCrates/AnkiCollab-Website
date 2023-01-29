use rocket::tokio;
use tokio_postgres::{Client, NoTls, Error};

pub static mut TOKIO_POSTGRES_CLIENT: Option<Client> = None;

pub async fn establish_connection()-> Result<(), Error> {
    unsafe {        
        let (client, connection) =
        tokio_postgres::connect("postgresql://postgres:password@localhost/anki", NoTls).await?;

        // The connection object performs the actual communication with the database,
        // so spawn it off to run on its own.
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("connection error: {}", e);
            }
        });
        TOKIO_POSTGRES_CLIENT = Some(client);
        Ok(())
    }
}
