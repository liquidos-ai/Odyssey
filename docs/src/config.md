# Configuration

Odyssey supports layered JSON5 configuration. This document reflects the current loader
and runtime behavior.

## File discovery and layering
Layer precedence (low → high):
1. Requirements constraints (`/etc/odyssey/requirements.json5` on Unix)
2. System config (`/etc/odyssey/odyssey.json5` on Unix)
3. User config (`~/.odyssey/odyssey.json5`)
4. Project root (`odyssey.json5`, project root is detected by `.git`)
5. CWD (`odyssey.json5`)
6. Repo config (`.odyssey/odyssey.json5` in the project root)
7. Runtime overrides (explicit paths)

Layers are validated before merge, then merged with requirements acting as constraints
that prevent later overrides for constrained keys.

## Top-level schema (JSON5)
```json5
{
  agents: {
    list: [
      {
        id: "odyssey-orchestrator",
        description: "Summarizes files.",
        prompt: "Focus on file summaries.",
        model: { provider: "openai", name: "gpt-4.1-mini" },
        tools: { allow: ["Read", "Write"], deny: ["Bash"] },
        memory: null,
        sandbox: null,
        permissions: null
      }
    ]
  },
  tools: {
    output_policy: {
      max_string_bytes: 32000,
      max_array_len: 256,
      max_object_entries: 256,
      redact_keys: ["api_key", "token"],
      redact_values: ["sk-"],
      replacement: "[REDACTED]"
    }
  },
  permissions: {
    mode: "default", // default | accept_edits | bypass_permissions | plan
    rules: [
      { action: "deny", tool: "Bash" },
      { action: "ask", tool: "Write" },
      { action: "allow", path: "src/**", access: "write" }
    ]
  },
  memory: {
    provider: "file",
    path: ".odyssey/memory",
    recall_k: 6,
    instruction_roots: ["."],
    capture: {
      capture_messages: true,
      capture_tool_output: false,
      deny_patterns: [],
      redact_patterns: [],
      max_message_chars: 4000,
      max_tool_output_chars: 20000,
      detect_secrets: true,
      secret_entropy_threshold: 3.7
    },
    recall: {
      mode: "text", // text | vector | hybrid
      text_weight: 0.3,
      vector_weight: 0.7,
      min_score: null
    },
    compaction: {
      enabled: false,
      max_messages: 40,
      summary_max_chars: 1500,
      max_total_chars: null
    }
  },
  skills: {
    setting_sources: ["user", "project"],
    paths: [],
    allow: ["*"],
    deny: []
  },
  mcp: {
    enabled: false,
    servers: [
      {
        name: "filesystem",
        protocol: "stdio",
        command: "node",
        args: ["./mcp/filesystem-server.js"],
        cwd: ".",
        description: "Sandboxed filesystem MCP server",
        env: {},
        sandbox: {
          filesystem: {
            read: ["./mcp"],
            write: [],
            exec: [],
          },
          network: {
            mode: "disabled",
          },
          env: {
            inherit: ["PATH"],
            set: {},
          },
          limits: {
            wall_clock_seconds: 300,
            stdout_bytes: 65536,
            stderr_bytes: 65536,
          },
        },
      },
    ],
  },

  sandbox: {
    enabled: false,
    provider: null,
    mode: "workspace_write", // read_only | workspace_write | danger_full_access
    filesystem: {
      read: [],
      write: [],
      exec: []
    },
    network: {
      mode: "disabled" // disabled | allow_all
    },
    env: {
      inherit: ["PATH"],
      set: {}
    },
    limits: {
      cpu_seconds: null,
      memory_bytes: null,
      nofile: null,
      pids: null,
      wall_clock_seconds: 60,
      stdout_bytes: 65536,
      stderr_bytes: 65536
    }
  },
  sessions: {
    enabled: false,
    path: ".odyssey/sessions"
  }
}
```

## MCP notes
- MCP servers are connected during orchestrator startup and each server gets its own managed
  sandbox cell. Odyssey keeps the cell alive for the MCP client lifetime and creates a dedicated
  execution layout inside that cell for the stdio transport.
- MCP requires an isolated sandbox backend. On Linux, Odyssey uses `bubblewrap` for namespace
  isolation and an internal Landlock helper binary inside the MCP process so each server can only
  access its private cell plus explicitly allow-listed filesystem roots. `host`/`local` backends
  are rejected when `mcp.enabled = true`.
- Any non-system paths needed by an MCP server must be allow-listed under
  `mcp.servers[].sandbox.filesystem`.
- Knowledge skills stay prompt-only for now. The future executable-skill design is tracked in
  `docs/executable-skills/README.md`.

## Current gaps
- Agent prompting and default-agent behavior are now configured via `agents.list`.
- The legacy `orchestrator` config block has been removed from schema and runtime loading.

## References
See `odyssey.json5` for the example template used in this repo.
