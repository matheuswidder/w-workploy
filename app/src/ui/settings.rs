use rune::{Cursor, El, ScrollSpec, Transition, Tx, col, effect, el, icon, row, text};
use rune_kit::{Kit, Switch, TextInput};
use workploy::model::{CapacityTier, GithubRepoSummary, VpsServer};

use crate::gfx::MARK_PULSE_RADAR;
use crate::state::{AppState, ModalState, SettingsTab};
use crate::ui::components::{
    btn_danger, btn_ghost, btn_neutral, chip, field_box, header_card, pill_badge,
    primary_button, runner_connected_dot, surface_card, tool_chip,
};
use crate::ui::icons;
use crate::ui::theme::{UiTheme, ty};

const REPOS_PER_PAGE: usize = 8;
const REPO_PUBLIC_URL: &str = "https://github.com/matheuswidder/w-workploy";

pub fn view(s: &AppState, t: UiTheme, k: &Kit, narrow: bool) -> El {
    let subtitle = match s.settings_tab {
        SettingsTab::GitHub => {
            "Token PAT no Cofre do Windows, catálogo de repositórios, vínculos e vigia local."
        }
        SettingsTab::Vps => {
            "Gerencie servidores VPS Linux, provisionamento do workploy-runner e regras de limpeza."
        }
        SettingsTab::Appearance => {
            "Personalize o tema visual e o comportamento das animações da interface."
        }
        SettingsTab::About => {
            "Informações do Workploy, repositório oficial e verificação de novas versões."
        }
    };

    let mut hdr_row = row()
        .px(20.0)
        .py(16.0)
        .gap(14.0)
        .center_y()
        .justify_between()
        .flex_wrap()
        .child(
            col()
                .grow(1.0)
                .shrink()
                .min_w(180.0)
                .gap(4.0)
                .child(
                    text("Configurações")
                        .font(ty::H1)
                        .color(t.text_primary),
                )
                .child(
                    text(subtitle)
                        .font(ty::SMALL)
                        .color(t.text_muted)
                        .wrap(),
                ),
        );

    if matches!(s.settings_tab, SettingsTab::GitHub | SettingsTab::Vps) {
        hdr_row = hdr_row.child(
            primary_button(
                t,
                "btn-save-settings",
                "Salvar configurações",
                Some(&icons::CHECK_CIRCLE),
                34.0,
                |s: &mut AppState, tx: &mut Tx| s.save_settings(tx),
            )
            .help("Salva as preferências de GitHub, vigia local e intervalo de sondagem."),
        );
    }

    let hdr = header_card(t, "settings-hdr", 10.0, hdr_row);

    let tabs = row()
        .key("settings-tabs")
        .w_full()
        .gap(8.0)
        .flex_wrap()
        .child(
            chip(
                t,
                "tab-settings-github",
                "GitHub",
                s.settings_tab == SettingsTab::GitHub,
                |s: &mut AppState, _| s.settings_tab = SettingsTab::GitHub,
            )
            .help("Token PAT, busca de repositórios, vínculos e vigia local."),
        )
        .child(
            chip(
                t,
                "tab-settings-vps",
                "VPS & Motor",
                s.settings_tab == SettingsTab::Vps,
                |s: &mut AppState, _| s.settings_tab = SettingsTab::Vps,
            )
            .help("Cadastro de servidores VPS, provisionamento do workploy-runner e regras de limpeza."),
        )
        .child(
            chip(
                t,
                "tab-settings-appearance",
                "Aparência",
                s.settings_tab == SettingsTab::Appearance,
                |s: &mut AppState, _| s.settings_tab = SettingsTab::Appearance,
            )
            .help("Escolha entre tema Escuro/Claro e ative ou desative as animações (1x/0x)."),
        )
        .child(
            chip(
                t,
                "tab-settings-about",
                "Sobre",
                s.settings_tab == SettingsTab::About,
                |s: &mut AppState, _| s.settings_tab = SettingsTab::About,
            )
            .help("Versão instalada do Workploy, link do repositório e busca de atualizações."),
        );

    let mut scroll_col = col()
        .key("settings-scroll")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .scroll_edge_fade(16.0)
        .gap(14.0)
        .pb(16.0);

    match s.settings_tab {
        SettingsTab::GitHub => {
            scroll_col = scroll_col
                .child(github_and_bindings_card(s, t, k, narrow))
                .child(watcher_card(s, t, k));
        }
        SettingsTab::Vps => {
            if !s.snapshot.runner_sync_status.trim().is_empty() {
                scroll_col = scroll_col.child(sync_banner(s, t));
            }
            scroll_col = scroll_col.child(servers_card(s, t));
            if s.snapshot.provisioning_server_id.is_some()
                || !s.snapshot.provision_log.trim().is_empty()
            {
                scroll_col = scroll_col.child(provision_log_card(s, t));
            }
        }
        SettingsTab::Appearance => {
            scroll_col = scroll_col.child(appearance_card(s, t, k));
        }
        SettingsTab::About => {
            scroll_col = scroll_col.child(about_card(s, t));
        }
    }

    col()
        .key("settings-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(20.0)
        .pb(16.0)
        .gap(14.0)
        .transition(Transition::opacity())
        .child(hdr)
        .child(tabs)
        .child(scroll_col)
}

fn sync_banner(s: &AppState, t: UiTheme) -> El {
    let sync_lower = s.snapshot.runner_sync_status.to_lowercase();
    let sync_err = sync_lower.contains("falha")
        || sync_lower.contains("erro")
        || sync_lower.contains("não foi possível");
    row()
        .key("settings-sync-banner")
        .w_full()
        .px(14.0)
        .py(10.0)
        .gap(8.0)
        .center_y()
        .radius(8.0)
        .fill(if sync_err { t.danger_bg } else { t.surface_1 })
        .border(
            1.0,
            if sync_err {
                t.danger.alpha(0.35)
            } else {
                t.border_strong
            },
        )
        .child(
            icon(
                if sync_err {
                    &icons::X_CIRCLE
                } else {
                    &icons::SERVER
                },
                14.0,
            )
            .color(if sync_err { t.danger } else { t.accent }),
        )
        .child(
            text(s.snapshot.runner_sync_status.clone())
                .font(ty::SMALL)
                .color(if sync_err { t.danger } else { t.text_secondary })
                .grow(1.0)
                .shrink()
                .wrap(),
        )
}

fn github_and_bindings_card(s: &AppState, t: UiTheme, k: &Kit, narrow: bool) -> El {
    let has_token = s.snapshot.settings.has_github_token;
    let gh_user = &s.snapshot.settings.github_user;

    let status_line = if !s.snapshot.github_status.trim().is_empty() {
        s.snapshot.github_status.clone()
    } else if !gh_user.is_empty() {
        format!("Conectado como @{gh_user}")
    } else {
        "Informe seu Token Pessoal (PAT fine-grained) para listar repositórios e configurar disparos automáticos.".to_string()
    };

    let status_lower = status_line.to_lowercase();
    let status_is_err = !s.snapshot.github_testing
        && (s
            .snapshot
            .github_last_event
            .as_ref()
            .is_some_and(|(_, ok, msg)| !*ok && msg == &status_line)
            || status_lower.contains("erro")
            || status_lower.contains("401")
            || status_lower.contains("falha")
            || status_lower.contains("inválido")
            || status_lower.contains("expirado")
            || status_lower.contains("não foi possível"));
    let status_is_ok = !s.snapshot.github_testing
        && !status_is_err
        && (status_lower.starts_with("conectado") || (!gh_user.is_empty() && has_token));
    let status_color = if status_is_err {
        t.danger
    } else if status_is_ok {
        t.success
    } else {
        t.text_muted
    };

    let token_badge = if has_token {
        pill_badge(t, "Salvo no Cofre do Windows", t.success, t.success_bg)
    } else {
        pill_badge(t, "Token não configurado", t.text_muted, t.surface_2)
    };

    let placeholder = if has_token {
        "•••••••• (salvo no Cofre do Windows)"
    } else {
        "github_pat_... ou ghp_..."
    };

    let token_input = TextInput::new(&s.settings_draft.github_token)
        .label("Token de acesso do GitHub (PAT)")
        .placeholder(placeholder)
        .secure()
        .revealed(s.settings_draft.reveal_token)
        .width(Some(300.0))
        .on_edit(|s: &mut AppState| &mut s.settings_draft.github_token)
        .build(k)
        .cursor(Cursor::Text);

    let mut token_actions = row()
        .w_full()
        .gap(10.0)
        .center_y()
        .flex_wrap()
        .child(field_box(
            t,
            "Novo token GitHub (nunca exibido após salvo)",
            token_input,
        ))
        .child(
            col().pt(19.0).child(
                btn_neutral(
                    t,
                    "btn-toggle-token",
                    if s.settings_draft.reveal_token {
                        "Ocultar"
                    } else {
                        "Mostrar"
                    },
                    Some(if s.settings_draft.reveal_token {
                        k.glyph.eye_off
                    } else {
                        k.glyph.eye
                    }),
                    32.0,
                    |s: &mut AppState, _| {
                        s.settings_draft.reveal_token = !s.settings_draft.reveal_token;
                    },
                )
                .help("Mostra ou oculta o texto digitado neste campo antes de salvar."),
            ),
        )
        .child(
            col().pt(19.0).child(
                btn_neutral(
                    t,
                    "btn-github-test",
                    if s.snapshot.github_testing {
                        "Consultando GitHub..."
                    } else {
                        "Testar conexão"
                    },
                    Some(&icons::BRANCH),
                    32.0,
                    |s: &mut AppState, tx: &mut Tx| {
                        if !s.snapshot.github_testing {
                            s.test_github_and_list(tx);
                        }
                    },
                )
                .help("Valida o token PAT no GitHub, salva no Cofre do Windows e atualiza os repositórios."),
            ),
        );

    if has_token {
        token_actions = token_actions.child(
            col().pt(19.0).child(
                btn_ghost(
                    t,
                    "btn-github-clear",
                    "Remover token",
                    Some(&icons::TRASH),
                    32.0,
                    |s: &mut AppState, tx: &mut Tx| s.clear_github_token(tx),
                )
                .help("Apaga o token PAT salvo no Cofre do Windows e limpa a lista carregada."),
            ),
        );
    }

    let mut card_col = col()
        .px(18.0)
        .py(16.0)
        .gap(14.0)
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .flex_wrap()
                .gap(8.0)
                .child(
                    col()
                        .grow(1.0)
                        .shrink()
                        .min_w(180.0)
                        .gap(2.0)
                        .child(
                            text("Conexão GitHub & Repositórios")
                                .font(ty::H3)
                                .color(t.text_primary),
                        )
                        .child(
                            text(status_line)
                                .font(ty::CAPTION)
                                .color(status_color)
                                .wrap(),
                        ),
                )
                .child(token_badge),
        )
        .child(token_actions);

    if !s.snapshot.github_repos.is_empty() {
        let query = s.github_repo_search.text.trim().to_lowercase();
        let filtered: Vec<(usize, &GithubRepoSummary)> = s
            .snapshot
            .github_repos
            .iter()
            .enumerate()
            .filter(|(_, repo)| {
                if query.is_empty() {
                    return true;
                }
                repo.full_name.to_lowercase().contains(&query)
                    || repo.description.to_lowercase().contains(&query)
            })
            .collect();

        let total_filtered = filtered.len();
        let total_pages = total_filtered.div_ceil(REPOS_PER_PAGE).max(1);
        let page = s.github_repo_page.min(total_pages - 1);
        let start = page * REPOS_PER_PAGE;
        let end = (start + REPOS_PER_PAGE).min(total_filtered);
        let page_items = &filtered[start..end];

        let repo_search = TextInput::new(&s.github_repo_search)
            .placeholder("Buscar repositório por nome ou descrição...")
            .label("Buscar repositório")
            .leading_icon(k.glyph.search)
            .clear_button()
            .width(if narrow { None } else { Some(380.0) })
            .on_edit(|s: &mut AppState| &mut s.github_repo_search)
            .on_change(|s: &mut AppState, _| {
                s.github_repo_page = 0;
            })
            .on_action(|s: &mut AppState, _| {
                s.github_repo_search.clear();
                s.github_repo_page = 0;
            })
            .build(k)
            .key("gh-repo-search")
            .cursor(Cursor::Text);

        let repo_header = row()
            .w_full()
            .justify_between()
            .center_y()
            .flex_wrap()
            .gap(10.0)
            .child(
                text(format!(
                    "Repositórios disponíveis ({}/{})",
                    total_filtered,
                    s.snapshot.github_repos.len()
                ))
                .font(ty::SMALL_MED)
                .color(t.text_secondary),
            )
            .child(repo_search);

        let mut repos_grid = col().key("gh-repos-grid").w_full().gap(8.0);
        if page_items.is_empty() {
            repos_grid = repos_grid.child(
                row()
                    .key("gh-repos-empty-search")
                    .w_full()
                    .px(14.0)
                    .py(18.0)
                    .center()
                    .radius(8.0)
                    .fill(t.surface_2)
                    .border(1.0, t.border_subtle)
                    .child(
                        text("Nenhum repositório encontrado para essa busca.")
                            .font(ty::SMALL)
                            .color(t.text_muted),
                    ),
            );
        } else if narrow {
            for (orig_idx, repo) in page_items {
                repos_grid = repos_grid.child(repo_card(s, t, *orig_idx, repo));
            }
        } else {
            for pair in page_items.chunks(2) {
                let mut pair_row = row().w_full().gap(10.0).items_stretch();
                let (idx0, repo0) = pair[0];
                pair_row = pair_row.child(
                    repo_card(s, t, idx0, repo0)
                        .grow(1.0)
                        .shrink()
                        .min_w(0.0),
                );
                if let Some(&(idx1, repo1)) = pair.get(1) {
                    pair_row = pair_row.child(
                        repo_card(s, t, idx1, repo1)
                            .grow(1.0)
                            .shrink()
                            .min_w(0.0),
                    );
                } else {
                    pair_row = pair_row.child(el().grow(1.0).shrink().min_w(0.0));
                }
                repos_grid = repos_grid.child(pair_row);
            }
        }

        let repo_word = if total_filtered == 1 {
            "repositório"
        } else {
            "repositórios"
        };
        let can_prev = page > 0;
        let can_next = page + 1 < total_pages;

        let pagination = row()
            .key("gh-repos-pagination")
            .w_full()
            .justify_between()
            .center_y()
            .flex_wrap()
            .gap(8.0)
            .child(
                text(format!(
                    "Página {} de {} · {} {repo_word}",
                    page + 1,
                    total_pages,
                    total_filtered
                ))
                .font(ty::CAPTION)
                .color(t.text_muted),
            )
            .child(
                row()
                    .gap(8.0)
                    .center_y()
                    .child(
                        btn_neutral(
                            t,
                            "btn-gh-prev",
                            "Anterior",
                            Some(&icons::ARROW_LEFT),
                            30.0,
                            move |s: &mut AppState, _| {
                                if can_prev {
                                    s.github_repo_page = page.saturating_sub(1);
                                }
                            },
                        )
                        .disabled(!can_prev)
                        .help("Ir para a página anterior de repositórios."),
                    )
                    .child(
                        btn_neutral(
                            t,
                            "btn-gh-next",
                            "Próxima",
                            Some(&icons::CHEVRON_RIGHT),
                            30.0,
                            move |s: &mut AppState, _| {
                                if can_next {
                                    s.github_repo_page = page + 1;
                                }
                            },
                        )
                        .disabled(!can_next)
                        .help("Ir para a próxima página de repositórios."),
                    ),
            );

        card_col = card_col.child(
            col()
                .w_full()
                .gap(10.0)
                .child(repo_header)
                .child(repos_grid)
                .child(pagination),
        );
    }

    // Active repo bindings
    let mut bindings_col = col().w_full().gap(8.0);
    if s.snapshot.settings.repo_bindings.is_empty() {
        bindings_col = bindings_col.child(
            row()
                .w_full()
                .px(14.0)
                .py(14.0)
                .center()
                .radius(8.0)
                .fill(t.surface_2)
                .border(1.0, t.border_subtle)
                .child(
                    text("Nenhum vínculo ativo. Conecte seu GitHub acima ou clique em '+ Novo vínculo' para mapear um repositório.")
                        .font(ty::SMALL)
                        .color(t.text_muted)
                        .wrap(),
                ),
        );
    } else {
        for b in &s.snapshot.settings.repo_bindings {
            let target_name = s.server_label(b.target_server_id.as_deref());
            let wf_name = b
                .workflow_id
                .as_deref()
                .and_then(|wid| s.snapshot.workflows.iter().find(|w| w.id == wid))
                .map(|w| w.name.clone())
                .unwrap_or_else(|| "Automático (por gatilho)".into());

            let bid_edit = b.id.clone();
            let bid_del = b.id.clone();
            let srv_sync = b.target_server_id.clone();

            let mut row_actions = row().gap(6.0).center_y().flex_wrap();
            if let Some(sid) = srv_sync {
                row_actions = row_actions.child(
                    btn_neutral(
                        t,
                        format!("btn-sync-bind-{}", b.id),
                        "Sincronizar na VPS",
                        Some(&icons::REFRESH),
                        28.0,
                        move |s: &mut AppState, tx: &mut Tx| s.sync_bindings_to_vps(&sid, tx),
                    )
                    .help("Envia este vínculo e suas regras para o agente workploy-runner na VPS."),
                );
            }
            row_actions = row_actions
                .child(
                    btn_ghost(
                        t,
                        format!("btn-edit-bind-{}", b.id),
                        "Editar",
                        Some(&icons::EDIT),
                        28.0,
                        move |s: &mut AppState, _| s.open_edit_binding_modal(&bid_edit),
                    )
                    .help("Edita branch, servidor destino, modo de disparo e fluxo deste vínculo."),
                )
                .child(
                    btn_danger(
                        t,
                        format!("btn-del-bind-{}", b.id),
                        "Remover",
                        Some(&icons::TRASH),
                        28.0,
                        move |s: &mut AppState, tx: &mut Tx| s.delete_binding(&bid_del, tx),
                    )
                    .help("Remove este vínculo de repositório."),
                );

            bindings_col = bindings_col.child(
                row()
                    .key(format!("binding-row-{}", b.id))
                    .w_full()
                    .px(12.0)
                    .py(10.0)
                    .gap(10.0)
                    .center_y()
                    .justify_between()
                    .flex_wrap()
                    .radius(8.0)
                    .fill(t.surface_2)
                    .border(
                        1.0,
                        if b.enabled {
                            t.border_subtle
                        } else {
                            t.warning.alpha(0.3)
                        },
                    )
                    .child(
                        col()
                            .grow(1.0)
                            .shrink()
                            .min_w(190.0)
                            .gap(3.0)
                            .child(
                                row()
                                    .gap(8.0)
                                    .center_y()
                                    .flex_wrap()
                                    .child(
                                        text(format!("{} ({})", b.full_name, b.branch))
                                            .font(ty::BODY_MED)
                                            .color(t.text_primary),
                                    )
                                    .child(pill_badge(t, &target_name, t.accent, t.accent_soft))
                                    .child(pill_badge(
                                        t,
                                        b.trigger_mode.label(),
                                        t.text_secondary,
                                        t.surface_3,
                                    )),
                            )
                            .child(
                                text(format!(
                                    "Fluxo: {}  ·  {}{}",
                                    wf_name,
                                    if b.enabled { "" } else { "Pausado · " },
                                    if b.last_status.is_empty() {
                                        "Pronto para sincronizar"
                                    } else {
                                        b.last_status.as_str()
                                    }
                                ))
                                .font(ty::CAPTION)
                                .color(t.text_muted)
                                .wrap(),
                            ),
                    )
                    .child(row_actions),
            );
        }
    }

    card_col = card_col
        .child(el().w_full().h(t.hair).fill(t.border_subtle))
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .flex_wrap()
                .gap(8.0)
                .child(
                    text(format!(
                        "Vínculos ativos ({})",
                        s.snapshot.settings.repo_bindings.len()
                    ))
                    .font(ty::SMALL_MED)
                    .color(t.text_primary),
                )
                .child(
                    btn_neutral(
                        t,
                        "btn-new-binding",
                        "Novo vínculo",
                        Some(&icons::PLUS),
                        28.0,
                        |s: &mut AppState, _| s.open_new_binding_modal(None),
                    )
                    .help("Associa um repositório GitHub a uma VPS ou à máquina local."),
                ),
        )
        .child(bindings_col);

    surface_card(t, "settings-github-card", 12.0, card_col)
}

fn repo_card(s: &AppState, t: UiTheme, idx: usize, repo: &GithubRepoSummary) -> El {
    let repo_clone = repo.clone();
    let already_bound = s
        .snapshot
        .settings
        .repo_bindings
        .iter()
        .any(|b| b.full_name.eq_ignore_ascii_case(&repo.full_name));

    let desc_line = if repo.description.trim().is_empty() {
        "Sem descrição cadastrada no GitHub.".to_string()
    } else {
        repo.description.clone()
    };

    col()
        .key(format!("gh-repo-{idx}"))
        .px(12.0)
        .py(10.0)
        .gap(8.0)
        .justify_between()
        .radius(8.0)
        .fill(t.surface_2)
        .border(1.0, t.border_subtle)
        .child(
            col()
                .w_full()
                .gap(4.0)
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .center_y()
                        .justify_between()
                        .child(
                            text(repo.full_name.clone())
                                .font(ty::SMALL_MED)
                                .color(t.text_primary)
                                .grow(1.0)
                                .shrink()
                                .truncate(),
                        )
                        .child(pill_badge(
                            t,
                            if repo.private { "Privado" } else { "Público" },
                            if repo.private { t.accent } else { t.text_muted },
                            if repo.private { t.accent_soft } else { t.surface_3 },
                        )),
                )
                .child(
                    text(desc_line)
                        .font(ty::CAPTION)
                        .color(t.text_muted)
                        .truncate(),
                ),
        )
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .gap(8.0)
                .child(
                    text(format!("branch {}", repo.default_branch))
                        .font(ty::MONO_SM)
                        .color(t.text_secondary),
                )
                .child(
                    btn_neutral(
                        t,
                        format!("btn-bind-repo-{idx}"),
                        if already_bound {
                            "Vincular outro"
                        } else {
                            "Vincular"
                        },
                        Some(&icons::PLUS),
                        28.0,
                        move |s: &mut AppState, _| s.open_new_binding_modal(Some(&repo_clone)),
                    )
                    .help("Abre o vínculo deste repositório para escolher servidor, gatilho e fluxo."),
                ),
        )
}

fn watcher_card(s: &AppState, t: UiTheme, k: &Kit) -> El {
    surface_card(
        t,
        "settings-watch-card",
        12.0,
        col()
            .px(18.0)
            .py(16.0)
            .gap(12.0)
            .child(
                row()
                    .w_full()
                    .justify_between()
                    .center_y()
                    .flex_wrap()
                    .gap(10.0)
                    .child(
                        col()
                            .grow(1.0)
                            .shrink()
                            .min_w(180.0)
                            .gap(2.0)
                            .child(
                                text("Vigia de pasta local & Sondagem")
                                    .font(ty::H3)
                                    .color(t.text_primary),
                            )
                            .child(
                                text("Dispara fluxos configurados com gatilho 'Mudança na pasta' e define o intervalo mínimo de verificação.")
                                    .font(ty::CAPTION)
                                    .color(t.text_muted)
                                    .wrap(),
                            ),
                    )
                    .child(
                        Switch::new(s.settings_draft.watch_enabled)
                            .label("Ativar vigia local")
                            .on_toggle(|s: &mut AppState| {
                                s.settings_draft.watch_enabled = !s.settings_draft.watch_enabled;
                            })
                            .build(k)
                            .cursor(Cursor::Pointer)
                            .help("Monitora a pasta local abaixo e dispara fluxos com gatilho de mudança na pasta."),
                    ),
            )
            .child(
                row()
                    .w_full()
                    .gap(12.0)
                    .flex_wrap()
                    .child(field_box(
                        t,
                        "Caminho da pasta monitorada",
                        TextInput::new(&s.settings_draft.watch_folder)
                            .placeholder("C:\\Projects\\meu-repo")
                            .width(Some(320.0))
                            .on_edit(|s: &mut AppState| &mut s.settings_draft.watch_folder)
                            .build(k)
                            .cursor(Cursor::Text),
                    ))
                    .child(field_box(
                        t,
                        "Intervalo de verificação GitHub (s)",
                        TextInput::new(&s.settings_draft.poll_secs)
                            .placeholder("60")
                            .width(Some(140.0))
                            .on_edit(|s: &mut AppState| &mut s.settings_draft.poll_secs)
                            .build(k)
                            .cursor(Cursor::Text),
                    )),
            ),
    )
}

fn servers_card(s: &AppState, t: UiTheme) -> El {
    let local_card = row()
        .key("server-local-card")
        .w_full()
        .px(14.0)
        .py(12.0)
        .gap(12.0)
        .center_y()
        .justify_between()
        .flex_wrap()
        .radius(10.0)
        .fill(t.surface_2)
        .border(1.0, t.border_subtle)
        .child(
            row()
                .grow(1.0)
                .shrink()
                .min_w(180.0)
                .gap(10.0)
                .center_y()
                .child(runner_connected_dot(t, true))
                .child(
                    col()
                        .grow(1.0)
                        .shrink()
                        .min_w(0.0)
                        .gap(2.0)
                        .child(
                            text("Runner Local (Esta máquina)")
                                .font(ty::BODY_MED)
                                .color(t.text_primary),
                        )
                        .child(
                            text("Execução direta neste computador · Sem fila externa · Sempre disponível")
                                .font(ty::CAPTION)
                                .color(t.text_muted)
                                .wrap(),
                        ),
                ),
        )
        .child(pill_badge(t, "Ativo", t.success, t.success_bg));

    let mut vps_list = col().w_full().gap(10.0);
    if s.snapshot.settings.servers.is_empty() {
        vps_list = vps_list.child(
            row()
                .key("servers-vps-empty")
                .w_full()
                .px(14.0)
                .py(16.0)
                .center()
                .radius(10.0)
                .fill(t.surface_2)
                .border(1.0, t.border_subtle)
                .child(
                    text("Nenhuma VPS cadastrada ainda. Clique em '+ Adicionar VPS' para conectar um servidor Linux.")
                        .font(ty::SMALL)
                        .color(t.text_muted)
                        .wrap(),
                ),
        );
    } else {
        for srv in &s.snapshot.settings.servers {
            vps_list = vps_list.child(vps_server_item(s, t, srv));
        }
    }

    let purge_rule = &s.snapshot.settings.purge_rule;
    let purge_summary = if !purge_rule.enabled {
        "Limpeza automática: desativada".to_string()
    } else {
        let mut triggers = Vec::new();
        if purge_rule.every_n_runs > 0 {
            triggers.push(format!("a cada {} execuções", purge_rule.every_n_runs));
        }
        if purge_rule.max_size_mb > 0 {
            if purge_rule.max_size_mb >= 1024 && purge_rule.max_size_mb.is_multiple_of(1024) {
                triggers.push(format!("acima de {} GB", purge_rule.max_size_mb / 1024));
            } else {
                triggers.push(format!("acima de {} MB", purge_rule.max_size_mb));
            }
        }
        if triggers.is_empty() {
            "Limpeza automática: ativa (sem gatilho numérico)".to_string()
        } else {
            format!("Limpeza automática: {}", triggers.join(" ou "))
        }
    };

    let inner = col()
        .px(18.0)
        .py(16.0)
        .gap(12.0)
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .flex_wrap()
                .gap(10.0)
                .child(
                    col()
                        .grow(1.0)
                        .shrink()
                        .min_w(180.0)
                        .gap(2.0)
                        .child(
                            text("Servidores de execução (Local + Multi-VPS)")
                                .font(ty::H3)
                                .color(t.text_primary),
                        )
                        .child(
                            text("Cadastre servidores, provisione o agente workploy-runner 24/7 e controle a limpeza automática de disco.")
                                .font(ty::CAPTION)
                                .color(t.text_muted)
                                .wrap(),
                        )
                        .child(
                            text(purge_summary)
                                .font(ty::CAPTION)
                                .color(if purge_rule.enabled {
                                    t.success
                                } else {
                                    t.text_muted
                                }),
                        ),
                )
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .flex_wrap()
                        .child(
                            btn_neutral(
                                t,
                                "btn-purge-rule",
                                "Regra de limpeza (Purge)",
                                Some(&icons::SLIDERS),
                                32.0,
                                |s: &mut AppState, _| s.open_purge_rule_modal(),
                            )
                            .help("Define quando a VPS limpa automaticamente workspaces antigos e cache Docker."),
                        )
                        .child(
                            btn_neutral(
                                t,
                                "btn-add-vps",
                                "Adicionar VPS",
                                Some(&icons::PLUS),
                                32.0,
                                |s: &mut AppState, _| s.open_new_server_modal(),
                            )
                            .help("Cadastra um novo servidor Linux via SSH com alias, chave privada ou senha no Cofre."),
                        ),
                ),
        )
        .child(local_card)
        .child(vps_list);

    surface_card(t, "settings-vps-card", 12.0, inner)
}

fn vps_server_item(s: &AppState, t: UiTheme, srv: &VpsServer) -> El {
    let is_probing = s.snapshot.probing_server_id.as_deref() == Some(srv.id.as_str());
    let is_provisioning = s.snapshot.provisioning_server_id.as_deref() == Some(srv.id.as_str());
    let probe_ok = srv.last_probe.as_ref().is_some_and(|p| p.ok);
    let runner_online = srv
        .last_probe
        .as_ref()
        .is_some_and(|p| p.ok && (p.runner_active || srv.runner_installed))
        || probe_ok;

    let status_dot = if is_probing || is_provisioning {
        el().size(8.0, 8.0)
            .center()
            .child(
                effect(MARK_PULSE_RADAR)
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .radius(4.0)
                    .effect_bleed(10.0)
                    .effect_data([4.0, 10.0, 1.4, 0.0]),
            )
            .child(el().size(8.0, 8.0).radius(4.0).fill(t.info))
    } else {
        runner_connected_dot(t, runner_online)
    };

    let endpoint = if srv.user.trim().is_empty() {
        format!("{}:{}", srv.host, srv.port)
    } else {
        format!("{}@{}:{}", srv.user, srv.host, srv.port)
    };

    let mut badges = row().gap(6.0).center_y().flex_wrap().child(pill_badge(
        t,
        srv.auth.label(),
        t.text_secondary,
        t.surface_3,
    ));

    if srv.runner_installed {
        badges = badges.child(pill_badge(
            t,
            &format!("workploy-runner :{}", srv.runner_port),
            t.success,
            t.success_bg,
        ));
    }

    let id_probe = srv.id.clone();
    let id_prov = srv.id.clone();
    let id_sync = srv.id.clone();
    let id_purge = srv.id.clone();
    let id_edit = srv.id.clone();
    let id_del = srv.id.clone();
    let name_del = srv.name.clone();

    let actions = row()
        .gap(6.0)
        .center_y()
        .flex_wrap()
        .child(
            btn_neutral(
                t,
                format!("btn-probe-{}", srv.id),
                if is_probing {
                    "Testando..."
                } else {
                    "Testar conexão"
                },
                Some(&icons::SERVER),
                30.0,
                move |s: &mut AppState, tx: &mut Tx| {
                    if !is_probing {
                        s.probe_server(&id_probe, tx);
                    }
                },
            )
            .help("Conecta via SSH e mede vCPU, RAM, disco livre e ferramentas instaladas na VPS."),
        )
        .child(
            btn_neutral(
                t,
                format!("btn-provision-{}", srv.id),
                if is_provisioning {
                    "Provisionando..."
                } else {
                    "Provisionar Motor"
                },
                Some(&icons::BOLT),
                30.0,
                move |s: &mut AppState, _| s.open_provision_modal(&id_prov),
            )
            .help("Instala Git, Docker, Node, Rust e o serviço workploy-runner 24/7 via SSH."),
        )
        .child(
            btn_neutral(
                t,
                format!("btn-sync-{}", srv.id),
                "Sincronizar",
                Some(&icons::REFRESH),
                30.0,
                move |s: &mut AppState, tx: &mut Tx| s.sync_server_runs(&id_sync, tx),
            )
            .help("Sincroniza configurações e histórico de execuções com esta VPS."),
        )
        .child(
            btn_neutral(
                t,
                format!("btn-purge-vps-{}", srv.id),
                "Limpar agora",
                Some(&icons::TRASH),
                30.0,
                move |s: &mut AppState, tx: &mut Tx| s.purge_server(&id_purge, tx),
            )
            .help("Executa imediatamente a limpeza de workspaces antigos e cache Docker nesta VPS."),
        )
        .child(
            btn_ghost(
                t,
                format!("btn-edit-srv-{}", srv.id),
                "Editar",
                Some(&icons::EDIT),
                30.0,
                move |s: &mut AppState, _| s.open_edit_server_modal(&id_edit),
            )
            .help("Edita host, usuário, porta SSH ou método de autenticação desta VPS."),
        )
        .child(
            btn_danger(
                t,
                format!("btn-del-srv-{}", srv.id),
                "Excluir",
                Some(&icons::TRASH),
                30.0,
                move |s: &mut AppState, _| {
                    s.modal = ModalState::DeleteServerConfirm {
                        server_id: id_del.clone(),
                        server_name: name_del.clone(),
                    };
                },
            )
            .help("Remove este servidor e suas credenciais salvas no Cofre do Windows."),
        );

    let mut card = col()
        .key(format!("srv-card-{}", srv.id))
        .w_full()
        .px(14.0)
        .py(12.0)
        .gap(10.0)
        .radius(10.0)
        .fill(t.surface_2)
        .border(
            1.0,
            if runner_online {
                t.success.alpha(0.32)
            } else {
                t.border_subtle
            },
        )
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .flex_wrap()
                .gap(8.0)
                .child(
                    row()
                        .grow(1.0)
                        .shrink()
                        .min_w(180.0)
                        .gap(10.0)
                        .center_y()
                        .child(status_dot)
                        .child(
                            col()
                                .grow(1.0)
                                .shrink()
                                .min_w(0.0)
                                .gap(2.0)
                                .child(
                                    row()
                                        .gap(8.0)
                                        .center_y()
                                        .flex_wrap()
                                        .child(
                                            text(srv.name.clone())
                                                .font(ty::BODY_MED)
                                                .color(t.text_primary),
                                        )
                                        .child(
                                            text(endpoint)
                                                .font(ty::MONO_SM)
                                                .color(t.accent),
                                        ),
                                )
                                .child(badges),
                        ),
                ),
        );

    if let Some(probe) = &srv.last_probe
        && probe.ok
    {
        let (tier_fg, tier_bg) = match probe.tier {
            CapacityTier::Heavy | CapacityTier::Standard => (t.success, t.success_bg),
            CapacityTier::Light => (t.warning, t.warning_bg),
            CapacityTier::Insufficient => (t.danger, t.danger_bg),
        };
        let specs_line = format!(
            "{} ({})  ·  {} vCPU  ·  {:.1}/{:.1} GB RAM livres  ·  {} GB disco livre",
            probe.os,
            probe.arch,
            probe.vcpus,
            probe.ram_free_mb as f64 / 1024.0,
            probe.ram_total_mb as f64 / 1024.0,
            probe.disk_free_gb,
        );
        let tools_row = row()
            .w_full()
            .gap(6.0)
            .flex_wrap()
            .child(tool_chip(t, "Git", probe.has_git))
            .child(tool_chip(t, "Docker", probe.has_docker))
            .child(tool_chip(t, "Node", probe.has_node))
            .child(tool_chip(t, "pnpm", probe.has_pnpm))
            .child(tool_chip(t, "Rust", probe.has_rust))
            .child(tool_chip(
                t,
                if probe.runner_active {
                    "workploy-runner ativo"
                } else {
                    "workploy-runner"
                },
                probe.has_widder_runner,
            ));

        card = card.child(
            col()
                .w_full()
                .px(12.0)
                .py(10.0)
                .gap(8.0)
                .radius(8.0)
                .fill(t.surface_1)
                .border(1.0, t.border_subtle)
                .child(
                    row()
                        .w_full()
                        .justify_between()
                        .center_y()
                        .flex_wrap()
                        .gap(8.0)
                        .child(
                            text(specs_line)
                                .font(ty::CAPTION)
                                .color(t.text_secondary)
                                .grow(1.0)
                                .shrink()
                                .wrap(),
                        )
                        .child(pill_badge(
                            t,
                            &format!("Capacidade {}", probe.tier.label()),
                            tier_fg,
                            tier_bg,
                        )),
                )
                .child(tools_row),
        );
    }

    card.child(actions)
}

fn provision_log_card(s: &AppState, t: UiTheme) -> El {
    let tail: String = {
        let lines: Vec<&str> = s.snapshot.provision_log.lines().collect();
        let start = lines.len().saturating_sub(24);
        lines[start..].join("\n")
    };
    let active = s.snapshot.provisioning_server_id.is_some();
    surface_card(
        t,
        "settings-provision-log",
        12.0,
        col()
            .px(18.0)
            .py(14.0)
            .gap(10.0)
            .child(
                row()
                    .w_full()
                    .justify_between()
                    .center_y()
                    .child(
                        row()
                            .gap(8.0)
                            .center_y()
                            .child(
                                text("Log de provisionamento")
                                    .font(ty::H3)
                                    .color(t.text_primary),
                            )
                            .child(if active {
                                pill_badge(t, "Em andamento", t.info, t.info_bg)
                            } else {
                                pill_badge(t, "Última execução", t.text_secondary, t.surface_2)
                            }),
                    )
                    .child(btn_ghost(
                        t,
                        "btn-clear-prov-log",
                        "Limpar log",
                        None,
                        28.0,
                        |s: &mut AppState, _| {
                            s.snapshot.provision_log.clear();
                            if !s.demo_mode
                                && let Ok(mut g) = s.shared.lock()
                            {
                                g.provision_log.clear();
                            }
                        },
                    )),
            )
            .child(
                el().w_full()
                    .padding(12.0)
                    .radius(8.0)
                    .fill(t.surface_code)
                    .border(1.0, t.border_subtle)
                    .child(
                        text(if tail.is_empty() {
                            "Aguardando saída da VPS...".to_string()
                        } else {
                            tail
                        })
                        .font(ty::MONO_SM)
                        .color(rune::color::hex("#e2e8f0").unwrap_or(t.text_primary))
                        .wrap(),
                    ),
            ),
    )
}

fn appearance_card(s: &AppState, t: UiTheme, k: &Kit) -> El {
    let theme_row = row()
        .w_full()
        .px(14.0)
        .py(14.0)
        .gap(14.0)
        .center_y()
        .justify_between()
        .flex_wrap()
        .radius(10.0)
        .fill(t.surface_2)
        .border(1.0, t.border_subtle)
        .child(
            col()
                .grow(1.0)
                .shrink()
                .min_w(180.0)
                .gap(2.0)
                .child(text("Tema").font(ty::BODY_MED).color(t.text_primary))
                .child(
                    text("Alterne entre a paleta escura e a paleta clara do Workploy.")
                        .font(ty::CAPTION)
                        .color(t.text_muted)
                        .wrap(),
                ),
        )
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(
                    chip(
                        t,
                        "app-theme-dark",
                        "Escuro",
                        s.dark,
                        |s: &mut AppState, tx: &mut Tx| {
                            s.dark = true;
                            tx.color_motion(rune::tok::motion::CALM);
                        },
                    )
                    .help("Ativa o tema escuro de alto contraste."),
                )
                .child(
                    chip(
                        t,
                        "app-theme-light",
                        "Claro",
                        !s.dark,
                        |s: &mut AppState, tx: &mut Tx| {
                            s.dark = false;
                            tx.color_motion(rune::tok::motion::CALM);
                        },
                    )
                    .help("Ativa o tema claro para ambientes bem iluminados."),
                ),
        );

    let anim_row = row()
        .w_full()
        .px(14.0)
        .py(14.0)
        .gap(14.0)
        .center_y()
        .justify_between()
        .flex_wrap()
        .radius(10.0)
        .fill(t.surface_2)
        .border(1.0, t.border_subtle)
        .child(
            col()
                .grow(1.0)
                .shrink()
                .min_w(180.0)
                .gap(2.0)
                .child(
                    text("Animações")
                        .font(ty::BODY_MED)
                        .color(t.text_primary),
                )
                .child(
                    text("Mantenha as transições fluidas (1x) ou desligue o movimento (0x).")
                        .font(ty::CAPTION)
                        .color(t.text_muted)
                        .wrap(),
                ),
        )
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(
                    chip(
                        t,
                        "app-anim-1x",
                        "1x Ligado",
                        !s.reduce_motion,
                        |s: &mut AppState, tx: &mut Tx| {
                            s.set_animations_enabled(true, tx.clock() as f32);
                        },
                    )
                    .help("Mantém transições, indicadores de pulso e movimentos ativos em velocidade padrão (1x)."),
                )
                .child(
                    chip(
                        t,
                        "app-anim-0x",
                        "0x Desligado",
                        s.reduce_motion,
                        |s: &mut AppState, tx: &mut Tx| {
                            s.set_animations_enabled(false, tx.clock() as f32);
                        },
                    )
                    .help("Desativa animações contínuas e reduz deslocamentos na interface (0x)."),
                ),
        );

    let tray_row = row()
        .w_full()
        .px(14.0)
        .py(14.0)
        .gap(14.0)
        .center_y()
        .justify_between()
        .flex_wrap()
        .radius(10.0)
        .fill(t.surface_2)
        .border(1.0, t.border_subtle)
        .child(
            col()
                .grow(1.0)
                .shrink()
                .min_w(180.0)
                .gap(2.0)
                .child(
                    text("Minimizar para a bandeja ao fechar")
                        .font(ty::BODY_MED)
                        .color(t.text_primary),
                )
                .child(
                    text("Ao fechar a janela, o Workploy continua rodando na bandeja do sistema (System Tray) para manter o monitoramento ativo.")
                        .font(ty::CAPTION)
                        .color(t.text_muted)
                        .wrap(),
                ),
        )
        .child(
            Switch::new(s.snapshot.settings.minimize_to_tray)
                .label("Ativo")
                .on_toggle_tx(|s: &mut AppState, tx: &mut Tx| {
                    s.toggle_minimize_to_tray(tx);
                })
                .build(k)
                .key("app-minimize-to-tray")
                .cursor(Cursor::Pointer)
                .help("Mantém o aplicativo oculto na bandeja do sistema ao fechar a janela."),
        );

    surface_card(
        t,
        "settings-appearance-card",
        12.0,
        col()
            .px(18.0)
            .py(16.0)
            .gap(12.0)
            .child(
                col()
                    .gap(2.0)
                    .child(
                        text("Tema & Animações")
                            .font(ty::H3)
                            .color(t.text_primary),
                    )
                    .child(
                        text("Ajuste visual rápido da interface e comportamento da janela.")
                            .font(ty::CAPTION)
                            .color(t.text_muted),
                    ),
            )
            .child(theme_row)
            .child(anim_row)
            .child(tray_row),
    )
}

fn format_release_date_br(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return dt
            .with_timezone(&chrono::Local)
            .format("%d/%m/%Y %H:%M")
            .to_string();
    }
    trimmed.to_string()
}

fn about_card(s: &AppState, t: UiTheme) -> El {
    let current_ver = env!("CARGO_PKG_VERSION");

    let mut body = col()
        .px(18.0)
        .py(16.0)
        .gap(14.0)
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .flex_wrap()
                .gap(12.0)
                .child(
                    row()
                        .gap(12.0)
                        .center_y()
                        .child(
                            el().pixels(icons::brand_icon_for_theme(s.dark))
                                .w(42.0)
                                .h(42.0)
                                .radius(10.0),
                        )
                        .child(
                            col()
                                .gap(2.0)
                                .child(
                                    row()
                                        .gap(8.0)
                                        .center_y()
                                        .child(
                                            text("Workploy")
                                                .font(ty::H2)
                                                .color(t.text_primary),
                                        )
                                        .child(pill_badge(
                                            t,
                                            &format!("v{current_ver}"),
                                            t.accent,
                                            t.accent_soft,
                                        )),
                                )
                                .child(
                                    text("Orquestrador desktop self-hosted para CI/CD e deploy multi-VPS.")
                                        .font(ty::SMALL)
                                        .color(t.text_muted),
                                ),
                        ),
                )
                .child(
                    row()
                        .gap(8.0)
                        .center_y()
                        .flex_wrap()
                        .child(
                            btn_neutral(
                                t,
                                "btn-open-repo",
                                "Repositório no GitHub",
                                Some(&icons::EXTERNAL_LINK),
                                32.0,
                                |_: &mut AppState, _| AppState::open_external_url(REPO_PUBLIC_URL),
                            )
                            .help("Abre https://github.com/matheuswidder/w-workploy no navegador."),
                        )
                        .child(
                            primary_button(
                                t,
                                "btn-check-updates",
                                if s.snapshot.update_checking {
                                    "Buscando..."
                                } else {
                                    "Buscar atualizações"
                                },
                                Some(&icons::REFRESH),
                                32.0,
                                |s: &mut AppState, tx: &mut Tx| {
                                    if !s.snapshot.update_checking && !s.snapshot.update_downloading {
                                        s.check_for_updates(tx);
                                    }
                                },
                            )
                            .disabled(s.snapshot.update_checking || s.snapshot.update_downloading)
                            .opacity(if s.snapshot.update_checking || s.snapshot.update_downloading {
                                0.65
                            } else {
                                1.0
                            })
                            .help("Consulta as releases oficiais no GitHub para verificar se há versão mais nova."),
                        ),
                ),
        )
        .child(
            row()
                .w_full()
                .px(14.0)
                .py(10.0)
                .justify_between()
                .center_y()
                .flex_wrap()
                .gap(8.0)
                .radius(8.0)
                .fill(t.surface_2)
                .border(1.0, t.border_subtle)
                .child(
                    text("Repositório público oficial")
                        .font(ty::SMALL_MED)
                        .color(t.text_secondary),
                )
                .child(
                    text(REPO_PUBLIC_URL)
                        .font(ty::MONO_SM)
                        .color(t.accent),
                ),
        );

    if !s.snapshot.update_status.trim().is_empty() {
        let status_text = s.snapshot.update_status.clone();
        let lower = status_text.to_lowercase();
        let is_err = !s.snapshot.update_checking
            && (lower.contains("erro")
                || lower.contains("não foi possível")
                || lower.contains("falha"));
        let has_new = s
            .snapshot
            .latest_release
            .as_ref()
            .is_some_and(|r| r.has_update);

        body = body.child(
            row()
                .key("about-update-status")
                .w_full()
                .px(14.0)
                .py(10.0)
                .gap(8.0)
                .center_y()
                .radius(8.0)
                .fill(if is_err {
                    t.danger_bg
                } else if has_new {
                    t.success_bg
                } else {
                    t.surface_2
                })
                .border(
                    1.0,
                    if is_err {
                        t.danger.alpha(0.35)
                    } else if has_new {
                        t.success.alpha(0.35)
                    } else {
                        t.border_subtle
                    },
                )
                .child(
                    icon(
                        if is_err {
                            &icons::X_CIRCLE
                        } else if has_new {
                            &icons::CHECK_CIRCLE
                        } else {
                            &icons::INFO
                        },
                        14.0,
                    )
                    .color(if is_err {
                        t.danger
                    } else if has_new {
                        t.success
                    } else {
                        t.accent
                    }),
                )
                .child(
                    text(status_text)
                        .font(ty::SMALL)
                        .color(if is_err {
                            t.danger
                        } else if has_new {
                            t.success
                        } else {
                            t.text_primary
                        })
                        .grow(1.0)
                        .shrink()
                        .wrap(),
                ),
        );
    }

    if let Some(rel) = &s.snapshot.latest_release {
        let open_url = if !rel.installer_url.trim().is_empty() {
            rel.installer_url.clone()
        } else {
            rel.html_url.clone()
        };
        let github_url = if !rel.html_url.trim().is_empty() {
            rel.html_url.clone()
        } else {
            open_url.clone()
        };
        let pub_br = format_release_date_br(&rel.published_at);

        let action_area = if rel.has_update {
            let mut acts = row().gap(8.0).center_y().flex_wrap();
            if s.snapshot.update_downloading {
                acts = acts.child(
                    primary_button(
                        t,
                        "btn-update-install",
                        "Baixando atualização...",
                        Some(&icons::REFRESH),
                        30.0,
                        |_: &mut AppState, _| {},
                    )
                    .disabled(true)
                    .opacity(0.6)
                    .cursor(Cursor::Default)
                    .help("Download em andamento. O Workploy será atualizado e reiniciado ao concluir."),
                );
            } else {
                acts = acts.child(
                    primary_button(
                        t,
                        "btn-update-install",
                        format!("Baixar e instalar v{} agora", rel.version),
                        Some(&icons::BOLT),
                        30.0,
                        |s: &mut AppState, tx: &mut Tx| s.install_latest_update(tx),
                    )
                    .help("Baixa o instalador oficial, aplica a atualização silenciosamente e reinicia o Workploy."),
                );
            }
            if !github_url.is_empty() {
                let url_for_btn = github_url.clone();
                acts = acts.child(
                    btn_ghost(
                        t,
                        "btn-update-open",
                        "Ver no GitHub",
                        Some(&icons::EXTERNAL_LINK),
                        30.0,
                        move |_: &mut AppState, _| AppState::open_external_url(&url_for_btn),
                    )
                    .help("Abre a página desta release no GitHub no navegador."),
                );
            }
            acts
        } else if !open_url.is_empty() {
            btn_neutral(
                t,
                "btn-open-release",
                "Abrir release no GitHub",
                Some(&icons::EXTERNAL_LINK),
                30.0,
                move |_: &mut AppState, _| AppState::open_external_url(&open_url),
            )
            .help("Abre a página da release no navegador padrão.")
        } else {
            el()
        };

        let mut rel_col = col()
            .key("about-latest-release")
            .w_full()
            .px(14.0)
            .py(12.0)
            .gap(10.0)
            .radius(10.0)
            .fill(t.surface_2)
            .border(
                1.0,
                if rel.has_update {
                    t.success.alpha(0.35)
                } else {
                    t.border_subtle
                },
            )
            .child(
                row()
                    .w_full()
                    .justify_between()
                    .center_y()
                    .flex_wrap()
                    .gap(8.0)
                    .child(
                        row()
                            .gap(8.0)
                            .center_y()
                            .flex_wrap()
                            .child(
                                text(format!("Última release: {}", rel.tag_name))
                                    .font(ty::BODY_MED)
                                    .color(t.text_primary),
                            )
                            .child(if !pub_br.is_empty() {
                                text(format!("Publicada em {pub_br}"))
                                    .font(ty::CAPTION)
                                    .color(t.text_muted)
                            } else {
                                el()
                            }),
                    )
                    .child(action_area),
            );

        if rel.has_update && s.snapshot.update_downloading {
            let frac = s.snapshot.update_progress.clamp(0.0, 1.0);
            let pct = frac * 100.0;
            let status_msg = if s.snapshot.update_status.trim().is_empty() {
                format!("Baixando Workploy v{}...", rel.version)
            } else {
                s.snapshot.update_status.clone()
            };
            rel_col = rel_col.child(
                col()
                    .key("about-update-progress")
                    .w_full()
                    .gap(6.0)
                    .child(
                        row()
                            .role("progressbar")
                            .label("Progresso do download da atualização")
                            .w_full()
                            .h(8.0)
                            .radius(999.0)
                            .fill(t.surface_1)
                            .border(1.0, t.border_subtle)
                            .clip_soft(0.0)
                            .child(
                                el().key("about-update-progress-fill")
                                    .h_full()
                                    .radius(999.0)
                                    .fill(if frac >= 1.0 { t.success } else { t.accent })
                                    .w_frac(frac.max(0.02)),
                            ),
                    )
                    .child(
                        text(format!("{pct:.0}% · {status_msg}"))
                            .font(ty::CAPTION)
                            .color(t.text_secondary)
                            .wrap(),
                    ),
            );
        }

        if !rel.notes.trim().is_empty() {
            rel_col = rel_col.child(
                text(rel.notes.trim().to_string())
                    .font(ty::SMALL)
                    .color(t.text_secondary)
                    .wrap(),
            );
        }

        body = body.child(rel_col);
    }

    body = body.child(
        row()
            .w_full()
            .px(14.0)
            .py(10.0)
            .justify_between()
            .center_y()
            .flex_wrap()
            .gap(8.0)
            .radius(8.0)
            .fill(t.surface_2)
            .border(1.0, t.border_subtle)
            .child(
                text("Primeiras configurações e atalhos iniciais")
                    .font(ty::SMALL)
                    .color(t.text_muted),
            )
            .child(
                btn_ghost(
                    t,
                    "btn-reopen-onboarding",
                    "Reabrir guia inicial (Onboarding)",
                    Some(&icons::HOME),
                    30.0,
                    |s: &mut AppState, _| s.reopen_onboarding(),
                )
                .help("Abre novamente o guia inicial do Workploy."),
            ),
    );

    surface_card(t, "settings-about-card", 12.0, body)
}
