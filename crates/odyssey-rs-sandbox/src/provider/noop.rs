//! No-op sandbox provider that executes commands without isolation.

use crate::{SandboxError, provider::run_local_command};
use log::{debug, info};
use std::path::Path;

use async_trait::async_trait;

use crate::{
    AccessDecision, AccessMode, CommandOutputSink, CommandResult, CommandSpec, SandboxContext,
    SandboxHandle, SandboxProvider,
};

/// Sandbox provider that performs no isolation checks.
#[derive(Debug, Default)]
pub struct NoSandboxProvider;

impl NoSandboxProvider {
    /// Create a new no-op sandbox provider.
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl SandboxProvider for NoSandboxProvider {
    /// Prepare a no-op sandbox handle.
    async fn prepare(&self, _ctx: &SandboxContext) -> Result<SandboxHandle, SandboxError> {
        Ok(SandboxHandle {
            id: uuid::Uuid::new_v4(),
        })
    }

    /// Run a command without isolation.
    async fn run_command(
        &self,
        _handle: &SandboxHandle,
        spec: CommandSpec,
    ) -> Result<CommandResult, SandboxError> {
        debug!(
            "noop sandbox run (args_len={}, has_cwd={})",
            spec.args.len(),
            spec.cwd.is_some()
        );
        run_local_command(spec).await
    }

    /// Run a command and stream output via sink.
    async fn run_command_streaming(
        &self,
        handle: &SandboxHandle,
        spec: CommandSpec,
        sink: &mut dyn CommandOutputSink,
    ) -> Result<CommandResult, SandboxError> {
        let result = self.run_command(handle, spec).await?;
        if !result.stdout.is_empty() {
            sink.stdout(&result.stdout);
        }
        if !result.stderr.is_empty() {
            sink.stderr(&result.stderr);
        }
        Ok(result)
    }

    /// Allow all access checks.
    fn check_access(
        &self,
        _handle: &SandboxHandle,
        _path: &Path,
        _mode: AccessMode,
    ) -> AccessDecision {
        AccessDecision::Allow
    }

    /// No-op shutdown for the provider.
    async fn shutdown(&self, _handle: SandboxHandle) {
        info!("noop sandbox shutdown");
    }
}
