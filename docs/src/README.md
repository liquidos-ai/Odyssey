# Odyssey Documentation

> **Active Development:** the architecture is actively evolving toward a production-ready secure runtime.

Odyssey is a Rust-based secure agent runtime and SDK with sandboxed tool execution, explicit permission policy, built-in tools, an integrated TUI, and support for both config-managed and Rust-defined agents.

### What's in this repo
- `crates/odyssey-rs-core`: AgentRuntime builder/runtime, permissions, sessions, folded memory, prompt ownership.
- `crates/odyssey-rs-tools`: Tool registry and built-in tools.
- `crates/odyssey-rs-sandbox`: Sandbox policies and providers.
- `crates/odyssey-rs-protocol`: Event, request, and schema types.
- `crates/odyssey-rs`: SDK re-exports and helpers.
- `crates/odyssey-rs-tui`: Terminal UI client.
- `docs/src`: mdBook documentation source.

### Quickstart (SDK)
```bash
export OPENAI_API_KEY="your-key"
cargo run -p odyssey-rs-hello-world
```

### Run the TUI
```bash
export OPENAI_API_KEY="your-key"
cargo run -p odyssey-rs-tui
```
