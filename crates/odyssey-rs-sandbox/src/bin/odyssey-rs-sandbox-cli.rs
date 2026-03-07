use clap::{Parser, Subcommand};
use odyssey_rs_config::OdysseyConfig;
use odyssey_rs_protocol::SandboxMode;
use odyssey_rs_sandbox::{
    CommandSpec, SandboxContext, SandboxEnvPolicy, SandboxFilesystemPolicy, SandboxLimits,
    SandboxNetworkMode, SandboxNetworkPolicy, SandboxPolicy, SandboxRunRequest, SandboxRunner,
};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "odyssey-rs-sandbox-cli")]
#[command(about = "Standalone runner for Odyssey sandbox profiles")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Print provider support information.
    Support {
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        mode: Option<String>,
    },
    /// Run a command inside the sandbox.
    Run {
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        mode: Option<String>,
        #[arg(long)]
        workspace: Option<PathBuf>,
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
}

#[tokio::main]
async fn main() {
    if let Err(err) = run_cli().await {
        eprintln!("sandbox-cli error: {err}");
        std::process::exit(1);
    }
}

async fn run_cli() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Support { provider, mode } => {
            let mode = parse_mode(mode.as_deref())?;
            let runner = SandboxRunner::from_provider_name(provider.as_deref(), mode)?;
            let support = runner.support();
            println!("provider: {}", support.provider);
            println!("available: {}", support.available);
            if !support.errors.is_empty() {
                println!("errors:");
                for error in support.errors {
                    println!("- {error}");
                }
            }
            if !support.warnings.is_empty() {
                println!("warnings:");
                for warning in support.warnings {
                    println!("- {warning}");
                }
            }
        }
        Commands::Run {
            config,
            provider,
            mode,
            workspace,
            cwd,
            command,
        } => {
            let config = match config {
                Some(path) => OdysseyConfig::load_from_path(path)?,
                None => OdysseyConfig::default(),
            };
            let sandbox_mode = mode
                .as_deref()
                .map(|value| parse_mode(Some(value)))
                .transpose()?
                .unwrap_or(config.sandbox.mode);
            let workspace_root = match workspace {
                Some(path) => path,
                None => std::env::current_dir()?,
            };

            let runner = SandboxRunner::from_provider_name(
                provider.as_deref().or(config.sandbox.provider.as_deref()),
                sandbox_mode,
            )?;
            let (program, args) = command
                .split_first()
                .ok_or_else(|| "command cannot be empty".to_string())?;

            let mut spec = CommandSpec::new(program);
            spec.args = args.to_vec();
            spec.cwd = cwd;
            let result = runner
                .run(SandboxRunRequest {
                    context: SandboxContext {
                        workspace_root,
                        mode: sandbox_mode,
                        policy: sandbox_policy_from_config(&config.sandbox),
                    },
                    command: spec,
                })
                .await?;

            if !result.stdout.is_empty() {
                print!("{}", result.stdout);
            }
            if !result.stderr.is_empty() {
                eprint!("{}", result.stderr);
            }
            std::process::exit(result.status_code.unwrap_or(1));
        }
    }
    Ok(())
}

fn parse_mode(value: Option<&str>) -> Result<SandboxMode, String> {
    match value.unwrap_or("workspace_write") {
        "read_only" => Ok(SandboxMode::ReadOnly),
        "workspace_write" => Ok(SandboxMode::WorkspaceWrite),
        "danger_full_access" => Ok(SandboxMode::DangerFullAccess),
        other => Err(format!("invalid sandbox mode: {other}")),
    }
}

fn sandbox_policy_from_config(config: &odyssey_rs_config::SandboxConfig) -> SandboxPolicy {
    SandboxPolicy {
        filesystem: SandboxFilesystemPolicy {
            read_roots: config.filesystem.read.clone(),
            write_roots: config.filesystem.write.clone(),
            exec_roots: config.filesystem.exec.clone(),
        },
        env: SandboxEnvPolicy {
            inherit: config.env.inherit.clone(),
            set: config.env.set.clone().into_iter().collect(),
        },
        network: SandboxNetworkPolicy {
            mode: match config.network.mode {
                odyssey_rs_config::SandboxNetworkMode::Disabled => SandboxNetworkMode::Disabled,
                odyssey_rs_config::SandboxNetworkMode::AllowAll => SandboxNetworkMode::AllowAll,
            },
        },
        limits: SandboxLimits {
            cpu_seconds: config.limits.cpu_seconds,
            memory_bytes: config.limits.memory_bytes,
            nofile: config.limits.nofile,
            pids: config.limits.pids,
            wall_clock_seconds: config.limits.wall_clock_seconds,
            stdout_bytes: config.limits.stdout_bytes,
            stderr_bytes: config.limits.stderr_bytes,
        },
    }
}
