use crate::{Tool, ToolContext, ToolError};
use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};
use std::fs;
use walkdir::WalkDir;

#[derive(Debug)]
pub struct ReadTool;
#[derive(Debug)]
pub struct WriteTool;
#[derive(Debug)]
pub struct EditTool;
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
        let path = ctx.resolve_workspace_path(&input.path)?;
        ctx.check_read(&path)?;
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
        let path = ctx.resolve_workspace_path(&input.path)?;
        ctx.check_write(&path)?;
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
        let path = ctx.resolve_workspace_path(&input.path)?;
        ctx.check_read(&path)?;
        ctx.check_write(&path)?;
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
        for entry in WalkDir::new(&ctx.bundle_root)
            .into_iter()
            .filter_map(Result::ok)
        {
            if entry.file_type().is_file() {
                let rel = entry
                    .path()
                    .strip_prefix(&ctx.bundle_root)
                    .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?
                    .to_string_lossy()
                    .to_string();
                if regex.is_match(&rel) {
                    matches.push(rel);
                }
            }
        }
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
        for entry in WalkDir::new(&ctx.bundle_root)
            .into_iter()
            .filter_map(Result::ok)
        {
            if entry.file_type().is_file() {
                let content = match fs::read_to_string(entry.path()) {
                    Ok(content) => content,
                    Err(_) => continue,
                };
                for (line_no, line) in content.lines().enumerate() {
                    if regex.is_match(line) {
                        let rel = entry
                            .path()
                            .strip_prefix(&ctx.bundle_root)
                            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?
                            .to_string_lossy()
                            .to_string();
                        matches.push(json!({"path": rel, "line": line_no + 1, "text": line}));
                    }
                }
            }
        }
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
