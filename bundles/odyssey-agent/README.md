# Odyssey Agent Bundle

This is an general assistant agent bundle for Odyssey.

## Build

```bash
cargo run -p odyssey-rs -- build bundles/odyssey-agent
```

Or write the built bundle to a custom directory:

```bash
cargo run -p odyssey-rs -- build bundles/odyssey-agent --output ./dist
```

## Run

```bash
export OPENAI_API_KEY="your-key"
cargo run -p odyssey-rs -- run odyssey-agent@latest --prompt "Heyy, There!"
```

Host filesystem mounts are configured under `sandbox.permissions.filesystem.mounts`.
The default cowork bundle leaves both `read` and `write` empty, so host volume access is denied by default.
