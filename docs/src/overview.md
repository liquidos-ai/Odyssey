# Odyssey

Odyssey is a bundle-first agent runtime written in Rust. The workspace is split into a few clear layers:

- `odyssey-rs-manifest` parses and validates bundle manifests and agent specs.
- `odyssey-rs-bundle` builds, installs, exports, imports, publishes, and pulls bundles.
- `odyssey-rs-runtime` owns sessions, sandbox preparation, tool routing, approvals, prompt assembly, and execution.
- `odyssey-rs-tools` provides the builtin tool set.
- `odyssey-rs-sandbox` provides host and Linux bubblewrap sandbox backends.
- `odyssey-rs-server`, `odyssey-rs`, and `odyssey-rs-tui` expose HTTP, CLI, and terminal UI surfaces over the same runtime.

What the current codebase supports:

- local bundle authoring and installation
- portable `.odyssey` export and import
- namespaced bundle publish and pull through the hub client
- persistent local sessions backed by JSON files
- async execution through a small in-process scheduler
- sandboxed tool execution with per-tool approval prompts
- CLI, HTTP, and TUI operator surfaces

Current hard limits worth knowing up front:

- only prebuilt executors are accepted by manifest validation, and `react` is the only executor wired into the runtime
- only prebuilt memory providers are accepted, and `sliding_window` is the only implemented provider
- only builtin tools are accepted by manifest validation
- cloud LLM providers are implemented;

Road Map:

- Support custom WASM Tools
- Support custom Executors
- Support custom MemoryProviders
- Support Custom Agent Hook
- Suppot Local Models

