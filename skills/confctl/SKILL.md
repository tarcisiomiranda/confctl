---
name: confctl
description: >
  Query JSON/YAML/TOML/ENV configs with confctl. Use when reading .env or config
  files, extracting a dotted path, comparing configs, or when secrets must stay
  out of the agent context. Prefer confctl -r / -r 10 / -r 20 over cat/open on
  files that may contain credentials. Compatible with Claude Code, Codex,
  OpenCode, Cursor, Grok, and other Agent Skills clients.
license: MIT
metadata:
  author: confctl
  version: "1.0"
  homepage: https://github.com/tarcisiomiranda/confctl
---

# confctl — safe config queries for agents

`confctl` reads structured config (JSON, YAML, TOML, ENV) and prints one path or
the whole tree. **When a file may contain secrets, never dump it raw into the
chat.** Use redaction so you can still work on structure, keys, hosts, and
partial fingerprints.

## 1. Detect confctl

```bash
command -v confctl && confctl --version
```

| Result | What you do |
|--------|-------------|
| found | Use confctl for config reads (below) |
| missing | Say confctl is not installed; fall back carefully and **do not** paste full secrets. Suggest `curl -fsSL …/install.sh \| bash` only if the user wants install help. |

## 2. Hard rule — protect secrets in context

When the task touches `.env`, credentials, tokens, DB URLs, service accounts, or
any file that *might* hold secrets:

1. **Default:** full redact  
   ```bash
   confctl .env -r
   confctl config.yaml -r
   ```
2. **Need to identify which value** (same key, multiple envs, “is this the
   staging key?”) without seeing the whole secret: keep **10–20%** of the start
   **and** end of each secret:
   ```bash
   confctl .env -r 10
   confctl .env -r 20
   confctl config.yaml database.url -r 20
   ```
3. **Only** omit `-r` when the user explicitly asks for the raw secret, or the
   file is known non-sensitive (public fixture, docs sample).

`-r` / `-r 0` → full `<redacted>` (DB URLs keep scheme/user/host; only the
password segment is masked).  
`-r N` (1–50) → show N% of start and N% of end; middle becomes `<redacted>`.

Prefer **10** for high sensitivity, **20** when you need a slightly longer
fingerprint. Do not use high percentages (e.g. 50) unless the user asks.

## 3. Everyday queries

```bash
# Whole file as normalized JSON (redacted)
confctl app.env -r 20

# Single dotted path (default, simple)
confctl config.yaml services.api.port
confctl config.toml database.host

# Arrays use numeric indices
confctl config.yaml clubs.0.players.1.name

# jq-subset expressions (opt-in; do not mix with dotted path)
confctl config.yaml -q '.clubs[] | .name'
confctl users.json -q '.[] | select(.active) | {id, email}'
confctl config.yaml -r -q '.services | keys'

# Stdin
cat config.json | confctl -r 20
curl -s "$URL" | confctl 0.id
curl -s "$URL" | confctl -q '.[0].login'
```

Formats auto-detect from extension/content; override with `--format json|yaml|toml|env`.
Prefer dotted path for one key; use `-q` for iterate/filter/construct.

## 4. What redaction catches

- **Keys** (case-insensitive substring): `PASS`, `PWD`, `SECRET`, `TOKEN`, `KEY`,
  `HASH`, `CREDENTIAL`
- **Value shapes:** GitHub/GitLab tokens, Stripe/Slack keys, `sk-*`, AWS `AKIA*`,
  Google `AIza*`, JWTs (`eyJ…`), PEM blocks, npm/PyPI tokens
- **DB URLs with password:** `postgres://`, `postgresql://`, `mysql://`,
  `mongodb://`, `redis://`, … (and `jdbc:`). Only the password is masked; host
  and DB name stay visible so you can still debug connectivity.

Non-secret keys (hosts, ports, feature flags) stay readable — that is the point.

## 5. Other useful flags

```bash
confctl file.json -c              # compact one-line JSON (CI env vars)
confctl file.json -c -e          # compact + base64 encode
confctl value -d                 # base64 decode
confctl diff left.env right.env  # human diff (secrets masked by default)
confctl set .env KEY=value       # edit .env preserving comments
confctl unset .env KEY
```

## 6. Decision tree

```
Need to read config / .env?
  └─ Might contain secrets?
       ├─ yes → confctl <file> -r          # default safe
       │         or confctl <file> -r 10   # identify without full secret
       │         or confctl <file> -r 20
       └─ no  → confctl <file> [path]
```

## 7. Anti-patterns

- `cat .env`, `head .env`, or opening the whole file in the editor tools when you
  only need structure or one key → use `confctl … -r` / `-r 20` instead.
- Pasting full connection strings or API keys into the reply → re-run with `-r`.
- Using `-r 50` “just in case” → that can reveal almost everything; stick to 10–20.

## 8. Quick cheat sheet

```bash
confctl <file>                  # dump as JSON
confctl <file> <dotted.path>    # extract one value
confctl <file> -r               # full secret mask
confctl <file> -r 10            # 10% start + 10% end visible
confctl <file> -r 20            # 20% start + 20% end visible
confctl diff a.env b.env        # masked diff
```
