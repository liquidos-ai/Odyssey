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
- Use a non-empty `network` list only when the bundle intentionally needs network-capable command execution.
