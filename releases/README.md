# Release notes (YAML)

Each GitHub release can ship **hand-written notes** instead of (or before)
auto-generated commit lists.

## File layout

```
releases/
  v0.0.5.yaml    # notes for tag v0.0.5
  v0.0.6.yaml
  README.md
```

The publisher (`.github/scripts/release.py`) looks for:

```text
releases/<tag>.yaml
releases/<tag>.yml
```

If the file exists, it becomes the GitHub release body. If it is missing, the
script falls back to conventional-commit subjects since the previous SemVer tag.

> **Note:** `releases_notes.toml` is deprecated. Prefer `releases/<tag>.yaml`.

## Schema

```yaml
# Optional — when present must match the release tag
tag: v0.0.6

# Heading under "## …" (default: What's new)
title: Partial redaction

# Optional one-paragraph intro
highlights: |
  Short summary of the release for humans.

# Bullet lists (any can be omitted when empty)
features:
  - "User-facing feature in plain language"
  - "Another feature — markdown **bold** is fine"

fixes:
  - "Bug fix description"

changes:
  - "Docs, UX polish, refactors that are worth calling out"

breaking:
  - "Incompatible change (if any)"

# Optional: previous tag for the compare link (auto-detected when omitted)
# previous: v0.0.5
```

Rules:

- Keys are optional; omit empty sections.
- List items are strings (quote if they contain `:` or start with special YAML chars).
- `highlights` may use a folded (`>`) or literal (`|`) block.
- The `tag` field is optional; when present it must match the release tag.

## Workflow

1. Implement the release.
2. Add `releases/vX.Y.Z.yaml` describing what shipped.
3. Commit, then either:
   - tag `vX.Y.Z` and push the tag, or
   - run the **Auto Tag** workflow (`tag.yml`) which bumps the next patch tag, or
   - use **Release** `workflow_dispatch` with the tag input.
4. CI builds binaries, writes `checksums.txt`, and publishes the GitHub Release body
   from this YAML (or from commits if the file is missing).
