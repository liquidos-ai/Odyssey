# Executable Skills Design Context

Knowledge skills remain the only supported skill type today. They are prompt artifacts discovered from `SKILL.md` files and loaded through the `Skill` tool.

## Why executable skills are deferred

Secure data isolation needs a runtime-owned execution boundary. Prompt-only skills do not provide a reliable lifecycle boundary for process tracking, private state, or cleanup.

## Planned model

Executable skills should eventually become packaged runtime units with:

- `SKILL.md` for model-facing guidance
- a machine-readable manifest for runtime policy and entrypoints
- a dedicated reusable sandbox cell per skill identity
- execution sessions with explicit terminal states
- artifact-based data exchange instead of direct shared filesystem access

## Current MCP decision

MCP servers are the first component promoted to isolated reusable cells because they already have an explicit client/server lifecycle. Each configured MCP connection now gets:

- a startup-owned sandbox cell
- a private data/cache area
- a connection-scoped working layout
- a long-lived stdio transport created inside the sandbox backend
- Linux filesystem confinement enforced again with Landlock so MCP servers cannot cross-read other cells

This design is the reference point for the future executable-skill runtime.
