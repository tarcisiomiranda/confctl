# confctl

A simplified [jq](https://jqlang.github.io/jq/) for configuration files. Read values from JSON, YAML, and TOML files using a simple dotted path syntax.

```bash
confctl get club.toml players.0.name
# Vinicius Junior
```

## Installation

### Download the binary

Download the latest static binary from the [Releases](../../releases/latest) page:

```bash
# Linux x86_64 (static — no dependencies required)
curl -L https://github.com/corebunker/confctl/releases/latest/download/confctl \
  -o confctl
chmod +x confctl
```

### Add to your PATH

**Option 1 — user-local (recommended, no sudo)**

```bash
mkdir -p ~/.local/bin
mv confctl ~/.local/bin/confctl

# Add to PATH if not already there (add to ~/.bashrc or ~/.zshrc)
export PATH="$HOME/.local/bin:$PATH"
```

**Option 2 — system-wide (requires sudo)**

```bash
sudo mv confctl /usr/local/bin/confctl
```

Verify:

```bash
confctl --version
# confctl 0.1.0
```

## Usage

```
confctl get <file> <path>    Read a value by dotted path
confctl print <file>         Print the whole file as normalized JSON
```

### `get` — Read a value

Use dot-separated keys to navigate nested structures. Use numeric indices to access array elements.

```bash
confctl get club.json club.name
# Arsenal FC

confctl get club.yaml players.0.name
# Vinicius Junior

confctl get club.toml titles.champions_league
# 5

confctl get club.yaml players.1.position
# midfielder
```

### `print` — Normalize to JSON

```bash
confctl print club.toml
```

```json
{
  "club": {
    "name": "FC Barcelona",
    "founded": 1899,
    "stadium": "Spotify Camp Nou"
  },
  "players": [
    { "name": "Lamine Yamal", "number": 19, "position": "winger" },
    { "name": "Pedri", "number": 8, "position": "midfielder" }
  ]
}
```

### Error handling

```bash
confctl get club.json missing.key
# Error: Key not found: 'missing' (at path 'missing')
# exit code 1
```

## Supported formats

| Extension | Format |
|---|---|
| `.json` | JSON |
| `.yaml`, `.yml` | YAML |
| `.toml` | TOML |

Format is detected automatically from the file extension.

## Path syntax

| Example | Description |
|---|---|
| `club.name` | Nested object key |
| `players.0.name` | Array index + key |
| `titles.la_liga` | Deep key |
| `season` | Top-level key (returns the whole object) |

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
  get /data/club.yaml players.0.name
```
