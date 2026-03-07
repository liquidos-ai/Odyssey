//! Schema validation helpers for Odyssey JSON5 configuration.

use super::SchemaMode;
use crate::ConfigError;
use serde_json::{Map, Value};

/// Validate a single config layer against the schema.
pub(super) fn validate_layer_schema(
    value: &Value,
    _mode: SchemaMode,
    layer: &str,
) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, "")?;
    let allowed = [
        "$schema",
        "agents",
        "tools",
        "permissions",
        "memory",
        "skills",
        "mcp",
        "sandbox",
        "sessions",
    ];
    ensure_allowed_keys(map, &allowed, layer, "")?;

    if let Some(value) = map.get("$schema") {
        expect_string(value, layer, "$schema")?;
    }
    if let Some(value) = map.get("agents") {
        validate_agents(value, layer, "agents")?;
    }
    if let Some(value) = map.get("tools") {
        validate_tools(value, layer, "tools")?;
    }
    if let Some(value) = map.get("permissions") {
        validate_permissions(value, layer, "permissions")?;
    }
    if let Some(value) = map.get("memory") {
        validate_memory(value, layer, "memory")?;
    }
    if let Some(value) = map.get("skills") {
        validate_skills(value, layer, "skills")?;
    }
    if let Some(value) = map.get("mcp") {
        validate_mcp(value, layer, "mcp")?;
    }
    if let Some(value) = map.get("sandbox") {
        validate_sandbox(value, layer, "sandbox")?;
    }
    if let Some(value) = map.get("sessions") {
        validate_sessions(value, layer, "sessions")?;
    }

    Ok(())
}

/// Validate the "agents" block.
fn validate_agents(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    ensure_allowed_keys(map, &["list"], layer, path)?;
    if let Some(list) = map.get("list") {
        let arr = expect_array(list, layer, &join_path(path, "list"))?;
        for (idx, entry) in arr.iter().enumerate() {
            validate_agent(entry, layer, &format!("{path}.list[{idx}]"))?;
        }
    }
    Ok(())
}

/// Validate a single agent definition.
fn validate_agent(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    let allowed = [
        "id",
        "description",
        "prompt",
        "model",
        "tools",
        "memory",
        "sandbox",
        "permissions",
    ];
    ensure_allowed_keys(map, &allowed, layer, path)?;

    let id_path = join_path(path, "id");
    let Some(id_value) = map.get("id") else {
        return Err(invalid_field(layer, &id_path, "missing required field"));
    };
    expect_string(id_value, layer, &id_path)?;

    if let Some(value) = map.get("description") {
        expect_string(value, layer, &join_path(path, "description"))?;
    }
    if let Some(value) = map.get("prompt") {
        expect_string(value, layer, &join_path(path, "prompt"))?;
    }
    if let Some(value) = map.get("model") {
        validate_model(value, layer, &join_path(path, "model"))?;
    }
    if let Some(value) = map.get("tools") {
        validate_tool_policy(value, layer, &join_path(path, "tools"))?;
    }
    if let Some(value) = map.get("memory") {
        validate_memory(value, layer, &join_path(path, "memory"))?;
    }
    if let Some(value) = map.get("sandbox") {
        validate_agent_sandbox(value, layer, &join_path(path, "sandbox"))?;
    }
    if let Some(value) = map.get("permissions") {
        validate_agent_permissions(value, layer, &join_path(path, "permissions"))?;
    }
    Ok(())
}

/// Validate a model provider configuration.
fn validate_model(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    ensure_allowed_keys(map, &["provider", "name"], layer, path)?;

    let provider_path = join_path(path, "provider");
    let provider = map
        .get("provider")
        .ok_or_else(|| invalid_field(layer, &provider_path, "missing required field"))?;
    expect_string(provider, layer, &provider_path)?;

    let name_path = join_path(path, "name");
    let name = map
        .get("name")
        .ok_or_else(|| invalid_field(layer, &name_path, "missing required field"))?;
    expect_string(name, layer, &name_path)?;

    Ok(())
}

/// Validate a tool allow/deny policy.
fn validate_tool_policy(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    ensure_allowed_keys(map, &["allow", "deny"], layer, path)?;

    if let Some(value) = map.get("allow") {
        validate_string_array(value, layer, &join_path(path, "allow"))?;
    }
    if let Some(value) = map.get("deny") {
        validate_string_array(value, layer, &join_path(path, "deny"))?;
    }
    Ok(())
}

/// Validate the global tools block.
fn validate_tools(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    ensure_allowed_keys(map, &["output_policy"], layer, path)?;

    if let Some(value) = map.get("output_policy") {
        validate_tool_output_policy(value, layer, &join_path(path, "output_policy"))?;
    }
    Ok(())
}

/// Validate the tool output policy block.
fn validate_tool_output_policy(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    let allowed = [
        "max_string_bytes",
        "max_array_len",
        "max_object_entries",
        "redact_keys",
        "redact_values",
        "replacement",
    ];
    ensure_allowed_keys(map, &allowed, layer, path)?;

    if let Some(value) = map.get("max_string_bytes") {
        expect_u64(value, layer, &join_path(path, "max_string_bytes"))?;
    }
    if let Some(value) = map.get("max_array_len") {
        expect_u64(value, layer, &join_path(path, "max_array_len"))?;
    }
    if let Some(value) = map.get("max_object_entries") {
        expect_u64(value, layer, &join_path(path, "max_object_entries"))?;
    }
    if let Some(value) = map.get("redact_keys") {
        validate_string_array(value, layer, &join_path(path, "redact_keys"))?;
    }
    if let Some(value) = map.get("redact_values") {
        validate_string_array(value, layer, &join_path(path, "redact_values"))?;
    }
    if let Some(value) = map.get("replacement") {
        expect_string(value, layer, &join_path(path, "replacement"))?;
    }
    Ok(())
}

/// Validate the global permissions block.
fn validate_permissions(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    ensure_allowed_keys(map, &["mode", "rules"], layer, path)?;

    if let Some(value) = map.get("mode") {
        validate_permission_mode(value, layer, &join_path(path, "mode"))?;
    }
    if let Some(value) = map.get("rules") {
        let arr = expect_array(value, layer, &join_path(path, "rules"))?;
        for (idx, entry) in arr.iter().enumerate() {
            validate_permission_rule(entry, layer, &format!("{path}.rules[{idx}]"))?;
        }
    }
    Ok(())
}

/// Validate permission mode values.
fn validate_permission_mode(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let Some(mode) = value.as_str() else {
        return Err(invalid_field(layer, path, "expected string"));
    };
    if matches!(
        mode,
        "default" | "accept_edits" | "bypass_permissions" | "plan"
    ) {
        Ok(())
    } else {
        Err(invalid_field(layer, path, "invalid permission mode"))
    }
}

/// Validate a single permission rule entry.
fn validate_permission_rule(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    let allowed = ["action", "tool", "path", "command", "access"];
    ensure_allowed_keys(map, &allowed, layer, path)?;

    let action_path = join_path(path, "action");
    let action = map
        .get("action")
        .ok_or_else(|| invalid_field(layer, &action_path, "missing required field"))?;
    validate_permission_action(action, layer, &action_path)?;

    if let Some(value) = map.get("tool") {
        expect_string(value, layer, &join_path(path, "tool"))?;
    }
    if let Some(value) = map.get("path") {
        expect_string(value, layer, &join_path(path, "path"))?;
    }
    if let Some(value) = map.get("command") {
        validate_string_array(value, layer, &join_path(path, "command"))?;
    }
    if let Some(value) = map.get("access") {
        validate_path_access(value, layer, &join_path(path, "access"))?;
    }
    Ok(())
}

/// Validate permission action values.
fn validate_permission_action(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let Some(action) = value.as_str() else {
        return Err(invalid_field(layer, path, "expected string"));
    };
    if matches!(action, "allow" | "deny" | "ask") {
        Ok(())
    } else {
        Err(invalid_field(layer, path, "invalid permission action"))
    }
}

/// Validate path access mode values.
fn validate_path_access(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let Some(access) = value.as_str() else {
        return Err(invalid_field(layer, path, "expected string"));
    };
    if matches!(access, "read" | "write" | "execute") {
        Ok(())
    } else {
        Err(invalid_field(layer, path, "invalid access mode"))
    }
}

/// Validate the global memory block.
fn validate_memory(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    let allowed = [
        "path",
        "recall_k",
        "instruction_roots",
        "capture_tool_output",
    ];
    ensure_allowed_keys(map, &allowed, layer, path)?;

    if let Some(value) = map.get("path") {
        expect_string(value, layer, &join_path(path, "path"))?;
    }
    if let Some(value) = map.get("recall_k") {
        expect_u64(value, layer, &join_path(path, "recall_k"))?;
    }
    if let Some(value) = map.get("instruction_roots") {
        validate_string_array(value, layer, &join_path(path, "instruction_roots"))?;
    }
    if let Some(value) = map.get("capture_tool_output") {
        expect_bool(value, layer, &join_path(path, "capture_tool_output"))?;
    }
    Ok(())
}

/// Validate the skills block.
fn validate_skills(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    let allowed = [
        "enabled",
        "setting_sources",
        "settingSources",
        "paths",
        "allow",
        "deny",
    ];
    ensure_allowed_keys(map, &allowed, layer, path)?;

    if let Some(value) = map.get("enabled") {
        expect_bool(value, layer, &join_path(path, "enabled"))?;
    }
    if let Some(value) = map.get("setting_sources") {
        validate_setting_sources(value, layer, &join_path(path, "setting_sources"))?;
    }
    if let Some(value) = map.get("settingSources") {
        validate_setting_sources(value, layer, &join_path(path, "settingSources"))?;
    }
    if let Some(value) = map.get("paths") {
        validate_string_array(value, layer, &join_path(path, "paths"))?;
    }
    if let Some(value) = map.get("allow") {
        validate_string_array(value, layer, &join_path(path, "allow"))?;
    }
    if let Some(value) = map.get("deny") {
        validate_string_array(value, layer, &join_path(path, "deny"))?;
    }
    Ok(())
}

/// Validate skill setting source values.
fn validate_setting_sources(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let arr = expect_array(value, layer, path)?;
    for (idx, entry) in arr.iter().enumerate() {
        let Some(source) = entry.as_str() else {
            return Err(invalid_field(
                layer,
                &format!("{path}[{idx}]"),
                "expected string",
            ));
        };
        if !matches!(source, "user" | "project" | "system") {
            return Err(invalid_field(
                layer,
                &format!("{path}[{idx}]"),
                "invalid setting source",
            ));
        }
    }
    Ok(())
}

/// Validate MCP client configuration.
fn validate_mcp(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    ensure_allowed_keys(map, &["enabled", "servers"], layer, path)?;

    if let Some(value) = map.get("enabled") {
        expect_bool(value, layer, &join_path(path, "enabled"))?;
    }
    if let Some(value) = map.get("servers") {
        let arr = expect_array(value, layer, &join_path(path, "servers"))?;
        for (idx, entry) in arr.iter().enumerate() {
            validate_mcp_server(entry, layer, &format!("{path}.servers[{idx}]"))?;
        }
    }
    Ok(())
}

/// Validate a single MCP server entry.
fn validate_mcp_server(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    let allowed = [
        "name",
        "protocol",
        "command",
        "args",
        "env",
        "cwd",
        "description",
        "sandbox",
    ];
    ensure_allowed_keys(map, &allowed, layer, path)?;

    for key in ["name", "command"] {
        let key_path = join_path(path, key);
        let value = map
            .get(key)
            .ok_or_else(|| invalid_field(layer, &key_path, "missing required field"))?;
        expect_string(value, layer, &key_path)?;
    }

    if let Some(value) = map.get("protocol") {
        let protocol_path = join_path(path, "protocol");
        let Some(protocol) = value.as_str() else {
            return Err(invalid_field(layer, &protocol_path, "expected string"));
        };
        if protocol != "stdio" {
            return Err(invalid_field(
                layer,
                &protocol_path,
                "only stdio MCP servers are supported",
            ));
        }
    }
    if let Some(value) = map.get("args") {
        validate_string_array(value, layer, &join_path(path, "args"))?;
    }
    if let Some(value) = map.get("env") {
        let env_map = expect_object(value, layer, &join_path(path, "env"))?;
        for (key, value) in env_map {
            if value.as_str().is_none() {
                return Err(invalid_field(
                    layer,
                    &join_path(&join_path(path, "env"), key),
                    "expected string",
                ));
            }
        }
    }
    if let Some(value) = map.get("cwd") {
        expect_string(value, layer, &join_path(path, "cwd"))?;
    }
    if let Some(value) = map.get("description") {
        expect_string(value, layer, &join_path(path, "description"))?;
    }
    if let Some(value) = map.get("sandbox") {
        validate_mcp_server_sandbox(value, layer, &join_path(path, "sandbox"))?;
    }
    Ok(())
}

/// Validate sandbox overrides for an MCP server.
fn validate_mcp_server_sandbox(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    ensure_allowed_keys(
        map,
        &["filesystem", "network", "env", "limits"],
        layer,
        path,
    )?;

    if let Some(value) = map.get("filesystem") {
        validate_sandbox_filesystem(value, layer, &join_path(path, "filesystem"))?;
    }
    if let Some(value) = map.get("network") {
        validate_sandbox_network(value, layer, &join_path(path, "network"))?;
    }
    if let Some(value) = map.get("env") {
        validate_sandbox_env(value, layer, &join_path(path, "env"))?;
    }
    if let Some(value) = map.get("limits") {
        validate_sandbox_limits(value, layer, &join_path(path, "limits"))?;
    }
    Ok(())
}

/// Validate sandbox configuration.
fn validate_sandbox(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    let allowed = [
        "enabled",
        "provider",
        "mode",
        "filesystem",
        "network",
        "env",
        "limits",
    ];
    ensure_allowed_keys(map, &allowed, layer, path)?;

    if let Some(value) = map.get("enabled") {
        expect_bool(value, layer, &join_path(path, "enabled"))?;
    }
    if let Some(value) = map.get("provider") {
        expect_string(value, layer, &join_path(path, "provider"))?;
    }
    if let Some(value) = map.get("mode") {
        validate_sandbox_mode(value, layer, &join_path(path, "mode"))?;
    }
    if let Some(value) = map.get("filesystem") {
        validate_sandbox_filesystem(value, layer, &join_path(path, "filesystem"))?;
    }
    if let Some(value) = map.get("network") {
        validate_sandbox_network(value, layer, &join_path(path, "network"))?;
    }
    if let Some(value) = map.get("env") {
        validate_sandbox_env(value, layer, &join_path(path, "env"))?;
    }
    if let Some(value) = map.get("limits") {
        validate_sandbox_limits(value, layer, &join_path(path, "limits"))?;
    }
    Ok(())
}

/// Validate sandbox mode values.
fn validate_sandbox_mode(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let Some(mode) = value.as_str() else {
        return Err(invalid_field(layer, path, "expected string"));
    };
    if matches!(mode, "read_only" | "workspace_write" | "danger_full_access") {
        Ok(())
    } else {
        Err(invalid_field(layer, path, "invalid sandbox mode"))
    }
}

/// Validate filesystem sandbox configuration.
fn validate_sandbox_filesystem(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    ensure_allowed_keys(map, &["read", "write", "exec"], layer, path)?;

    for key in ["read", "write", "exec"] {
        if let Some(value) = map.get(key) {
            validate_string_array(value, layer, &join_path(path, key))?;
        }
    }

    Ok(())
}

/// Validate network sandbox configuration.
fn validate_sandbox_network(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    ensure_allowed_keys(map, &["mode"], layer, path)?;

    if let Some(value) = map.get("mode") {
        let Some(mode) = value.as_str() else {
            return Err(invalid_field(
                layer,
                &join_path(path, "mode"),
                "expected string",
            ));
        };
        if !matches!(mode, "disabled" | "allow_all") {
            return Err(invalid_field(
                layer,
                &join_path(path, "mode"),
                "invalid sandbox network mode",
            ));
        }
    }
    Ok(())
}

/// Validate environment sandbox configuration.
fn validate_sandbox_env(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    ensure_allowed_keys(map, &["inherit", "set"], layer, path)?;

    if let Some(value) = map.get("inherit") {
        validate_string_array(value, layer, &join_path(path, "inherit"))?;
    }
    if let Some(value) = map.get("set") {
        let set_map = expect_object(value, layer, &join_path(path, "set"))?;
        for (key, value) in set_map {
            if value.as_str().is_none() {
                return Err(invalid_field(
                    layer,
                    &join_path(&join_path(path, "set"), key),
                    "expected string",
                ));
            }
        }
    }
    Ok(())
}

/// Validate sandbox limits configuration.
fn validate_sandbox_limits(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    ensure_allowed_keys(
        map,
        &[
            "cpu_seconds",
            "memory_bytes",
            "nofile",
            "pids",
            "wall_clock_seconds",
            "stdout_bytes",
            "stderr_bytes",
        ],
        layer,
        path,
    )?;

    for key in [
        "cpu_seconds",
        "memory_bytes",
        "nofile",
        "pids",
        "wall_clock_seconds",
        "stdout_bytes",
        "stderr_bytes",
    ] {
        if let Some(value) = map.get(key) {
            expect_u64(value, layer, &join_path(path, key))?;
        }
    }
    Ok(())
}

/// Validate per-agent sandbox overrides.
fn validate_agent_sandbox(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    ensure_allowed_keys(map, &["enabled", "provider", "mode"], layer, path)?;

    if let Some(value) = map.get("enabled") {
        expect_bool(value, layer, &join_path(path, "enabled"))?;
    }
    if let Some(value) = map.get("provider") {
        expect_string(value, layer, &join_path(path, "provider"))?;
    }
    if let Some(value) = map.get("mode") {
        validate_sandbox_mode(value, layer, &join_path(path, "mode"))?;
    }
    Ok(())
}

/// Validate per-agent permission overrides.
fn validate_agent_permissions(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    ensure_allowed_keys(map, &["mode"], layer, path)?;

    if let Some(value) = map.get("mode") {
        validate_permission_mode(value, layer, &join_path(path, "mode"))?;
    }
    Ok(())
}

/// Validate session persistence configuration.
fn validate_sessions(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let map = expect_object(value, layer, path)?;
    ensure_allowed_keys(map, &["enabled", "provider", "path"], layer, path)?;

    if let Some(value) = map.get("enabled") {
        expect_bool(value, layer, &join_path(path, "enabled"))?;
    }
    if let Some(value) = map.get("provider") {
        expect_string(value, layer, &join_path(path, "provider"))?;
    }
    if let Some(value) = map.get("path") {
        expect_string(value, layer, &join_path(path, "path"))?;
    }
    Ok(())
}

/// Expect a JSON object or return a typed error.
fn expect_object<'a>(
    value: &'a Value,
    layer: &str,
    path: &str,
) -> Result<&'a Map<String, Value>, ConfigError> {
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(invalid_field(layer, path, "expected object")),
    }
}

/// Expect a JSON array or return a typed error.
fn expect_array<'a>(
    value: &'a Value,
    layer: &str,
    path: &str,
) -> Result<&'a Vec<Value>, ConfigError> {
    match value {
        Value::Array(arr) => Ok(arr),
        _ => Err(invalid_field(layer, path, "expected array")),
    }
}

/// Expect a JSON string or return a typed error.
fn expect_string(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    if value.as_str().is_some() {
        Ok(())
    } else {
        Err(invalid_field(layer, path, "expected string"))
    }
}

/// Expect a JSON boolean or return a typed error.
fn expect_bool(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    if matches!(value, Value::Bool(_)) {
        Ok(())
    } else {
        Err(invalid_field(layer, path, "expected bool"))
    }
}

/// Expect a JSON u64 or return a typed error.
fn expect_u64(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    if value.is_u64() || value.is_i64() {
        Ok(())
    } else {
        Err(invalid_field(layer, path, "expected integer"))
    }
}

/// Validate that a value is an array of strings.
fn validate_string_array(value: &Value, layer: &str, path: &str) -> Result<(), ConfigError> {
    let arr = match value {
        Value::Array(arr) => arr,
        _ => return Err(invalid_field(layer, path, "expected array")),
    };
    for (idx, entry) in arr.iter().enumerate() {
        if entry.as_str().is_none() {
            return Err(invalid_field(
                layer,
                &format!("{path}[{idx}]"),
                "expected string",
            ));
        }
    }
    Ok(())
}

/// Ensure an object contains only allowed keys.
fn ensure_allowed_keys(
    map: &Map<String, Value>,
    allowed: &[&str],
    layer: &str,
    path: &str,
) -> Result<(), ConfigError> {
    for key in map.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(invalid_field(layer, &join_path(path, key), "unknown key"));
        }
    }
    Ok(())
}

/// Join nested paths for better error messages.
fn join_path(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

/// Build a structured invalid-field error.
fn invalid_field(layer: &str, path: &str, message: &str) -> ConfigError {
    let normalized_path = if path.is_empty() { "root" } else { path };
    ConfigError::InvalidField {
        path: format!("{layer}:{normalized_path}"),
        message: message.to_string(),
    }
}
