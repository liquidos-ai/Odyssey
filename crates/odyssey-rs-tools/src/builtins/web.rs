//! Built-in tools for web search and fetch.

use crate::builtins::utils::parse_args;
use crate::{Tool, ToolContext};
use async_trait::async_trait;
use autoagents_core::tool::ToolInputT;
use autoagents_derive::ToolInput;
use log::info;
use odyssey_rs_protocol::ToolError;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Default search result limit.
const DEFAULT_SEARCH_LIMIT: usize = 5;
/// Default max bytes for fetch output.
const DEFAULT_MAX_FETCH_BYTES: usize = 50_000;

/// Tool for web search queries.
#[derive(Debug, Default)]
pub struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "WebSearch"
    }

    fn description(&self) -> &str {
        "Search the web for current information"
    }

    fn args_schema(&self) -> Value {
        let params_str = WebSearchArgs::io_schema();
        serde_json::from_str(params_str).expect("Error parsing tool parameters")
    }

    fn supports_parallel(&self) -> bool {
        true
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let input: WebSearchArgs = parse_args(args)?;
        if input.query.trim().is_empty() {
            return Err(ToolError::InvalidArguments(
                "query cannot be empty".to_string(),
            ));
        }
        let provider =
            ctx.services.web.as_ref().ok_or_else(|| {
                ToolError::ExecutionFailed("web provider not configured".to_string())
            })?;
        let limit = input.limit.unwrap_or(DEFAULT_SEARCH_LIMIT);
        info!(
            "web search (query_len={}, limit={})",
            input.query.len(),
            limit
        );
        let results = provider.search(&input.query, limit).await?;
        Ok(json!({
            "query": input.query,
            "results": results,
        }))
    }
}

/// Tool for fetching web page content.
#[derive(Debug, Default)]
pub struct WebFetchTool;

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "WebFetch"
    }

    fn description(&self) -> &str {
        "Fetch and return web page content"
    }

    fn args_schema(&self) -> Value {
        let params_str = WebFetchArgs::io_schema();
        serde_json::from_str(params_str).expect("Error parsing tool parameters")
    }

    fn supports_parallel(&self) -> bool {
        true
    }

    async fn call(&self, ctx: &ToolContext, args: Value) -> Result<Value, ToolError> {
        let input: WebFetchArgs = parse_args(args)?;
        if input.url.trim().is_empty() {
            return Err(ToolError::InvalidArguments(
                "url cannot be empty".to_string(),
            ));
        }
        let provider =
            ctx.services.web.as_ref().ok_or_else(|| {
                ToolError::ExecutionFailed("web provider not configured".to_string())
            })?;
        let max_bytes = input.max_bytes.unwrap_or_else(|| {
            ctx.services
                .output_policy
                .as_ref()
                .map(|policy| policy.max_string_bytes)
                .unwrap_or(DEFAULT_MAX_FETCH_BYTES)
        });
        info!(
            "web fetch (url_len={}, max_bytes={})",
            input.url.len(),
            max_bytes
        );
        let result = provider.fetch(&input.url, max_bytes).await?;
        Ok(json!({
            "url": result.url,
            "status": result.status,
            "content_type": result.content_type,
            "body": result.body,
            "truncated": result.truncated,
        }))
    }
}

/// Arguments for WebSearchTool.
#[derive(Debug, Serialize, Deserialize, ToolInput)]
struct WebSearchArgs {
    #[input(description = "Search query to execute.")]
    query: String,
    #[input(description = "Maximum number of results to return.")]
    #[serde(default)]
    limit: Option<usize>,
}

/// Arguments for WebFetchTool.
#[derive(Debug, Serialize, Deserialize, ToolInput)]
struct WebFetchArgs {
    #[input(description = "URL to fetch.")]
    url: String,
    #[input(description = "Maximum bytes to return from the response.")]
    #[serde(default)]
    max_bytes: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::{WebFetchTool, WebSearchTool};
    use crate::{
        Tool, ToolContext, ToolOutputPolicy, TurnServices, WebFetchResult, WebProvider,
        WebSearchResult,
    };
    use async_trait::async_trait;
    use odyssey_rs_protocol::ToolError;
    use parking_lot::Mutex;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::sync::Arc;
    use tempfile::tempdir;
    use uuid::Uuid;

    #[derive(Default)]
    struct DummyWebProvider {
        last_search: Mutex<Option<(String, usize)>>,
        last_fetch: Mutex<Option<(String, usize)>>,
    }

    #[async_trait]
    impl WebProvider for DummyWebProvider {
        async fn search(
            &self,
            query: &str,
            limit: usize,
        ) -> Result<Vec<WebSearchResult>, ToolError> {
            *self.last_search.lock() = Some((query.to_string(), limit));
            Ok(vec![WebSearchResult {
                title: "result".to_string(),
                url: "https://example.com".to_string(),
                snippet: "snippet".to_string(),
            }])
        }

        async fn fetch(&self, url: &str, max_bytes: usize) -> Result<WebFetchResult, ToolError> {
            *self.last_fetch.lock() = Some((url.to_string(), max_bytes));
            Ok(WebFetchResult {
                url: url.to_string(),
                status: Some(200),
                content_type: Some("text/plain".to_string()),
                body: "ok".to_string(),
                truncated: false,
            })
        }
    }

    fn base_context(root: &std::path::Path) -> ToolContext {
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
    async fn web_search_rejects_empty_query() {
        let temp = tempdir().expect("tempdir");
        let ctx = base_context(temp.path());
        let tool = WebSearchTool;
        let err = tool
            .call(&ctx, json!({ "query": " " }))
            .await
            .expect_err("empty query");
        let ToolError::InvalidArguments(message) = err else {
            panic!("expected invalid arguments");
        };
        assert_eq!(message, "query cannot be empty");
    }

    #[tokio::test]
    async fn web_search_errors_without_provider() {
        let temp = tempdir().expect("tempdir");
        let ctx = base_context(temp.path());
        let tool = WebSearchTool;
        let err = tool
            .call(&ctx, json!({ "query": "odyssey" }))
            .await
            .expect_err("missing provider");
        let ToolError::ExecutionFailed(message) = err else {
            panic!("expected execution failed");
        };
        assert_eq!(message, "web provider not configured");
    }

    #[tokio::test]
    async fn web_search_uses_provider_defaults() {
        let temp = tempdir().expect("tempdir");
        let provider = Arc::new(DummyWebProvider::default());
        let ctx = ToolContext {
            services: Arc::new(TurnServices {
                cwd: temp.path().to_path_buf(),
                workspace_root: temp.path().to_path_buf(),
                output_policy: None,
                sandbox: None,
                web: Some(provider.clone()),
                event_sink: None,
                skill_provider: None,
                question_handler: None,
                permission_checker: None,
                tool_result_handler: None,
            }),
            ..base_context(temp.path())
        };
        let tool = WebSearchTool;
        let result = tool
            .call(&ctx, json!({ "query": "odyssey" }))
            .await
            .expect("search");

        assert_eq!(result["results"].as_array().unwrap().len(), 1);
        let (query, limit) = provider.last_search.lock().clone().expect("search");
        assert_eq!(query, "odyssey".to_string());
        assert_eq!(limit, 5);
    }

    #[tokio::test]
    async fn web_fetch_rejects_empty_url() {
        let temp = tempdir().expect("tempdir");
        let ctx = base_context(temp.path());
        let tool = WebFetchTool;
        let err = tool
            .call(&ctx, json!({ "url": "" }))
            .await
            .expect_err("empty url");
        let ToolError::InvalidArguments(message) = err else {
            panic!("expected invalid arguments");
        };
        assert_eq!(message, "url cannot be empty");
    }

    #[tokio::test]
    async fn web_fetch_uses_output_policy_limit() {
        let temp = tempdir().expect("tempdir");
        let provider = Arc::new(DummyWebProvider::default());
        let ctx = ToolContext {
            services: Arc::new(TurnServices {
                cwd: temp.path().to_path_buf(),
                workspace_root: temp.path().to_path_buf(),
                output_policy: Some(ToolOutputPolicy {
                    max_string_bytes: 12,
                    max_array_len: 8,
                    max_object_entries: 8,
                    redact_keys: Vec::new(),
                    redact_values: Vec::new(),
                    replacement: "[X]".to_string(),
                }),
                sandbox: None,
                web: Some(provider.clone()),
                event_sink: None,
                skill_provider: None,
                question_handler: None,
                permission_checker: None,
                tool_result_handler: None,
            }),
            ..base_context(temp.path())
        };
        let tool = WebFetchTool;
        let result = tool
            .call(&ctx, json!({ "url": "https://example.com" }))
            .await
            .expect("fetch");

        assert_eq!(result["status"], 200);
        let (url, limit) = provider.last_fetch.lock().clone().expect("fetch");
        assert_eq!(url, "https://example.com".to_string());
        assert_eq!(limit, 12);
    }
}
