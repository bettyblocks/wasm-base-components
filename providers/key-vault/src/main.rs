mod provider;

use provider::KeyVaultProvider;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    KeyVaultProvider::run().await?;
    eprintln!("KeyVaultProvider exiting");
    Ok(())
}
