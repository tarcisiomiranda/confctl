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
