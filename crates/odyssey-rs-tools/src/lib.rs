//! Tooling interfaces and built-in tools for Odyssey.

pub mod adaptor;
pub mod builtins;
pub mod context;
pub mod events;
pub mod output_policy;
pub mod permissions;
pub mod question;
pub mod registry;
pub mod tool;
pub mod web;

/// Tool adaptor helpers.
pub use adaptor::{ToolAdaptor, tool_to_adaptor, tools_to_adaptors};
/// Built-in tool registry and registration helper.
pub use builtins::{builtin_tool_registry, register_builtin_tools};
/// Tool context and result handling types.
pub use context::{ToolContext, ToolResultHandler, ToolSandbox, TurnServices};
/// Event sink for streaming events (re-exported from protocol).
pub use events::EventSink;
/// Tool output policy.
pub use output_policy::ToolOutputPolicy;
/// Permission checking interfaces for tool execution.
pub use permissions::{PermissionChecker, PermissionContext, PermissionOutcome};
/// Question prompt types for interactive tools.
pub use question::{Question, QuestionAnswer, QuestionHandler, QuestionOption};
/// Tool registry type.
pub use registry::ToolRegistry;
/// Tool trait and spec type.
pub use tool::{Tool, ToolSpec};
/// Web provider types.
pub use web::{WebFetchResult, WebProvider, WebSearchResult};
