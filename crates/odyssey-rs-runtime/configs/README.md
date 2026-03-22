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
- `sandbox.system_tools_mode` controls host executable policy for sandboxed process execution:
  `explicit`, `standard`, or `all`.
- `sandbox.system_tools` lists additional named host binaries when `system_tools_mode` is
  `explicit` or when you want to supplement `standard`.
