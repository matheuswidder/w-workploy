# Active Handoff

This file should stay small. Refresh it after substantive work or run `orquestrador-maestro compact-worklog`.

## Snapshot

- Updated: 2026-11-18
- Read order: `INDEX.md` -> `HANDOFF.md` -> `CONTEXT.md` -> `SPECS/ACTIVE.md`
- Active spec: `SPECS/ACTIVE.md`
- Verification source: `VERIFY.md`
- Worklog archive: `HANDOFFS/WORKLOG_ARCHIVE.md`

## Latest Work

- Entry: 2026-11-18 - Instalador Windows nativo (`widder-ci-setup.exe`) e pacote portátil
- Spec: `DEV/SPECS/ACTIVE.md`
- Changed: `app/src/model.rs`, `app/Cargo.toml`, `installer/{Cargo.toml,build.rs,src/main.rs}`, `scripts/build-installer.ps1`, `dist/`
- Verified: `cargo clippy` (`0` warnings em `app` e `installer`), `cargo test` (`29/29` + `8/8`), `scripts/build-installer.ps1` (`exit 0`), e smoke E2E isolado de instalação, abertura ao vivo (`Widder CI`) e desinstalação limpa.
- Risks: Atalhos COM reais e entrada `HKCU\...\Uninstall\WidderCI` testados por código/unit tests (não gravados no perfil real durante QA); validação ao vivo com `widder-vps` e PAT GitHub ainda pendente.
- Next context: Executar `dist/Widder-CI-Setup-0.1.0.exe` para instalar no Windows (em `%LOCALAPPDATA%\Programs\Widder CI`) ou usar `dist/widder-ci-0.1.0-win64-portable.zip`.

## Recent Entries

- 2026-11-18: Instalador Windows nativo `Widder-CI-Setup-0.1.0.exe` + ZIP portátil gerados e validados em smoke E2E.
- 2026-11-17: Executado plano completo aprovado (1A + 2A + 3) com gate `SMOKE` aprovado pelo `qa-verifier`.
