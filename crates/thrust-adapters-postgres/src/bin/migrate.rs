//! `DATABASE_URL=... migrate` — applies the adapter's own SQL. Native only (feature `native`).
//! The URL is read from the environment (it contains the password), never from argv.

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL must be set")?;
    let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await?;
    let driver = tokio::spawn(conn);
    thrust_adapters_postgres::migrate(&client).await?;
    drop(client);
    let _ = driver.await;
    eprintln!("migrated");
    Ok(())
}
