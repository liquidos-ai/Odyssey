use anyhow::Result;
use clap::{Parser, Subcommand};
use crossterm::style::Stylize;
use odyssey_rs_protocol::SandboxMode;
use odyssey_rs_runtime::{RuntimeConfig, RuntimeEngine};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "odyssey-rs", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    Init {
        path: String,
    },
    Build {
        path: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Inspect {
        reference: String,
    },
    Run {
        reference: String,
        #[arg(long)]
        prompt: String,
        #[arg(long)]
        dangerous_sandbox_mode: bool,
    },
    Serve {
        #[arg(long)]
        bind: Option<String>,
        #[arg(long)]
        dangerous_sandbox_mode: bool,
    },
    Publish {
        source: String,
        #[arg(long)]
        to: String,
        #[arg(long = "hub", visible_alias = "registry")]
        hub: Option<String>,
    },
    Pull {
        reference: String,
        #[arg(long = "hub", visible_alias = "registry")]
        hub: Option<String>,
    },
    Export {
        reference: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Import {
        path: PathBuf,
    },
}

pub async fn run_cli(cli: Cli) -> Result<()> {
    let mut config = RuntimeConfig::default();
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
    }
    if let Some(hub_url) = hub_override(&cli.command) {
        config.hub_url = hub_url;
    }
    let runtime = RuntimeEngine::new(config.clone())?;
    match cli.command {
        Command::Init { path } => {
            runtime.init(&path)?;
            print_init_summary(&path);
        }
        Command::Build { path, output } => {
            if let Some(output) = output {
                let artifact = runtime.build_to(&path, &output)?;
                println!(
                    "{} {}@{} {} {}",
                    "built".green().bold(),
                    artifact.metadata.id,
                    artifact.metadata.version,
                    artifact.metadata.digest,
                    artifact.path.display()
                );
            } else {
                let install = runtime.build_and_install(path)?;
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
            let metadata = runtime.inspect_bundle(&reference)?;
            println!("{}", "bundle metadata".cyan().bold());
            println!("{}", serde_json::to_string_pretty(&metadata)?);
        }
        Command::Run {
            reference, prompt, ..
        } => {
            let session = runtime.create_session(&reference)?;
            let result = runtime.run(session.id, prompt).await?;
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
        Command::Publish { source, to, .. } => {
            let published = runtime.publish(&source, &to).await?;
            println!(
                "{} {} {}",
                "published".green().bold(),
                format!("{}@{}", published.id, published.version).cyan(),
                published.digest.cyan()
            );
        }
        Command::Pull { reference, .. } => {
            let install = runtime.pull(&reference).await?;
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
            let path = runtime.export_bundle(&reference, output)?;
            println!("{} {}", "exported".green().bold(), path.display());
        }
        Command::Import { path } => {
            let install = runtime.import_bundle(path)?;
            println!(
                "{} {}/{}@{}",
                "imported".green().bold(),
                install.metadata.namespace,
                install.metadata.id,
                install.metadata.version
            );
        }
    }
    Ok(())
}

fn hub_override(command: &Command) -> Option<String> {
    match command {
        Command::Publish { hub: Some(hub), .. } | Command::Pull { hub: Some(hub), .. } => {
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
        "set key:".dark_grey().bold(),
        "export OPENAI_API_KEY=\"your-key\"".cyan()
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
