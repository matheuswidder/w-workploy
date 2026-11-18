# Active Spec - w-workploy (Widder CI)

## Goal

Evoluir o **Widder CI** (`C:\Projects\w-workploy`) conforme o plano aprovado (**1A + 2A + 3**):
1. **Arquitetura Real Local + Multi-VPS + GitHub (`1A`)**: cadastro de múltiplos servidores VPS (além de execução Local), cofre seguro no Windows Credential Manager (zero segredos em JSON), diagnóstico real de hardware/capacidade com recomendações de VPS (Leve/Padrão/Pesada), provisionamento 1-clique com agente `widder-runner` via `systemd` na VPS (webhooks/polling 24/7 mesmo com o Windows desligado), conexão GitHub com listagem de repositórios e mapeamento `Repo → VPS / Local`.
2. **Linguagem Visual Unificada no Rune (`2A`)**: paleta coesa **Rose Gold + Amber Gold** sobre base escura neutra (`#090a0e`/`#0d0f14`), botões primários `LiquidMetal::gold()` com preenchimento total (sem placa escura interna), `Pulse` restrito a micro-indicadores de 8px e `Electric` âmbar/dourado sutil apenas em portões de aprovação.
3. **Polimento Desktop Windows + Onboarding (`3`)**: eliminar janela de console/terminal no Windows (`windows_subsystem = "windows"` + `CREATE_NO_WINDOW`), adicionar ícone próprio (`.png` via `rune::set_app_icon` + `.ico` via `winresource`) e substituir abertura vazia por onboarding em 3 passos fora do `--demo`.

## In Scope

- `app/Cargo.toml`, `app/build.rs`, `app/assets/icon.png`, `app/assets/icon.ico`
- `app/src/vault.rs`, `app/src/vps.rs`, `app/src/github.rs`, `app/src/model.rs`, `app/src/engine.rs`, `app/src/yaml.rs`, `app/src/lib.rs`
- `app/src/main.rs`, `app/src/gfx.rs`, `app/src/state.rs`, `app/src/demo.rs`, `app/src/ui/*.rs`
- Testes unitários de backend/UI, `cargo clippy --no-deps -p widder-ci --all-targets` com 0 warnings, snapshots `--snap` em `1280x820` e `900x700`

## Out Of Scope

- Commit ou push automático para GitHub
- Provisionamento destrutivo automático em VPS de produção sem disparo explícito do usuário

## Acceptance

- `cargo check --manifest-path app/Cargo.toml --all-targets` passa com exit 0.
- `cargo clippy --manifest-path app/Cargo.toml --no-deps -p widder-ci --all-targets` tem **0 warnings** em `widder-ci`.
- `cargo test --manifest-path app/Cargo.toml` passa todos os testes de lib e UI.
- Segredos (PAT GitHub, senhas SSH) nunca são gravados em texto puro em `data/*.json`.
- Subprocessos no Windows usam `CREATE_NO_WINDOW` (`0x08000000`) e binário GUI não abre console.
- Screenshots `--snap` (`1280x820` e `900x700`) inspecionados visualmente com paleta Rose Gold + Amber Gold harmônica, onboarding claro e telas Multi-VPS/GitHub sem clipping/sobreposição.

## Constraints

- Seguir `C:/Users/teco_/.orquestrador/rules.md`, `maestro.md` e `PERSISTENCE.md`.
- Não alterar arquivos de `vendor/rune/`.

## Verification Plan

- `cargo check --manifest-path app/Cargo.toml --all-targets`
- `cargo clippy --manifest-path app/Cargo.toml --no-deps -p widder-ci --all-targets`
- `cargo test --manifest-path app/Cargo.toml`
- `cargo run --manifest-path app/Cargo.toml -- --snap ... --scene ...` em `1280x820` e `900x700`
- `orquestrador-maestro check-dev-gates --project-path C:\Projects\w-workploy`

## Status

- State: smoke_verified
- Owner: orquestrador (integrado com backend-builder, frontend-taste e qa-verifier)
- Last updated: 2026-11-17
