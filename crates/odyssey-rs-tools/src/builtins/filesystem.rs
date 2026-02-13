//! Built-in filesystem tools (read/write/edit/glob/grep).

use crate::builtins::utils::{ResolveMode, parse_args, relative_display, resolve_workspace_path};
use crate::{Tool, ToolContext};
use async_trait::async_trait;
use autoagents_core::tool::ToolInputT;
use autoagents_derive::ToolInput;
use log::{debug, info};
use odyssey_rs_protocol::PathAccess;
use odyssey_rs_protocol::ToolError;
use odyssey_rs_sandbox::AccessMode;
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::Path;
use walkdir::WalkDir;

/// Default maximum number of bytes to read from a file.
const DEFAULT_MAX_READ_BYTES: usize = 200_000;
/// Default maximum number of results for glob/grep.
const DEFAULT_MAX_RESULTS: usize = 200;

/// Tool for reading workspace files.
#[derive(Debug, Default)]
pub struct ReadTool;

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "Read"
    }

    fn description(&self) -> &str {
        "Read a text file from the workspace"
    }

    fn args_schema(&self) -> Value {
        let params_str = ReadArgs::io_schema();
        serde_json::from_str(params_str).expect("Error parsing tool parameters")
    }

    fn supports_parallel(&self) -> bool {
        true
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let input: ReadArgs = parse_args(args)?;
        let path = resolve_workspace_path(ctx, &input.path, ResolveMode::Existing)?;
        ctx.authorize_path(&path, PathAccess::Read).await?;
        ctx.check_access(&path, AccessMode::Read)?;

        let metadata = fs::metadata(&path).map_err(|err| {
            ToolError::ExecutionFailed(format!("failed to read metadata for {path:?}: {err}"))
        })?;
        if metadata.is_dir() {
            return Err(ToolError::ExecutionFailed(
                "path is a directory".to_string(),
            ));
        }

        let bytes = fs::read(&path)
            .map_err(|err| ToolError::ExecutionFailed(format!("failed to read {path:?}: {err}")))?;
        let max_bytes = input.max_bytes.unwrap_or_else(|| {
            ctx.services
                .output_policy
                .as_ref()
                .map(|policy| policy.max_string_bytes)
                .unwrap_or(DEFAULT_MAX_READ_BYTES)
        });
        let truncated = bytes.len() > max_bytes;
        let slice = if truncated {
            &bytes[..max_bytes]
        } else {
            &bytes
        };
        let content = String::from_utf8_lossy(slice).to_string();
        info!(
            "read file (bytes_read={}, truncated={})",
            slice.len(),
            truncated
        );

        Ok(json!({
            "path": relative_display(&ctx.services.workspace_root, &path),
            "content": content,
            "truncated": truncated,
            "bytes_read": slice.len(),
        }))
    }
}

/// Tool for writing or overwriting workspace files.
#[derive(Debug, Default)]
pub struct WriteTool;

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "Write"
    }

    fn description(&self) -> &str {
        "Create or overwrite a file in the workspace"
    }

    fn args_schema(&self) -> Value {
        let params_str = WriteArgs::io_schema();
        serde_json::from_str(params_str).expect("Error parsing tool parameters")
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let input: WriteArgs = parse_args(args)?;
        let path = resolve_workspace_path(ctx, &input.path, ResolveMode::AllowMissing)?;
        ctx.authorize_path(&path, PathAccess::Write).await?;
        ctx.check_access(&path, AccessMode::Write)?;

        let existed = path.exists();
        if existed && !input.overwrite {
            return Err(ToolError::ExecutionFailed(
                "file exists; set overwrite to true to replace".to_string(),
            ));
        }
        if path.is_dir() {
            return Err(ToolError::ExecutionFailed(
                "path is a directory".to_string(),
            ));
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                ToolError::ExecutionFailed(format!("failed to create directories: {err}"))
            })?;
        }

        fs::write(&path, input.content.as_bytes())
            .map_err(|err| ToolError::ExecutionFailed(format!("failed to write file: {err}")))?;
        info!(
            "wrote file (bytes_written={}, overwritten={})",
            input.content.len(),
            existed
        );

        Ok(json!({
            "path": relative_display(&ctx.services.workspace_root, &path),
            "bytes_written": input.content.len(),
            "overwritten": existed,
        }))
    }
}

/// Tool for replacing text in a file.
#[derive(Debug, Default)]
pub struct EditTool;

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "Edit"
    }

    fn description(&self) -> &str {
        "Replace text in an existing file"
    }

    fn args_schema(&self) -> Value {
        let params_str = EditArgs::io_schema();
        serde_json::from_str(params_str).expect("Error parsing tool parameters")
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let input: EditArgs = parse_args(args)?;
        if input.old_text.is_empty() {
            return Err(ToolError::InvalidArguments(
                "old_text cannot be empty".to_string(),
            ));
        }
        let path = resolve_workspace_path(ctx, &input.path, ResolveMode::Existing)?;
        ctx.authorize_path(&path, PathAccess::Read).await?;
        ctx.authorize_path(&path, PathAccess::Write).await?;
        ctx.check_access(&path, AccessMode::Read)?;
        ctx.check_access(&path, AccessMode::Write)?;

        let content = fs::read_to_string(&path)
            .map_err(|err| ToolError::ExecutionFailed(format!("failed to read file: {err}")))?;
        let occurrences = content.match_indices(&input.old_text).count();
        if occurrences == 0 {
            return Err(ToolError::ExecutionFailed(
                "no matches found for old_text".to_string(),
            ));
        }
        if occurrences > 1 && !input.replace_all {
            return Err(ToolError::ExecutionFailed(
                "multiple matches found; set replace_all to true or provide a unique snippet"
                    .to_string(),
            ));
        }

        let updated = if input.replace_all {
            content.replace(&input.old_text, &input.new_text)
        } else {
            content.replacen(&input.old_text, &input.new_text, 1)
        };
        fs::write(&path, updated.as_bytes())
            .map_err(|err| ToolError::ExecutionFailed(format!("failed to write file: {err}")))?;
        info!(
            "edited file (replacements={})",
            if input.replace_all { occurrences } else { 1 }
        );

        Ok(json!({
            "path": relative_display(&ctx.services.workspace_root, &path),
            "replaced": if input.replace_all { occurrences } else { 1 },
        }))
    }
}

/// Tool for globbing workspace files.
#[derive(Debug, Default)]
pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "Glob"
    }

    fn description(&self) -> &str {
        "Find files by glob pattern within the workspace"
    }

    fn args_schema(&self) -> Value {
        let params_str = GlobArgs::io_schema();
        serde_json::from_str(params_str).expect("Error parsing tool parameters")
    }

    fn supports_parallel(&self) -> bool {
        true
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let input: GlobArgs = parse_args(args)?;
        let root = match input.root.as_deref() {
            Some(root) => resolve_workspace_path(ctx, root, ResolveMode::Existing)?,
            None => ctx.services.workspace_root.clone(),
        };
        if !root.is_dir() {
            return Err(ToolError::ExecutionFailed(
                "root is not a directory".to_string(),
            ));
        }
        ctx.authorize_path(&root, PathAccess::Read).await?;
        ctx.check_access(&root, AccessMode::Read)?;

        let mut builder = globset::GlobSetBuilder::new();
        let glob = globset::Glob::new(&input.pattern)
            .map_err(|err| ToolError::InvalidArguments(err.to_string()))?;
        builder.add(glob);
        let set = builder
            .build()
            .map_err(|err| ToolError::InvalidArguments(err.to_string()))?;

        let max_results = input.max_results.unwrap_or_else(|| {
            ctx.services
                .output_policy
                .as_ref()
                .map(|policy| policy.max_array_len)
                .unwrap_or(DEFAULT_MAX_RESULTS)
        });

        let mut matches = Vec::new();
        let mut truncated = false;
        for entry in WalkDir::new(&root) {
            let entry = entry.map_err(|err| {
                ToolError::ExecutionFailed(format!("failed to walk directory: {err}"))
            })?;
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let relative = path.strip_prefix(&root).unwrap_or(path);
            if set.is_match(relative) {
                ctx.check_access(path, AccessMode::Read)?;
                matches.push(relative_display(&ctx.services.workspace_root, path));
                if matches.len() >= max_results {
                    truncated = true;
                    break;
                }
            }
        }
        info!(
            "glob completed (matches={}, truncated={})",
            matches.len(),
            truncated
        );

        Ok(json!({
            "matches": matches,
            "truncated": truncated,
        }))
    }
}

/// Tool for searching file contents with a regex.
#[derive(Debug, Default)]
pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "Grep"
    }

    fn description(&self) -> &str {
        "Search file contents with a regular expression"
    }

    fn args_schema(&self) -> Value {
        let params_str = GrepArgs::io_schema();
        serde_json::from_str(params_str).expect("Error parsing tool parameters")
    }

    fn supports_parallel(&self) -> bool {
        true
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let input: GrepArgs = parse_args(args)?;
        let root = match input.path.as_deref() {
            Some(path) => resolve_workspace_path(ctx, path, ResolveMode::Existing)?,
            None => ctx.services.workspace_root.clone(),
        };
        ctx.authorize_path(&root, PathAccess::Read).await?;
        let case_sensitive = input.case_sensitive.unwrap_or(true);
        debug!(
            "grep starting (case_sensitive={}, has_glob={})",
            case_sensitive,
            input.glob.is_some()
        );
        let regex = RegexBuilder::new(&input.pattern)
            .case_insensitive(!case_sensitive)
            .build()
            .map_err(|err| ToolError::InvalidArguments(err.to_string()))?;

        let mut glob = None;
        if let Some(pattern) = input.glob.as_ref() {
            let mut builder = globset::GlobSetBuilder::new();
            let glob_pattern = globset::Glob::new(pattern)
                .map_err(|err| ToolError::InvalidArguments(err.to_string()))?;
            builder.add(glob_pattern);
            let set = builder
                .build()
                .map_err(|err| ToolError::InvalidArguments(err.to_string()))?;
            glob = Some(set);
        }

        let max_results = input.max_results.unwrap_or_else(|| {
            ctx.services
                .output_policy
                .as_ref()
                .map(|policy| policy.max_array_len)
                .unwrap_or(DEFAULT_MAX_RESULTS)
        });

        let mut matches = Vec::new();
        let mut truncated = false;

        if root.is_file() {
            search_file(
                ctx,
                &regex,
                glob.as_ref(),
                &root,
                &mut matches,
                max_results,
                &mut truncated,
            )?;
        } else {
            for entry in WalkDir::new(&root) {
                let entry = entry.map_err(|err| {
                    ToolError::ExecutionFailed(format!("failed to walk directory: {err}"))
                })?;
                if !entry.file_type().is_file() {
                    continue;
                }
                if truncated {
                    break;
                }
                let path = entry.path();
                search_file(
                    ctx,
                    &regex,
                    glob.as_ref(),
                    path,
                    &mut matches,
                    max_results,
                    &mut truncated,
                )?;
            }
        }
        info!(
            "grep completed (matches={}, truncated={})",
            matches.len(),
            truncated
        );

        Ok(json!({
            "pattern": input.pattern,
            "matches": matches,
            "truncated": truncated,
        }))
    }
}

/// Search a file and append matching lines into the results vector.
fn search_file(
    ctx: &ToolContext,
    regex: &regex::Regex,
    glob: Option<&globset::GlobSet>,
    path: &Path,
    matches: &mut Vec<Value>,
    max_results: usize,
    truncated: &mut bool,
) -> Result<(), ToolError> {
    if let Some(set) = glob {
        let relative = path
            .strip_prefix(&ctx.services.workspace_root)
            .unwrap_or(path);
        if !set.is_match(relative) {
            return Ok(());
        }
    }
    ctx.check_access(path, AccessMode::Read)?;

    let file = File::open(path)
        .map_err(|err| ToolError::ExecutionFailed(format!("failed to open file: {err}")))?;
    let reader = BufReader::new(file);

    for (index, line_result) in reader.lines().enumerate() {
        let line = match line_result {
            Ok(line) => line,
            Err(err) => {
                if err.kind() == std::io::ErrorKind::InvalidData {
                    return Ok(());
                }
                return Err(ToolError::ExecutionFailed(format!(
                    "failed to read line: {err}"
                )));
            }
        };
        if regex.is_match(&line) {
            matches.push(json!({
                "path": relative_display(&ctx.services.workspace_root, path),
                "line": index + 1,
                "text": line,
            }));
            if matches.len() >= max_results {
                *truncated = true;
                break;
            }
        }
    }

    Ok(())
}

/// Arguments for ReadTool.
#[derive(Debug, Serialize, Deserialize, ToolInput)]
struct ReadArgs {
    #[input(description = "Path to the file to read.")]
    path: String,
    #[input(description = "Maximum number of bytes to read.")]
    #[serde(default)]
    max_bytes: Option<usize>,
}

/// Arguments for WriteTool.
#[derive(Debug, Serialize, Deserialize, ToolInput)]
struct WriteArgs {
    #[input(description = "Path to the file to write.")]
    path: String,
    #[input(description = "Content to write into the file.")]
    content: String,
    #[input(description = "Overwrite the file if it already exists.")]
    #[serde(default)]
    overwrite: bool,
}

/// Arguments for EditTool.
#[derive(Debug, Serialize, Deserialize, ToolInput)]
struct EditArgs {
    #[input(description = "Path to the file to edit.")]
    path: String,
    #[input(description = "Text to search for in the file.")]
    old_text: String,
    #[input(description = "Replacement text.")]
    new_text: String,
    #[input(description = "Replace all occurrences instead of the first match.")]
    #[serde(default)]
    replace_all: bool,
}

/// Arguments for GlobTool.
#[derive(Debug, Serialize, Deserialize, ToolInput)]
struct GlobArgs {
    #[input(description = "Glob pattern to match.")]
    pattern: String,
    #[input(description = "Optional root directory to search from.")]
    #[serde(default)]
    root: Option<String>,
    #[input(description = "Maximum number of results to return.")]
    #[serde(default)]
    max_results: Option<usize>,
}

/// Arguments for GrepTool.
#[derive(Debug, Serialize, Deserialize, ToolInput)]
struct GrepArgs {
    #[input(description = "Regex pattern to search for.")]
    pattern: String,
    #[input(description = "Optional file or directory path to search.")]
    #[serde(default)]
    path: Option<String>,
    #[input(description = "Optional glob filter to apply to file paths.")]
    #[serde(default)]
    glob: Option<String>,
    #[input(description = "Case sensitive search when true.")]
    #[serde(default)]
    case_sensitive: Option<bool>,
    #[input(description = "Maximum number of results to return.")]
    #[serde(default)]
    max_results: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::{EditTool, GlobTool, GrepTool, ReadTool, WriteTool};
    use crate::{Tool, ToolContext, TurnServices};
    use odyssey_rs_protocol::ToolError;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::tempdir;
    use uuid::Uuid;

    fn context_for_root(root: &std::path::Path) -> ToolContext {
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

    #[tokio::test]
    async fn read_tool_reads_file() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("file.txt");
        std::fs::write(&path, "hello").expect("write");
        let ctx = context_for_root(temp.path());
        let tool = ReadTool;

        let result = tool
            .call(&ctx, json!({ "path": "file.txt" }))
            .await
            .expect("read");

        assert_eq!(result["content"], "hello");
        assert_eq!(result["truncated"], false);
    }

    #[tokio::test]
    async fn write_tool_creates_file() {
        let temp = tempdir().expect("tempdir");
        let ctx = context_for_root(temp.path());
        let tool = WriteTool;

        let result = tool
            .call(
                &ctx,
                json!({
                    "path": "out.txt",
                    "content": "data",
                    "overwrite": false
                }),
            )
            .await
            .expect("write");

        assert_eq!(result["bytes_written"], 4);
        assert_eq!(
            std::fs::read_to_string(temp.path().join("out.txt")).expect("read"),
            "data"
        );
    }

    #[tokio::test]
    async fn edit_tool_replaces_text() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("edit.txt");
        std::fs::write(&path, "hello world").expect("write");
        let ctx = context_for_root(temp.path());
        let tool = EditTool;

        let result = tool
            .call(
                &ctx,
                json!({
                    "path": "edit.txt",
                    "old_text": "world",
                    "new_text": "odyssey",
                    "replace_all": false
                }),
            )
            .await
            .expect("edit");

        assert_eq!(result["replaced"], 1);
        assert_eq!(
            std::fs::read_to_string(path).expect("read"),
            "hello odyssey"
        );
    }

    #[tokio::test]
    async fn glob_tool_finds_matches() {
        let temp = tempdir().expect("tempdir");
        std::fs::write(temp.path().join("a.txt"), "a").expect("write");
        std::fs::write(temp.path().join("b.md"), "b").expect("write");
        let ctx = context_for_root(temp.path());
        let tool = GlobTool;

        let result = tool
            .call(
                &ctx,
                json!({
                    "pattern": "*.txt",
                    "root": null
                }),
            )
            .await
            .expect("glob");

        let matches = result["matches"].as_array().expect("array");
        assert_eq!(matches.len(), 1);
    }

    #[tokio::test]
    async fn grep_tool_finds_lines() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("grep.txt");
        std::fs::write(&path, "alpha\nneedle\nbeta").expect("write");
        let ctx = context_for_root(temp.path());
        let tool = GrepTool;

        let result = tool
            .call(
                &ctx,
                json!({
                    "pattern": "needle",
                    "path": "grep.txt",
                    "case_sensitive": true
                }),
            )
            .await
            .expect("grep");

        let matches = result["matches"].as_array().expect("array");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["line"], 2);
    }

    #[tokio::test]
    async fn read_tool_rejects_directory() {
        let temp = tempdir().expect("tempdir");
        let dir = temp.path().join("dir");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let ctx = context_for_root(temp.path());
        let tool = ReadTool;

        let err = tool
            .call(&ctx, json!({ "path": "dir" }))
            .await
            .expect_err("directory");
        let ToolError::ExecutionFailed(message) = err else {
            panic!("expected execution failed");
        };
        assert_eq!(message, "path is a directory");
    }

    #[tokio::test]
    async fn write_tool_rejects_existing_without_overwrite() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("exists.txt");
        std::fs::write(&path, "data").expect("write");
        let ctx = context_for_root(temp.path());
        let tool = WriteTool;

        let err = tool
            .call(
                &ctx,
                json!({
                    "path": "exists.txt",
                    "content": "new",
                    "overwrite": false
                }),
            )
            .await
            .expect_err("exists");
        let ToolError::ExecutionFailed(message) = err else {
            panic!("expected execution failed");
        };
        assert_eq!(message, "file exists; set overwrite to true to replace");
    }

    #[tokio::test]
    async fn edit_tool_rejects_missing_text() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("edit.txt");
        std::fs::write(&path, "hello").expect("write");
        let ctx = context_for_root(temp.path());
        let tool = EditTool;

        let err = tool
            .call(
                &ctx,
                json!({
                    "path": "edit.txt",
                    "old_text": "missing",
                    "new_text": "ok",
                    "replace_all": false
                }),
            )
            .await
            .expect_err("missing");
        let ToolError::ExecutionFailed(message) = err else {
            panic!("expected execution failed");
        };
        assert_eq!(message, "no matches found for old_text");
    }

    #[tokio::test]
    async fn glob_tool_rejects_non_directory_root() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("file.txt");
        std::fs::write(&path, "data").expect("write");
        let ctx = context_for_root(temp.path());
        let tool = GlobTool;

        let err = tool
            .call(
                &ctx,
                json!({
                    "pattern": "*.txt",
                    "root": "file.txt"
                }),
            )
            .await
            .expect_err("root");
        let ToolError::ExecutionFailed(message) = err else {
            panic!("expected execution failed");
        };
        assert_eq!(message, "root is not a directory");
    }
}
