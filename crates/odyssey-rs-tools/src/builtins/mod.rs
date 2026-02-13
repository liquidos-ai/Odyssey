//! Built-in tools bundled with Odyssey.

mod bash;
mod filesystem;
mod question;
mod skill;
// mod task;
mod utils;
mod web;

use crate::ToolRegistry;
use log::info;
use std::sync::Arc;

pub use bash::BashTool;
pub use filesystem::{EditTool, GlobTool, GrepTool, ReadTool, WriteTool};
pub use question::AskUserQuestionTool;
pub use skill::SkillTool;
pub use web::{WebFetchTool, WebSearchTool};

/// Register all built-in tools with the provided registry.
pub fn register_builtin_tools(registry: &ToolRegistry) {
    registry.register(Arc::new(ReadTool));
    registry.register(Arc::new(WriteTool));
    registry.register(Arc::new(EditTool));
    registry.register(Arc::new(BashTool {}));
    registry.register(Arc::new(GlobTool));
    registry.register(Arc::new(GrepTool));
    registry.register(Arc::new(WebSearchTool));
    registry.register(Arc::new(WebFetchTool));
    registry.register(Arc::new(AskUserQuestionTool));
    registry.register(Arc::new(SkillTool));
    // registry.register(Arc::new(TaskTool));
    info!("registered built-in tools");
}

/// Build a registry pre-populated with built-in tools.
pub fn builtin_tool_registry() -> ToolRegistry {
    let registry = ToolRegistry::new();
    register_builtin_tools(&registry);
    registry
}
