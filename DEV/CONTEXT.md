# Current Context

## State

- Project: `w-workploy` (**Widder CI** — CI/CD self-hosted desktop em Rust + Rune)
- Active handoff: `HANDOFF.md`
- Active spec: `SPECS/ACTIVE.md`
- Update this file when commands, architecture, environment, risks, or active decisions change.

## Commands

- Check: `cargo check --manifest-path app/Cargo.toml --all-targets`
- Lint: `cargo clippy --manifest-path app/Cargo.toml --no-deps -p widder-ci --all-targets`
- Tests: `cargo test --manifest-path app/Cargo.toml` e `cargo test --manifest-path installer/Cargo.toml`
- Build installer + portable ZIP: `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-installer.ps1`
- Run (live): `cargo run --manifest-path app/Cargo.toml`
- Run (demo): `cargo run --manifest-path app/Cargo.toml -- --demo`
- Snap headless: `cargo run --manifest-path app/Cargo.toml -- --snap <out.png> --scene <scene> --size 1280x820`
- DEV gate: `orquestrador-maestro check-dev-gates --project-path C:\Projects\w-workploy`

## Constraints And Risks

- Windows MSVC usa `[patch.crates-io]` locais em `vendor/rune/vendor/` (não remover).
- `vendor/rune/` gera avisos próprios de dependências; o gate de 0 warnings vale para o pacote `widder-ci`.
- Segredos (GitHub PAT e senhas SSH) devem residir exclusivamente no Windows Credential Manager, nunca em `data/*.json`.

## Next Context

- Concluir Frentes 1 (Backend/Vault/Multi-VPS/GitHub) e 2 (UI Rose+Amber Gold, Ícone, Zero Terminal, Onboarding, Multi-VPS/Repo Mapping) e validar com Frente 3 (QA Independente).
