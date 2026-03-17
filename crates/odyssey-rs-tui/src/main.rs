//! Terminal UI for interacting with the embedded Odyssey runtime.

use clap::Parser;
use odyssey_rs_runtime::{RuntimeConfig, RuntimeEngine};
use odyssey_rs_tui::{TuiRunConfig, resolve_bundle_ref};
use std::sync::Arc;

/// Entry point for the Odyssey TUI client.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = env_logger::builder().format_timestamp_millis().try_init();
    let cli = odyssey_rs_tui::cli::Cli::parse();
    let runtime = Arc::new(RuntimeEngine::new(RuntimeConfig::default())?);
    let bundle_ref = resolve_bundle_ref(&runtime, cli.bundle)?;
    odyssey_rs_tui::run(
        runtime,
        TuiRunConfig {
            bundle_ref,
            user_name: cli.user,
            cwd: cli.cwd,
        },
    )
    .await
}
