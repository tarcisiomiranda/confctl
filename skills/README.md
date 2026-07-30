# confctl agent skills

Canonical skill definition: [`confctl/SKILL.md`](confctl/SKILL.md)

Follows the [Agent Skills](https://agentskills.io/specification) open format (`name` + `description` frontmatter + markdown body).

## Why this skill exists

Agents often `cat` or open `.env` / config files and pull **full secrets** into
context. The confctl skill tells them to use:

```bash
confctl .env -r        # full mask
confctl .env -r 10     # 10% start + end (identify without full secret)
confctl .env -r 20     # 20% start + end
```

so they can still debug hosts, keys, and structure safely — including on **Grok**.

## Install with the confctl binary installer (optional)

The curl/`install.sh` installer **does not** install agent skills by default.

```bash
# Binary only (default)
curl -fsSL …/install.sh | bash

# Binary + detect AIs and install SKILL.md globally
curl -fsSL …/install.sh | CONFCTL_INSTALL_SKILLS=1 bash
```

Truthy values: `1`, `true`, `yes`, `on`.

## Install from a git checkout (detects AIs on this PC)

From the repository root:

```bash
# See which agents are present
mise run skills:list
# or: python scripts/install_skills.py --list

# Install for detected agents (project + global paths)
mise run skills:install
# or: python scripts/install_skills.py

# Force every known tool path (includes Grok)
mise run skills:install:all

# Scope / dry-run
python scripts/install_skills.py --dry-run
python scripts/install_skills.py --project-only
python scripts/install_skills.py --global-only
python scripts/install_skills.py --only grok --only claude
```

The installer (`scripts/install_skills.py`) looks for config dirs under `$HOME`
(e.g. `~/.claude`, `~/.codex`, `~/.grok`) and binaries on `PATH`, then copies
`skills/confctl/SKILL.md` only where those tools live.

`./scripts/install-skills.sh` is a thin wrapper around the same Python script.

## Discovery paths by tool

| Tool | Project | Global |
|------|---------|--------|
| **Claude Code** | `.claude/skills/confctl/SKILL.md` | `~/.claude/skills/confctl/SKILL.md` |
| **OpenAI Codex** | `.codex/skills/confctl/SKILL.md` | `~/.codex/skills/confctl/SKILL.md` |
| **OpenCode** | `.opencode/skills/confctl/SKILL.md` | `~/.config/opencode/skills/confctl/SKILL.md` |
| **Cursor** | `.cursor/skills/confctl/SKILL.md` | `~/.cursor/skills/confctl/SKILL.md` |
| **Grok / xAI** | `.grok/skills/confctl/SKILL.md` | `~/.grok/skills/confctl/SKILL.md` |
| **Generic / multi-agent** | `.agents/skills/confctl/SKILL.md` | `~/.agents/skills/confctl/SKILL.md` |

## When agents should load this skill

- User asks to inspect `.env`, YAML/TOML/JSON config, or a single config key
- Task needs secrets/DB URLs without putting full values in context
- Mentions of confctl, redact, `-r`, or “don’t show me the whole secret”
