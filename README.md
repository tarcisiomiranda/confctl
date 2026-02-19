# confctl

Inspired by [jq](https://jqlang.github.io/jq/), `confctl` brings the same idea of querying structured data to configuration files. Instead of JSON streams, it targets the files engineers actually use day-to-day — JSON, YAML, and TOML — with a simple dotted path syntax and no filter language to learn.

```bash
confctl get club.yaml players.0.name
# Vinicius Junior

confctl print club.toml
# { "club": { "name": "FC Barcelona", ... } }
```

## Installation

### One-line install (auto-detects root vs user)

```bash
curl -fsSL https://github.com/tarcisiomiranda/confctl/releases/latest/download/confctl \
  -o /tmp/confctl && chmod +x /tmp/confctl

if [ "$(id -u)" -eq 0 ]; then
  mv /tmp/confctl /usr/local/bin/confctl
  echo "Installed to /usr/local/bin/confctl"
else
  mkdir -p ~/.local/bin
  mv /tmp/confctl ~/.local/bin/confctl
  echo "Installed to ~/.local/bin/confctl"
  echo ""
  echo "Make sure ~/.local/bin is in your PATH. Add to ~/.bashrc or ~/.zshrc:"
  echo '  export PATH="$HOME/.local/bin:$PATH"'
fi
```

### Manual install

**As root / sudo:**
```bash
curl -L https://github.com/tarcisiomiranda/confctl/releases/latest/download/confctl \
  | sudo install /dev/stdin /usr/local/bin/confctl
```

**As regular user (no sudo):**
```bash
mkdir -p ~/.local/bin
curl -fsSL https://github.com/tarcisiomiranda/confctl/releases/latest/download/confctl \
  -o ~/.local/bin/confctl
chmod +x ~/.local/bin/confctl

# Add to your shell profile if not already there:
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc
```

Verify:

```bash
confctl --version
# confctl 0.1.0
```

### Install in a Dockerfile (using ADD)

The binary is fully static — no dependencies, no glibc. Use `ADD` to pull it directly from GitHub Releases:

```dockerfile
ADD https://github.com/tarcisiomiranda/confctl/releases/latest/download/confctl \
    /usr/local/bin/confctl
RUN chmod +x /usr/local/bin/confctl
```

Or pin to a specific version for reproducibility:

```dockerfile
ADD https://github.com/tarcisiomiranda/confctl/releases/download/v0.1.0/confctl \
    /usr/local/bin/confctl
RUN chmod +x /usr/local/bin/confctl
```

Works with any base image (`debian`, `alpine`, `ubuntu`, `scratch`-based) — no apt/apk install required.

---

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

confctl get club.yaml season
# { "year": "2024-25", "league": "La Liga" }
```

### `print` — Normalize to JSON

Prints the entire file as formatted JSON — useful for exploring available keys before querying.

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
  ],
  "titles": { "la_liga": 27, "champions_league": 5 }
}
```

### Error handling

```bash
confctl get club.json missing.key
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

Format is detected automatically from the file extension.

## Path syntax

| Example | Description |
|---|---|
| `club.name` | Nested object key |
| `players.0.name` | Array index + key |
| `titles.la_liga` | Deep key |
| `season` | Top-level key (returns the whole object) |

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
  get /data/club.yaml players.0.name
```
