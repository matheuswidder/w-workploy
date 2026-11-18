pub mod approvals;
pub mod components;
pub mod connections;
pub mod detail;
pub mod icons;
pub mod modals;
pub mod onboarding;
pub mod runs;
pub mod settings;
pub mod sidebar;
pub mod theme;

use rune::tok::motion;
use rune::{Cx, El, row};

use crate::state::{AppState, NavTarget};
use crate::ui::components::toast_overlay;
use crate::ui::theme::UiTheme;

pub fn view(s: &AppState, cx: &mut Cx<AppState>) -> El {
    cx.reduced_motion(s.reduce_motion || cx.system_reduced_motion());
    cx.time_scale(1.0);

    let narrow = cx
        .layout_with("workploy-shell", |r| r[2] < 820.0)
        .unwrap_or(false);
    let main_w = cx
        .layout_with("workploy-main", |r| r[2])
        .unwrap_or(1032.0);

    let t = UiTheme::new(s.dark, cx.env().scale);
    let k = t.build_kit();
    cx.foreground(t.text_primary);
    cx.focus_ring(2.0, 2.0, t.accent);

    let mut layer: Vec<El> = Vec::new();

    let main_content = match &s.nav {
        NavTarget::Onboarding => onboarding::view(s, t),
        NavTarget::Runs => runs::view(
            s,
            t,
            &k,
            s.selected_workflow_id.as_deref(),
            main_w,
            &mut layer,
        ),
        NavTarget::WorkflowRuns(id) => runs::view(s, t, &k, Some(id), main_w, &mut layer),
        NavTarget::Approvals => approvals::view(s, t),
        NavTarget::Connections => connections::view(s, t, main_w),
        NavTarget::Settings => settings::view(s, t, &k, narrow),
        NavTarget::RunDetail { run_id, tab } => detail::view(s, t, &k, run_id, *tab),
    };

    modals::mount_modal(s, t, &k, &mut layer);

    if let Some(toast_el) = toast_overlay(s, t, &k) {
        layer.push(toast_el);
    }

    let shell = row()
        .key("workploy-shell")
        .role("window")
        .w_full()
        .h_full()
        .min_h(0.0)
        .fill(t.bg_canvas)
        .motion(motion::DEFAULT)
        .child(sidebar::view(s, t))
        .child(
            rune::col()
                .key("workploy-main")
                .grow(1.0)
                .shrink()
                .min_w(0.0)
                .min_h(0.0)
                .h_full()
                .child(main_content),
        );

    rune_kit::root(shell, layer)
}
