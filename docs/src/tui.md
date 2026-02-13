# TUI

The TUI embeds the orchestrator and connects directly to it over an in-process event bus.

## Run
```bash
export OPENAI_API_KEY="your-key"
cargo run -p odyssey-rs-tui
```

Optional flags:
```bash
cargo run -p odyssey-rs-tui -- --config ./odyssey.json5 --model gpt-4.1-mini
```

## Local llama.cpp
Build with the `local` feature to enable the llama.cpp provider. Optional GPU support is
available with the `cuda` or `metal` features.

```bash
cargo run -p odyssey-rs-tui --features local -- --local --local-gguf /path/to/model.gguf
```

HuggingFace repo example:
```bash
cargo run -p odyssey-rs-tui --features local -- --local \
  --local-hf-repo unsloth/Llama-3.2-3B-Instruct-GGUF \
  --local-hf-filename Llama-3.2-3B-Instruct-Q8_0.gguf
```
If `--local` is set without an explicit source, the default is:
`unsloth/Llama-3.2-3B-Instruct-GGUF` with `Llama-3.2-3B-Instruct-Q8_0.gguf`.
If `--local` is enabled and `OPENAI_API_KEY` is not set, the local provider becomes the default.

## Controls
- `Ctrl+N` create session
- `Ctrl+S` select highlighted session
- `Ctrl+R` refresh sessions
- `Enter` send message
- `PageUp`/`PageDown` scroll chat
- `y`/`a`/`n` approve permission (once / always / deny)

## Slash commands
- `/new` create a new session
- `/sessions` list sessions
- `/skills` list skills
- `/models` list registered models
- `/model <id>` select a model by id
- `/join <id>` join a session by id
