pub mod provider;

use provider::DataAPIProvider;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    DataAPIProvider::run().await?;
    eprintln!("DataAPIProvider exiting");
    Ok(())
}
