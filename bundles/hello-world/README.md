# Odyssey Hello World

This is an Hello World Agent Bundle 

## Build

```bash
cargo run -p odyssey-rs -- build bundles/hello-world
```

Or write the built bundle to a custom directory:

```bash
cargo run -p odyssey-rs -- build bundles/hello-world --output ./dist
```

## Run

```bash
export OPENAI_API_KEY="your-key"
cargo run -p odyssey-rs -- run hello-world@latest --prompt "Hey, What is your name?"
```

Host filesystem mounts are configured under `sandbox.permissions.filesystem.mounts`.
The default cowork bundle leaves both `read` and `write` empty, so host volume access is denied by default.
