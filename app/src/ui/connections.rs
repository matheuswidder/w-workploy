use rune::color::Color;
use rune::{El, Icon, ScrollSpec, Transition, Tx, col, effect, el, icon, row, text};
use workploy::model::{TriggerMode, VpsServer};

use crate::gfx::MARK_PULSE_RADAR;
use crate::state::{
    AppState, ConnectionsHealth, SettingsTab, vps_health_status, vps_ram_used_pct,
};
use crate::ui::components::{
    btn_ghost, btn_neutral, header_card, health_pulse_dot, pill_badge, runner_connected_dot,
    surface_card, tool_chip,
};
use crate::ui::icons;
use crate::ui::theme::{UiTheme, ty};

fn usage_bar_color(t: UiTheme, pct: u8) -> Color {
    if pct >= 80 {
        t.danger
    } else if pct >= 65 {
        t.warning
    } else {
        t.success
    }
}

pub fn view(s: &AppState, t: UiTheme, main_w: f32) -> El {
    let any_probing =
        s.snapshot.probing_server_id.is_some() || !s.probe_queue.is_empty();

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
                        text("Visão analítica de disponibilidade, uso de recursos das VPSs e automação GitHub.")
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
                            "conn-test-all-vps",
                            if any_probing {
                                "Testando VPSs..."
                            } else {
                                "Testar todas as VPSs"
                            },
                            Some(&icons::SERVER),
                            34.0,
                            |s: &mut AppState, tx: &mut Tx| {
                                if s.snapshot.probing_server_id.is_none()
                                    && s.probe_queue.is_empty()
                                {
                                    s.probe_all_servers(tx);
                                }
                            },
                        )
                        .help("Executa o diagnóstico SSH em todas as VPSs cadastradas e atualiza os KPIs."),
                    )
                    .child(
                        btn_neutral(
                            t,
                            "conn-test-github",
                            if s.snapshot.github_testing {
                                "Testando GitHub..."
                            } else {
                                "Testar GitHub"
                            },
                            Some(&icons::BRANCH),
                            34.0,
                            |s: &mut AppState, tx: &mut Tx| {
                                if !s.snapshot.github_testing {
                                    s.test_github_and_list(tx);
                                }
                            },
                        )
                        .help("Valida o token PAT salvo e atualiza o estado da conta GitHub."),
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

    scroll_col = scroll_col.child(kpi_strip(s, t, main_w));

    if let Some(alert_banner) = health_alert_banner(s, t) {
        scroll_col = scroll_col.child(alert_banner);
    }

    scroll_col = scroll_col.child(vps_telemetry_section(s, t, main_w));

    if has_recent_activity(s) {
        scroll_col = scroll_col.child(activity_log_card(s, t));
    }

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

struct KpiSpec<'a> {
    key: &'a str,
    label: &'a str,
    icon: &'static Icon,
    value: String,
    use_h1: bool,
    subtext: String,
    bar_frac: f32,
    bar_color: Color,
}

fn kpi_card(t: UiTheme, spec: KpiSpec<'_>) -> El {
    let clamped = spec.bar_frac.clamp(0.0, 1.0);
    let fill_frac = if clamped > 0.0 { clamped.max(0.04) } else { 0.0 };

    surface_card(
        t,
        spec.key,
        10.0,
        col()
            .px(14.0)
            .py(12.0)
            .gap(8.0)
            .justify_between()
            .child(
                col()
                    .w_full()
                    .gap(5.0)
                    .child(
                        row()
                            .w_full()
                            .gap(6.0)
                            .center_y()
                            .child(icon(spec.icon, 12.0).color(t.text_muted))
                            .child(
                                text(spec.label.to_string())
                                    .font(ty::EYEBROW)
                                    .color(t.text_muted)
                                    .grow(1.0)
                                    .shrink()
                                    .truncate(),
                            ),
                    )
                    .child(
                        text(spec.value)
                            .font(if spec.use_h1 { ty::H1 } else { ty::H2 })
                            .color(t.text_primary)
                            .truncate(),
                    )
                    .child(
                        text(spec.subtext)
                            .font(ty::CAPTION)
                            .color(t.text_muted)
                            .wrap(),
                    ),
            )
            .child(
                row()
                    .w_full()
                    .h(5.0)
                    .radius(999.0)
                    .fill(t.surface_2)
                    .border(1.0, t.border_subtle)
                    .clip_soft(0.0)
                    .child(
                        el().h_full()
                            .radius(999.0)
                            .fill(spec.bar_color)
                            .w_frac(fill_frac),
                    ),
            ),
    )
}

fn kpi_strip(s: &AppState, t: UiTheme, main_w: f32) -> El {
    let servers = &s.snapshot.settings.servers;
    let total_vps = servers.len();
    let online_probes: Vec<_> = servers
        .iter()
        .filter_map(|srv| srv.last_probe.as_ref().filter(|p| p.ok))
        .collect();
    let online_vps = online_probes.len();
    let active_runners = online_probes.iter().filter(|p| p.runner_active).count();

    let servers_val = format!("{online_vps} / {total_vps}");
    let servers_sub = if total_vps == 0 {
        "Nenhuma VPS cadastrada".to_string()
    } else {
        let word = if active_runners == 1 {
            "workploy-runner ativo"
        } else {
            "workploy-runners ativos"
        };
        format!("{active_runners} {word}")
    };
    let servers_frac = if total_vps == 0 {
        0.0
    } else {
        online_vps as f32 / total_vps as f32
    };
    let servers_color = if total_vps == 0 {
        t.text_muted
    } else if online_vps == total_vps {
        t.success
    } else if online_vps > 0 {
        t.warning
    } else {
        t.danger
    };

    let (cpu_val, cpu_sub, cpu_frac, cpu_color) = if online_probes.is_empty() {
        (
            "—".to_string(),
            "Aguardando inspeção".to_string(),
            0.0,
            t.text_muted,
        )
    } else {
        let sum_cpu: u64 = online_probes.iter().map(|p| p.cpu_usage_pct as u64).sum();
        let avg_cpu = (sum_cpu as f64 / online_probes.len() as f64)
            .round()
            .clamp(0.0, 100.0) as u8;
        let total_vcpus: u32 = online_probes.iter().map(|p| p.vcpus).sum();
        (
            format!("{avg_cpu}%"),
            format!("{total_vcpus} vCPU em {online_vps} VPS(s)"),
            avg_cpu as f32 / 100.0,
            usage_bar_color(t, avg_cpu),
        )
    };

    let ram_probes: Vec<_> = online_probes
        .iter()
        .copied()
        .filter(|p| p.ram_total_mb > 0)
        .collect();
    let (ram_val, ram_sub, ram_frac, ram_color) = if ram_probes.is_empty() {
        (
            "—".to_string(),
            "Sem leitura de RAM".to_string(),
            0.0,
            t.text_muted,
        )
    } else {
        let total_mb: u64 = ram_probes.iter().map(|p| p.ram_total_mb).sum();
        let used_mb: u64 = ram_probes
            .iter()
            .map(|p| p.ram_total_mb.saturating_sub(p.ram_free_mb))
            .sum();
        let avg_ram = ((used_mb.saturating_mul(100)) / total_mb.max(1)).min(100) as u8;
        (
            format!("{avg_ram}%"),
            format!(
                "{:.1} / {:.1} GB",
                used_mb as f64 / 1024.0,
                total_mb as f64 / 1024.0
            ),
            avg_ram as f32 / 100.0,
            usage_bar_color(t, avg_ram),
        )
    };

    let (disk_val, disk_sub, disk_frac, disk_color) = if online_probes.is_empty() {
        (
            "—".to_string(),
            "Sem leitura de disco".to_string(),
            0.0,
            t.text_muted,
        )
    } else {
        let sum_disk: u64 = online_probes.iter().map(|p| p.disk_used_pct as u64).sum();
        let avg_disk = (sum_disk as f64 / online_probes.len() as f64)
            .round()
            .clamp(0.0, 100.0) as u8;
        let free_gb: u64 = online_probes.iter().map(|p| p.disk_free_gb).sum();
        (
            format!("{avg_disk}%"),
            format!("{free_gb} GB livres"),
            avg_disk as f32 / 100.0,
            usage_bar_color(t, avg_disk),
        )
    };

    let gh_user = s.snapshot.settings.github_user.trim();
    let has_token = s.snapshot.settings.has_github_token;
    let gh_connected = has_token && !gh_user.is_empty();
    let gh_val = if !gh_user.is_empty() {
        format!("@{gh_user}")
    } else if has_token {
        "Token salvo".to_string()
    } else {
        "Desconectado".to_string()
    };

    let active_bindings: Vec<_> = s
        .snapshot
        .settings
        .repo_bindings
        .iter()
        .filter(|b| b.enabled)
        .collect();
    let webhook_cnt = active_bindings
        .iter()
        .filter(|b| b.trigger_mode == TriggerMode::WebhookOnVps)
        .count();
    let poll_cnt = active_bindings
        .iter()
        .filter(|b| {
            matches!(
                b.trigger_mode,
                TriggerMode::PollOnVps | TriggerMode::PollOnDesktop
            )
        })
        .count();
    let bind_word = if active_bindings.len() == 1 {
        "vínculo ativo"
    } else {
        "vínculos ativos"
    };
    let gh_sub = format!(
        "{} {bind_word} ({webhook_cnt} Webhook · {poll_cnt} Sondagem)",
        active_bindings.len()
    );
    let (gh_frac, gh_color) = if gh_connected {
        (1.0, t.success)
    } else if has_token {
        (0.5, t.warning)
    } else {
        (0.0, t.text_muted)
    };

    let cards = vec![
        kpi_card(
            t,
            KpiSpec {
                key: "kpi-servers-online",
                label: "SERVIDORES ONLINE",
                icon: &icons::SERVER,
                value: servers_val,
                use_h1: true,
                subtext: servers_sub,
                bar_frac: servers_frac,
                bar_color: servers_color,
            },
        ),
        kpi_card(
            t,
            KpiSpec {
                key: "kpi-cpu-avg",
                label: "CPU MÉDIA",
                icon: &icons::ACTIVITY,
                value: cpu_val,
                use_h1: true,
                subtext: cpu_sub,
                bar_frac: cpu_frac,
                bar_color: cpu_color,
            },
        ),
        kpi_card(
            t,
            KpiSpec {
                key: "kpi-ram-avg",
                label: "RAM MÉDIA",
                icon: &icons::BOLT,
                value: ram_val,
                use_h1: true,
                subtext: ram_sub,
                bar_frac: ram_frac,
                bar_color: ram_color,
            },
        ),
        kpi_card(
            t,
            KpiSpec {
                key: "kpi-disk-avg",
                label: "DISCO MÉDIO",
                icon: &icons::SHIELD,
                value: disk_val,
                use_h1: true,
                subtext: disk_sub,
                bar_frac: disk_frac,
                bar_color: disk_color,
            },
        ),
        kpi_card(
            t,
            KpiSpec {
                key: "kpi-github-auto",
                label: "GITHUB & AUTOMAÇÃO",
                icon: &icons::BRANCH,
                value: gh_val,
                use_h1: false,
                subtext: gh_sub,
                bar_frac: gh_frac,
                bar_color: gh_color,
            },
        ),
    ];

    if main_w >= 860.0 {
        let mut r = row().key("conn-kpi-grid").w_full().gap(10.0).items_stretch();
        for c in cards {
            r = r.child(c.grow(1.0).shrink().min_w(0.0));
        }
        r
    } else if main_w >= 520.0 {
        let mut grid = col().key("conn-kpi-grid").w_full().gap(10.0);
        for chunk in cards.chunks(2) {
            let mut r = row().w_full().gap(10.0).items_stretch();
            for c in chunk {
                r = r.child(c.clone().grow(1.0).shrink().min_w(0.0));
            }
            if chunk.len() == 1 {
                r = r.child(el().grow(1.0).shrink().min_w(0.0));
            }
            grid = grid.child(r);
        }
        grid
    } else {
        let mut grid = col().key("conn-kpi-grid").w_full().gap(10.0);
        for c in cards {
            grid = grid.child(c);
        }
        grid
    }
}

fn health_alert_banner(s: &AppState, t: UiTheme) -> Option<El> {
    let mut alert_entries: Vec<(ConnectionsHealth, String)> = Vec::new();
    for srv in &s.snapshot.settings.servers {
        let (h, alerts) = vps_health_status(srv);
        if h != ConnectionsHealth::Healthy && !alerts.is_empty() {
            alert_entries.push((h, format!("{}: {}", srv.name, alerts.join(" · "))));
        }
    }
    if alert_entries.is_empty() {
        return None;
    }

    let has_danger = alert_entries
        .iter()
        .any(|(h, _)| *h == ConnectionsHealth::Danger);
    let (bg, border, fg, title, ic) = if has_danger {
        (
            t.danger_bg,
            t.danger.alpha(0.42),
            t.danger,
            "Atenção: falha de conexão ou agente inativo em VPS",
            &icons::X_CIRCLE,
        )
    } else {
        (
            t.warning_bg,
            t.warning.alpha(0.45),
            t.warning,
            "Alerta de capacidade (CPU, RAM ou Disco acima de 80%)",
            &icons::BOLT,
        )
    };

    let mut alert_lines = col().w_full().gap(3.0);
    for (_, line) in &alert_entries {
        alert_lines = alert_lines.child(
            text(line.clone())
                .font(ty::SMALL)
                .color(t.text_primary)
                .wrap(),
        );
    }

    Some(
        row()
            .key("connections-health-alert-banner")
            .w_full()
            .px(14.0)
            .py(12.0)
            .gap(10.0)
            .radius(10.0)
            .fill(bg)
            .border(1.0, border)
            .child(icon(ic, 16.0).color(fg))
            .child(
                col()
                    .grow(1.0)
                    .shrink()
                    .min_w(0.0)
                    .gap(4.0)
                    .child(text(title).font(ty::BODY_MED).color(fg))
                    .child(alert_lines),
            ),
    )
}

fn vps_telemetry_section(s: &AppState, t: UiTheme, main_w: f32) -> El {
    let mut header = row()
        .w_full()
        .justify_between()
        .center_y()
        .flex_wrap()
        .gap(8.0)
        .child(
            col()
                .gap(2.0)
                .child(
                    text("Telemetria por VPS")
                        .font(ty::H3)
                        .color(t.text_primary),
                )
                .child(
                    text("Consumo em tempo real da última inspeção SSH, ferramentas e ações rápidas.")
                        .font(ty::CAPTION)
                        .color(t.text_muted)
                        .wrap(),
                ),
        );

    if let Some((ok, msg)) = &s.snapshot.ssh_test {
        header = header.child(pill_badge(
            t,
            msg,
            if *ok { t.success } else { t.danger },
            if *ok { t.success_bg } else { t.danger_bg },
        ));
    }

    let mut list = col().w_full().gap(10.0);
    if s.snapshot.settings.servers.is_empty() {
        list = list.child(
            row()
                .key("conn-vps-empty")
                .w_full()
                .px(14.0)
                .py(18.0)
                .justify_between()
                .center_y()
                .flex_wrap()
                .gap(10.0)
                .radius(10.0)
                .fill(t.surface_2)
                .border(1.0, t.border_subtle)
                .child(
                    text("Nenhuma VPS cadastrada ainda. Adicione seus servidores em Configurações → VPS & Motor.")
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
            list = list.child(vps_telemetry_card(s, t, srv, main_w));
        }
    }

    surface_card(
        t,
        "conn-vps-card",
        12.0,
        col().px(18.0).py(16.0).gap(12.0).child(header).child(list),
    )
}

fn mini_usage_bar(t: UiTheme, label: &str, detail: String, pct: u8) -> El {
    let color = usage_bar_color(t, pct);
    let frac = (pct as f32 / 100.0).clamp(0.0, 1.0);
    let fill_frac = if pct > 0 { frac.max(0.04) } else { 0.0 };

    col()
        .gap(5.0)
        .px(10.0)
        .py(8.0)
        .radius(8.0)
        .fill(t.surface_1)
        .border(1.0, t.border_subtle)
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .gap(6.0)
                .child(
                    text(label.to_string())
                        .font(ty::CAPTION)
                        .color(t.text_secondary),
                )
                .child(
                    text(detail)
                        .font(ty::MONO_SM)
                        .color(t.text_primary)
                        .truncate(),
                ),
        )
        .child(
            row()
                .w_full()
                .h(6.0)
                .radius(999.0)
                .fill(t.surface_2)
                .border(1.0, t.border_subtle)
                .clip_soft(0.0)
                .child(
                    el().h_full()
                        .radius(999.0)
                        .fill(color)
                        .w_frac(fill_frac),
                ),
        )
}

fn vps_telemetry_card(s: &AppState, t: UiTheme, srv: &VpsServer, main_w: f32) -> El {
    let is_probing = s.snapshot.probing_server_id.as_deref() == Some(srv.id.as_str());
    let is_provisioning = s.snapshot.provisioning_server_id.as_deref() == Some(srv.id.as_str());
    let (srv_health, srv_alerts) = vps_health_status(srv);
    let probe_ok = srv.last_probe.as_ref().is_some_and(|p| p.ok);
    let runner_online = srv
        .last_probe
        .as_ref()
        .is_some_and(|p| p.ok && (!p.has_widder_runner || p.runner_active));

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
    } else if srv.last_probe.is_some() && srv_health != ConnectionsHealth::Healthy {
        health_pulse_dot(t, srv_health)
    } else {
        runner_connected_dot(t, runner_online)
    };

    let endpoint = if srv.user.trim().is_empty() {
        format!("{}:{}", srv.host, srv.port)
    } else {
        format!("{}@{}:{}", srv.user, srv.host, srv.port)
    };

    let (health_label, health_fg, health_bg) = match &srv.last_probe {
        None => ("Não inspecionada", t.text_muted, t.surface_3),
        Some(p) if !p.ok => ("Offline", t.danger, t.danger_bg),
        Some(_) => match srv_health {
            ConnectionsHealth::Danger => ("Offline", t.danger, t.danger_bg),
            ConnectionsHealth::Warning => ("Recurso >=80%", t.warning, t.warning_bg),
            ConnectionsHealth::Healthy => ("Saudável", t.success, t.success_bg),
        },
    };

    let sys_subtitle = match &srv.last_probe {
        Some(p) if p.ok => format!(
            "{} ({}) · {} vCPU · Verificado {}",
            p.os, p.arch, p.vcpus, p.checked_at
        ),
        Some(p) if !p.checked_at.trim().is_empty() => {
            format!("Última tentativa: {}", p.checked_at)
        }
        _ => "Aguardando primeira inspeção SSH".to_string(),
    };

    let id_probe = srv.id.clone();
    let id_purge = srv.id.clone();

    let actions = row()
        .gap(6.0)
        .center_y()
        .flex_wrap()
        .child(pill_badge(t, health_label, health_fg, health_bg))
        .child(
            btn_neutral(
                t,
                format!("conn-test-vps-{}", srv.id),
                if is_probing {
                    "Testando..."
                } else {
                    "Testar agora"
                },
                Some(&icons::SERVER),
                28.0,
                move |s: &mut AppState, tx: &mut Tx| {
                    if !is_probing {
                        s.probe_server(&id_probe, tx);
                    }
                },
            )
            .help("Testa a conexão SSH e atualiza CPU, RAM, disco e ferramentas desta VPS."),
        )
        .child(
            btn_neutral(
                t,
                format!("conn-purge-vps-{}", srv.id),
                "Limpar agora",
                Some(&icons::TRASH),
                28.0,
                move |s: &mut AppState, tx: &mut Tx| s.purge_server(&id_purge, tx),
            )
            .help("Executa imediatamente a limpeza manual de workspaces antigos e cache Docker nesta VPS."),
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
            match srv_health {
                ConnectionsHealth::Danger => t.danger.alpha(0.45),
                ConnectionsHealth::Warning => t.warning.alpha(0.45),
                ConnectionsHealth::Healthy => {
                    if probe_ok && runner_online {
                        t.success.alpha(0.30)
                    } else {
                        t.border_subtle
                    }
                }
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
                        .min_w(200.0)
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
                                .child(
                                    text(sys_subtitle)
                                        .font(ty::CAPTION)
                                        .color(t.text_muted)
                                        .wrap(),
                                ),
                        ),
                )
                .child(actions),
        );

    if let Some(probe) = &srv.last_probe {
        if probe.ok {
            let ram_pct = vps_ram_used_pct(probe);
            let ram_used_gb = probe.ram_total_mb.saturating_sub(probe.ram_free_mb) as f64 / 1024.0;
            let ram_total_gb = probe.ram_total_mb as f64 / 1024.0;

            let cpu_bar = mini_usage_bar(
                t,
                "CPU",
                format!("{}%", probe.cpu_usage_pct),
                probe.cpu_usage_pct,
            );
            let ram_bar = mini_usage_bar(
                t,
                "RAM",
                format!("{ram_pct}% ({ram_used_gb:.1}/{ram_total_gb:.1} GB)"),
                ram_pct,
            );
            let disk_bar = mini_usage_bar(
                t,
                "Disco",
                format!("{}% ({} GB livres)", probe.disk_used_pct, probe.disk_free_gb),
                probe.disk_used_pct,
            );

            let bars = if main_w >= 580.0 {
                row()
                    .w_full()
                    .gap(10.0)
                    .items_stretch()
                    .child(cpu_bar.grow(1.0).shrink().min_w(0.0))
                    .child(ram_bar.grow(1.0).shrink().min_w(0.0))
                    .child(disk_bar.grow(1.0).shrink().min_w(0.0))
            } else {
                col()
                    .w_full()
                    .gap(8.0)
                    .child(cpu_bar)
                    .child(ram_bar)
                    .child(disk_bar)
            };

            let tools_row = row()
                .w_full()
                .gap(6.0)
                .flex_wrap()
                .child(tool_chip(t, "Git", probe.has_git))
                .child(tool_chip(t, "Docker", probe.has_docker))
                .child(tool_chip(t, "Node", probe.has_node))
                .child(tool_chip(t, "Rust", probe.has_rust))
                .child(tool_chip(
                    t,
                    if probe.runner_active {
                        "workploy-runner ativo"
                    } else {
                        "workploy-runner"
                    },
                    probe.has_widder_runner && probe.runner_active,
                ));

            card = card.child(bars).child(tools_row);

            if !srv_alerts.is_empty() {
                let is_danger = srv_health == ConnectionsHealth::Danger;
                let (abg, abd, afg, aic) = if is_danger {
                    (
                        t.danger_bg,
                        t.danger.alpha(0.35),
                        t.danger,
                        &icons::X_CIRCLE,
                    )
                } else {
                    (
                        t.warning_bg,
                        t.warning.alpha(0.4),
                        t.warning,
                        &icons::BOLT,
                    )
                };
                card = card.child(
                    row()
                        .key(format!("conn-srv-alert-{}", srv.id))
                        .w_full()
                        .px(10.0)
                        .py(7.0)
                        .gap(8.0)
                        .center_y()
                        .radius(6.0)
                        .fill(abg)
                        .border(1.0, abd)
                        .child(icon(aic, 13.0).color(afg))
                        .child(
                            text(srv_alerts.join(" · "))
                                .font(ty::CAPTION)
                                .color(afg)
                                .grow(1.0)
                                .shrink()
                                .wrap(),
                        ),
                );
            }
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
            text("Ainda não inspecionada. Clique em 'Testar agora' para coletar CPU, RAM, disco e ferramentas.")
                .font(ty::CAPTION)
                .color(t.text_muted)
                .wrap(),
        );
    }

    card
}

fn has_recent_activity(s: &AppState) -> bool {
    s.snapshot.provisioning_server_id.is_some()
        || !s.snapshot.provision_log.trim().is_empty()
        || !s.snapshot.runner_sync_status.trim().is_empty()
}

fn activity_log_card(s: &AppState, t: UiTheme) -> El {
    let tail: String = {
        let lines: Vec<&str> = s.snapshot.provision_log.lines().collect();
        let start = lines.len().saturating_sub(20);
        lines[start..].join("\n")
    };
    let active = s.snapshot.provisioning_server_id.is_some();
    let sync = s.snapshot.runner_sync_status.trim();

    let body_text = if !tail.is_empty() && !sync.is_empty() {
        format!("{sync}\n\n{tail}")
    } else if !tail.is_empty() {
        tail
    } else {
        sync.to_string()
    };

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
                                text("Log de atividade")
                                    .font(ty::H3)
                                    .color(t.text_primary),
                            )
                            .child(if active {
                                pill_badge(t, "Em andamento", t.info, t.info_bg)
                            } else {
                                pill_badge(t, "Recente", t.text_secondary, t.surface_2)
                            }),
                    )
                    .child(
                        btn_ghost(
                            t,
                            "btn-clear-prov-log",
                            "Limpar",
                            None,
                            28.0,
                            |s: &mut AppState, _| {
                                s.snapshot.provision_log.clear();
                                s.snapshot.runner_sync_status.clear();
                                if !s.demo_mode
                                    && let Ok(mut g) = s.shared.lock()
                                {
                                    g.provision_log.clear();
                                    g.runner_sync_status.clear();
                                }
                            },
                        )
                        .help("Limpa o log de atividade recente exibido nesta sessão."),
                    ),
            )
            .child(
                el().w_full()
                    .padding(12.0)
                    .radius(8.0)
                    .fill(t.surface_code)
                    .border(1.0, t.border_subtle)
                    .child(
                        text(body_text)
                            .font(ty::MONO_SM)
                            .color(rune::color::hex("#e2e8f0").unwrap_or(t.text_primary))
                            .wrap(),
                    ),
            ),
    )
}
