use std::collections::BTreeSet;

use rune::tok::motion;
use rune::{Cursor, El, ScrollSpec, Transition, Tx, col, effect, el, icon, row, text};
use rune_kit::Placement;
use rune_kit::{Kit, Select, SelectAppearance, TextInput};
use workploy::model::{self, PlatformTarget, RepoBinding, Run, RunStatus, StepStatus, TriggerMode};

use crate::gfx::{MARK_PULSE_RADAR, PULSE_INTENSITY_ACTIVE, PULSE_TEMPO_ACTIVE};
use crate::state::{AppState, ModalState, SettingsTab, StatusFilter, platform_compat_warning};
use crate::ui::components::{
    active_electric_effect, btn_danger, btn_ghost, btn_icon, btn_neutral, chip, copyable_sha,
    header_card, pill_badge, primary_button, run_progress_strip, run_status_icon, status_pill,
    surface_card,
};
use crate::ui::icons;
use crate::ui::theme::{UiTheme, ty};

pub fn view(
    s: &AppState,
    t: UiTheme,
    k: &Kit,
    wf_filter: Option<&str>,
    main_w: f32,
    layer: &mut Vec<El>,
) -> El {
    let wf_filter = wf_filter.or(s.selected_workflow_id.as_deref());
    let wf = wf_filter.and_then(|id| s.snapshot.workflows.iter().find(|w| w.id == id));
    let selected_project = if wf.is_none() {
        s.selected_project_id.as_deref().and_then(|pid| {
            s.snapshot
                .settings
                .repo_bindings
                .iter()
                .find(|b| b.id == pid)
        })
    } else {
        None
    };

    let wf_word = if s.snapshot.workflows.len() == 1 {
        "receita configurada"
    } else {
        "receitas configuradas"
    };
    let run_word = if s.snapshot.runs.len() == 1 {
        "execução registrada"
    } else {
        "execuções registradas"
    };

    // Header card
    let (title, subtitle) = if let Some(w) = wf {
        let step_word = if w.steps.len() == 1 { "passo" } else { "passos" };
        (
            w.name.clone(),
            format!(
                "Gatilho: {}  ·  Receita reutilizável ({} {step_word})",
                w.trigger.label(),
                w.steps.len()
            ),
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
            .child(
                primary_button(
                    t,
                    "btn-run-wf",
                    "Executar fluxo",
                    Some(&icons::PLAY),
                    34.0,
                    move |s: &mut AppState, tx| s.start_workflow(&wid_run, "manual", tx),
                )
                .help("Dispara imediatamente uma execução manual desta receita de fluxo."),
            )
            .child(
                btn_neutral(
                    t,
                    "btn-edit-wf",
                    "Editar fluxo",
                    Some(&icons::EDIT),
                    34.0,
                    move |s: &mut AppState, _| s.open_edit_workflow_modal(&wid_edit),
                )
                .help("Abre o editor para alterar nome, gatilho, pasta de trabalho, encadeamento e passos."),
            )
            .child(
                btn_neutral(
                    t,
                    "btn-yaml-wf",
                    "Arquivo YAML",
                    Some(&icons::CODE),
                    34.0,
                    move |s: &mut AppState, _| s.open_yaml_modal(Some(&wid_yaml)),
                )
                .help("Visualiza, copia ou edita a definição deste fluxo em formato YAML."),
            )
            .child(
                btn_danger(
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
                )
                .help("Remove este fluxo de trabalho (o histórico de execuções anteriores é mantido)."),
            );
    } else {
        if let Some(primary) = s.snapshot.workflows.first() {
            let wid = primary.id.clone();
            header_actions = header_actions.child(
                primary_button(
                    t,
                    "btn-run-primary",
                    "Executar fluxo",
                    Some(&icons::PLAY),
                    34.0,
                    move |s: &mut AppState, tx| s.start_workflow(&wid, "manual", tx),
                )
                .help("Dispara imediatamente uma execução manual do fluxo principal."),
            );
        }
        header_actions = header_actions
            .child(
                btn_neutral(
                    t,
                    "btn-new-wf-top",
                    "Novo fluxo",
                    Some(&icons::WORKFLOW),
                    34.0,
                    |s: &mut AppState, _| s.open_new_workflow_modal(),
                )
                .help("Abre o editor para criar um novo fluxo de trabalho."),
            )
            .child(
                btn_neutral(
                    t,
                    "btn-yaml-import",
                    "Importar YAML",
                    Some(&icons::CODE),
                    34.0,
                    |s: &mut AppState, _| s.open_yaml_modal(None),
                )
                .help("Cria ou atualiza um fluxo colando uma definição YAML."),
            );
    }

    let title_el = if let Some(w) = wf {
        row()
            .w_full()
            .min_w(0.0)
            .gap(8.0)
            .center_y()
            .child(
                icon(icons::workflow_icon(&w.name), 18.0)
                    .color(t.accent)
                    .a11y_hidden(),
            )
            .child(
                text(title)
                    .font(ty::H1)
                    .color(t.text_primary)
                    .shrink()
                    .min_w(0.0)
                    .truncate(),
            )
    } else {
        text(title).font(ty::H1).color(t.text_primary).truncate()
    };

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
                    .child(title_el)
                    .child(text(subtitle).font(ty::SMALL).color(t.text_muted).truncate()),
            )
            .child(header_actions),
    );

    // Filter options collected from current runs
    let mut events_set = BTreeSet::new();
    let mut branches_set = BTreeSet::new();
    let mut actors_set = BTreeSet::new();
    for r in &s.snapshot.runs {
        let proj_ok = selected_project.is_none_or(|b| {
            let repo_trim = b.full_name.trim();
            r.repo_name
                .as_deref()
                .is_some_and(|rn| rn == b.full_name.as_str() || rn.eq_ignore_ascii_case(repo_trim))
                || (!repo_trim.is_empty() && r.trigger_label.contains(repo_trim))
        });
        if wf_filter.is_none_or(|id| r.workflow_id == id) && proj_ok {
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

    filter_bar = filter_bar.child(
        btn_neutral(
            t,
            "btn-clear-runs",
            "Limpar histórico",
            Some(&icons::TRASH),
            30.0,
            |s: &mut AppState, tx: &mut Tx| s.clear_finished_runs_history(tx),
        )
        .help("Remove execuções antigas já finalizadas do histórico."),
    );

    // Filter runs
    let q = s.search_runs.text.trim().to_lowercase();
    let filtered: Vec<&Run> = s
        .snapshot
        .runs
        .iter()
        .filter(|r| wf_filter.is_none_or(|id| r.workflow_id == id))
        .filter(|r| {
            selected_project.is_none_or(|b| {
                let repo_trim = b.full_name.trim();
                r.repo_name.as_deref().is_some_and(|rn| {
                    rn == b.full_name.as_str() || rn.eq_ignore_ascii_case(repo_trim)
                }) || (!repo_trim.is_empty() && r.trigger_label.contains(repo_trim))
            })
        })
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

    let top_hdr = if let Some(b) = selected_project {
        project_dashboard_header(s, t, b)
    } else {
        hdr_card
    };

    let mut page = col()
        .key("runs-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(20.0)
        .pb(16.0)
        .gap(14.0)
        .transition(Transition::opacity())
        .child(top_hdr);

    if let Some(w) = wf {
        // Aviso amarelo quando o yml no repo mudou desde a importação:
        // banner compacto com a mensagem e "Atualizar do repo" direto.
        if let Some((_, _, drift_msg)) = s
            .snapshot
            .workflow_drift
            .iter()
            .find(|(id, _, _)| id == &w.id)
        {
            let wid_up = w.id.clone();
            let drift_msg = drift_msg.clone();
            page = page.child(
                row()
                    .key("wf-drift-banner")
                    .w_full()
                    .min_w(0.0)
                    .px(14.0)
                    .py(10.0)
                    .gap(10.0)
                    .center_y()
                    .flex_wrap()
                    .radius(9.0)
                    .fill(t.warning_bg)
                    .border(1.0, t.warning.alpha(0.4))
                    .child(
                        icon(&icons::REFRESH, 15.0)
                            .color(t.warning)
                            .a11y_hidden(),
                    )
                    .child(
                        text(drift_msg)
                            .font(ty::SMALL)
                            .color(t.warning)
                            .grow(1.0)
                            .shrink()
                            .min_w(0.0)
                            .wrap(),
                    )
                    .child(
                        btn_neutral(
                            t,
                            format!("wf-drift-upd-repo-{}", w.id),
                            "Atualizar do repo",
                            Some(&icons::REFRESH),
                            28.0,
                            move |s: &mut AppState, tx: &mut Tx| {
                                s.update_workflow_from_repo(&wid_up, tx);
                            },
                        )
                        .help("Reimporta a versão atual do arquivo no repositório, preservando id, encadeamento e vínculos."),
                    ),
            );
        }
        page = page.child(workflow_pipeline_summary(s, t, w));
        return page.child(filter_bar).child(
            col()
                .key("runs-scroll")
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .scroll(ScrollSpec { stick_bottom: false })
                .scroll_edge_fade(16.0)
                .pb(16.0)
                .child(list_body),
        );
    }

    if let Some(b) = selected_project {
        return page.child(
            col()
                .key("runs-scroll")
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .scroll(ScrollSpec { stick_bottom: false })
                .scroll_edge_fade(16.0)
                .gap(16.0)
                .pb(16.0)
                .child(project_kpi_strip(s, t, b, main_w))
                .child(project_workflows_section(s, t, b))
                .child(
                    col()
                        .key("runs-history-section")
                        .w_full()
                        .min_w(0.0)
                        .gap(10.0)
                        .child(
                            text("Histórico de execuções")
                                .font(ty::SMALL_MED)
                                .color(t.text_secondary),
                        )
                        .child(filter_bar)
                        .child(list_body),
                ),
        );
    }

    page.child(
        col()
            .key("runs-scroll")
            .grow(1.0)
            .min_h(0.0)
            .w_full()
            .scroll(ScrollSpec { stick_bottom: false })
            .scroll_edge_fade(16.0)
            .gap(16.0)
            .pb(16.0)
            .child(projects_section(s, t, main_w))
            .child(
                col()
                    .key("runs-history-section")
                    .w_full()
                    .min_w(0.0)
                    .gap(10.0)
                    .child(
                        text("Histórico de execuções")
                            .font(ty::SMALL_MED)
                            .color(t.text_secondary),
                    )
                    .child(filter_bar)
                    .child(list_body),
            ),
    )
}

fn projects_section(s: &AppState, t: UiTheme, main_w: f32) -> El {
    let active_bindings: Vec<&RepoBinding> = s
        .snapshot
        .settings
        .repo_bindings
        .iter()
        .filter(|b| b.enabled)
        .collect();

    let count_label = if active_bindings.is_empty() {
        "Nenhum ativo".to_string()
    } else if active_bindings.len() == 1 {
        "1 ativo".to_string()
    } else {
        format!("{} ativos", active_bindings.len())
    };

    let hdr = row()
        .w_full()
        .min_w(0.0)
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
                    text("Projetos & Repositórios configurados")
                        .font(ty::H3)
                        .color(t.text_primary),
                )
                .child(pill_badge(
                    t,
                    &count_label,
                    if active_bindings.is_empty() {
                        t.text_muted
                    } else {
                        t.accent
                    },
                    if active_bindings.is_empty() {
                        t.surface_2
                    } else {
                        t.accent_soft
                    },
                )),
        )
        .child(
            btn_ghost(
                t,
                "btn-projects-manage",
                "Gerenciar vínculos",
                Some(&icons::SLIDERS),
                28.0,
                |s: &mut AppState, _| s.open_settings(SettingsTab::GitHub),
            )
            .help("Abre Configurações → GitHub para adicionar, pausar ou editar vínculos."),
        );

    let body = if active_bindings.is_empty() {
        let title = if s.snapshot.settings.repo_bindings.is_empty() {
            "Nenhum repositório vinculado ainda"
        } else {
            "Todos os vínculos configurados estão pausados"
        };
        row()
            .key("projects-empty")
            .w_full()
            .min_w(0.0)
            .px(16.0)
            .py(14.0)
            .gap(12.0)
            .center_y()
            .justify_between()
            .flex_wrap()
            .radius(10.0)
            .fill(t.surface_1)
            .border(1.0, t.border_subtle)
            .child(
                row()
                    .grow(1.0)
                    .shrink()
                    .min_w(180.0)
                    .gap(10.0)
                    .center_y()
                    .child(icon(&icons::BRANCH, 16.0).color(t.text_muted).a11y_hidden())
                    .child(
                        col()
                            .grow(1.0)
                            .shrink()
                            .min_w(0.0)
                            .gap(2.0)
                            .child(
                                text(title)
                                    .font(ty::BODY_MED)
                                    .color(t.text_primary)
                                    .wrap(),
                            )
                            .child(
                                text("Configure em Configurações → GitHub qual repositório, branch, destino (VPS ou Local) e receita executar.")
                                    .font(ty::CAPTION)
                                    .color(t.text_muted)
                                    .wrap(),
                            ),
                    ),
            )
            .child(
                btn_neutral(
                    t,
                    "btn-projects-goto-github",
                    "Configurações → GitHub",
                    Some(&icons::BRANCH),
                    30.0,
                    |s: &mut AppState, _| s.open_settings(SettingsTab::GitHub),
                )
                .help("Ir para Configurações → GitHub e vincular seu primeiro repositório."),
            )
    } else if main_w < 760.0 {
        let mut list = col().key("projects-grid").w_full().min_w(0.0).gap(10.0);
        for b in active_bindings {
            list = list.child(project_card(s, t, b, main_w));
        }
        list
    } else {
        let mut grid = col().key("projects-grid").w_full().min_w(0.0).gap(10.0);
        for pair in active_bindings.chunks(2) {
            let mut r = row().w_full().min_w(0.0).gap(10.0).items_stretch();
            r = r.child(project_card(s, t, pair[0], main_w).grow(1.0).shrink().min_w(0.0));
            if let Some(second) = pair.get(1) {
                r = r.child(project_card(s, t, second, main_w).grow(1.0).shrink().min_w(0.0));
            } else {
                r = r.child(el().grow(1.0).shrink().min_w(0.0));
            }
            grid = grid.child(r);
        }
        grid
    };

    col()
        .key("projects-section")
        .w_full()
        .min_w(0.0)
        .gap(10.0)
        .child(hdr)
        .child(body)
}

fn project_card(s: &AppState, t: UiTheme, b: &RepoBinding, main_w: f32) -> El {
    let repo_trim = b.full_name.trim();
    let repo_matches = |r: &Run| -> bool {
        r.repo_name
            .as_deref()
            .is_some_and(|rn| rn.eq_ignore_ascii_case(repo_trim))
            || (!repo_trim.is_empty() && r.trigger_label.contains(repo_trim))
    };

    let running_run = s
        .snapshot
        .runs
        .iter()
        .find(|r| repo_matches(r) && matches!(r.status, RunStatus::Queued | RunStatus::Running));
    let approval_run = if running_run.is_none() {
        s.snapshot
            .runs
            .iter()
            .find(|r| repo_matches(r) && r.status == RunStatus::AwaitingApproval)
    } else {
        None
    };
    let last_finished = s
        .snapshot
        .runs
        .iter()
        .find(|r| repo_matches(r) && !r.status.active());

    let border_col = if running_run.is_some() {
        t.info.alpha(0.42)
    } else if approval_run.is_some() {
        t.warning.alpha(0.45)
    } else {
        t.border_subtle
    };

    let status_badge = if running_run.is_some() {
        let pulse_dot = el()
            .size(8.0, 8.0)
            .center()
            .child(
                effect(MARK_PULSE_RADAR)
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .radius(4.0)
                    .effect_bleed(7.0)
                    .effect_data([4.0, 7.0, PULSE_TEMPO_ACTIVE, PULSE_INTENSITY_ACTIVE]),
            )
            .child(el().size(7.0, 7.0).radius(3.5).fill(t.info));
        row()
            .key(format!("project-status-{}", b.id))
            .h(22.0)
            .px(8.0)
            .gap(6.0)
            .center_y()
            .radius(11.0)
            .fill(t.info_bg)
            .border(1.0, t.info.alpha(0.32))
            .child(pulse_dot)
            .child(text("Rodando").font(ty::CAPTION).color(t.info))
    } else if approval_run.is_some() {
        row()
            .key(format!("project-status-{}", b.id))
            .h(22.0)
            .px(8.0)
            .gap(5.0)
            .center_y()
            .radius(11.0)
            .fill(t.warning_bg)
            .border(1.0, t.warning.alpha(0.38))
            .child(icon(&icons::BOLT, 11.5).color(t.warning))
            .child(
                text("Aguardando aprovação")
                    .font(ty::CAPTION)
                    .color(t.warning),
            )
    } else {
        row()
            .key(format!("project-status-{}", b.id))
            .h(22.0)
            .px(8.0)
            .gap(6.0)
            .center_y()
            .radius(11.0)
            .fill(t.success_bg)
            .border(1.0, t.success.alpha(0.28))
            .child(el().size(6.0, 6.0).radius(3.0).fill(t.success))
            .child(
                text("Em standby · aguardando push")
                    .font(ty::CAPTION)
                    .color(t.success),
            )
    };

    let target_label = match b.target_server_id.as_deref() {
        None => "Local (Esta máquina)".to_string(),
        Some(sid) => {
            let srv = s.snapshot.settings.servers.iter().find(|v| v.id == sid);
            match srv {
                Some(v) if v.runner_installed => format!("VPS: {} · Runner 24/7", v.name),
                Some(v) => format!("VPS: {} · SSH direto", v.name),
                None => "VPS removida".to_string(),
            }
        }
    };
    let mode_label = match b.trigger_mode {
        TriggerMode::WebhookOnVps => "Webhook 24/7",
        TriggerMode::PollOnVps => "Sondagem 24/7",
        TriggerMode::PollOnDesktop => "Desktop",
    };
    let recipe_label = b
        .workflow_id
        .as_deref()
        .and_then(|wid| s.snapshot.workflows.iter().find(|w| w.id == wid))
        .map(|w| w.name.as_str())
        .unwrap_or("Automático");

    let branch_label = if b.branch.trim().is_empty() {
        "main"
    } else {
        b.branch.trim()
    };

    let footer_left = if let Some(active) = running_run.or(approval_run) {
        text(format!("#{} · {}", active.number, active.workflow_name))
            .font(ty::CAPTION)
            .color(t.text_secondary)
            .truncate()
    } else if let Some(last) = last_finished {
        let (lbl, col) = match last.status {
            RunStatus::Success => ("Última: Sucesso", t.success),
            RunStatus::Failed => ("Última: Falhou", t.danger),
            RunStatus::Cancelled => ("Última: Cancelada", t.text_muted),
            _ => ("Última: Concluída", t.text_secondary),
        };
        let age = last
            .started_ts
            .map(model::stamp_age)
            .unwrap_or_default();
        let mut r = row().gap(6.0).center_y().flex_wrap().child(
            text(lbl).font(ty::CAPTION).color(col),
        );
        if !age.is_empty() {
            r = r.child(text(format!("· {age}")).font(ty::CAPTION).color(t.text_muted));
        }
        r
    } else {
        let hint = if b.last_sha.is_empty() {
            "Aguardando primeiro push".to_string()
        } else {
            let short = &b.last_sha[..7.min(b.last_sha.len())];
            format!("Commit atual: {short}")
        };
        text(hint).font(ty::CAPTION).color(t.text_muted).truncate()
    };

    let bid_open = b.id.clone();
    let bid_run = b.id.clone();
    let bid_commits = b.id.clone();

    let mut badges_row = row()
        .w_full()
        .min_w(0.0)
        .gap(6.0)
        .center_y()
        .flex_wrap()
        .child(pill_badge(
            t,
            &format!("Destino: {target_label}"),
            t.accent,
            t.accent_soft,
        ))
        .child(pill_badge(
            t,
            &format!("Modo: {mode_label}"),
            t.text_secondary,
            t.surface_2,
        ))
        .child(pill_badge(
            t,
            &format!("Receita: {recipe_label}"),
            t.text_secondary,
            t.surface_2,
        ));
    if b.platform != PlatformTarget::CrossPlatform {
        badges_row = badges_row.child(
            pill_badge(t, b.platform.label(), t.text_secondary, t.surface_2)
                .key(format!("project-platform-{}", b.id)),
        );
    }

    let mut card = col()
        .key(format!("project-card-{}", b.id))
        .w_full()
        .min_w(0.0)
        .padding(14.0)
        .gap(10.0)
        .radius(10.0)
        .fill(t.surface_1)
        .border(1.0, border_col)
        .child(
            row()
                .w_full()
                .min_w(0.0)
                .justify_between()
                .center_y()
                .flex_wrap()
                .gap(8.0)
                .child(
                    row()
                        .grow(1.0)
                        .shrink()
                        .min_w(140.0)
                        .gap(7.0)
                        .center_y()
                        .child(icon(&icons::BRANCH, 14.0).color(t.accent).a11y_hidden())
                        .child(
                            text(b.full_name.clone())
                                .font(ty::BODY_MED)
                                .color(t.text_primary)
                                .shrink()
                                .min_w(0.0)
                                .truncate(),
                        )
                        .child(
                            row()
                                .h(20.0)
                                .px(6.0)
                                .center_y()
                                .radius(5.0)
                                .fill(t.surface_2)
                                .child(
                                    text(branch_label.to_string())
                                        .font(ty::MONO_SM)
                                        .color(t.text_secondary),
                                ),
                        ),
                )
                .child(status_badge),
        )
        .child(badges_row);

    if let Some(warn_msg) = platform_compat_warning(b.platform, b.target_server_id.as_deref()) {
        card = card.child(
            row()
                .key(format!("project-platform-warn-{}", b.id))
                .w_full()
                .min_w(0.0)
                .px(10.0)
                .py(7.0)
                .gap(7.0)
                .center_y()
                .radius(7.0)
                .fill(t.warning_bg)
                .border(1.0, t.warning.alpha(0.38))
                .child(icon(&icons::BOLT, 13.0).color(t.warning).a11y_hidden())
                .child(
                    text(warn_msg)
                        .font(ty::CAPTION)
                        .color(t.warning)
                        .grow(1.0)
                        .shrink()
                        .min_w(0.0)
                        .wrap(),
                ),
        );
    }

    let mut actions_row = row()
        .min_w(0.0)
        .shrink()
        .gap(6.0)
        .center_y()
        .flex_wrap()
        .child(
            btn_neutral(
                t,
                format!("open-proj-{}", b.id),
                "Abrir projeto",
                Some(&icons::CHEVRON_RIGHT),
                28.0,
                move |s: &mut AppState, _| {
                    s.selected_project_id = Some(bid_open.clone());
                    s.selected_workflow_id = None;
                },
            )
            .help("Abre o dashboard deste projeto com KPIs, fluxos vinculados e histórico filtrado."),
        )
        .child(
            btn_neutral(
                t,
                format!("commits-binding-{}", b.id),
                "Ver commits",
                Some(&icons::COMMIT),
                28.0,
                move |s: &mut AppState, tx: &mut Tx| {
                    s.open_repo_commits_modal(&bid_commits, tx);
                },
            )
            .help("Lista os últimos commits da branch e permite disparar o fluxo para um commit específico."),
        )
        .child(
            primary_button(
                t,
                format!("run-binding-{}", b.id),
                "Executar agora",
                Some(&icons::PLAY),
                28.0,
                move |s: &mut AppState, tx: &mut Tx| {
                    s.run_binding_now(&bid_run, tx);
                },
            )
            .help("Dispara imediatamente uma execução para este projeto usando a receita e o destino configurados."),
        );
    if main_w < 600.0 {
        actions_row = actions_row.w_full();
    }

    card.child(
        row()
            .w_full()
            .min_w(0.0)
            .justify_between()
            .center_y()
            .flex_wrap()
            .gap(8.0)
            .child(
                row()
                    .grow(1.0)
                    .shrink()
                    .min_w(120.0)
                    .center_y()
                    .child(footer_left),
            )
            .child(actions_row),
    )
}

fn project_dashboard_header(s: &AppState, t: UiTheme, b: &RepoBinding) -> El {
    let repo_trim = b.full_name.trim();
    let repo_matches = |r: &Run| -> bool {
        r.repo_name
            .as_deref()
            .is_some_and(|rn| rn.eq_ignore_ascii_case(repo_trim))
            || (!repo_trim.is_empty() && r.trigger_label.contains(repo_trim))
    };
    let running_run = s
        .snapshot
        .runs
        .iter()
        .find(|r| repo_matches(r) && matches!(r.status, RunStatus::Queued | RunStatus::Running));
    let approval_run = if running_run.is_none() {
        s.snapshot
            .runs
            .iter()
            .find(|r| repo_matches(r) && r.status == RunStatus::AwaitingApproval)
    } else {
        None
    };

    let status_badge = if running_run.is_some() {
        let pulse_dot = el()
            .size(8.0, 8.0)
            .center()
            .child(
                effect(MARK_PULSE_RADAR)
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .radius(4.0)
                    .effect_bleed(7.0)
                    .effect_data([4.0, 7.0, PULSE_TEMPO_ACTIVE, PULSE_INTENSITY_ACTIVE]),
            )
            .child(el().size(7.0, 7.0).radius(3.5).fill(t.info));
        row()
            .key(format!("project-status-{}", b.id))
            .h(22.0)
            .px(8.0)
            .gap(6.0)
            .center_y()
            .radius(11.0)
            .fill(t.info_bg)
            .border(1.0, t.info.alpha(0.32))
            .child(pulse_dot)
            .child(text("Rodando").font(ty::CAPTION).color(t.info))
    } else if approval_run.is_some() {
        row()
            .key(format!("project-status-{}", b.id))
            .h(22.0)
            .px(8.0)
            .gap(5.0)
            .center_y()
            .radius(11.0)
            .fill(t.warning_bg)
            .border(1.0, t.warning.alpha(0.38))
            .child(icon(&icons::BOLT, 11.5).color(t.warning))
            .child(
                text("Aguardando aprovação")
                    .font(ty::CAPTION)
                    .color(t.warning),
            )
    } else {
        row()
            .key(format!("project-status-{}", b.id))
            .h(22.0)
            .px(8.0)
            .gap(6.0)
            .center_y()
            .radius(11.0)
            .fill(t.success_bg)
            .border(1.0, t.success.alpha(0.28))
            .child(el().size(6.0, 6.0).radius(3.0).fill(t.success))
            .child(
                text("Em standby · aguardando push")
                    .font(ty::CAPTION)
                    .color(t.success),
            )
    };

    let branch_label = if b.branch.trim().is_empty() {
        "main"
    } else {
        b.branch.trim()
    };

    let bid_commits = b.id.clone();
    let bid_gen = b.id.clone();
    let bid_imp = b.id.clone();
    let bid_cfg = b.id.clone();
    let bid_run = b.id.clone();

    let header_actions = row()
        .min_w(0.0)
        .shrink()
        .gap(8.0)
        .center_y()
        .flex_wrap()
        .child(
            btn_neutral(
                t,
                "proj-back-all",
                "← Todos os projetos",
                Some(&icons::ARROW_LEFT),
                32.0,
                |s: &mut AppState, _| {
                    s.selected_project_id = None;
                    s.selected_workflow_id = None;
                },
            )
            .help("Volta para a visão geral com todos os projetos e execuções."),
        )
        .child(
            btn_neutral(
                t,
                format!("commits-binding-{}", b.id),
                "Ver commits",
                Some(&icons::COMMIT),
                32.0,
                move |s: &mut AppState, tx: &mut Tx| {
                    s.open_repo_commits_modal(&bid_commits, tx);
                },
            )
            .help("Lista os últimos commits da branch e permite disparar o fluxo para um commit específico."),
        )
        .child(
            btn_neutral(
                t,
                format!("gen-wf-binding-{}", b.id),
                "Gerar receita",
                Some(&icons::BOLT),
                32.0,
                move |s: &mut AppState, tx: &mut Tx| {
                    s.generate_workflow_for_binding(&bid_gen, tx);
                },
            )
            .help("Lê package.json, Cargo.toml e docker-compose.yml no GitHub para criar passos reais de testes e redeploy automático."),
        )
        .child(
            btn_neutral(
                t,
                format!("imp-repo-{}", b.id),
                "Importar do repositório",
                Some(&icons::DOCUMENTS),
                32.0,
                move |s: &mut AppState, tx: &mut Tx| {
                    s.open_import_from_repo_modal(&bid_imp, tx);
                },
            )
            .help("Busca o yml direto no repositório vinculado (.workploy.yml ou infra/workploy) — o repo vira a fonte da verdade."),
        )
        .child(
            btn_neutral(
                t,
                format!("cfg-binding-{}", b.id),
                "Configurar",
                Some(&icons::SLIDERS),
                32.0,
                move |s: &mut AppState, _| s.open_edit_binding_modal(&bid_cfg),
            )
            .help("Abre as configurações deste vínculo (branch, destino, modo e receita)."),
        )
        .child(
            primary_button(
                t,
                format!("run-binding-{}", b.id),
                "Executar agora",
                Some(&icons::PLAY),
                32.0,
                move |s: &mut AppState, tx: &mut Tx| {
                    s.run_binding_now(&bid_run, tx);
                },
            )
            .help("Dispara imediatamente uma execução para este projeto usando a receita e o destino configurados."),
        );

    let header_dest_badge = match b.target_server_id.as_deref() {
        None => pill_badge(t, "Destino: Local", t.text_secondary, t.surface_2),
        Some(sid) => {
            let srv = s.snapshot.settings.servers.iter().find(|v| v.id == sid);
            match srv {
                Some(v) if v.runner_installed => pill_badge(
                    t,
                    &format!("VPS: {} · Runner 24/7", v.name),
                    t.success,
                    t.success_bg,
                ),
                Some(v) => pill_badge(
                    t,
                    &format!("VPS: {} · SSH direto", v.name),
                    t.accent,
                    t.accent_soft,
                ),
                None => pill_badge(t, "VPS removida", t.danger, t.danger_bg),
            }
        }
    };

    let title_row = row()
        .w_full()
        .min_w(0.0)
        .gap(8.0)
        .center_y()
        .flex_wrap()
        .child(icon(&icons::BRANCH, 16.0).color(t.accent).a11y_hidden())
        .child(
            text(b.full_name.clone())
                .font(ty::H1)
                .color(t.text_primary)
                .shrink()
                .min_w(0.0)
                .truncate(),
        )
        .child(
            row()
                .h(22.0)
                .px(8.0)
                .center_y()
                .radius(5.0)
                .fill(t.surface_2)
                .child(
                    text(branch_label.to_string())
                        .font(ty::MONO_SM)
                        .color(t.text_secondary),
                ),
        )
        .child(
            pill_badge(t, b.platform.label(), t.text_secondary, t.surface_2)
                .key(format!("project-platform-{}", b.id)),
        )
        .child(header_dest_badge)
        .child(status_badge);

    let mut inner = col()
        .px(20.0)
        .py(16.0)
        .gap(12.0)
        .child(
            row()
                .w_full()
                .min_w(0.0)
                .gap(16.0)
                .center_y()
                .justify_between()
                .flex_wrap()
                .child(
                    col()
                        .grow(1.0)
                        .shrink()
                        .min_w(170.0)
                        .gap(4.0)
                        .child(title_row),
                )
                .child(header_actions),
        );

    if let Some(warn_msg) = platform_compat_warning(b.platform, b.target_server_id.as_deref()) {
        inner = inner.child(
            row()
                .key(format!("project-platform-warn-{}", b.id))
                .w_full()
                .min_w(0.0)
                .px(10.0)
                .py(7.0)
                .gap(7.0)
                .center_y()
                .radius(7.0)
                .fill(t.warning_bg)
                .border(1.0, t.warning.alpha(0.38))
                .child(icon(&icons::BOLT, 13.0).color(t.warning).a11y_hidden())
                .child(
                    text(warn_msg)
                        .font(ty::CAPTION)
                        .color(t.warning)
                        .grow(1.0)
                        .shrink()
                        .min_w(0.0)
                        .wrap(),
                ),
        );
    }

    header_card(t, "project-dashboard-hdr", 10.0, inner)
}

fn project_kpi_card(
    t: UiTheme,
    key: &str,
    label: &str,
    value: String,
    subtext: String,
    accent_color: rune::color::Color,
) -> El {
    surface_card(
        t,
        key,
        10.0,
        col()
            .px(14.0)
            .py(12.0)
            .gap(6.0)
            .justify_between()
            .child(
                text(label.to_string())
                    .font(ty::EYEBROW)
                    .color(t.text_muted)
                    .truncate(),
            )
            .child(
                text(value)
                    .font(ty::H3)
                    .color(accent_color)
                    .wrap(),
            )
            .child(
                text(subtext)
                    .font(ty::CAPTION)
                    .color(t.text_muted)
                    .wrap(),
            ),
    )
}

fn project_kpi_strip(s: &AppState, t: UiTheme, b: &RepoBinding, main_w: f32) -> El {
    let (total_runs, success_pct, avg_secs, estimated_resources_label) =
        model::compute_project_kpis(&s.snapshot.runs, &b.full_name, b.platform);

    let where_val = s.server_label(b.target_server_id.as_deref());
    let mode_label = match b.trigger_mode {
        TriggerMode::WebhookOnVps => "Webhook 24/7",
        TriggerMode::PollOnVps => "Sondagem 24/7",
        TriggerMode::PollOnDesktop => "Desktop",
    };
    let where_sub = b
        .target_server_id
        .as_deref()
        .and_then(|sid| s.snapshot.settings.servers.iter().find(|srv| srv.id == sid))
        .and_then(|srv| srv.last_probe.as_ref())
        .filter(|p| p.vcpus > 0 && p.ram_total_mb > 0)
        .map(|p| {
            let ram_gb = (p.ram_total_mb as f64 / 1024.0).round() as u64;
            format!("{mode_label} · {} vCPU · {ram_gb} GB RAM", p.vcpus)
        })
        .unwrap_or_else(|| mode_label.to_string());

    let avg_val = if total_runs > 0 {
        model::fmt_dur(avg_secs)
    } else {
        "—".to_string()
    };

    let success_val = format!("{success_pct}%");
    let success_sub = format!("{total_runs} execuções deste projeto");
    let success_col = if total_runs == 0 {
        t.text_primary
    } else if success_pct >= 80 {
        t.success
    } else if success_pct >= 50 {
        t.warning
    } else {
        t.danger
    };

    let cards = vec![
        project_kpi_card(
            t,
            "proj-kpi-where",
            "ONDE RODA",
            where_val,
            where_sub,
            t.text_primary,
        ),
        project_kpi_card(
            t,
            "proj-kpi-resources",
            "REQUISITOS ESTIMADOS",
            estimated_resources_label.to_string(),
            b.platform.label().to_string(),
            t.text_primary,
        ),
        project_kpi_card(
            t,
            "proj-kpi-avg-dur",
            "TEMPO MÉDIO",
            avg_val,
            "Média das execuções concluídas".to_string(),
            t.text_primary,
        ),
        project_kpi_card(
            t,
            "proj-kpi-success",
            "TAXA DE SUCESSO",
            success_val,
            success_sub,
            success_col,
        ),
    ];

    if main_w >= 860.0 {
        let mut r = row().key("proj-kpi-grid").w_full().min_w(0.0).gap(10.0).items_stretch();
        for c in cards {
            r = r.child(c.grow(1.0).shrink().min_w(0.0));
        }
        r
    } else if main_w >= 520.0 {
        let mut grid = col().key("proj-kpi-grid").w_full().min_w(0.0).gap(10.0);
        for chunk in cards.chunks(2) {
            let mut r = row().w_full().min_w(0.0).gap(10.0).items_stretch();
            for c in chunk {
                r = r.child(c.clone().grow(1.0).shrink().min_w(0.0));
            }
            grid = grid.child(r);
        }
        grid
    } else {
        let mut stack = col().key("proj-kpi-grid").w_full().min_w(0.0).gap(10.0);
        for c in cards {
            stack = stack.child(c.w_full().min_w(0.0));
        }
        stack
    }
}

fn project_workflows_section(s: &AppState, t: UiTheme, b: &RepoBinding) -> El {
    let workflows = s.project_workflows(b);
    let mut list = col().w_full().min_w(0.0).gap(8.0);

    if workflows.is_empty() {
        let bid_gen = b.id.clone();
        let bid_imp = b.id.clone();
        list = list.child(
            row()
                .key("proj-workflows-empty")
                .w_full()
                .min_w(0.0)
                .px(14.0)
                .py(12.0)
                .gap(10.0)
                .center_y()
                .justify_between()
                .flex_wrap()
                .radius(10.0)
                .fill(t.surface_1)
                .border(1.0, t.border_subtle)
                .child(
                    text("Nenhum fluxo vinculado a este projeto.")
                        .font(ty::SMALL)
                        .color(t.text_muted),
                )
                .child(
                    btn_neutral(
                        t,
                        format!("proj-empty-gen-wf-{}", b.id),
                        "Gerar receita",
                        Some(&icons::BOLT),
                        28.0,
                        move |s: &mut AppState, tx: &mut Tx| {
                            s.generate_workflow_for_binding(&bid_gen, tx);
                        },
                    )
                    .help("Gera automaticamente uma receita a partir dos arquivos do repositório."),
                )
                .child(
                    btn_neutral(
                        t,
                        format!("imp-repo-{}", b.id),
                        "Importar do repositório",
                        Some(&icons::DOCUMENTS),
                        28.0,
                        move |s: &mut AppState, tx: &mut Tx| {
                            s.open_import_from_repo_modal(&bid_imp, tx);
                        },
                    )
                    .help("Busca o yml direto no repositório vinculado (.workploy.yml ou infra/workploy) — o repo vira a fonte da verdade."),
                ),
        );
    } else {
        for wf in workflows {
            let step_count = wf.steps.len();
            let step_word = if step_count == 1 { "passo" } else { "passos" };
            let wid_open = wf.id.clone();
            let wid_edit = wf.id.clone();
            let wid_run = wf.id.clone();
            let bid_open = b.id.clone();

            let mut badges = row()
                .min_w(0.0)
                .shrink()
                .gap(6.0)
                .center_y()
                .flex_wrap()
                .child(pill_badge(
                    t,
                    &format!("{step_count} {step_word}"),
                    t.accent,
                    t.accent_soft,
                ))
                .child(pill_badge(
                    t,
                    &format!("Gatilho: {}", wf.trigger.label()),
                    t.text_secondary,
                    t.surface_2,
                ));
            if let Some(next_id) = &wf.on_success {
                let next_name = s
                    .snapshot
                    .workflows
                    .iter()
                    .find(|nw| &nw.id == next_id)
                    .map(|nw| nw.name.as_str())
                    .unwrap_or(next_id.as_str());
                badges = badges.child(pill_badge(
                    t,
                    &format!("Encadeado → {next_name}"),
                    t.accent,
                    t.accent_soft,
                ));
            }

            let actions = row()
                .min_w(0.0)
                .shrink()
                .gap(6.0)
                .center_y()
                .flex_wrap()
                .child(
                    btn_ghost(
                        t,
                        format!("proj-wf-open-{}", wf.id),
                        "Ver fluxo",
                        Some(&icons::CHEVRON_RIGHT),
                        28.0,
                        move |s: &mut AppState, _| {
                            s.selected_project_id = Some(bid_open.clone());
                            s.selected_workflow_id = Some(wid_open.clone());
                        },
                    )
                    .help("Abre a visão detalhada e o pipeline desta receita de fluxo."),
                )
                .child(
                    btn_neutral(
                        t,
                        format!("proj-wf-edit-{}", wf.id),
                        "Editar fluxo",
                        Some(&icons::EDIT),
                        28.0,
                        move |s: &mut AppState, _| s.open_edit_workflow_modal(&wid_edit),
                    )
                    .help("Edita os passos, gatilho e encadeamento deste fluxo."),
                )
                .child(
                    primary_button(
                        t,
                        format!("proj-wf-run-{}", wf.id),
                        "Executar",
                        Some(&icons::PLAY),
                        28.0,
                        move |s: &mut AppState, tx: &mut Tx| s.start_workflow(&wid_run, "manual", tx),
                    )
                    .help("Dispara imediatamente uma execução manual deste fluxo."),
                );

            list = list.child(
                row()
                    .key(format!("proj-wf-card-{}", wf.id))
                    .w_full()
                    .min_w(0.0)
                    .px(14.0)
                    .py(10.0)
                    .gap(10.0)
                    .center_y()
                    .justify_between()
                    .flex_wrap()
                    .radius(10.0)
                    .fill(t.surface_1)
                    .border(1.0, t.border_subtle)
                    .child(
                        row()
                            .grow(1.0)
                            .shrink()
                            .min_w(160.0)
                            .gap(10.0)
                            .center_y()
                            .flex_wrap()
                            .child(
                                icon(icons::workflow_icon(&wf.name), 14.5)
                                    .color(t.accent)
                                    .a11y_hidden(),
                            )
                            .child(
                                text(wf.name.clone())
                                    .font(ty::BODY_MED)
                                    .color(t.text_primary)
                                    .shrink()
                                    .min_w(0.0)
                                    .truncate(),
                            )
                            .child(badges),
                    )
                    .child(actions),
            );
        }
    }

    col()
        .key("proj-workflows-section")
        .w_full()
        .min_w(0.0)
        .gap(8.0)
        .child(
            text("Fluxos deste projeto")
                .font(ty::SMALL_MED)
                .color(t.text_secondary),
        )
        .child(list)
}

fn workflow_pipeline_summary(s: &AppState, t: UiTheme, w: &model::Workflow) -> El {
    let mut chain = row().w_full().min_w(0.0).gap(6.0).center_y().flex_wrap();
    chain = chain.child(
        row()
            .gap(5.0)
            .center_y()
            .child(icon(icons::workflow_icon(&w.name), 13.5).color(t.accent))
            .child(
                text("Pipeline:")
                    .font(ty::CAPTION)
                    .color(t.text_muted),
            ),
    );

    if w.steps.is_empty() {
        chain = chain.child(
            text("Nenhum passo configurado")
                .font(ty::CAPTION)
                .color(t.text_muted),
        );
    } else {
        for (idx, st) in w.steps.iter().enumerate() {
            if idx > 0 {
                chain = chain.child(icon(&icons::CHEVRON_RIGHT, 12.0).color(t.text_muted));
            }
            let mut step_pill = row()
                .px(8.0)
                .py(4.0)
                .gap(5.0)
                .center_y()
                .radius(6.0)
                .fill(t.surface_2)
                .border(
                    1.0,
                    if st.approval {
                        t.warning.alpha(0.42)
                    } else {
                        t.border_subtle
                    },
                )
                .child(
                    text(format!("{}. {}", idx + 1, st.name))
                        .font(ty::SMALL_MED)
                        .color(t.text_primary),
                );
            if st.approval {
                step_pill = step_pill.child(
                    row()
                        .px(5.0)
                        .py(1.0)
                        .radius(4.0)
                        .fill(t.warning_bg)
                        .border(1.0, t.warning.alpha(0.38))
                        .child(
                            text("[Aprovação]")
                                .font(ty::CAPTION)
                                .color(t.warning),
                        ),
                );
            }
            chain = chain.child(step_pill);
        }
    }

    if let Some(next_id) = &w.on_success {
        let next_name = s
            .snapshot
            .workflows
            .iter()
            .find(|nw| nw.id == *next_id)
            .map(|nw| nw.name.as_str())
            .unwrap_or(next_id.as_str());
        chain = chain
            .child(icon(&icons::CHEVRON_RIGHT, 12.0).color(t.accent))
            .child(
                row()
                    .px(8.0)
                    .py(4.0)
                    .gap(5.0)
                    .center_y()
                    .radius(6.0)
                    .fill(t.accent_soft)
                    .border(1.0, t.accent.alpha(0.38))
                    .child(
                        text(format!("Encadeado: {next_name}"))
                            .font(ty::SMALL_MED)
                            .color(t.accent),
                    ),
            );
    }

    let matching_bindings: Vec<&model::RepoBinding> = s
        .snapshot
        .settings
        .repo_bindings
        .iter()
        .filter(|b| s.project_workflows(b).iter().any(|wf| wf.id == w.id))
        .collect();

    let mut triggers_row = row().w_full().min_w(0.0).gap(6.0).center_y().flex_wrap().child(
        row()
            .gap(5.0)
            .center_y()
            .child(icon(&icons::BRANCH, 12.5).color(t.text_muted))
            .child(
                text("Disparado por:")
                    .font(ty::CAPTION)
                    .color(t.text_muted),
            ),
    );

    if matching_bindings.is_empty() {
        let empty_msg = match w.trigger {
            model::Trigger::Manual => "Somente execução manual ou por encadeamento.",
            model::Trigger::Folder => "Vigia de pasta local configurado.",
            model::Trigger::Push => "Nenhum vínculo de repositório aponta para este fluxo.",
        };
        triggers_row = triggers_row.child(
            text(empty_msg)
                .font(ty::CAPTION)
                .color(t.text_muted)
                .wrap(),
        );
    } else {
        for b in matching_bindings {
            let srv_label = s.server_label(b.target_server_id.as_deref());
            let state_suffix = if b.enabled { "" } else { " · pausado" };
            let fg = if b.enabled { t.text_secondary } else { t.warning };
            triggers_row = triggers_row.child(pill_badge(
                t,
                &format!("{} ({}) · {}{}", b.full_name, b.branch, srv_label, state_suffix),
                fg,
                t.surface_2,
            ));
        }
    }

    let effective_binding = s.resolve_binding_for_workflow(&w.id);
    let effective_server_id = effective_binding
        .as_ref()
        .and_then(|b| b.target_server_id.as_deref())
        .or(w.server_id.as_deref());
    let dest_label = if effective_server_id.is_some()
        || w.runner == workploy::model::RunnerKind::Ssh
    {
        match effective_server_id {
            Some(sid) => format!("Destino configurado: {}", s.server_label(Some(sid))),
            None => "Destino configurado: VPS via SSH (selecione uma VPS no vínculo)".to_string(),
        }
    } else {
        "Destino configurado: Local (Esta máquina)".to_string()
    };
    let dest_warn = w.runner == workploy::model::RunnerKind::Ssh
        && effective_server_id.is_none()
        && s.snapshot.settings.ssh_host.trim().is_empty();
    triggers_row = triggers_row.child(pill_badge(
        t,
        &dest_label,
        if dest_warn { t.warning } else { t.accent },
        if dest_warn { t.warning_bg } else { t.accent_soft },
    ));

    // Receita importada do repo: linha compacta de proveniência
    // (repo · caminho · SHA curto) com "Atualizar do repo" ao lado.
    let mut source_row = None;
    if let Some(src) = &w.source {
        let sha_short = src
            .sha
            .as_deref()
            .map(|sha| sha.chars().take(7).collect::<String>())
            .unwrap_or_else(|| "sem SHA".into());
        let prov = format!(
            "Origem: {} · {} · {sha_short}",
            src.repo.trim(),
            src.path.trim()
        );
        let wid_up = w.id.clone();
        source_row = Some(
            row()
                .key(format!("wf-source-row-{}", w.id))
                .w_full()
                .min_w(0.0)
                .gap(8.0)
                .center_y()
                .flex_wrap()
                .child(icon(&icons::DOCUMENTS, 13.0).color(t.text_muted).a11y_hidden())
                .child(
                    text(prov)
                        .font(ty::CAPTION)
                        .color(t.text_muted)
                        .grow(1.0)
                        .shrink()
                        .min_w(0.0)
                        .truncate()
                        .a11y_hidden(),
                )
                .child(
                    btn_ghost(
                        t,
                        format!("upd-repo-{}", w.id),
                        "Atualizar do repo",
                        Some(&icons::REFRESH),
                        28.0,
                        move |s: &mut AppState, tx: &mut Tx| {
                            s.update_workflow_from_repo(&wid_up, tx);
                        },
                    )
                    .help("Traz de novo a versão do arquivo no repositório, preservando id, encadeamento e vínculos."),
                ),
        );
    }

    let mut summary = col()
        .key("wf-pipeline-summary")
        .w_full()
        .min_w(0.0)
        .px(16.0)
        .py(12.0)
        .gap(8.0)
        .radius(10.0)
        .fill(t.surface_1)
        .border(1.0, t.border_subtle)
        .child(chain)
        .child(triggers_row);
    if let Some(sr) = source_row {
        summary = summary.child(sr);
    }
    summary
}

fn run_row(s: &AppState, t: UiTheme, r: &Run, show_wf_name: bool) -> El {
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

    let resolved_srv_id = r
        .vps_server_id
        .clone()
        .or_else(|| s.is_remote_vps_run(r))
        .or_else(|| {
            s.snapshot
                .workflows
                .iter()
                .find(|w| w.id == r.workflow_id)
                .and_then(|w| w.server_id.clone())
        })
        .or_else(|| {
            r.repo_name.as_deref().and_then(|rn| {
                s.snapshot
                    .settings
                    .repo_bindings
                    .iter()
                    .find(|b| b.full_name.eq_ignore_ascii_case(rn.trim()))
                    .and_then(|b| b.target_server_id.clone())
            })
        });
    let server_badge_label = match resolved_srv_id.as_deref() {
        Some(sid) => format!("VPS: {}", s.server_label(Some(sid))),
        None => {
            let is_ssh = s
                .snapshot
                .workflows
                .iter()
                .find(|w| w.id == r.workflow_id)
                .is_some_and(|w| w.runner == model::RunnerKind::Ssh);
            if is_ssh || r.id.starts_with("run-vps-") {
                "VPS".to_string()
            } else {
                "Local".to_string()
            }
        }
    };

    let mut meta = row().gap(8.0).center_y().flex_wrap();
    meta = meta.child(status_pill(t, r.status));
    meta = meta.child(
        row()
            .h(20.0)
            .px(7.0)
            .gap(4.0)
            .center_y()
            .radius(5.0)
            .fill(t.surface_2)
            .child(
                icon(
                    if resolved_srv_id.is_some() || server_badge_label.starts_with("VPS") {
                        &icons::SERVER
                    } else {
                        &icons::LAPTOP
                    },
                    10.5,
                )
                .color(t.text_muted)
                .a11y_hidden(),
            )
            .child(
                text(server_badge_label)
                    .font(ty::CAPTION)
                    .color(t.text_secondary),
            ),
    );
    // Execução "quente": vários jobs do mesmo fluxo falharam juntos.
    if r.status == RunStatus::Failed {
        let failed_jobs = r
            .steps
            .iter()
            .filter(|st| st.status == StepStatus::Failed)
            .count();
        if failed_jobs >= 2 {
            meta = meta.child(
                row()
                    .h(20.0)
                    .px(7.0)
                    .gap(4.0)
                    .center_y()
                    .radius(5.0)
                    .fill(t.danger_bg)
                    .child(icon(&icons::FIRE, 11.0).color(t.danger).a11y_hidden())
                    .child(
                        text(format!("{failed_jobs} jobs falhos"))
                            .font(ty::CAPTION)
                            .color(t.danger),
                    ),
            );
        }
    }
    if show_wf_name {
        meta = meta.child(
            row()
                .gap(4.0)
                .center_y()
                .child(
                    icon(icons::workflow_icon(&r.workflow_name), 12.0)
                        .color(t.accent)
                        .a11y_hidden(),
                )
                .child(
                    text(r.workflow_name.clone())
                        .font(ty::CAPTION)
                        .color(t.accent),
                ),
        );
    }
    if let Some(repo) = &r.repo_name
        && !repo.trim().is_empty()
    {
        meta = meta.child(
            text(repo.clone())
                .font(ty::CAPTION)
                .color(t.text_secondary),
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
            // Cancelamento já solicitado e execução ainda ativa: o botão
            // vira estado de espera, sem clique repetido.
            if r.cancel_requested && r.status.active() {
                right_actions = right_actions.child(
                    btn_danger(
                        t,
                        format!("row-cancel-{}", r.id),
                        "Cancelando...",
                        None,
                        28.0,
                        |_: &mut AppState, _| {},
                    )
                    .disabled(true)
                    .opacity(0.6)
                    .cursor(Cursor::Default)
                    .hover_alone(),
                );
            } else {
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

    let mut title_and_meta = col()
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
        .child(meta);

    if let Some(fail_reason) = workploy::diagnostic::short_failure_reason(r) {
        title_and_meta = title_and_meta.child(
            row()
                .w_full()
                .min_w(0.0)
                .gap(6.0)
                .center_y()
                .child(icon(&icons::BUG, 11.5).color(t.danger).a11y_hidden())
                .child(
                    text(fail_reason)
                        .font(ty::CAPTION)
                        .color(t.danger)
                        .grow(1.0)
                        .shrink()
                        .min_w(0.0)
                        .truncate(),
                ),
        );
    }

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
                .child(title_and_meta),
        )
        .child(right_cluster);

    let mut card_body = col().w_full().child(content);
    if r.status.active() {
        let frac = model::run_progress_frac(r);
        let (bar_col, cidx) = if r.status == RunStatus::AwaitingApproval {
            (t.warning, 2.0)
        } else {
            (t.info, 4.0)
        };
        let active_step = r
            .steps
            .iter()
            .enumerate()
            .find(|(_, st)| matches!(st.status, StepStatus::Running | StepStatus::WaitingApproval));
        let pct = (frac * 100.0).round().clamp(0.0, 100.0) as u8;
        let prog_label = if let Some((sidx, st)) = active_step {
            format!(
                "Etapa {}/{} · {} ({pct}%)",
                sidx + 1,
                r.steps.len().max(1),
                st.name
            )
        } else {
            format!("{pct}%")
        };
        card_body = card_body.child(
            col()
                .w_full()
                .px(16.0)
                .pb(10.0)
                .gap(5.0)
                .child(
                    row()
                        .w_full()
                        .justify_between()
                        .center_y()
                        .child(
                            text(prog_label)
                                .font(ty::CAPTION)
                                .color(bar_col)
                                .truncate(),
                        )
                        .child(
                            text(format!("{pct}%"))
                                .font(ty::MONO_SM)
                                .color(t.text_muted),
                        ),
                )
                .child(run_progress_strip(
                    t,
                    format!("row-prog-{}", r.id),
                    frac,
                    matches!(r.status, RunStatus::Running | RunStatus::Queued),
                    bar_col,
                    cidx,
                )),
        );
    }

    let mut card = row()
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
                RunStatus::Running | RunStatus::Queued => t.info.alpha(0.38),
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
        .on_press(move |s: &mut AppState| s.open_run_summary(&rid_open));

    if matches!(r.status, RunStatus::Running | RunStatus::Queued) {
        card = card.child(active_electric_effect(4.0, 8.0, 0.30));
    } else if r.status == RunStatus::AwaitingApproval {
        card = card.child(active_electric_effect(2.0, 8.0, 0.24));
    }
    card.child(card_body)
}
