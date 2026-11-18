use rune::{El, ScrollSpec, Transition, Tx, col, effect, el, icon, row, text};
use workploy::model::{CapacityTier, VpsServer};

use crate::gfx::MARK_PULSE_RADAR;
use crate::state::{AppState, SettingsTab};
use crate::ui::components::{
    btn_ghost, btn_neutral, header_card, pill_badge, runner_connected_dot, surface_card,
    tool_chip,
};
use crate::ui::icons;
use crate::ui::theme::{UiTheme, ty};

pub fn view(s: &AppState, t: UiTheme) -> El {
    let hdr = header_card(
        t,
        "connections-hdr",
        10.0,
        row()
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
                        text("Conexões & Diagnóstico")
                            .font(ty::H1)
                            .color(t.text_primary),
                    )
                    .child(
                        text("Teste conexões SSH e GitHub, acompanhe capacidade (vCPU, RAM, disco, ferramentas e workploy-runner), veja logs e acione limpeza manual.")
                            .font(ty::SMALL)
                            .color(t.text_muted)
                            .wrap(),
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
                            "conn-hdr-purge-rule",
                            "Regra de limpeza (Purge)",
                            Some(&icons::SLIDERS),
                            34.0,
                            |s: &mut AppState, _| s.open_purge_rule_modal(),
                        )
                        .help("Configura a regra automática de limpeza de workspaces e cache Docker nas VPSs."),
                    )
                    .child(
                        btn_neutral(
                            t,
                            "conn-hdr-vps-settings",
                            "Gerenciar VPSs",
                            Some(&icons::SERVER),
                            34.0,
                            |s: &mut AppState, _| s.open_settings(SettingsTab::Vps),
                        )
                        .help("Abre Configurações → VPS & Motor para adicionar, editar ou remover servidores."),
                    ),
            ),
    );

    let mut scroll_col = col()
        .key("connections-scroll")
        .grow(1.0)
        .min_h(0.0)
        .w_full()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .scroll_edge_fade(16.0)
        .gap(14.0)
        .pb(16.0);

    if !s.snapshot.runner_sync_status.trim().is_empty() {
        let sync_lower = s.snapshot.runner_sync_status.to_lowercase();
        let sync_err = sync_lower.contains("falha")
            || sync_lower.contains("erro")
            || sync_lower.contains("não foi possível");
        scroll_col = scroll_col.child(
            row()
                .key("connections-sync-banner")
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
                ),
        );
    }

    scroll_col = scroll_col
        .child(github_quick_card(s, t))
        .child(vps_diagnostics_card(s, t))
        .child(logs_card(s, t))
        .child(recommendations_card(t));

    col()
        .key("connections-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(20.0)
        .pb(16.0)
        .gap(14.0)
        .transition(Transition::opacity())
        .child(hdr)
        .child(scroll_col)
}

fn github_quick_card(s: &AppState, t: UiTheme) -> El {
    let has_token = s.snapshot.settings.has_github_token;
    let gh_user = &s.snapshot.settings.github_user;
    let status_line = if !s.snapshot.github_status.trim().is_empty() {
        s.snapshot.github_status.clone()
    } else if !gh_user.is_empty() {
        format!("Conectado como @{gh_user}")
    } else if has_token {
        "Token salvo no Cofre do Windows. Clique em 'Testar conexão' para validar.".into()
    } else {
        "Nenhum token configurado ainda. Cadastre seu PAT em Configurações → GitHub.".into()
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

    surface_card(
        t,
        "conn-github-card",
        12.0,
        row()
            .px(18.0)
            .py(14.0)
            .gap(12.0)
            .center_y()
            .justify_between()
            .flex_wrap()
            .child(
                row()
                    .grow(1.0)
                    .shrink()
                    .min_w(200.0)
                    .gap(10.0)
                    .center_y()
                    .child(runner_connected_dot(t, status_is_ok))
                    .child(
                        col()
                            .grow(1.0)
                            .shrink()
                            .min_w(0.0)
                            .gap(3.0)
                            .child(
                                row()
                                    .gap(8.0)
                                    .center_y()
                                    .flex_wrap()
                                    .child(
                                        text("Conexão GitHub")
                                            .font(ty::H3)
                                            .color(t.text_primary),
                                    )
                                    .child(pill_badge(
                                        t,
                                        if has_token {
                                            "PAT no Cofre do Windows"
                                        } else {
                                            "Sem token"
                                        },
                                        if has_token { t.success } else { t.text_muted },
                                        if has_token { t.success_bg } else { t.surface_2 },
                                    )),
                            )
                            .child(
                                text(status_line)
                                    .font(ty::SMALL)
                                    .color(status_color)
                                    .wrap(),
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
                            "conn-test-github",
                            if s.snapshot.github_testing {
                                "Testando..."
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
                        .help("Valida o token PAT no GitHub e atualiza a lista de repositórios disponíveis."),
                    )
                    .child(
                        btn_ghost(
                            t,
                            "conn-open-gh-settings",
                            "Configurar GitHub",
                            Some(&icons::EDIT),
                            32.0,
                            |s: &mut AppState, _| s.open_settings(SettingsTab::GitHub),
                        )
                        .help("Abre Configurações → GitHub para editar token, buscar repositórios e gerenciar vínculos."),
                    ),
            ),
    )
}

fn vps_diagnostics_card(s: &AppState, t: UiTheme) -> El {
    let local_card = row()
        .key("conn-local-card")
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
                            text("Execução direta neste computador · Sempre pronto")
                                .font(ty::CAPTION)
                                .color(t.text_muted)
                                .wrap(),
                        ),
                ),
        )
        .child(pill_badge(t, "Online", t.success, t.success_bg));

    let mut vps_list = col().w_full().gap(10.0);
    if s.snapshot.settings.servers.is_empty() {
        vps_list = vps_list.child(
            row()
                .key("conn-vps-empty")
                .w_full()
                .px(14.0)
                .py(16.0)
                .justify_between()
                .center_y()
                .flex_wrap()
                .gap(10.0)
                .radius(10.0)
                .fill(t.surface_2)
                .border(1.0, t.border_subtle)
                .child(
                    text("Nenhuma VPS cadastrada ainda. Cadastre um servidor em Configurações → VPS & Motor.")
                        .font(ty::SMALL)
                        .color(t.text_muted)
                        .grow(1.0)
                        .shrink()
                        .wrap(),
                )
                .child(btn_neutral(
                    t,
                    "conn-empty-add-vps",
                    "Configurar VPS",
                    Some(&icons::PLUS),
                    30.0,
                    |s: &mut AppState, _| s.open_settings(SettingsTab::Vps),
                )),
        );
    } else {
        for srv in &s.snapshot.settings.servers {
            vps_list = vps_list.child(vps_diag_row(s, t, srv));
        }
    }

    let mut inner = col()
        .px(18.0)
        .py(16.0)
        .gap(12.0)
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .flex_wrap()
                .gap(8.0)
                .child(
                    col()
                        .gap(2.0)
                        .child(
                            text("Saúde das VPSs & Capacidade")
                                .font(ty::H3)
                                .color(t.text_primary),
                        )
                        .child(
                            text("Verifique latência SSH, vCPU, RAM, disco livre, ferramentas e estado do agente workploy-runner.")
                                .font(ty::CAPTION)
                                .color(t.text_muted)
                                .wrap(),
                        ),
                ),
        )
        .child(local_card)
        .child(vps_list);

    if let Some((ok, msg)) = &s.snapshot.ssh_test {
        inner = inner.child(
            row()
                .key("conn-ssh-test-banner")
                .w_full()
                .px(12.0)
                .py(9.0)
                .gap(8.0)
                .center_y()
                .radius(8.0)
                .fill(if *ok { t.success_bg } else { t.danger_bg })
                .border(
                    1.0,
                    if *ok {
                        t.success.alpha(0.35)
                    } else {
                        t.danger.alpha(0.35)
                    },
                )
                .child(
                    icon(
                        if *ok {
                            &icons::CHECK_CIRCLE
                        } else {
                            &icons::X_CIRCLE
                        },
                        14.0,
                    )
                    .color(if *ok { t.success } else { t.danger }),
                )
                .child(
                    text(msg.clone())
                        .font(ty::SMALL)
                        .color(if *ok { t.success } else { t.danger })
                        .grow(1.0)
                        .shrink()
                        .wrap(),
                ),
        );
    }

    surface_card(t, "conn-vps-card", 12.0, inner)
}

fn vps_diag_row(s: &AppState, t: UiTheme, srv: &VpsServer) -> El {
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
    let id_sync = srv.id.clone();
    let id_purge = srv.id.clone();
    let id_prov = srv.id.clone();

    let actions = row()
        .gap(6.0)
        .center_y()
        .flex_wrap()
        .child(
            btn_neutral(
                t,
                format!("conn-test-vps-{}", srv.id),
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
            .help("Testa a conexão SSH e coleta métricas de vCPU, RAM, disco e ferramentas da VPS."),
        )
        .child(
            btn_neutral(
                t,
                format!("conn-sync-vps-{}", srv.id),
                "Sincronizar",
                Some(&icons::REFRESH),
                30.0,
                move |s: &mut AppState, tx: &mut Tx| s.sync_server_runs(&id_sync, tx),
            )
            .help("Sincroniza regras, vínculos e execuções recentes com o agente workploy-runner da VPS."),
        )
        .child(
            btn_neutral(
                t,
                format!("conn-purge-vps-{}", srv.id),
                "Limpar agora",
                Some(&icons::TRASH),
                30.0,
                move |s: &mut AppState, tx: &mut Tx| s.purge_server(&id_purge, tx),
            )
            .help("Aciona imediatamente a limpeza de workspaces antigos e cache Docker nesta VPS."),
        )
        .child(
            btn_ghost(
                t,
                format!("conn-prov-vps-{}", srv.id),
                if is_provisioning {
                    "Provisionando..."
                } else {
                    "Provisionar Motor"
                },
                Some(&icons::BOLT),
                30.0,
                move |s: &mut AppState, _| s.open_provision_modal(&id_prov),
            )
            .help("Abre o assistente para instalar ou atualizar Git, Docker, Node, Rust e workploy-runner."),
        );

    let mut card = col()
        .key(format!("conn-srv-{}", srv.id))
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
                                .gap(3.0)
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
                )
                .child(actions),
        );

    if let Some(probe) = &srv.last_probe {
        if probe.ok {
            let (tier_fg, tier_bg) = match probe.tier {
                CapacityTier::Heavy | CapacityTier::Standard => (t.success, t.success_bg),
                CapacityTier::Light => (t.warning, t.warning_bg),
                CapacityTier::Insufficient => (t.danger, t.danger_bg),
            };
            let specs_line = format!(
                "{} ({})  ·  {} vCPU  ·  {:.1}/{:.1} GB RAM livres  ·  {} GB disco livre  ·  Verificado: {}",
                probe.os,
                probe.arch,
                probe.vcpus,
                probe.ram_free_mb as f64 / 1024.0,
                probe.ram_total_mb as f64 / 1024.0,
                probe.disk_free_gb,
                probe.checked_at
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
                    .child(tools_row)
                    .child(
                        text(probe.verdict.clone())
                            .font(ty::CAPTION)
                            .color(t.text_muted)
                            .wrap(),
                    ),
            );
        } else {
            card = card.child(
                row()
                    .w_full()
                    .px(12.0)
                    .py(8.0)
                    .gap(8.0)
                    .center_y()
                    .radius(8.0)
                    .fill(t.danger_bg)
                    .border(1.0, t.danger.alpha(0.35))
                    .child(icon(&icons::X_CIRCLE, 14.0).color(t.danger))
                    .child(
                        text(probe.verdict.clone())
                            .font(ty::CAPTION)
                            .color(t.danger)
                            .grow(1.0)
                            .shrink()
                            .wrap(),
                    ),
            );
        }
    } else {
        card = card.child(
            text("Ainda não inspecionada. Clique em 'Testar conexão' para coletar vCPU, RAM, disco e ferramentas.")
                .font(ty::CAPTION)
                .color(t.text_muted)
                .wrap(),
        );
    }

    card
}

fn logs_card(s: &AppState, t: UiTheme) -> El {
    let tail: String = {
        let lines: Vec<&str> = s.snapshot.provision_log.lines().collect();
        let start = lines.len().saturating_sub(24);
        lines[start..].join("\n")
    };
    let active = s.snapshot.provisioning_server_id.is_some();

    surface_card(
        t,
        "conn-provision-log",
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
                    .flex_wrap()
                    .gap(8.0)
                    .child(
                        row()
                            .gap(8.0)
                            .center_y()
                            .child(
                                text("Logs de provisionamento e sincronização")
                                    .font(ty::H3)
                                    .color(t.text_primary),
                            )
                            .child(if active {
                                pill_badge(t, "Em andamento", t.info, t.info_bg)
                            } else if !tail.is_empty() {
                                pill_badge(t, "Última execução", t.text_secondary, t.surface_2)
                            } else {
                                pill_badge(t, "Sem saída recente", t.text_muted, t.surface_2)
                            }),
                    )
                    .child(
                        btn_ghost(
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
                        )
                        .help("Limpa a saída de provisionamento exibida abaixo."),
                    ),
            )
            .child(
                el().w_full()
                    .padding(12.0)
                    .radius(8.0)
                    .fill(t.surface_code)
                    .border(1.0, t.border_subtle)
                    .child(
                        text(if tail.is_empty() {
                            if s.snapshot.runner_sync_status.trim().is_empty() {
                                "Nenhum log de provisionamento ou sincronização registrado nesta sessão.".to_string()
                            } else {
                                format!("Último status de sincronização:\n{}", s.snapshot.runner_sync_status)
                            }
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

fn recommendations_card(t: UiTheme) -> El {
    let mut rows = col().w_full().gap(8.0);
    for (tier, spec, usage) in CapacityTier::recommendations() {
        let (fg, bg) = match tier {
            CapacityTier::Light => (t.warning, t.warning_bg),
            CapacityTier::Standard => (t.accent, t.accent_soft),
            CapacityTier::Heavy => (t.success, t.success_bg),
            CapacityTier::Insufficient => (t.danger, t.danger_bg),
        };
        rows = rows.child(
            row()
                .w_full()
                .px(12.0)
                .py(9.0)
                .gap(12.0)
                .center_y()
                .flex_wrap()
                .radius(8.0)
                .fill(t.surface_2)
                .border(1.0, t.border_subtle)
                .child(pill_badge(t, tier.label(), fg, bg))
                .child(
                    text((*spec).to_string())
                        .font(ty::SMALL_MED)
                        .color(t.text_primary)
                        .min_w(200.0),
                )
                .child(
                    text((*usage).to_string())
                        .font(ty::CAPTION)
                        .color(t.text_muted)
                        .grow(1.0)
                        .shrink()
                        .wrap(),
                ),
        );
    }

    surface_card(
        t,
        "conn-vps-recommendations",
        12.0,
        col()
            .px(18.0)
            .py(14.0)
            .gap(10.0)
            .child(
                col()
                    .gap(2.0)
                    .child(
                        text("Faixas recomendadas de capacidade VPS")
                            .font(ty::H3)
                            .color(t.text_primary),
                    )
                    .child(
                        text("Referência usada pelo inspetor automático ao testar uma VPS.")
                            .font(ty::CAPTION)
                            .color(t.text_muted)
                            .wrap(),
                    ),
            )
            .child(rows),
    )
}
