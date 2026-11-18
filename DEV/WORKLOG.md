# Worklog

Record a short summary here after substantive work.

Use `HANDOFF.md` for the current snapshot and `HANDOFFS/WORKLOG_ARCHIVE.md` for older entries after compaction.

## 2026-11-18 - Instalador Windows nativo (`widder-ci-setup.exe`) e pacote portátil

- Spec: `DEV/SPECS/ACTIVE.md`
- Changed: `app/src/model.rs` (`resolve_data_dir` para uso de `%LOCALAPPDATA%\Widder CI\data` em instalações e `portable.flag` no modo portátil), `app/Cargo.toml` (`strip = "symbols"`, `lto = "thin"`), `installer/{Cargo.toml,build.rs,src/main.rs}`, `scripts/build-installer.ps1`, artefatos em `dist/`
- Why: Gerar instalador Windows autônomo (`Widder-CI-Setup-0.1.0.exe`) com GUI Win32 pt-BR sem console, payload comprimido via `flate2`, atalhos COM (`IShellLinkW`), registro em Programas e Recursos (`HKCU`), desinstalador `--uninstall` com auto-remoção limpa e pacote portátil `.zip`.
- Verified: `cargo clippy` (`0` warnings em `widder-ci` e `widder-ci-setup` com `-D warnings`), `cargo test` (`29/29` em `app` + `8/8` em `installer`), `scripts/build-installer.ps1` (`exit 0`), e teste E2E isolado em `Widder CI QA` (instalação silenciosa, abertura ao vivo da janela `Widder CI` com escrita em `WIDDER_CI_DATA`, e desinstalação removendo `uninstall.exe` e a pasta por completo).
- Risks: Atalhos reais no Menu Iniciar/Desktop e chave HKCU foram verificados por código + testes unitários (pulados no smoke E2E com `--no-shortcuts --no-registry` para não sujar o ambiente).
- Next context: Executar `dist/Widder-CI-Setup-0.1.0.exe` para instalar no Windows ou extrair `dist/widder-ci-0.1.0-win64-portable.zip`.

## 2026-11-17 - Multi-VPS, Cofre Windows, Provisionador systemd, Visual Rose/Amber Gold, Ícone, Zero Terminal e Onboarding

- Spec: `DEV/SPECS/ACTIVE.md`
- Changed: `app/Cargo.toml`, `app/build.rs`, `app/assets/icon.{png,ico}`, `app/src/{lib,model,vault,vps,github,engine,yaml,main,gfx,state,demo}.rs`, `app/src/ui/{mod,theme,components,sidebar,runs,runner,approvals,detail,modals}.rs`
- Why: Retomar e concluir o plano aprovado (1A + 2A + 3): arquitetura real Local + Multi-VPS com cofre Windows Credential Manager, provisionamento `widder-runner` via `systemd`, mapeamento GitHub `Repo → VPS/Local`, paleta unificada Rose Gold + Amber Gold (sem placa preta nem conflito azul elétrico), ícone próprio, zero terminal no Windows e onboarding de 3 passos.
- Verified: `cargo check --all-targets` (exit 0), `cargo clippy --no-deps -p widder-ci --all-targets` (0 warnings em `widder-ci`), `cargo test` (28/28 verdes: 23 lib + 5 UI), auditoria de segredos/CREATE_NO_WINDOW/`windows_subsystem` e inspeção visual das capturas `--snap` em `1280x820`, `1280x2200` e `900x700` pelo `qa-verifier`.
- Risks: Acesso real ao Windows Credential Manager e provisionamento/webhook ao vivo na VPS (`widder-vps`) e GitHub ainda dependem de teste interativo do usuário no app desktop.
- Next context: Abrir o app (`cargo run --manifest-path app/Cargo.toml` ou instalado), cadastrar/inspecionar a `widder-vps`, salvar o PAT GitHub no cofre e validar o provisionamento 1-clique + webhook/polling ao vivo.
