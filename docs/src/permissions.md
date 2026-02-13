# Permissions

Odyssey enforces permissions for tools, file paths, and command execution. Decisions are
resolved in this order:

1. Hooks
2. Rules (deny → allow → ask)
3. Implicit allow for tool follow-ups
4. Permission mode fallback

## How decisions are made
1. **Hooks**  
   Hook decisions (allow/deny) end evaluation immediately.

2. **Rules**  
   Rules are checked in this order: deny, allow, ask. The first match wins.

3. **Implicit allow for follow-ups**  
   If a tool is explicitly allowed, follow-up path/command checks from that tool are
   implicitly allowed unless a deny rule matches the specific path/command.

4. **Mode fallback**  
   - `default`: asks for approval; if no handler or event sink is configured, it auto-allows.
   - `accept_edits`: allows Read/Write/Edit/Glob/Grep tool calls plus workspace paths; asks for
     everything else.
   - `bypass_permissions`: allows all.
   - `plan`: denies tool usage by default.

## Rules
Rules live under `permissions.rules` and must target a tool, path, or command. Empty rules
are rejected by config validation.

```json5
permissions: {
  mode: "default",
  rules: [
    { action: "deny", tool: "Bash" },
    { action: "ask", tool: "Write" },
    { action: "allow", path: "src/**", access: "write" }
  ]
}
```

## Approval persistence
When a user responds with `allow_always`, Odyssey stores the decision at
`~/.odyssey/permission.jsonl`. The store is scoped to the current workspace root.
