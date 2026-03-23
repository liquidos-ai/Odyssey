# {{ bundle_id }}

This is a starter Odyssey bundle.

## Install Odyssey

```bash
curl -fsSL https://raw.githubusercontent.com/liquidos-ai/odyssey/main/install.sh | bash
```

The installer downloads the prebuilt release archive for the current platform and installs
`odyssey-rs` into `~/.local/bin` by default. You can also install it directly with Cargo:

```bash
cargo install odyssey-rs
```

## Build

```bash
odyssey-rs build {{ bundle_path }}
```

Or write the built bundle to a custom directory:

```bash
odyssey-rs build {{ bundle_path }} --output ./dist
```

## Run

```bash
export OPENAI_API_KEY="your-key"
odyssey-rs run {{ bundle_id }}@latest --prompt "Hey, What is your name?"
```

## Starter Template Defaults

The generated `odyssey.bundle.json5` starts with a development-friendly policy:

- `executor.id: "react"`
- `memory.id: "sliding_window"`
- all builtin tools listed in `manifest.tools`
- `sandbox.mode: "read_only"`
- `sandbox.permissions.network: ["*"]`
- `sandbox.system_tools_mode: "standard"`
- `sandbox.system_tools: []`
- `sandbox.env: {}`

The generated `agent.yaml` starts with:

- an OpenAI model entry using `gpt-4.1-mini`
- empty `tools.allow`, `tools.ask`, and `tools.deny`

Because those tool-rule lists are empty, the agent still has access to the tools declared by the
manifest. Tighten both files before you treat the bundle as production automation.

## Sandbox And Tool Notes

- `sandbox.permissions.network: []` disables outbound network access for commands run through
  bundle tools such as `Bash`.
- `sandbox.permissions.network: ["*"]` enables unrestricted outbound network access. Hostname
  allowlists are not implemented in v1.
- `agent.yaml` owns tool permissions through `tools.allow`, `tools.ask`, and `tools.deny`.
- Tool permission entries can be coarse like `Bash` or granular like `Bash(cargo test:*)` and
  `Bash(find:*)`.
- Invalid tool permission entries are rejected instead of silently falling back to broad tool-name
  matching.
- `sandbox.env` maps sandbox variable names to host environment variable names for sandboxed bundle
  commands. Missing host variables are skipped.
- Model-provider credentials such as `OPENAI_API_KEY` are read by the Odyssey runtime process
  itself, not from `sandbox.env`.
- `sandbox.system_tools_mode` controls host executable policy for sandboxed process execution:
  `explicit`, `standard`, or `all`.
- `sandbox.system_tools` lists additional named host binaries when `system_tools_mode` is
  `explicit` or when you want to supplement `standard`.
- In confined Linux sandboxes, `explicit` mounts only the declared binaries and bundle-local exec
  paths, `standard` mounts the standard host executable roots, and `all` removes the execute
  allowlist for files already visible inside the sandbox.

## Platform Notes

- Restricted sandboxes require Linux and `bubblewrap` (`bwrap`).
- On macOS and Windows, use `--dangerous-sandbox-mode` for local development or run Odyssey inside
  a Linux container if you want confined execution.
