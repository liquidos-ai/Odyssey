# Odyssey

<div align="center">
  <img src="assets/logo.png" alt="Odyssey logo" width="180" height="180">

  <p><strong>Bundle-first agent runtime and SDK in Rust</strong></p>
  <p>Author bundles locally, build OCI-style artifacts, run them through one runtime, and ship the same system through the CLI, HTTP server, and TUI.</p>

  [![License](https://img.shields.io/github/license/liquidos-ai/odyssey)](https://github.com/liquidos-ai/odyssey/blob/main/APACHE_LICENSE)
  [![CI](https://github.com/liquidos-ai/odyssey/actions/workflows/ci-chek.yml/badge.svg)](https://github.com/liquidos-ai/odyssey/actions/workflows/ci-chek.yml)
  [![Coverage](https://github.com/liquidos-ai/odyssey/actions/workflows/coverage.yml/badge.svg)](https://github.com/liquidos-ai/odyssey/actions/workflows/coverage.yml)
  [![Codecov](https://codecov.io/gh/liquidos-ai/odyssey/graph/badge.svg)](https://codecov.io/gh/liquidos-ai/odyssey)
  [![Docs](https://github.com/liquidos-ai/odyssey/actions/workflows/docs.yml/badge.svg)](https://github.com/liquidos-ai/odyssey/actions/workflows/docs.yml)
  [![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/liquidos-ai/Odyssey)

  [Documentation](https://liquidos-ai.github.io/odyssey/) | [Bundles](bundles/) | [Contributing](CONTRIBUTING.md)
</div>

---

> **Status:** Odyssey is under active development and should be treated as pre-production software.

Odyssey is a Rust-based agent runtime built around a bundle-first workflow. Instead of wiring agent behavior directly into each application surface, Odyssey packages an agent as a bundle defined by `odyssey.bundle.json5`, `agent.yaml`, optional skills, and resources. That bundle can then be built, installed, exported, imported, published, pulled, and executed through the same runtime engine.

The current architecture centers on:

- a manifest and bundle pipeline for packaging agents
- a runtime engine for sessions, execution, approvals, and event streaming
- built-in tools and sandbox enforcement
- multiple operator surfaces on top of the same runtime: CLI, HTTP server, and TUI

Odyssey currently uses prebuilt executors and memory providers, with AutoAgents-backed execution in the runtime layer.

## Why Odyssey

- **Bundle-first delivery:** agent projects are portable artifacts rather than ad hoc app-local configs.
- **Single runtime model:** the SDK, CLI, HTTP server, and TUI all operate on the same runtime engine.
- **Security-oriented execution:** sandbox mode, filesystem mounts, network allowlists, and per-tool approval rules are part of the bundle contract.
- **Operationally simple:** local installs, OCI-style blob storage, `.odybundle` export/import, and hub push/pull workflows are built in.
- **Rust-native:** small crates, explicit types, and embeddable runtime components.

## Repository Layout

- `crates/odyssey-rs`: CLI entrypoint and SDK facade
- `crates/odyssey-rs-manifest`: bundle manifest parsing and validation
- `crates/odyssey-rs-bundle`: bundle build, install, inspect, export, import, publish, and pull
- `crates/odyssey-rs-protocol`: shared runtime, session, event, approval, and sandbox protocol types
- `crates/odyssey-rs-runtime`: runtime engine, prompt assembly, session store, sandbox bridge, skill loading, and execution
- `crates/odyssey-rs-tools`: built-in tool registry and tool adaptors
- `crates/odyssey-rs-sandbox`: sandbox runtime and providers
- `crates/odyssey-rs-server`: Axum-based HTTP API and SSE session streaming
- `crates/odyssey-rs-tui`: Ratatui-based local operator interface
- `bundles/hello-world`: minimal example agent
- `bundles/odyssey-agent`: first-party general-purpose agent

## Architecture

### 1. Authoring

An Odyssey bundle is a directory containing:

- `odyssey.bundle.json5`: bundle manifest, runtime policy, tool rules, resources, and server flags
- `agent.yaml`: agent identity, prompt, model, and allow/deny tool lists
- `skills/`: optional reusable prompt extensions
- `resources/`: optional bundle-local assets

### 2. Packaging

Bundles are built into OCI-style layouts. You can:

- install them into the local cache
- export them into a portable `.odybundle` archive
- import them back into a local install
- publish or pull them through a hub-compatible registry flow

### 3. Execution

At runtime, Odyssey creates sessions, assembles prompts, loads skills, resolves tools, stages a bundle workspace, and executes the agent through the configured executor.

### 4. Interfaces

The same runtime engine is exposed through:

- the `odyssey-rs` CLI
- the embedded TUI in `odyssey-rs-tui`
- the HTTP server in `odyssey-rs-server`

## Quickstart

### Initialize a new bundle project

```bash
cargo run -p odyssey-rs -- init ./hello-world
```

This creates:

- `odyssey.bundle.json5`
- `agent.yaml`
- `README.md`
- `skills/`
- `resources/`

### Build and install the bundle locally

```bash
cargo run -p odyssey-rs -- build ./hello-world
```

### Build to an output directory instead of the local cache

```bash
cargo run -p odyssey-rs -- build ./hello-world --output ./dist
```

### Run a bundle

```bash
export OPENAI_API_KEY="your-key"
cargo run -p odyssey-rs -- run hello-world@latest --prompt "Summarize this bundle"
```

### Inspect installed bundle metadata

```bash
cargo run -p odyssey-rs -- inspect hello-world@latest
```

## Bundle Distribution

Export a portable archive:

```bash
cargo run -p odyssey-rs -- export local/hello-world:0.1.0 --output ./dist
```

Import a portable archive:

```bash
cargo run -p odyssey-rs -- import ./dist/hello-world-0.1.0.odybundle
```

Publish to a hub:

```bash
cargo run -p odyssey-rs -- publish ./hello-world --to team/hello-world:0.1.0 --hub http://127.0.0.1:8473
```

Pull from a hub:

```bash
cargo run -p odyssey-rs -- pull team/hello-world:0.1.0 --hub http://127.0.0.1:8473
```

## Running the System

### CLI

The top-level CLI supports:

- `init`
- `build`
- `inspect`
- `run`
- `serve`
- `publish`
- `pull`
- `export`
- `import`

### HTTP server

Start the HTTP API:

```bash
cargo run -p odyssey-rs -- serve --bind 127.0.0.1:8472
```

The server exposes bundle lifecycle endpoints, session management, asynchronous run submission, approval resolution, and session event streaming over SSE.

### TUI

Run the terminal UI:

```bash
cargo run -p odyssey-rs-tui --
```

Install the first-party default bundle if you want the TUI to resolve `odyssey@latest` immediately:

```bash
cargo run -p odyssey-rs -- build bundles/odyssey
```

Run the TUI against a specific installed bundle:

```bash
cargo run -p odyssey-rs-tui -- --bundle hello-world@latest
```

If `--bundle` is omitted, the TUI tries `odyssey@latest` first and otherwise falls back to the first installed bundle it can resolve.

Useful TUI commands:

- `/bundle install .`
- `/bundle use odyssey@latest`
- `/agents`
- `/agent odyssey`
- `/sessions`

## Approvals and Sandbox Model

Odyssey treats execution policy as part of the runtime, not an afterthought.

- Bundles declare a sandbox mode such as `read_only` or `workspace_write`.
- Tool policies can `allow`, `deny`, or `ask`.
- Filesystem host access is controlled through `sandbox.permissions.filesystem.mounts`.
- Outbound network access for sandboxed commands is controlled by `sandbox.permissions.network`.
- Approval requests suspend the active turn and resume the same turn after resolution.

The runtime emits approval events, the TUI can resolve them locally, and the HTTP server exposes `POST /approvals/{id}` for remote clients.

For local development and debugging, the CLI and server also support `--dangerous-sandbox-mode`, which overrides bundle sandboxing with host execution. That mode should be used sparingly.

## Development

### Prerequisites

- Rust toolchain
- `rg`
- `tokei`

### Quality gates

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
tokei -t Rust --exclude tests
```

If you change shared runtime surfaces such as protocol, runtime, or manifest handling, run the broader test suite expected by the repository guidelines.

## License

Odyssey is licensed under Apache 2.0. See `APACHE_LICENSE`.
