//! Web provider interfaces for tools.

use async_trait::async_trait;
use odyssey_rs_protocol::ToolError;
use serde::{Deserialize, Serialize};

/// Search result returned by a web provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchResult {
    /// Result title.
    pub title: String,
    /// Result URL.
    pub url: String,
    /// Result snippet.
    pub snippet: String,
}

/// Fetch result returned by a web provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebFetchResult {
    /// Fetch URL.
    pub url: String,
    /// Optional HTTP status code.
    pub status: Option<u16>,
    /// Optional content type.
    pub content_type: Option<String>,
    /// Response body (possibly truncated).
    pub body: String,
    /// Whether the body was truncated.
    pub truncated: bool,
}

/// Web provider interface for search and fetch operations.
#[async_trait]
pub trait WebProvider: Send + Sync {
    /// Perform a web search query.
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<WebSearchResult>, ToolError>;
    /// Fetch a URL with a maximum byte limit.
    async fn fetch(&self, url: &str, max_bytes: usize) -> Result<WebFetchResult, ToolError>;
}
