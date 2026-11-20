use rune::tok::motion;
use rune::{Cursor, El, ScrollSpec, Transition, Tx, col, effect, el, icon, row, text};
use rune_kit::{Kit, Progress, Switch, TextArea, TextInput};
use workploy::model::{PlatformTarget, Trigger, TriggerMode};

use crate::gfx::{MARK_PULSE_RADAR, PULSE_INTENSITY_ACTIVE, PULSE_TEMPO_ACTIVE};
use crate::state::{
    AppState, ModalState, PurgeSizeUnit, ServerAuthChoice, StepDraft, ToastKind, WorkflowPreset,
    platform_compat_warning,
};
use crate::ui::components::{
    btn_danger, btn_ghost, btn_icon, btn_neutral, chip, chip_with_icon, pill_badge, primary_button,
};
use crate::ui::icons;
use crate::ui::theme::{UiTheme, ty};

pub fn mount_modal(s: &AppState, t: UiTheme, k: &Kit, layer: &mut Vec<El>) {
    match &s.modal {
        ModalState::None => {}
        ModalState::WorkflowEditor(draft) => {
            let is_edit = draft.id.is_some();
            let title = if is_edit {
                "Editar fluxo de trabalho"
            } else {
                "Novo fluxo de trabalho"
            };

            let mut presets_row = row().gap(6.0).flex_wrap();
            for preset in WorkflowPreset::ALL {
                let (key, preset_icon) = match preset {
                    WorkflowPreset::NodePnpmCi => ("ed-preset-node", &icons::WORKFLOW),
                    WorkflowPreset::RustCargoCi => ("ed-preset-rust", &icons::WORKFLOW),
                    WorkflowPreset::DockerComposeRedeploy => ("ed-preset-docker", &icons::DEPLOY),
                    WorkflowPreset::AuditAndSmoke => ("ed-preset-audit", &icons::WORKFLOW),
                };
                let active = draft.name.text.trim() == preset.label();
                presets_row = presets_row.child(
                    chip_with_icon(
                        t,
                        key,
                        preset.label(),
                        Some(preset_icon),
                        active,
                        move |s: &mut AppState, _| {
                            s.next_uid += 1;
                            let mut next_uid = s.next_uid;
                            if let ModalState::WorkflowEditor(d) = &mut s.modal {
                                d.apply_preset(preset, &mut next_uid);
                            }
                            s.next_uid = next_uid;
                        },
                    )
                    .help(preset.help()),
                );
            }

            let name_input = TextInput::new(&draft.name)
                .label("Nome do fluxo")
                .placeholder("Ex: Deploy VPS")
                .width(None)
                .on_edit(|s: &mut AppState| match &mut s.modal {
                    ModalState::WorkflowEditor(d) => &mut d.name,
                    _ => unreachable!(),
                })
                .build(k)
                .cursor(Cursor::Text)
                .help("Nome exibido na barra lateral, no histórico de execuções e nos vínculos de repositório.");

            let workdir_input = TextInput::new(&draft.workdir)
                .label("Pasta de trabalho")
                .placeholder("Opcional (ex: /srv/app ou C:\\Projects\\app)")
                .width(None)
                .on_edit(|s: &mut AppState| match &mut s.modal {
                    ModalState::WorkflowEditor(d) => &mut d.workdir,
                    _ => unreachable!(),
                })
                .build(k)
                .cursor(Cursor::Text)
                .help("Diretório onde os comandos do fluxo serão executados (ex: /srv/app na VPS ou C:\\Projects\\app no Windows).");

            let triggers = row()
                .gap(6.0)
                .flex_wrap()
                .child(
                    chip(
                        t,
                        "ed-trig-manual",
                        "Manual (botão)",
                        draft.trigger == Trigger::Manual,
                        |s: &mut AppState, _| {
                            if let ModalState::WorkflowEditor(d) = &mut s.modal {
                                d.trigger = Trigger::Manual;
                            }
                        },
                    )
                    .help("Executa apenas quando você clicar em 'Executar fluxo' ou quando for encadeado por outro fluxo."),
                )
                .child(
                    chip(
                        t,
                        "ed-trig-push",
                        "push no GitHub",
                        draft.trigger == Trigger::Push,
                        |s: &mut AppState, _| {
                            if let ModalState::WorkflowEditor(d) = &mut s.modal {
                                d.trigger = Trigger::Push;
                            }
                        },
                    )
                    .help("Dispara automaticamente quando houver um novo push na branch monitorada de um repositório vinculado."),
                )
                .child(
                    chip(
                        t,
                        "ed-trig-folder",
                        "Mudança na pasta",
                        draft.trigger == Trigger::Folder,
                        |s: &mut AppState, _| {
                            if let ModalState::WorkflowEditor(d) = &mut s.modal {
                                d.trigger = Trigger::Folder;
                            }
                        },
                    )
                    .help("Dispara automaticamente quando o vigia de pasta local detectar alterações na pasta monitorada."),
                );

            let binding_hint = row()
                .key("ed-binding-hint")
                .w_full()
                .min_w(0.0)
                .px(12.0)
                .py(9.0)
                .gap(8.0)
                .center_y()
                .radius(8.0)
                .fill(t.surface_2)
                .border(1.0, t.border_subtle)
                .child(icon(&icons::INFO, 14.0).color(t.accent).a11y_hidden())
                .child(
                    text("Onde e quando este fluxo roda (qual VPS, branch e modo Webhook/Sondagem) é definido nos Vínculos em Configurações → GitHub.")
                        .font(ty::CAPTION)
                        .color(t.text_muted)
                        .grow(1.0)
                        .shrink()
                        .min_w(0.0)
                        .wrap(),
                );

            let mut chain_row = row().gap(6.0).flex_wrap().child(
                chip(
                    t,
                    "ed-chain-none",
                    "Nenhum",
                    draft.on_success.is_none(),
                    |s: &mut AppState, _| {
                        if let ModalState::WorkflowEditor(d) = &mut s.modal {
                            d.on_success = None;
                        }
                    },
                )
                .help("Encerra o pipeline ao concluir todos os passos, sem disparar outro fluxo."),
            );
            for wf in &s.snapshot.workflows {
                if draft.id.as_deref() == Some(wf.id.as_str()) {
                    continue;
                }
                let target_id = wf.id.clone();
                let active = draft.on_success.as_deref() == Some(wf.id.as_str());
                let tip = format!(
                    "Se este fluxo terminar com sucesso, inicia automaticamente '{}'.",
                    wf.name
                );
                chain_row = chain_row.child(
                    chip(
                        t,
                        format!("ed-chain-{}", wf.id),
                        wf.name.clone(),
                        active,
                        move |s: &mut AppState, _| {
                            if let ModalState::WorkflowEditor(d) = &mut s.modal {
                                d.on_success = Some(target_id.clone());
                            }
                        },
                    )
                    .help(tip),
                );
            }

            let step_count = draft.steps.len();
            let mut steps_col = col().w_full().gap(10.0);
            for (idx, st) in draft.steps.iter().enumerate() {
                let uid = st.uid;
                let step_name_in = TextInput::new(&st.name)
                    .label(format!("Nome do passo {}", idx + 1))
                    .placeholder("Ex: Compilar projeto")
                    .width(None)
                    .on_edit(move |s: &mut AppState| match &mut s.modal {
                        ModalState::WorkflowEditor(d) => &mut d.steps[idx].name,
                        _ => unreachable!(),
                    })
                    .build(k)
                    .cursor(Cursor::Text)
                    .help("Título curto do passo exibido nos logs e no resumo visual do pipeline.");

                let step_cmd_in = TextInput::new(&st.run)
                    .label(format!("Comando do passo {}", idx + 1))
                    .placeholder("pnpm test")
                    .width(None)
                    .on_edit(move |s: &mut AppState| match &mut s.modal {
                        ModalState::WorkflowEditor(d) => &mut d.steps[idx].run,
                        _ => unreachable!(),
                    })
                    .build(k)
                    .cursor(Cursor::Text)
                    .help("Comando shell executado nesta etapa. Se falhar, o fluxo para aqui.");

                let mut controls = row().gap(4.0).center_y();
                if idx > 0 {
                    controls = controls.child(btn_icon(
                        t,
                        format!("step-up-{uid}"),
                        "Mover passo para cima",
                        &icons::CHEVRON_UP,
                        26.0,
                        move |s: &mut AppState, _| {
                            if let ModalState::WorkflowEditor(d) = &mut s.modal
                                && idx > 0
                                && idx < d.steps.len()
                            {
                                d.steps.swap(idx - 1, idx);
                            }
                        },
                    ));
                }
                if idx + 1 < step_count {
                    controls = controls.child(btn_icon(
                        t,
                        format!("step-down-{uid}"),
                        "Mover passo para baixo",
                        &icons::CHEVRON_DOWN,
                        26.0,
                        move |s: &mut AppState, _| {
                            if let ModalState::WorkflowEditor(d) = &mut s.modal
                                && idx + 1 < d.steps.len()
                            {
                                d.steps.swap(idx, idx + 1);
                            }
                        },
                    ));
                }
                if step_count > 1 {
                    controls = controls.child(btn_icon(
                        t,
                        format!("step-del-{uid}"),
                        "Remover passo",
                        &icons::TRASH,
                        26.0,
                        move |s: &mut AppState, _| {
                            if let ModalState::WorkflowEditor(d) = &mut s.modal
                                && d.steps.len() > 1
                                && idx < d.steps.len()
                            {
                                d.steps.remove(idx);
                            }
                        },
                    ));
                }

                let step_card = col()
                    .key(format!("ed-step-{uid}"))
                    .w_full()
                    .padding(12.0)
                    .gap(8.0)
                    .radius(10.0)
                    .fill(t.surface_2)
                    .border(
                        1.0,
                        if st.approval {
                            t.warning.alpha(0.45)
                        } else {
                            t.border_subtle
                        },
                    )
                    .child(
                        row()
                            .w_full()
                            .justify_between()
                            .center_y()
                            .child(
                                text(format!("Passo {}", idx + 1))
                                    .font(ty::SMALL_MED)
                                    .color(t.accent),
                            )
                            .child(controls),
                    )
                    .child(step_name_in)
                    .child(step_cmd_in)
                    .child(
                        row()
                            .w_full()
                            .justify_between()
                            .center_y()
                            .gap(12.0)
                            .child(
                                Switch::new(st.approval)
                                    .label("Pedir aprovação antes deste passo")
                                    .on_toggle(move |s: &mut AppState| {
                                        if let ModalState::WorkflowEditor(d) = &mut s.modal
                                            && let Some(st) = d.steps.get_mut(idx)
                                        {
                                            st.approval = !st.approval;
                                        }
                                    })
                                    .build(k)
                                    .cursor(Cursor::Pointer)
                                    .help("Pausa a execução antes deste passo e aguarda aprovação manual na aba Aprovações."),
                            )
                            .child(
                                Switch::new(st.skip)
                                    .label("Pular este passo")
                                    .on_toggle(move |s: &mut AppState| {
                                        if let ModalState::WorkflowEditor(d) = &mut s.modal
                                            && let Some(st) = d.steps.get_mut(idx)
                                        {
                                            st.skip = !st.skip;
                                        }
                                    })
                                    .build(k)
                                    .cursor(Cursor::Pointer)
                                    .help("Ignora este passo na execução (o fluxo continua sem ele). Útil para depurar ou rodar só parte do gate."),
                            ),
                    );

                steps_col = steps_col.child(step_card);
            }

            let cpu_limit_in = TextInput::new(&draft.cpu_limit)
                .label("Limite de vCPU (vazio = sem limite)")
                .placeholder("2")
                .width(Some(150.0))
                .on_edit(|s: &mut AppState| match &mut s.modal {
                    ModalState::WorkflowEditor(d) => &mut d.cpu_limit,
                    _ => unreachable!(),
                })
                .build(k)
                .cursor(Cursor::Text)
                .help("Prende o passo em N núcleos (taskset) na VPS, deixando os demais livres para a produção.");

            let mem_limit_in = TextInput::new(&draft.mem_limit_mb)
                .label("Limite de RAM em MB (vazio = sem limite)")
                .placeholder("4096")
                .width(Some(190.0))
                .on_edit(|s: &mut AppState| match &mut s.modal {
                    ModalState::WorkflowEditor(d) => &mut d.mem_limit_mb,
                    _ => unreachable!(),
                })
                .build(k)
                .cursor(Cursor::Text)
                .help("Cria um cgroup com teto de memória (systemd-run --scope). Ao estourar, o passo é encerrado com OOM claro em vez de derrubar a VPS.");

            let suggestion = s.limit_suggestion_for_draft(draft);
            let mut resources_section = col()
                .w_full()
                .min_w(0.0)
                .gap(10.0)
                .child(
                    row()
                        .w_full()
                        .min_w(0.0)
                        .gap(12.0)
                        .flex_wrap()
                        .child(cpu_limit_in)
                        .child(mem_limit_in),
                );
            if let Some(sug) = suggestion.as_ref() {
                let sug_cpu = sug.cpu_limit.map(|c| c.to_string()).unwrap_or_default();
                let sug_mem = sug.mem_limit_mb.map(|m| m.to_string()).unwrap_or_default();
                let reason = sug.reason.clone();
                resources_section = resources_section.child(
                    row()
                        .key("wf-limits-suggestion")
                        .w_full()
                        .min_w(0.0)
                        .px(12.0)
                        .py(10.0)
                        .gap(10.0)
                        .center_y()
                        .flex_wrap()
                        .radius(8.0)
                        .fill(t.accent_soft)
                        .border(1.0, t.accent.alpha(0.38))
                        .child(icon(&icons::INFO, 14.0).color(t.accent).a11y_hidden())
                        .child(
                            col()
                                .grow(1.0)
                                .shrink()
                                .min_w(0.0)
                                .gap(2.0)
                                .child(
                                    text(format!(
                                        "Sugestão: {} vCPU · {} MB de RAM",
                                        sug_cpu, sug_mem
                                    ))
                                    .font(ty::SMALL_MED)
                                    .color(t.text_primary),
                                )
                                .child(
                                    text(reason)
                                        .font(ty::CAPTION)
                                        .color(t.text_secondary)
                                        .wrap(),
                                ),
                        )
                        .child(
                            btn_neutral(
                                t,
                                "wf-limits-use-suggestion",
                                "Usar sugestão",
                                Some(&icons::CHECK_CIRCLE),
                                28.0,
                                move |s: &mut AppState, _| {
                                    if let ModalState::WorkflowEditor(d) = &mut s.modal {
                                        d.cpu_limit.replace_text(sug_cpu.clone());
                                        d.mem_limit_mb.replace_text(sug_mem.clone());
                                    }
                                },
                            )
                            .help("Aplica a sugestão de vCPU e RAM calculada para esta VPS."),
                        ),
                );
            }

            let mut form = col()
                .w_full()
                .gap(12.0)
                .child(labeled(t, "Presets rápidos", presets_row))
                .child(labeled(t, "Nome do fluxo", name_input))
                .child(labeled(t, "Gatilho", triggers))
                .child(binding_hint)
                .child(labeled(t, "Pasta de trabalho (opcional)", workdir_input))
                .child(labeled(t, "Recursos na VPS (opcional)", resources_section))
                .child(labeled(t, "Encadear ao concluir com sucesso", chain_row))
                .child(
                    row()
                        .w_full()
                        .justify_between()
                        .center_y()
                        .child(
                            text(format!("Passos ({step_count})"))
                                .font(ty::SMALL_MED)
                                .color(t.text_primary),
                        )
                        .child(
                            btn_neutral(
                                t,
                                "btn-add-step",
                                "Adicionar passo",
                                Some(&icons::PLUS),
                                28.0,
                                |s: &mut AppState, _| {
                                    s.next_uid += 1;
                                    let uid = s.next_uid;
                                    if let ModalState::WorkflowEditor(d) = &mut s.modal {
                                        d.steps.push(StepDraft {
                                            uid,
                                            name: rune::TextEdit::new(format!(
                                                "Passo {}",
                                                d.steps.len() + 1
                                            )),
                                            run: rune::TextEdit::new("echo ok"),
                                            approval: false,
                                            skip: false,
                                        });
                                    }
                                },
                            )
                            .help("Adiciona um novo passo sequencial ao final deste fluxo."),
                        ),
                )
                .child(steps_col);

            if let Some(err) = &draft.error {
                form = form.child(error_banner(t, err));
            }

            // Receita com origem no repo: banner de drift + linha compacta de
            // proveniência (repo · caminho · SHA curto) com "Atualizar do repo".
            if let Some(wf_id) = draft.id.as_deref()
                && let Some(wf) = s.snapshot.workflows.iter().find(|w| w.id == wf_id)
            {
                if let Some((_, _, drift_msg)) = s
                    .snapshot
                    .workflow_drift
                    .iter()
                    .find(|(id, _, _)| id == wf_id)
                {
                    let wid_up = wf_id.to_string();
                    let drift_msg = drift_msg.clone();
                    form = form.child(
                        row()
                            .key("ed-drift-banner")
                            .w_full()
                            .px(12.0)
                            .py(8.0)
                            .gap(9.0)
                            .center_y()
                            .flex_wrap()
                            .radius(8.0)
                            .fill(t.warning_bg)
                            .border(1.0, t.warning.alpha(0.4))
                            .child(
                                icon(&icons::REFRESH, 14.0)
                                    .color(t.warning)
                                    .a11y_hidden(),
                            )
                            .child(
                                text(drift_msg)
                                    .font(ty::SMALL)
                                    .color(t.warning)
                                    .grow(1.0)
                                    .shrink()
                                    .wrap(),
                            )
                            .child(
                                btn_neutral(
                                    t,
                                    format!("ed-drift-upd-repo-{wf_id}"),
                                    "Atualizar do repo",
                                    Some(&icons::REFRESH),
                                    28.0,
                                    move |s: &mut AppState, tx: &mut Tx| {
                                        // A receita mudou no repo: descarta o
                                        // rascunho e busca a versão da fonte.
                                        s.modal = ModalState::None;
                                        s.update_workflow_from_repo(&wid_up, tx);
                                    },
                                )
                                .help("Reimporta a versão atual do arquivo no repositório, preservando id e encadeamento."),
                            ),
                    );
                }
                if let Some(src) = &wf.source {
                    let sha_short = src
                        .sha
                        .as_deref()
                        .map(|s| s.chars().take(7).collect::<String>())
                        .unwrap_or_else(|| "sem SHA".into());
                    let prov = format!(
                        "{} · {} · {sha_short}",
                        src.repo.trim(),
                        src.path.trim()
                    );
                    let wid_up = wf_id.to_string();
                    form = form.child(
                        row()
                            .key("ed-source-row")
                            .w_full()
                            .px(12.0)
                            .py(8.0)
                            .gap(9.0)
                            .center_y()
                            .flex_wrap()
                            .radius(8.0)
                            .fill(t.surface_2)
                            .border(1.0, t.border_subtle)
                            .child(
                                icon(&icons::DOCUMENTS, 13.5)
                                    .color(t.text_muted)
                                    .a11y_hidden(),
                            )
                            .child(
                                text(prov)
                                    .font(ty::MONO_SM)
                                    .color(t.text_secondary)
                                    .grow(1.0)
                                    .shrink()
                                    .truncate()
                                    .a11y_hidden(),
                            )
                            .child(
                                btn_ghost(
                                    t,
                                    format!("ed-upd-repo-{wf_id}"),
                                    "Atualizar do repo",
                                    Some(&icons::REFRESH),
                                    28.0,
                                    move |s: &mut AppState, tx: &mut Tx| {
                                        s.modal = ModalState::None;
                                        s.update_workflow_from_repo(&wid_up, tx);
                                    },
                                )
                                .help("Traz de novo a versão do arquivo no repositório para esta receita."),
                            ),
                    );
                }
            }

            // Rodapé: "Importar do repositório" quando a receita editada tem
            // vínculo; à direita seguem Cancelar e Salvar.
            let mut footer_left = el();
            if let Some(wf_id) = draft.id.as_deref()
                && let Some(wf) = s.snapshot.workflows.iter().find(|w| w.id == wf_id)
                && let Some(bid_imp) = s.binding_id_for_workflow(wf)
            {
                footer_left = btn_neutral(
                    t,
                    "bind-imp-repo",
                    "Importar do repositório",
                    Some(&icons::DOCUMENTS),
                    34.0,
                    move |s: &mut AppState, tx: &mut Tx| {
                        s.open_import_from_repo_modal(&bid_imp, tx);
                    },
                )
                .help("Busca o yml direto no repositório vinculado (.workploy.yml ou infra/workploy) — o repo vira a fonte da verdade.");
            }
            let footer = row()
                .w_full()
                .justify_between()
                .gap(10.0)
                .child(footer_left)
                .child(
                    row()
                        .gap(10.0)
                        .child(
                            btn_neutral(
                                t,
                                "ed-cancel",
                                "Cancelar",
                                None,
                                34.0,
                                |s: &mut AppState, _| {
                                    s.modal = ModalState::None;
                                },
                            )
                            .help("Descarta as alterações e fecha o editor."),
                        )
                        .child(
                            primary_button(
                                t,
                                "ed-save",
                                "Salvar fluxo",
                                Some(&icons::CHECK_CIRCLE),
                                34.0,
                                |s: &mut AppState, tx: &mut Tx| s.save_workflow_draft(tx),
                            )
                            .help("Salva a definição do fluxo e atualiza os gatilhos configurados."),
                        ),
                );

            mount_shell_dialog(t, "editor", title, 720.0, form, footer, layer);
        }
        ModalState::YamlModal(draft) => {
            let yaml_copy = draft.edit.text.clone();
            let area = TextArea::new(&draft.edit)
                .label("Conteúdo YAML do fluxo")
                .rows(14)
                .max_height(360.0)
                .width(None)
                .on_edit(|s: &mut AppState| match &mut s.modal {
                    ModalState::YamlModal(d) => &mut d.edit,
                    _ => unreachable!(),
                })
                .build(k)
                .cursor(Cursor::Text);

            let mut body = col()
                .w_full()
                .gap(10.0)
                .child(
                    text("Você pode editar ou colar um YAML compatível e clicar em 'Aplicar YAML'.")
                        .font(ty::SMALL)
                        .color(t.text_muted),
                )
                .child(area);

            if let Some(err) = &draft.error {
                body = body.child(error_banner(t, err));
            }

            let footer = row()
                .w_full()
                .justify_between()
                .center_y()
                .child(btn_neutral(
                    t,
                    "yaml-copy",
                    "Copiar YAML",
                    Some(&icons::COPY),
                    34.0,
                    move |s: &mut AppState, tx: &mut Tx| {
                        tx.copy(yaml_copy.clone());
                        s.push_toast(
                            "copy-yaml",
                            "YAML copiado",
                            Some("Definição do fluxo copiada para a área de transferência.".into()),
                            ToastKind::Success,
                            tx,
                        );
                    },
                ))
                .child(
                    row()
                        .gap(10.0)
                        .child(btn_neutral(
                            t,
                            "yaml-close",
                            "Fechar",
                            None,
                            34.0,
                            |s: &mut AppState, _| s.modal = ModalState::None,
                        ))
                        .child(primary_button(
                            t,
                            "yaml-apply",
                            "Aplicar YAML",
                            Some(&icons::CHECK_CIRCLE),
                            34.0,
                            |s: &mut AppState, tx: &mut Tx| s.apply_yaml_draft(tx),
                        )),
                );

            mount_shell_dialog(
                t,
                "yaml",
                &format!("Arquivo YAML — {}", draft.wf_name),
                700.0,
                body,
                footer,
                layer,
            );
        }
        ModalState::DeleteConfirm { wf_name, .. } => {
            let body = col()
                .w_full()
                .gap(8.0)
                .child(
                    text(format!("Tem certeza que deseja excluir o fluxo '{wf_name}'?"))
                        .font(ty::BODY_MED)
                        .color(t.text_primary)
                        .wrap(),
                )
                .child(
                    text("A definição do fluxo será removida. Execuções antigas permanecem no histórico.")
                        .font(ty::SMALL)
                        .color(t.text_muted)
                        .wrap(),
                );
            let footer = row()
                .w_full()
                .justify_end()
                .gap(10.0)
                .child(btn_neutral(
                    t,
                    "del-cancel",
                    "Cancelar",
                    None,
                    34.0,
                    |s: &mut AppState, _| s.modal = ModalState::None,
                ))
                .child(btn_danger(
                    t,
                    "del-confirm",
                    "Excluir fluxo",
                    Some(&icons::TRASH),
                    34.0,
                    |s: &mut AppState, tx: &mut Tx| s.confirm_delete_workflow(tx),
                ));

            mount_shell_dialog(
                t,
                "delete",
                "Confirmar exclusão",
                460.0,
                body,
                footer,
                layer,
            );
        }
        ModalState::ServerEditor(draft) => {
            let title = if draft.id.is_some() {
                "Editar servidor VPS"
            } else {
                "Adicionar servidor VPS"
            };

            let name_in = TextInput::new(&draft.name)
                .label("Nome do servidor")
                .placeholder("Ex: VPS Produção")
                .width(None)
                .on_edit(|s: &mut AppState| match &mut s.modal {
                    ModalState::ServerEditor(d) => &mut d.name,
                    _ => unreachable!(),
                })
                .build(k)
                .cursor(Cursor::Text);

            let host_in = TextInput::new(&draft.host)
                .label("Host, IP ou alias SSH")
                .placeholder("Ex: 167.99.42.18 ou workploy-vps")
                .width(None)
                .on_edit(|s: &mut AppState| match &mut s.modal {
                    ModalState::ServerEditor(d) => &mut d.host,
                    _ => unreachable!(),
                })
                .build(k)
                .cursor(Cursor::Text);

            let user_in = TextInput::new(&draft.user)
                .label("Usuário SSH")
                .placeholder("root ou deploy")
                .width(Some(200.0))
                .on_edit(|s: &mut AppState| match &mut s.modal {
                    ModalState::ServerEditor(d) => &mut d.user,
                    _ => unreachable!(),
                })
                .build(k)
                .cursor(Cursor::Text);

            let port_in = TextInput::new(&draft.port)
                .label("Porta SSH")
                .placeholder("22")
                .width(Some(110.0))
                .on_edit(|s: &mut AppState| match &mut s.modal {
                    ModalState::ServerEditor(d) => &mut d.port,
                    _ => unreachable!(),
                })
                .build(k)
                .cursor(Cursor::Text);

            let runner_port_in = TextInput::new(&draft.runner_port)
                .label("Porta do workploy-runner")
                .placeholder("8787")
                .width(Some(140.0))
                .on_edit(|s: &mut AppState| match &mut s.modal {
                    ModalState::ServerEditor(d) => &mut d.runner_port,
                    _ => unreachable!(),
                })
                .build(k)
                .cursor(Cursor::Text);

            let auth_chips = row()
                .gap(6.0)
                .flex_wrap()
                .child(chip_with_icon(
                    t,
                    "srv-auth-alias",
                    "Alias ~/.ssh/config",
                    Some(&icons::CODE),
                    draft.auth_choice == ServerAuthChoice::SshConfigAlias,
                    |s: &mut AppState, _| {
                        if let ModalState::ServerEditor(d) = &mut s.modal {
                            d.auth_choice = ServerAuthChoice::SshConfigAlias;
                        }
                    },
                ))
                .child(chip_with_icon(
                    t,
                    "srv-auth-key",
                    "Chave privada SSH",
                    Some(&icons::KEY),
                    draft.auth_choice == ServerAuthChoice::PrivateKey,
                    |s: &mut AppState, _| {
                        if let ModalState::ServerEditor(d) = &mut s.modal {
                            d.auth_choice = ServerAuthChoice::PrivateKey;
                        }
                    },
                ))
                .child(chip_with_icon(
                    t,
                    "srv-auth-pass",
                    "Senha no Cofre do Windows",
                    Some(&icons::PASSWORD),
                    draft.auth_choice == ServerAuthChoice::PasswordInVault,
                    |s: &mut AppState, _| {
                        if let ModalState::ServerEditor(d) = &mut s.modal {
                            d.auth_choice = ServerAuthChoice::PasswordInVault;
                        }
                    },
                ));

            let mut form = col()
                .w_full()
                .gap(12.0)
                .child(labeled(t, "Nome do servidor", name_in))
                .child(labeled(t, "Endereço (IP, domínio ou alias SSH)", host_in))
                .child(
                    row()
                        .w_full()
                        .gap(12.0)
                        .flex_wrap()
                        .child(
                            col()
                                .gap(4.0)
                                .child(text("Usuário SSH").font(ty::CAPTION).color(t.text_secondary))
                                .child(user_in),
                        )
                        .child(
                            col()
                                .gap(4.0)
                                .child(text("Porta SSH").font(ty::CAPTION).color(t.text_secondary))
                                .child(port_in),
                        )
                        .child(
                            col()
                                .gap(4.0)
                                .child(
                                    text("Porta do agente 24/7")
                                        .font(ty::CAPTION)
                                        .color(t.text_secondary),
                                )
                                .child(runner_port_in),
                        ),
                )
                .child(labeled(t, "Método de autenticação", auth_chips));

            match draft.auth_choice {
                ServerAuthChoice::SshConfigAlias => {
                    form = form.child(
                        text("Usa o alias configurado em ~/.ssh/config sem solicitar senha.")
                            .font(ty::CAPTION)
                            .color(t.text_muted)
                            .wrap(),
                    );
                }
                ServerAuthChoice::PrivateKey => {
                    let key_in = TextInput::new(&draft.key_path)
                        .label("Caminho da chave privada")
                        .placeholder("C:\\Users\\seu_usuario\\.ssh\\id_ed25519")
                        .width(None)
                        .on_edit(|s: &mut AppState| match &mut s.modal {
                            ModalState::ServerEditor(d) => &mut d.key_path,
                            _ => unreachable!(),
                        })
                        .build(k)
                        .cursor(Cursor::Text);

                    form = form.child(
                        col()
                            .w_full()
                            .gap(8.0)
                            .child(labeled(t, "Caminho da chave privada SSH", key_in))
                            .child(
                                row()
                                    .gap(8.0)
                                    .center_y()
                                    .child(btn_neutral(
                                        t,
                                        "srv-gen-key",
                                        "Gerar chave Ed25519",
                                        Some(&icons::KEY),
                                        30.0,
                                        |s: &mut AppState, tx: &mut Tx| s.generate_server_keypair(tx),
                                    ))
                                    .child(
                                        text("Gera um par dedicado e preenche o caminho automaticamente.")
                                            .font(ty::CAPTION)
                                            .color(t.text_muted),
                                    ),
                            ),
                    );

                    if let Some(pub_key) = &draft.generated_pubkey {
                        let pub_copy = pub_key.clone();
                        form = form.child(
                            col()
                                .w_full()
                                .padding(10.0)
                                .gap(6.0)
                                .radius(8.0)
                                .fill(t.surface_2)
                                .border(1.0, t.success.alpha(0.4))
                                .child(
                                    row()
                                        .w_full()
                                        .justify_between()
                                        .center_y()
                                        .child(
                                            text("Chave pública (cole em ~/.ssh/authorized_keys na VPS)")
                                                .font(ty::CAPTION)
                                                .color(t.success),
                                        )
                                        .child(btn_neutral(
                                            t,
                                            "srv-copy-pubkey",
                                            "Copiar chave pública",
                                            Some(&icons::COPY),
                                            26.0,
                                            move |s: &mut AppState, tx: &mut Tx| {
                                                tx.copy(pub_copy.clone());
                                                s.push_toast(
                                                    "copy-pubkey",
                                                    "Chave pública copiada",
                                                    None,
                                                    ToastKind::Success,
                                                    tx,
                                                );
                                            },
                                        )),
                                )
                                .child(
                                    text(pub_key.clone())
                                        .font(ty::MONO_SM)
                                        .color(t.text_secondary)
                                        .wrap(),
                                ),
                        );
                    }
                }
                ServerAuthChoice::PasswordInVault => {
                    let pass_in = TextInput::new(&draft.password)
                        .label("Senha SSH")
                        .placeholder(if draft.has_saved_password {
                            "•••••••• (salva no Cofre do Windows)"
                        } else {
                            "Digite a senha SSH"
                        })
                        .secure()
                        .width(None)
                        .on_edit(|s: &mut AppState| match &mut s.modal {
                            ModalState::ServerEditor(d) => &mut d.password,
                            _ => unreachable!(),
                        })
                        .build(k)
                        .cursor(Cursor::Text);

                    form = form
                        .child(labeled(t, "Senha SSH (armazenada no Cofre do Windows)", pass_in))
                        .child(
                            text("A senha é salva com criptografia no Gerenciador de Credenciais do Windows e nunca vai para arquivos JSON.")
                                .font(ty::CAPTION)
                                .color(t.text_muted)
                                .wrap(),
                        );
                }
            }

            if let Some(err) = &draft.error {
                form = form.child(error_banner(t, err));
            }

            let footer = row()
                .w_full()
                .justify_end()
                .gap(10.0)
                .child(btn_neutral(
                    t,
                    "srv-cancel",
                    "Cancelar",
                    None,
                    34.0,
                    |s: &mut AppState, _| s.modal = ModalState::None,
                ))
                .child(primary_button(
                    t,
                    "srv-save",
                    "Salvar servidor",
                    Some(&icons::CHECK_CIRCLE),
                    34.0,
                    |s: &mut AppState, tx: &mut Tx| s.save_server_draft(tx),
                ));

            mount_shell_dialog(t, "server", title, 620.0, form, footer, layer);
        }
        ModalState::DeleteServerConfirm { server_name, .. } => {
            let body = col()
                .w_full()
                .gap(8.0)
                .child(
                    text(format!("Remover o servidor '{server_name}'?"))
                        .font(ty::BODY_MED)
                        .color(t.text_primary)
                        .wrap(),
                )
                .child(
                    text("As credenciais salvas no Cofre do Windows para esta VPS também serão apagadas.")
                        .font(ty::SMALL)
                        .color(t.text_muted)
                        .wrap(),
                );
            let footer = row()
                .w_full()
                .justify_end()
                .gap(10.0)
                .child(btn_neutral(
                    t,
                    "delsrv-cancel",
                    "Cancelar",
                    None,
                    34.0,
                    |s: &mut AppState, _| s.modal = ModalState::None,
                ))
                .child(btn_danger(
                    t,
                    "delsrv-confirm",
                    "Excluir servidor",
                    Some(&icons::TRASH),
                    34.0,
                    |s: &mut AppState, tx: &mut Tx| s.confirm_delete_server(tx),
                ));

            mount_shell_dialog(
                t,
                "delete-server",
                "Excluir servidor VPS",
                460.0,
                body,
                footer,
                layer,
            );
        }
        ModalState::BindingEditor(draft) => {
            let title = if draft.id.is_some() {
                "Editar vínculo de repositório"
            } else {
                "Vincular repositório a servidor"
            };

            let repo_in = TextInput::new(&draft.full_name)
                .label("Repositório GitHub (dono/nome)")
                .placeholder("widder/erp-central")
                .width(None)
                .on_edit(|s: &mut AppState| match &mut s.modal {
                    ModalState::BindingEditor(d) => &mut d.full_name,
                    _ => unreachable!(),
                })
                .build(k)
                .cursor(Cursor::Text);

            let repo_trimmed = draft.full_name.text.trim().to_string();
            let branch_in = TextInput::new(&draft.branch)
                .label("Branch monitorada")
                .placeholder("main ou *")
                .width(Some(220.0))
                .on_edit(|s: &mut AppState| match &mut s.modal {
                    ModalState::BindingEditor(d) => &mut d.branch,
                    _ => unreachable!(),
                })
                .build(k)
                .cursor(Cursor::Text);

            let branches_match = !repo_trimmed.is_empty()
                && s.snapshot
                    .repo_branches_repo
                    .eq_ignore_ascii_case(&repo_trimmed);
            let show_branch_status = !s.snapshot.repo_branches_status.trim().is_empty()
                && (branches_match || s.snapshot.repo_branches_repo.is_empty());
            let selected_branches: Vec<String> = draft
                .branch
                .text
                .split(',')
                .map(str::trim)
                .filter(|x| !x.is_empty() && *x != "*")
                .map(str::to_string)
                .collect();

            let branch_quick_chips = row()
                .w_full()
                .min_w(0.0)
                .gap(6.0)
                .flex_wrap()
                .child(
                    chip(
                        t,
                        "bind-branch-all",
                        "Todas (*)",
                        draft.branch.text.trim() == "*",
                        |s: &mut AppState, _| {
                            if let ModalState::BindingEditor(d) = &mut s.modal {
                                d.branch = rune::TextEdit::new("*");
                            }
                        },
                    )
                    .help("Monitora todas as branches deste repositório."),
                )
                .child(
                    chip(
                        t,
                        "bind-branch-none",
                        "Nenhuma",
                        draft.branch.text.trim().is_empty(),
                        |s: &mut AppState, _| {
                            if let ModalState::BindingEditor(d) = &mut s.modal {
                                d.branch = rune::TextEdit::new("");
                            }
                        },
                    )
                    .help("Limpa a seleção atual de branches para escolher manualmente."),
                );

            let branches_loading = s.snapshot.repo_branches_loading;
            let mut branch_section = col()
                .w_full()
                .min_w(0.0)
                .gap(8.0)
                .child(
                    row()
                        .w_full()
                        .min_w(0.0)
                        .gap(8.0)
                        .center_y()
                        .flex_wrap()
                        .child(branch_in)
                        .child(
                            btn_neutral(
                                t,
                                "bind-fetch-branches",
                                if branches_loading {
                                    "Buscando..."
                                } else {
                                    "Buscar branches"
                                },
                                Some(&icons::BRANCH),
                                32.0,
                                move |s: &mut AppState, tx: &mut Tx| {
                                    if !branches_loading {
                                        s.fetch_binding_branches(tx);
                                    }
                                },
                            )
                            .help("Consulta as branches disponíveis deste repositório no GitHub."),
                        ),
                )
                .child(branch_quick_chips);

            if branches_match && !s.snapshot.repo_branches.is_empty() {
                let mut fetched_chips = row().w_full().min_w(0.0).gap(6.0).flex_wrap();
                for (idx, br) in s.snapshot.repo_branches.iter().enumerate() {
                    let active = selected_branches.iter().any(|x| x == br);
                    let br_name = br.clone();
                    fetched_chips = fetched_chips.child(
                        chip(
                            t,
                            format!("bind-branch-opt-{idx}"),
                            br.clone(),
                            active,
                            move |s: &mut AppState, _| {
                                if let ModalState::BindingEditor(d) = &mut s.modal {
                                    AppState::toggle_binding_branch(&mut d.branch, &br_name);
                                }
                            },
                        )
                        .help(format!("Adicionar ou remover '{br}' da lista de branches monitoradas.")),
                    );
                }
                branch_section = branch_section.child(
                    col()
                        .key("bind-branches-scroll")
                        .w_full()
                        .min_w(0.0)
                        .max_h(104.0)
                        .min_h(0.0)
                        .padding(8.0)
                        .radius(8.0)
                        .fill(t.surface_2)
                        .border(1.0, t.border_subtle)
                        .clip_rounded(8.0)
                        .clip_soft(0.0)
                        .scroll(ScrollSpec {
                            stick_bottom: false,
                        })
                        .scroll_edge_fade(8.0)
                        .child(fetched_chips),
                );
            }

            if show_branch_status {
                let st_lower = s.snapshot.repo_branches_status.to_lowercase();
                let st_err = st_lower.contains("erro")
                    || st_lower.contains("falha")
                    || st_lower.contains("não encontrado")
                    || st_lower.contains("informe")
                    || st_lower.contains("inválido");
                branch_section = branch_section.child(
                    text(s.snapshot.repo_branches_status.clone())
                        .font(ty::CAPTION)
                        .color(if st_err { t.danger } else { t.text_muted })
                        .wrap(),
                );
            }

            let mut target_chips = row().gap(6.0).flex_wrap().child(
                chip_with_icon(
                    t,
                    "bind-srv-local",
                    "Local (Esta máquina)",
                    Some(&icons::LAPTOP),
                    draft.target_server_id.is_none(),
                    |s: &mut AppState, _| {
                        if let ModalState::BindingEditor(d) = &mut s.modal {
                            d.target_server_id = None;
                            if d.trigger_mode != TriggerMode::PollOnDesktop {
                                d.trigger_mode = TriggerMode::PollOnDesktop;
                            }
                        }
                    },
                )
                .help("Executa o fluxo diretamente nesta máquina Windows sempre que houver disparo manual ou sondagem local."),
            );
            for srv in &s.snapshot.settings.servers {
                let sid = srv.id.clone();
                let active = draft.target_server_id.as_deref() == Some(srv.id.as_str());
                let tip = format!(
                    "Executa os builds e deploys remotamente na VPS '{}' ({}:{}).",
                    srv.name, srv.host, srv.port
                );
                target_chips = target_chips.child(
                    chip_with_icon(
                        t,
                        format!("bind-srv-{}", srv.id),
                        srv.name.clone(),
                        Some(&icons::SERVER),
                        active,
                        move |s: &mut AppState, _| {
                            if let ModalState::BindingEditor(d) = &mut s.modal {
                                d.target_server_id = Some(sid.clone());
                            }
                        },
                    )
                    .help(tip),
                );
            }

            let platform_chips = row()
                .w_full()
                .min_w(0.0)
                .gap(6.0)
                .flex_wrap()
                .child(
                    chip(
                        t,
                        "bind-plat-auto",
                        "Multi / Auto",
                        draft.platform == PlatformTarget::CrossPlatform,
                        |s: &mut AppState, _| {
                            if let ModalState::BindingEditor(d) = &mut s.modal {
                                d.platform = PlatformTarget::CrossPlatform;
                            }
                        },
                    )
                    .help("Projeto multiplataforma (Node/Web/Python/etc.) que pode rodar tanto no Windows Local quanto em VPS Linux."),
                )
                .child(
                    chip(
                        t,
                        "bind-plat-windows",
                        "Windows",
                        draft.platform == PlatformTarget::Windows,
                        |s: &mut AppState, _| {
                            if let ModalState::BindingEditor(d) = &mut s.modal {
                                d.platform = PlatformTarget::Windows;
                            }
                        },
                    )
                    .help("Repositório que exige toolchain Windows (.exe, MSI, PowerShell, Win32)."),
                )
                .child(
                    chip(
                        t,
                        "bind-plat-linux",
                        "Linux",
                        draft.platform == PlatformTarget::Linux,
                        |s: &mut AppState, _| {
                            if let ModalState::BindingEditor(d) = &mut s.modal {
                                d.platform = PlatformTarget::Linux;
                            }
                        },
                    )
                    .help("Repositório que exige ambiente Linux (Docker, Compose, systemd ou shell Unix)."),
                )
                .child(
                    chip(
                        t,
                        "bind-plat-macos",
                        "macOS",
                        draft.platform == PlatformTarget::MacOS,
                        |s: &mut AppState, _| {
                            if let ModalState::BindingEditor(d) = &mut s.modal {
                                d.platform = PlatformTarget::MacOS;
                            }
                        },
                    )
                    .help("Repositório Apple/Xcode que exige runner macOS."),
                );

            let mode_chips = row()
                .gap(6.0)
                .flex_wrap()
                .child(
                    chip_with_icon(
                        t,
                        "bind-mode-webhook",
                        "Webhook 24/7 na VPS",
                        Some(&icons::CLOUD_BOLT),
                        draft.trigger_mode == TriggerMode::WebhookOnVps,
                        |s: &mut AppState, _| {
                            if let ModalState::BindingEditor(d) = &mut s.modal {
                                d.trigger_mode = TriggerMode::WebhookOnVps;
                            }
                        },
                    )
                    .help("Recebe eventos de push do GitHub em tempo real direto na VPS, funcionando 24/7 mesmo com este PC desligado."),
                )
                .child(
                    chip_with_icon(
                        t,
                        "bind-mode-pollvps",
                        "Sondagem 24/7 na VPS",
                        Some(&icons::REFRESH),
                        draft.trigger_mode == TriggerMode::PollOnVps,
                        |s: &mut AppState, _| {
                            if let ModalState::BindingEditor(d) = &mut s.modal {
                                d.trigger_mode = TriggerMode::PollOnVps;
                            }
                        },
                    )
                    .help("O agente workploy-runner na VPS consulta o GitHub periodicamente 24/7, sem precisar expor porta pública para webhook."),
                )
                .child(
                    chip_with_icon(
                        t,
                        "bind-mode-polldesktop",
                        "Sondagem no Desktop",
                        Some(&icons::LAPTOP),
                        draft.trigger_mode == TriggerMode::PollOnDesktop,
                        |s: &mut AppState, _| {
                            if let ModalState::BindingEditor(d) = &mut s.modal {
                                d.trigger_mode = TriggerMode::PollOnDesktop;
                            }
                        },
                    )
                    .help("Este aplicativo verifica novos commits no GitHub enquanto estiver aberto no seu computador."),
                );

            let mut wf_chips = row().gap(6.0).flex_wrap().child(
                chip(
                    t,
                    "bind-wf-auto",
                    "Automático (por gatilho)",
                    draft.workflow_id.is_none(),
                    |s: &mut AppState, _| {
                        if let ModalState::BindingEditor(d) = &mut s.modal {
                            d.workflow_id = None;
                        }
                    },
                )
                .help("Seleciona automaticamente os fluxos compatíveis configurados com gatilho de push no GitHub."),
            );
            for wf in &s.snapshot.workflows {
                let wid = wf.id.clone();
                let active = draft.workflow_id.as_deref() == Some(wf.id.as_str());
                let tip = format!(
                    "Dispara especificamente o fluxo '{}' quando este repositório receber atualização.",
                    wf.name
                );
                wf_chips = wf_chips.child(
                    chip(
                        t,
                        format!("bind-wf-{}", wf.id),
                        wf.name.clone(),
                        active,
                        move |s: &mut AppState, _| {
                            if let ModalState::BindingEditor(d) = &mut s.modal {
                                d.workflow_id = Some(wid.clone());
                            }
                        },
                    )
                    .help(tip),
                );
            }

            let workflows_match = !repo_trimmed.is_empty()
                && s.snapshot
                    .repo_workflows_repo
                    .eq_ignore_ascii_case(&repo_trimmed);
            let show_wf_status = !s.snapshot.repo_workflows_status.trim().is_empty()
                && (workflows_match || s.snapshot.repo_workflows_repo.is_empty());
            let workflows_loading = s.snapshot.repo_workflows_loading;

            let mut wf_actions = row()
                .w_full()
                .min_w(0.0)
                .gap(8.0)
                .center_y()
                .flex_wrap()
                .child(wf_chips)
                .child(
                    btn_neutral(
                        t,
                        "bind-scan-workflows",
                        if workflows_loading {
                            "Buscando..."
                        } else {
                            "Buscar fluxos no repo"
                        },
                        Some(&icons::CODE),
                        30.0,
                        move |s: &mut AppState, tx: &mut Tx| {
                            if !workflows_loading {
                                s.scan_binding_workflows(tx);
                            }
                        },
                    )
                    .help("Procura arquivos de automação (.github/workflows, Dockerfile, etc.) no repositório."),
                );
            if let Some(binding_id) = draft.id.clone() {
                wf_actions = wf_actions.child(
                    btn_neutral(
                        t,
                        "bind-gen-wf",
                        "Gerar receita do repo",
                        Some(&icons::BOLT),
                        30.0,
                        move |s: &mut AppState, tx: &mut Tx| {
                            s.generate_workflow_for_binding(&binding_id, tx);
                            s.modal = ModalState::None;
                        },
                    )
                    .help("Lê package.json, Cargo.toml e docker-compose.yml no GitHub para criar passos reais de testes e redeploy automático."),
                );
            }

            let mut wf_section = col()
                .w_full()
                .min_w(0.0)
                .gap(8.0)
                .child(wf_actions);

            if show_wf_status {
                let wf_st_lower = s.snapshot.repo_workflows_status.to_lowercase();
                let wf_st_err = wf_st_lower.contains("erro")
                    || wf_st_lower.contains("falha")
                    || wf_st_lower.contains("não encontrado")
                    || wf_st_lower.contains("informe")
                    || wf_st_lower.contains("não foi possível");
                let mut detected_box = col()
                    .key("bind-detected-workflows")
                    .w_full()
                    .padding(10.0)
                    .gap(6.0)
                    .radius(8.0)
                    .fill(t.surface_2)
                    .border(
                        1.0,
                        if wf_st_err {
                            t.danger.alpha(0.35)
                        } else {
                            t.border_subtle
                        },
                    )
                    .child(
                        text(s.snapshot.repo_workflows_status.clone())
                            .font(ty::CAPTION)
                            .color(if wf_st_err { t.danger } else { t.text_secondary })
                            .wrap(),
                    );
                if workflows_match && !s.snapshot.repo_workflows.is_empty() {
                    let mut items_col = col().w_full().gap(4.0);
                    for dw in &s.snapshot.repo_workflows {
                        items_col = items_col.child(
                            row()
                                .w_full()
                                .justify_between()
                                .center_y()
                                .flex_wrap()
                                .gap(6.0)
                                .child(
                                    text(dw.path.clone())
                                        .font(ty::MONO_SM)
                                        .color(t.text_primary)
                                        .grow(1.0)
                                        .shrink()
                                        .truncate(),
                                )
                                .child(pill_badge(
                                    t,
                                    &dw.name,
                                    t.accent,
                                    t.accent_soft,
                                )),
                        );
                    }
                    detected_box = detected_box.child(items_col).child(
                        text("Dica: o Workploy pode disparar o fluxo selecionado acima automaticamente a cada push neste repositório.")
                            .font(ty::CAPTION)
                            .color(t.text_muted)
                            .wrap(),
                    );
                }
                wf_section = wf_section.child(detected_box);
            }

            let mut form = col()
                .w_full()
                .min_w(0.0)
                .gap(12.0)
                .child(labeled(t, "Repositório GitHub (dono/nome)", repo_in))
                .child(labeled(t, "Branch monitorada", branch_section))
                .child(labeled(t, "Servidor de destino", target_chips))
                .child(labeled(t, "Plataforma exigida", platform_chips));

            if let Some(warn) =
                platform_compat_warning(draft.platform, draft.target_server_id.as_deref())
            {
                form = form.child(warning_banner(t, warn).key("bind-platform-warning"));
            }

            form = form
                .child(labeled(t, "Modo de disparo", mode_chips))
                .child(labeled(t, "Fluxo disparado", wf_section))
                .child(
                    Switch::new(draft.enabled)
                        .label("Vínculo ativo")
                        .on_toggle(|s: &mut AppState| {
                            if let ModalState::BindingEditor(d) = &mut s.modal {
                                d.enabled = !d.enabled;
                            }
                        })
                        .build(k)
                        .cursor(Cursor::Pointer)
                        .help("Ativa ou pausa o monitoramento e os disparos automáticos deste vínculo sem apagar sua configuração."),
                );

            if let Some(err) = &draft.error {
                form = form.child(error_banner(t, err));
            }

            let footer = row()
                .w_full()
                .justify_end()
                .gap(10.0)
                .child(btn_neutral(
                    t,
                    "bind-cancel",
                    "Cancelar",
                    None,
                    34.0,
                    |s: &mut AppState, _| s.modal = ModalState::None,
                ))
                .child(primary_button(
                    t,
                    "bind-save",
                    "Salvar vínculo",
                    Some(&icons::CHECK_CIRCLE),
                    34.0,
                    |s: &mut AppState, tx: &mut Tx| s.save_binding_draft(tx),
                ));

            mount_shell_dialog(t, "binding", title, 640.0, form, footer, layer);
        }
        ModalState::ImportFromRepo {
            binding_id,
            loading,
            paths,
            status,
            selected,
        } => {
            let repo_label = s
                .snapshot
                .settings
                .repo_bindings
                .iter()
                .find(|b| b.id == *binding_id)
                .map(|b| b.full_name.trim().to_string())
                .unwrap_or_default();
            let title = format!("Importar do repositório — {repo_label}");

            let mut form = col()
                .w_full()
                .gap(12.0)
                .child(
                    text("O repositório vira a fonte da verdade: o Workploy procura .workploy.yml na raiz (ou receitas em infra/workploy/*.yml), importa ou atualiza preservando id e vínculos, e resolve o encadeamento (on_success) automaticamente.")
                        .font(ty::SMALL)
                        .color(t.text_muted)
                        .wrap(),
                );

            if *loading {
                let pulse_dot = el()
                    .size(8.0, 8.0)
                    .center()
                    .child(
                        effect(crate::gfx::MARK_PULSE_RADAR)
                            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                            .radius(4.0)
                            .effect_bleed(7.0)
                            .effect_data([4.0, 7.0, PULSE_TEMPO_ACTIVE, PULSE_INTENSITY_ACTIVE]),
                    )
                    .child(el().size(8.0, 8.0).radius(4.0).fill(t.info));
                form = form.child(
                    row()
                        .key("imp-loading")
                        .w_full()
                        .px(14.0)
                        .py(16.0)
                        .gap(10.0)
                        .center_y()
                        .radius(8.0)
                        .fill(t.surface_2)
                        .border(1.0, t.border_subtle)
                        .child(pulse_dot)
                        .child(
                            text("Buscando receitas no repositório...")
                                .font(ty::SMALL)
                                .color(t.text_secondary),
                        ),
                );
            } else if paths.is_empty() {
                form = form.child(
                    row()
                        .key("imp-empty")
                        .w_full()
                        .px(14.0)
                        .py(14.0)
                        .gap(9.0)
                        .center_y()
                        .radius(8.0)
                        .fill(t.warning_bg)
                        .border(1.0, t.warning.alpha(0.35))
                        .child(icon(&icons::INFO, 14.0).color(t.warning).a11y_hidden())
                        .child(
                            text("Nenhum .workploy.yml ou infra/workploy/*.yml encontrado — commit o arquivo e tente de novo.")
                                .font(ty::SMALL)
                                .color(t.warning)
                                .grow(1.0)
                                .shrink()
                                .wrap(),
                        ),
                );
            } else {
                let mut list = col().w_full().gap(6.0);
                for (i, path) in paths.iter().enumerate() {
                    let is_sel = selected.as_deref() == Some(path.as_str());
                    let path_owned = path.clone();
                    list = list.child(
                        row()
                            .key(format!("imp-path-{i}"))
                            .role("button")
                            .label(format!("Escolher receita {path}"))
                            .checked(is_sel)
                            .w_full()
                            .min_h(34.0)
                            .px(10.0)
                            .py(6.0)
                            .gap(9.0)
                            .center_y()
                            .radius(7.0)
                            .fill(if is_sel {
                                t.accent_soft
                            } else {
                                t.surface_2
                            })
                            .border(
                                1.0,
                                if is_sel {
                                    t.accent.alpha(0.45)
                                } else {
                                    t.border_subtle
                                },
                            )
                            .cursor(Cursor::Pointer)
                            .focusable(true)
                            .focus_ring(2.0, 2.0, t.accent)
                            .hover(t.hover)
                            .press(t.press)
                            .on_press(move |s: &mut AppState| {
                                if let ModalState::ImportFromRepo { selected, .. } =
                                    &mut s.modal
                                {
                                    *selected = Some(path_owned.clone());
                                }
                            })
                            .child(
                                el()
                                    .size(14.0, 14.0)
                                    .center()
                                    .radius(7.0)
                                    .border(1.4, if is_sel { t.accent } else { t.border_strong })
                                    .child(if is_sel {
                                        el().size(6.0, 6.0).radius(3.0).fill(t.accent)
                                    } else {
                                        el()
                                    }),
                            )
                            .child(
                                icon(&icons::DOCUMENTS, 13.5)
                                    .color(if is_sel { t.accent } else { t.text_muted })
                                    .a11y_hidden(),
                            )
                            .child(
                                text(path.clone())
                                    .font(ty::MONO_SM)
                                    .color(if is_sel {
                                        t.text_primary
                                    } else {
                                        t.text_secondary
                                    })
                                    .grow(1.0)
                                    .shrink()
                                    .truncate()
                                    .a11y_hidden(),
                            ),
                    );
                }
                form = form.child(list);
            }

            form = form.child(
                text(status.clone())
                    .font(ty::CAPTION)
                    .color(t.text_muted)
                    .wrap(),
            );

            let can_import = !*loading && !paths.is_empty() && selected.is_some();
            let footer = row()
                .w_full()
                .justify_between()
                .gap(10.0)
                .child(
                    btn_neutral(
                        t,
                        "imp-cancel",
                        "Cancelar",
                        None,
                        34.0,
                        |s: &mut AppState, _| {
                            s.modal = ModalState::None;
                        },
                    )
                    .help("Fecha sem importar nada deste repositório."),
                )
                .child(
                    primary_button(
                        t,
                        "imp-confirm",
                        if *loading { "Buscando..." } else { "Importar receita" },
                        Some(&icons::DOCUMENTS),
                        34.0,
                        move |s: &mut AppState, tx: &mut Tx| {
                            if let ModalState::ImportFromRepo {
                                binding_id, selected, ..
                            } = &s.modal
                                && let Some(path) = selected.clone()
                            {
                                let bid = binding_id.clone();
                                s.import_recipe_from_repo(&bid, &path, tx);
                            }
                        },
                    )
                    .disabled(!can_import)
                    .opacity(if can_import { 1.0 } else { 0.65 })
                    .help("Reimporta o arquivo escolhido: atualiza a receita existente (preservando id e encadeamento) ou cria uma nova."),
                );

            mount_shell_dialog(t, "import-repo", &title, 560.0, form, footer, layer);
        }
        ModalState::RepoCommits {
            binding_id,
            full_name,
            branch_input,
        } => {
            let commits_loading = s.snapshot.repo_commits_loading;
            let branch_in = TextInput::new(branch_input)
                .label("Branch")
                .placeholder("main")
                .width(Some(220.0))
                .height(32.0)
                .on_edit(|s: &mut AppState| match &mut s.modal {
                    ModalState::RepoCommits { branch_input, .. } => branch_input,
                    _ => unreachable!(),
                })
                .build(k)
                .key("commits-branch-input")
                .cursor(Cursor::Text);

            let top_controls = row()
                .w_full()
                .min_w(0.0)
                .gap(8.0)
                .center_y()
                .flex_wrap()
                .child(branch_in)
                .child(
                    btn_neutral(
                        t,
                        "commits-fetch-btn",
                        if commits_loading {
                            "Buscando..."
                        } else {
                            "Buscar commits"
                        },
                        Some(&icons::REFRESH),
                        32.0,
                        move |s: &mut AppState, tx: &mut Tx| {
                            if !commits_loading {
                                s.fetch_repo_commits_for_modal(tx);
                            }
                        },
                    )
                    .disabled(commits_loading)
                    .help("Consulta os últimos commits da branch informada no GitHub."),
                );

            let mut body = col()
                .w_full()
                .min_w(0.0)
                .gap(10.0)
                .child(
                    text("Escolha um commit abaixo para executar imediatamente o fluxo deste projeto na versão exata selecionada.")
                        .font(ty::SMALL)
                        .color(t.text_muted)
                        .wrap(),
                )
                .child(top_controls);

            if commits_loading {
                body = body.child(
                    col()
                        .key("commits-loading")
                        .w_full()
                        .min_w(0.0)
                        .gap(6.0)
                        .child(
                            text(if s.snapshot.repo_commits_status.is_empty() {
                                "Buscando commits no GitHub...".to_string()
                            } else {
                                s.snapshot.repo_commits_status.clone()
                            })
                            .font(ty::CAPTION)
                            .color(t.info)
                            .wrap(),
                        )
                        .child(Progress::new(None).build(k)),
                );
            } else if !s.snapshot.repo_commits_status.trim().is_empty() {
                let st_lower = s.snapshot.repo_commits_status.to_lowercase();
                let st_err = st_lower.contains("erro")
                    || st_lower.contains("falha")
                    || st_lower.contains("não encontrado")
                    || st_lower.contains("inválido")
                    || st_lower.contains("expirado");
                body = body.child(
                    text(s.snapshot.repo_commits_status.clone())
                        .key("commits-status-line")
                        .font(ty::CAPTION)
                        .color(if st_err { t.danger } else { t.text_muted })
                        .wrap(),
                );
            }

            let list_content = if s.snapshot.repo_commits.is_empty() {
                row()
                    .key("commits-empty")
                    .w_full()
                    .min_w(0.0)
                    .px(14.0)
                    .py(18.0)
                    .center()
                    .radius(8.0)
                    .fill(t.surface_2)
                    .border(1.0, t.border_subtle)
                    .child(
                        text(if commits_loading {
                            "Carregando histórico de commits..."
                        } else {
                            "Nenhum commit encontrado para esta branch."
                        })
                        .font(ty::SMALL)
                        .color(t.text_muted)
                        .wrap(),
                    )
            } else {
                let mut rows_col = col().w_full().min_w(0.0).gap(8.0);
                for (idx, c) in s.snapshot.repo_commits.iter().enumerate() {
                    let already_ran = s
                        .snapshot
                        .runs
                        .iter()
                        .any(|r| r.commit.as_deref() == Some(c.sha.as_str()));
                    let status_badge = if already_ran {
                        pill_badge(t, "Já rodou", t.success, t.success_bg)
                    } else {
                        pill_badge(t, "Pendente", t.text_muted, t.surface_3)
                    };
                    let bid_run = binding_id.clone();
                    let sha_run = c.sha.clone();
                    let msg_run = c.message.clone();
                    let br_run = if c.branch.trim().is_empty() {
                        branch_input.text.trim().to_string()
                    } else {
                        c.branch.clone()
                    };
                    let date_label = fmt_commit_date(&c.date);

                    let br_help = br_run.clone();
                    rows_col = rows_col.child(
                        row()
                            .key(format!("commit-row-{idx}"))
                            .w_full()
                            .min_w(0.0)
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
                                if already_ran {
                                    t.success.alpha(0.32)
                                } else {
                                    t.border_subtle
                                },
                            )
                            .child(
                                col()
                                    .grow(1.0)
                                    .shrink()
                                    .min_w(0.0)
                                    .gap(4.0)
                                    .child(
                                        row()
                                            .w_full()
                                            .min_w(0.0)
                                            .gap(6.0)
                                            .center_y()
                                            .flex_wrap()
                                            .child(
                                                row()
                                                    .h(20.0)
                                                    .px(6.0)
                                                    .gap(4.0)
                                                    .center_y()
                                                    .radius(5.0)
                                                    .fill(t.surface_3)
                                                    .child(
                                                        icon(&icons::COMMIT, 11.0)
                                                            .color(t.accent)
                                                            .a11y_hidden(),
                                                    )
                                                    .child(
                                                        text(c.short_sha.clone())
                                                            .font(ty::MONO_SM)
                                                            .color(t.text_primary),
                                                    ),
                                            )
                                            .child(status_badge.key(format!("commit-badge-{idx}"))),
                                    )
                                    .child(
                                        text(c.message.clone())
                                            .font(ty::BODY_MED)
                                            .color(t.text_primary)
                                            .w_full()
                                            .min_w(0.0)
                                            .wrap(),
                                    )
                                    .child(
                                        text(if date_label.is_empty() {
                                            format!("por {}", c.author)
                                        } else {
                                            format!("por {} · {}", c.author, date_label)
                                        })
                                        .font(ty::CAPTION)
                                        .color(t.text_muted)
                                        .w_full()
                                        .min_w(0.0)
                                        .wrap(),
                                    ),
                            )
                            .child(
                                primary_button(
                                    t,
                                    format!("commit-run-{idx}"),
                                    "Rodar fluxo",
                                    Some(&icons::PLAY),
                                    30.0,
                                    move |s: &mut AppState, tx: &mut Tx| {
                                        s.run_binding_commit_now(
                                            &bid_run, &sha_run, &msg_run, &br_run, tx,
                                        );
                                    },
                                )
                                .help(format!(
                                    "Dispara o fluxo deste projeto para o commit {} ({}).",
                                    c.short_sha, br_help
                                )),
                            ),
                    );
                }
                rows_col
            };

            body = body.child(
                col()
                    .key("commits-list-scroll")
                    .w_full()
                    .min_w(0.0)
                    .max_h(360.0)
                    .min_h(0.0)
                    .clip_rounded(8.0)
                    .clip_soft(0.0)
                    .scroll(ScrollSpec {
                        stick_bottom: false,
                    })
                    .scroll_edge_fade(10.0)
                    .child(list_content),
            );

            let footer = row()
                .w_full()
                .min_w(0.0)
                .justify_end()
                .child(btn_neutral(
                    t,
                    "commits-close",
                    "Fechar",
                    None,
                    34.0,
                    |s: &mut AppState, _| s.modal = ModalState::None,
                ));

            mount_shell_dialog(
                t,
                "commits",
                &format!("Commits — {full_name}"),
                680.0,
                body,
                footer,
                layer,
            );
        }
        ModalState::ProvisionModal(draft) => {
            let is_provisioning =
                s.snapshot.provisioning_server_id.as_deref() == Some(draft.server_id.as_str());
            let has_other_provisioning =
                s.snapshot.provisioning_server_id.is_some() && !is_provisioning;
            let prov_succeeded = !is_provisioning
                && s.snapshot
                    .provision_log
                    .contains("=== Provisionamento concluído! ===");
            let prov_failed = !is_provisioning
                && !prov_succeeded
                && (s.snapshot.provision_log.contains("[erro]")
                    || s.snapshot
                        .runner_sync_status
                        .contains("Provisionamento falhou"));

            let port_in = TextInput::new(&draft.runner_port)
                .label("Porta do workploy-runner")
                .placeholder("8787")
                .width(Some(140.0))
                .on_edit(|s: &mut AppState| match &mut s.modal {
                    ModalState::ProvisionModal(d) => &mut d.runner_port,
                    _ => unreachable!(),
                })
                .build(k)
                .cursor(Cursor::Text);

            let checklist = col()
                .w_full()
                .min_w(0.0)
                .padding(12.0)
                .gap(10.0)
                .radius(10.0)
                .fill(t.surface_2)
                .border(1.0, t.border_subtle)
                .child(
                    Switch::new(draft.install_git)
                        .label("Git (controle de versão)")
                        .on_toggle(|s: &mut AppState| {
                            if let ModalState::ProvisionModal(d) = &mut s.modal {
                                d.install_git = !d.install_git;
                            }
                        })
                        .build(k)
                        .cursor(Cursor::Pointer),
                )
                .child(
                    Switch::new(draft.install_docker)
                        .label("Docker + Docker Compose")
                        .on_toggle(|s: &mut AppState| {
                            if let ModalState::ProvisionModal(d) = &mut s.modal {
                                d.install_docker = !d.install_docker;
                            }
                        })
                        .build(k)
                        .cursor(Cursor::Pointer),
                )
                .child(
                    Switch::new(draft.install_node)
                        .label("Node.js 22 + pnpm (via corepack)")
                        .on_toggle(|s: &mut AppState| {
                            if let ModalState::ProvisionModal(d) = &mut s.modal {
                                d.install_node = !d.install_node;
                            }
                        })
                        .build(k)
                        .cursor(Cursor::Pointer),
                )
                .child(
                    Switch::new(draft.install_rust)
                        .label("Rust toolchain (rustup + cargo)")
                        .on_toggle(|s: &mut AppState| {
                            if let ModalState::ProvisionModal(d) = &mut s.modal {
                                d.install_rust = !d.install_rust;
                            }
                        })
                        .build(k)
                        .cursor(Cursor::Pointer),
                )
                .child(
                    Switch::new(draft.install_runner_service)
                        .label("Agente workploy-runner 24/7 (serviço systemd)")
                        .on_toggle(|s: &mut AppState| {
                            if let ModalState::ProvisionModal(d) = &mut s.modal {
                                d.install_runner_service = !d.install_runner_service;
                            }
                        })
                        .build(k)
                        .cursor(Cursor::Pointer),
                );

            let mut form = col()
                .w_full()
                .min_w(0.0)
                .gap(12.0)
                .child(
                    text("Selecione os componentes para instalar ou atualizar via SSH na sua VPS Linux.")
                        .font(ty::SMALL)
                        .color(t.text_muted)
                        .wrap(),
                )
                .child(checklist)
                .child(labeled(t, "Porta HTTP do agente workploy-runner", port_in));

            if is_provisioning {
                let pulse_dot = el()
                    .key("prov-active-pulse")
                    .size(8.0, 8.0)
                    .center()
                    .child(
                        effect(MARK_PULSE_RADAR)
                            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                            .radius(4.0)
                            .effect_bleed(8.0)
                            .effect_data([4.0, 8.0, PULSE_TEMPO_ACTIVE, PULSE_INTENSITY_ACTIVE]),
                    )
                    .child(el().size(8.0, 8.0).radius(4.0).fill(t.info));

                form = form.child(
                    col()
                        .key("prov-progress-indicator")
                        .w_full()
                        .min_w(0.0)
                        .padding(10.0)
                        .gap(8.0)
                        .radius(8.0)
                        .fill(t.info_bg)
                        .border(1.0, t.info.alpha(0.32))
                        .child(
                            row()
                                .w_full()
                                .min_w(0.0)
                                .gap(8.0)
                                .center_y()
                                .child(pulse_dot)
                                .child(
                                    text("Provisionando servidor via SSH...")
                                        .font(ty::SMALL_MED)
                                        .color(t.info)
                                        .grow(1.0)
                                        .shrink()
                                        .min_w(0.0)
                                        .wrap(),
                                )
                                .child(pill_badge(t, "Ao vivo", t.info, t.surface_1)),
                        )
                        .child(Progress::new(None).build(k)),
                );
            }

            if is_provisioning || !s.snapshot.provision_log.trim().is_empty() {
                let tail: String = {
                    let lines: Vec<&str> = s.snapshot.provision_log.lines().collect();
                    let start = lines.len().saturating_sub(18);
                    lines[start..].join("\n")
                };

                let mut log_header = row()
                    .w_full()
                    .min_w(0.0)
                    .justify_between()
                    .center_y()
                    .flex_wrap()
                    .gap(6.0)
                    .child(
                        text(if is_provisioning {
                            "Log ao vivo da instalação"
                        } else {
                            "Saída do provisionamento"
                        })
                        .font(ty::CAPTION)
                        .color(t.text_secondary),
                    );

                if is_provisioning {
                    log_header =
                        log_header.child(pill_badge(t, "Em andamento", t.info, t.info_bg));
                } else if prov_succeeded {
                    log_header = log_header.child(
                        pill_badge(t, "Concluído", t.success, t.success_bg)
                            .key("prov-success-badge"),
                    );
                } else if prov_failed {
                    log_header = log_header.child(
                        pill_badge(t, "Falhou", t.danger, t.danger_bg)
                            .key("prov-error-badge"),
                    );
                }

                form = form.child(
                    col()
                        .w_full()
                        .min_w(0.0)
                        .gap(6.0)
                        .child(log_header)
                        .child(
                            col()
                                .key("prov-log-box")
                                .w_full()
                                .min_w(0.0)
                                .padding(10.0)
                                .radius(8.0)
                                .fill(t.surface_code)
                                .border(1.0, t.border_subtle)
                                .clip_rounded(8.0)
                                .clip_soft(0.0)
                                .child(
                                    text(if tail.is_empty() {
                                        "Conectando à VPS...".to_string()
                                    } else {
                                        tail
                                    })
                                    .key("prov-log-text")
                                    .w_full()
                                    .min_w(0.0)
                                    .shrink()
                                    .font(ty::MONO_SM)
                                    .color(rune::color::hex("#e2e8f0").unwrap_or(t.text_primary))
                                    .wrap_anywhere(),
                                ),
                        ),
                );
            }

            if prov_succeeded {
                form = form.child(
                    row()
                        .key("prov-success-banner")
                        .w_full()
                        .min_w(0.0)
                        .px(12.0)
                        .py(8.0)
                        .gap(8.0)
                        .center_y()
                        .radius(8.0)
                        .fill(t.success_bg)
                        .border(1.0, t.success.alpha(0.38))
                        .child(
                            icon(&icons::CHECK_CIRCLE, 15.0)
                                .color(t.success)
                                .a11y_hidden(),
                        )
                        .child(
                            text("Provisionamento concluído! O motor na VPS está pronto para uso.")
                                .font(ty::SMALL)
                                .color(t.success)
                                .grow(1.0)
                                .shrink()
                                .min_w(0.0)
                                .wrap(),
                        ),
                );
            }

            if let Some(err) = &draft.error {
                form = form.child(error_banner(t, err));
            }

            let buttons = if is_provisioning {
                row()
                    .w_full()
                    .min_w(0.0)
                    .justify_end()
                    .flex_wrap()
                    .gap(10.0)
                    .child(btn_neutral(
                        t,
                        "prov-close",
                        "Fechar",
                        None,
                        34.0,
                        |s: &mut AppState, _| s.modal = ModalState::None,
                    ))
                    .child(
                        primary_button(
                            t,
                            "prov-start",
                            "Em andamento...",
                            Some(&icons::REFRESH),
                            34.0,
                            |_: &mut AppState, _| {},
                        )
                        .disabled(true)
                        .opacity(0.6)
                        .cursor(Cursor::Default),
                    )
            } else if prov_succeeded {
                let mut rerun_btn = btn_neutral(
                    t,
                    "prov-rerun",
                    "Executar novamente",
                    Some(&icons::REFRESH),
                    34.0,
                    move |s: &mut AppState, tx: &mut Tx| {
                        if !has_other_provisioning {
                            s.start_provision(tx);
                        }
                    },
                );
                if has_other_provisioning {
                    rerun_btn = rerun_btn
                        .disabled(true)
                        .opacity(0.6)
                        .cursor(Cursor::Default);
                }
                row()
                    .w_full()
                    .min_w(0.0)
                    .justify_end()
                    .flex_wrap()
                    .gap(10.0)
                    .child(rerun_btn)
                    .child(primary_button(
                        t,
                        "prov-close",
                        "Concluir",
                        Some(&icons::CHECK_CIRCLE),
                        34.0,
                        |s: &mut AppState, _| s.modal = ModalState::None,
                    ))
            } else if prov_failed {
                let mut retry_btn = primary_button(
                    t,
                    "prov-start",
                    "Tentar novamente",
                    Some(&icons::REFRESH),
                    34.0,
                    move |s: &mut AppState, tx: &mut Tx| {
                        if !has_other_provisioning {
                            s.start_provision(tx);
                        }
                    },
                );
                if has_other_provisioning {
                    retry_btn = retry_btn
                        .disabled(true)
                        .opacity(0.6)
                        .cursor(Cursor::Default);
                }
                row()
                    .w_full()
                    .min_w(0.0)
                    .justify_end()
                    .flex_wrap()
                    .gap(10.0)
                    .child(btn_neutral(
                        t,
                        "prov-close",
                        "Fechar",
                        None,
                        34.0,
                        |s: &mut AppState, _| s.modal = ModalState::None,
                    ))
                    .child(retry_btn)
            } else {
                let mut start_btn = primary_button(
                    t,
                    "prov-start",
                    "Iniciar provisionamento",
                    Some(&icons::BOLT),
                    34.0,
                    move |s: &mut AppState, tx: &mut Tx| {
                        if !has_other_provisioning {
                            s.start_provision(tx);
                        }
                    },
                );
                if has_other_provisioning {
                    start_btn = start_btn
                        .disabled(true)
                        .opacity(0.6)
                        .cursor(Cursor::Default);
                }
                row()
                    .w_full()
                    .min_w(0.0)
                    .justify_end()
                    .flex_wrap()
                    .gap(10.0)
                    .child(btn_neutral(
                        t,
                        "prov-close",
                        "Fechar",
                        None,
                        34.0,
                        |s: &mut AppState, _| s.modal = ModalState::None,
                    ))
                    .child(start_btn)
            };

            let footer = if prov_failed {
                col()
                    .w_full()
                    .min_w(0.0)
                    .gap(10.0)
                    .child(
                        row()
                            .key("prov-error-banner")
                            .w_full()
                            .min_w(0.0)
                            .px(12.0)
                            .py(8.0)
                            .gap(8.0)
                            .center_y()
                            .radius(8.0)
                            .fill(t.danger_bg)
                            .border(1.0, t.danger.alpha(0.4))
                            .child(
                                icon(&icons::X_CIRCLE, 15.0)
                                    .color(t.danger)
                                    .a11y_hidden(),
                            )
                            .child(
                                text("O provisionamento falhou. Verifique os erros no log acima e tente novamente.")
                                    .font(ty::SMALL)
                                    .color(t.danger)
                                    .grow(1.0)
                                    .shrink()
                                    .min_w(0.0)
                                    .wrap(),
                            ),
                    )
                    .child(buttons)
            } else {
                buttons
            };

            mount_shell_dialog(
                t,
                "provision",
                &format!("Provisionar Motor — {}", draft.server_name),
                660.0,
                form,
                footer,
                layer,
            );
        }
        ModalState::PurgeRuleEditor(draft) => {
            let runs_in = TextInput::new(&draft.every_n_runs)
                .label("A cada X execuções/testes")
                .placeholder("10")
                .width(Some(140.0))
                .on_edit(|s: &mut AppState| match &mut s.modal {
                    ModalState::PurgeRuleEditor(d) => &mut d.every_n_runs,
                    _ => unreachable!(),
                })
                .build(k)
                .cursor(Cursor::Text);

            let size_in = TextInput::new(&draft.size_value)
                .label("Quando workspaces atingirem")
                .placeholder(if draft.size_unit == PurgeSizeUnit::Gb {
                    "5"
                } else {
                    "5120"
                })
                .width(Some(140.0))
                .on_edit(|s: &mut AppState| match &mut s.modal {
                    ModalState::PurgeRuleEditor(d) => &mut d.size_value,
                    _ => unreachable!(),
                })
                .build(k)
                .cursor(Cursor::Text);

            let unit_chips = row()
                .gap(6.0)
                .center_y()
                .child(chip(
                    t,
                    "purge-unit-mb",
                    "MB",
                    draft.size_unit == PurgeSizeUnit::Mb,
                    |s: &mut AppState, _| {
                        if let ModalState::PurgeRuleEditor(d) = &mut s.modal {
                            d.set_unit(PurgeSizeUnit::Mb);
                        }
                    },
                ))
                .child(chip(
                    t,
                    "purge-unit-gb",
                    "GB",
                    draft.size_unit == PurgeSizeUnit::Gb,
                    |s: &mut AppState, _| {
                        if let ModalState::PurgeRuleEditor(d) = &mut s.modal {
                            d.set_unit(PurgeSizeUnit::Gb);
                        }
                    },
                ));

            let size_preview = match draft.parsed_max_size_mb() {
                Some(0) => "Limite por tamanho desligado (0 MB)".to_string(),
                Some(mb) => format!(
                    "Limite efetivo: {} MB ({:.1} GB)",
                    mb,
                    mb as f64 / 1024.0
                ),
                None => "Informe um número válido (ex: 5 GB ou 5120 MB)".to_string(),
            };

            let toggles_card = col()
                .w_full()
                .padding(12.0)
                .gap(10.0)
                .radius(8.0)
                .fill(t.surface_2)
                .border(1.0, t.border_subtle)
                .child(
                    Switch::new(draft.clean_workspaces)
                        .label("Limpar workspaces e caches (target/, node_modules/)")
                        .on_toggle(|s: &mut AppState| {
                            if let ModalState::PurgeRuleEditor(d) = &mut s.modal {
                                d.clean_workspaces = !d.clean_workspaces;
                            }
                        })
                        .build(k)
                        .cursor(Cursor::Pointer),
                )
                .child(
                    Switch::new(draft.clean_docker)
                        .label("Limpar cache Docker (docker system prune)")
                        .on_toggle(|s: &mut AppState| {
                            if let ModalState::PurgeRuleEditor(d) = &mut s.modal {
                                d.clean_docker = !d.clean_docker;
                            }
                        })
                        .build(k)
                        .cursor(Cursor::Pointer),
                );

            let mut form = col()
                .w_full()
                .gap(12.0)
                .child(
                    text("Configure quando o agente workploy-runner na VPS deve limpar caches antigos de builds e imagens Docker.")
                        .font(ty::SMALL)
                        .color(t.text_muted)
                        .wrap(),
                )
                .child(
                    row()
                        .w_full()
                        .padding(12.0)
                        .radius(8.0)
                        .fill(t.surface_2)
                        .border(1.0, t.border_subtle)
                        .child(
                            Switch::new(draft.enabled)
                                .label("Ativar limpeza automática na VPS")
                                .on_toggle(|s: &mut AppState| {
                                    if let ModalState::PurgeRuleEditor(d) = &mut s.modal {
                                        d.enabled = !d.enabled;
                                    }
                                })
                                .build(k)
                                .cursor(Cursor::Pointer),
                        ),
                )
                .child(labeled(
                    t,
                    "A cada X execuções/testes (0 = desligado)",
                    runs_in,
                ))
                .child(labeled(
                    t,
                    "Quando workspaces atingirem",
                    col()
                        .gap(6.0)
                        .child(
                            row()
                                .gap(8.0)
                                .center_y()
                                .flex_wrap()
                                .child(size_in)
                                .child(unit_chips),
                        )
                        .child(
                            text(size_preview)
                                .font(ty::CAPTION)
                                .color(t.text_muted),
                        ),
                ))
                .child(labeled(t, "O que limpar automaticamente", toggles_card));

            if let Some(err) = &draft.error {
                form = form.child(error_banner(t, err));
            }

            let footer = row()
                .w_full()
                .justify_end()
                .gap(10.0)
                .child(btn_neutral(
                    t,
                    "purge-cancel",
                    "Cancelar",
                    None,
                    34.0,
                    |s: &mut AppState, _| s.modal = ModalState::None,
                ))
                .child(primary_button(
                    t,
                    "purge-save",
                    "Salvar regra",
                    Some(&icons::CHECK_CIRCLE),
                    34.0,
                    |s: &mut AppState, tx: &mut Tx| s.save_purge_rule_draft(tx),
                ));

            mount_shell_dialog(
                t,
                "purge-rule",
                "Definir regra de limpeza (Purge)",
                560.0,
                form,
                footer,
                layer,
            );
        }
    }
}

fn error_banner(t: UiTheme, msg: &str) -> El {
    row()
        .w_full()
        .min_w(0.0)
        .px(12.0)
        .py(8.0)
        .radius(8.0)
        .fill(t.danger_bg)
        .border(1.0, t.danger.alpha(0.4))
        .child(
            text(msg.to_string())
                .font(ty::SMALL)
                .color(t.danger)
                .grow(1.0)
                .shrink()
                .min_w(0.0)
                .wrap(),
        )
}

fn warning_banner(t: UiTheme, msg: &str) -> El {
    row()
        .w_full()
        .min_w(0.0)
        .px(12.0)
        .py(8.0)
        .gap(8.0)
        .center_y()
        .radius(8.0)
        .fill(t.warning_bg)
        .border(1.0, t.warning.alpha(0.45))
        .child(icon(&icons::BOLT, 14.5).color(t.warning).a11y_hidden())
        .child(
            text(msg.to_string())
                .font(ty::SMALL)
                .color(t.warning)
                .grow(1.0)
                .shrink()
                .min_w(0.0)
                .wrap(),
        )
}

fn fmt_commit_date(raw: &str) -> String {
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

fn labeled(t: UiTheme, label: &str, child: El) -> El {
    col()
        .w_full()
        .gap(5.0)
        .child(
            text(label.to_string())
                .font(ty::CAPTION)
                .color(t.text_secondary),
        )
        .child(child)
}

fn mount_shell_dialog(
    t: UiTheme,
    key: &str,
    title: &str,
    max_w: f32,
    body: El,
    footer: El,
    layer: &mut Vec<El>,
) {
    let close_btn = row()
        .key(format!("{key}-modal-close"))
        .role("button")
        .label("Fechar")
        .help("Fechar janela")
        .size(28.0, 28.0)
        .center()
        .radius(7.0)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent)
        .hover(t.hover)
        .press(t.press)
        .on_press(|s: &mut AppState| {
            s.modal = ModalState::None;
            s.open_select = None;
        })
        .child(icon(&icons::X_CIRCLE, 16.0).color(t.text_muted).a11y_hidden());

    let panel_inner = col()
        .w_full()
        .min_w(0.0)
        .max_h(720.0)
        .min_h(0.0)
        .child(
            row()
                .w_full()
                .min_w(0.0)
                .px(20.0)
                .py(14.0)
                .center_y()
                .justify_between()
                .gap(10.0)
                .child(
                    text(title.to_string())
                        .font(ty::H2)
                        .color(t.text_primary)
                        .grow(1.0)
                        .shrink()
                        .min_w(0.0)
                        .truncate(),
                )
                .child(close_btn),
        )
        .child(el().w_full().h(t.hair).fill(t.border_subtle))
        .child(
            col()
                .key(format!("{key}-modal-scroll"))
                .w_full()
                .min_w(0.0)
                .max_h(520.0)
                .min_h(0.0)
                .px(20.0)
                .py(16.0)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .scroll_edge_fade(12.0)
                .child(body),
        )
        .child(el().w_full().h(t.hair).fill(t.border_subtle))
        .child(row().w_full().min_w(0.0).px(20.0).py(14.0).child(footer));

    let panel = el()
        .key(format!("{key}-modal-dialog"))
        .role("dialog")
        .label(title.to_string())
        .w_full()
        .max_w(max_w)
        .radius(10.0)
        .fill(t.surface_1)
        .border(1.0, t.border_strong)
        .shadow(10.0, 28.0, t.shadow)
        .transition(Transition::scale(0.96))
        .motion(motion::SNAPPY)
        .modal(true, |s: &mut AppState| {
            s.modal = ModalState::None;
            s.open_select = None;
        })
        .child(panel_inner);

    layer.push(
        el().key(format!("{key}-modal-scrim"))
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .fill(t.scrim)
            .transition(Transition::opacity())
            .a11y_hidden(),
    );
    layer.push(
        el().key(format!("{key}-modal-wrap"))
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .padding(24.0)
            .center()
            .child(panel),
    );
}
