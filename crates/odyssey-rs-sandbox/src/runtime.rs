use crate::{
    SandboxContext, SandboxError, SandboxHandle, SandboxPolicy, SandboxProvider, SandboxSupport,
    default_provider_name,
    provider::{DependencyReport, local::HostExecProvider},
};
use odyssey_rs_protocol::SandboxMode;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{collections::HashMap, fmt::Display};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SandboxCellKind {
    Tooling,
    Skill,
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SandboxCellKey {
    pub session_id: Option<Uuid>,
    pub agent_id: String,
    pub kind: SandboxCellKind,
    pub component_id: String,
}

impl SandboxCellKey {
    pub fn tooling(session_id: Uuid, agent_id: impl Into<String>) -> Self {
        Self {
            session_id: Some(session_id),
            agent_id: agent_id.into(),
            kind: SandboxCellKind::Tooling,
            component_id: "tools".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum SandboxCellRoot {
    SharedWorkspace(PathBuf),
    ManagedPrivate,
}

#[derive(Debug, Clone)]
pub struct SandboxCellSpec {
    pub key: SandboxCellKey,
    pub root: SandboxCellRoot,
    pub mode: SandboxMode,
    pub policy: SandboxPolicy,
}

impl SandboxCellSpec {
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

#[derive(Debug, Clone)]
pub struct SandboxExecutionLayout {
    pub execution_id: Uuid,
    pub root: PathBuf,
    pub inbox: PathBuf,
    pub outbox: PathBuf,
    pub work: PathBuf,
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

#[derive(Clone)]
pub struct SandboxCellLease {
    provider: Arc<dyn SandboxProvider>,
    state: Arc<SandboxCellState>,
}

impl SandboxCellLease {
    pub fn provider(&self) -> Arc<dyn SandboxProvider> {
        self.provider.clone()
    }

    pub fn handle(&self) -> SandboxHandle {
        self.state.handle.clone()
    }

    pub fn key(&self) -> &SandboxCellKey {
        &self.state.key
    }

    pub fn workspace_root(&self) -> &Path {
        &self.state.workspace_root
    }

    pub fn cell_root(&self) -> &Path {
        &self.state.cell_root
    }

    pub fn mode(&self) -> SandboxMode {
        self.state.mode
    }

    pub fn policy(&self) -> &SandboxPolicy {
        &self.state.policy
    }

    pub fn data_dir(&self) -> PathBuf {
        self.state.cell_root.join("data")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.state.cell_root.join("cache")
    }

    pub fn app_dir(&self) -> PathBuf {
        self.state.cell_root.join("app")
    }

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

pub struct SandboxRuntime {
    provider_name: String,
    provider: Arc<dyn SandboxProvider>,
    storage_root: PathBuf,
    cells: Mutex<HashMap<SandboxCellKey, Arc<SandboxCellState>>>,
}

impl Display for SandboxRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Sandbox Provider: {}", self.provider_name)
    }
}

impl SandboxRuntime {
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

    pub fn support(&self) -> SandboxSupport {
        let DependencyReport { errors, warnings } = self.provider.dependency_report();
        SandboxSupport {
            provider: self.provider_name.clone(),
            available: errors.is_empty(),
            errors,
            warnings,
        }
    }

    pub fn provider_name(&self) -> &str {
        &self.provider_name
    }

    pub fn storage_root(&self) -> &Path {
        &self.storage_root
    }

    pub fn managed_cell_root(&self, key: &SandboxCellKey) -> Result<PathBuf, SandboxError> {
        let root = self.materialize_cell_root(key)?;
        self.ensure_managed_cell_dirs(&root)?;
        Ok(root)
    }

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
        let workspace_root = match &spec.root {
            SandboxCellRoot::SharedWorkspace(path) => path.clone(),
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
        sanitize_segment,
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

    #[test]
    fn sanitize_segment_rewrites_unsafe_characters() {
        assert_eq!(sanitize_segment("agent/name"), "agent_name");
        assert_eq!(sanitize_segment(""), "default");
        assert_eq!(sanitize_segment("safe-_value"), "safe-_value");
    }

    #[test]
    fn runtime_from_provider_name_validates_known_and_unknown_backends() {
        let temp = tempdir().expect("tempdir");
        let runtime = SandboxRuntime::from_provider_name(
            Some("host"),
            SandboxMode::DangerFullAccess,
            temp.path().join("sandbox"),
        )
        .expect("runtime");

        assert_eq!(runtime.provider_name(), "host");
        assert_eq!(runtime.storage_root().ends_with("sandbox"), true);
        assert_eq!(runtime.support().available, true);

        let error = match SandboxRuntime::from_provider_name(
            Some("invalid"),
            SandboxMode::WorkspaceWrite,
            temp.path().join("other"),
        ) {
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

    #[test]
    fn managed_cell_root_creates_expected_directories() {
        let temp = tempdir().expect("tempdir");
        let runtime = SandboxRuntime::new(
            "host",
            Arc::new(LocalSandboxProvider::new()),
            temp.path().join("sandbox"),
        )
        .expect("runtime");
        let key = SandboxCellKey {
            session_id: None,
            agent_id: "agent/name".to_string(),
            kind: SandboxCellKind::Mcp,
            component_id: "comp:id".to_string(),
        };

        let root = runtime.managed_cell_root(&key).expect("managed root");

        assert_eq!(root.ends_with("comp_id"), true);
        assert_eq!(root.join("app").exists(), true);
        assert_eq!(root.join("data").exists(), true);
        assert_eq!(root.join("cache").exists(), true);
        assert_eq!(root.join("runs").exists(), true);
        assert_eq!(root.join("logs").exists(), true);
    }
}
