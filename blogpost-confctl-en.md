# confctl: a simple CLI to query JSON, YAML, TOML, and ENV files

If you automate anything in shell scripts, you probably know the pain: one script reads `config.yaml`, another reads `config.toml`, and then JSON shows up and you end up with `grep`, `awk`, and `sed`.

`confctl` was built to solve this with one binary and one query syntax.

![confctl demo](https://storage.googleapis.com/tarcisio-blog/5-confctl-video.gif)

## What is confctl?

`confctl` is a command-line tool for querying structured data from configuration files using dotted path syntax, like:

```bash
confctl config.yaml clubs.0.players.1.name
```

It supports:

- JSON (`.json`)
- YAML (`.yaml`, `.yml`)
- TOML (`.toml`)
- ENV (`.env` and `KEY=value` files)

## Installation

Quick install:

```bash
curl -fsSL https://raw.githubusercontent.com/tarcisiomiranda/confctl/main/install.sh | bash
```

Install a specific version:

```bash
curl -fsSL https://raw.githubusercontent.com/tarcisiomiranda/confctl/main/install.sh | bash -s v0.0.3
```

Verify installation:

```bash
confctl --version
```

## Using confctl with files

General syntax:

```bash
confctl [file|-] [path] [--format <json|yaml|toml|env>]
```

Examples:

```bash
# JSON
confctl config.json clubs.0.name

# YAML
confctl config.yaml clubs.0.players.1.name

# TOML
confctl config.toml clubs.2.titles.champions_league

# .env
confctl .env DATABASE_URL
```

If you omit `path`, `confctl` prints the full input normalized as JSON:

```bash
confctl config.toml
```

## Using confctl with curl (stdin)

When data is piped, `confctl` reads from stdin automatically:

```bash
curl -s https://api.github.com/users | confctl
curl -s https://api.github.com/users | confctl 0.login
```

You can still use `-` explicitly:

```bash
curl -s https://api.github.com/users | confctl - 0.login
```

If format auto-detection is ambiguous, force it:

```bash
curl -s https://api.github.com/users | confctl 0.login --format json
```

## Useful features

- One query syntax across multiple formats.
- Colored output in terminal.
- Clear path-aware error messages.
- Built-in Base64 encode/decode with `--encode` and `--decode`.

## Where it helps most

- CI/CD pipelines.
- Deployment scripts.
- Fast API response inspection.
- Reading config values without juggling multiple tools.

Project: https://github.com/tarcisiomiranda/confctl
