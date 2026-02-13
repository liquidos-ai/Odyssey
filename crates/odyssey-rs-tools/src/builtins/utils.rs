//! Utility helpers shared by built-in tools.

use crate::ToolContext;
use odyssey_rs_protocol::ToolError;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// Controls how path resolution treats missing components.
#[derive(Debug, Clone, Copy)]
pub(super) enum ResolveMode {
    /// Require the path to exist.
    Existing,
    /// Allow non-existent target paths.
    AllowMissing,
}

/// Parse JSON args into a typed struct for tool calls.
pub(super) fn parse_args<T: DeserializeOwned>(args: Value) -> Result<T, ToolError> {
    serde_json::from_value(args).map_err(|err| ToolError::InvalidArguments(err.to_string()))
}

/// Resolve a workspace-relative path and validate it.
pub(super) fn resolve_workspace_path(
    ctx: &ToolContext,
    input: &str,
    mode: ResolveMode,
) -> Result<PathBuf, ToolError> {
    if input.trim().is_empty() {
        return Err(ToolError::InvalidArguments(
            "path cannot be empty".to_string(),
        ));
    }
    let root = &ctx.services.workspace_root;
    let resolved = normalize_relative_path(root, input)?;
    ensure_within_root(root, &resolved, mode)?;
    Ok(resolved)
}

/// Format a path relative to a root for display.
pub(super) fn relative_display(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

/// Normalize a relative path while preventing root escape.
fn normalize_relative_path(root: &Path, input: &str) -> Result<PathBuf, ToolError> {
    let path = Path::new(input);
    if path.is_absolute() {
        return Err(ToolError::InvalidArguments(
            "path must be relative to workspace root".to_string(),
        ));
    }

    let mut parts: Vec<OsString> = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_os_string()),
            Component::CurDir => (),
            Component::ParentDir => {
                if parts.pop().is_none() {
                    return Err(ToolError::InvalidArguments(
                        "path escapes workspace root".to_string(),
                    ));
                }
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(ToolError::InvalidArguments(
                    "path must be relative to workspace root".to_string(),
                ));
            }
        }
    }

    let mut resolved = root.to_path_buf();
    for part in parts {
        resolved.push(part);
    }
    Ok(resolved)
}

/// Ensure a resolved path stays within the workspace root.
fn ensure_within_root(root: &Path, path: &Path, mode: ResolveMode) -> Result<(), ToolError> {
    let root = root.canonicalize().map_err(|err| {
        ToolError::ExecutionFailed(format!("failed to resolve workspace root: {err}"))
    })?;

    let target = match mode {
        ResolveMode::Existing => path.canonicalize().map_err(|err| {
            ToolError::ExecutionFailed(format!("failed to resolve path {path:?}: {err}"))
        })?,
        ResolveMode::AllowMissing => {
            let existing = find_existing_parent(path).ok_or_else(|| {
                ToolError::ExecutionFailed("path has no existing parent".to_string())
            })?;
            existing.canonicalize().map_err(|err| {
                ToolError::ExecutionFailed(format!("failed to resolve path {existing:?}: {err}"))
            })?
        }
    };

    if !target.starts_with(&root) {
        return Err(ToolError::PermissionDenied(
            "path is outside workspace root".to_string(),
        ));
    }

    Ok(())
}

/// Find the nearest existing parent path for a non-existent target.
fn find_existing_parent(path: &Path) -> Option<&Path> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate.exists() {
            return Some(candidate);
        }
        current = candidate.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{ResolveMode, parse_args, relative_display, resolve_workspace_path};
    use crate::{ToolContext, TurnServices};
    use odyssey_rs_protocol::ToolError;
    use pretty_assertions::assert_eq;
    use serde::Deserialize;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use tempfile::tempdir;
    use uuid::Uuid;

    fn context_for_root(root: &Path) -> ToolContext {
        ToolContext {
            session_id: Uuid::nil(),
            agent_id: "agent".to_string(),
            turn_id: None,
            tool_call_id: None,
            tool_name: None,
            services: Arc::new(TurnServices {
                cwd: root.to_path_buf(),
                workspace_root: root.to_path_buf(),
                output_policy: None,
                sandbox: None,
                web: None,
                event_sink: None,
                skill_provider: None,
                question_handler: None,
                permission_checker: None,
                tool_result_handler: None,
            }),
        }
    }

    #[test]
    fn resolve_workspace_path_rejects_empty() {
        let temp = tempdir().expect("tempdir");
        let ctx = context_for_root(temp.path());
        let err = resolve_workspace_path(&ctx, "", ResolveMode::Existing).expect_err("error");
        match err {
            ToolError::InvalidArguments(message) => {
                assert_eq!(message, "path cannot be empty");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn resolve_workspace_path_allows_existing_file() {
        let temp = tempdir().expect("tempdir");
        let ctx = context_for_root(temp.path());
        let path = temp.path().join("file.txt");
        std::fs::write(&path, "data").expect("write");

        let resolved =
            resolve_workspace_path(&ctx, "file.txt", ResolveMode::Existing).expect("resolved");
        assert_eq!(resolved, path);
    }

    #[test]
    fn resolve_workspace_path_allows_missing_target() {
        let temp = tempdir().expect("tempdir");
        let ctx = context_for_root(temp.path());
        let resolved = resolve_workspace_path(&ctx, "missing.txt", ResolveMode::AllowMissing)
            .expect("resolved");
        assert_eq!(resolved, temp.path().join("missing.txt"));
    }

    #[test]
    fn resolve_workspace_path_blocks_escape() {
        let temp = tempdir().expect("tempdir");
        let ctx = context_for_root(temp.path());
        let err = resolve_workspace_path(&ctx, "../outside", ResolveMode::AllowMissing)
            .expect_err("error");
        match err {
            ToolError::InvalidArguments(message) => {
                assert_eq!(message, "path escapes workspace root");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn relative_display_prefers_relative_path() {
        let root = PathBuf::from("/workspace");
        let path = PathBuf::from("/workspace/sub/file.txt");
        assert_eq!(relative_display(&root, &path), "sub/file.txt".to_string());
    }

    #[test]
    fn relative_display_falls_back_to_absolute() {
        let root = PathBuf::from("/workspace");
        let path = PathBuf::from("/other/file.txt");
        assert_eq!(
            relative_display(&root, &path),
            "/other/file.txt".to_string()
        );
    }

    #[test]
    fn parse_args_reads_struct_fields() {
        #[derive(Deserialize)]
        struct Args {
            name: String,
        }

        let args: Args = parse_args(serde_json::json!({ "name": "odyssey" })).expect("args");
        assert_eq!(args.name, "odyssey".to_string());
    }

    #[test]
    fn resolve_workspace_path_rejects_absolute_paths() {
        let temp = tempdir().expect("tempdir");
        let ctx = context_for_root(temp.path());
        let err = resolve_workspace_path(&ctx, "/tmp/file.txt", ResolveMode::AllowMissing)
            .expect_err("error");
        match err {
            ToolError::InvalidArguments(message) => {
                assert_eq!(message, "path must be relative to workspace root");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
