# specs/

Canonical inventory of `confctl` — one YAML per functional domain of the CLI.

`confctl` is a single Rust binary, not a web app, so the usual `models / graphql / rest / frontend` taxonomy does not apply. Instead, each spec describes one **functional area** of the CLI: its public surface (flags, inputs), the module(s) that implement it, the key functions/types, and any relevant tests or fixtures.

## Read order when entering a domain

1. Open [index.yaml](index.yaml) to locate the domain.
2. Read `{domain}.yaml` — it lists every relevant file path and symbol.
3. Only then open the Rust sources referenced.

## File layout

- `_schema.yaml` — canonical template with every field a domain YAML may use.
- `index.yaml` — one-line description of every domain in the project.
- `{domain}.yaml` — per-domain inventory. Each item has a `path:` pointing at a real file (line hints optional).
- `README.md` — this file.

## Sync rule (golden rule)

When you **add, remove, or rename** any item listed in a spec (a module, function, CLI flag, supported format, test, build target), update the domain YAML **in the same commit** and bump `last_updated` to today's date.

Pure refactors that don't change the public surface do **not** need a spec update.

If a whole new functional area appears, copy `_schema.yaml` to `specs/{new}.yaml`, populate it, and add the entry to `index.yaml`. If an area is removed, delete its YAML and its line in `index.yaml`.

## Checklist before closing a code task

- [ ] Added / removed / renamed anything listed in a spec? → update the YAML.
- [ ] `last_updated` bumped?
- [ ] New domain → added to `index.yaml`?
- [ ] Domain gone → YAML deleted and removed from `index.yaml`?

## Current domains

See [index.yaml](index.yaml).
