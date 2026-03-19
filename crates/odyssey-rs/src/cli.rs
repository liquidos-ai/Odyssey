use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use crossterm::style::Stylize;
use log::info;
use odyssey_rs_bundle::{BundleBuilder, BundleProject, BundleStore};
use odyssey_rs_protocol::{ExecutionRequest, SandboxMode, SessionSpec, Task};
use odyssey_rs_runtime::{OdysseyRuntime, RuntimeConfig};
use std::path::{Path, PathBuf};
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
}

pub async fn run_cli(cli: Cli) -> Result<()> {
    info!("Running Odyssey CLI");
    let mut config = RuntimeConfig::default();
    if let Some(remote) = &cli.remote {
        config.bind_addr = remote.clone();
    }
    if let Command::Serve {
        bind: Some(bind), ..
    } = &cli.command
    {
        config.bind_addr = bind.clone();
    }
    if matches!(
        &cli.command,
        Command::Run {
            dangerous_sandbox_mode: true,
            ..
        } | Command::Serve {
            dangerous_sandbox_mode: true,
            ..
        }
    ) {
        config.sandbox_mode_override = Some(SandboxMode::DangerFullAccess);
        info!("Registered DangerFullAccess Sandbox mode")
    }
    if let Some(hub_url) = hub_override(&cli.command) {
        config.hub_url = hub_url;
    }
    let use_remote = cli.remote.is_some();
    if use_remote
        && matches!(
            cli.command,
            Command::Build { .. }
                | Command::Serve { .. }
                | Command::Init { .. }
                | Command::Push { .. }
                | Command::Export { .. }
                | Command::Import { .. }
        )
    {
        return Err(anyhow!("--remote is not supported with this command"));
    }
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
    match cli.command {
        Command::Init { path } => {
            runtime.init(&path)?;
            print_init_summary(&path);
        }
        Command::Build { path, output } => {
            if let Some(output) = output {
                let project = BundleProject::load(&path)?;
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
        }
        Command::Inspect { reference } => {
            let metadata = if let Some(remote) = &remote {
                remote.inspect(&reference).await?
            } else {
                bundles.resolve(&reference)?.metadata
            };
            println!("{}", "bundle metadata".cyan().bold());
            println!("{}", serde_json::to_string_pretty(&metadata)?);
        }
        Command::Run {
            reference, prompt, ..
        } => {
            let result = if let Some(remote) = &remote {
                remote.run(reference, prompt).await?
            } else {
                let session = runtime.create_session(SessionSpec::from(reference))?;
                let task = Task::new(prompt);
                let request_id = Uuid::new_v4();
                runtime
                    .run(ExecutionRequest {
                        request_id,
                        session_id: session.id,
                        input: task,
                        turn_context: None,
                    })
                    .await?
            };
            println!("{}", "assistant".cyan().bold());
            println!("{}", result.response);
        }
        Command::Serve { .. } => {
            println!(
                "{} {}",
                "serving".green().bold(),
                config.bind_addr.as_str().cyan()
            );
            odyssey_rs_server::serve(config).await?;
        }
        Command::Push { source, to, .. } => {
            let published = bundles.publish(&source, &to, &config.hub_url).await?;
            println!(
                "{} {} {}",
                "published".green().bold(),
                format!("{}@{}", published.id, published.version).cyan(),
                published.digest.cyan()
            );
        }
        Command::Pull { reference, .. } => {
            let install = if let Some(remote) = &remote {
                remote.pull(&reference, &config.hub_url).await?
            } else {
                bundles.pull(&reference, &config.hub_url).await?
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
        }
        Command::Export { reference, output } => {
            let output = output.unwrap_or_else(|| PathBuf::from("."));
            let path = bundles.export(&reference, output)?;
            println!("{} {}", "exported".green().bold(), path.display());
        }
        Command::Import { path } => {
            let install = bundles.import(path)?;
            println!(
                "{} {}/{}@{}",
                "imported".green().bold(),
                install.metadata.namespace,
                install.metadata.id,
                install.metadata.version
            );
        }
        Command::Bundles => {
            let bundles = if let Some(remote) = &remote {
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
        }
        Command::Sessions => {
            let sessions = if let Some(remote) = &remote {
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
        }
        Command::Session { id, delete } => {
            if delete {
                if let Some(remote) = &remote {
                    remote.delete_session(id).await?;
                } else {
                    runtime.delete_session(id)?;
                }
                println!("{} {}", "deleted".green().bold(), id);
            } else {
                let session = if let Some(remote) = &remote {
                    remote.get_session(id).await?
                } else {
                    runtime.get_session(id)?
                };
                println!("{}", serde_json::to_string_pretty(&session)?);
            }
        }
    }
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
    use super::default_bundle_id;
    use pretty_assertions::assert_eq;
    use std::path::Path;

    #[test]
    fn derives_bundle_id_from_cli_path() {
        assert_eq!(
            default_bundle_id(Path::new("./bundles/My Starter Agent")),
            "my-starter-agent"
        );
    }
}
