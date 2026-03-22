use anyhow::{Context, ensure};
use odyssey_rs_protocol::{ExecutionRequest, SessionSpec, Task};
use odyssey_rs_runtime::{OdysseyRuntime, RuntimeConfig};
use std::env;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const DEFAULT_PROMPT: &str = "Introduce yourself in one sentence and say that the bundle was loaded through the Rust SDK runtime.";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    ensure!(
        env::var_os("OPENAI_API_KEY").is_some(),
        "set OPENAI_API_KEY before running this example"
    );

    let prompt = env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_PROMPT.to_string());
    let workspace_root = workspace_root()?;
    let bundle_root = workspace_root.join("bundles").join("hello-world");
    let runtime = OdysseyRuntime::new(example_runtime_config(&workspace_root))?;

    let install = runtime
        .build_and_install(&bundle_root)
        .with_context(|| format!("failed to build bundle at {}", bundle_root.display()))?;
    let bundle_ref = format!("local/{}@{}", install.metadata.id, install.metadata.version);

    println!("Loaded bundle: {bundle_ref}");
    println!("Install path: {}", install.path.display());

    let session = runtime
        .create_session(SessionSpec::from(bundle_ref.as_str()))
        .with_context(|| format!("failed to create session for {bundle_ref}"))?;

    let result = runtime
        .run(ExecutionRequest {
            request_id: Uuid::new_v4(),
            session_id: session.id,
            input: Task::new(prompt),
            turn_context: None,
        })
        .await
        .with_context(|| format!("failed to execute bundle {bundle_ref}"))?;

    println!("\nAssistant response:\n{}", result.response);
    Ok(())
}

fn workspace_root() -> anyhow::Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .context("failed to resolve workspace root")
}

fn example_runtime_config(workspace_root: &Path) -> RuntimeConfig {
    let example_root = workspace_root
        .join("target")
        .join("examples")
        .join("hello-world");
    RuntimeConfig {
        cache_root: example_root.join("bundles"),
        session_root: example_root.join("sessions"),
        sandbox_root: example_root.join("sandbox"),
        ..RuntimeConfig::default()
    }
}
