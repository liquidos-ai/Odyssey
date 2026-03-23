use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use crossterm::style::Stylize;
use log::info;
use odyssey_rs_bundle::{BundleBuilder, BundleProject, BundleStore};
use odyssey_rs_protocol::{ExecutionRequest, SandboxMode, SessionSpec, Task};
use odyssey_rs_runtime::{OdysseyRuntime, RuntimeConfig};
use odyssey_rs_tui::{TuiRunConfig, resolve_bundle_ref};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::time::Instant;
use uuid::Uuid;

use crate::remote::RemoteRuntimeClient;

#[derive(Parser)]
#[command(name = "odyssey-rs", version)]
pub struct Cli {
    #[arg(long, global = true)]
    pub remote: Option<String>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    #[command(about = "Initialize a new Odyssey project")]
    Init { path: String },
    #[command(about = "Build Odyssey agent bundle")]
    Build {
        path: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    #[command(about = "Inspect Odyssey agent bundle")]
    Inspect { reference: String },
    #[command(about = "Run an Odyssey agent bundle")]
    Run {
        #[arg(help = "The bundle reference")]
        reference: String,
        #[arg(long)]
        prompt: String,
        #[arg(long)]
        dangerous_sandbox_mode: bool,
    },
    #[command(
        about = "Start a local runtimer server",
        long_about = "Starts a runtime server and listens for incoming requests. Useful when running multiple agent instancecs on a shared resources."
    )]
    Serve {
        #[arg(long)]
        bind: Option<String>,
        #[arg(long)]
        dangerous_sandbox_mode: bool,
    },
    #[command(about = "Publish Odyssey agent bundle to Hub")]
    Push {
        source: String,
        #[arg(long)]
        to: String,
        #[arg(long = "hub", visible_alias = "registry")]
        hub: Option<String>,
    },
    #[command(about = "Pull Odyssey agent bundle to Hub")]
    Pull {
        reference: String,
        #[arg(long = "hub", visible_alias = "registry")]
        hub: Option<String>,
    },
    #[command(about = "Export Odyssey to a .odyssey file")]
    Export {
        reference: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    #[command(about = "Import and Install from .odyssey file")]
    Import { path: PathBuf },
    #[command(about = "List Installed bundles")]
    Bundles,
    #[command(about = "List sessions")]
    Sessions,
    #[command(about = "Get or Delete Session")]
    Session {
        id: Uuid,
        #[arg(long)]
        delete: bool,
    },
    #[command(about = "Run the TUI")]
    Tui {
        /// Bundle reference to run, such as `hello-world@latest`.
        #[arg(long, short = 'b')]
        bundle: Option<String>,
        /// Optional working directory label shown in the header.
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long)]
        dangerous_sandbox_mode: bool,
    },
}

pub async fn run_cli(cli: Cli) -> Result<()> {
    info!("Running Odyssey CLI");
    validate_remote_usage(&cli)?;
    let config = build_runtime_config(&cli);
    let start_time = Instant::now();
    let runtime = OdysseyRuntime::new(config.clone())?;
    let bundles = BundleStore::new(config.cache_root.clone());
    let end_time = start_time.elapsed();
    info!("Initiated Runtime and Bundle in {:?}", end_time);
    let remote = cli
        .remote
        .as_deref()
        .map(RemoteRuntimeClient::new)
        .transpose()?;
    execute_command(cli.command, &config, &runtime, &bundles, remote.as_ref()).await?;
    Ok(())
}

fn build_runtime_config(cli: &Cli) -> RuntimeConfig {
    let mut config = RuntimeConfig::default();
    if let Some(remote) = &cli.remote {
        config.bind_addr.clone_from(remote);
    }
    if let Command::Serve {
        bind: Some(bind), ..
    } = &cli.command
    {
        config.bind_addr.clone_from(bind);
    }
    if dangerous_sandbox_mode_enabled(&cli.command) {
        config.sandbox_mode_override = Some(SandboxMode::DangerFullAccess);
        info!("Registered DangerFullAccess Sandbox mode");
    }
    if let Some(hub_url) = hub_override(&cli.command) {
        config.hub_url = hub_url;
    }
    config
}

fn dangerous_sandbox_mode_enabled(command: &Command) -> bool {
    matches!(
        command,
        Command::Run {
            dangerous_sandbox_mode: true,
            ..
        } | Command::Serve {
            dangerous_sandbox_mode: true,
            ..
        } | Command::Tui {
            dangerous_sandbox_mode: true,
            ..
        }
    )
}

fn validate_remote_usage(cli: &Cli) -> Result<()> {
    if cli.remote.is_some() && !remote_command_supported(&cli.command) {
        return Err(anyhow!("--remote is not supported with this command"));
    }
    Ok(())
}

fn remote_command_supported(command: &Command) -> bool {
    matches!(
        command,
        Command::Inspect { .. }
            | Command::Run { .. }
            | Command::Pull { .. }
            | Command::Bundles
            | Command::Sessions
            | Command::Session { .. }
    )
}

async fn execute_command(
    command: Command,
    config: &RuntimeConfig,
    runtime: &OdysseyRuntime,
    bundles: &BundleStore,
    remote: Option<&RemoteRuntimeClient>,
) -> Result<()> {
    match command {
        Command::Init { path } => handle_init(runtime, &path),
        Command::Build { path, output } => handle_build(bundles, &path, output),
        Command::Inspect { reference } => handle_inspect(bundles, remote, &reference).await,
        Command::Run {
            reference, prompt, ..
        } => handle_run(runtime, remote, reference, prompt).await,
        Command::Serve { .. } => handle_serve(config.clone()).await,
        Command::Push { source, to, .. } => handle_push(bundles, config, &source, &to).await,
        Command::Pull { reference, .. } => handle_pull(bundles, remote, config, &reference).await,
        Command::Export { reference, output } => handle_export(bundles, &reference, output),
        Command::Import { path } => handle_import(bundles, path),
        Command::Bundles => handle_bundles(bundles, remote).await,
        Command::Sessions => handle_sessions(runtime, remote).await,
        Command::Session { id, delete } => handle_session(runtime, remote, id, delete).await,
        Command::Tui { bundle, cwd, .. } => handle_tui(bundle, cwd, runtime).await,
    }
}

fn handle_init(runtime: &OdysseyRuntime, path: &str) -> Result<()> {
    runtime.init(path)?;
    print_init_summary(path);
    Ok(())
}

fn handle_build(bundles: &BundleStore, path: &str, output: Option<PathBuf>) -> Result<()> {
    if let Some(output) = output {
        let project = BundleProject::load(path)?;
        let artifact = BundleBuilder::new(project).build(&output)?;
        println!(
            "{} {}@{} {} {}",
            "built".green().bold(),
            artifact.metadata.id,
            artifact.metadata.version,
            artifact.metadata.digest,
            artifact.path.display()
        );
    } else {
        let install = bundles.build_and_install(path)?;
        println!(
            "{} {}@{} {} {}",
            "installed".green().bold(),
            install.metadata.id,
            install.metadata.version,
            install.metadata.digest,
            install.path.display()
        );
    }
    Ok(())
}

async fn handle_inspect(
    bundles: &BundleStore,
    remote: Option<&RemoteRuntimeClient>,
    reference: &str,
) -> Result<()> {
    let metadata = if let Some(remote) = remote {
        remote.inspect(reference).await?
    } else {
        bundles.resolve(reference)?.metadata
    };
    println!("{}", "bundle metadata".cyan().bold());
    println!("{}", serde_json::to_string_pretty(&metadata)?);
    Ok(())
}

async fn handle_run(
    runtime: &OdysseyRuntime,
    remote: Option<&RemoteRuntimeClient>,
    reference: String,
    prompt: String,
) -> Result<()> {
    let result = if let Some(remote) = remote {
        remote.run(reference, prompt).await?
    } else {
        let session = runtime.create_session(SessionSpec::from(reference))?;
        let request_id = Uuid::new_v4();
        runtime
            .run(ExecutionRequest {
                request_id,
                session_id: session.id,
                input: Task::new(prompt),
                turn_context: None,
            })
            .await?
    };
    println!("{}", "assistant".cyan().bold());
    println!("{}", result.response);
    Ok(())
}

async fn handle_serve(config: RuntimeConfig) -> Result<()> {
    println!(
        "{} {}",
        "serving".green().bold(),
        config.bind_addr.as_str().cyan()
    );
    odyssey_rs_server::serve(config).await?;
    Ok(())
}

async fn handle_push(
    bundles: &BundleStore,
    config: &RuntimeConfig,
    source: &str,
    to: &str,
) -> Result<()> {
    let published = bundles.publish(source, to, &config.hub_url).await?;
    println!(
        "{} {} {}",
        "published".green().bold(),
        format!("{}@{}", published.id, published.version).cyan(),
        published.digest.cyan()
    );
    Ok(())
}

async fn handle_pull(
    bundles: &BundleStore,
    remote: Option<&RemoteRuntimeClient>,
    config: &RuntimeConfig,
    reference: &str,
) -> Result<()> {
    let install = if let Some(remote) = remote {
        remote.pull(reference, &config.hub_url).await?
    } else {
        bundles.pull(reference, &config.hub_url).await?
    };
    println!(
        "{} {} {}",
        "pulled".green().bold(),
        format!(
            "{}/{}@{}",
            install.metadata.namespace, install.metadata.id, install.metadata.version
        )
        .cyan(),
        install.path.display()
    );
    Ok(())
}

fn handle_export(bundles: &BundleStore, reference: &str, output: Option<PathBuf>) -> Result<()> {
    let output = output.unwrap_or_else(|| PathBuf::from("."));
    let path = bundles.export(reference, output)?;
    println!("{} {}", "exported".green().bold(), path.display());
    Ok(())
}

fn handle_import(bundles: &BundleStore, path: PathBuf) -> Result<()> {
    let install = bundles.import(path)?;
    println!(
        "{} {}/{}@{}",
        "imported".green().bold(),
        install.metadata.namespace,
        install.metadata.id,
        install.metadata.version
    );
    Ok(())
}

async fn handle_bundles(bundles: &BundleStore, remote: Option<&RemoteRuntimeClient>) -> Result<()> {
    let bundles = if let Some(remote) = remote {
        remote.list_bundles().await?
    } else {
        bundles.list_installed()?
    };
    if bundles.is_empty() {
        println!("{}", "no bundles installed".dark_grey());
    } else {
        for bundle in bundles {
            println!(
                "{} {}",
                format!("{}/{}@{}", bundle.namespace, bundle.id, bundle.version)
                    .cyan()
                    .bold(),
                bundle.path.display()
            );
        }
    }
    Ok(())
}

async fn handle_sessions(
    runtime: &OdysseyRuntime,
    remote: Option<&RemoteRuntimeClient>,
) -> Result<()> {
    let sessions = if let Some(remote) = remote {
        remote.list_sessions().await?
    } else {
        runtime.list_sessions(None)
    };
    if sessions.is_empty() {
        println!("{}", "no sessions".dark_grey());
    } else {
        for session in sessions {
            println!(
                "{} {} {}",
                session.id.to_string().cyan().bold(),
                session.agent_id,
                session.created_at
            );
        }
    }
    Ok(())
}

async fn handle_session(
    runtime: &OdysseyRuntime,
    remote: Option<&RemoteRuntimeClient>,
    id: Uuid,
    delete: bool,
) -> Result<()> {
    if delete {
        if let Some(remote) = remote {
            remote.delete_session(id).await?;
        } else {
            runtime.delete_session(id).await?;
        }
        println!("{} {}", "deleted".green().bold(), id);
    } else {
        let session = if let Some(remote) = remote {
            remote.get_session(id).await?
        } else {
            runtime.get_session(id)?
        };
        println!("{}", serde_json::to_string_pretty(&session)?);
    }
    Ok(())
}

async fn handle_tui(
    bundle: Option<String>,
    cwd: Option<PathBuf>,
    runtime: &OdysseyRuntime,
) -> Result<()> {
    let _ = env_logger::builder().format_timestamp_millis().try_init();
    let bundle_ref = resolve_bundle_ref(runtime, bundle)?;
    let runtime = Arc::new(runtime.clone());

    odyssey_rs_tui::run(runtime, TuiRunConfig { bundle_ref, cwd }).await?;
    Ok(())
}

fn hub_override(command: &Command) -> Option<String> {
    match command {
        Command::Push { hub: Some(hub), .. } | Command::Pull { hub: Some(hub), .. } => {
            Some(hub.clone())
        }
        _ => None,
    }
}

fn print_init_summary(path: &str) {
    let bundle_id = default_bundle_id(Path::new(path));
    let bundle_ref = format!("{bundle_id}@latest");

    println!(
        "{} {}",
        "initialized bundle".green().bold(),
        bundle_id.as_str().cyan().bold()
    );
    println!("{} {}", "path".dark_grey().bold(), path);
    println!();
    println!("{}", "Get Started".yellow().bold());
    println!(
        "{} {}",
        "build:".dark_grey().bold(),
        format!("odyssey-rs -- build {path}").cyan()
    );
    println!(
        "{} {}",
        "run:".dark_grey().bold(),
        format!("odyssey-rs -- run {bundle_ref} --prompt \"Hey, What is your name?\"").cyan()
    );
}

fn default_bundle_id(root: &Path) -> String {
    let raw = root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("hello-world");
    let mut slug = String::with_capacity(raw.len());
    let mut previous_dash = false;
    for ch in raw.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            previous_dash = false;
            ch.to_ascii_lowercase()
        } else {
            if previous_dash {
                continue;
            }
            previous_dash = true;
            '-'
        };
        slug.push(mapped);
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        "hello-world".to_string()
    } else {
        slug.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command, build_runtime_config, default_bundle_id};
    use clap::Parser;
    use odyssey_rs_protocol::SandboxMode;
    use pretty_assertions::assert_eq;
    use std::path::Path;

    #[test]
    fn derives_bundle_id_from_cli_path() {
        assert_eq!(
            default_bundle_id(Path::new("./bundles/My Starter Agent")),
            "my-starter-agent"
        );
    }

    #[test]
    fn tui_cli_accepts_dangerous_sandbox_mode_flag() {
        let cli = Cli::parse_from([
            "odyssey-rs",
            "tui",
            "--bundle",
            "local/demo@0.1.0",
            "--dangerous-sandbox-mode",
        ]);

        match cli.command {
            Command::Tui {
                bundle,
                dangerous_sandbox_mode,
                ..
            } => {
                assert_eq!(bundle, Some("local/demo@0.1.0".to_string()));
                assert!(dangerous_sandbox_mode);
            }
            _ => panic!("unexpected command"),
        }
    }

    #[test]
    fn tui_dangerous_sandbox_mode_overrides_runtime_config() {
        let cli = Cli::parse_from(["odyssey-rs", "tui", "--dangerous-sandbox-mode"]);

        let config = build_runtime_config(&cli);

        assert_eq!(
            config.sandbox_mode_override,
            Some(SandboxMode::DangerFullAccess)
        );
    }
}
