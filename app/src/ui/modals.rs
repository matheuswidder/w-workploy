use rune::tok::motion;
use rune::{Cursor, El, ScrollSpec, Transition, Tx, col, el, icon, row, text};
use rune_kit::{Kit, Switch, TextArea, TextInput};
use workploy::model::{RunnerKind, Trigger, TriggerMode};

use crate::state::{
    AppState, ModalState, PurgeSizeUnit, ServerAuthChoice, StepDraft, ToastKind,
};
use crate::ui::components::{btn_danger, btn_icon, btn_neutral, chip, pill_badge, primary_button};
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

            let name_input = TextInput::new(&draft.name)
                .label("Nome do fluxo")
                .placeholder("Ex: Deploy VPS")
                .width(None)
                .on_edit(|s: &mut AppState| match &mut s.modal {
                    ModalState::WorkflowEditor(d) => &mut d.name,
                    _ => unreachable!(),
                })
                .build(k)
                .cursor(Cursor::Text);

            let workdir_input = TextInput::new(&draft.workdir)
                .label("Pasta de trabalho")
                .placeholder("Opcional (ex: /srv/app ou C:\\Projects\\app)")
                .width(None)
                .on_edit(|s: &mut AppState| match &mut s.modal {
                    ModalState::WorkflowEditor(d) => &mut d.workdir,
                    _ => unreachable!(),
                })
                .build(k)
                .cursor(Cursor::Text);

            let triggers = row()
                .gap(6.0)
                .flex_wrap()
                .child(chip(
                    t,
                    "ed-trig-manual",
                    "Manual (botão)",
                    draft.trigger == Trigger::Manual,
                    |s: &mut AppState, _| {
                        if let ModalState::WorkflowEditor(d) = &mut s.modal {
                            d.trigger = Trigger::Manual;
                        }
                    },
                ))
                .child(chip(
                    t,
                    "ed-trig-push",
                    "push no GitHub",
                    draft.trigger == Trigger::Push,
                    |s: &mut AppState, _| {
                        if let ModalState::WorkflowEditor(d) = &mut s.modal {
                            d.trigger = Trigger::Push;
                        }
                    },
                ))
                .child(chip(
                    t,
                    "ed-trig-folder",
                    "Mudança na pasta",
                    draft.trigger == Trigger::Folder,
                    |s: &mut AppState, _| {
                        if let ModalState::WorkflowEditor(d) = &mut s.modal {
                            d.trigger = Trigger::Folder;
                        }
                    },
                ));

            let runners = row()
                .gap(6.0)
                .flex_wrap()
                .child(chip(
                    t,
                    "ed-run-local",
                    "Esta máquina",
                    draft.runner == RunnerKind::Local,
                    |s: &mut AppState, _| {
                        if let ModalState::WorkflowEditor(d) = &mut s.modal {
                            d.runner = RunnerKind::Local;
                        }
                    },
                ))
                .child(chip(
                    t,
                    "ed-run-ssh",
                    "VPS via SSH",
                    draft.runner == RunnerKind::Ssh,
                    |s: &mut AppState, _| {
                        if let ModalState::WorkflowEditor(d) = &mut s.modal {
                            d.runner = RunnerKind::Ssh;
                        }
                    },
                ));

            let mut server_row = row().gap(6.0).flex_wrap().child(chip(
                t,
                "ed-srv-default",
                "Alias global",
                draft.server_id.is_none(),
                |s: &mut AppState, _| {
                    if let ModalState::WorkflowEditor(d) = &mut s.modal {
                        d.server_id = None;
                    }
                },
            ));
            for srv in &s.snapshot.settings.servers {
                let sid = srv.id.clone();
                let active = draft.server_id.as_deref() == Some(srv.id.as_str());
                server_row = server_row.child(chip(
                    t,
                    format!("ed-srv-{}", srv.id),
                    srv.name.clone(),
                    active,
                    move |s: &mut AppState, _| {
                        if let ModalState::WorkflowEditor(d) = &mut s.modal {
                            d.server_id = Some(sid.clone());
                        }
                    },
                ));
            }

            let mut chain_row = row().gap(6.0).flex_wrap().child(chip(
                t,
                "ed-chain-none",
                "Nenhum",
                draft.on_success.is_none(),
                |s: &mut AppState, _| {
                    if let ModalState::WorkflowEditor(d) = &mut s.modal {
                        d.on_success = None;
                    }
                },
            ));
            for wf in &s.snapshot.workflows {
                if draft.id.as_deref() == Some(wf.id.as_str()) {
                    continue;
                }
                let target_id = wf.id.clone();
                let active = draft.on_success.as_deref() == Some(wf.id.as_str());
                chain_row = chain_row.child(chip(
                    t,
                    format!("ed-chain-{}", wf.id),
                    wf.name.clone(),
                    active,
                    move |s: &mut AppState, _| {
                        if let ModalState::WorkflowEditor(d) = &mut s.modal {
                            d.on_success = Some(target_id.clone());
                        }
                    },
                ));
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
                    .cursor(Cursor::Text);

                let step_cmd_in = TextInput::new(&st.run)
                    .label(format!("Comando do passo {}", idx + 1))
                    .placeholder("echo ok")
                    .width(None)
                    .on_edit(move |s: &mut AppState| match &mut s.modal {
                        ModalState::WorkflowEditor(d) => &mut d.steps[idx].run,
                        _ => unreachable!(),
                    })
                    .build(k)
                    .cursor(Cursor::Text);

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
                                    .cursor(Cursor::Pointer),
                            ),
                    );

                steps_col = steps_col.child(step_card);
            }

            let mut form = col()
                .w_full()
                .gap(12.0)
                .child(labeled(t, "Nome do fluxo", name_input))
                .child(
                    row()
                        .w_full()
                        .gap(16.0)
                        .flex_wrap()
                        .child(labeled(t, "Gatilho", triggers))
                        .child(labeled(t, "Runner de execução", runners)),
                );

            if draft.runner == RunnerKind::Ssh {
                form = form.child(labeled(t, "Servidor VPS cadastrado", server_row));
            }

            form = form
                .child(labeled(t, "Pasta de trabalho", workdir_input))
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
                        .child(btn_neutral(
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
                                    });
                                }
                            },
                        )),
                )
                .child(steps_col);

            if let Some(err) = &draft.error {
                form = form.child(error_banner(t, err));
            }

            let footer = row()
                .w_full()
                .justify_end()
                .gap(10.0)
                .child(btn_neutral(
                    t,
                    "ed-cancel",
                    "Cancelar",
                    None,
                    34.0,
                    |s: &mut AppState, _| {
                        s.modal = ModalState::None;
                    },
                ))
                .child(primary_button(
                    t,
                    "ed-save",
                    "Salvar fluxo",
                    Some(&icons::CHECK_CIRCLE),
                    34.0,
                    |s: &mut AppState, tx: &mut Tx| s.save_workflow_draft(tx),
                ));

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
                .child(chip(
                    t,
                    "srv-auth-alias",
                    "Alias ~/.ssh/config",
                    draft.auth_choice == ServerAuthChoice::SshConfigAlias,
                    |s: &mut AppState, _| {
                        if let ModalState::ServerEditor(d) = &mut s.modal {
                            d.auth_choice = ServerAuthChoice::SshConfigAlias;
                        }
                    },
                ))
                .child(chip(
                    t,
                    "srv-auth-key",
                    "Chave privada SSH",
                    draft.auth_choice == ServerAuthChoice::PrivateKey,
                    |s: &mut AppState, _| {
                        if let ModalState::ServerEditor(d) = &mut s.modal {
                            d.auth_choice = ServerAuthChoice::PrivateKey;
                        }
                    },
                ))
                .child(chip(
                    t,
                    "srv-auth-pass",
                    "Senha no Cofre do Windows",
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
                                        Some(&icons::SHIELD),
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

            let mut branch_chips = row()
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
            if branches_match {
                for (idx, br) in s.snapshot.repo_branches.iter().enumerate() {
                    let active = selected_branches.iter().any(|x| x == br);
                    let br_name = br.clone();
                    branch_chips = branch_chips.child(
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
            }

            let branches_loading = s.snapshot.repo_branches_loading;
            let mut branch_section = col()
                .w_full()
                .gap(8.0)
                .child(
                    row()
                        .w_full()
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
                .child(branch_chips);

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
                chip(
                    t,
                    "bind-srv-local",
                    "Local (Esta máquina)",
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
                    chip(
                        t,
                        format!("bind-srv-{}", srv.id),
                        srv.name.clone(),
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

            let mode_chips = row()
                .gap(6.0)
                .flex_wrap()
                .child(
                    chip(
                        t,
                        "bind-mode-webhook",
                        "Webhook 24/7 na VPS",
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
                    chip(
                        t,
                        "bind-mode-pollvps",
                        "Sondagem 24/7 na VPS",
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
                    chip(
                        t,
                        "bind-mode-polldesktop",
                        "Sondagem no Desktop",
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

            let mut wf_section = col()
                .w_full()
                .gap(8.0)
                .child(
                    row()
                        .w_full()
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
                        ),
                );

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
                .gap(12.0)
                .child(labeled(t, "Repositório GitHub (dono/nome)", repo_in))
                .child(labeled(t, "Branch monitorada", branch_section))
                .child(labeled(t, "Servidor de destino", target_chips))
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
        ModalState::ProvisionModal(draft) => {
            let is_running =
                s.snapshot.provisioning_server_id.as_deref() == Some(draft.server_id.as_str());

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
                .gap(12.0)
                .child(
                    text("Selecione os componentes para instalar ou atualizar via SSH na sua VPS Linux.")
                        .font(ty::SMALL)
                        .color(t.text_muted)
                        .wrap(),
                )
                .child(checklist)
                .child(labeled(t, "Porta HTTP do agente workploy-runner", port_in));

            if is_running || !s.snapshot.provision_log.trim().is_empty() {
                let tail: String = {
                    let lines: Vec<&str> = s.snapshot.provision_log.lines().collect();
                    let start = lines.len().saturating_sub(18);
                    lines[start..].join("\n")
                };
                form = form.child(
                    col()
                        .w_full()
                        .gap(6.0)
                        .child(
                            text(if is_running {
                                "Log ao vivo da instalação"
                            } else {
                                "Saída do provisionamento"
                            })
                            .font(ty::CAPTION)
                            .color(t.text_secondary),
                        )
                        .child(
                            el().w_full()
                                .padding(10.0)
                                .radius(8.0)
                                .fill(t.surface_code)
                                .border(1.0, t.border_subtle)
                                .child(
                                    text(if tail.is_empty() {
                                        "Conectando à VPS...".to_string()
                                    } else {
                                        tail
                                    })
                                    .font(ty::MONO_SM)
                                    .color(rune::color::hex("#e2e8f0").unwrap_or(t.text_primary))
                                    .wrap(),
                                ),
                        ),
                );
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
                    "prov-close",
                    "Fechar",
                    None,
                    34.0,
                    |s: &mut AppState, _| s.modal = ModalState::None,
                ))
                .child(primary_button(
                    t,
                    "prov-start",
                    if is_running {
                        "Provisionando..."
                    } else {
                        "Iniciar provisionamento"
                    },
                    Some(&icons::BOLT),
                    34.0,
                    move |s: &mut AppState, tx: &mut Tx| {
                        if !is_running {
                            s.start_provision(tx);
                        }
                    },
                ));

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
        .px(12.0)
        .py(8.0)
        .radius(8.0)
        .fill(t.danger_bg)
        .border(1.0, t.danger.alpha(0.4))
        .child(
            text(msg.to_string())
                .font(ty::SMALL)
                .color(t.danger)
                .wrap(),
        )
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
        .max_h(720.0)
        .min_h(0.0)
        .child(
            row()
                .w_full()
                .px(20.0)
                .py(14.0)
                .center_y()
                .justify_between()
                .child(text(title.to_string()).font(ty::H2).color(t.text_primary))
                .child(close_btn),
        )
        .child(el().w_full().h(t.hair).fill(t.border_subtle))
        .child(
            col()
                .key(format!("{key}-modal-scroll"))
                .w_full()
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
        .child(row().w_full().px(20.0).py(14.0).child(footer));

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
