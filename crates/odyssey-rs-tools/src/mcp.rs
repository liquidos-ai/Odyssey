//! MCP client connections backed by isolated sandbox cells.

use crate::{Tool, ToolContext, ToolRegistry};
use async_trait::async_trait;
use odyssey_rs_config::{
    McpConfig, McpServerConfig, SandboxFilesystem, SandboxNetworkMode as ConfigSandboxNetworkMode,
};
use odyssey_rs_protocol::ToolError;
use odyssey_rs_sandbox::{
    CommandLandlockPolicy, CommandSpec, SandboxCellKey, SandboxCellLease, SandboxCellSpec,
    SandboxEnvPolicy, SandboxExecutionLayout, SandboxFilesystemPolicy, SandboxLimits,
    SandboxNetworkMode, SandboxNetworkPolicy, SandboxPolicy, SandboxRuntime,
    resolve_internal_landlock_helper_path,
};
use rmcp::{
    model::{CallToolRequestParams, CallToolResult, ClientInfo, Tool as RemoteTool},
    service::{RoleClient, RunningService, ServiceExt},
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MCP_ENV_CELL_ROOT: &str = "ODYSSEY_MCP_CELL_ROOT";
const MCP_ENV_DATA_DIR: &str = "ODYSSEY_MCP_DATA_DIR";
const MCP_ENV_CACHE_DIR: &str = "ODYSSEY_MCP_CACHE_DIR";
const MCP_ENV_RUN_DIR: &str = "ODYSSEY_MCP_RUN_DIR";
const MCP_ENV_INBOX_DIR: &str = "ODYSSEY_MCP_INBOX_DIR";
const MCP_ENV_OUTBOX_DIR: &str = "ODYSSEY_MCP_OUTBOX_DIR";
const MCP_ENV_WORK_DIR: &str = "ODYSSEY_MCP_WORK_DIR";
const MCP_ENV_TMP_DIR: &str = "ODYSSEY_MCP_TMP_DIR";
const SANDBOX_DEFAULT_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// Errors raised while connecting MCP servers or adapting their tools.
#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("invalid MCP configuration: {0}")]
    InvalidConfig(String),
    #[error("sandbox error: {0}")]
    Sandbox(String),
    #[error("MCP transport error: {0}")]
    Transport(String),
    #[error("MCP protocol error: {0}")]
    Protocol(String),
}

struct McpConnection {
    service: Arc<RunningService<RoleClient, ClientInfo>>,
    _lease: Arc<SandboxCellLease>,
    _execution: SandboxExecutionLayout,
}

/// Startup-owned MCP manager that keeps sandboxed connections and adapted tools alive.
pub struct McpClientManager {
    connections: HashMap<String, Arc<McpConnection>>,
    tools: Vec<Arc<dyn Tool>>,
}

impl fmt::Debug for McpClientManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut servers = self.connections.keys().cloned().collect::<Vec<_>>();
        servers.sort();
        let mut tools = self
            .tools
            .iter()
            .map(|tool| tool.name().to_string())
            .collect::<Vec<_>>();
        tools.sort();
        f.debug_struct("McpClientManager")
            .field("servers", &servers)
            .field("tools", &tools)
            .finish()
    }
}

impl McpClientManager {
    /// Connect all configured MCP servers and adapt their tools into Odyssey tools.
    pub async fn connect(
        config: &McpConfig,
        sandbox_runtime: Arc<SandboxRuntime>,
        owner_agent_id: &str,
        base_dir: &Path,
    ) -> Result<Option<Self>, McpError> {
        if !config.enabled || config.servers.is_empty() {
            return Ok(None);
        }

        let mut connections = HashMap::new();
        let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
        let mut seen_servers = HashSet::new();

        for server in &config.servers {
            if !seen_servers.insert(server.name.clone()) {
                return Err(McpError::InvalidConfig(format!(
                    "duplicate MCP server name: {}",
                    server.name
                )));
            }

            let connection = Arc::new(
                connect_server(server, sandbox_runtime.clone(), owner_agent_id, base_dir).await?,
            );
            let remote_tools = connection.service.list_tools(None).await.map_err(|error| {
                McpError::Protocol(format!(
                    "failed to list tools for MCP server {}: {error:?}",
                    server.name
                ))
            })?;

            for remote_tool in remote_tools.tools {
                tools.push(Arc::new(McpToolAdapter::new(
                    server,
                    remote_tool,
                    connection.service.clone(),
                )));
            }

            connections.insert(server.name.clone(), connection);
        }

        Ok(Some(Self { connections, tools }))
    }

    /// Register all connected MCP tools into the provided registry.
    pub fn register_tools(&self, registry: &ToolRegistry) {
        for tool in &self.tools {
            registry.register(tool.clone());
        }
    }

    /// Return the tool names exposed by all MCP servers.
    pub fn tool_names(&self) -> Vec<String> {
        self.tools
            .iter()
            .map(|tool| tool.name().to_string())
            .collect()
    }

    /// Return the configured server names.
    pub fn server_names(&self) -> Vec<String> {
        self.connections.keys().cloned().collect()
    }
}

#[derive(Clone)]
struct McpToolAdapter {
    registered_name: String,
    remote_name: String,
    description: String,
    args_schema: Value,
    server_name: String,
    service: Arc<RunningService<RoleClient, ClientInfo>>,
}

impl McpToolAdapter {
    fn new(
        server: &McpServerConfig,
        remote_tool: RemoteTool,
        service: Arc<RunningService<RoleClient, ClientInfo>>,
    ) -> Self {
        let remote_name = remote_tool.name.to_string();
        let registered_name = format!(
            "mcp__{}__{}",
            sanitize_tool_segment(&server.name),
            sanitize_tool_segment(&remote_name)
        );
        let args_schema = serde_json::to_value(&remote_tool.input_schema).unwrap_or_else(|_| {
            json!({
                "type": "object",
                "properties": {},
                "required": []
            })
        });
        let base_description = remote_tool.description.unwrap_or_default().to_string();
        let description = if base_description.is_empty() {
            match &server.description {
                Some(server_description) => {
                    format!(
                        "MCP tool {remote_name} from {} ({server_description})",
                        server.name
                    )
                }
                None => format!("MCP tool {remote_name} from {}", server.name),
            }
        } else {
            format!("{base_description} [MCP server: {}]", server.name)
        };

        Self {
            registered_name,
            remote_name,
            description,
            args_schema,
            server_name: server.name.clone(),
            service,
        }
    }
}

impl fmt::Debug for McpToolAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpToolAdapter")
            .field("registered_name", &self.registered_name)
            .field("remote_name", &self.remote_name)
            .field("server_name", &self.server_name)
            .finish()
    }
}

#[async_trait]
impl Tool for McpToolAdapter {
    fn name(&self) -> &str {
        &self.registered_name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn args_schema(&self) -> Value {
        self.args_schema.clone()
    }

    async fn call(&self, _ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let Some(arguments) = args.as_object().cloned() else {
            return Err(ToolError::InvalidArguments(
                "MCP tool arguments must be a JSON object".to_string(),
            ));
        };

        let request = CallToolRequestParams {
            meta: None,
            name: self.remote_name.clone().into(),
            arguments: Some(arguments),
            task: None,
        };
        let result = self.service.call_tool(request).await.map_err(|error| {
            ToolError::ExecutionFailed(format!(
                "MCP tool {} on {} failed: {error:?}",
                self.remote_name, self.server_name
            ))
        })?;

        convert_mcp_result(&self.server_name, &self.remote_name, result)
    }
}

async fn connect_server(
    server: &McpServerConfig,
    sandbox_runtime: Arc<SandboxRuntime>,
    owner_agent_id: &str,
    base_dir: &Path,
) -> Result<McpConnection, McpError> {
    if server.protocol != "stdio" {
        return Err(McpError::InvalidConfig(format!(
            "only stdio MCP servers are supported today: {}",
            server.name
        )));
    }

    let launcher_path = resolve_internal_landlock_helper_path().map_err(|error| {
        McpError::Sandbox(format!(
            "failed to resolve internal Landlock helper for {}: {error}",
            server.name
        ))
    })?;

    let lease = sandbox_runtime
        .lease_cell(SandboxCellSpec::managed_component(
            SandboxCellKey::shared_mcp(owner_agent_id.to_string(), server.name.clone()),
            odyssey_rs_protocol::SandboxMode::WorkspaceWrite,
            build_server_policy(server, base_dir, &launcher_path),
        ))
        .await
        .map_err(|error| {
            McpError::Sandbox(format!(
                "failed to prepare MCP sandbox {}: {error}",
                server.name
            ))
        })?;
    let execution = lease.begin_execution().map_err(|error| {
        McpError::Sandbox(format!(
            "failed to create MCP execution layout for {}: {error}",
            server.name
        ))
    })?;
    let command_spec =
        build_server_command_spec(server, &lease, &execution, base_dir, &launcher_path)?;
    let command = lease
        .provider()
        .spawn_command(&lease.handle(), command_spec)
        .map_err(|error| {
            McpError::Sandbox(format!(
                "failed to build sandboxed MCP command for {}: {error}",
                server.name
            ))
        })?;
    let transport = TokioChildProcess::new(command.configure(|_| {})).map_err(|error| {
        McpError::Transport(format!(
            "failed to start MCP child transport for {}: {error}",
            server.name
        ))
    })?;
    let service = ClientInfo::default()
        .serve(transport)
        .await
        .map_err(|error| {
            McpError::Protocol(format!(
                "failed to connect MCP client for {}: {error:?}",
                server.name
            ))
        })?;

    Ok(McpConnection {
        service: Arc::new(service),
        _lease: lease,
        _execution: execution,
    })
}

fn build_server_policy(
    server: &McpServerConfig,
    base_dir: &Path,
    launcher_path: &Path,
) -> SandboxPolicy {
    let mut read_roots = absolutize_paths(&server.sandbox.filesystem, base_dir, |fs| &fs.read);
    read_roots.extend(system_runtime_read_roots());
    read_roots.extend(resolve_command_support_read_roots(server, base_dir));
    let write_roots = absolutize_paths(&server.sandbox.filesystem, base_dir, |fs| &fs.write);
    let mut exec_roots = absolutize_paths(&server.sandbox.filesystem, base_dir, |fs| &fs.exec);

    if let Some(cwd) = server.cwd.as_deref() {
        read_roots.push(resolve_host_path(base_dir, cwd).display().to_string());
    }
    for command_root in resolve_command_mount_roots(server, base_dir) {
        read_roots.push(command_root.clone());
        exec_roots.push(command_root);
    }
    if let Some(launcher_root) = launcher_path.parent() {
        let launcher_root = launcher_root.display().to_string();
        read_roots.push(launcher_root.clone());
        exec_roots.push(launcher_root);
    }

    let mut env = BTreeMap::new();
    for (key, value) in &server.sandbox.env.set {
        env.insert(key.clone(), value.clone());
    }
    for (key, value) in &server.env {
        env.insert(key.clone(), value.clone());
    }
    for key in [
        MCP_ENV_CELL_ROOT,
        MCP_ENV_DATA_DIR,
        MCP_ENV_CACHE_DIR,
        MCP_ENV_RUN_DIR,
        MCP_ENV_INBOX_DIR,
        MCP_ENV_OUTBOX_DIR,
        MCP_ENV_WORK_DIR,
        MCP_ENV_TMP_DIR,
    ] {
        env.entry(key.to_string()).or_default();
    }

    SandboxPolicy {
        filesystem: SandboxFilesystemPolicy {
            read_roots: dedupe_paths(read_roots),
            write_roots: dedupe_paths(write_roots),
            exec_roots: dedupe_paths(exec_roots),
        },
        env: SandboxEnvPolicy {
            inherit: server.sandbox.env.inherit.clone(),
            set: env,
        },
        network: SandboxNetworkPolicy {
            mode: match server.sandbox.network.mode {
                ConfigSandboxNetworkMode::Disabled => SandboxNetworkMode::Disabled,
                ConfigSandboxNetworkMode::AllowAll => SandboxNetworkMode::AllowAll,
            },
        },
        limits: SandboxLimits {
            cpu_seconds: server.sandbox.limits.cpu_seconds,
            memory_bytes: server.sandbox.limits.memory_bytes,
            nofile: server.sandbox.limits.nofile,
            pids: server.sandbox.limits.pids,
            wall_clock_seconds: server.sandbox.limits.wall_clock_seconds,
            stdout_bytes: server.sandbox.limits.stdout_bytes,
            stderr_bytes: server.sandbox.limits.stderr_bytes,
        },
    }
}

fn build_server_command_spec(
    server: &McpServerConfig,
    lease: &SandboxCellLease,
    execution: &SandboxExecutionLayout,
    base_dir: &Path,
    launcher_path: &Path,
) -> Result<CommandSpec, McpError> {
    let command = if has_path_components(&server.command) {
        resolve_host_path(base_dir, &server.command)
    } else {
        PathBuf::from(&server.command)
    };
    let cwd = server
        .cwd
        .as_deref()
        .map(|cwd| resolve_host_path(base_dir, cwd))
        .unwrap_or_else(|| execution.work.clone());

    let mut spec = CommandSpec::new(command);
    spec.args = server.args.clone();
    spec.cwd = Some(cwd);
    spec.env
        .insert("HOME".to_string(), lease.data_dir().display().to_string());
    spec.env
        .insert("TMPDIR".to_string(), execution.tmp.display().to_string());
    spec.env.insert(
        MCP_ENV_CELL_ROOT.to_string(),
        lease.cell_root().display().to_string(),
    );
    spec.env.insert(
        MCP_ENV_DATA_DIR.to_string(),
        lease.data_dir().display().to_string(),
    );
    spec.env.insert(
        MCP_ENV_CACHE_DIR.to_string(),
        lease.cache_dir().display().to_string(),
    );
    spec.env.insert(
        MCP_ENV_RUN_DIR.to_string(),
        execution.root.display().to_string(),
    );
    spec.env.insert(
        MCP_ENV_INBOX_DIR.to_string(),
        execution.inbox.display().to_string(),
    );
    spec.env.insert(
        MCP_ENV_OUTBOX_DIR.to_string(),
        execution.outbox.display().to_string(),
    );
    spec.env.insert(
        MCP_ENV_WORK_DIR.to_string(),
        execution.work.display().to_string(),
    );
    spec.env.insert(
        MCP_ENV_TMP_DIR.to_string(),
        execution.tmp.display().to_string(),
    );
    spec.landlock = Some(build_server_landlock_policy(
        server,
        lease,
        execution,
        base_dir,
        launcher_path,
    )?);
    Ok(spec)
}

fn build_server_landlock_policy(
    server: &McpServerConfig,
    lease: &SandboxCellLease,
    execution: &SandboxExecutionLayout,
    base_dir: &Path,
    launcher_path: &Path,
) -> Result<CommandLandlockPolicy, McpError> {
    let mut read_roots = system_runtime_roots();
    read_roots.extend(system_runtime_read_roots_pathbuf());
    read_roots.extend(resolve_existing_paths(
        &server.sandbox.filesystem,
        base_dir,
        |filesystem| &filesystem.read,
    )?);
    for root in resolve_command_support_read_roots_path(server, base_dir) {
        read_roots.push(canonicalize_path(root)?);
    }
    read_roots.extend(resolve_existing_paths(
        &server.sandbox.filesystem,
        base_dir,
        |filesystem| &filesystem.exec,
    )?);
    read_roots.push(canonicalize_path(lease.app_dir())?);
    read_roots.push(canonicalize_path(lease.data_dir())?);
    read_roots.push(canonicalize_path(lease.cache_dir())?);
    read_roots.push(canonicalize_path(&execution.root)?);
    if let Some(cwd) = server.cwd.as_deref() {
        read_roots.push(canonicalize_path(resolve_host_path(base_dir, cwd))?);
    }
    for command_root in resolve_command_mount_roots_path(server, base_dir) {
        read_roots.push(canonicalize_path(command_root)?);
    }
    if let Some(launcher_root) = launcher_path.parent() {
        read_roots.push(canonicalize_path(launcher_root)?);
    }

    let mut write_roots =
        resolve_existing_paths(&server.sandbox.filesystem, base_dir, |filesystem| {
            &filesystem.write
        })?;
    write_roots.push(canonicalize_path(lease.data_dir())?);
    write_roots.push(canonicalize_path(lease.cache_dir())?);
    write_roots.push(canonicalize_path(&execution.root)?);

    let mut exec_roots = system_runtime_roots();
    exec_roots.extend(resolve_existing_paths(
        &server.sandbox.filesystem,
        base_dir,
        |filesystem| &filesystem.exec,
    )?);
    for command_root in resolve_command_mount_roots_path(server, base_dir) {
        exec_roots.push(canonicalize_path(command_root)?);
    }
    if let Some(launcher_root) = launcher_path.parent() {
        exec_roots.push(canonicalize_path(launcher_root)?);
    }

    Ok(CommandLandlockPolicy {
        read_roots: dedupe_path_bufs(read_roots),
        write_roots: dedupe_path_bufs(write_roots),
        exec_roots: dedupe_path_bufs(exec_roots),
    })
}

fn convert_mcp_result(
    server_name: &str,
    remote_name: &str,
    result: CallToolResult,
) -> Result<Value, ToolError> {
    if result.is_error.unwrap_or(false) {
        let error_message = result
            .content
            .iter()
            .find_map(|content| {
                serde_json::to_value(&content.raw).ok().and_then(|value| {
                    value
                        .get("text")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
            })
            .unwrap_or_else(|| format!("MCP server {server_name} reported an error"));
        return Err(ToolError::ExecutionFailed(error_message));
    }

    let content = result
        .content
        .iter()
        .map(|entry| {
            serde_json::to_value(&entry.raw).unwrap_or_else(|_| {
                json!({
                    "type": "unknown",
                    "message": "failed to serialize MCP content"
                })
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "server": server_name,
        "tool": remote_name,
        "success": true,
        "content": content,
    }))
}

fn absolutize_paths(
    filesystem: &SandboxFilesystem,
    base_dir: &Path,
    selector: impl Fn(&SandboxFilesystem) -> &Vec<String>,
) -> Vec<String> {
    selector(filesystem)
        .iter()
        .map(|entry| resolve_host_path(base_dir, entry).display().to_string())
        .collect()
}

fn resolve_command_mount_roots(server: &McpServerConfig, base_dir: &Path) -> Vec<String> {
    resolve_command_mount_roots_path(server, base_dir)
        .into_iter()
        .map(|root| root.display().to_string())
        .collect()
}

fn resolve_command_mount_roots_path(server: &McpServerConfig, base_dir: &Path) -> Vec<PathBuf> {
    if !has_path_components(&server.command) {
        let mut roots = Vec::new();
        if let Ok(resolved) = which::which(&server.command) {
            if let Some(parent) = resolved.parent() {
                roots.push(parent.to_path_buf());
            }
            if let Ok(canonical) = resolved.canonicalize()
                && let Some(parent) = canonical.parent()
            {
                roots.push(parent.to_path_buf());
            }
        }
        for root in std::env::split_paths(SANDBOX_DEFAULT_PATH) {
            let candidate = root.join(&server.command);
            if candidate.exists() {
                roots.push(root);
                if let Ok(canonical) = candidate.canonicalize()
                    && let Some(parent) = canonical.parent()
                {
                    roots.push(parent.to_path_buf());
                }
            }
        }
        return dedupe_path_bufs(roots);
    }
    let command_path = resolve_host_path(base_dir, &server.command);
    if command_path.is_dir() {
        return vec![command_path];
    }
    command_path
        .parent()
        .map(Path::to_path_buf)
        .into_iter()
        .collect()
}

fn resolve_command_support_read_roots(server: &McpServerConfig, base_dir: &Path) -> Vec<String> {
    resolve_command_support_read_roots_path(server, base_dir)
        .into_iter()
        .map(|root| root.display().to_string())
        .collect()
}

fn resolve_command_support_read_roots_path(
    server: &McpServerConfig,
    base_dir: &Path,
) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let command_roots = resolve_command_mount_roots_path(server, base_dir);
    for root in command_roots {
        if let Some(parent) = root.parent() {
            roots.push(parent.to_path_buf());
            if let Some(package_root) = find_package_root(parent) {
                roots.push(package_root);
            }
        }
    }
    dedupe_path_bufs(roots)
}

fn find_package_root(start: &Path) -> Option<PathBuf> {
    let mut current = start.to_path_buf();
    for _ in 0..6 {
        if current.join("package.json").exists() {
            return Some(current);
        }
        if !current.pop() {
            break;
        }
    }
    None
}

fn resolve_existing_paths(
    filesystem: &SandboxFilesystem,
    base_dir: &Path,
    selector: impl Fn(&SandboxFilesystem) -> &Vec<String>,
) -> Result<Vec<PathBuf>, McpError> {
    selector(filesystem)
        .iter()
        .map(|entry| canonicalize_path(resolve_host_path(base_dir, entry)))
        .collect()
}

fn system_runtime_roots() -> Vec<PathBuf> {
    ["/usr", "/lib", "/lib64", "/bin", "/sbin", "/opt"]
        .into_iter()
        .map(PathBuf::from)
        .filter(|path| path.exists())
        .filter_map(|path| canonicalize_path(path).ok())
        .collect()
}

fn system_runtime_read_roots() -> Vec<String> {
    system_runtime_read_roots_pathbuf()
        .into_iter()
        .map(|path| path.display().to_string())
        .collect()
}

fn system_runtime_read_roots_pathbuf() -> Vec<PathBuf> {
    [
        "/etc/ssl",
        "/etc/pki",
        "/etc/ca-certificates",
        "/etc/crypto-policies",
        "/etc/resolv.conf",
        "/etc/hosts",
        "/etc/nsswitch.conf",
        "/etc/passwd",
        "/etc/group",
    ]
    .into_iter()
    .map(PathBuf::from)
    .filter(|path| path.exists())
    .filter_map(|path| canonicalize_path(path).ok())
    .collect()
}

fn canonicalize_path(path: impl AsRef<Path>) -> Result<PathBuf, McpError> {
    let path = path.as_ref();
    path.canonicalize().map_err(|error| {
        McpError::InvalidConfig(format!("failed to resolve {}: {error}", path.display()))
    })
}

fn dedupe_path_bufs(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.sort();
    paths.dedup();
    paths
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            std::path::Component::RootDir => normalized.push(Path::new("/")),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::Normal(part) => normalized.push(part),
        }
    }
    normalized
}

fn resolve_host_path(base_dir: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        normalize_lexical(&path)
    } else {
        normalize_lexical(&base_dir.join(path))
    }
}

fn dedupe_paths(paths: Vec<String>) -> Vec<String> {
    let mut paths = paths;
    paths.sort();
    paths.dedup();
    paths
}

fn has_path_components(value: &str) -> bool {
    value.contains(std::path::MAIN_SEPARATOR) || value.contains('/')
}

fn sanitize_tool_segment(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
            sanitized.push(character.to_ascii_lowercase());
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
    use super::{McpClientManager, build_server_policy, sanitize_tool_segment};
    use odyssey_rs_config::{
        McpConfig, McpServerConfig, McpServerSandboxConfig, SandboxFilesystem, SandboxNetwork,
        SandboxNetworkMode,
    };
    use odyssey_rs_sandbox::{HostExecProvider, SandboxRuntime};
    use pretty_assertions::assert_eq;
    use std::path::Path;
    use std::sync::Arc;
    use tempfile::tempdir;

    #[test]
    fn sanitize_tool_segment_normalizes_tool_names() {
        assert_eq!(sanitize_tool_segment("GitHub Search"), "github_search");
        assert_eq!(sanitize_tool_segment(""), "default");
    }

    #[test]
    fn build_server_policy_adds_command_and_cwd_roots() {
        let sandbox = McpServerSandboxConfig {
            filesystem: SandboxFilesystem {
                read: vec!["./extra-read".to_string()],
                write: vec![],
                exec: vec![],
            },
            network: SandboxNetwork {
                mode: SandboxNetworkMode::Disabled,
            },
            ..McpServerSandboxConfig::default()
        };
        let server = McpServerConfig {
            name: "fs".to_string(),
            command: "./bin/server".to_string(),
            cwd: Some("./workspace".to_string()),
            sandbox,
            ..McpServerConfig::default()
        };
        let policy = build_server_policy(
            &server,
            Path::new("/repo"),
            Path::new("/launcher/bin/odyssey-rs-sandbox-internal-landlock-helper"),
        );

        let expected_read_roots = vec![
            "/launcher/bin".to_string(),
            "/repo/bin".to_string(),
            "/repo/extra-read".to_string(),
            "/repo/workspace".to_string(),
        ];
        for expected in expected_read_roots {
            assert_eq!(policy.filesystem.read_roots.contains(&expected), true);
        }
        let expected_exec_roots = vec!["/launcher/bin".to_string(), "/repo/bin".to_string()];
        for expected in expected_exec_roots {
            assert_eq!(policy.filesystem.exec_roots.contains(&expected), true);
        }
    }

    #[tokio::test]
    async fn disabled_config_skips_connection_work() {
        let temp = tempdir().expect("tempdir");
        let runtime = SandboxRuntime::new(
            "host",
            Arc::new(HostExecProvider::new()),
            temp.path().join("sandbox"),
        )
        .expect("runtime");
        let config = McpConfig::default();
        let manager = McpClientManager::connect(&config, Arc::new(runtime), "agent", temp.path())
            .await
            .expect("connect");
        assert_eq!(manager.is_none(), true);
    }
}
