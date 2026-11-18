# Workploy

Orquestrador desktop em Rust e Rune para CI/CD e deploy self-hosted multi-VPS.

[![Versão](https://img.shields.io/badge/versão-0.1.3-d97706)](https://github.com/matheuswidder/w-workploy/releases)
[![Rust](https://img.shields.io/badge/Rust-2024-b45309?logo=rust)](app/Cargo.toml)
[![Plataforma](https://img.shields.io/badge/Windows-Win32%20%2F%20DX12-0078D6?logo=windows)](https://github.com/matheuswidder/w-workploy/releases)
[![Licença MIT](https://img.shields.io/badge/licença-MIT-16a34a)](LICENSE)

---

## O que é e por que usar

O Workploy centraliza pipelines de build, aprovação manual e deploy no seu desktop e nos seus próprios servidores Linux via SSH. Você conecta sua conta do GitHub, cadastra múltiplas VPSs, define fluxos encadeados e acompanha logs ao vivo em uma interface nativa escrita em Rust com o framework GPU [Rune](vendor/rune/).

Executar builds na própria infraestrutura elimina limites de minutos e custos de runners pagos em nuvem. Como os segredos ficam no Cofre do Windows e o agente `workploy-runner` roda como serviço `systemd` na VPS, seus deploys continuam operando 24/7 mesmo quando você fecha o computador.

## Principais recursos

| Recurso | Como funciona |
|---|---|
| **Conexão GitHub** | Salva o Personal Access Token (PAT) no Cofre do Windows (`Windows Credential Manager`), lista repositórios com busca e paginação em 2 colunas, detecta branches e fluxos automatizados no repositório e vincula cada projeto a uma VPS ou à máquina local. |
| **Multi-VPS via SSH e Telemetria** | Autentica por chave **Ed25519** (com gerador integrado via `ssh-keygen`), alias do `~/.ssh/config` ou senha armazenada no Cofre via `SSH_ASKPASS`. A sonda via SSH mede sistema operacional, arquitetura, vCPU, RAM livre/total, disco disponível e ferramentas (`Git`, `Docker`, `Node`, `pnpm`, `Rust`, `workploy-runner`), classificando a capacidade do servidor. |
| **Motor 24/7 (`workploy-runner`)** | Serviço `systemd` provisionado em 1 clique na VPS (`/etc/workploy/config.json` e `/var/lib/workploy/runs.json`). Aceita disparo por **Webhook na VPS**, **Sondagem 24/7 na VPS** (intervalo mínimo de 15 s) ou **Sondagem no Desktop** enquanto o aplicativo está aberto. |
| **Workflows encadeados** | Executa etapas em sequência com portões manuais de aprovação (`x-approval` na aba **Aprovações**) e encadeamento automático (`on_success` / `x-chain`) no formato **CI -> Portão de Aprovação -> Redeploy automático**, além de importação e exportação em YAML. |
| **Regra de Limpeza (Purge)** | Remove workspaces antigos, caches de build (`target`, `node_modules`, `.next`, `.turbo`, `.parcel-cache`, `.cache`, `dist`, `build`) e cache/imagens do Docker por quantidade máxima de execuções ou teto em **MB/GB**, com execução automática programada ou limpeza manual por servidor. |
| **Bandeja do sistema e atualizações** | Suporta operação na bandeja do sistema (System Tray) no desktop e consulta novas versões na aba **Sobre** usando a API oficial do GitHub Releases. |

## Arquitetura

```mermaid
flowchart LR
    subgraph Desktop["Desktop Workploy (Rust + Rune)"]
        UI["Interface GPU (Rune / wgpu)"]
        Tray["System Tray & Updater"]
        Vault["Cofre do Windows (PAT e Senhas SSH)"]
        LocalEngine["Motor Local & Cliente SSH"]
        UI --- Vault
        UI --- LocalEngine
        UI --- Tray
    end

    subgraph GitHub["GitHub"]
        API["REST API (/user, /user/repos, commits)"]
        Hook["Webhooks de Push"]
        Releases["GitHub Releases"]
    end

    subgraph VPS["VPS Linux (Multi-VPS)"]
        Runner["workploy-runner (systemd 24/7)"]
        Workspaces["/var/lib/workploy/workspaces"]
        Docker["Docker & Ferramentas de Build"]
        Purge["Purge (Workspaces + Docker Prune)"]
        Runner --> Workspaces
        Runner --> Docker
        Runner --> Purge
    end

    Desktop <-->|"HTTPS (PAT)"| API
    Tray -->|"Verifica vX.Y.Z"| Releases
    LocalEngine <-->|"SSH (Ed25519 / config / askpass)"| Runner
    Hook -->|"POST Webhook"| Runner
    Runner -->|"Sondagem 24/7 & git clone/fetch"| API
```

## Instalação (Windows)

Baixe o pacote da versão desejada na página de [Releases](https://github.com/matheuswidder/w-workploy/releases):

- **`Workploy-Setup-<versão>.exe`**: instalador nativo Win32 sem janela de console, com atalhos opcionais, registro em *Aplicativos Instalados* (`HKCU`) e desinstalador próprio.
- **`workploy-<versão>-win64-portable.zip`**: pacote portátil com `portable.flag`, pronto para extrair e executar em qualquer pasta sem alterar o registro do Windows.

## Instalação (Linux x86_64)

Baixe o pacote `workploy-<versão>-linux-x86_64.tar.gz` na página de [Releases](https://github.com/matheuswidder/w-workploy/releases) e execute o instalador incluído:

```bash
tar -xzf workploy-0.1.3-linux-x86_64.tar.gz
cd workploy-0.1.3-linux-x86_64
./install.sh
```

O script instala o binário em `~/.local/bin/workploy`, o ícone e o atalho `.desktop` no menu de aplicativos do usuário (e inclui `./uninstall.sh` para remoção limpa). Em sistemas Linux, [`app/src/vault.rs`](app/src/vault.rs) usa armazenamento em memória na sessão enquanto a integração com Secret Service entra nas próximas versões.

Para compilar e executar o cliente desktop manualmente no Linux:

```bash
cargo check --manifest-path app/Cargo.toml --all-targets
cargo run --manifest-path app/Cargo.toml
```

Para inspecionar a interface sem configurar servidores ou tokens, inicie o modo demonstração:

```bash
cargo run --manifest-path app/Cargo.toml -- --demo
```

## Compilação & Desenvolvimento local

### Pré-requisitos

- Toolchain Rust com edição 2024 (`rustup`, `cargo`); no Windows, alvo `x86_64-pc-windows-msvc`
- Cliente OpenSSH (`ssh` e `ssh-keygen`) disponível no `PATH` para conexões e geração de chaves Ed25519
- PowerShell (no Windows, para empacotar o instalador e o ZIP portátil)

O Cargo resolve as dependências gráficas e de interface a partir de [`vendor/rune/`](vendor/rune/) via `[patch.crates-io]` em [`app/Cargo.toml`](app/Cargo.toml).

### Verificação e testes

```powershell
cargo check --manifest-path app/Cargo.toml --all-targets
cargo check --manifest-path installer/Cargo.toml --all-targets

cargo clippy --manifest-path app/Cargo.toml --no-deps -p workploy --all-targets

cargo test --manifest-path app/Cargo.toml
cargo test --manifest-path installer/Cargo.toml
```

### Executar em desenvolvimento

```powershell
# Modo completo
cargo run --manifest-path app/Cargo.toml

# Modo demonstração com dados fictícios
cargo run --manifest-path app/Cargo.toml -- --demo
```

### Gerar instalador e pacote portátil (Windows)

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-installer.ps1
```

O script [`scripts/build-installer.ps1`](scripts/build-installer.ps1) compila `app` e `installer` em modo release, empacota `Workploy-Setup-<versão>.exe` e `workploy-<versão>-win64-portable.zip` em `dist/` e imprime os hashes SHA-256 de ambos os artefatos.

## Estrutura do projeto

| Caminho | Função |
|---|---|
| [`app/`](app/) | Aplicação desktop em Rust + Rune: UI, cofre de credenciais, integração GitHub, cliente SSH, telemetria, engine de pipelines e agente `workploy-runner` |
| [`app/src/`](app/src/) | Código-fonte de backend (`model.rs`, `vault.rs`, `vps.rs`, `github.rs`, `engine.rs`, `yaml.rs`) e interface (`ui/`, `state.rs`, `gfx.rs`, `demo.rs`) |
| [`app/assets/`](app/assets/) | Ícones da aplicação (`icon.png` e `icon.ico`) |
| [`installer/`](installer/) | Instalador nativo Win32 (`workploy-setup`) com compressão `flate2` e desinstalador |
| [`scripts/`](scripts/) | Automação de empacotamento e geração de hashes ([`build-installer.ps1`](scripts/build-installer.ps1)) |
| [`vendor/rune/`](vendor/rune/) | Framework UI GPU Rune e patches locais usados no build |
| [`DEV/`](DEV/) | Documentação técnica, arquitetura ([`ARCHITECTURE.md`](DEV/ARCHITECTURE.md)), contexto ([`CONTEXT.md`](DEV/CONTEXT.md)) e índice ([`INDEX.md`](DEV/INDEX.md)) |
| [`LICENSE`](LICENSE) | Texto integral da licença MIT |

## Licença

Este projeto usa a licença **MIT**. Leia o texto completo em [`LICENSE`](LICENSE).
