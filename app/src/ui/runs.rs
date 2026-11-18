use std::collections::BTreeSet;

use rune::tok::motion;
use rune::{Cursor, El, ScrollSpec, Transition, Tx, col, el, icon, row, text};
use rune_kit::Placement;
use rune_kit::{Kit, Select, SelectAppearance, TextInput};
use workploy::model::{self, Run, RunStatus};

use crate::state::{AppState, ModalState, StatusFilter};
use crate::ui::components::{
    btn_danger, btn_ghost, btn_icon, btn_neutral, chip, copyable_sha, header_card,
    primary_button, run_status_icon, status_pill,
};
use crate::ui::icons;
use crate::ui::theme::{UiTheme, ty};

pub fn view(
    s: &AppState,
    t: UiTheme,
    k: &Kit,
    wf_filter: Option<&str>,
    layer: &mut Vec<El>,
) -> El {
    let wf = wf_filter.and_then(|id| s.snapshot.workflows.iter().find(|w| w.id == id));

    let wf_word = if s.snapshot.workflows.len() == 1 {
        "fluxo configurado"
    } else {
        "fluxos configurados"
    };
    let run_word = if s.snapshot.runs.len() == 1 {
        "execução registrada"
    } else {
        "execuções registradas"
    };

    // Header card
    let (title, subtitle) = if let Some(w) = wf {
        let srv_info = match w.runner {
            model::RunnerKind::Local => "Local (Esta máquina)".to_string(),
            model::RunnerKind::Ssh => match w.server_id.as_deref() {
                Some(sid) => s.server_label(Some(sid)),
                None => w.runner.label().to_string(),
            },
        };
        (
            w.name.clone(),
            format!("Gatilho: {}  ·  Servidor: {}", w.trigger.label(), srv_info),
        )
    } else {
        (
            "Execuções".to_string(),
            format!(
                "{} {wf_word}  ·  {} {run_word}",
                s.snapshot.workflows.len(),
                s.snapshot.runs.len()
            ),
        )
    };

    let mut header_actions = row().gap(8.0).center_y().flex_wrap();
    if let Some(w) = wf {
        let wid_run = w.id.clone();
        let wid_edit = w.id.clone();
        let wid_yaml = w.id.clone();
        let wid_del = w.id.clone();
        let wname_del = w.name.clone();
        header_actions = header_actions
            .child(primary_button(
                t,
                "btn-run-wf",
                "Executar fluxo",
                Some(&icons::PLAY),
                34.0,
                move |s: &mut AppState, tx| s.start_workflow(&wid_run, "manual", tx),
            ))
            .child(btn_neutral(
                t,
                "btn-edit-wf",
                "Editar fluxo",
                Some(&icons::EDIT),
                34.0,
                move |s: &mut AppState, _| s.open_edit_workflow_modal(&wid_edit),
            ))
            .child(btn_neutral(
                t,
                "btn-yaml-wf",
                "Arquivo YAML",
                Some(&icons::CODE),
                34.0,
                move |s: &mut AppState, _| s.open_yaml_modal(Some(&wid_yaml)),
            ))
            .child(btn_danger(
                t,
                "btn-del-wf",
                "Excluir",
                Some(&icons::TRASH),
                34.0,
                move |s: &mut AppState, _| {
                    s.modal = ModalState::DeleteConfirm {
                        wf_id: wid_del.clone(),
                        wf_name: wname_del.clone(),
                    };
                },
            ));
    } else {
        if let Some(primary) = s.snapshot.workflows.first() {
            let wid = primary.id.clone();
            header_actions = header_actions.child(primary_button(
                t,
                "btn-run-primary",
                "Executar fluxo",
                Some(&icons::PLAY),
                34.0,
                move |s: &mut AppState, tx| s.start_workflow(&wid, "manual", tx),
            ));
        }
        header_actions = header_actions
            .child(btn_neutral(
                t,
                "btn-new-wf-top",
                "Novo fluxo",
                Some(&icons::PLUS),
                34.0,
                |s: &mut AppState, _| s.open_new_workflow_modal(),
            ))
            .child(btn_neutral(
                t,
                "btn-yaml-import",
                "Importar YAML",
                Some(&icons::CODE),
                34.0,
                |s: &mut AppState, _| s.open_yaml_modal(None),
            ));
    }

    let hdr_card = header_card(
        t,
        "runs-header-card",
        10.0,
        row()
            .px(20.0)
            .py(16.0)
            .gap(16.0)
            .center_y()
            .flex_wrap()
            .child(
                col()
                    .grow(1.0)
                    .shrink()
                    .min_w(170.0)
                    .gap(4.0)
                    .child(text(title).font(ty::H1).color(t.text_primary).truncate())
                    .child(text(subtitle).font(ty::SMALL).color(t.text_muted).truncate()),
            )
            .child(header_actions),
    );

    // Filter options collected from current runs
    let mut events_set = BTreeSet::new();
    let mut branches_set = BTreeSet::new();
    let mut actors_set = BTreeSet::new();
    for r in &s.snapshot.runs {
        if wf_filter.is_none_or(|id| r.workflow_id == id) {
            if !r.trigger_label.is_empty() {
                events_set.insert(r.trigger_label.clone());
            }
            if !r.branch.is_empty() {
                branches_set.insert(r.branch.clone());
            }
            if !r.actor.is_empty() {
                actors_set.insert(r.actor.clone());
            }
        }
    }

    let status_opts: Vec<String> = StatusFilter::ALL
        .iter()
        .map(|st| st.label().to_string())
        .collect();
    let mut event_opts = vec!["Todos eventos".to_string()];
    event_opts.extend(events_set);
    let mut branch_opts = vec!["Todas branches".to_string()];
    branch_opts.extend(branches_set);
    let mut actor_opts = vec!["Todos autores".to_string()];
    actor_opts.extend(actors_set);

    let search_input = TextInput::new(&s.search_runs)
        .placeholder("Pesquisar execuções...")
        .label("Pesquisar execuções")
        .leading_icon(k.glyph.search)
        .clear_button()
        .width(Some(236.0))
        .on_edit(|s: &mut AppState| &mut s.search_runs)
        .on_action(|s: &mut AppState, _| s.search_runs.clear())
        .build(k)
        .cursor(Cursor::Text);

    let sel_status = Select::new("flt-status", &status_opts)
        .label("Status")
        .appearance(SelectAppearance::Field)
        .width(154.0)
        .placement(Placement::BottomStart)
        .current(Some(s.filter_status.label()))
        .open(s.open_select.as_deref() == Some("flt-status"))
        .on_toggle(|s: &mut AppState| {
            s.open_select = if s.open_select.as_deref() == Some("flt-status") {
                None
            } else {
                Some("flt-status".into())
            };
        })
        .on_close(|s: &mut AppState| s.open_select = None)
        .on_choose(|s: &mut AppState, v: &str, _| {
            s.filter_status = StatusFilter::from_label(v);
            s.open_select = None;
        })
        .build(k, layer)
        .cursor(Cursor::Pointer);

    let sel_event = Select::new("flt-event", &event_opts)
        .label("Evento")
        .appearance(SelectAppearance::Field)
        .width(154.0)
        .placement(Placement::BottomStart)
        .current(Some(
            s.filter_event
                .as_deref()
                .unwrap_or("Todos eventos"),
        ))
        .open(s.open_select.as_deref() == Some("flt-event"))
        .on_toggle(|s: &mut AppState| {
            s.open_select = if s.open_select.as_deref() == Some("flt-event") {
                None
            } else {
                Some("flt-event".into())
            };
        })
        .on_close(|s: &mut AppState| s.open_select = None)
        .on_choose(|s: &mut AppState, v: &str, _| {
            s.filter_event = if v == "Todos eventos" {
                None
            } else {
                Some(v.to_string())
            };
            s.open_select = None;
        })
        .build(k, layer)
        .cursor(Cursor::Pointer);

    let sel_branch = Select::new("flt-branch", &branch_opts)
        .label("Branch")
        .appearance(SelectAppearance::Field)
        .width(158.0)
        .placement(Placement::BottomStart)
        .current(Some(
            s.filter_branch
                .as_deref()
                .unwrap_or("Todas branches"),
        ))
        .open(s.open_select.as_deref() == Some("flt-branch"))
        .on_toggle(|s: &mut AppState| {
            s.open_select = if s.open_select.as_deref() == Some("flt-branch") {
                None
            } else {
                Some("flt-branch".into())
            };
        })
        .on_close(|s: &mut AppState| s.open_select = None)
        .on_choose(|s: &mut AppState, v: &str, _| {
            s.filter_branch = if v == "Todas branches" {
                None
            } else {
                Some(v.to_string())
            };
            s.open_select = None;
        })
        .build(k, layer)
        .cursor(Cursor::Pointer);

    let sel_actor = Select::new("flt-actor", &actor_opts)
        .label("Autor")
        .appearance(SelectAppearance::Field)
        .width(154.0)
        .placement(Placement::BottomStart)
        .current(Some(
            s.filter_actor.as_deref().unwrap_or("Todos autores"),
        ))
        .open(s.open_select.as_deref() == Some("flt-actor"))
        .on_toggle(|s: &mut AppState| {
            s.open_select = if s.open_select.as_deref() == Some("flt-actor") {
                None
            } else {
                Some("flt-actor".into())
            };
        })
        .on_close(|s: &mut AppState| s.open_select = None)
        .on_choose(|s: &mut AppState, v: &str, _| {
            s.filter_actor = if v == "Todos autores" {
                None
            } else {
                Some(v.to_string())
            };
            s.open_select = None;
        })
        .build(k, layer)
        .cursor(Cursor::Pointer);

    let has_filters = !s.search_runs.text.trim().is_empty()
        || s.filter_status != StatusFilter::All
        || s.filter_event.is_some()
        || s.filter_branch.is_some()
        || s.filter_actor.is_some();

    let mut filter_bar = row()
        .key("filter-bar")
        .w_full()
        .gap(8.0)
        .center_y()
        .flex_wrap()
        .child(search_input)
        .child(sel_status)
        .child(sel_event)
        .child(sel_branch)
        .child(sel_actor)
        .child(chip(
            t,
            "chip-autorefresh",
            if s.auto_refresh {
                "Atualização automática: Ligada"
            } else {
                "Atualização automática: Pausada"
            },
            s.auto_refresh,
            |s: &mut AppState, _| s.auto_refresh = !s.auto_refresh,
        ));

    if has_filters {
        filter_bar = filter_bar.child(btn_ghost(
            t,
            "btn-clear-filters",
            "Limpar filtros",
            Some(&icons::X_CIRCLE),
            30.0,
            |s: &mut AppState, _| {
                s.search_runs.clear();
                s.filter_status = StatusFilter::All;
                s.filter_event = None;
                s.filter_branch = None;
                s.filter_actor = None;
            },
        ));
    }

    // Filter runs
    let q = s.search_runs.text.trim().to_lowercase();
    let filtered: Vec<&Run> = s
        .snapshot
        .runs
        .iter()
        .filter(|r| wf_filter.is_none_or(|id| r.workflow_id == id))
        .filter(|r| s.filter_status.matches(r.status))
        .filter(|r| {
            s.filter_event
                .as_ref()
                .is_none_or(|ev| &r.trigger_label == ev)
        })
        .filter(|r| {
            s.filter_branch
                .as_ref()
                .is_none_or(|br| &r.branch == br)
        })
        .filter(|r| s.filter_actor.as_ref().is_none_or(|ac| &r.actor == ac))
        .filter(|r| {
            if q.is_empty() {
                return true;
            }
            r.title.to_lowercase().contains(&q)
                || r.workflow_name.to_lowercase().contains(&q)
                || r.actor.to_lowercase().contains(&q)
                || r.branch.to_lowercase().contains(&q)
                || r.commit
                    .as_deref()
                    .is_some_and(|c| c.to_lowercase().contains(&q))
                || format!("#{}", r.number).contains(&q)
        })
        .collect();

    let list_body = if filtered.is_empty() {
        col()
            .key("runs-empty")
            .w_full()
            .py(48.0)
            .px(24.0)
            .center()
            .gap(10.0)
            .radius(12.0)
            .fill(t.surface_1)
            .border(1.0, t.border_subtle)
            .child(icon(&icons::LIST, 24.0).color(t.text_muted))
            .child(
                text(if has_filters {
                    "Nenhuma execução encontrada para os filtros aplicados"
                } else {
                    "Nenhuma execução registrada ainda"
                })
                .font(ty::H3)
                .color(t.text_primary),
            )
            .child(
                text(if has_filters {
                    "Ajuste a busca ou limpe os filtros para ver todo o histórico."
                } else {
                    "Clique em 'Executar fluxo' acima para disparar a primeira execução."
                })
                .font(ty::SMALL)
                .color(t.text_muted),
            )
            .child(if has_filters {
                btn_neutral(
                    t,
                    "empty-clear",
                    "Limpar filtros",
                    None,
                    32.0,
                    |s: &mut AppState, _| {
                        s.search_runs.clear();
                        s.filter_status = StatusFilter::All;
                        s.filter_event = None;
                        s.filter_branch = None;
                        s.filter_actor = None;
                    },
                )
            } else {
                el()
            })
    } else {
        let mut stack = col().key("runs-stack").w_full().gap(8.0);
        for r in filtered {
            stack = stack.child(run_row(s, t, r, wf_filter.is_none()));
        }
        stack
    };

    col()
        .key("runs-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(20.0)
        .pb(16.0)
        .gap(14.0)
        .transition(Transition::opacity())
        .child(hdr_card)
        .child(filter_bar)
        .child(
            col()
                .key("runs-scroll")
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .scroll(ScrollSpec { stick_bottom: false })
                .scroll_edge_fade(16.0)
                .pb(16.0)
                .child(list_body),
        )
}

fn run_row(_s: &AppState, t: UiTheme, r: &Run, show_wf_name: bool) -> El {
    let rid_open = r.id.clone();
    let rid_logs = r.id.clone();
    let rid_approve = r.id.clone();
    let rid_reject = r.id.clone();
    let rid_cancel = r.id.clone();
    let wf_rerun = r.workflow_id.clone();

    let title = if r.title.is_empty() {
        r.workflow_name.clone()
    } else {
        r.title.clone()
    };

    let age = r
        .started_ts
        .map(model::stamp_age)
        .unwrap_or_else(|| r.started_at.clone());
    let dur = model::fmt_dur(r.elapsed_secs());

    let mut meta = row().gap(8.0).center_y().flex_wrap();
    meta = meta.child(status_pill(t, r.status));
    if show_wf_name {
        meta = meta.child(
            text(r.workflow_name.clone())
                .font(ty::CAPTION)
                .color(t.accent),
        );
    }
    meta = meta.child(
        text(format!("#{} · {}", r.number, r.trigger_label))
            .font(ty::CAPTION)
            .color(t.text_muted),
    );
    if !r.branch.is_empty() {
        meta = meta.child(
            row()
                .h(20.0)
                .px(7.0)
                .gap(4.0)
                .center_y()
                .radius(5.0)
                .fill(t.surface_2)
                .child(icon(&icons::BRANCH, 10.5).color(t.text_muted))
                .child(text(r.branch.clone()).font(ty::MONO_SM).color(t.text_secondary)),
        );
    }
    if let Some(sha) = &r.commit {
        meta = meta.child(copyable_sha(t, format!("sha-{}", r.id), sha).hover_alone());
    }
    if !r.actor.is_empty() {
        meta = meta.child(
            text(format!("por {}", r.actor))
                .font(ty::CAPTION)
                .color(t.text_muted),
        );
    }

    let mut right_actions = row().gap(8.0).center_y();
    match r.status {
        RunStatus::AwaitingApproval => {
            right_actions = right_actions
                .child(
                    primary_button(
                        t,
                        format!("row-approve-{}", r.id),
                        "Aprovar",
                        Some(&icons::CHECK_CIRCLE),
                        28.0,
                        move |s: &mut AppState, tx: &mut Tx| s.approve_run(&rid_approve, tx),
                    )
                    .hover_alone(),
                )
                .child(
                    btn_ghost(
                        t,
                        format!("row-reject-{}", r.id),
                        "Rejeitar",
                        None,
                        28.0,
                        move |s: &mut AppState, tx: &mut Tx| {
                            s.cancel_or_reject_run(&rid_reject, true, tx)
                        },
                    )
                    .hover_alone(),
                );
        }
        RunStatus::Running | RunStatus::Queued => {
            right_actions = right_actions.child(
                btn_danger(
                    t,
                    format!("row-cancel-{}", r.id),
                    "Cancelar",
                    None,
                    28.0,
                    move |s: &mut AppState, tx: &mut Tx| {
                        s.cancel_or_reject_run(&rid_cancel, false, tx)
                    },
                )
                .hover_alone(),
            );
        }
        _ => {
            right_actions = right_actions
                .child(
                    btn_icon(
                        t,
                        format!("row-rerun-{}", r.id),
                        "Re-executar fluxo",
                        &icons::REFRESH,
                        28.0,
                        move |s: &mut AppState, tx: &mut Tx| {
                            s.start_workflow(&wf_rerun, "re-execução manual", tx)
                        },
                    )
                    .hover_alone(),
                )
                .child(
                    btn_icon(
                        t,
                        format!("row-logs-{}", r.id),
                        "Abrir logs",
                        &icons::CODE,
                        28.0,
                        move |s: &mut AppState, _| s.open_run_logs(&rid_logs, 0),
                    )
                    .hover_alone(),
                );
        }
    }

    let right_cluster = row()
        .gap(12.0)
        .center_y()
        .child(
            col()
                .items_end()
                .w(88.0)
                .gap(2.0)
                .child(text(dur).font(ty::MONO_SM).color(t.text_secondary))
                .child(text(age).font(ty::CAPTION).color(t.text_muted)),
        )
        .child(right_actions);

    let content = row()
        .w_full()
        .px(16.0)
        .py(12.0)
        .gap(12.0)
        .center_y()
        .flex_wrap()
        .child(
            row()
                .grow(1.0)
                .shrink()
                .min_w(160.0)
                .gap(12.0)
                .center_y()
                .child(run_status_icon(t, r.status, 18.0))
                .child(
                    col()
                        .grow(1.0)
                        .shrink()
                        .min_w(0.0)
                        .gap(5.0)
                        .child(
                            text(title)
                                .font(ty::BODY_MED)
                                .color(t.text_primary)
                                .w_full()
                                .truncate(),
                        )
                        .child(meta),
                ),
        )
        .child(right_cluster);

    row()
        .key(format!("run-row-{}", r.id))
        .role("button")
        .label(format!("Abrir execução {} #{}", r.workflow_name, r.number))
        .w_full()
        .min_h(64.0)
        .radius(8.0)
        .fill(t.surface_1)
        .border(
            1.0,
            match r.status {
                RunStatus::Failed => t.danger.alpha(0.35),
                RunStatus::AwaitingApproval => t.warning.alpha(0.38),
                _ => t.border_subtle,
            },
        )
        .cursor(Cursor::Pointer)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent)
        .hover(t.hover)
        .press(t.press)
        .press_scale(0.995)
        .motion(motion::SNAPPY)
        .on_press(move |s: &mut AppState| s.open_run_summary(&rid_open))
        .child(content)
}
