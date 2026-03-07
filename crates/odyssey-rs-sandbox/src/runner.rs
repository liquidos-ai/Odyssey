//! Standalone sandbox runner API.

use crate::{
    CommandOutputSink, CommandResult, SandboxContext, SandboxError, SandboxHandle, SandboxProvider,
    SandboxRunRequest, SandboxRunResult, SandboxSupport, default_provider_name,
    provider::{DependencyReport, local::HostExecProvider},
};
use odyssey_rs_protocol::SandboxMode;
use std::sync::Arc;

/// High-level standalone runner that owns a concrete provider.
#[derive(Clone)]
pub struct SandboxRunner {
    provider_name: String,
    provider: Arc<dyn SandboxProvider>,
}

impl SandboxRunner {
    /// Create a runner from an already-constructed provider.
    pub fn new(provider_name: impl Into<String>, provider: Arc<dyn SandboxProvider>) -> Self {
        Self {
            provider_name: provider_name.into(),
            provider,
        }
    }

    /// Construct a runner from a provider name and sandbox mode.
    pub fn from_provider_name(
        provider_name: Option<&str>,
        mode: SandboxMode,
    ) -> Result<Self, SandboxError> {
        let name = provider_name.unwrap_or_else(|| default_provider_name(mode));
        match name {
            "host" | "local" | "none" | "nosandbox" => {
                Ok(Self::new("host", Arc::new(HostExecProvider::new())))
            }
            #[cfg(target_os = "linux")]
            "bubblewrap" | "bwrap" => Ok(Self::new(
                "bubblewrap",
                Arc::new(crate::BubblewrapProvider::new()?),
            )),
            #[cfg(not(target_os = "linux"))]
            "bubblewrap" | "bwrap" => Err(SandboxError::Unsupported(
                "bubblewrap sandboxing is only supported on Linux".to_string(),
            )),
            other => Err(SandboxError::InvalidConfig(format!(
                "unknown sandbox provider: {other}"
            ))),
        }
    }

    /// Return provider support information for standalone tooling.
    pub fn support(&self) -> SandboxSupport {
        let DependencyReport { errors, warnings } = self.provider.dependency_report();
        SandboxSupport {
            provider: self.provider_name.clone(),
            available: errors.is_empty(),
            errors,
            warnings,
        }
    }

    /// Prepare a context and return the provider handle.
    pub async fn prepare(&self, context: &SandboxContext) -> Result<SandboxHandle, SandboxError> {
        self.provider.prepare(context).await
    }

    /// Run a single command and tear down the prepared sandbox afterwards.
    pub async fn run(&self, request: SandboxRunRequest) -> Result<SandboxRunResult, SandboxError> {
        let handle = self.prepare(&request.context).await?;
        let result = self.provider.run_command(&handle, request.command).await;
        self.provider.shutdown(handle).await;
        result
    }

    /// Run a single command with streaming output and tear down afterwards.
    pub async fn run_streaming(
        &self,
        request: SandboxRunRequest,
        sink: &mut dyn CommandOutputSink,
    ) -> Result<CommandResult, SandboxError> {
        let handle = self.prepare(&request.context).await?;
        let result = self
            .provider
            .run_command_streaming(&handle, request.command, sink)
            .await;
        self.provider.shutdown(handle).await;
        result
    }

    /// Borrow the provider for runtime integration.
    pub fn provider(&self) -> Arc<dyn SandboxProvider> {
        self.provider.clone()
    }

    /// Provider display name.
    pub fn provider_name(&self) -> &str {
        &self.provider_name
    }
}

#[cfg(test)]
mod tests {
    use super::SandboxRunner;
    use odyssey_rs_protocol::SandboxMode;
    use pretty_assertions::assert_eq;

    #[test]
    fn host_runner_is_available() {
        let runner = SandboxRunner::from_provider_name(Some("host"), SandboxMode::DangerFullAccess)
            .expect("runner");
        let support = runner.support();
        assert_eq!(support.provider, "host");
        assert_eq!(support.available, true);
    }
}
