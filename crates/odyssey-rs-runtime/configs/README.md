# {{ bundle_id }}

This is an Hello World Agent Bundle 

## Build

```bash
cargo run -p odyssey-rs -- build {{ bundle_path }}
```

Or write the built bundle to a custom directory:

```bash
cargo run -p odyssey-rs -- build {{ bundle_path }} --output ./dist
```

## Run

```bash
export OPENAI_API_KEY="your-key"
cargo run -p odyssey-rs -- run {{ bundle_id }}@latest --prompt "Hey, What is your name?"
```

## Sandbox

`odyssey.bundle.json5` uses default-deny networking for sandboxed bundle commands.

- `sandbox.mode` controls the default command isolation mode for the bundle. Use `workspace_write` unless the bundle has a strong reason to require `read_only` or `danger_full_access`.
- `sandbox.permissions.network: []` disables outbound network access for commands run through bundle tools such as `Bash`.
- `sandbox.permissions.network: ["*"]` enables unrestricted outbound network access. Hostname allowlists are not implemented in v1.
- `sandbox.permissions.tools` supports both legacy `rules` entries and grouped `allow` / `ask` / `deny` lists.
- Tool permission entries can be coarse like `Bash` or granular like `Bash(cargo test:*)` and `Bash(find:*)`.
- `sandbox.env` maps sandbox variable names to host environment variable names, and Odyssey reads those host values at runtime before launching the sandbox.
- `sandbox.system_tools_mode` controls host executable policy for sandboxed process execution:
  `explicit`, `standard`, or `all`.
- `sandbox.system_tools` lists additional named host binaries when `system_tools_mode` is
  `explicit` or when you want to supplement `standard`.
