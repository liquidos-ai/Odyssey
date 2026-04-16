# Skills

Skills are markdown instructions stored in `SKILL.md` files. Odyssey scans configured
roots, builds summaries, and exposes a tool that can load full skill content at runtime.

## Skill file format
```md
---
name: your-skill-name
description: Brief description of what this Skill does and when to use it
---

# Your Skill Name

## Instructions
Step-by-step guidance here.
```

Frontmatter is optional. If `name` is missing:
- The first `# Heading` is used as the skill name.
- If no heading exists, the parent directory name is used.

If `description` is missing, the first non-heading, non-empty line is used.

## Discovery rules
- Roots are determined from `skills.setting_sources` and `skills.paths`.
- `setting_sources` supports `user`, `project`, and `system`.
- User skills live at `~/.odyssey/skills/**/SKILL.md`.
- Project skills live at `.odyssey/skills/**/SKILL.md`.
- System skills live at `/etc/odyssey/skills/**/SKILL.md` (Unix).
- `skills.allow`/`skills.deny` are applied by case-insensitive name.
- Duplicate skill names across roots are rejected.

## Config snippet
```json5
skills: {
  setting_sources: ["user", "project"],
  paths: [],
  allow: ["*"],
  deny: []
}
```

## Tool usage
The model can call the `Skill` tool with a skill name to load the full SKILL.md content.
The summary list is inserted into the system prompt by the `PromptBuilder`.
