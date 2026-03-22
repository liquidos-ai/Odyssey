mod bash;
mod filesystem;
mod skill;

use crate::ToolRegistry;
use std::sync::Arc;

pub use bash::BashTool;
pub use filesystem::{EditTool, GlobTool, GrepTool, LsTool, ReadTool, WriteTool};
pub use skill::SkillTool;

pub fn builtin_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(ReadTool));
    registry.register(Arc::new(WriteTool));
    registry.register(Arc::new(EditTool));
    registry.register(Arc::new(LsTool));
    registry.register(Arc::new(GlobTool));
    registry.register(Arc::new(GrepTool));
    registry.register(Arc::new(BashTool));
    registry.register(Arc::new(SkillTool));
    registry
}
