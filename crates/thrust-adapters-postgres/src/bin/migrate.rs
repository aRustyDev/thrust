//! `DATABASE_URL=... migrate` — applies the adapter's own SQL. Native only (feature `native`).
//! The URL is read from the environment (it contains the password), never from argv.

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .expect("connect");
    let driver = tokio::spawn(conn);
    thrust_adapters_postgres::migrate(&client).await.expect("migrate");
    drop(client);
    let _ = driver.await;
    eprintln!("migrated");
}
