use crate::{
    SandboxContext, SandboxError, SandboxHandle, SandboxPolicy, SandboxProvider, SandboxSupport,
    default_provider_name,
    provider::{DependencyReport, canonicalize_existing_path, local::HostExecProvider},
};
use odyssey_rs_protocol::SandboxMode;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{collections::HashMap, fmt::Display};
use tokio::sync::Mutex;
use uuid::Uuid;

/// High-level buckets for runtime-managed sandbox cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SandboxCellKind {
    Tooling,
}

impl SandboxCellKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tooling => "tooling",
        }
    }
}

/// Stable identity for a reusable sandbox cell.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SandboxCellKey {
    pub session_id: Option<Uuid>,
    pub agent_id: String,
    pub kind: SandboxCellKind,
    pub component_id: String,
}

impl SandboxCellKey {
    pub fn tooling(session_id: Uuid, agent_id: impl Into<String>) -> Self {
        Self::tooling_component(session_id, agent_id, "tools")
    }

    pub fn tooling_component(
        session_id: Uuid,
        agent_id: impl Into<String>,
        component_id: impl Into<String>,
    ) -> Self {
        Self {
            session_id: Some(session_id),
            agent_id: agent_id.into(),
            kind: SandboxCellKind::Tooling,
            component_id: component_id.into(),
        }
    }
}

/// Runtime request used to create or reuse a managed sandbox cell.
#[derive(Debug, Clone)]
pub struct SandboxCellSpec {
    pub key: SandboxCellKey,
    pub cell_root: PathBuf,
    pub mode: SandboxMode,
    pub policy: SandboxPolicy,
}

impl SandboxCellSpec {
    pub fn managed_component(
        key: SandboxCellKey,
        cell_root: PathBuf,
        mode: SandboxMode,
        policy: SandboxPolicy,
    ) -> Self {
        Self {
            key,
            cell_root,
            mode,
            policy,
        }
    }
}

/// Per-command layout created inside a managed cell.
#[derive(Debug, Clone)]
pub struct SandboxExecutionLayout {
    pub execution_id: Uuid,
    pub root: PathBuf,
    pub inbox: PathBuf,
    pub outbox: PathBuf,
    pub work: PathBuf,
    pub tmp: PathBuf,
}

// Managed cells keep immutable staged bundle content separate from mutable state.
// That lets restricted sandboxes keep `app/` read-only while still exposing private
// locations for HOME, caches, temp files, and per-execution scratch data.
#[derive(Debug, Clone)]
struct ManagedCellLayout {
    root: PathBuf,
}

impl ManagedCellLayout {
    fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn root(&self) -> &Path {
        &self.root
    }

    fn app_dir(&self) -> PathBuf {
        self.root.join("app")
    }

    fn data_dir(&self) -> PathBuf {
        self.root.join("data")
    }

    fn home_dir(&self) -> PathBuf {
        self.data_dir().join("home")
    }

    fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }

    fn tmp_dir(&self) -> PathBuf {
        self.root.join("tmp")
    }

    fn runs_dir(&self) -> PathBuf {
        self.root.join("runs")
    }

    fn ensure(&self) -> Result<(), SandboxError> {
        let _ = ensure_child_directory(self.root(), "app")?;
        let data = ensure_child_directory(self.root(), "data")?;
        let _ = ensure_child_directory(&data, "home")?;
        let _ = ensure_child_directory(self.root(), "cache")?;
        let _ = ensure_child_directory(self.root(), "tmp")?;
        let _ = ensure_child_directory(self.root(), "runs")?;
        Ok(())
    }

    fn begin_execution(&self, execution_id: Uuid) -> Result<SandboxExecutionLayout, SandboxError> {
        let root = self.runs_dir().join(execution_id.to_string());
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

#[derive(Debug, Clone)]
struct SandboxCellState {
    key: SandboxCellKey,
    handle: SandboxHandle,
    workspace_root: PathBuf,
    cell_root: PathBuf,
    mode: SandboxMode,
    policy: SandboxPolicy,
}

/// Active reference to a prepared sandbox cell.
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
        ManagedCellLayout::new(self.state.cell_root.clone()).data_dir()
    }

    pub fn cache_dir(&self) -> PathBuf {
        ManagedCellLayout::new(self.state.cell_root.clone()).cache_dir()
    }

    pub fn app_dir(&self) -> PathBuf {
        ManagedCellLayout::new(self.state.cell_root.clone()).app_dir()
    }

    pub fn begin_execution(&self) -> Result<SandboxExecutionLayout, SandboxError> {
        let execution_id = Uuid::new_v4();
        ManagedCellLayout::new(self.state.cell_root.clone()).begin_execution(execution_id)
    }
}

/// Registry of prepared sandbox cells backed by a single provider implementation.
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
        let storage_root = canonicalize_existing_path(&storage_root)?;
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
                Self::new("host", Arc::new(HostExecProvider::default()), storage_root)
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
        self.ensure_managed_cell_dirs(&root)
    }

    /// Create or reuse a managed sandbox cell for the given key and config.
    ///
    /// The runtime normalizes the managed layout under `cell_root`, augments the
    /// policy with runtime-owned writable paths, and returns the existing cell
    /// when the key has already been prepared with the same root, mode, and policy.
    pub async fn lease_cell(
        &self,
        spec: SandboxCellSpec,
    ) -> Result<Arc<SandboxCellLease>, SandboxError> {
        let cell_root = self.ensure_managed_cell_dirs(&spec.cell_root)?;
        let layout = ManagedCellLayout::new(cell_root.clone());
        let workspace_root = canonicalize_existing_path(&layout.app_dir())?;
        let policy = augment_managed_cell_policy(spec.mode, spec.policy.clone(), &layout);

        let mut cells = self.cells.lock().await;
        if let Some(state) = cells.get(&spec.key) {
            if state.workspace_root != workspace_root
                || state.cell_root != cell_root
                || state.mode != spec.mode
                || state.policy != policy
            {
                return Err(SandboxError::InvalidConfig(format!(
                    "sandbox cell '{}' already exists with a different root, mode, or policy",
                    spec.key.component_id
                )));
            }

            return Ok(Arc::new(SandboxCellLease {
                provider: self.provider.clone(),
                state: state.clone(),
            }));
        }

        let context = SandboxContext {
            workspace_root: workspace_root.clone(),
            mode: spec.mode,
            policy: policy.clone(),
        };
        let handle = self.provider.prepare(&context).await?;
        let state = Arc::new(SandboxCellState {
            key: spec.key.clone(),
            handle,
            workspace_root,
            cell_root,
            mode: spec.mode,
            policy,
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
            .map_or_else(|| "shared".to_string(), |value| value.to_string());
        let cells_root = ensure_child_directory(&self.storage_root, "cells")?;
        let kind_root = ensure_child_directory(&cells_root, key.kind.as_str())?;
        let agent_root = ensure_child_directory(&kind_root, &sanitize_segment(&key.agent_id))?;
        let session_root = ensure_child_directory(&agent_root, &session)?;
        ensure_child_directory(&session_root, &sanitize_segment(&key.component_id))
    }

    fn ensure_managed_cell_dirs(&self, root: &Path) -> Result<PathBuf, SandboxError> {
        std::fs::create_dir_all(root).map_err(SandboxError::Io)?;
        let root = canonicalize_existing_path(root)?;
        ManagedCellLayout::new(root.clone()).ensure()?;
        Ok(root)
    }
}

fn ensure_child_directory(parent: &Path, name: &str) -> Result<PathBuf, SandboxError> {
    let child = parent.join(name);
    match std::fs::symlink_metadata(&child) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(SandboxError::InvalidConfig(format!(
                    "sandbox path must not be a symlink: {}",
                    child.display()
                )));
            }
            if !metadata.is_dir() {
                return Err(SandboxError::InvalidConfig(format!(
                    "sandbox path must be a directory: {}",
                    child.display()
                )));
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(&child).map_err(SandboxError::Io)?;
        }
        Err(err) => return Err(SandboxError::Io(err)),
    }

    canonicalize_existing_path(&child)
}

fn push_unique_path(paths: &mut Vec<String>, path: &Path) {
    let value = path.display().to_string();
    if !paths.iter().any(|existing| existing == &value) {
        paths.push(value);
        paths.sort();
        paths.dedup();
    }
}

fn augment_managed_cell_policy(
    mode: SandboxMode,
    mut policy: SandboxPolicy,
    layout: &ManagedCellLayout,
) -> SandboxPolicy {
    push_unique_path(&mut policy.filesystem.read_roots, layout.root());

    match mode {
        SandboxMode::DangerFullAccess | SandboxMode::WorkspaceWrite => {
            push_unique_path(&mut policy.filesystem.write_roots, layout.root());
        }
        SandboxMode::ReadOnly => {
            for path in [
                layout.data_dir(),
                layout.cache_dir(),
                layout.tmp_dir(),
                layout.runs_dir(),
            ] {
                push_unique_path(&mut policy.filesystem.write_roots, &path);
            }
        }
    }

    policy
        .env
        .set
        .entry("HOME".to_string())
        .or_insert_with(|| layout.home_dir().display().to_string());
    policy
        .env
        .set
        .entry("TMPDIR".to_string())
        .or_insert_with(|| layout.tmp_dir().display().to_string());
    policy
        .env
        .set
        .entry("XDG_DATA_HOME".to_string())
        .or_insert_with(|| layout.data_dir().display().to_string());
    policy
        .env
        .set
        .entry("XDG_CACHE_HOME".to_string())
        .or_insert_with(|| layout.cache_dir().display().to_string());

    policy
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
        sanitized = "default".to_string();
    }
    if sanitized.len() > 32 {
        sanitized.truncate(32);
        while sanitized.ends_with('_') {
            sanitized.pop();
        }
        if sanitized.is_empty() {
            sanitized = "default".to_string();
        }
    }

    let digest = Sha256::digest(value.as_bytes());
    let suffix = hex::encode(&digest[..6]);
    format!("{sanitized}-{suffix}")
}

#[cfg(test)]
fn has_same_sanitized_prefix(left: &str, right: &str) -> bool {
    fn prefix(value: &str) -> &str {
        value.rsplit_once('-').map_or(value, |(prefix, _)| prefix)
    }

    prefix(left) == prefix(right)
}

#[cfg(test)]
mod tests {
    use super::{
        SandboxCellKey, SandboxCellKind, SandboxCellSpec, SandboxRuntime,
        augment_managed_cell_policy, has_same_sanitized_prefix, sanitize_segment,
    };
    use crate::{
        AccessDecision, AccessMode, CommandResult, CommandSpec, LocalSandboxProvider,
        SandboxContext, SandboxError, SandboxHandle, SandboxPolicy, SandboxProvider,
    };
    use async_trait::async_trait;
    use odyssey_rs_protocol::SandboxMode;
    use parking_lot::Mutex as ParkingMutex;
    use pretty_assertions::assert_eq;
    use std::path::Path;
    use std::sync::Arc;
    use tempfile::tempdir;
    use uuid::Uuid;

    #[tokio::test]
    async fn runtime_reuses_the_same_cell_handle() {
        let temp = tempdir().expect("tempdir");
        let runtime = SandboxRuntime::new(
            "host",
            Arc::new(LocalSandboxProvider::default()),
            temp.path().join("sandbox"),
        )
        .expect("runtime");
        let key = SandboxCellKey {
            session_id: Some(Uuid::nil()),
            agent_id: "agent".to_string(),
            kind: SandboxCellKind::Tooling,
            component_id: "writer".to_string(),
        };
        let root = runtime.managed_cell_root(&key).expect("managed root");

        let first = runtime
            .lease_cell(SandboxCellSpec::managed_component(
                key.clone(),
                root.clone(),
                SandboxMode::DangerFullAccess,
                SandboxPolicy::default(),
            ))
            .await
            .expect("first lease");
        let second = runtime
            .lease_cell(SandboxCellSpec::managed_component(
                key,
                root,
                SandboxMode::DangerFullAccess,
                SandboxPolicy::default(),
            ))
            .await
            .expect("second lease");

        assert_eq!(first.handle().id, second.handle().id);
    }

    #[test]
    fn sanitize_segment_keeps_colliding_prefixes_distinct() {
        let first = sanitize_segment("agent/name");
        let second = sanitize_segment("agent?name");

        assert!(has_same_sanitized_prefix(&first, &second));
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn managed_cells_get_private_roots_and_execution_dirs() {
        let temp = tempdir().expect("tempdir");
        let runtime = SandboxRuntime::new(
            "host",
            Arc::new(LocalSandboxProvider::default()),
            temp.path().join("sandbox"),
        )
        .expect("runtime");
        let key = SandboxCellKey {
            session_id: Some(Uuid::nil()),
            agent_id: "agent".to_string(),
            kind: SandboxCellKind::Tooling,
            component_id: "writer".to_string(),
        };
        let root = runtime.managed_cell_root(&key).expect("managed root");

        let lease = runtime
            .lease_cell(SandboxCellSpec {
                key,
                cell_root: root,
                mode: SandboxMode::DangerFullAccess,
                policy: SandboxPolicy::default(),
            })
            .await
            .expect("lease");

        let component_name = lease
            .cell_root()
            .file_name()
            .and_then(|value| value.to_str())
            .expect("component name");
        assert!(component_name.starts_with("writer-"));
        assert_eq!(lease.workspace_root(), lease.app_dir());
        assert!(lease.data_dir().exists());
        assert!(lease.cache_dir().exists());
        assert!(lease.cell_root().join("tmp").exists());
        let execution = lease.begin_execution().expect("execution dirs");
        assert!(execution.root.exists());
        assert!(execution.inbox.exists());
        assert!(execution.outbox.exists());
        assert!(execution.work.exists());
        assert!(execution.tmp.exists());
    }

    #[tokio::test]
    async fn managed_cells_use_private_runtime_environment_defaults() {
        let temp = tempdir().expect("tempdir");
        let runtime = SandboxRuntime::new(
            "host",
            Arc::new(LocalSandboxProvider::default()),
            temp.path().join("sandbox"),
        )
        .expect("runtime");
        let key = SandboxCellKey {
            session_id: Some(Uuid::nil()),
            agent_id: "agent".to_string(),
            kind: SandboxCellKind::Tooling,
            component_id: "writer".to_string(),
        };
        let root = runtime.managed_cell_root(&key).expect("managed root");

        let lease = runtime
            .lease_cell(SandboxCellSpec::managed_component(
                key,
                root.clone(),
                SandboxMode::DangerFullAccess,
                SandboxPolicy::default(),
            ))
            .await
            .expect("lease");

        let mut spec = CommandSpec::new("sh");
        spec.args = vec![
            "-c".to_string(),
            "printf '%s\\n%s\\n%s\\n%s' \"$HOME\" \"$TMPDIR\" \"$XDG_CACHE_HOME\" \"$XDG_DATA_HOME\""
                .to_string(),
        ];

        let result = lease
            .provider()
            .run_command(&lease.handle(), spec)
            .await
            .expect("run");

        assert_eq!(
            result.stdout,
            format!(
                "{}\n{}\n{}\n{}",
                lease.data_dir().join("home").display(),
                lease.cell_root().join("tmp").display(),
                lease.cache_dir().display(),
                lease.data_dir().display()
            )
        );
    }

    #[tokio::test]
    async fn runtime_rejects_conflicting_cell_reuse() {
        let temp = tempdir().expect("tempdir");
        let runtime = SandboxRuntime::new(
            "host",
            Arc::new(LocalSandboxProvider::default()),
            temp.path().join("sandbox"),
        )
        .expect("runtime");
        let alternate = temp.path().join("alternate");
        std::fs::create_dir_all(&alternate).expect("alternate");
        let key = SandboxCellKey {
            session_id: Some(Uuid::nil()),
            agent_id: "agent".to_string(),
            kind: SandboxCellKind::Tooling,
            component_id: "writer".to_string(),
        };
        let root = runtime.managed_cell_root(&key).expect("managed root");

        runtime
            .lease_cell(SandboxCellSpec::managed_component(
                key.clone(),
                root,
                SandboxMode::DangerFullAccess,
                SandboxPolicy::default(),
            ))
            .await
            .expect("first lease");

        let error = match runtime
            .lease_cell(SandboxCellSpec::managed_component(
                key,
                alternate,
                SandboxMode::DangerFullAccess,
                SandboxPolicy::default(),
            ))
            .await
        {
            Ok(_) => panic!("conflicting lease should fail"),
            Err(error) => error,
        };

        assert!(
            error
                .to_string()
                .contains("already exists with a different root, mode, or policy")
        );
    }

    #[test]
    fn sanitize_segment_rewrites_unsafe_characters() {
        assert!(sanitize_segment("agent/name").starts_with("agent_name-"));
        assert!(sanitize_segment("").starts_with("default-"));
        assert!(sanitize_segment("safe-_value").starts_with("safe-_value-"));
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
        assert!(runtime.storage_root().ends_with("sandbox"));
        assert!(runtime.support().available);

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
            Arc::new(LocalSandboxProvider::default()),
            temp.path().join("sandbox"),
        )
        .expect("runtime");
        let key = SandboxCellKey {
            session_id: None,
            agent_id: "agent/name".to_string(),
            kind: SandboxCellKind::Tooling,
            component_id: "comp:id".to_string(),
        };

        let root = runtime.managed_cell_root(&key).expect("managed root");

        let component_name = root
            .file_name()
            .and_then(|value| value.to_str())
            .expect("component name");
        assert!(component_name.starts_with("comp_id-"));
        assert!(root.join("app").exists());
        assert!(root.join("data").exists());
        assert!(root.join("data").join("home").exists());
        assert!(root.join("cache").exists());
        assert!(root.join("tmp").exists());
        assert!(root.join("runs").exists());
    }

    #[test]
    fn managed_read_only_policy_keeps_app_read_only_but_exposes_private_state_dirs() {
        let temp = tempdir().expect("tempdir");
        let layout_root = temp.path().join("cell");
        std::fs::create_dir_all(&layout_root).expect("layout root");
        let policy = augment_managed_cell_policy(
            SandboxMode::ReadOnly,
            SandboxPolicy::default(),
            &super::ManagedCellLayout::new(layout_root.clone()),
        );

        assert!(
            policy
                .filesystem
                .read_roots
                .contains(&layout_root.display().to_string())
        );
        assert!(
            !policy
                .filesystem
                .write_roots
                .contains(&layout_root.display().to_string())
        );
        assert!(
            policy
                .filesystem
                .write_roots
                .contains(&layout_root.join("data").display().to_string())
        );
        assert!(
            policy
                .filesystem
                .write_roots
                .contains(&layout_root.join("cache").display().to_string())
        );
        assert!(
            policy
                .filesystem
                .write_roots
                .contains(&layout_root.join("tmp").display().to_string())
        );
        assert_eq!(
            policy.env.set.get("HOME"),
            Some(&layout_root.join("data").join("home").display().to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn managed_cell_root_rejects_symlinked_storage_components() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().expect("tempdir");
        let runtime = SandboxRuntime::new(
            "host",
            Arc::new(LocalSandboxProvider::default()),
            temp.path().join("sandbox"),
        )
        .expect("runtime");
        let key = SandboxCellKey {
            session_id: Some(Uuid::nil()),
            agent_id: "agent".to_string(),
            kind: SandboxCellKind::Tooling,
            component_id: "writer".to_string(),
        };
        let agent_segment = sanitize_segment(&key.agent_id);
        let kind_root = runtime.storage_root().join("cells").join("tooling");
        let session_parent = kind_root.join(agent_segment);
        std::fs::create_dir_all(&session_parent).expect("session parent");
        symlink(
            temp.path().join("outside"),
            session_parent.join(Uuid::nil().to_string()),
        )
        .expect("session symlink");

        let error = runtime
            .managed_cell_root(&key)
            .expect_err("symlinked component rejected");

        assert!(
            error
                .to_string()
                .contains("sandbox path must not be a symlink")
        );
    }

    #[derive(Default)]
    struct RecordingProvider {
        contexts: ParkingMutex<Vec<SandboxContext>>,
    }

    #[async_trait]
    impl SandboxProvider for RecordingProvider {
        async fn prepare(&self, ctx: &SandboxContext) -> Result<SandboxHandle, SandboxError> {
            self.contexts.lock().push(ctx.clone());
            Ok(SandboxHandle { id: Uuid::nil() })
        }

        async fn run_command(
            &self,
            _handle: &SandboxHandle,
            _spec: CommandSpec,
        ) -> Result<CommandResult, SandboxError> {
            Err(SandboxError::Unsupported("not used in test".to_string()))
        }

        async fn run_command_streaming(
            &self,
            _handle: &SandboxHandle,
            _spec: CommandSpec,
            _sink: &mut dyn crate::CommandOutputSink,
        ) -> Result<CommandResult, SandboxError> {
            Err(SandboxError::Unsupported("not used in test".to_string()))
        }

        fn check_access(
            &self,
            _handle: &SandboxHandle,
            _path: &Path,
            _mode: AccessMode,
        ) -> AccessDecision {
            AccessDecision::Allow
        }

        async fn shutdown(&self, _handle: SandboxHandle) {}
    }

    #[tokio::test]
    async fn runtime_passes_augmented_policy_to_managed_read_only_cells() {
        let temp = tempdir().expect("tempdir");
        let provider = Arc::new(RecordingProvider::default());
        let runtime =
            SandboxRuntime::new("recording", provider.clone(), temp.path().join("sandbox"))
                .expect("runtime");
        let key = SandboxCellKey {
            session_id: Some(Uuid::nil()),
            agent_id: "agent".to_string(),
            kind: SandboxCellKind::Tooling,
            component_id: "writer".to_string(),
        };
        let root = runtime.managed_cell_root(&key).expect("managed root");

        let _ = runtime
            .lease_cell(SandboxCellSpec::managed_component(
                key,
                root.clone(),
                SandboxMode::ReadOnly,
                SandboxPolicy::default(),
            ))
            .await
            .expect("lease");

        let contexts = provider.contexts.lock();
        let context = contexts.first().expect("recorded context");
        assert_eq!(context.workspace_root, root.join("app"));
        assert!(
            context
                .policy
                .filesystem
                .read_roots
                .contains(&root.display().to_string())
        );
        assert!(
            !context
                .policy
                .filesystem
                .write_roots
                .contains(&root.display().to_string())
        );
        assert!(
            context
                .policy
                .filesystem
                .write_roots
                .contains(&root.join("tmp").display().to_string())
        );
        assert_eq!(
            context.policy.env.set.get("TMPDIR"),
            Some(&root.join("tmp").display().to_string())
        );
    }
}
