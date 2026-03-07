# Crate Boundaries and Dependency Rules

This document captures the current crate layout.

## Dependency rules
- `odyssey-rs-protocol` is the lowest layer.
- `odyssey-rs-config` may depend on protocol for shared enums and schemas.
- `odyssey-rs-core` depends on config + protocol + runtime provider crates.
- `odyssey-rs` depends on core + config + protocol.

## Crate responsibilities
- `crates/odyssey-rs-protocol`
  - Event schema, tool call contracts, permission request/decision types, sandbox modes.
- `crates/odyssey-rs-config`
  - JSON5 schema, layered loader, validation, and programmatic builder.
- `crates/odyssey-rs-core`
  - AgentRuntime builder/runtime, permissions, sessions, managed/custom agent registration, prompt ownership, and folded memory store/provider.
- `crates/odyssey-rs-tools`
  - Tool traits, registry, built-in tools, MCP bridge, and output policy.
- `crates/odyssey-rs-sandbox`
  - Sandbox providers and policy enforcement.
- `crates/odyssey-rs`
  - User-facing SDK surface and re-exports.
- `crates/odyssey-rs-tui`
  - Terminal UI client embedding the orchestrator.
- `crates/odyssey-rs-test-utils`
  - Shared test-only helpers.

## Module placement guide
- Orchestration flow, permissions, sessions, and runtime memory live in `odyssey-rs-core`.
- Tool implementations live in `odyssey-rs-tools`.
- Sandbox execution lives in `odyssey-rs-sandbox`.
