use rune::tok::motion;
use rune::{Cursor, El, ScrollSpec, Transition, Tx, col, el, icon, row, text};
use rune_kit::{Kit, TextInput};
use workploy::model::{self, AnnKind, Run, RunStatus, StepRun, StepStatus};

use crate::state::{AppState, DetailTab, JobFilter, ToastKind};
use crate::ui::components::{
    btn_danger, btn_ghost, btn_neutral, chip, copyable_sha, gate_card, header_card, phase_spinner,
    primary_button, run_status_icon, status_pill, step_status_icon, surface_card,
};
use crate::ui::icons;
use crate::ui::theme::{UiTheme, ty};

pub fn view(
    s: &AppState,
    t: UiTheme,
    k: &Kit,
    run_id: &str,
    tab: DetailTab,
) -> El {
    let Some(run) = s.snapshot.runs.iter().find(|r| r.id == run_id) else {
        return col()
            .key("detail-missing")
            .w_full()
            .h_full()
            .center()
            .gap(12.0)
            .child(text("Execução não encontrada").font(ty::H2).color(t.text_primary))
            .child(btn_neutral(
                t,
                "back-missing",
                "Voltar aos fluxos",
                Some(&icons::ARROW_LEFT),
                34.0,
                |s: &mut AppState, _| {
                    let prev = s.previous_list_nav.clone();
                    s.navigate_to(prev);
                },
            ));
    };

    let wf = s
        .snapshot
        .workflows
        .iter()
        .find(|w| w.id == run.workflow_id);

    // Breadcrumb + Top header
    let breadcrumb = row()
        .key("detail-breadcrumb")
        .role("button")
        .label("Voltar ao fluxo")
        .h(30.0)
        .px(10.0)
        .gap(6.0)
        .center_y()
        .radius(7.0)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent)
        .hover(t.hover)
        .press(t.press)
        .press_scale(0.97)
        .on_press(|s: &mut AppState| {
            let prev = s.previous_list_nav.clone();
            s.navigate_to(prev);
        })
        .child(icon(&icons::ARROW_LEFT, 13.0).color(t.accent).a11y_hidden())
        .child(
            text("Voltar ao fluxo")
                .font(ty::SMALL_MED)
                .color(t.text_secondary)
                .a11y_hidden(),
        );

    let title_str = if run.title.is_empty() {
        run.workflow_name.clone()
    } else {
        run.title.clone()
    };

    let mut header_actions = row().gap(8.0).center_y().flex_wrap();
    let rid_approve = run.id.clone();
    let rid_reject = run.id.clone();
    let rid_cancel = run.id.clone();
    let wf_id_rerun = run.workflow_id.clone();

    // Spinner de fase junto do status enquanto a execução está ativa
    // (família pulse-* para CI, blocks-* para deploy), com o percentual
    // de passos concluídos.
    let active_phase_widgets: Vec<El> =
        if matches!(run.status, RunStatus::Running | RunStatus::Queued) {
            let kind = model::run_phase_spinner_kind(run, &run.workflow_name);
            vec![
                phase_spinner(t, kind, 18.0, format!("detail-phase-{}", run.id)),
                text(format!("{}%", model::steps_completed_pct(run)))
                    .font(ty::MONO_SM)
                    .color(t.info),
            ]
        } else {
            Vec::new()
        };

    match run.status {
        RunStatus::AwaitingApproval => {
            header_actions = header_actions
                .child(primary_button(
                    t,
                    "detail-approve",
                    "Aprovar e continuar",
                    Some(&icons::CHECK_CIRCLE),
                    34.0,
                    move |s: &mut AppState, tx: &mut Tx| s.approve_run(&rid_approve, tx),
                ))
                .child(btn_danger(
                    t,
                    "detail-reject",
                    "Rejeitar",
                    Some(&icons::X_CIRCLE),
                    34.0,
                    move |s: &mut AppState, tx: &mut Tx| {
                        s.cancel_or_reject_run(&rid_reject, true, tx)
                    },
                ));
        }
        RunStatus::Running | RunStatus::Queued => {
            header_actions = header_actions.child(btn_danger(
                t,
                "detail-cancel",
                "Cancelar execução",
                Some(&icons::X_CIRCLE),
                34.0,
                move |s: &mut AppState, tx: &mut Tx| {
                    s.cancel_or_reject_run(&rid_cancel, false, tx)
                },
            ));
        }
        _ => {
            header_actions = header_actions.child(primary_button(
                t,
                "detail-rerun",
                "Re-executar tudo",
                Some(&icons::REFRESH),
                34.0,
                move |s: &mut AppState, tx: &mut Tx| {
                    s.start_workflow(&wf_id_rerun, "re-execução manual", tx)
                },
            ));
        }
    }

    let header_inner = row()
        .px(18.0)
        .py(14.0)
        .gap(14.0)
        .center_y()
        .flex_wrap()
        .child(run_status_icon(t, run.status, 22.0))
        .child(
            col()
                .grow(1.0)
                .shrink()
                .min_w(220.0)
                .gap(4.0)
                .child(
                    row()
                        .gap(10.0)
                        .center_y()
                        .flex_wrap()
                        .child(
                            text(title_str)
                                .font(ty::H2)
                                .color(t.text_primary)
                                .shrink(),
                        )
                        .child(
                            text(format!("#{}", run.number))
                                .font(ty::MONO_SM)
                                .color(t.accent),
                        )
                        .child(status_pill(t, run.status))
                        .children(active_phase_widgets),
                )
                .child(
                    text(format!(
                        "{}  ·  Gatilho: {}  ·  Duração: {}",
                        run.workflow_name,
                        run.trigger_label,
                        model::fmt_dur(run.elapsed_secs())
                    ))
                    .font(ty::SMALL)
                    .color(t.text_muted),
                ),
        )
        .child(header_actions);

    let hdr_card = match run.status {
        RunStatus::Success => el()
            .key("detail-hdr-card")
            .w_full()
            .radius(10.0)
            .fill(t.surface_1)
            .border(1.0, t.success.alpha(0.35))
            .child(header_inner.w_full()),
        RunStatus::Failed => el()
            .key("detail-hdr-card")
            .w_full()
            .radius(10.0)
            .fill(t.surface_1)
            .border(1.0, t.danger.alpha(0.40))
            .child(header_inner.w_full()),
        RunStatus::AwaitingApproval => {
            gate_card(t, "detail-hdr-card", 10.0, header_inner)
        }
        _ => header_card(t, "detail-hdr-card", 10.0, header_inner),
    };

    // Sub-menu lateral da execução
    let sub_menu = detail_sub_menu(s, t, run, wf, tab);

    // Main detail pane
    let main_pane = match tab {
        DetailTab::Summary => summary_view(s, t, run),
        DetailTab::Logs { job_idx } => logs_view(s, t, k, run, job_idx),
    };

    col()
        .key("detail-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(14.0)
        .pb(16.0)
        .gap(12.0)
        .transition(Transition::opacity())
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(breadcrumb)
                .child(
                    text(format!("Iniciado {}", run.started_at))
                        .font(ty::CAPTION)
                        .color(t.text_muted),
                ),
        )
        .child(hdr_card)
        .child(
            row()
                .key("detail-body-split")
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .gap(16.0)
                .child(sub_menu)
                .child(main_pane),
        )
}

fn detail_sub_menu(
    s: &AppState,
    t: UiTheme,
    run: &Run,
    wf: Option<&workploy::model::Workflow>,
    tab: DetailTab,
) -> El {
    let menu_w = 244.0;
    let is_summary = matches!(tab, DetailTab::Summary);
    let rid_sum = run.id.clone();

    let mut sub_rows = col().w_full().gap(6.0);

    // Resumo tab with sliding lead indicator
    let sel_top = if is_summary { 0.0 } else { 42.0 };
    let mut tab_stack = el()
        .key("detail-tabs-group")
        .w_full()
        .h(78.0)
        .child(
            el().key("detail-tab-indicator")
                .absolute(0.0, sel_top)
                .size(menu_w - 20.0, 34.0)
                .radius(6.0)
                .fill(t.surface_3)
                .border(1.0, t.border_subtle)
                .lead(motion::LEAD, motion::DEFAULT),
        )
        .child(
            row()
                .key("tab-summary")
                .role("tab")
                .label("Resumo")
                .checked(is_summary)
                .absolute(0.0, 0.0)
                .size(menu_w - 20.0, 34.0)
                .px(10.0)
                .gap(8.0)
                .center_y()
                .radius(6.0)
                .cursor(Cursor::Pointer)
                .focusable(true)
                .focus_ring(2.0, 2.0, t.accent)
                .hover(if is_summary {
                    rune::Color::TRANSPARENT
                } else {
                    t.hover
                })
                .press(t.press)
                .on_press(move |s: &mut AppState| s.open_run_summary(&rid_sum))
                .child(
                    icon(&icons::DOCUMENTS, 14.0)
                        .color(if is_summary { t.text_primary } else { t.text_muted })
                        .a11y_hidden(),
                )
                .child(
                    text("Resumo")
                        .font(if is_summary { ty::BODY_MED } else { ty::BODY })
                        .color(if is_summary {
                            t.text_primary
                        } else {
                            t.text_secondary
                        })
                        .a11y_hidden(),
                ),
        );

    let rid_first_log = run.id.clone();
    let active_job = match tab {
        DetailTab::Logs { job_idx } => job_idx,
        _ => 0,
    };
    tab_stack = tab_stack.child(
        row()
            .key("tab-logs")
            .role("tab")
            .label("Logs dos jobs")
            .checked(!is_summary)
            .absolute(0.0, 42.0)
            .size(menu_w - 20.0, 34.0)
            .px(10.0)
            .gap(8.0)
            .center_y()
            .radius(6.0)
            .cursor(Cursor::Pointer)
            .focusable(true)
            .focus_ring(2.0, 2.0, t.accent)
            .hover(if !is_summary {
                rune::Color::TRANSPARENT
            } else {
                t.hover
            })
            .press(t.press)
            .on_press(move |s: &mut AppState| s.open_run_logs(&rid_first_log, active_job))
            .child(
                icon(&icons::NOTES, 14.0)
                    .color(if !is_summary { t.text_primary } else { t.text_muted })
                    .a11y_hidden(),
            )
            .child(
                text("Logs dos jobs")
                    .font(if !is_summary { ty::BODY_MED } else { ty::BODY })
                    .color(if !is_summary {
                        t.text_primary
                    } else {
                        t.text_secondary
                    })
                    .a11y_hidden(),
            ),
    );

    sub_rows = sub_rows.child(tab_stack);

    // Todos os jobs + filter
    sub_rows = sub_rows
        .child(el().w_full().h(t.hair).fill(t.border_subtle).my(6.0))
        .child(
            text("TODOS OS JOBS")
                .font(ty::EYEBROW)
                .color(t.text_muted)
                .px(4.0),
        )
        .child(
            row()
                .gap(4.0)
                .flex_wrap()
                .child(chip(
                    t,
                    "jf-all",
                    "Todos",
                    s.job_filter == JobFilter::All,
                    |s: &mut AppState, _| s.job_filter = JobFilter::All,
                ))
                .child(chip(
                    t,
                    "jf-fail",
                    "Falhos",
                    s.job_filter == JobFilter::Failed,
                    |s: &mut AppState, _| s.job_filter = JobFilter::Failed,
                ))
                .child(chip(
                    t,
                    "jf-act",
                    "Ativos",
                    s.job_filter == JobFilter::Active,
                    |s: &mut AppState, _| s.job_filter = JobFilter::Active,
                )),
        );

    let mut jobs_list = col().w_full().gap(4.0).mt(4.0);
    let mut visible_jobs = 0usize;
    for (idx, step) in run.steps.iter().enumerate() {
        let keep = match s.job_filter {
            JobFilter::All => true,
            JobFilter::Failed => step.status == StepStatus::Failed,
            JobFilter::Active => matches!(
                step.status,
                StepStatus::Running | StepStatus::WaitingApproval
            ),
        };
        if !keep {
            continue;
        }
        visible_jobs += 1;
        let selected = matches!(tab, DetailTab::Logs { job_idx } if job_idx == idx);
        let rid = run.id.clone();
        // Passo em execução: o spinner carrega a fase e a família do fluxo
        // (pulse-* para CI, blocks-* para deploy) em vez do ponto genérico.
        let job_status_icon_el = if step.status == StepStatus::Running {
            let kind = model::run_phase_spinner_kind(run, &run.workflow_name);
            phase_spinner(
                t,
                kind,
                14.0,
                format!("detail-job-phase-{}-{idx}", run.id),
            )
        } else {
            step_status_icon(t, step.status, 14.0)
        };
        jobs_list = jobs_list.child(
            row()
                .key(format!("sub-job-{idx}"))
                .role("button")
                .label(format!("Abrir logs de {}", step.name))
                .w_full()
                .min_h(34.0)
                .px(8.0)
                .py(6.0)
                .gap(8.0)
                .center_y()
                .radius(7.0)
                .fill(if selected { t.surface_2 } else { rune::Color::TRANSPARENT })
                .border(
                    1.0,
                    if selected {
                        t.accent.alpha(0.45)
                    } else {
                        rune::Color::TRANSPARENT
                    },
                )
                .cursor(Cursor::Pointer)
                .focusable(true)
                .focus_ring(2.0, 2.0, t.accent)
                .hover(t.hover)
                .press(t.press)
                .on_press(move |s: &mut AppState| s.open_run_logs(&rid, idx))
                .child(job_status_icon_el)
                .child(
                    text(step.name.clone())
                        .font(if selected { ty::SMALL_MED } else { ty::SMALL })
                        .color(if selected {
                            t.text_primary
                        } else {
                            t.text_secondary
                        })
                        .grow(1.0)
                        .shrink()
                        .truncate(),
                )
                .child(
                    text(model::fmt_dur(step.elapsed_secs()))
                        .font(ty::CAPTION)
                        .color(t.text_muted),
                ),
        );
    }
    if visible_jobs == 0 {
        jobs_list = jobs_list.child(
            text("Nenhum job neste filtro.")
                .font(ty::CAPTION)
                .color(t.text_muted)
                .px(6.0)
                .py(6.0),
        );
    }
    sub_rows = sub_rows.child(jobs_list);

    // Detalhes ("Uso", "Arquivo YAML", "Runner")
    sub_rows = sub_rows
        .child(el().w_full().h(t.hair).fill(t.border_subtle).my(8.0))
        .child(
            text("DETALHES")
                .font(ty::EYEBROW)
                .color(t.text_muted)
                .px(4.0),
        );

    let remote_srv_id = s.is_remote_vps_run(run);
    let runner_label = if let Some(sid) = remote_srv_id.as_deref() {
        s.server_label(Some(sid))
    } else if let Some(w) = wf {
        if w.runner == workploy::model::RunnerKind::Ssh {
            match w.server_id.as_deref() {
                Some(sid) => s.server_label(Some(sid)),
                None => w.runner.label().to_string(),
            }
        } else {
            w.runner.label().to_string()
        }
    } else {
        "Esta máquina".to_string()
    };
    sub_rows = sub_rows
        .child(
            row()
                .w_full()
                .px(6.0)
                .py(4.0)
                .justify_between()
                .child(text("Servidor").font(ty::CAPTION).color(t.text_muted))
                .child(text(runner_label).font(ty::CAPTION).color(t.text_secondary)),
        )
        .child(
            row()
                .w_full()
                .px(6.0)
                .py(4.0)
                .justify_between()
                .child(text("Uso de cota").font(ty::CAPTION).color(t.text_muted))
                .child(text("Ilimitado (local)").font(ty::CAPTION).color(t.success)),
        );

    if let Some(sid) = remote_srv_id {
        sub_rows = sub_rows.child(
            row()
                .key("detail-sync-vps")
                .role("button")
                .label("Sincronizar com a VPS")
                .w_full()
                .h(30.0)
                .px(8.0)
                .gap(7.0)
                .center_y()
                .radius(7.0)
                .fill(t.surface_2)
                .border(1.0, t.border_subtle)
                .cursor(Cursor::Pointer)
                .focusable(true)
                .focus_ring(2.0, 2.0, t.accent)
                .hover(t.hover)
                .press(t.press)
                .on_press_tx(move |s: &mut AppState, tx: &mut Tx| s.sync_server_runs(&sid, tx))
                .child(icon(&icons::REFRESH, 13.0).color(t.accent).a11y_hidden())
                .child(
                    text("Sincronizar VPS")
                        .font(ty::SMALL_MED)
                        .color(t.text_secondary)
                        .a11y_hidden(),
                ),
        );
    }

    if let Some(w) = wf {
        let wid = w.id.clone();
        sub_rows = sub_rows.child(
            row()
                .key("detail-open-yaml")
                .role("button")
                .label("Abrir arquivo YAML")
                .w_full()
                .h(30.0)
                .px(8.0)
                .gap(7.0)
                .center_y()
                .radius(7.0)
                .fill(t.surface_2)
                .border(1.0, t.border_subtle)
                .cursor(Cursor::Pointer)
                .focusable(true)
                .focus_ring(2.0, 2.0, t.accent)
                .hover(t.hover)
                .press(t.press)
                .on_press(move |s: &mut AppState| s.open_yaml_modal(Some(&wid)))
                .child(icon(&icons::CODE, 13.0).color(t.accent).a11y_hidden())
                .child(
                    text("Arquivo YAML")
                        .font(ty::SMALL_MED)
                        .color(t.text_secondary)
                        .a11y_hidden(),
                ),
        );
    }

    surface_card(
        t,
        "detail-submenu-card",
        12.0,
        col()
            .w(menu_w)
            .h_full()
            .min_h(0.0)
            .padding(10.0)
            .scroll(ScrollSpec { stick_bottom: false })
            .child(sub_rows),
    )
    .w(menu_w)
    .h_full()
    .min_h(0.0)
}

fn summary_view(s: &AppState, t: UiTheme, run: &Run) -> El {
    let mut body = col().w_full().gap(14.0).pb(16.0);

    // 1. Approval banner if AwaitingApproval
    if run.status == RunStatus::AwaitingApproval {
        let rid_app = run.id.clone();
        let rid_rej = run.id.clone();
        let waiting_step = run
            .steps
            .iter()
            .find(|st| st.status == StepStatus::WaitingApproval)
            .map(|st| st.name.as_str())
            .unwrap_or("Passo protegido");

        let banner_inner = row()
            .w_full()
            .px(18.0)
            .py(14.0)
            .gap(14.0)
            .center_y()
            .flex_wrap()
            .child(icon(&icons::BOLT, 20.0).color(t.warning))
            .child(
                col()
                    .grow(1.0)
                    .shrink()
                    .min_w(200.0)
                    .gap(3.0)
                    .child(
                        text("Portão de aprovação pendente")
                            .font(ty::H3)
                            .color(t.text_primary),
                    )
                    .child(
                        text(format!(
                            "O passo '{waiting_step}' aguarda sua confirmação para continuar."
                        ))
                        .font(ty::SMALL)
                        .color(t.text_secondary)
                        .wrap(),
                    ),
            )
            .child(
                row()
                    .gap(8.0)
                    .center_y()
                    .child(primary_button(
                        t,
                        "banner-approve",
                        "Aprovar e continuar",
                        Some(&icons::CHECK_CIRCLE),
                        34.0,
                        move |s: &mut AppState, tx: &mut Tx| s.approve_run(&rid_app, tx),
                    ))
                    .child(btn_danger(
                        t,
                        "banner-reject",
                        "Rejeitar",
                        Some(&icons::X_CIRCLE),
                        34.0,
                        move |s: &mut AppState, tx: &mut Tx| {
                            s.cancel_or_reject_run(&rid_rej, true, tx)
                        },
                    )),
            );
        body = body.child(gate_card(
            t,
            "summary-approval-banner",
            10.0,
            banner_inner,
        ));
    }

    // 2. Metadata card
    let mut meta_grid = row().w_full().gap(18.0).flex_wrap();
    meta_grid = meta_grid
        .child(meta_cell(t, "Gatilho", &run.trigger_label))
        .child(meta_cell(
            t,
            "Branch",
            if run.branch.is_empty() { "main" } else { &run.branch },
        ))
        .child(meta_cell(
            t,
            "Autor",
            if run.actor.is_empty() { "maestro" } else { &run.actor },
        ))
        .child(meta_cell(
            t,
            "Duração total",
            &model::fmt_dur(run.elapsed_secs()),
        ));
    if let Some(sha) = &run.commit {
        meta_grid = meta_grid.child(
            col()
                .gap(4.0)
                .child(text("Commit").font(ty::CAPTION).color(t.text_muted))
                .child(copyable_sha(t, "detail-meta-sha", sha)),
        );
    }
    body = body.child(surface_card(
        t,
        "summary-meta-card",
        10.0,
        col()
            .px(18.0)
            .py(14.0)
            .gap(10.0)
            .child(text("Detalhes da execução").font(ty::H3).color(t.text_primary))
            .child(meta_grid),
    ));

    // 3. Interactive Job Graph
    let mut graph_row = row().w_full().gap(10.0).center_y().flex_wrap();
    for (idx, step) in run.steps.iter().enumerate() {
        if idx > 0 {
            graph_row = graph_row.child(
                row()
                    .w(20.0)
                    .center()
                    .child(icon(&icons::CHEVRON_RIGHT, 14.0).color(t.text_muted)),
            );
        }
        let rid = run.id.clone();
        let node_border = match step.status {
            StepStatus::Success => t.success.alpha(0.45),
            StepStatus::Failed => t.danger.alpha(0.55),
            StepStatus::WaitingApproval => t.warning.alpha(0.65),
            StepStatus::Running => t.info.alpha(0.55),
            _ => t.border_subtle,
        };
        graph_row = graph_row.child(
            row()
                .key(format!("graph-job-{idx}"))
                .role("button")
                .label(format!("Ver logs do job {}", step.name))
                .min_w(195.0)
                .max_w(260.0)
                .px(12.0)
                .py(10.0)
                .gap(10.0)
                .center_y()
                .radius(10.0)
                .fill(t.surface_2)
                .border(1.0, node_border)
                .cursor(Cursor::Pointer)
                .focusable(true)
                .focus_ring(2.0, 2.0, t.accent)
                .hover(t.hover)
                .press(t.press)
                .press_scale(0.98)
                .on_press(move |s: &mut AppState| s.open_run_logs(&rid, idx))
                .child(step_status_icon(t, step.status, 18.0))
                .child(
                    col()
                        .grow(1.0)
                        .shrink()
                        .min_w(0.0)
                        .gap(2.0)
                        .child(
                            text(step.name.clone())
                                .font(ty::SMALL_MED)
                                .color(t.text_primary)
                                .truncate(),
                        )
                        .child(
                            text(format!(
                                "{} · {}",
                                step.status.label(),
                                model::fmt_dur(step.elapsed_secs())
                            ))
                            .font(ty::CAPTION)
                            .color(t.text_muted)
                            .truncate(),
                        ),
                ),
        );
    }

    body = body.child(surface_card(
        t,
        "summary-graph-card",
        12.0,
        col()
            .px(18.0)
            .py(14.0)
            .gap(12.0)
            .child(
                row()
                    .w_full()
                    .justify_between()
                    .center_y()
                    .child(text("Grafo de jobs").font(ty::H3).color(t.text_primary))
                    .child(
                        text("Clique em um job para abrir seus logs")
                            .font(ty::CAPTION)
                            .color(t.text_muted),
                    ),
            )
            .child(graph_row),
    ));

    // 4. Deploy summary card (`run.summary`)
    if !run.summary.trim().is_empty() {
        let summary_copy = run.summary.clone();
        let mut lines_col = col().w_full().gap(4.0);
        let mut in_code = false;
        let mut code_buf = Vec::new();
        for line in run.summary.lines() {
            if line.trim_start().starts_with("```") {
                if in_code {
                    let block_text = code_buf.join("\n");
                    lines_col = lines_col.child(
                        el().w_full()
                            .p_block(10.0)
                            .radius(8.0)
                            .fill(t.surface_code)
                            .border(1.0, t.border_subtle)
                            .child(
                                text(block_text)
                                    .font(ty::MONO_SM)
                                    .color(rune::color::hex("#e2e8f0").unwrap_or(t.text_primary))
                                    .wrap(),
                            ),
                    );
                    code_buf.clear();
                    in_code = false;
                } else {
                    in_code = true;
                }
                continue;
            }
            if in_code {
                code_buf.push(line.to_string());
            } else if let Some(h) = line.strip_prefix("# ") {
                lines_col = lines_col.child(text(h.to_string()).font(ty::BODY_MED).color(t.text_primary));
            } else {
                lines_col = lines_col.child(
                    text(line.to_string())
                        .font(ty::SMALL)
                        .color(t.text_secondary)
                        .wrap(),
                );
            }
        }
        if !code_buf.is_empty() {
            lines_col = lines_col.child(
                el().w_full()
                    .p_block(10.0)
                    .radius(8.0)
                    .fill(t.surface_code)
                    .child(text(code_buf.join("\n")).font(ty::MONO_SM).color(t.text_primary).wrap()),
            );
        }

        let summary_inner = col()
            .px(18.0)
            .py(14.0)
            .gap(10.0)
            .child(
                row()
                    .w_full()
                    .justify_between()
                    .center_y()
                    .child(text("Resumo do deploy").font(ty::H3).color(t.text_primary))
                    .child(btn_neutral(
                        t,
                        "copy-deploy-summary",
                        "Copiar resumo",
                        Some(&icons::COPY),
                        28.0,
                        move |s: &mut AppState, tx: &mut Tx| {
                            tx.copy(summary_copy.clone());
                            s.push_toast(
                                "copy-summary",
                                "Resumo copiado",
                                Some("Bloco de resumo copiado para a área de transferência.".into()),
                                ToastKind::Success,
                                tx,
                            );
                        },
                    )),
            )
            .child(lines_col);

        body = body.child(if run.status == RunStatus::Success {
            el().key("summary-deploy-card")
                .w_full()
                .radius(10.0)
                .fill(t.surface_1)
                .border(1.0, t.success.alpha(0.35))
                .child(summary_inner.w_full())
        } else {
            surface_card(t, "summary-deploy-card", 10.0, summary_inner)
        });
    }

    // 5. Annotations card (expandable)
    if !run.annotations.is_empty() {
        let total = run.annotations.len();
        let show_n = if s.annotations_expanded {
            total
        } else {
            total.min(2)
        };
        let mut ann_list = col().w_full().gap(8.0);
        for (i, ann) in run.annotations.iter().take(show_n).enumerate() {
            let (bg, fg, label) = match ann.kind {
                AnnKind::Error => (t.danger_bg, t.danger, "Erro"),
                AnnKind::Warning => (t.warning_bg, t.warning, "Aviso"),
                AnnKind::Notice => (t.info_bg, t.info, "Nota"),
            };
            let mut badge_row = row().h(20.0).px(7.0).center().radius(5.0).fill(bg);
            if ann.kind == AnnKind::Error {
                badge_row = badge_row
                    .gap(4.0)
                    .child(icon(&icons::BUG, 11.5).color(fg).a11y_hidden());
            }
            badge_row = badge_row.child(text(label).font(ty::CAPTION).color(fg));
            ann_list = ann_list.child(
                row()
                    .key(format!("ann-{i}"))
                    .w_full()
                    .px(12.0)
                    .py(10.0)
                    .gap(10.0)
                    .items_start()
                    .radius(8.0)
                    .fill(t.surface_2)
                    .border(1.0, t.border_subtle)
                    .child(badge_row)
                    .child(
                        col()
                            .grow(1.0)
                            .shrink()
                            .min_w(0.0)
                            .gap(2.0)
                            .child(text(ann.job.clone()).font(ty::SMALL_MED).color(t.text_primary))
                            .child(
                                text(ann.msg.clone())
                                    .font(ty::SMALL)
                                    .color(t.text_secondary)
                                    .wrap(),
                            ),
                    ),
            );
        }

        let mut ann_hdr = row()
            .w_full()
            .justify_between()
            .center_y()
            .child(
                text(format!("Anotações ({total})"))
                    .font(ty::H3)
                    .color(t.text_primary),
            );
        if total > 2 {
            ann_hdr = ann_hdr.child(btn_ghost(
                t,
                "toggle-annotations",
                if s.annotations_expanded {
                    "Mostrar menos"
                } else {
                    "Mostrar mais"
                },
                Some(if s.annotations_expanded {
                    &icons::CHEVRON_UP
                } else {
                    &icons::CHEVRON_DOWN
                }),
                28.0,
                |s: &mut AppState, _| s.annotations_expanded = !s.annotations_expanded,
            ));
        }

        body = body.child(surface_card(
            t,
            "summary-annotations-card",
            12.0,
            col()
                .px(18.0)
                .py(14.0)
                .gap(10.0)
                .child(ann_hdr)
                .child(ann_list),
        ));
    }

    col()
        .key("detail-summary-scroll")
        .grow(1.0)
        .min_h(0.0)
        .h_full()
        .scroll(ScrollSpec { stick_bottom: false })
        .scroll_edge_fade(16.0)
        .child(body)
}

fn meta_cell(t: UiTheme, label: &str, value: &str) -> El {
    col()
        .gap(3.0)
        .min_w(105.0)
        .child(text(label.to_string()).font(ty::CAPTION).color(t.text_muted))
        .child(
            text(value.to_string())
                .font(ty::BODY_MED)
                .color(t.text_primary),
        )
}

trait PadBlock {
    fn p_block(self, p: f32) -> Self;
}

impl PadBlock for El {
    fn p_block(self, p: f32) -> Self {
        self.padding(p)
    }
}

fn logs_view(s: &AppState, t: UiTheme, k: &Kit, run: &Run, job_idx: usize) -> El {
    let idx = job_idx.min(run.steps.len().saturating_sub(1));
    let Some(job) = run.steps.get(idx) else {
        return surface_card(
            t,
            "logs-empty",
            12.0,
            col()
                .padding(24.0)
                .center()
                .child(text("Nenhum job encontrado neste fluxo.").font(ty::BODY).color(t.text_muted)),
        );
    };

    let search = TextInput::new(&s.log_search)
        .placeholder("Buscar nos logs...")
        .label("Buscar nos logs")
        .leading_icon(k.glyph.search)
        .clear_button()
        .width(Some(220.0))
        .on_edit(|s: &mut AppState| &mut s.log_search)
        .on_action(|s: &mut AppState, _| s.log_search.clear())
        .build(k)
        .cursor(Cursor::Text);

    let full_log = collect_job_log(job);
    let toolbar = row()
        .w_full()
        .gap(8.0)
        .center_y()
        .flex_wrap()
        .child(search)
        .child(chip(
            t,
            "chip-line-nums",
            "Números de linha",
            s.log_line_numbers,
            |s: &mut AppState, _| s.log_line_numbers = !s.log_line_numbers,
        ))
        .child(chip(
            t,
            "chip-wrap-lines",
            "Quebrar linhas longas",
            s.log_wrap_lines,
            |s: &mut AppState, _| s.log_wrap_lines = !s.log_wrap_lines,
        ))
        .child(btn_neutral(
            t,
            "btn-reload-logs",
            "Recarregar",
            Some(&icons::REFRESH),
            30.0,
            |s: &mut AppState, tx| {
                if !s.demo_mode
                    && let Ok(guard) = s.shared.lock()
                {
                    s.snapshot = guard.clone();
                }
                s.push_toast(
                    "logs-reloaded",
                    "Logs atualizados",
                    None,
                    ToastKind::Info,
                    tx,
                );
            },
        ))
        .child(btn_neutral(
            t,
            "btn-copy-all-logs",
            "Copiar logs",
            Some(&icons::COPY),
            30.0,
            move |s: &mut AppState, tx| {
                tx.copy(full_log.clone());
                s.push_toast(
                    "copy-logs",
                    "Logs copiados",
                    Some("Saída completa do job copiada.".into()),
                    ToastKind::Success,
                    tx,
                );
            },
        ));

    let query = s.log_search.text.trim().to_lowercase();
    let mut steps_col = col().w_full().gap(10.0).pb(16.0);

    let run_active = run.status.active();

    if job.substeps.is_empty() {
        let key = format!("step-{idx}");
        steps_col = steps_col.child(collapsible_step_log(
            s,
            t,
            &key,
            job,
            &query,
            run_active,
        ));
    } else {
        for (si, sub) in job.substeps.iter().enumerate() {
            let key = format!("sub-{idx}-{si}");
            steps_col = steps_col.child(collapsible_step_log(
                s,
                t,
                &key,
                sub,
                &query,
                run_active,
            ));
        }
    }

    col()
        .key("logs-pane")
        .grow(1.0)
        .min_h(0.0)
        .h_full()
        .gap(12.0)
        .child(surface_card(
            t,
            "logs-header-card",
            12.0,
            col()
                .px(16.0)
                .py(12.0)
                .gap(10.0)
                .child(
                    row()
                        .w_full()
                        .justify_between()
                        .center_y()
                        .gap(10.0)
                        .child(
                            row()
                                .gap(10.0)
                                .center_y()
                                .child(step_status_icon(t, job.status, 18.0))
                                .child(text(job.name.clone()).font(ty::H3).color(t.text_primary)),
                        )
                        .child(
                            text(format!(
                                "{} · {}",
                                job.status.label(),
                                model::fmt_dur(job.elapsed_secs())
                            ))
                            .font(ty::MONO_SM)
                            .color(t.text_muted),
                        ),
                )
                .child(toolbar),
        ))
        .child(
            col()
                .key("logs-scroll")
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .scroll(ScrollSpec {
                    stick_bottom: run_active,
                })
                .scroll_edge_fade(16.0)
                .child(steps_col),
        )
}

fn collapsible_step_log(
    s: &AppState,
    t: UiTheme,
    step_key: &str,
    step: &StepRun,
    query: &str,
    run_active: bool,
) -> El {
    let force_open = run_active && step.status == StepStatus::Running;
    let collapsed = !force_open && s.collapsed_steps.contains(step_key);
    let key_toggle = step_key.to_string();

    let header = row()
        .key(format!("hdr-{step_key}"))
        .role("button")
        .label(format!("Alternar passo {}", step.name))
        .w_full()
        .h(38.0)
        .px(12.0)
        .gap(8.0)
        .center_y()
        .radius(8.0)
        .fill(t.surface_2)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent)
        .hover(t.hover)
        .press(t.press)
        .on_press(move |s: &mut AppState| {
            if !s.collapsed_steps.remove(&key_toggle) {
                s.collapsed_steps.insert(key_toggle.clone());
            }
        })
        .child(
            icon(
                if collapsed {
                    &icons::CHEVRON_RIGHT
                } else {
                    &icons::CHEVRON_DOWN
                },
                14.0,
            )
            .color(t.text_muted)
            .a11y_hidden(),
        )
        .child(step_status_icon(t, step.status, 15.0))
        .child(
            text(step.name.clone())
                .font(ty::BODY_MED)
                .color(t.text_primary)
                .grow(1.0)
                .shrink()
                .truncate()
                .a11y_hidden(),
        )
        .child(
            text(model::fmt_dur(step.elapsed_secs()))
                .font(ty::MONO_SM)
                .color(t.text_muted)
                .a11y_hidden(),
        );

    let mut card = col()
        .key(format!("card-{step_key}"))
        .w_full()
        .radius(10.0)
        .fill(t.surface_1)
        .border(1.0, t.border_subtle)
        .child(header);

    if !collapsed {
        let mut lines_box = col()
            .w_full()
            .p_block(10.0)
            .gap(2.0)
            .fill(t.surface_code)
            .corners([0.0, 0.0, 10.0, 10.0]);
        let raw = if step.log.trim().is_empty() {
            "(sem saída registrada neste passo)"
        } else {
            step.log.as_str()
        };
        let mut shown = 0usize;
        for (ln, line) in raw.lines().enumerate() {
            let trimmed_marker = line.trim().trim_matches('"');
            let substep_name = trimmed_marker
                .strip_prefix("::workploy-substep::")
                .map(str::trim);
            let display_line = if let Some(sub_name) = substep_name {
                format!(
                    "▸ {}",
                    if sub_name.is_empty() {
                        "Sub-etapa"
                    } else {
                        sub_name
                    }
                )
            } else {
                line.to_string()
            };

            if !query.is_empty()
                && !line.to_lowercase().contains(query)
                && !display_line.to_lowercase().contains(query)
            {
                continue;
            }
            shown += 1;
            let is_err = line.contains("erro")
                || line.contains("error")
                || line.contains("falhou")
                || line.contains("fatal");
            let is_ok = line.contains("✓")
                || line.contains("PASS")
                || line.contains("200 OK")
                || line.contains("aprovado");
            let line_col = if substep_name.is_some() {
                t.accent
            } else if is_err {
                rune::color::hex("#fca5a5").unwrap_or(t.danger)
            } else if is_ok {
                rune::color::hex("#6ee7b7").unwrap_or(t.success)
            } else {
                rune::color::hex("#e2e8f0").unwrap_or(t.text_primary)
            };
            let mut lr = row().w_full().gap(10.0).items_start();
            if s.log_line_numbers {
                lr = lr.child(
                    text(format!("{:>3}", ln + 1))
                        .w(30.0)
                        .font(ty::MONO_SM)
                        .color(rune::color::hex("#64748b").unwrap_or(t.text_muted))
                        .align(rune::Align::End),
                );
            }
            let mut txt = text(display_line)
                .font(ty::MONO)
                .color(line_col)
                .grow(1.0)
                .shrink();
            if s.log_wrap_lines {
                txt = txt.wrap();
            } else {
                txt = txt.truncate();
            }
            lr = lr.child(txt);
            lines_box = lines_box.child(lr);
        }
        if shown == 0 {
            lines_box = lines_box.child(
                text("Nenhuma linha corresponde à busca.")
                    .font(ty::MONO_SM)
                    .color(t.text_muted),
            );
        }
        if !s.log_wrap_lines {
            card = card.child(
                el().w_full()
                    .scroll_x()
                    .child(lines_box),
            );
        } else {
            card = card.child(lines_box);
        }
    }

    card
}

fn collect_job_log(job: &StepRun) -> String {
    if job.substeps.is_empty() {
        return job.log.clone();
    }
    let mut out = String::new();
    for s in &job.substeps {
        out.push_str(&format!("=== {} ===\n{}\n\n", s.name, s.log));
    }
    out
}
