use crate::{Tool, ToolContext, ToolError};
use async_trait::async_trait;
use ignore::WalkBuilder;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct ReadTool;
#[derive(Debug)]
pub struct WriteTool;
#[derive(Debug)]
pub struct EditTool;
#[derive(Debug)]
pub struct LsTool;
#[derive(Debug)]
pub struct GlobTool;
#[derive(Debug)]
pub struct GrepTool;

#[derive(Debug, Deserialize)]
struct ReadArgs {
    path: String,
}

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "Read"
    }
    fn description(&self) -> &str {
        "Read a text file"
    }
    fn args_schema(&self) -> Value {
        json!({"type":"object","required":["path"],"properties":{"path":{"type":"string"}}})
    }
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        ctx.authorize_tool(self.name()).await?;
        let input: ReadArgs = serde_json::from_value(args)
            .map_err(|err| ToolError::InvalidArguments(err.to_string()))?;
        let path = resolve_visible_file_for_read(ctx, &input.path)?;
        let content =
            fs::read_to_string(&path).map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        Ok(json!({"path": input.path, "content": content}))
    }
}

#[derive(Debug, Deserialize)]
struct WriteArgs {
    path: String,
    content: String,
}

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "Write"
    }
    fn description(&self) -> &str {
        "Write a text file"
    }
    fn args_schema(&self) -> Value {
        json!({"type":"object","required":["path","content"],"properties":{"path":{"type":"string"},"content":{"type":"string"}}})
    }
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        ctx.authorize_tool(self.name()).await?;
        let input: WriteArgs = serde_json::from_value(args)
            .map_err(|err| ToolError::InvalidArguments(err.to_string()))?;
        let workspace_path = ctx.resolve_workspace_path(&input.path)?;
        ctx.check_write(&workspace_path)?;
        let path = ctx.resolve_host_path(&workspace_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        }
        fs::write(&path, input.content.as_bytes())
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        Ok(json!({"path": input.path, "bytes": input.content.len()}))
    }
}

#[derive(Debug, Deserialize)]
struct EditArgs {
    path: String,
    old_text: String,
    new_text: String,
}

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "Edit"
    }
    fn description(&self) -> &str {
        "Replace text in a file"
    }
    fn args_schema(&self) -> Value {
        json!({"type":"object","required":["path","old_text","new_text"],"properties":{"path":{"type":"string"},"old_text":{"type":"string"},"new_text":{"type":"string"}}})
    }
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        ctx.authorize_tool(self.name()).await?;
        let input: EditArgs = serde_json::from_value(args)
            .map_err(|err| ToolError::InvalidArguments(err.to_string()))?;
        let workspace_path = ctx.resolve_workspace_path(&input.path)?;
        ctx.check_read(&workspace_path)?;
        ctx.check_write(&workspace_path)?;
        let path = ctx.resolve_host_path(&workspace_path);
        let content =
            fs::read_to_string(&path).map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        if !content.contains(&input.old_text) {
            return Err(ToolError::ExecutionFailed("old_text not found".to_string()));
        }
        let updated = content.replacen(&input.old_text, &input.new_text, 1);
        fs::write(&path, updated.as_bytes())
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        Ok(json!({"path": input.path, "edited": true}))
    }
}

#[derive(Debug, Deserialize)]
struct LsArgs {
    path: Option<String>,
}

#[async_trait]
impl Tool for LsTool {
    fn name(&self) -> &str {
        "LS"
    }
    fn description(&self) -> &str {
        "List visible files and directories"
    }
    fn args_schema(&self) -> Value {
        json!({"type":"object","properties":{"path":{"type":"string"}}})
    }
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        ctx.authorize_tool(self.name()).await?;
        let input: LsArgs = serde_json::from_value(args)
            .map_err(|err| ToolError::InvalidArguments(err.to_string()))?;
        let requested_path = input.path.unwrap_or_else(|| ".".to_string());
        let resolved = resolve_visible_path(ctx, &requested_path)?;
        ctx.check_read(&resolved.visible_path)?;
        let synthetic_mount_dir = is_mount_scaffold_path(ctx, &resolved.visible_path);
        if !resolved.host_path.is_dir() && !synthetic_mount_dir {
            return Err(ToolError::ExecutionFailed(format!(
                "path `{}` is not a directory",
                display_workspace_path(ctx, &resolved.visible_path)?
            )));
        }
        if !synthetic_mount_dir && !path_is_visible(ctx, &resolved)? {
            return Err(ToolError::PermissionDenied(format!(
                "path `{}` is ignored by .gitignore",
                display_workspace_path(ctx, &resolved.visible_path)?
            )));
        }

        let mut entries = Vec::new();
        let mut seen = BTreeSet::new();
        if resolved.host_path.is_dir() {
            for entry in fs::read_dir(&resolved.host_path)
                .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?
            {
                let entry = entry.map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
                let visible_path = resolved.visible_path.join(entry.file_name());
                ctx.check_read(&visible_path)?;
                let child = ResolvedVisiblePath {
                    visible_path: visible_path.clone(),
                    host_path: entry.path(),
                    visible_root: resolved.visible_root.clone(),
                    host_root: resolved.host_root.clone(),
                };
                if !path_is_visible(ctx, &child)? {
                    continue;
                }
                let entry_type = entry
                    .file_type()
                    .map(classify_entry)
                    .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
                entries.push(json!({
                    "name": entry.file_name().to_string_lossy(),
                    "path": display_workspace_path(ctx, &visible_path)?,
                    "type": entry_type,
                }));
                seen.insert(entry.file_name().to_string_lossy().to_string());
            }
        }
        for entry in synthetic_mount_entries(ctx, &resolved.visible_path)? {
            let name = entry["name"].as_str().unwrap_or_default().to_string();
            if seen.insert(name) {
                entries.push(entry);
            }
        }
        entries.sort_by(|left, right| {
            let left_type = left["type"].as_str().unwrap_or_default();
            let right_type = right["type"].as_str().unwrap_or_default();
            let left_name = left["name"].as_str().unwrap_or_default();
            let right_name = right["name"].as_str().unwrap_or_default();
            left_type.cmp(right_type).then(left_name.cmp(right_name))
        });
        Ok(json!({
            "path": display_workspace_path(ctx, &resolved.visible_path)?,
            "entries": entries,
        }))
    }
}

#[derive(Debug, Deserialize)]
struct GlobArgs {
    pattern: String,
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "Glob"
    }
    fn description(&self) -> &str {
        "Find files matching a glob-like pattern"
    }
    fn args_schema(&self) -> Value {
        json!({"type":"object","required":["pattern"],"properties":{"pattern":{"type":"string"}}})
    }
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        ctx.authorize_tool(self.name()).await?;
        let input: GlobArgs = serde_json::from_value(args)
            .map_err(|err| ToolError::InvalidArguments(err.to_string()))?;
        let regex = glob_to_regex(&input.pattern)?;
        let mut matches = Vec::new();
        visit_visible_files(ctx, |file| {
            let rel = workspace_relative_path(ctx, &file.visible_path)?;
            if regex.is_match(&rel) {
                matches.push(rel);
            }
            Ok(())
        })?;
        matches.sort();
        Ok(json!({"matches": matches}))
    }
}

#[derive(Debug, Deserialize)]
struct GrepArgs {
    pattern: String,
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "Grep"
    }
    fn description(&self) -> &str {
        "Search file contents with regex"
    }
    fn args_schema(&self) -> Value {
        json!({"type":"object","required":["pattern"],"properties":{"pattern":{"type":"string"}}})
    }
    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        ctx.authorize_tool(self.name()).await?;
        let input: GrepArgs = serde_json::from_value(args)
            .map_err(|err| ToolError::InvalidArguments(err.to_string()))?;
        let regex = Regex::new(&input.pattern)
            .map_err(|err| ToolError::InvalidArguments(err.to_string()))?;
        let mut matches = Vec::new();
        visit_visible_files(ctx, |file| {
            let content = match fs::read_to_string(&file.host_path) {
                Ok(content) => content,
                Err(_) => return Ok(()),
            };
            let rel = workspace_relative_path(ctx, &file.visible_path)?;
            for (line_no, line) in content.lines().enumerate() {
                if regex.is_match(line) {
                    matches.push(json!({"path": rel, "line": line_no + 1, "text": line}));
                }
            }
            Ok(())
        })?;
        matches.sort_by(|left, right| {
            let left_path = left["path"].as_str().unwrap_or_default();
            let right_path = right["path"].as_str().unwrap_or_default();
            let left_line = left["line"].as_u64().unwrap_or_default();
            let right_line = right["line"].as_u64().unwrap_or_default();
            left_path.cmp(right_path).then(left_line.cmp(&right_line))
        });
        Ok(json!({"matches": matches}))
    }
}

fn glob_to_regex(pattern: &str) -> Result<Regex, ToolError> {
    let mut regex = String::from("^");
    for ch in pattern.chars() {
        match ch {
            '*' => regex.push_str(".*"),
            '.' => regex.push_str("\\."),
            '/' => regex.push('/'),
            other => regex.push(other),
        }
    }
    regex.push('$');
    Regex::new(&regex).map_err(|err| ToolError::InvalidArguments(err.to_string()))
}

#[derive(Debug)]
struct VisibleFile {
    visible_path: PathBuf,
    host_path: PathBuf,
}

#[derive(Debug)]
struct ResolvedVisiblePath {
    visible_path: PathBuf,
    host_path: PathBuf,
    visible_root: PathBuf,
    host_root: PathBuf,
}

fn resolve_visible_file_for_read(
    ctx: &ToolContext,
    input_path: &str,
) -> Result<PathBuf, ToolError> {
    let resolved = resolve_visible_path(ctx, input_path)?;
    ctx.check_read(&resolved.visible_path)?;
    if !path_is_visible(ctx, &resolved)? {
        return Err(ToolError::PermissionDenied(format!(
            "path `{}` is ignored by .gitignore",
            display_workspace_path(ctx, &resolved.visible_path)?
        )));
    }
    Ok(resolved.host_path)
}

fn resolve_visible_path(
    ctx: &ToolContext,
    input_path: &str,
) -> Result<ResolvedVisiblePath, ToolError> {
    let visible_path = ctx.resolve_workspace_path(input_path)?;
    let mount = ctx
        .workspace_mounts
        .iter()
        .filter_map(|mount| {
            visible_path
                .strip_prefix(&mount.visible_root)
                .ok()
                .map(|suffix| (suffix.components().count(), mount, suffix.to_path_buf()))
        })
        .max_by_key(|(depth, _, _)| *depth);
    if let Some((_, mount, suffix)) = mount {
        return Ok(ResolvedVisiblePath {
            visible_path,
            host_path: mount.host_root.join(suffix),
            visible_root: mount.visible_root.clone(),
            host_root: mount.host_root.clone(),
        });
    }
    if visible_path.starts_with(&ctx.bundle_root) {
        return Ok(ResolvedVisiblePath {
            host_path: visible_path.clone(),
            visible_root: ctx.bundle_root.clone(),
            host_root: ctx.bundle_root.clone(),
            visible_path,
        });
    }
    Err(ToolError::PermissionDenied(format!(
        "path {} is outside the visible workspace",
        visible_path.display()
    )))
}

fn visit_visible_files<F>(ctx: &ToolContext, mut visit: F) -> Result<(), ToolError>
where
    F: FnMut(VisibleFile) -> Result<(), ToolError>,
{
    visit_visible_root_files(ctx, &ctx.bundle_root, &ctx.bundle_root, &mut visit)?;
    for mount in &ctx.workspace_mounts {
        visit_visible_root_files(ctx, &mount.visible_root, &mount.host_root, &mut visit)?;
    }
    Ok(())
}

fn visit_visible_root_files<F>(
    ctx: &ToolContext,
    visible_root: &Path,
    host_root: &Path,
    visit: &mut F,
) -> Result<(), ToolError>
where
    F: FnMut(VisibleFile) -> Result<(), ToolError>,
{
    let mut builder = visible_walk_builder(host_root);
    if host_root == ctx.bundle_root {
        let root = host_root.to_path_buf();
        let mounts_root = ctx.bundle_root.join("mount");
        builder.filter_entry(move |entry| {
            entry.path() == root || !entry.path().starts_with(&mounts_root)
        });
    }
    for entry in builder.build() {
        let entry = entry.map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        if !entry
            .file_type()
            .map(|file_type| file_type.is_file())
            .unwrap_or(false)
        {
            continue;
        }
        let suffix = entry
            .path()
            .strip_prefix(host_root)
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        let visible_path = visible_root.join(suffix);
        ctx.check_read(&visible_path)?;
        visit(VisibleFile {
            visible_path,
            host_path: entry.into_path(),
        })?;
    }
    Ok(())
}

fn path_is_visible(ctx: &ToolContext, resolved: &ResolvedVisiblePath) -> Result<bool, ToolError> {
    if resolved.host_path == resolved.host_root {
        return Ok(true);
    }
    if is_mount_scaffold_path(ctx, &resolved.visible_path) {
        return Ok(true);
    }

    let mut builder = visible_walk_builder(&resolved.host_root);
    if resolved.host_root == ctx.bundle_root {
        let root = resolved.host_root.clone();
        let mounts_root = ctx.bundle_root.join("mount");
        builder.filter_entry(move |entry| {
            entry.path() == root || !entry.path().starts_with(&mounts_root)
        });
    }

    for entry in builder.build() {
        let entry = entry.map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        if entry.path() != resolved.host_path {
            continue;
        }
        let suffix = entry
            .path()
            .strip_prefix(&resolved.host_root)
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
        let visible_path = resolved.visible_root.join(suffix);
        ctx.check_read(&visible_path)?;
        return Ok(true);
    }
    Ok(false)
}

fn is_mount_scaffold_path(ctx: &ToolContext, path: &Path) -> bool {
    path.starts_with(&ctx.bundle_root)
        && ctx
            .workspace_mounts
            .iter()
            .any(|mount| mount.visible_root.starts_with(path))
}

fn synthetic_mount_entries(ctx: &ToolContext, directory: &Path) -> Result<Vec<Value>, ToolError> {
    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();
    for mount in &ctx.workspace_mounts {
        let Ok(suffix) = mount.visible_root.strip_prefix(directory) else {
            continue;
        };
        let mut components = suffix.components();
        let Some(component) = components.next() else {
            continue;
        };
        let name = component.as_os_str().to_string_lossy().to_string();
        if name.is_empty() || !seen.insert(name.clone()) {
            continue;
        }
        let child_path = directory.join(&name);
        ctx.check_read(&child_path)?;
        entries.push(json!({
            "name": name,
            "path": display_workspace_path(ctx, &child_path)?,
            "type": "dir",
        }));
    }
    Ok(entries)
}

fn visible_walk_builder(root: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .ignore(false)
        .git_global(false)
        .git_ignore(true)
        .git_exclude(true)
        .parents(true)
        .require_git(false)
        .sort_by_file_path(|left, right| left.cmp(right));
    builder
}

fn classify_entry(file_type: fs::FileType) -> &'static str {
    if file_type.is_dir() {
        "dir"
    } else if file_type.is_file() {
        "file"
    } else if file_type.is_symlink() {
        "symlink"
    } else {
        "other"
    }
}

fn display_workspace_path(ctx: &ToolContext, path: &Path) -> Result<String, ToolError> {
    let relative = workspace_relative_path(ctx, path)?;
    if relative.is_empty() {
        Ok(".".to_string())
    } else {
        Ok(relative)
    }
}

fn workspace_relative_path(ctx: &ToolContext, path: &Path) -> Result<String, ToolError> {
    path.strip_prefix(&ctx.bundle_root)
        .map(|relative| relative.to_string_lossy().to_string())
        .map_err(|err| ToolError::ExecutionFailed(err.to_string()))
}
