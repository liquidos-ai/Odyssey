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
  orchestrator: {
    // NOTE: Accepted by schema but not wired yet (see "Current gaps" below).
    system_prompt: "You are the Odyssey Orchestrator.",
    append_system_prompt: "Keep replies concise.",
    subagent_window_size: 20
  },
  agents: {
    setting_sources: ["project", "user"],
    paths: ["./.odyssey/agents"],
    list: [
      {
        id: "writer",
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
  sandbox: {
    enabled: false,
    provider: null,
    mode: "workspace_write", // read_only | workspace_write | danger_full_access
    filesystem: {
      allow_read: [],
      deny_read: [],
      allow_write: [],
      deny_write: [],
      allow_exec: [],
      deny_exec: []
    },
    network: {
      allow_domains: [],
      deny_domains: []
    },
    env: {
      allow: ["PATH"],
      deny: [],
      set: {}
    },
    limits: {
      cpu_seconds: null,
      memory_bytes: null,
      nofile: null,
      pids: null
    }
  },
  sessions: {
    enabled: false,
    path: ".odyssey/sessions"
  }
}
```

## Current gaps
- `orchestrator.system_prompt` and `orchestrator.append_system_prompt` are validated by the
  loader but are not currently consumed by the runtime.
- `orchestrator.additional_instruction_prompt` exists in the Rust config type but is not
  accepted by the JSON5 schema yet, so it cannot be set in config files.
- `agents` is validated by the loader but is not yet wired to automatic agent registration.
  Agents must be registered programmatically.

## References
See `odyssey.json5` for the example template used in this repo.
