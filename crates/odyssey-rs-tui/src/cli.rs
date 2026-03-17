//! Command-line interface definition.

use clap::Parser;
use std::path::PathBuf;

/// Command-line options for the TUI client.
#[derive(Parser)]
#[command(name = "odyssey-rs-tui", version)]
pub struct Cli {
    /// Bundle reference to run, such as `hello-world@latest`.
    #[arg(long, short = 'b')]
    pub bundle: Option<String>,
    /// Optional working directory label shown in the header.
    #[arg(long)]
    pub cwd: Option<PathBuf>,
    /// Optional user name shown in the header.
    #[arg(long)]
    pub user: Option<String>,
}
