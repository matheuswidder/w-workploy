use rune::color::Color;
use rune::tok::motion;
use rune::{Cursor, El, Icon, TextStyle, Transition, Tx, col, effect, el, icon, row, text};
use rune_kit::{Kit, Toast, ToastEvent, ToastRegion};
use workploy::model::{RunStatus, StepStatus};

use crate::gfx::{MARK_PULSE_BREATHE, MARK_PULSE_RADAR};
use crate::state::{AppState, ConnectionsHealth, ToastKind};
use crate::ui::icons;
use crate::ui::theme::{UiTheme, ty};

pub fn primary_button<S: 'static>(
    t: UiTheme,
    key: impl Into<String>,
    label: impl Into<String>,
    ic: Option<&'static Icon>,
    h: f32,
    on_press: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let label = label.into();
    let r: f32 = 8.0;
    let mut btn = row()
        .key(key)
        .role("button")
        .label(label.clone())
        .h(h)
        .px(14.0)
        .gap(7.0)
        .center()
        .radius(r)
        .fill(t.primary_bg)
        .border(1.0, t.border_strong)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent)
        .hover(Color::WHITE.alpha(0.12))
        .press(Color::BLACK.alpha(0.14))
        .press_scale(0.98)
        .motion(motion::SNAPPY)
        .on_press_tx(on_press);
    if let Some(i) = ic {
        btn = btn.child(icon(i, 14.0).color(t.primary_fg).a11y_hidden());
    }
    btn.child(
        text(label)
            .font(TextStyle::new(13.0, 600.0).lh(18.0))
            .color(t.primary_fg)
            .a11y_hidden(),
    )
}

pub fn btn_neutral<S: 'static>(
    t: UiTheme,
    key: impl Into<String>,
    label: impl Into<String>,
    ic: Option<&'static Icon>,
    h: f32,
    on_press: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let label = label.into();
    let mut r = row()
        .key(key)
        .role("button")
        .label(label.clone())
        .h(h)
        .px(12.0)
        .gap(6.0)
        .center()
        .radius(8.0)
        .fill(t.surface_2)
        .border(1.0, t.border_strong)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent)
        .hover(t.hover)
        .press(t.press)
        .press_scale(0.98)
        .motion(motion::SNAPPY)
        .on_press_tx(on_press);
    if let Some(i) = ic {
        r = r.child(icon(i, 14.0).color(t.text_secondary).a11y_hidden());
    }
    r.child(text(label).font(ty::BODY_MED).color(t.text_primary).a11y_hidden())
}

pub fn btn_ghost<S: 'static>(
    t: UiTheme,
    key: impl Into<String>,
    label: impl Into<String>,
    ic: Option<&'static Icon>,
    h: f32,
    on_press: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let label = label.into();
    let mut r = row()
        .key(key)
        .role("button")
        .label(label.clone())
        .h(h)
        .px(10.0)
        .gap(6.0)
        .center()
        .radius(6.0)
        .fill(Color::TRANSPARENT)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent)
        .hover(t.hover)
        .press(t.press)
        .press_scale(0.98)
        .motion(motion::SNAPPY)
        .on_press_tx(on_press);
    if let Some(i) = ic {
        r = r.child(icon(i, 14.0).color(t.text_secondary).a11y_hidden());
    }
    r.child(text(label).font(ty::SMALL_MED).color(t.text_secondary).a11y_hidden())
}

pub fn btn_danger<S: 'static>(
    t: UiTheme,
    key: impl Into<String>,
    label: impl Into<String>,
    ic: Option<&'static Icon>,
    h: f32,
    on_press: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let label = label.into();
    let mut r = row()
        .key(key)
        .role("button")
        .label(label.clone())
        .h(h)
        .px(12.0)
        .gap(6.0)
        .center()
        .radius(8.0)
        .fill(t.danger_bg)
        .border(1.0, t.danger.alpha(0.38))
        .cursor(Cursor::Pointer)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.danger)
        .hover(t.danger.alpha(0.20))
        .press(t.danger.alpha(0.28))
        .press_scale(0.98)
        .motion(motion::SNAPPY)
        .on_press_tx(on_press);
    if let Some(i) = ic {
        r = r.child(icon(i, 14.0).color(t.danger).a11y_hidden());
    }
    r.child(text(label).font(ty::BODY_MED).color(t.danger).a11y_hidden())
}

pub fn btn_icon<S: 'static>(
    t: UiTheme,
    key: impl Into<String>,
    help_text: impl Into<String>,
    ic: &'static Icon,
    side: f32,
    on_press: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let tip = help_text.into();
    row()
        .key(key)
        .role("button")
        .label(tip.clone())
        .help(tip)
        .size(side, side)
        .center()
        .radius(6.0)
        .fill(t.surface_2)
        .border(1.0, t.border_subtle)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent)
        .hover(t.hover)
        .press(t.press)
        .press_scale(0.96)
        .motion(motion::SNAPPY)
        .on_press_tx(on_press)
        .child(icon(ic, 14.0).color(t.text_secondary).a11y_hidden())
}

pub fn chip<S: 'static>(
    t: UiTheme,
    key: impl Into<String>,
    label: impl Into<String>,
    active: bool,
    on_press: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    chip_with_icon(t, key, label, None, active, on_press)
}

pub fn chip_with_icon<S: 'static>(
    t: UiTheme,
    key: impl Into<String>,
    label: impl Into<String>,
    ic: Option<&'static Icon>,
    active: bool,
    on_press: impl Fn(&mut S, &mut Tx) + 'static,
) -> El {
    let label = label.into();
    let (fill, border, fg) = if active {
        (t.accent_soft, t.accent.alpha(0.45), t.accent)
    } else {
        (t.surface_2, t.border_subtle, t.text_secondary)
    };
    let mut r = row()
        .key(key)
        .role("button")
        .label(label.clone())
        .checked(active)
        .h(30.0)
        .px(11.0)
        .gap(6.0)
        .center()
        .radius(6.0)
        .fill(fill)
        .border(1.0, border)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent)
        .hover(t.hover)
        .press(t.press)
        .press_scale(0.98)
        .motion(motion::SNAPPY)
        .on_press_tx(on_press);
    if let Some(i) = ic {
        r = r.child(icon(i, 13.0).color(fg).a11y_hidden());
    }
    r.child(text(label).font(ty::SMALL_MED).color(fg).a11y_hidden())
}

pub fn header_card(t: UiTheme, key: impl Into<String>, radius: f32, inner: El) -> El {
    el().key(key)
        .w_full()
        .radius(radius)
        .fill(t.surface_1)
        .border(1.0, t.border_strong)
        .child(inner.w_full())
}

pub fn surface_card(t: UiTheme, key: impl Into<String>, radius: f32, inner: El) -> El {
    el().key(key)
        .w_full()
        .radius(radius)
        .fill(t.surface_1)
        .border(1.0, t.border_subtle)
        .child(inner.w_full())
}

pub fn gate_card(t: UiTheme, key: impl Into<String>, radius: f32, inner: El) -> El {
    el().key(key)
        .w_full()
        .radius(radius)
        .fill(t.surface_1)
        .border(1.0, t.warning.alpha(0.38))
        .child(inner.w_full())
}

pub fn run_status_icon(t: UiTheme, status: RunStatus, size: f32) -> El {
    let box_el = el().size(size, size).center();
    match status {
        RunStatus::Success => box_el.child(icon(&icons::CHECK_CIRCLE, size).color(t.success)),
        RunStatus::Failed => box_el.child(icon(&icons::X_CIRCLE, size).color(t.danger)),
        RunStatus::Cancelled => box_el.child(icon(&icons::X_CIRCLE, size).color(t.text_muted)),
        RunStatus::Running | RunStatus::Queued => {
            let dot_side = 8.0_f32.min(size);
            let micro_dot = el()
                .size(dot_side, dot_side)
                .center()
                .child(
                    effect(MARK_PULSE_RADAR)
                        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                        .radius(dot_side * 0.5)
                        .effect_bleed(8.0)
                        .effect_data([4.0, 8.0, 1.8, 0.7]),
                )
                .child(
                    el().size(dot_side, dot_side)
                        .radius(dot_side * 0.5)
                        .fill(t.info),
                );
            box_el.child(micro_dot)
        }
        RunStatus::AwaitingApproval => {
            let dot_side = 8.0_f32.min(size);
            let gate_dot = el()
                .size(dot_side, dot_side)
                .center()
                .child(
                    effect(MARK_PULSE_BREATHE)
                        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                        .radius(dot_side * 0.5)
                        .effect_bleed(6.0)
                        .effect_data([2.0, 6.0, 2.2, 0.55]),
                )
                .child(icon(&icons::BOLT, size).color(t.warning));
            box_el.child(gate_dot)
        }
    }
}

pub fn step_status_icon(t: UiTheme, status: StepStatus, size: f32) -> El {
    let box_el = el().size(size, size).center();
    match status {
        StepStatus::Success => box_el.child(icon(&icons::CHECK_CIRCLE, size).color(t.success)),
        StepStatus::Failed => box_el.child(icon(&icons::X_CIRCLE, size).color(t.danger)),
        StepStatus::Skipped | StepStatus::Pending => {
            box_el.child(icon(&icons::CLOCK, size).color(t.text_muted))
        }
        StepStatus::Running => {
            let dot_side = 8.0_f32.min(size);
            let micro_dot = el()
                .size(dot_side, dot_side)
                .center()
                .child(
                    effect(MARK_PULSE_RADAR)
                        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                        .radius(dot_side * 0.5)
                        .effect_bleed(8.0)
                        .effect_data([4.0, 8.0, 1.8, 0.7]),
                )
                .child(
                    el().size(dot_side, dot_side)
                        .radius(dot_side * 0.5)
                        .fill(t.info),
                );
            box_el.child(micro_dot)
        }
        StepStatus::WaitingApproval => box_el.child(icon(&icons::BOLT, size).color(t.warning)),
    }
}

pub fn status_pill(t: UiTheme, status: RunStatus) -> El {
    let (bg, fg, bd) = match status {
        RunStatus::Success => (t.success_bg, t.success, t.success.alpha(0.32)),
        RunStatus::Failed => (t.danger_bg, t.danger, t.danger.alpha(0.35)),
        RunStatus::Running | RunStatus::Queued => (t.info_bg, t.info, t.info.alpha(0.32)),
        RunStatus::AwaitingApproval => (t.warning_bg, t.warning, t.warning.alpha(0.38)),
        RunStatus::Cancelled => (t.surface_3, t.text_muted, t.border_subtle),
    };
    let indicator = match status {
        RunStatus::Success => icon(&icons::CHECK_CIRCLE, 12.0).color(fg),
        RunStatus::Failed | RunStatus::Cancelled => icon(&icons::X_CIRCLE, 12.0).color(fg),
        RunStatus::Running | RunStatus::Queued => el().size(7.0, 7.0).radius(3.5).fill(fg),
        RunStatus::AwaitingApproval => icon(&icons::BOLT, 12.0).color(fg),
    };
    row()
        .h(22.0)
        .px(8.0)
        .gap(6.0)
        .center_y()
        .radius(11.0)
        .fill(bg)
        .border(1.0, bd)
        .child(indicator)
        .child(text(status.label()).font(ty::CAPTION).color(fg))
}

pub fn runner_connected_dot(t: UiTheme, connected: bool) -> El {
    let mut d = el().size(8.0, 8.0).center();
    if connected {
        d = d
            .child(
                effect(MARK_PULSE_BREATHE)
                    .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                    .radius(4.0)
                    .effect_bleed(6.0)
                    .effect_data([1.0, 6.0, 2.6, 0.55]),
            )
            .child(el().size(8.0, 8.0).radius(4.0).fill(t.success));
    } else {
        d = d.child(el().size(8.0, 8.0).radius(4.0).fill(t.text_muted));
    }
    d
}

pub fn health_pulse_dot(t: UiTheme, health: ConnectionsHealth) -> El {
    let (key, mark, color_idx, dot_color, reach, period, intensity) = match health {
        ConnectionsHealth::Healthy => (
            "sidebar-conn-pulse-healthy",
            MARK_PULSE_BREATHE,
            1.0,
            t.success,
            6.0,
            2.6,
            0.55,
        ),
        ConnectionsHealth::Warning => (
            "sidebar-conn-pulse-warning",
            MARK_PULSE_BREATHE,
            2.0,
            t.warning,
            7.0,
            2.0,
            0.65,
        ),
        ConnectionsHealth::Danger => (
            "sidebar-conn-pulse-danger",
            MARK_PULSE_RADAR,
            3.0,
            t.danger,
            8.0,
            1.5,
            0.75,
        ),
    };
    el().key(key)
        .size(8.0, 8.0)
        .center()
        .child(
            effect(mark)
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .radius(4.0)
                .effect_bleed(reach)
                .effect_data([color_idx, reach, period, intensity]),
        )
        .child(el().size(8.0, 8.0).radius(4.0).fill(dot_color))
}

pub fn copyable_sha(t: UiTheme, key: impl Into<String>, full_sha: &str) -> El {
    let short = &full_sha[..7.min(full_sha.len())];
    let sha_owned = full_sha.to_string();
    let short_owned = short.to_string();
    row()
        .key(key)
        .role("button")
        .label(format!("Copiar commit {short_owned}"))
        .help("Copiar hash completo do commit")
        .h(22.0)
        .px(7.0)
        .gap(5.0)
        .center_y()
        .radius(6.0)
        .fill(t.surface_2)
        .border(1.0, t.border_subtle)
        .cursor(Cursor::Pointer)
        .focusable(true)
        .focus_ring(2.0, 2.0, t.accent)
        .hover(t.hover)
        .press(t.press)
        .press_scale(0.97)
        .on_press_tx(move |s: &mut AppState, tx| {
            tx.copy(sha_owned.clone());
            s.push_toast(
                "copy-sha",
                "SHA copiado",
                Some(format!("Commit {short_owned} copiado para a área de transferência.")),
                ToastKind::Success,
                tx,
            );
        })
        .child(icon(&icons::COMMIT, 11.0).color(t.text_muted).a11y_hidden())
        .child(text(short.to_string()).font(ty::MONO_SM).color(t.text_secondary).a11y_hidden())
        .child(icon(&icons::COPY, 10.5).color(t.text_muted).a11y_hidden())
}

pub fn toast_overlay(s: &AppState, t: UiTheme, k: &Kit) -> Option<El> {
    let entries = s.toasts.entries();
    if entries.is_empty() {
        return None;
    }
    let mut region = ToastRegion::new();
    for entry in entries.iter().rev().take(3).rev() {
        let id = entry.id;
        let msg = &entry.options;
        let lead_ic = match msg.kind {
            ToastKind::Success => icon(&icons::CHECK_CIRCLE, 16.0).color(t.success),
            ToastKind::Warning => icon(&icons::BOLT, 16.0).color(t.warning),
            ToastKind::Danger => icon(&icons::X_CIRCLE, 16.0).color(t.danger),
            ToastKind::Info => icon(&icons::SHIELD, 16.0).color(t.accent),
        };
        let mut toast = Toast::new(format!("toast-{id}"), msg.title.clone())
            .lead(lead_ic)
            .on_close(move |s: &mut AppState| {
                let now = s.clock_t;
                s.toasts.step(ToastEvent::Dismissed { id: Some(id), now });
            });
        if let Some(desc) = &msg.description {
            toast = toast.description(desc.clone());
        }
        region = region.toast(toast.build(k));
    }
    Some(
        region
            .build(k)
            .transition(Transition::move_edge(rune::Edge::Bottom)),
    )
}

pub fn field_box(t: UiTheme, label: &str, input: El) -> El {
    col()
        .gap(4.0)
        .child(
            text(label.to_string())
                .font(ty::CAPTION)
                .color(t.text_secondary),
        )
        .child(input)
}

pub fn pill_badge(t: UiTheme, label: &str, fg: Color, bg: Color) -> El {
    row()
        .h(22.0)
        .px(8.0)
        .center()
        .radius(11.0)
        .fill(bg)
        .border(1.0, t.border_subtle)
        .child(text(label.to_string()).font(ty::CAPTION).color(fg))
}

pub fn tool_chip(t: UiTheme, name: &str, present: bool) -> El {
    let (fg, bg) = if present {
        (t.success, t.success_bg)
    } else {
        (t.text_muted, t.surface_2)
    };
    row()
        .h(22.0)
        .px(8.0)
        .gap(5.0)
        .center_y()
        .radius(6.0)
        .fill(bg)
        .border(1.0, t.border_subtle)
        .child(icon(if present { &icons::CHECK_CIRCLE } else { &icons::CLOCK }, 11.0).color(fg))
        .child(text(name.to_string()).font(ty::CAPTION).color(fg))
}
