# Odyssey

<div align="center">
  <img src="assets/logo.png" alt="Odyssey logo" width="180" height="180">

  <p><strong>Define, build, and run agents as portable bundles.</strong></p>
  <p>Open-source Rust runtime for packaging, securing, and operating portable AI agents.</p>

  [![License](https://img.shields.io/github/license/liquidos-ai/odyssey)](https://github.com/liquidos-ai/odyssey/blob/main/APACHE_LICENSE)
  [![CI](https://github.com/liquidos-ai/odyssey/actions/workflows/ci-chek.yml/badge.svg)](https://github.com/liquidos-ai/odyssey/actions/workflows/ci-chek.yml)
  [![Coverage](https://github.com/liquidos-ai/odyssey/actions/workflows/coverage.yml/badge.svg)](https://github.com/liquidos-ai/odyssey/actions/workflows/coverage.yml)
  [![Codecov](https://codecov.io/gh/liquidos-ai/odyssey/graph/badge.svg)](https://codecov.io/gh/liquidos-ai/odyssey)
  [![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/liquidos-ai/Odyssey)

  [Documentation](https://liquidos-ai.github.io/odyssey/) | [Docs Index](docs/README.md) | [Architecture](docs/architecture-runtime.md) | [Contributing](CONTRIBUTING.md)
</div>

---

> **Status:** Odyssey is under active development and should be treated as pre-production software.

Odyssey is an open-source, bundle-first agent runtime written in Rust on top of [AutoAgents](https://github.com/liquidos-ai/AutoAgents) - Agent Framework in Rust. It lets you define an agent once, package it as a portable artifact, and run it through the same execution model across local development, embedded SDK usage, shared runtime servers, and terminal workflows.

The project is built around a simple idea:

- author agents as portable bundles
- run them with built-in sandboxing and approvals
- ship with prebuilt tools, executors, and memory providers
- expose the same runtime through CLI, HTTP, and TUI surfaces
- grow toward custom executors, memory providers, tools, and WASM-based extensions

Odyssey currently uses prebuilt executors and prebuilt memory providers in v1. Custom extensions and WASM support are planned.

## Why Odyssey

- **Bundle-first delivery:** agents are packaged artifacts, not scattered app-local configuration.
- **One runtime, many surfaces:** the SDK, CLI, HTTP server, and TUI all sit on the same runtime contract.
- **Security by default:** sandbox mode, tool approvals, filesystem mounts, and network rules are part of the bundle definition.
- **Operationally practical:** local install, export/import, and hub push/pull workflows are built in.
- **Rust-native core:** explicit types, embeddable crates, and a runtime that can be used directly in applications.

## What You Get Today

- Embeddable runtime via `OdysseyRuntime`
- Bundle manifests and validation
- Local bundle build and install flow
- Portable `.odyssey` export/import
- Built-in tools and policy-controlled approvals
- Sandboxed execution with filesystem and network controls
- CLI for authoring and operations
- HTTP server for remote runtime access
- TUI for local operator workflows

## Roadmap

- Hub push/pull distribution flow
- Custom executors
- Custom memory providers
- Native macOS and Windows sandbox providers
- Custom tools
- WASM-based extension support
- Stronger pluggability around runtime components

## Core Concepts

### Bundles

An Odyssey bundle is the unit of portability. A typical bundle contains:

- `odyssey.bundle.json5` for runtime policy, tools, sandbox, executor, and memory configuration
- `agent.yaml` for agent identity, prompt, model, and tool policy
- `skills/` for reusable prompt extensions
- `resources/` for bundle-local assets

### Runtime

At execution time, Odyssey resolves an `AgentRef` to an installed bundle, creates a session, prepares an isolated workspace, loads skills and tools, applies sandbox policy, attaches memory, and executes the turn through the configured runtime pipeline.

### Interfaces

The same runtime is available through:

- `odyssey-rs` for CLI workflows
- `odyssey-rs-server` for HTTP access
- `odyssey-rs-tui` for terminal-native operation

## Quickstart

### 1. Initialize a bundle

```bash
cargo run -p odyssey-rs -- init ./hello-world
```

This creates a starter project with:

- `odyssey.bundle.json5`
- `agent.yaml`
- `README.md`
- `skills/`
- `resources/`

### 2. Build and install locally

```bash
cargo run -p odyssey-rs -- build ./hello-world
```

Build to a custom output directory instead:

```bash
cargo run -p odyssey-rs -- build ./hello-world --output ./dist
```

### 3. Run an agent

```bash
export OPENAI_API_KEY="your-key"
cargo run -p odyssey-rs -- run hello-world@latest --prompt "Hey, What are your capabilities?"
```
Run the agent in the TUI - The TUI Automatically loads the installed bundles, TUI Gives ability to run tools with "ASK" policy

```bash
export OPENAI_API_KEY="your-key"
cargo run --release -p odyssey-rs-tui
```

### 4. Inspect installed metadata

```bash
cargo run -p odyssey-rs -- inspect hello-world@latest
```

### 5. Start the runtime server

```bash
cargo run -p odyssey-rs -- serve --bind 127.0.0.1:8472
```

You can then target that runtime remotely:

```bash
cargo run -p odyssey-rs -- --remote http://127.0.0.1:8472 bundles
cargo run -p odyssey-rs -- --remote http://127.0.0.1:8472 inspect hello-world@latest
cargo run -p odyssey-rs -- --remote http://127.0.0.1:8472 run hello-world@latest --prompt "Summarize this bundle"
cargo run -p odyssey-rs -- --remote http://127.0.0.1:8472 sessions
```

## Docker on macOS

Odyssey's confined sandbox backend is Linux-only today. If you are developing on macOS and want to run the current system with the Linux `bubblewrap` sandbox instead of host fallback mode, use the included Docker image.

Build the image:

```bash
docker compose build odyssey
```

Start an interactive shell in the Linux container:

```bash
docker compose run --rm odyssey
```

Inside the container, run the same commands you would run natively:

```bash
export OPENAI_API_KEY="your-key"
cargo run -p odyssey-rs --release -- build ./bundles/hello-world
cargo run -p odyssey-rs --release -- run hello-world@latest --prompt "What can you do?"
```

The Compose setup mounts this repository at `/workspace`. Odyssey creates and uses `/home/odyssey/.odyssey` inside the container itself, so it stays isolated from the host filesystem.

To expose the runtime server back to the host, publish ports and bind to `0.0.0.0`:

```bash
docker compose run --rm --service-ports odyssey \
  cargo run -p odyssey-rs --release -- serve --bind 0.0.0.0:8472
```

Then connect from the host with:

```bash
cargo run -p odyssey-rs -- --remote http://127.0.0.1:8472 bundles
```

If you only need host execution and do not need the Linux sandbox backend, native macOS runs are still possible with `--dangerous-sandbox-mode`.

## Bundle Distribution

Export a portable archive:

```bash
cargo run -p odyssey-rs -- export local/hello-world:0.1.0 --output ./dist
```

Import a portable archive:

```bash
cargo run -p odyssey-rs -- import ./dist/hello-world-0.1.0.odyssey
```

## Security Model

Odyssey treats execution policy as part of the runtime contract.

- Bundles declare a sandbox mode such as `read_only` or `workspace_write`.
- Tool actions can be set to `allow`, `deny`, or `ask`.
- Filesystem access is controlled through explicit host mounts.
- Outbound network access is controlled through sandbox policy.
- Approval flows suspend the active turn and resume it after resolution.

For local debugging, the CLI and server support `--dangerous-sandbox-mode`, which bypasses sandbox restrictions and runs with host access. Use it sparingly.

## Repository Layout

- `crates/odyssey-rs`: CLI entrypoint and facade crate
- `crates/odyssey-rs-manifest`: bundle manifest parsing and validation
- `crates/odyssey-rs-bundle`: build, install, inspect, export, import, push, and pull
- `crates/odyssey-rs-protocol`: shared runtime protocol types
- `crates/odyssey-rs-runtime`: core runtime, sessions, execution, tools, memory, and sandbox integration
- `crates/odyssey-rs-tools`: built-in tools and tool adaptors
- `crates/odyssey-rs-sandbox`: sandbox runtime and providers
- `crates/odyssey-rs-server`: Axum-based HTTP API
- `crates/odyssey-rs-tui`: Ratatui-based terminal UI
- `bundles/hello-world`: minimal example bundle
- `bundles/odyssey-agent`: Odyssey general purpose agent

## Development

### Prerequisites

- Rust toolchain
- `rg`
- `tokei`
- Docker Desktop or another recent Docker engine when running the Linux sandbox workflow on macOS

### Quality Gates

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --all-features
```

### Coverage

```bash
cargo tarpaulin --engine llvm --skip-clean --workspace --all-features --timeout 120 --out Xml --ignore-panics
```

## Documentation

- [Docs index](docs/README.md)
- [Runtime architecture](docs/architecture-runtime.md)
- [Hello world bundle](bundles/hello-world/README.md)
- [Odyssey agent bundle](bundles/odyssey-agent/README.md)

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for development workflow, quality expectations, and pull request guidance.

## License

Odyssey is licensed under Apache 2.0. See [APACHE_LICENSE](APACHE_LICENSE).
