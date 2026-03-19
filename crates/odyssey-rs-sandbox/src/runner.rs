use crate::{
    CommandOutputSink, CommandResult, SandboxContext, SandboxError, SandboxHandle, SandboxProvider,
    SandboxRunRequest, SandboxRunResult, SandboxSupport, default_provider_name,
    provider::{DependencyReport, local::HostExecProvider},
};
use odyssey_rs_protocol::SandboxMode;
use std::sync::Arc;

#[derive(Clone)]
pub struct SandboxRunner {
    provider_name: String,
    provider: Arc<dyn SandboxProvider>,
}

impl SandboxRunner {
    pub fn new(provider_name: impl Into<String>, provider: Arc<dyn SandboxProvider>) -> Self {
        Self {
            provider_name: provider_name.into(),
            provider,
        }
    }

    pub fn from_provider_name(
        provider_name: Option<&str>,
        mode: SandboxMode,
    ) -> Result<Self, SandboxError> {
        let name = provider_name.unwrap_or_else(|| default_provider_name(mode));
        match name {
            "host" | "local" | "none" | "nosandbox" => {
                Ok(Self::new("host", Arc::new(HostExecProvider::default())))
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

    pub fn support(&self) -> SandboxSupport {
        let DependencyReport { errors, warnings } = self.provider.dependency_report();
        SandboxSupport {
            provider: self.provider_name.clone(),
            available: errors.is_empty(),
            errors,
            warnings,
        }
    }

    pub async fn prepare(&self, context: &SandboxContext) -> Result<SandboxHandle, SandboxError> {
        self.provider.prepare(context).await
    }

    pub async fn run(&self, request: SandboxRunRequest) -> Result<SandboxRunResult, SandboxError> {
        let handle = self.prepare(&request.context).await?;
        let result = self.provider.run_command(&handle, request.command).await;
        self.provider.shutdown(handle).await;
        result
    }

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

    pub fn provider(&self) -> Arc<dyn SandboxProvider> {
        self.provider.clone()
    }

    pub fn provider_name(&self) -> &str {
        &self.provider_name
    }
}

#[cfg(test)]
mod tests {
    use super::SandboxRunner;
    use crate::{CommandOutputSink, CommandSpec, SandboxContext, SandboxPolicy};
    use odyssey_rs_protocol::SandboxMode;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    #[derive(Default)]
    struct RecordingSink {
        stdout: String,
        stderr: String,
    }

    impl CommandOutputSink for RecordingSink {
        fn stdout(&mut self, chunk: &str) {
            self.stdout.push_str(chunk);
        }

        fn stderr(&mut self, chunk: &str) {
            self.stderr.push_str(chunk);
        }
    }

    #[test]
    fn host_runner_is_available() {
        let runner = SandboxRunner::from_provider_name(Some("host"), SandboxMode::DangerFullAccess)
            .expect("runner");
        let support = runner.support();
        assert_eq!(support.provider, "host");
        assert!(support.available);
    }

    #[test]
    fn invalid_provider_name_is_rejected() {
        let error =
            match SandboxRunner::from_provider_name(Some("invalid"), SandboxMode::WorkspaceWrite) {
                Ok(_) => panic!("invalid provider should fail"),
                Err(error) => error,
            };
        assert_eq!(
            error
                .to_string()
                .contains("unknown sandbox provider: invalid"),
            true
        );
    }

    #[tokio::test]
    async fn runner_executes_commands_and_streams_output() {
        let workspace = tempdir().expect("workspace");
        let runner = SandboxRunner::from_provider_name(Some("host"), SandboxMode::DangerFullAccess)
            .expect("runner");
        let request = crate::SandboxRunRequest {
            context: SandboxContext {
                workspace_root: workspace.path().to_path_buf(),
                mode: SandboxMode::WorkspaceWrite,
                policy: SandboxPolicy::default(),
            },
            command: {
                let mut spec = CommandSpec::new("sh");
                spec.args.extend([
                    "-c".to_string(),
                    "printf 'hello'; printf 'warn' >&2".to_string(),
                ]);
                spec
            },
        };

        let result = runner.run(request.clone()).await.expect("run");
        assert_eq!(result.stdout, "hello");
        assert_eq!(result.stderr, "warn");
        assert_eq!(result.status_code, Some(0));

        let mut sink = RecordingSink::default();
        let streamed = runner
            .run_streaming(request, &mut sink)
            .await
            .expect("run streaming");
        assert_eq!(sink.stdout, "hello");
        assert_eq!(sink.stderr, "warn");
        assert_eq!(streamed.status_code, Some(0));
        assert_eq!(runner.provider_name(), "host");
        assert_eq!(
            runner.provider().dependency_report().errors.is_empty(),
            true
        );
    }
}
