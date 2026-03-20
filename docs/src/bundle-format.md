# Bundle Format

## Source Project Layout

A bundle project is loaded from a directory containing:

```text
<project>/
  odyssey.bundle.json5
  agent.yaml
  skills/
  resources/
```

`odyssey.bundle.json5` points to the agent spec file through `agent_spec`, so the YAML file does not have to be named `agent.yaml`, but the default templates use that name.

## Manifest Schema

The bundle manifest currently deserializes into `BundleManifest` with these top-level fields:

- `id`
- `version`
- `agent_spec`
- `executor`
- `memory`
- `resources`
- `skills`
- `tools`
- `server`
- `sandbox`

Important current validation rules:

- `executor.type` must be `prebuilt`
- `memory.provider.type` must be `prebuilt`
- `executor.id` and `memory.provider.id` must be non-empty
- each resource and skill path must exist inside the project
- host mount paths in `sandbox.permissions.filesystem.mounts.read` and `.write` must be absolute paths
- tool entries must use `source: "builtin"`

Current implementation limits:

- `executor.id = "react"` is the only executor implemented by the runtime
- `memory.provider.id = "sliding_window"` is the only memory provider implemented by the runtime

## Agent Spec

The agent spec YAML currently maps to:

- `id`
- `description`
- `prompt`
- `model`
- `tools.allow`
- `tools.deny`

`model` is a `ModelSpec { provider, name, config }`. Session creation can override that model, and per-turn execution can override it again through `TurnContextOverride.model`.

## Build Output Layout

`odyssey-rs-bundle` materializes an installable bundle layout that contains:

```text
<install-root>/
  agent.yaml
  bundle.json
  index.json
  oci-layout
  blobs/
  resources/
  skills/
```

Build behavior that matters when authoring bundles:

- the agent spec file is copied into the built layout as `agent.yaml`
- each skill entry is copied into `skills/<skill.name>`
- each resource entry is copied into `resources/<basename>`
