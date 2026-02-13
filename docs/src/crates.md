# Crate Boundaries and Dependency Rules

This document captures the current crate layout.

## Dependency rules
- `odyssey-rs-protocol` is the lowest layer.
  - No dependency on config or core.
- `odyssey-rs-config` may depend on protocol for shared enums and schemas.
  - No dependency on core.
- `odyssey-rs-core` depends on config + protocol and provider crates.
  - Never depends on `odyssey-rs` (SDK wrapper).
- `odyssey-rs` depends on core + config + protocol.

## Crate responsibilities
- `crates/odyssey-rs-protocol`
  - Event schema, tool call contracts, permission request/decision types, sandbox modes.
- `crates/odyssey-rs-config`
  - JSON5 schema, layered loader, validation, programmatic builder.
- `crates/odyssey-rs-core`
  - Orchestrator, permissions, sessions, tool router, prompt builder, skill store.
- `crates/odyssey-rs`
  - User-facing SDK surface and re-exports.
- `crates/odyssey-rs-tools`
  - Tool traits, registry, built-in tools, output policy.
- `crates/odyssey-rs-sandbox`
  - Sandbox providers and policy enforcement.
- `crates/odyssey-rs-memory`
  - Memory provider interface and file-backed implementation.
- `crates/odyssey-rs-tui`
  - Terminal UI client embedding the orchestrator.
- `crates/odyssey-rs-test-utils`
  - Shared test-only helpers (dummy agents, LLMs, tools, memory/skill stubs).

## Module placement guide
- Orchestration flow, permissions, and sessions live in `odyssey-rs-core`.
- Tool implementations live in `odyssey-rs-tools`.
- Sandbox execution lives in `odyssey-rs-sandbox`.
- Memory capture and recall live in `odyssey-rs-memory`.
