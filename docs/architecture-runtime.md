# Runtime Architecture

## Summary

Odyssey is structured as a **library-first agent runtime** with multiple operator surfaces on top of it.

The core idea is:

- **OCI / Docker owns environment packaging**
- **Odyssey owns agent execution**
- **Agents are execution units, not services**
- **The SDK embeds the runtime directly**
- **The CLI is a thin wrapper over the same runtime**
- **A single runtime process can execute many agent bundles concurrently**

This architecture keeps bundles portable, keeps execution policy centralized, and allows the same runtime contract to scale from a single in-process SDK call to a local daemon and later to distributed workers.

## Current Shape

The current repository is organized around:

- `odyssey-rs-runtime` contains the shared execution engine
- `odyssey-rs` exposes the CLI over that runtime
- `odyssey-rs-server` wraps the same runtime with HTTP
- `odyssey-rs-bundle` handles build, install, export, import, publish, and pull
- `odyssey-rs-sandbox` handles isolation

The main runtime surface now exposes:

- `OdysseyRuntime`
- `AgentRef`
- `SessionSpec`
- `ExecutionRequest`
- `ExecutionHandle`
- `ExecutionStatus`

The runtime still supports compatibility methods for bundle lifecycle operations, but the intended split is already in place:

- bundle lifecycle is owned by `odyssey-rs-bundle`
- execution is owned by `odyssey-rs-runtime`
- server and TUI run on top of the same runtime contract

## Target Architecture

Odyssey is split conceptually into four layers.

### 1. Artifact Layer

This layer owns portability only.

Responsibilities:

- bundle authoring format
- manifest validation
- OCI layout generation
- `.odyssey` export and import
- publish and pull from registries
- local artifact cache

This is the responsibility of `odyssey-rs-manifest` and `odyssey-rs-bundle`.

An agent bundle remains the portability unit, but it is **not** the runtime unit.
The runtime consumes a resolved bundle artifact and turns it into an execution request.

### 2. Runtime Core

This layer is the source of truth for Odyssey execution semantics.

Responsibilities:

- session and turn lifecycle
- prompt assembly
- model provider resolution
- tool routing
- skill loading
- memory integration
- policy enforcement
- approval flow
- event emission
- tracing and logging

This is centered in `odyssey-rs-runtime`, with execution as the primary responsibility and bundle operations composed from `odyssey-rs-bundle`.

The main public surface is a single embeddable runtime API:

```rust
pub struct OdysseyRuntime { /* shared state */ }
```

The SDK, CLI, local daemon, and future distributed workers should all use this same runtime type or a thin client around the same protocol.

### 3. Execution Engine

This layer sits inside the runtime core and owns concurrency.

Responsibilities:

- queue execution requests
- assign work to workers
- manage backpressure
- apply concurrency limits
- retry or fail execution cleanly
- support cancellation
- reuse shared resources across runs

The execution unit is an **agent run**, not a process, container, or long-lived service.

The current worker pool runs in-process with Tokio tasks behind an explicit scheduler. It can expand to stronger isolation or remote workers later without changing the runtime contract.

### 4. Environment / Sandbox Layer

This layer provides the operating environment in which execution happens.

Responsibilities:

- OCI image or prepared runtime filesystem
- sandbox provider selection
- filesystem isolation
- network policy
- CPU / memory limits
- command execution isolation

This is the responsibility of container tooling plus `odyssey-rs-sandbox`.

Odyssey should never require one container per agent.
Instead, the runtime process runs inside a prepared environment and schedules many agent executions within it.

## Core Runtime Model

The runtime exposes a stable execution contract independent of transport.

### Execution Request

The public execution request is:

```rust
pub struct ExecutionRequest {
    pub session_id: uuid::Uuid,
    pub input: String,
    pub turn_context: Option<TurnContextOverride>,
}
```

Agent resolution happens at session creation through `SessionSpec { agent_ref, ... }`. The runtime resolves that `AgentRef` to an installed bundle internally before execution begins.

### Execution Handle

Async submissions return a handle:

```rust
pub struct ExecutionHandle {
    pub session_id: uuid::Uuid,
    pub turn_id: uuid::Uuid,
}
```

The handle supports:

- status lookup
- event subscription
- cancellation
- final result retrieval

### Execution Context

Each run gets an isolated execution context built from shared runtime state:

- resolved bundle payload
- working directory
- sandbox lease
- permission rules
- tool registry view
- model selection
- memory scope

This context is per-run.
The runtime itself keeps process-wide shared state.

## Shared Runtime Resources

To scale, Odyssey must separate **shared resources** from **execution-local state**.

Shared runtime resources include:

- installed bundle cache
- OCI layer cache
- tool registry
- model clients
- memory backends
- approval manager
- event bus
- sandbox runtime handles
- telemetry exporters
- scheduler queues

Execution-local state includes:

- session id
- turn id
- prompt input
- sandboxed workspace
- transient tool call state
- stream buffers

This separation is what allows one runtime process to execute many bundles efficiently.

## Execution Modes

The same runtime contract supports three modes.

### Embedded Mode

Used by:

- Rust SDK consumers
- CLI
- local tests
- local developer workflows

Behavior:

- application constructs `OdysseyRuntime` in-process
- bundle is resolved locally
- execution request is submitted directly to the runtime
- events stream in-process

This is the default mode and the reference implementation.

### Local Runtime Mode

Used by:

- long-lived developer daemons
- local multi-user services
- applications that want shared worker pools and shared caches

Behavior:

- a single runtime process hosts queues, workers, bundle caches, and sandbox backends
- the CLI or SDK can submit execution requests over a transport layer
- the execution model is identical to embedded mode

`odyssey-rs-server` is a transport adapter over runtime APIs, not a separate runtime implementation.

### Distributed Mode

Used later for:

- horizontally scaled execution
- centralized operations
- shared fleet scheduling

Behavior:

- control plane accepts execution requests
- workers pull or mount bundle artifacts
- workers execute through the same runtime core
- event and approval protocol remains the same

Distributed mode should add transport and scheduling distribution, not new execution semantics.

## Bundle Model

Bundles remain the portable unit of authoring and distribution:

- `agent.yaml`
- prompts
- skills
- tools
- policies
- resources
- metadata

The runtime resolves the bundle into an internal execution definition before the scheduler sees it.

That gives Odyssey two important properties:

- the packaging format can evolve without rewriting the execution engine
- the execution engine can later support more than one agent profile or execution entrypoint without turning the bundle itself into a service boundary

## Scheduler And Worker Design

The runtime has moved from "run this bundle now" toward "submit this execution unit to the engine."

Current structure:

- `Scheduler`
  - accepts `ExecutionRequest`
  - enforces concurrency limits
  - assigns work to workers
  - handles retry and cancellation

- `WorkerPool`
  - owns worker capacity
  - executes requests using shared runtime state
  - supports in-process, subprocess, or stronger sandbox-backed workers

- `ExecutionCoordinator`
  - resolves bundle and agent definition
  - acquires sandbox and resource leases
  - builds tool context, memory, and prompt
  - runs executor and streams events

This keeps the execution engine explicit and makes scaling behavior visible.

## OCI And Sandbox Responsibilities

Odyssey keeps this split strict.

### OCI / Docker

Owns:

- OS dependencies
- language runtimes
- system packages
- reproducible environment distribution

Does not own:

- agent scheduling
- tool policy
- runtime approvals
- session state
- event streaming

### Odyssey Runtime

Owns:

- agent execution loop
- tool calls
- memory
- permissions
- sandbox policy
- approvals
- tracing
- result persistence

This is the critical separation that makes Odyssey scalable.
Containers package environments; the runtime schedules and executes agents.

## Crate Direction

The repository responsibilities are:

### `odyssey-rs-runtime`

Responsibilities:

- embeddable runtime API
- execution engine
- scheduler / worker pool
- session and turn management
- event streaming
- approval flow
- execution context assembly

### `odyssey-rs-bundle`

Responsibilities:

- artifact build
- install and inspect
- export and import
- publish and pull
- local cache layout

### `odyssey-rs-sandbox`

Responsibilities:

- sandbox providers
- lease management
- policy enforcement primitives
- isolated command execution backends

### `odyssey-rs-server`

Responsibilities:

- HTTP transport
- authentication and authorization when needed
- request / response mapping
- SSE or stream transport

It should not become the place where runtime logic forks from embedded execution.

### `odyssey-rs`

Responsibilities:

- CLI parsing
- embedded runtime startup
- output formatting

The CLI should stay thin and delegate real work to the runtime library.

## Commands And Surfaces

### CLI

The CLI is intentionally thin:

- `init` scaffolds a new bundle project
- `build` builds and optionally installs a bundle
- `bundles` lists installed bundles
- `inspect` reads bundle metadata from the bundle store
- `run` creates a session and executes one turn through `OdysseyRuntime`
- `serve` starts the HTTP server over the same runtime
- `sessions` lists session summaries
- `session <id>` fetches or deletes a specific session
- `publish`, `pull`, `export`, and `import` operate through `BundleStore`

The CLI also supports `--remote <url>` as an alternate transport. In remote mode it calls the runtime server instead of embedding the runtime locally, while keeping the same command vocabulary for common operational tasks.

### HTTP Server

The HTTP server exposes the same execution model remotely:

- `GET /bundles`
  - lists installed bundle summaries
- `POST /sessions`
  - creates a session from `agent_ref`
- `GET /sessions`
  - lists session summaries
- `POST /sessions/{id}/run`
  - submits an `ExecutionRequest`
- `POST /sessions/{id}/run-sync`
  - executes synchronously and returns `RunOutput`
- `GET /sessions/{id}/events`
  - streams runtime events
- `POST /approvals/{id}`
  - resolves approval requests

Bundle lifecycle routes remain separate from runtime execution routes.

### TUI

The TUI still lets users browse and switch bundle references, but its client now:

- creates sessions with `SessionSpec`
- filters sessions by `agent_ref`
- submits turns with `ExecutionRequest`
- subscribes to runtime session events

## Acceptance Criteria

This architecture is working as intended when:

- the SDK embeds the same runtime used by the CLI
- the CLI contains no execution-specific business logic
- the server exposes the same runtime contract rather than reimplementing it
- a single runtime process can execute many bundles concurrently
- bundle packaging stays independent from runtime scheduling
- agents remain execution units, not service boundaries
- OCI remains the environment and distribution layer, not the execution orchestrator

## Final Position

Odyssey should be understood as a **shared execution runtime for portable agent bundles**.

The bundle is what gets packaged and distributed.
The runtime is what gets embedded, operated, and scaled.
The agent is what gets executed.

That separation is what enables:

- embeddable SDK usage
- a thin CLI
- a long-lived local runtime
- future distributed workers
- secure execution with shared resources and high agent density
