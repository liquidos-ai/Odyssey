# Runtime Architecture

## Current Execution Stack

1. A bundle is resolved from the local bundle store.
2. A session is created or loaded from the session store.
3. The scheduler enqueues the request and assigns it to a Tokio worker slot.
4. The runtime stages the bundle into a sandbox cell and builds a `ToolContext`.
5. The runtime selects builtin tools, loads bundle skills, resolves the model provider, and preloads memory from prior turns.
6. The `react` executor runs the turn and emits streamed events.
7. The completed turn is normalized into session history and persisted to disk.

## Shared State Vs Per-Run State

`OdysseyRuntime` keeps a few process-wide resources alive:

- `BundleStore`
- `SessionStore`
- one host `SandboxRuntime`
- the builtin `ToolRegistry`
- the `ApprovalStore`
- the execution scheduler

Each run then builds its own execution-local state:

- the resolved bundle and agent
- the staged sandbox cell
- the working directory
- the selected tool set
- the model provider instance
- the memory provider for that session's history
- the turn context and event sender

That split is what lets the runtime execute multiple turns concurrently without rebuilding global state each time.

## Scheduler

The scheduler is an in-process queue backed by:

- a Tokio `mpsc` channel for pending jobs
- a semaphore for concurrency control
- an in-memory map of `turn_id -> ExecutionStatus`

`submit` enqueues work and returns immediately. `run` submits the same work but also waits on a completion channel for the final `RunOutput`.

There is no distributed worker system in the current repository. Concurrency is local to one process.

## Session Persistence

Sessions are persisted as JSON files under the configured session root. Each file contains:

- session metadata
- model selection
- every completed turn
- normalized tool use and tool result history when tool calls happened

This means history survives runtime restarts.

## Transport Surfaces

Three entry points sit on top of the same runtime contract:

- the CLI constructs `OdysseyRuntime` directly for local commands
- the HTTP server constructs `OdysseyRuntime` once and exposes bundle and session routes
- the TUI constructs `OdysseyRuntime` directly and subscribes to its session event stream

The server is not a separate execution engine. It is a thin network wrapper over the same runtime type.
