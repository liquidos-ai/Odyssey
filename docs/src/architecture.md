# Odyssey SDK Architecture 

This document reflects the architecture as implemented in the Rust crates.

## Scope and non-goals
- Scope: the core orchestration SDK (`odyssey-rs-core`) and supporting crates.
- Server: `odyssey-rs-server` is currently a stub.

## Core components
- **Orchestrator (odyssey-rs-core)**  
  Owns config, tool routing, permission engine, skill store, sessions, and execution.
- **ToolRouter (odyssey-rs-core)**  
  Filters tools by allow/deny and adapts them for AutoAgents.
- **PermissionEngine (odyssey-rs-core)**  
  Enforces rules, hooks, and modes; emits approval events.
- **PromptBuilder (odyssey-rs-core)**  
  Builds system prompts using bootstrap files, memory recall, and skill summaries.
- **MemoryProvider (odyssey-rs-memory)**  
  Captures, recalls, and compacts session memory.
- **SandboxProvider (odyssey-rs-sandbox)**  
  Enforces filesystem policy and runs commands (local or bubblewrap).
- **SkillStore (odyssey-rs-core)**  
  Discovers and loads `SKILL.md` files from configured roots.
- **StateStore (odyssey-rs-core)**  
  JSONL persistence for sessions (JsonlStateStore).

## Configuration flow (JSON5 + programmatic)
1. Discover layers: requirements → system → user → project → CWD → repo → runtime.
2. Validate each layer schema.
3. Merge layers with requirements acting as constraints.
4. Deserialize into `OdysseyConfig`.

Notes:
- `agents` and `orchestrator.system_prompt` are validated but not wired yet.
- Agents must be registered programmatically.

## System prompt assembly
`PromptBuilder` uses:
- `memory.instruction_roots` (defaults to `cwd` if empty).
- Bootstrap files: `AGENTS.md`, `SOUL.md`, `USER.md`, `TOOLS.md`, `IDENTITY.md`.
- Memory recall (initial records) and skill summaries.

The final prompt includes:
- Header (time, runtime, workspace, memory/skills paths)
- Bootstrap file sections
- Memory section
- Skills section
- Footer notes

## Agent registration flow
1. Create `Orchestrator`.
2. Register LLM providers with `register_llm_provider(LLMEntry)`.
   Use `list_llm_ids()` to enumerate registered LLM provider ids.
3. Register agents with `register_agent(AgentBuilder)`.
4. Optionally set default agent id.

The default agent id constant is `odyssey-orchestrator`. The default LLM id used by the
registry is `default_LLM`.

## Session lifecycle
- `create_session(agent_id?)` creates a session and records it in state store (if enabled).
- `resume_session(session_id)` loads session state.
- `list_sessions()` lists sessions from state store or cache.
- `delete_session(session_id)` deletes persisted rollouts when enabled.

## Run flow (Orchestrator::run)
1. Resolve agent and session.
2. Build system prompt with `PromptBuilder`.
3. Build tool context and tool specs for the agent.
4. Execute the AutoAgents runtime.
5. Capture outputs to memory and sessions.
6. Return `RunResult`.

## Streaming flow (Orchestrator::run_stream)
1. Resolve agent/session and start turn executor.
2. Emit events through the run event bus.
3. Caller consumes the event stream and calls `finish()` for the final result.

## Tool call flow
1. Agent emits tool call.
2. ToolRouter verifies allow/deny.
3. PermissionEngine evaluates rules and mode.
4. Tool executes with sandbox + output policy.
5. Tool result is emitted as events and returned to the model.

## Skills discovery and invocation
1. SkillStore scans roots from `skills.setting_sources` and `skills.paths`.
2. Each `SKILL.md` is parsed for frontmatter or heading.
3. Skill summaries are inserted into the system prompt.
4. The Skill tool loads the full content when requested.

## Sandbox execution
1. SandboxProvider prepares a policy-backed handle.
2. Commands run with path checks and output streaming.
3. Local provider runs on host with policy enforcement.
