# Fiz um CLI para parar de sofrer lendo arquivo de config em pipeline

Tive que fazer leitura de arquivos de configuração e, pra quem me conhece, eu não curto muito ficar montando comando complexo em bash.

Também não queria depender de baixar várias ferramentas separadas (`jq`, `yq`, parser de TOML) só pra extrair alguns campos em script.

Então decidi usar Rust e construir um binário único com `serde` pra ler JSON, YAML, TOML e `.env`.

No fim ficou exatamente o que eu queria: um executável leve, simples de usar em pipeline e script pequeno, sem aquela bagunça de vários binários diferentes pra cada formato.

Neste fim de semana também implementei suporte melhor pra uso com `curl`, então dá pra ler resposta JSON direto no pipe com highlight no terminal no estilo do `jq`.

Também funciona direto com o stdout do `curl` para visualizar a resposta com highlight:

```bash
curl -s https://api.github.com/users | confctl
```

Curti bastante o resultado e decidi deixar open source.

Se quiser testar:

```bash
curl -fsSL https://raw.githubusercontent.com/tarcisiomiranda/confctl/main/install.sh | bash
```

```bash
curl -s https://api.github.com/users | confctl 0.login
```

Projeto: https://github.com/tarcisiomiranda/confctl

---

# I built a CLI to stop suffering with config parsing in pipelines

I had to read configuration files and, if you know me, I really do not enjoy building complex Bash commands for that.

I also did not want to depend on installing multiple tools (`jq`, `yq`, TOML parsers) just to extract a few values in scripts.

So I decided to use Rust and build a single binary with `serde` to read JSON, YAML, TOML, and `.env`.

In the end, it became exactly what I wanted: a lightweight executable, easy to use in pipelines and small scripts, without juggling multiple binaries for each file format.

This weekend I also improved support for `curl` workflows, so now I can pipe JSON responses directly and get terminal highlight similar to `jq`.

It also works directly from `curl` stdout if you just want to inspect the full response with highlight:

```bash
curl -s https://api.github.com/users | confctl
```

I liked the result a lot, so I decided to open source it.

If you want to try it:

```bash
curl -fsSL https://raw.githubusercontent.com/tarcisiomiranda/confctl/main/install.sh | bash
```

```bash
curl -s https://api.github.com/users | confctl 0.login
```

Project: https://github.com/tarcisiomiranda/confctl
