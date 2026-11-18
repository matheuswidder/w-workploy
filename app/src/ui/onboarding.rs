use rune::{El, ScrollSpec, Transition, Tx, col, el, icon, row, text};

use crate::state::{AppState, NavTarget, SettingsTab};
use crate::ui::components::{btn_neutral, header_card, pill_badge, primary_button, surface_card};
use crate::ui::icons;
use crate::ui::theme::{UiTheme, ty};

pub fn view(s: &AppState, t: UiTheme) -> El {
    let primary_wf_id = s.snapshot.workflows.first().map(|w| w.id.clone());
    let has_github = s.snapshot.settings.has_github_token;
    let vps_count = s.snapshot.settings.servers.len();
    let binding_count = s.snapshot.settings.repo_bindings.len();
    let wf_count = s.snapshot.workflows.len();

    let hdr = header_card(
        t,
        "onboarding-hdr",
        10.0,
        row()
            .px(20.0)
            .py(16.0)
            .gap(16.0)
            .center_y()
            .justify_between()
            .flex_wrap()
            .child(
                col()
                    .grow(1.0)
                    .shrink()
                    .min_w(200.0)
                    .gap(4.0)
                    .child(
                        text("Início · Workploy")
                            .font(ty::H1)
                            .color(t.text_primary),
                    )
                    .child(
                        text("Orquestrador desktop self-hosted para workflows de CI/CD, análise de código e deploy contínuo em múltiplas VPS.")
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
                    .child(btn_neutral(
                        t,
                        "onb-hdr-runs",
                        "Execuções",
                        Some(&icons::LIST),
                        34.0,
                        |s: &mut AppState, _| s.navigate_to(NavTarget::Runs),
                    ))
                    .child(primary_button(
                        t,
                        "onb-hdr-run",
                        "Executar fluxo",
                        Some(&icons::PLAY),
                        34.0,
                        move |s: &mut AppState, tx: &mut Tx| {
                            if let Some(id) = &primary_wf_id {
                                s.start_workflow(id, "manual", tx);
                            } else {
                                s.open_new_workflow_modal();
                            }
                        },
                    )),
            ),
    );

    let overview_card = surface_card(
        t,
        "onb-overview-card",
        12.0,
        col()
            .px(20.0)
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
                        text("O que é o Workploy e como o pipeline funciona")
                            .font(ty::H2)
                            .color(t.text_primary),
                    )
                    .child(
                        row()
                            .gap(6.0)
                            .flex_wrap()
                            .child(pill_badge(
                                t,
                                if has_github {
                                    "GitHub conectado"
                                } else {
                                    "GitHub pendente"
                                },
                                if has_github { t.success } else { t.text_muted },
                                if has_github { t.success_bg } else { t.surface_2 },
                            ))
                            .child(pill_badge(
                                t,
                                &format!("{vps_count} VPS · {binding_count} vínculos"),
                                t.accent,
                                t.accent_soft,
                            )),
                    ),
            )
            .child(
                text("Cada repositório pode ser vinculado à sua máquina local ou a uma VPS Linux dedicada com o agente workploy-runner 24/7, mantendo fluxos encadeados com segurança.")
                    .font(ty::SMALL)
                    .color(t.text_secondary)
                    .wrap(),
            )
            .child(
                row()
                    .w_full()
                    .gap(10.0)
                    .flex_wrap()
                    .child(pipeline_stage_card(
                        t,
                        "onb-stage-ci",
                        "1. CI & Análise",
                        "Gatilho manual, pasta local, sondagem ou webhook 24/7 executa build, testes, contratos e varredura de segredos.",
                        &icons::CODE,
                        t.info,
                        t.info_bg,
                    ))
                    .child(pipeline_stage_card(
                        t,
                        "onb-stage-approval",
                        "2. Portão de Aprovação",
                        "Passos críticos pausam o fluxo e aguardam sua liberação explícita na tela de Aprovações antes de tocar produção.",
                        &icons::SHIELD,
                        t.warning,
                        t.warning_bg,
                    ))
                    .child(pipeline_stage_card(
                        t,
                        "onb-stage-redeploy",
                        "3. Redeploy Multi-VPS",
                        "Ao concluir o CI ou aprovar o portão, o encadeamento dispara o redeploy na VPS destino de cada repositório.",
                        &icons::SERVER,
                        t.success,
                        t.success_bg,
                    )),
            ),
    );

    let step1 = onboarding_step_row(
        t,
        "onb-step-1",
        "1",
        "Conectar conta GitHub e vincular repositórios",
        "Salve seu Token Pessoal (PAT) com segurança no Cofre do Windows, pesquise repositórios e defina o destino e modo de disparo de cada um.",
        has_github && binding_count > 0,
        row().child(
            btn_neutral(
                t,
                "onb-btn-github",
                "Configurações → GitHub",
                Some(&icons::BRANCH),
                32.0,
                |s: &mut AppState, _| s.open_settings(SettingsTab::GitHub),
            )
            .help("Abrir aba GitHub para informar o token PAT e vincular repositórios."),
        ),
    );

    let step2 = onboarding_step_row(
        t,
        "onb-step-2",
        "2",
        "Cadastrar VPS Linux e provisionar workploy-runner",
        "Configure servidores via SSH, instale Docker/Node/Rust e mantenha o serviço workploy-runner ativo 24/7 com limpeza automática de disco.",
        vps_count > 0,
        row().child(
            btn_neutral(
                t,
                "onb-btn-vps",
                "Configurações → VPS",
                Some(&icons::SERVER),
                32.0,
                |s: &mut AppState, _| s.open_settings(SettingsTab::Vps),
            )
            .help("Abrir aba VPS & Motor para cadastrar servidores e provisionar o runner."),
        ),
    );

    let step3 = onboarding_step_row(
        t,
        "onb-step-3",
        "3",
        "Validar conexões, hardware e logs de diagnóstico",
        "Teste SSH e GitHub em segundos, confira vCPU, RAM, disco livre e ferramentas instaladas ou acione a limpeza manual da VPS.",
        s.snapshot
            .settings
            .servers
            .iter()
            .any(|srv| srv.last_probe.as_ref().is_some_and(|p| p.ok)),
        row().child(
            btn_neutral(
                t,
                "onb-btn-connections",
                "Conexões & Diagnóstico",
                Some(&icons::ACTIVITY),
                32.0,
                |s: &mut AppState, _| s.navigate_to(NavTarget::Connections),
            )
            .help("Abrir painel de diagnóstico para testar conexões SSH/GitHub e ver capacidade das VPSs."),
        ),
    );

    let primary_wf_id_2 = s.snapshot.workflows.first().map(|w| w.id.clone());
    let step4 = onboarding_step_row(
        t,
        "onb-step-4",
        "4",
        "Executar workflows e acompanhar resultados",
        &format!(
            "Você já tem {wf_count} fluxo(s) cadastrado(s). Rode agora ou crie um novo fluxo com passos e portões de aprovação."
        ),
        !s.snapshot.runs.is_empty(),
        row()
            .gap(8.0)
            .center_y()
            .flex_wrap()
            .child(
                btn_neutral(
                    t,
                    "onb-btn-runs",
                    "Execuções",
                    Some(&icons::LIST),
                    32.0,
                    |s: &mut AppState, _| s.navigate_to(NavTarget::Runs),
                )
                .help("Ver histórico completo, filtros e logs de execuções."),
            )
            .child(
                primary_button(
                    t,
                    "onb-btn-run",
                    "Executar fluxo",
                    Some(&icons::PLAY),
                    32.0,
                    move |s: &mut AppState, tx: &mut Tx| {
                        if let Some(id) = &primary_wf_id_2 {
                            s.start_workflow(id, "manual", tx);
                        } else {
                            s.open_new_workflow_modal();
                        }
                    },
                )
                .help("Iniciar uma execução agora."),
            )
            .child(
                btn_neutral(
                    t,
                    "onb-btn-new",
                    "Novo fluxo",
                    Some(&icons::PLUS),
                    32.0,
                    |s: &mut AppState, _| s.open_new_workflow_modal(),
                )
                .help("Criar um novo fluxo de trabalho do zero."),
            ),
    );

    let checklist_card = surface_card(
        t,
        "onboarding-card",
        12.0,
        col()
            .px(20.0)
            .py(18.0)
            .gap(12.0)
            .child(
                col()
                    .gap(4.0)
                    .child(
                        text("Primeiras configurações guiadas")
                            .font(ty::H2)
                            .color(t.text_primary),
                    )
                    .child(
                        text("Siga os atalhos abaixo na ordem que preferir para deixar seu ambiente local e remoto pronto.")
                            .font(ty::SMALL)
                            .color(t.text_muted)
                            .wrap(),
                    ),
            )
            .child(step1)
            .child(step2)
            .child(step3)
            .child(step4),
    );

    col()
        .key("onboarding-page")
        .w_full()
        .h_full()
        .min_h(0.0)
        .px(24.0)
        .pt(20.0)
        .pb(16.0)
        .gap(14.0)
        .transition(Transition::opacity())
        .child(hdr)
        .child(
            col()
                .key("onboarding-scroll")
                .grow(1.0)
                .min_h(0.0)
                .w_full()
                .scroll(ScrollSpec { stick_bottom: false })
                .scroll_edge_fade(16.0)
                .gap(14.0)
                .pb(16.0)
                .child(overview_card)
                .child(checklist_card),
        )
}

fn pipeline_stage_card(
    t: UiTheme,
    key: &str,
    title: &str,
    desc: &str,
    ic: &'static rune::Icon,
    fg: rune::color::Color,
    bg: rune::color::Color,
) -> El {
    col()
        .key(key)
        .grow(1.0)
        .shrink()
        .min_w(210.0)
        .px(14.0)
        .py(12.0)
        .gap(6.0)
        .radius(10.0)
        .fill(t.surface_2)
        .border(1.0, t.border_subtle)
        .child(
            row()
                .gap(8.0)
                .center_y()
                .child(
                    el().size(24.0, 24.0)
                        .radius(6.0)
                        .fill(bg)
                        .center()
                        .child(icon(ic, 13.0).color(fg)),
                )
                .child(text(title.to_string()).font(ty::BODY_MED).color(t.text_primary)),
        )
        .child(
            text(desc.to_string())
                .font(ty::CAPTION)
                .color(t.text_muted)
                .wrap(),
        )
}

fn onboarding_step_row(
    t: UiTheme,
    key: &str,
    num: &str,
    title: &str,
    desc: &str,
    done: bool,
    actions: El,
) -> El {
    row()
        .key(key)
        .w_full()
        .px(16.0)
        .py(14.0)
        .gap(14.0)
        .center_y()
        .flex_wrap()
        .radius(10.0)
        .fill(t.surface_2)
        .border(
            1.0,
            if done {
                t.success.alpha(0.32)
            } else {
                t.border_subtle
            },
        )
        .child(
            el().size(30.0, 30.0)
                .radius(15.0)
                .fill(if done { t.success_bg } else { t.accent_soft })
                .border(
                    1.0,
                    if done {
                        t.success.alpha(0.45)
                    } else {
                        t.accent.alpha(0.4)
                    },
                )
                .center()
                .child(if done {
                    icon(&icons::CHECK_CIRCLE, 15.0).color(t.success)
                } else {
                    text(num.to_string()).font(ty::BODY_MED).color(t.accent)
                }),
        )
        .child(
            col()
                .grow(1.0)
                .shrink()
                .min_w(180.0)
                .gap(3.0)
                .child(text(title.to_string()).font(ty::H3).color(t.text_primary))
                .child(
                    text(desc.to_string())
                        .font(ty::SMALL)
                        .color(t.text_secondary)
                        .wrap(),
                ),
        )
        .child(actions)
}
