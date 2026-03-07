//! Persistent sandbox runtime with reusable component cells.

use crate::{
    SandboxContext, SandboxError, SandboxHandle, SandboxPolicy, SandboxProvider, SandboxSupport,
    default_provider_name,
    provider::{DependencyReport, local::HostExecProvider},
};
use odyssey_rs_protocol::SandboxMode;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Long-lived sandbox cell kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SandboxCellKind {
    /// Shared tooling workspace for generic tool execution.
    Tooling,
    /// Private skill runtime cell.
    Skill,
    /// Private MCP runtime cell.
    Mcp,
}

impl SandboxCellKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tooling => "tooling",
            Self::Skill => "skill",
            Self::Mcp => "mcp",
        }
    }
}

/// Stable identity for a reusable sandbox cell.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SandboxCellKey {
    /// Session scope for the cell. `None` means process-global for the workspace.
    pub session_id: Option<Uuid>,
    /// Agent id that owns the cell.
    pub agent_id: String,
    /// Cell kind.
    pub kind: SandboxCellKind,
    /// Component identifier within the kind.
    pub component_id: String,
}

impl SandboxCellKey {
    /// Shared tooling cell for a session/agent pair.
    pub fn tooling(session_id: Uuid, agent_id: impl Into<String>) -> Self {
        Self {
            session_id: Some(session_id),
            agent_id: agent_id.into(),
            kind: SandboxCellKind::Tooling,
            component_id: "tools".to_string(),
        }
    }

    /// Private skill cell for a component.
    pub fn skill(
        session_id: Uuid,
        agent_id: impl Into<String>,
        component_id: impl Into<String>,
    ) -> Self {
        Self {
            session_id: Some(session_id),
            agent_id: agent_id.into(),
            kind: SandboxCellKind::Skill,
            component_id: component_id.into(),
        }
    }

    /// Private MCP cell for a component.
    pub fn mcp(
        session_id: Uuid,
        agent_id: impl Into<String>,
        component_id: impl Into<String>,
    ) -> Self {
        Self {
            session_id: Some(session_id),
            agent_id: agent_id.into(),
            kind: SandboxCellKind::Mcp,
            component_id: component_id.into(),
        }
    }

    /// Shared MCP cell owned by the orchestrator runtime.
    pub fn shared_mcp(agent_id: impl Into<String>, component_id: impl Into<String>) -> Self {
        Self {
            session_id: None,
            agent_id: agent_id.into(),
            kind: SandboxCellKind::Mcp,
            component_id: component_id.into(),
        }
    }
}

/// Root policy for a reusable sandbox cell.
#[derive(Debug, Clone)]
pub enum SandboxCellRoot {
    /// Reuse an existing workspace root.
    SharedWorkspace(PathBuf),
    /// Create a managed private root under the sandbox runtime storage directory.
    ManagedPrivate,
}

/// Specification used to materialize or reuse a cell.
#[derive(Debug, Clone)]
pub struct SandboxCellSpec {
    /// Stable identity for the cell.
    pub key: SandboxCellKey,
    /// Root strategy.
    pub root: SandboxCellRoot,
    /// Sandbox mode.
    pub mode: SandboxMode,
    /// Sandbox policy.
    pub policy: SandboxPolicy,
}

impl SandboxCellSpec {
    /// Shared tooling cell bound to an existing workspace root.
    pub fn tooling(
        session_id: Uuid,
        agent_id: impl Into<String>,
        workspace_root: PathBuf,
        mode: SandboxMode,
        policy: SandboxPolicy,
    ) -> Self {
        Self {
            key: SandboxCellKey::tooling(session_id, agent_id),
            root: SandboxCellRoot::SharedWorkspace(workspace_root),
            mode,
            policy,
        }
    }

    /// Managed private cell suitable for skill or MCP execution.
    pub fn managed_component(
        key: SandboxCellKey,
        mode: SandboxMode,
        policy: SandboxPolicy,
    ) -> Self {
        Self {
            key,
            root: SandboxCellRoot::ManagedPrivate,
            mode,
            policy,
        }
    }
}

/// Directories for a single execution inside a cell.
#[derive(Debug, Clone)]
pub struct SandboxExecutionLayout {
    /// Stable execution id.
    pub execution_id: Uuid,
    /// Root directory for the execution.
    pub root: PathBuf,
    /// Staged read-only/read-mostly inputs.
    pub inbox: PathBuf,
    /// Declared outputs.
    pub outbox: PathBuf,
    /// Working directory for commands.
    pub work: PathBuf,
    /// Temporary files for the execution.
    pub tmp: PathBuf,
}

#[derive(Debug, Clone)]
struct SandboxCellState {
    key: SandboxCellKey,
    handle: SandboxHandle,
    workspace_root: PathBuf,
    cell_root: PathBuf,
    mode: SandboxMode,
    policy: SandboxPolicy,
}

/// Reusable handle to a cached sandbox cell.
#[derive(Clone)]
pub struct SandboxCellLease {
    provider: Arc<dyn SandboxProvider>,
    state: Arc<SandboxCellState>,
}

impl SandboxCellLease {
    /// Provider implementation for command execution.
    pub fn provider(&self) -> Arc<dyn SandboxProvider> {
        self.provider.clone()
    }

    /// Provider-specific prepared handle.
    pub fn handle(&self) -> SandboxHandle {
        self.state.handle.clone()
    }

    /// Stable cell identity.
    pub fn key(&self) -> &SandboxCellKey {
        &self.state.key
    }

    /// Workspace root visible to commands in this cell.
    pub fn workspace_root(&self) -> &Path {
        &self.state.workspace_root
    }

    /// Cell-private root used for managed directories.
    pub fn cell_root(&self) -> &Path {
        &self.state.cell_root
    }

    /// Sandbox mode for the cell.
    pub fn mode(&self) -> SandboxMode {
        self.state.mode
    }

    /// Effective policy snapshot for the cell.
    pub fn policy(&self) -> &SandboxPolicy {
        &self.state.policy
    }

    /// Persistent private state directory for the cell.
    pub fn data_dir(&self) -> PathBuf {
        self.state.cell_root.join("data")
    }

    /// Private cache directory for the cell.
    pub fn cache_dir(&self) -> PathBuf {
        self.state.cell_root.join("cache")
    }

    /// Application bundle directory for the cell.
    pub fn app_dir(&self) -> PathBuf {
        self.state.cell_root.join("app")
    }

    /// Create a fresh per-execution directory layout inside the cell.
    pub fn begin_execution(&self) -> Result<SandboxExecutionLayout, SandboxError> {
        let execution_id = Uuid::new_v4();
        let root = self
            .state
            .cell_root
            .join("runs")
            .join(execution_id.to_string());
        let inbox = root.join("inbox");
        let outbox = root.join("outbox");
        let work = root.join("work");
        let tmp = root.join("tmp");

        for dir in [&root, &inbox, &outbox, &work, &tmp] {
            std::fs::create_dir_all(dir).map_err(SandboxError::Io)?;
        }

        Ok(SandboxExecutionLayout {
            execution_id,
            root,
            inbox,
            outbox,
            work,
            tmp,
        })
    }
}

/// Startup-owned sandbox runtime that caches prepared cells.
pub struct SandboxRuntime {
    provider_name: String,
    provider: Arc<dyn SandboxProvider>,
    storage_root: PathBuf,
    cells: Mutex<HashMap<SandboxCellKey, Arc<SandboxCellState>>>,
}

impl SandboxRuntime {
    /// Create a runtime from an existing provider.
    pub fn new(
        provider_name: impl Into<String>,
        provider: Arc<dyn SandboxProvider>,
        storage_root: PathBuf,
    ) -> Result<Self, SandboxError> {
        std::fs::create_dir_all(&storage_root).map_err(SandboxError::Io)?;
        Ok(Self {
            provider_name: provider_name.into(),
            provider,
            storage_root,
            cells: Mutex::new(HashMap::new()),
        })
    }

    /// Construct a runtime from a provider name and sandbox mode.
    pub fn from_provider_name(
        provider_name: Option<&str>,
        mode: SandboxMode,
        storage_root: PathBuf,
    ) -> Result<Self, SandboxError> {
        let name = provider_name.unwrap_or_else(|| default_provider_name(mode));
        match name {
            "host" | "local" | "none" | "nosandbox" => {
                Self::new("host", Arc::new(HostExecProvider::new()), storage_root)
            }
            #[cfg(target_os = "linux")]
            "bubblewrap" | "bwrap" => Self::new(
                "bubblewrap",
                Arc::new(crate::BubblewrapProvider::new()?),
                storage_root,
            ),
            #[cfg(not(target_os = "linux"))]
            "bubblewrap" | "bwrap" => Err(SandboxError::Unsupported(
                "bubblewrap sandboxing is only supported on Linux".to_string(),
            )),
            other => Err(SandboxError::InvalidConfig(format!(
                "unknown sandbox provider: {other}"
            ))),
        }
    }

    /// Return provider support information.
    pub fn support(&self) -> SandboxSupport {
        let DependencyReport { errors, warnings } = self.provider.dependency_report();
        SandboxSupport {
            provider: self.provider_name.clone(),
            available: errors.is_empty(),
            errors,
            warnings,
        }
    }

    /// Root directory used for cached cells and executions.
    pub fn storage_root(&self) -> &Path {
        &self.storage_root
    }

    /// Lease a cached cell, creating it once if needed.
    pub async fn lease_cell(
        &self,
        spec: SandboxCellSpec,
    ) -> Result<Arc<SandboxCellLease>, SandboxError> {
        let mut cells = self.cells.lock().await;
        if let Some(state) = cells.get(&spec.key) {
            return Ok(Arc::new(SandboxCellLease {
                provider: self.provider.clone(),
                state: state.clone(),
            }));
        }

        let cell_root = self.materialize_cell_root(&spec.key)?;
        let workspace_root = match spec.root {
            SandboxCellRoot::SharedWorkspace(path) => path,
            SandboxCellRoot::ManagedPrivate => {
                self.ensure_managed_cell_dirs(&cell_root)?;
                cell_root.clone()
            }
        };
        let context = SandboxContext {
            workspace_root: workspace_root.clone(),
            mode: spec.mode,
            policy: spec.policy.clone(),
        };
        let handle = self.provider.prepare(&context).await?;
        let state = Arc::new(SandboxCellState {
            key: spec.key.clone(),
            handle,
            workspace_root,
            cell_root,
            mode: spec.mode,
            policy: spec.policy,
        });
        cells.insert(spec.key, state.clone());

        Ok(Arc::new(SandboxCellLease {
            provider: self.provider.clone(),
            state,
        }))
    }

    /// Shut down all cached cells.
    pub async fn shutdown(&self) {
        let mut cells = self.cells.lock().await;
        let states = cells.drain().map(|(_, state)| state).collect::<Vec<_>>();
        drop(cells);
        for state in states {
            self.provider.shutdown(state.handle.clone()).await;
        }
    }

    fn materialize_cell_root(&self, key: &SandboxCellKey) -> Result<PathBuf, SandboxError> {
        let session = key
            .session_id
            .map(|value| value.to_string())
            .unwrap_or_else(|| "shared".to_string());
        let root = self
            .storage_root
            .join("cells")
            .join(key.kind.as_str())
            .join(sanitize_segment(&key.agent_id))
            .join(session)
            .join(sanitize_segment(&key.component_id));
        std::fs::create_dir_all(&root).map_err(SandboxError::Io)?;
        Ok(root)
    }

    fn ensure_managed_cell_dirs(&self, root: &Path) -> Result<(), SandboxError> {
        for dir in [
            root.join("app"),
            root.join("data"),
            root.join("cache"),
            root.join("runs"),
            root.join("logs"),
        ] {
            std::fs::create_dir_all(dir).map_err(SandboxError::Io)?;
        }
        Ok(())
    }
}

fn sanitize_segment(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
            sanitized.push(character);
        } else {
            sanitized.push('_');
        }
    }
    if sanitized.is_empty() {
        "default".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::{
        SandboxCellKey, SandboxCellKind, SandboxCellRoot, SandboxCellSpec, SandboxRuntime,
    };
    use crate::{LocalSandboxProvider, SandboxPolicy};
    use odyssey_rs_protocol::SandboxMode;
    use pretty_assertions::assert_eq;
    use std::sync::Arc;
    use tempfile::tempdir;
    use uuid::Uuid;

    #[tokio::test]
    async fn runtime_reuses_the_same_cell_handle() {
        let temp = tempdir().expect("tempdir");
        let runtime = SandboxRuntime::new(
            "host",
            Arc::new(LocalSandboxProvider::new()),
            temp.path().join("sandbox"),
        )
        .expect("runtime");
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");

        let first = runtime
            .lease_cell(SandboxCellSpec::tooling(
                Uuid::nil(),
                "agent",
                workspace.clone(),
                SandboxMode::WorkspaceWrite,
                SandboxPolicy::default(),
            ))
            .await
            .expect("first lease");
        let second = runtime
            .lease_cell(SandboxCellSpec::tooling(
                Uuid::nil(),
                "agent",
                workspace,
                SandboxMode::WorkspaceWrite,
                SandboxPolicy::default(),
            ))
            .await
            .expect("second lease");

        assert_eq!(first.handle().id, second.handle().id);
    }

    #[tokio::test]
    async fn managed_cells_get_private_roots_and_execution_dirs() {
        let temp = tempdir().expect("tempdir");
        let runtime = SandboxRuntime::new(
            "host",
            Arc::new(LocalSandboxProvider::new()),
            temp.path().join("sandbox"),
        )
        .expect("runtime");

        let lease = runtime
            .lease_cell(SandboxCellSpec {
                key: SandboxCellKey {
                    session_id: Some(Uuid::nil()),
                    agent_id: "agent".to_string(),
                    kind: SandboxCellKind::Skill,
                    component_id: "writer".to_string(),
                },
                root: SandboxCellRoot::ManagedPrivate,
                mode: SandboxMode::WorkspaceWrite,
                policy: SandboxPolicy::default(),
            })
            .await
            .expect("lease");

        assert_eq!(lease.cell_root().ends_with("writer"), true);
        assert_eq!(lease.data_dir().exists(), true);
        let execution = lease.begin_execution().expect("execution dirs");
        assert_eq!(execution.inbox.exists(), true);
        assert_eq!(execution.outbox.exists(), true);
        assert_eq!(execution.work.exists(), true);
        assert_eq!(execution.tmp.exists(), true);
    }
}
