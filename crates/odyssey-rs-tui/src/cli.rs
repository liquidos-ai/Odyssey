//! Command-line interface definition.

use clap::Parser;
use std::path::PathBuf;

/// Command-line options for the TUI client.
#[derive(Parser)]
#[command(name = "odyssey-rs-tui", version)]
pub struct Cli {
    /// Optional path to an odyssey.json5 config file
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// OpenAI model name for the default agent
    #[arg(long)]
    pub model: Option<String>,
    /// Default agent id
    #[arg(long)]
    pub agent: Option<String>,

    // ── Local llama.cpp options ───────────────────────────────────────────────
    /// Enable the local llama.cpp provider
    #[cfg(feature = "local")]
    #[arg(long)]
    pub local: bool,
    /// Local GGUF model path (mutually exclusive with --local-hf-repo)
    #[cfg(feature = "local")]
    #[arg(long)]
    pub local_gguf: Option<PathBuf>,
    /// HuggingFace repo id for a GGUF model
    #[cfg(feature = "local")]
    #[arg(long)]
    pub local_hf_repo: Option<String>,
    /// Optional HuggingFace GGUF filename
    #[cfg(feature = "local")]
    #[arg(long)]
    pub local_hf_filename: Option<String>,
    /// Optional HuggingFace mmproj filename
    #[cfg(feature = "local")]
    #[arg(long)]
    pub local_hf_mmproj: Option<String>,
    /// Optional chat template name or inline template
    #[cfg(feature = "local")]
    #[arg(long)]
    pub local_chat_template: Option<String>,
    /// Context size override
    #[cfg(feature = "local")]
    #[arg(long)]
    pub local_n_ctx: Option<u32>,
    /// Thread count override
    #[cfg(feature = "local")]
    #[arg(long)]
    pub local_n_threads: Option<i32>,
    /// Max tokens to generate
    #[cfg(feature = "local")]
    #[arg(long, default_value_t = 2048)]
    pub local_max_tokens: u32,
    /// Sampling temperature
    #[cfg(feature = "local")]
    #[arg(long)]
    pub local_temperature: Option<f32>,
    /// GPU layers to offload
    #[cfg(feature = "local")]
    #[arg(long)]
    pub local_n_gpu_layers: Option<u32>,
    /// Main GPU index
    #[cfg(feature = "local")]
    #[arg(long)]
    pub local_main_gpu: Option<i32>,
}

/// Returns true when the local llama.cpp provider flag was set.
pub fn local_enabled(cli: &Cli) -> bool {
    #[cfg(feature = "local")]
    {
        cli.local
    }
    #[cfg(not(feature = "local"))]
    {
        let _ = cli;
        false
    }
}
