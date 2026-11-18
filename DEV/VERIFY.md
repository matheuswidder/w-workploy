# Verify

## Latest Verification

- Date: 2026-11-18
- Scope: Instalador Windows nativo (`widder-ci-setup`), pacote portátil `.zip`, build `--release` do `widder-ci`, Clippy, testes e smoke E2E isolado (instalação → abertura ao vivo → desinstalação limpa).

## Commands

- `cargo clippy --manifest-path app/Cargo.toml --no-deps -p widder-ci --all-targets -- -D warnings` → PASS (exit `0`, **0 warnings**)
- `cargo test --manifest-path app/Cargo.toml` → PASS (exit `0`, **29/29**: `24` lib + `5` bin)
- `cargo clippy --manifest-path installer/Cargo.toml --all-targets -- -D warnings` → PASS (exit `0`, **0 warnings**)
- `cargo test --manifest-path installer/Cargo.toml` → PASS (exit `0`, **8/8**)
- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-installer.ps1` → PASS (exit `0`; `dist/Widder-CI-Setup-0.1.0.exe` 11,3 MB + `dist/widder-ci-0.1.0-win64-portable.zip` 10,92 MB)
- Smoke E2E isolado em `Widder CI QA` (`--silent --dir ... --no-shortcuts --no-registry` → abertura ao vivo `Widder CI` → `--uninstall --silent`) → PASS (`INSTALL_EXIT=0`, `APP_ALIVE=True`, `DATA_WRITTEN=True`, `UNINSTALL_EXIT=0`, `DIR_EXISTS_AFTER_UNINSTALL=False`, `UNINST_EXISTS_AFTER_UNINSTALL=False`)

## Outcome

- Passed: Build release, Clippy (`app` e `installer`), 37/37 testes totais (`29` app + `8` installer), empacotamento `dist/` e ciclo completo de instalação/execução/desinstalação isolada.
- Failed: Nenhum.
- Pending: Instalação interativa com atalhos/registro HKCU reais no sistema do usuário e teste ao vivo contra `widder-vps` + PAT GitHub real.
