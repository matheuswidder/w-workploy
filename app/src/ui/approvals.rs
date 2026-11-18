use rune::{El, ScrollSpec, Transition, Tx, col, icon, row, text};
use workploy::model::{self, RunStatus, StepStatus};

use crate::state::AppState;
use crate::ui::components::{
    btn_danger, btn_neutral, copyable_sha, gate_card, header_card, primary_button,
    run_status_icon, surface_card,
};
use crate::ui::icons;
use crate::ui::theme::{UiTheme, ty};

pub fn view(s: &AppState, t: UiTheme) -> El {
    let pending: Vec<_> = s
        .snapshot
        .runs
        .iter()
        .filter(|r| r.status == RunStatus::AwaitingApproval)
        .collect();

    let header_subtitle = if pending.is_empty() {
        "Nenhuma execução parada em portão de aprovação no momento.".to_string()
    } else if pending.len() == 1 {
        "1 execução aguardando sua liberação manual antes de prosseguir.".to_string()
    } else {
        format!(
            "{} execuções aguardando sua liberação manual antes de prosseguir.",
            pending.len()
        )
    };
    let badge_text = if pending.is_empty() {
        "Fila limpa".to_string()
    } else if pending.len() == 1 {
        "1 pendente".to_string()
    } else {
        format!("{} pendentes", pending.len())
    };

    let header = header_card(
        t,
        "approvals-hdr",
        10.0,
        row()
            .px(20.0)
            .py(16.0)
            .gap(14.0)
            .center_y()
            .justify_between()
            .child(
                col()
                    .gap(4.0)
                    .child(text("Aprovações").font(ty::H1).color(t.text_primary))
                    .child(
                        text(header_subtitle)
                            .font(ty::SMALL)
                            .color(t.text_muted),
                    ),
            )
            .child(
                row()
                    .h(28.0)
                    .px(12.0)
                    .gap(6.0)
                    .center_y()
                    .radius(14.0)
                    .fill(if pending.is_empty() {
                        t.success_bg
                    } else {
                        t.warning_bg
                    })
                    .border(
                        1.0,
                        if pending.is_empty() {
                            t.success.alpha(0.35)
                        } else {
                            t.warning.alpha(0.45)
                        },
                    )
                    .child(icon(&icons::SHIELD, 13.0).color(if pending.is_empty() {
                        t.success
                    } else {
                        t.warning
                    }))
                    .child(
                        text(badge_text)
                            .font(ty::CAPTION)
                            .color(if pending.is_empty() {
                                t.success
                            } else {
                                t.warning
                            }),
                    ),
            ),
    );

    let body = if pending.is_empty() {
        surface_card(
            t,
            "approvals-empty",
            12.0,
            col()
                .py(56.0)
                .px(24.0)
                .center()
                .gap(10.0)
                .child(icon(&icons::CHECK_CIRCLE, 28.0).color(t.success))
                .child(
                    text("Nenhum portão aguardando aprovação")
                        .font(ty::H2)
                        .color(t.text_primary),
                )
                .child(
                    text("Fluxos com passos protegidos por 'Pedir aprovação' aparecerão aqui para liberação manual.")
                        .font(ty::SMALL)
                        .color(t.text_muted),
                ),
        )
    } else {
        let mut stack = col().w_full().gap(18.0).pb(16.0);
        for r in pending {
            let rid_app = r.id.clone();
            let rid_rej = r.id.clone();
            let rid_sum = r.id.clone();
            let rid_logs = r.id.clone();

            let waiting_step = r
                .steps
                .iter()
                .find(|st| st.status == StepStatus::WaitingApproval);
            let step_name = waiting_step
                .map(|st| st.name.as_str())
                .unwrap_or("Passo protegido");
            let step_preview = waiting_step
                .map(|st| st.log.lines().take(2).collect::<Vec<_>>().join(" · "))
                .unwrap_or_default();

            let title = if r.title.is_empty() {
                r.workflow_name.clone()
            } else {
                r.title.clone()
            };

            let srv_label = if let Some(sid) = s.is_remote_vps_run(r) {
                s.server_label(Some(&sid))
            } else if let Some(wf) = s.snapshot.workflows.iter().find(|w| w.id == r.workflow_id) {
                if wf.runner == workploy::model::RunnerKind::Ssh {
                    wf.server_id
                        .as_deref()
                        .map(|sid| s.server_label(Some(sid)))
                        .unwrap_or_else(|| "VPS via SSH".into())
                } else {
                    "Esta máquina".into()
                }
            } else {
                "Esta máquina".into()
            };

            let mut meta = row().gap(8.0).center_y().flex_wrap();
            meta = meta
                .child(text(r.workflow_name.clone()).font(ty::SMALL_MED).color(t.accent))
                .child(text(srv_label).font(ty::CAPTION).color(t.text_secondary))
                .child(text(format!("#{}", r.number)).font(ty::MONO_SM).color(t.text_muted))
                .child(
                    row()
                        .h(20.0)
                        .px(7.0)
                        .gap(4.0)
                        .center_y()
                        .radius(5.0)
                        .fill(t.surface_2)
                        .child(icon(&icons::BRANCH, 10.5).color(t.text_muted))
                        .child(
                            text(if r.branch.is_empty() {
                                "main".into()
                            } else {
                                r.branch.clone()
                            })
                            .font(ty::MONO_SM)
                            .color(t.text_secondary),
                        ),
                );
            if let Some(sha) = &r.commit {
                meta = meta.child(copyable_sha(t, format!("appr-sha-{}", r.id), sha));
            }
            meta = meta.child(
                text(format!(
                    "Esperando há {}",
                    model::fmt_dur(r.elapsed_secs())
                ))
                .font(ty::CAPTION)
                .color(t.warning),
            );

            let card_inner = col()
                .w_full()
                .px(18.0)
                .py(16.0)
                .gap(12.0)
                .child(
                    row()
                        .w_full()
                        .gap(12.0)
                        .center_y()
                        .flex_wrap()
                        .child(run_status_icon(t, r.status, 22.0))
                        .child(
                            col()
                                .grow(1.0)
                                .shrink()
                                .min_w(220.0)
                                .gap(4.0)
                                .child(
                                    text(title)
                                        .font(ty::H3)
                                        .color(t.text_primary)
                                        .wrap(),
                                )
                                .child(meta),
                        )
                        .child(
                            row()
                                .gap(8.0)
                                .center_y()
                                .child(primary_button(
                                    t,
                                    format!("appr-ok-{}", r.id),
                                    "Aprovar e continuar",
                                    Some(&icons::CHECK_CIRCLE),
                                    34.0,
                                    move |s: &mut AppState, tx: &mut Tx| {
                                        s.approve_run(&rid_app, tx)
                                    },
                                ))
                                .child(btn_danger(
                                    t,
                                    format!("appr-rej-{}", r.id),
                                    "Rejeitar",
                                    Some(&icons::X_CIRCLE),
                                    34.0,
                                    move |s: &mut AppState, tx: &mut Tx| {
                                        s.cancel_or_reject_run(&rid_rej, true, tx)
                                    },
                                )),
                        ),
                )
                .child(
                    row()
                        .w_full()
                        .px(12.0)
                        .py(10.0)
                        .gap(10.0)
                        .center_y()
                        .justify_between()
                        .flex_wrap()
                        .radius(8.0)
                        .fill(t.surface_2)
                        .border(1.0, t.border_subtle)
                        .child(
                            col()
                                .grow(1.0)
                                .shrink()
                                .min_w(200.0)
                                .gap(2.0)
                                .child(
                                    text(format!("Passo retido: {step_name}"))
                                        .font(ty::SMALL_MED)
                                        .color(t.warning),
                                )
                                .child(
                                    text(step_preview)
                                        .font(ty::MONO_SM)
                                        .color(t.text_secondary)
                                        .truncate(),
                                ),
                        )
                        .child(
                            row()
                                .gap(6.0)
                                .child(btn_neutral(
                                    t,
                                    format!("appr-sum-{}", r.id),
                                    "Resumo",
                                    Some(&icons::LIST),
                                    28.0,
                                    move |s: &mut AppState, _| s.open_run_summary(&rid_sum),
                                ))
                                .child(btn_neutral(
                                    t,
                                    format!("appr-log-{}", r.id),
                                    "Ver logs",
                                    Some(&icons::CODE),
                                    28.0,
                                    move |s: &mut AppState, _| {
                                        s.open_run_logs(&rid_logs, 0)
                                    },
                                )),
                        ),
                );

            stack = stack.child(gate_card(
                t,
                format!("appr-card-{}", r.id),
                10.0,
                card_inner,
            ));
        }
        stack
    };

    col()
        .key("approvals-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(20.0)
        .pb(16.0)
        .gap(16.0)
        .transition(Transition::opacity())
        .child(header)
        .child(
            col()
                .key("approvals-scroll")
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .scroll(ScrollSpec { stick_bottom: false })
                .scroll_edge_fade(16.0)
                .child(body),
        )
}
