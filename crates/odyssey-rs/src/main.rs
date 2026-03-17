use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    odyssey_rs::init_logging();
    let cli = odyssey_rs::cli::Cli::parse();
    odyssey_rs::cli::run_cli(cli).await
}
