# confctl: uma CLI simples para consultar JSON, YAML, TOML e ENV

Quem trabalha com automação já passou por isso: em um script você precisa ler um valor de `config.yaml`, em outro de `config.toml`, e quando chega no JSON já está com `grep`, `awk` e `sed` na mão.

O `confctl` nasceu para resolver esse problema com um único comando e uma única sintaxe de consulta.

![demo do confctl](https://storage.googleapis.com/tarcisio-blog/5-confctl-video.gif)

## O que é o confctl

`confctl` é uma CLI para consultar dados estruturados em arquivos de configuração com notação por caminho (`dotted path`), como:

```bash
confctl config.yaml clubs.0.players.1.name
```

Ele suporta:

- JSON (`.json`)
- YAML (`.yaml`, `.yml`)
- TOML (`.toml`)
- ENV (`.env` e arquivos `CHAVE=valor`)

## Instalação

Instalação rápida:

```bash
curl -fsSL https://raw.githubusercontent.com/tarcisiomiranda/confctl/main/install.sh | bash
```

Instalando uma versão específica:

```bash
curl -fsSL https://raw.githubusercontent.com/tarcisiomiranda/confctl/main/install.sh | bash -s v0.0.3
```

Depois, valide:

```bash
confctl --version
```

## Como usar com arquivos

Sintaxe geral:

```bash
confctl [file|-] [path] [--format <json|yaml|toml|env>]
```

Exemplos reais:

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

Se você omitir o `path`, ele imprime o arquivo inteiro normalizado em JSON:

```bash
confctl config.toml
```

## Como usar com curl (stdin)

Quando há pipe, o `confctl` lê `stdin` automaticamente:

```bash
curl -s https://api.github.com/users | confctl
curl -s https://api.github.com/users | confctl 0.login
```

Também funciona com `-` explícito, se você preferir:

```bash
curl -s https://api.github.com/users | confctl - 0.login
```

Se a origem não tiver extensão e a autodetecção ficar ambígua, force o formato:

```bash
curl -s https://api.github.com/users | confctl 0.login --format json
```

## Recursos úteis no dia a dia

- Sintaxe única para múltiplos formatos.
- Saída colorida no terminal.
- Mensagens de erro com contexto de caminho.
- Encode/decode Base64 com `--encode` e `--decode`.

## Onde isso ajuda de verdade

- Pipelines de CI/CD.
- Scripts de deploy.
- Inspeção rápida de respostas HTTP.
- Leitura de configurações sem depender de várias ferramentas.

Projeto: https://github.com/tarcisiomiranda/confctl
