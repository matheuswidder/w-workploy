# Workploy

Orquestrador desktop em Rust e Rune para workflows de CI/CD e deploy self-hosted em múltiplas VPSs.

Você conecta repositórios do GitHub, cadastra seus servidores via SSH e acompanha builds, aprovações e deploys em uma interface nativa para Windows.

## Recursos

- **GitHub integrado**: armazena o Personal Access Token (PAT) no Cofre do Windows, lista repositórios com busca e paginação, e vincula cada projeto a uma branch e a um servidor de destino.
- **Múltiplas VPSs via SSH**: autentica por chave privada, `~/.ssh/config` ou senha salva no Cofre do Windows, com leitura de hardware do servidor (vCPU, RAM e espaço em disco).
- **Motor 24/7 na VPS (`workploy-runner`)**: serviço `systemd` que mantém os workflows ativos mesmo com o desktop desligado, com disparo por **Webhook** ou **Sondagem 24/7 na VPS**, além de **Sondagem no Desktop** quando o app está aberto.
- **Encadeamento de fluxos**: executa etapas em sequência no formato CI -> aprovação opcional -> Redeploy após a conclusão da etapa anterior.
- **Regra de limpeza (Purge)**: remove workspaces antigos e recursos Docker por quantidade máxima de execuções ou teto em MB/GB, com disparo programado e limpeza sob demanda.
- **Atualizações no app**: verifica novas versões na aba **Sobre** usando a API do GitHub Releases.

## Instalação (Windows)

Baixe o pacote da versão desejada na página de [Releases](https://github.com/matheuswidder/w-workploy/releases):

- **`Workploy-Setup-<versão>.exe`**: instalador nativo Win32 com registro no sistema e desinstalador.
- **`workploy-<versão>-win64-portable.zip`**: pacote portátil pronto para extrair e executar em qualquer pasta.

## Compilar localmente

### Pré-requisitos

- Toolchain Rust com alvo MSVC no Windows (`rustup`, `cargo`)
- PowerShell (para gerar o instalador e o ZIP portátil)

O Cargo resolve as dependências de interface em `vendor/rune/` pelos caminhos locais em [`app/Cargo.toml`](app/Cargo.toml).

### Verificação e testes

```powershell
cargo check --manifest-path app/Cargo.toml --all-targets
cargo check --manifest-path installer/Cargo.toml --all-targets

cargo test --manifest-path app/Cargo.toml
cargo test --manifest-path installer/Cargo.toml
```

### Executar o app em desenvolvimento

```powershell
cargo run --manifest-path app/Cargo.toml
```

### Gerar instalador e pacote portátil

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-installer.ps1
```

O script compila `app` e `installer` em modo release e grava os artefatos finais junto com os hashes SHA-256 em `dist/`.

## Estrutura do projeto

| Diretório | Função |
|---|---|
| [`app/`](app/) | Aplicação desktop em Rust + Rune (interface, cofre de credenciais, GitHub, SSH e motor de pipelines) |
| [`installer/`](installer/) | Instalador nativo Win32 para Windows |
| [`scripts/`](scripts/) | Script PowerShell de build e empacotamento (`build-installer.ps1`) |
| [`DEV/`](DEV/) | Documentação técnica, arquitetura, decisões e memória operacional do projeto |
