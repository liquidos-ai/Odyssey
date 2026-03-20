# Sandbox And Tools

## Builtin Tools

The builtin tool registry currently contains:

- `Read`
- `Write`
- `Edit`
- `Glob`
- `Grep`
- `Bash`
- `Skill`

## How Tool Selection Works

Tool availability is the result of two filters:

1. Bundle manifest filter:
   if `manifest.tools` is empty, the runtime starts from the full builtin registry;
   otherwise it starts from the named manifest tool list.
2. Agent spec filter:
   `tools.allow` narrows that list unless it contains `*`, and `tools.deny` removes entries.

The final list is sorted, deduplicated, and adapted into the executor's tool interface.

## Sandbox Modes

The protocol and manifest support three sandbox modes:

- `read_only`
- `workspace_write`
- `danger_full_access`

How they are used today:

- `danger_full_access` uses the host provider directly
- on Linux, `read_only` and `workspace_write` use the `bubblewrap` provider by default
- on non-Linux, confined modes fall back to the host provider because `bubblewrap` is Linux-only

## Filesystem Policy

The runtime builds sandbox filesystem policy from the manifest:

- `mounts.read` becomes host read roots
- `mounts.write` becomes host write roots
- `filesystem.exec` becomes executable paths under the staged bundle root

The sandbox cell root itself is always added as a readable root. In `workspace_write` and `danger_full_access`, that cell root is also writable.

## Network Policy

The current network implementation is intentionally simple:

- an empty `sandbox.permissions.network` list disables network access
- a non-empty list enables outbound network access

The individual strings in that list are not enforced as a hostname allowlist by the current sandbox bridge. At the moment the list acts like an on/off switch.

One practical consequence: when the runtime falls back to the host provider and the bundle disables network, execution is rejected because the host backend cannot safely emulate that restriction.

## Tool Permission Rules

Per-tool rules come from `sandbox.permissions.tools.rules`.

Important details from the current implementation:

- missing rules default to `allow`
- `sandbox.permissions.tools.mode` is not interpreted by the runtime yet
- approval prompts are only generated for tools marked `ask`

## System Tools

`sandbox.system_tools` is currently used as a preflight dependency check. Before execution starts, the runtime verifies that each named binary exists using   `which`.
