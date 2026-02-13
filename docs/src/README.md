# Odyssey Documentation

> **Active Development:** This project is still in development, and not ready for production use yet.

Odyssey is a Rust-based agentic orchestrator and SDK that provides a core runtime, built-in tools, memory, sandboxing, and an integrated TUI for end-to-end agent workflows. With Odyssey, you can build desktop applications, robotics, and embedded systems with powerful agentic capabilities. It is built on top of the open-source AutoAgents framework.

### What's in this repo
- `crates/odyssey-rs-core`: Orchestrator runtime, permissions, sessions, prompt assembly.
- `crates/odyssey-rs-tools`: Tool registry and built-in tools.
- `crates/odyssey-rs-memory`: File-backed memory provider and policies.
- `crates/odyssey-rs-sandbox`: Sandbox policies and providers.
- `crates/odyssey-rs-protocol`: Event, request, and schema types.
- `crates/odyssey-rs`: SDK re-exports and helpers.
- `crates/odyssey-rs-tui`: Terminal UI client.
- `docs/src`: mdBook documentation source.

### Quickstart (SDK)
Set your OpenAI API key, then run the SDK example (or use your own binary):

```bash
export OPENAI_API_KEY="your-key"
cargo run -p odyssey-rs-hello-world
```

For a custom integration, see `quickstart.md`.

### Run the TUI
```bash
export OPENAI_API_KEY="your-key"
cargo run -p odyssey-rs-tui
```

Optional flags:
```bash
cargo run -p odyssey-rs-tui -- --config ./docs/src/odyssey.json5 --model gpt-4.1-mini
```

### Developement Setup

#### Prerequisites

- **Rust** (latest stable recommended)
- **Cargo** package manager
- **LeftHook** for Git hooks management
- **tokei** For lines of code

#### Install LeftHook

**macOS (using Homebrew):**

```bash
brew install lefthook
```

**Linux/Windows:**

```bash
# Using npm
npm install -g lefthook
```

#### Running Tests

```bash
# Run all tests --
cargo test --all-features

# Run tests with coverage (requires cargo-tarpaulin)
cargo install cargo-tarpaulin
cargo tarpaulin --engine llvm --skip-clean \
  --workspace \
  --exclude odyssey-rs-server \
  --exclude odyssey-rs-tui \
  --all-features \
  --out html
```

### Docs (mdBook)
```bash
mdbook serve docs
```

### License
Apache-2.0
