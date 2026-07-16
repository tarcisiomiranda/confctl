# confctl

`confctl` is a Lightweight CLI for querying structured data in configuration files. It supports the formats engineers use day to day: JSON, YAML, and TOML with a simple dotted path syntax and no filter language to learn.

## Demo

![confctl demo](data/main_video.gif)

```bash
confctl config.yaml clubs.0.players.1.name
# Juninho Pernambucano

confctl config.toml
# { "clubs": [ { "name": "Club de Regatas Vasco da Gama", ... } ] }
```

## Installation

### Quick install (recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/tarcisiomiranda/confctl/main/install.sh | bash
```

Or install a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/tarcisiomiranda/confctl/main/install.sh | bash -s v0.0.3
```

### Available binaries

| Binary | Platform |
|--------|----------|
| `confctl-linux-amd64` | Linux x86_64 (static MUSL) |
| `confctl-darwin-arm64` | macOS Apple Silicon (M1/M2/M3) |
| `confctl-darwin-amd64` | macOS Intel |

### Install in a Dockerfile (using ADD)

The Linux binary is fully static no dependencies, no glibc. Use `ADD` to pull it directly from GitHub Releases:

```dockerfile
ADD https://github.com/tarcisiomiranda/confctl/releases/latest/download/confctl-linux-amd64 \
    /usr/local/bin/confctl
RUN chmod +x /usr/local/bin/confctl
```

Or pin to a specific version for reproducibility:

```dockerfile
ADD https://github.com/tarcisiomiranda/confctl/releases/download/v0.0.3/confctl-linux-amd64 \
    /usr/local/bin/confctl
RUN chmod +x /usr/local/bin/confctl
```

Works with any base image (`debian`, `alpine`, `ubuntu`, `scratch`-based) no apt/apk install required.

---

## Usage

```
confctl [file|-] [path] [--format <json|yaml|toml|env>]
```

- **With path**  extracts the value at the dotted path
- **Without path**  prints the entire input as normalized JSON

### Extracting values

Use dot-separated keys to navigate nested structures. Use numeric indices for arrays.

```bash
confctl config.json clubs.0.name
# Club de Regatas Vasco da Gama

confctl config.yaml clubs.0.players.1.name
# Juninho Pernambucano

confctl config.toml clubs.2.titles.champions_league
# 15

confctl config.yaml clubs.1.stadium
# Emirates Stadium
```

### Reading from stdin (curl)

When data is piped into `confctl`, stdin is used automatically.

```bash
curl -s https://api.github.com/users | confctl
curl -s https://api.github.com/users | confctl 0.login
```

You can still use `-` explicitly:

```bash
curl -s https://api.github.com/users | confctl - 0.login
```

If format auto-detection is ambiguous, pass `--format`:

```bash
curl -s https://api.github.com/users | confctl 0.login --format json
```

### Printing the whole file

Omit the path to dump the entire file as formatted JSON useful for discovering available keys.

```bash
confctl config.toml
```

```json
{
  "clubs": [
    {
      "name": "Club de Regatas Vasco da Gama",
      "country": "Brazil",
      "players": [
        { "name": "Edmundo", "number": 9, "position": "striker" },
        { "name": "Juninho Pernambucano", "number": 8, "position": "midfielder" }
      ]
    }
  ]
}
```

### Redacting secrets (`-r` / `--redact`)

Masks sensitive values with `<redacted>`, keeping everything else visible. Output is safe to paste into logs, issues, or AI chats — often the bug is in the non-sensitive values anyway.

Two detection layers:

- **By key** (case-insensitive substring): `PASS`, `PWD`, `SECRET`, `TOKEN`, `KEY`, `HASH`, `CREDENTIAL`
- **By value shape**, regardless of key: GitHub tokens (`ghp_*`, `github_pat_*`, …), GitLab (`glpat-*`), Stripe (`sk_live_*`, …), Slack (`xoxb-*`, …), OpenAI/Anthropic-style (`sk-*`), AWS access key IDs (`AKIA*`), Google API keys (`AIza*`), npm/PyPI tokens, JWTs (`eyJ…`), PEM blocks (`-----BEGIN`)

```bash
confctl .env -r
```

```json
{
  "API_KEY": "<redacted>",
  "DATABASE_HOST": "192.0.2.9",
  "DATABASE_PORT": 5432,
  "DEBUG": true,
  "POSTGRES_PASSWORD": "<redacted>"
}
```

Works with any format and recurses into nested objects and arrays. `confctl diff` masks secrets by default (`--show-secrets` reveals them there).

### Compact output + clipboard (`-c`, `--copy`)

`-c` / `--compact` prints minified single-line JSON — made for stuffing a whole file (service-account JSON, config blob) into a CI/pipeline env var. `--copy` also sends the final output to the system clipboard (`wl-copy`, `xclip`, `xsel`, or `pbcopy` — first one found; confirmation goes to stderr so pipes stay clean). Both compose with `-e`:

```bash
cat service_account.json | confctl -c            # {"type":"service_account","project_id":...}
cat service_account.json | confctl -c -e --copy  # minified → base64 → clipboard (and stdout)
```

Paste straight into GitHub Actions / GitLab CI as a secret, then decode on the other side:

```bash
echo "$GCP_SA_B64" | confctl - -d > service_account.json
```

### Editing .env files (`set` / `unset`)

Add, update, or remove keys **in place** without opening the file — comments, blank lines, ordering, `export` prefixes, and inline `#` comments are all preserved. Designed for scripts and AI agents that need to mutate a `.env` without reading its (possibly sensitive) contents.

```bash
confctl set .env DB_HOST=10.0.0.5 NEW_FLAG=on     # add or update, several at once
confctl set .env GREETING="hello world"           # values with spaces get quoted
confctl unset .env DEBUG OLD_KEY                  # remove keys
```

```text
✓ updated DB_HOST
✓ added NEW_FLAG
✓ removed DEBUG
· OLD_KEY not found (nothing to remove)
```

Rules: `set` creates the file if missing and appends new keys at the end; `unset` on a missing key is a no-op that still exits 0 (idempotent); commented-out lines like `# DB_HOST=old` are never matched.

### Error handling

```bash
confctl config.json missing.key
# Error: Key not found: 'missing' (at path 'missing')
# exit code 1
```

---

## Supported formats

| Extension | Format |
|---|---|
| `.json` | JSON |
| `.yaml`, `.yml` | YAML |
| `.toml` | TOML |
| `.env` | ENV |

Format is detected automatically from the file extension. For `stdin` (`-`) or extensionless files, `confctl` also tries to auto-detect content and supports `--format`.

## Path syntax

| Example | Description |
|---|---|
| `club.name` | Nested object key |
| `players.0.name` | Array index + key |
| `titles.la_liga` | Deep key |
| `season` | Top-level key (returns the whole object) |

---

## Vault (remote secrets)

`confctl vault` pushes and pulls secret files (`.env`, service-account JSONs, ad-hoc blobs) to a remote secret store. Unified CLI over multiple backends:

| Backend | Flag | Auth model |
| --- | --- | --- |
| `bunker` | `--backend bunker` | Zero-knowledge: Argon2id + XChaCha20-Poly1305, encrypted client-side. |
| `hcp` | `--backend hcp` | HashiCorp Vault KV v2. `X-Vault-Token` header. |
| `gcp` | `--backend gcp` | Google Secret Manager via ambient ADC (`gcloud auth application-default login`). |
| `aws` | `--backend aws` | AWS Secrets Manager. *(planned)* |
| `azure` | `--backend azure` | Azure Key Vault. *(planned)* |

### Config lookup

Non-secret config lives in TOML, looked up in this order (first existing wins):

1. `$CONFCTL_CONFIG`
2. `~/.config/confctl/vault.toml` — written by `vault login`
3. `/etc/confctl/vault.toml` — system-wide default, read-only

Secrets never land in `/etc/` — cached sessions go to the OS keyring (Secret Service / Keychain / Credential Manager), with a chmod-0600 file fallback under `~/.config/confctl/`.

### Auth: three ways to point at a backend

**(a) System config file** (`/etc/confctl/vault.toml`) — no login needed, reads credentials from files you pre-place:

```toml
backend = "hcp"

[hcp]
addr = "http://192.0.2.2:8200"
mount = "secret"
token_file = "/etc/confctl/hcp-token"   # chmod 0600
```

**(b) Environment variables** — upstream conventions (`VAULT_ADDR`, `VAULT_TOKEN`, `AWS_*`, `GOOGLE_APPLICATION_CREDENTIALS`, `AZURE_*`) work when set:

```bash
export VAULT_TOKEN='hvs.…'
confctl vault --backend hcp login --endpoint http://192.0.2.2:8200
```

**(c) Interactive login** — prompts for whatever's missing (token, password, etc.) and caches it in the OS keyring:

```bash
confctl vault --backend hcp login --endpoint http://192.0.2.2:8200
# Vault token: ***
```

### Commands

```bash
confctl vault login    [--backend X] [--endpoint URL] [--identity NAME]
confctl vault status
confctl vault push     <file|-> [--name NAME] [--mime TYPE] [--labels a,b] [--overwrite]
confctl vault list     [--json]
confctl vault pull     <name> [--id UUID] [--out PATH|-] [--force]
confctl vault rm       <name> [--yes]
confctl vault logout
```

`<file|->` reads a local path or stdin (`-`). `pull --out -` writes decrypted bytes to stdout. `--backend` is global and overrides the config file.

### Automatic secret names

`push` without `--name` generates `{current-dir}-{filename}-{YYYY-MM-DD}`, slugged to `[A-Za-z0-9_-]` (the strictest backend charset). Pushing `.env` from `/srv/myapp` on 2026-07-12 stores `myapp-env-2026-07-12` — a dated snapshot per push, no command memorization needed. Pass `--name` to override.

```bash
confctl vault push .env                       # → myapp-env-2026-07-12
confctl vault push .env --name myapp-env      # fixed name; --overwrite updates it
```

### GCP Secret Manager quickstart

Uses the Application Default Credentials you already have from `gcloud` — no separate login flow:

```bash
gcloud auth application-default login          # once, if not already done
confctl vault login --backend gcp --endpoint my-project-id
confctl vault push .env                        # → creates secret myapp-env-2026-07-12
confctl vault list
confctl vault pull myapp-env-2026-07-12 --out .env
```

The project is saved to `vault.toml`; omit `--endpoint` to pick it up from `GOOGLE_CLOUD_PROJECT` or `gcloud config get-value project`:

```toml
backend = "gcp"

[gcp]
project = "my-project-id"
# credentials_file = "/path/to/adc.json"   # optional explicit ADC file
```

Credential discovery order: `CLOUDSDK_AUTH_ACCESS_TOKEN` → `gcp.credentials_file` → `GOOGLE_APPLICATION_CREDENTIALS` → gcloud ADC file → `gcloud` CLI → GCE metadata server. Filename and MIME type are preserved via secret annotations, so `pull` restores the original file name.

---

## Building from source

Requires [mise](https://mise.jdx.dev/) and Rust 1.91.1.

```bash
mise run build             # default release build
mise run build:linux-musl  # fully static MUSL binary
mise run test              # run unit tests
```

## Docker

```bash
docker build -t confctl .

docker run --rm -v $(pwd)/testdata:/data confctl \
  /data/config.yaml clubs.0.name
```
