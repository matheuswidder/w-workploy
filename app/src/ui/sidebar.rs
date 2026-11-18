use rune::color::Color;
use rune::tok::motion;
use rune::{Cursor, El, Icon, TextStyle, col, el, icon, row, text};

use crate::state::{AppState, NavTarget};
use crate::ui::components::health_pulse_dot;
use crate::ui::icons;
use crate::ui::theme::{UiTheme, ty};

const EXPANDED_W: f32 = 248.0;
const COLLAPSED_W: f32 = 68.0;
const PILL_INSET: f32 = 10.0;
const ROW_H: f32 = 34.0;
const ROW_PITCH: f32 = 38.0;
const SECTION_H: f32 = 24.0;

pub fn view(s: &AppState, t: UiTheme) -> El {
    let w = if s.sidebar_collapsed { COLLAPSED_W } else { EXPANDED_W };
    let pill_w = w - 2.0 * PILL_INSET;
    let collapsed = s.sidebar_collapsed;

    let selected_id: String = match &s.nav {
        NavTarget::Onboarding => "nav-onboarding".into(),
        NavTarget::Runs => "nav-runs".into(),
        NavTarget::WorkflowRuns(id) => format!("wf-{id}"),
        NavTarget::Approvals => "nav-approvals".into(),
        NavTarget::Connections => "nav-connections".into(),
        NavTarget::Settings => "nav-settings".into(),
        NavTarget::RunDetail { .. } => match &s.previous_list_nav {
            NavTarget::Onboarding => "nav-onboarding".into(),
            NavTarget::WorkflowRuns(id) => format!("wf-{id}"),
            NavTarget::Approvals => "nav-approvals".into(),
            NavTarget::Connections => "nav-connections".into(),
            NavTarget::Settings => "nav-settings".into(),
            _ => "nav-runs".into(),
        },
    };

    let mut y = 4.0f32;
    let mut sel_y = y;
    let mut items: Vec<El> = Vec::new();

    // 1. Início / Onboarding (apenas na 1ª vez ou quando reaberto manualmente)
    let show_onboarding_nav = !s.snapshot.settings.onboarding_completed
        || matches!(s.nav, NavTarget::Onboarding)
        || (matches!(s.nav, NavTarget::RunDetail { .. })
            && matches!(s.previous_list_nav, NavTarget::Onboarding));
    if show_onboarding_nav {
        let is_onb = selected_id == "nav-onboarding";
        if is_onb {
            sel_y = y;
        }
        items.push(nav_row(
            t,
            "nav-onboarding",
            "Guia inicial",
            &icons::HOME,
            None,
            is_onb,
            collapsed,
            PILL_INSET,
            y,
            pill_w,
            |s: &mut AppState| s.navigate_to(NavTarget::Onboarding),
        ));
        y += ROW_PITCH;
    }

    // 2. Execuções
    let is_runs = selected_id == "nav-runs";
    if is_runs {
        sel_y = y;
    }
    items.push(nav_row(
        t,
        "nav-runs",
        "Execuções",
        &icons::LIST,
        None,
        is_runs,
        collapsed,
        PILL_INSET,
        y,
        pill_w,
        |s: &mut AppState| s.navigate_to(NavTarget::Runs),
    ));
    y += ROW_PITCH + 10.0;

    // 3. Fluxos de trabalho
    if !collapsed {
        items.push(
            row()
                .key("hdr-wfs")
                .absolute(0.0, y)
                .size(w, SECTION_H)
                .px(18.0)
                .center_y()
                .justify_between()
                .child(
                    text("FLUXOS DE TRABALHO")
                        .font(ty::EYEBROW)
                        .color(t.text_muted),
                ),
        );
        y += SECTION_H + 2.0;
    }

    for wf in &s.snapshot.workflows {
        let key = format!("wf-{}", wf.id);
        let is_sel = selected_id == key;
        if is_sel {
            sel_y = y;
        }
        let wf_id = wf.id.clone();
        let count = s
            .snapshot
            .runs
            .iter()
            .filter(|r| r.workflow_id == wf.id)
            .count();
        let badge = (!collapsed && count > 0).then(|| {
            text(count.to_string())
                .font(ty::CAPTION)
                .color(if is_sel { t.text_primary } else { t.text_muted })
        });
        items.push(nav_row(
            t,
            key,
            wf.name.clone(),
            &icons::PLAY,
            badge,
            is_sel,
            collapsed,
            PILL_INSET,
            y,
            pill_w,
            move |s: &mut AppState| s.navigate_to(NavTarget::WorkflowRuns(wf_id.clone())),
        ));
        y += ROW_PITCH;
    }

    // "+ Novo fluxo"
    let new_btn = if collapsed {
        row()
            .key("nav-new-wf")
            .role("button")
            .label("Novo fluxo")
            .help("Criar novo fluxo")
            .absolute(PILL_INSET, y + 2.0)
            .size(pill_w, ROW_H)
            .center()
            .radius(6.0)
            .border(1.0, t.border_subtle)
            .cursor(Cursor::Pointer)
            .focusable(true)
            .focus_ring(2.0, 2.0, t.accent)
            .hover(t.hover)
            .press(t.press)
            .press_scale(0.97)
            .on_press(|s: &mut AppState| s.open_new_workflow_modal())
            .child(icon(&icons::PLUS, 14.0).color(t.text_secondary).a11y_hidden())
    } else {
        row()
            .key("nav-new-wf")
            .role("button")
            .label("Novo fluxo")
            .absolute(PILL_INSET, y + 2.0)
            .size(pill_w, ROW_H)
            .px(10.0)
            .gap(8.0)
            .center_y()
            .radius(6.0)
            .border(1.0, t.border_subtle)
            .cursor(Cursor::Pointer)
            .focusable(true)
            .focus_ring(2.0, 2.0, t.accent)
            .hover(t.hover)
            .press(t.press)
            .press_scale(0.98)
            .on_press(|s: &mut AppState| s.open_new_workflow_modal())
            .child(icon(&icons::PLUS, 13.0).color(t.text_secondary).a11y_hidden())
            .child(
                text("Novo fluxo")
                    .font(ty::SMALL_MED)
                    .color(t.text_secondary)
                    .a11y_hidden(),
            )
    };
    items.push(new_btn);
    y += ROW_PITCH + 14.0;

    // 4. Gerenciamento
    if !collapsed {
        items.push(
            row()
                .key("hdr-mgmt")
                .absolute(0.0, y)
                .size(w, SECTION_H)
                .px(18.0)
                .center_y()
                .child(
                    text("GERENCIAMENTO")
                        .font(ty::EYEBROW)
                        .color(t.text_muted),
                ),
        );
        y += SECTION_H + 2.0;
    }

    let pending = s.pending_approvals();
    let is_appr = selected_id == "nav-approvals";
    if is_appr {
        sel_y = y;
    }
    let appr_badge = (pending > 0 && !collapsed).then(|| {
        row()
            .h(20.0)
            .min_w(22.0)
            .px(7.0)
            .center()
            .radius(10.0)
            .fill(t.warning_bg)
            .border(1.0, t.warning.alpha(0.45))
            .child(
                text(pending.to_string())
                    .font(ty::CAPTION)
                    .color(t.warning),
            )
    });
    items.push(nav_row(
        t,
        "nav-approvals",
        "Aprovações",
        &icons::SHIELD,
        appr_badge,
        is_appr,
        collapsed,
        PILL_INSET,
        y,
        pill_w,
        |s: &mut AppState| s.navigate_to(NavTarget::Approvals),
    ));
    y += ROW_PITCH;

    let is_conn = selected_id == "nav-connections";
    if is_conn {
        sel_y = y;
    }
    let conn_dot = (!collapsed)
        .then(|| s.sidebar_connections_indicator())
        .flatten()
        .map(|health| health_pulse_dot(t, health));
    items.push(nav_row(
        t,
        "nav-connections",
        "Conexões & Diagnóstico",
        &icons::ACTIVITY,
        conn_dot,
        is_conn,
        collapsed,
        PILL_INSET,
        y,
        pill_w,
        |s: &mut AppState| s.navigate_to(NavTarget::Connections),
    ));
    y += ROW_PITCH;

    let is_settings = selected_id == "nav-settings";
    if is_settings {
        sel_y = y;
    }
    items.push(nav_row(
        t,
        "nav-settings",
        "Configurações",
        &icons::SLIDERS,
        None,
        is_settings,
        collapsed,
        PILL_INSET,
        y,
        pill_w,
        |s: &mut AppState| s.navigate_to(NavTarget::Settings),
    ));
    y += ROW_PITCH;

    let nav_list = el()
        .key("sidebar-nav-list")
        .w_full()
        .h(y + 4.0)
        .child(
            el().key("sidebar-selection")
                .absolute(PILL_INSET, sel_y)
                .size(pill_w, ROW_H)
                .radius(6.0)
                .fill(t.surface_3)
                .border(1.0, t.border_subtle)
                .lead(motion::LEAD, motion::DEFAULT),
        )
        .children(items);

    let header = if collapsed {
        row()
            .key("sidebar-header")
            .w_full()
            .h(56.0)
            .center()
            .child(
                row()
                    .key("sidebar-expand")
                    .role("button")
                    .label("Expandir menu lateral")
                    .help("Expandir barra lateral")
                    .size(34.0, 34.0)
                    .center()
                    .radius(6.0)
                    .fill(t.surface_2)
                    .cursor(Cursor::Pointer)
                    .focusable(true)
                    .focus_ring(2.0, 2.0, t.accent)
                    .hover(t.hover)
                    .press(t.press)
                    .press_scale(0.95)
                    .on_press(|s: &mut AppState| s.sidebar_collapsed = false)
                    .child(icon(&icons::SIDEBAR_TOGGLE, 15.0).color(t.text_secondary).a11y_hidden()),
            )
    } else {
        row()
            .key("sidebar-header")
            .w_full()
            .h(56.0)
            .px(14.0)
            .center_y()
            .justify_between()
            .child(
                row()
                    .gap(9.0)
                    .center_y()
                    .child(
                        el().size(26.0, 26.0)
                            .radius(6.0)
                            .fill(t.primary_bg)
                            .center()
                            .child(icon(&icons::BOLT, 14.0).color(t.primary_fg)),
                    )
                    .child(
                        col()
                            .child(
                                text("Workploy")
                                    .font(TextStyle::new(14.0, 600.0).lh(17.0))
                                    .color(t.text_primary),
                            )
                            .child(
                                text("Runner local e VPS")
                                    .font(TextStyle::new(10.5, 500.0).lh(13.0))
                                    .color(t.text_muted),
                            ),
                    ),
            )
            .child(
                row()
                    .key("sidebar-collapse")
                    .role("button")
                    .label("Recolher menu lateral")
                    .help("Recolher barra lateral")
                    .size(28.0, 28.0)
                    .center()
                    .radius(6.0)
                    .cursor(Cursor::Pointer)
                    .focusable(true)
                    .focus_ring(2.0, 2.0, t.accent)
                    .hover(t.hover)
                    .press(t.press)
                    .press_scale(0.95)
                    .on_press(|s: &mut AppState| s.sidebar_collapsed = true)
                    .child(
                        icon(&icons::SIDEBAR_TOGGLE, 14.0)
                            .color(t.text_muted)
                            .a11y_hidden(),
                    ),
            )
    };

    let footer = if collapsed {
        col()
            .key("sidebar-footer")
            .w_full()
            .px(10.0)
            .pb(14.0)
            .gap(8.0)
            .items_center()
            .child(
                row()
                    .key("theme-toggle-mini")
                    .role("button")
                    .label("Alternar tema")
                    .help("Alternar tema claro/escuro")
                    .size(34.0, 34.0)
                    .center()
                    .radius(6.0)
                    .fill(t.surface_2)
                    .cursor(Cursor::Pointer)
                    .focusable(true)
                    .focus_ring(2.0, 2.0, t.accent)
                    .hover(t.hover)
                    .press(t.press)
                    .on_press_tx(|s: &mut AppState, tx| {
                        s.dark = !s.dark;
                        tx.color_motion(motion::CALM);
                    })
                    .child(icon(&icons::SUN_MOON, 14.0).color(t.text_secondary).a11y_hidden()),
            )
    } else {
        col()
            .key("sidebar-footer")
            .w_full()
            .px(12.0)
            .pb(14.0)
            .gap(10.0)
            .child(
                col()
                    .key("self-hosted-badge")
                    .w_full()
                    .px(11.0)
                    .py(9.0)
                    .gap(2.0)
                    .radius(8.0)
                    .fill(t.surface_1)
                    .border(1.0, t.border_subtle)
                    .child(
                        row()
                            .gap(6.0)
                            .center_y()
                            .child(icon(&icons::SHIELD, 13.0).color(t.success))
                            .child(
                                text("Self-hosted · Ilimitado")
                                    .font(TextStyle::new(12.0, 600.0).lh(16.0))
                                    .color(t.text_primary),
                            ),
                    )
                    .child(
                        text("Minutos ilimitados · Sem fila externa")
                            .font(TextStyle::new(10.5, 500.0).lh(14.0))
                            .color(t.text_muted),
                    ),
            )
            .child(
                row()
                    .w_full()
                    .gap(6.0)
                    .child(
                        row()
                            .key("theme-toggle")
                            .role("button")
                            .label(if s.dark { "Tema escuro" } else { "Tema claro" })
                            .help("Alternar entre tema escuro e claro")
                            .grow(1.0)
                            .h(30.0)
                            .px(10.0)
                            .gap(6.0)
                            .center_y()
                            .radius(6.0)
                            .fill(t.surface_1)
                            .border(1.0, t.border_subtle)
                            .cursor(Cursor::Pointer)
                            .focusable(true)
                            .focus_ring(2.0, 2.0, t.accent)
                            .hover(t.hover)
                            .press(t.press)
                            .press_scale(0.98)
                            .on_press_tx(|s: &mut AppState, tx| {
                                s.dark = !s.dark;
                                tx.color_motion(motion::CALM);
                            })
                            .child(icon(&icons::SUN_MOON, 13.0).color(t.text_secondary).a11y_hidden())
                            .child(
                                text(if s.dark { "Escuro" } else { "Claro" })
                                    .font(ty::CAPTION)
                                    .color(t.text_secondary)
                                    .a11y_hidden(),
                            ),
                    )
                    .child(
                        row()
                            .key("motion-quick-toggle")
                            .role("button")
                            .label(if s.reduce_motion {
                                "Animações desligadas (0x)"
                            } else {
                                "Animações ligadas (1x)"
                            })
                            .help("Alternar animações entre 1x (ligado) e 0x (desligado)")
                            .h(30.0)
                            .px(9.0)
                            .gap(5.0)
                            .center_y()
                            .radius(6.0)
                            .fill(if s.reduce_motion { t.accent_soft } else { t.surface_1 })
                            .border(
                                1.0,
                                if s.reduce_motion {
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
                            .on_press_tx(|s: &mut AppState, tx: &mut rune::Tx| {
                                let enable = s.reduce_motion;
                                s.set_animations_enabled(enable, tx.clock() as f32);
                            })
                            .child(
                                icon(&icons::SLIDERS, 12.5)
                                    .color(if s.reduce_motion { t.accent } else { t.text_muted })
                                    .a11y_hidden(),
                            )
                            .child(
                                text(if s.reduce_motion { "0x" } else { "1x" })
                                    .font(ty::CAPTION)
                                    .color(if s.reduce_motion { t.accent } else { t.text_muted })
                                    .a11y_hidden(),
                            ),
                    ),
            )
    };

    row()
        .key("sidebar-wrap")
        .h_full()
        .motion(motion::DEFAULT)
        .child(
            col()
                .key("sidebar")
                .w(w)
                .h_full()
                .min_h(0.0)
                .fill(t.bg_shell)
                .child(header)
                .child(el().w_full().h(t.hair).fill(t.border_subtle))
                .child(
                    col()
                        .key("sidebar-scroll")
                        .grow(1.0)
                        .min_h(0.0)
                        .w_full()
                        .scroll(rune::ScrollSpec { stick_bottom: false })
                        .pt(8.0)
                        .child(nav_list),
                )
                .child(footer),
        )
        .child(el().key("sidebar-border").w(t.hair).h_full().fill(t.border_subtle))
}

#[allow(clippy::too_many_arguments)]
fn nav_row(
    t: UiTheme,
    key: impl Into<String>,
    label: impl Into<String>,
    ic: &'static Icon,
    trailing: Option<El>,
    selected: bool,
    collapsed: bool,
    x: f32,
    y: f32,
    w: f32,
    on_press: impl Fn(&mut AppState) + 'static,
) -> El {
    let label = label.into();
    let ic_col = if selected { t.text_primary } else { t.text_muted };
    let fg_col = if selected { t.text_primary } else { t.text_secondary };
    let mut r = row()
        .key(key)
        .role("tab")
        .label(label.clone())
        .checked(selected)
        .absolute(x, y)
        .size(w, ROW_H)
        .radius(6.0)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent)
        .hover(if selected { Color::TRANSPARENT } else { t.hover })
        .press(t.press)
        .press_scale(0.98)
        .on_press(on_press);
    if collapsed {
        r = r
            .help(label)
            .center()
            .child(icon(ic, 15.0).color(ic_col).a11y_hidden());
    } else {
        r = r
            .px(10.0)
            .gap(9.0)
            .center_y()
            .child(icon(ic, 14.0).color(ic_col).a11y_hidden())
            .child(
                text(label)
                    .font(if selected { ty::BODY_MED } else { ty::BODY })
                    .color(fg_col)
                    .grow(1.0)
                    .shrink()
                    .truncate()
                    .a11y_hidden(),
            );
        if let Some(tr) = trailing {
            r = r.child(tr);
        }
    }
    r
}
