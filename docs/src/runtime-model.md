# Runtime Model

## Runtime Entry Point

`odyssey-rs-runtime` exposes `OdysseyRuntime` as the main embeddable API. The public surface is intentionally small:

- bundle helpers such as `init`, `build_and_install`, `build_to`, `inspect_bundle`, `export_bundle`, and `import_bundle`
- session helpers such as `create_session`, `list_sessions`, `get_session`, and `delete_session`
- execution helpers such as `run`, `submit`, `execution_status`, `subscribe_session`, and `resolve_approval`

## Default Runtime Configuration

`RuntimeConfig::default()` uses the following local directories:

- `~/.odyssey/bundles`
- `~/.odyssey/sessions`
- `~/.odyssey/sandbox`

It also defaults to:

- bind address `127.0.0.1:8472`
- hub URL `http://127.0.0.1:8473`
- `worker_count = 4`
- `queue_capacity = 128`

## Session Lifecycle

A session is created from a `SessionSpec` that points at an `AgentRef`. During creation, the runtime resolves the bundle, picks the session model, allocates a UUID, and persists a JSON record to disk.

Session records currently store:

- the bundle reference used to create the session
- the resolved agent id
- the chosen model provider, model name, and optional model config
- all completed turns

Turns are appended after execution completes. The runtime stores either a simple prompt/response pair or a normalized chat history that includes tool use and tool result records.

## Execution Flow

For each submitted `ExecutionRequest`, the runtime currently does the following:

1. Load the session record.
2. Resolve the bundle and agent from the bundle store.
3. Pick the effective sandbox mode from the runtime override, turn override, or manifest default.
4. Stage the installed bundle into a sandbox cell.
5. Load bundle skills and append their summary section to the base system prompt when skills exist.
6. Select builtin tools based on manifest entries plus agent allow/deny filters.
7. Resolve the active LLM provider from the chosen `ModelSpec`.
8. Build memory from prior turns.
9. Run the `react` executor.
10. Persist the completed turn back into the session store.

`run` waits for completion and returns `RunOutput`. `submit` only enqueues the request and returns an `ExecutionHandle`.

## Events

Session subscribers receive `EventMsg` values over a broadcast channel. The event payloads include:

- turn lifecycle events
- streamed assistant deltas
- streamed reasoning deltas
- tool call start, delta, and finish events
- streamed command output for tool-driven process execution
- permission requests and approval resolutions
- plan updates
- runtime errors

The HTTP server exposes the same session event stream over server-sent events.

## Approvals

Tool approvals are driven by bundle sandbox tool rules:

- `allow` lets the tool run immediately
- `deny` fails the tool call
- `ask` emits `PermissionRequested` and waits for `resolve_approval`

`ApprovalDecision::AllowAlways` is remembered for the rest of the current session only. It is not persisted across runtime restarts.

## Current Provider Support

Cloud model providers are wired up for:

- Anthropic
- Azure OpenAI
- DeepSeek
- Google or Gemini
- Groq
- MiniMax
- OpenAI
- OpenRouter
- Phind
- xAI
