#![cfg_attr(not(test), windows_subsystem = "windows")]

mod demo;
mod gfx;
mod state;
mod ui;

use rune::{App, AppSpec, Key, KeyEvent, Tx};
use state::{AppState, ModalState, poll_app, setup_app};

fn on_key(s: &mut AppState, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    if ev.key == Key::Escape {
        if s.open_select.is_some() {
            s.open_select = None;
            return true;
        }
        if !matches!(s.modal, ModalState::None) {
            s.modal = ModalState::None;
            return true;
        }
    }
    false
}

fn main() {
    let _ = rune::set_app_icon(include_bytes!("../assets/icon.png").to_vec());

    let spec = AppSpec::<AppState> {
        min_size: Some((720.0, 560.0)),
        scenes: &[
            ("runs", demo::scene_runs),
            ("onboarding", demo::scene_onboarding),
            ("summary", demo::scene_summary),
            ("logs", demo::scene_logs),
            ("approvals", demo::scene_approvals),
            ("connections", demo::scene_connections),
            ("settings", demo::scene_settings),
            ("editor", demo::scene_editor),
        ],
        fonts: &[
            include_bytes!("../../vendor/rune/crates/rune/assets/Inter-Regular.ttf"),
            include_bytes!("../../vendor/rune/crates/rune/assets/Inter-Medium.ttf"),
            include_bytes!("../../vendor/rune/crates/rune/assets/Inter-SemiBold.ttf"),
            include_bytes!("../../vendor/rune/crates/rune/assets/IBMPlexMono-Regular.ttf"),
            include_bytes!("../../vendor/rune/themes/opencode/assets/fonts/IBMPlexMono-Medium.ttf"),
        ],
        families: ("Inter", "IBM Plex Mono"),
        setup: Some(setup_app),
        poll: Some(poll_app),
        on_key: Some(on_key),
        ..AppSpec::new("Workploy", (1280.0, 820.0), AppState::init_live, ui::view)
    };

    App::new(spec)
        .compositor(gfx::WorkployGfx::default())
        .run();
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Env, Runtime};
    use state::{NavTarget, SettingsTab};
    use workploy::model::{ReleaseInfo, RunStatus};

    #[test]
    fn all_scenes_build_and_layout_across_viewports() {
        let scenes: &[fn(&mut AppState)] = &[
            demo::scene_runs,
            demo::scene_onboarding,
            demo::scene_summary,
            demo::scene_logs,
            demo::scene_approvals,
            demo::scene_connections,
            demo::scene_settings,
            demo::scene_editor,
        ];
        for (idx, setup_scene) in scenes.iter().enumerate() {
            for (w, h) in [
                (1280.0, 820.0),
                (900.0, 700.0),
                (768.0, 820.0),
                (360.0, 720.0),
            ] {
                let mut state = AppState::init_demo();
                setup_scene(&mut state);
                if w < 500.0 {
                    state.sidebar_collapsed = true;
                }
                let mut rt = Runtime::new(state, ui::view, (w, h), Env::new(true, 1.0));
                rt.frame(0.0);
                rt.frame(1.0 / 60.0);
                assert!(
                    !rt.instances.is_empty(),
                    "scene {idx} at {w}x{h} should produce render instances"
                );
            }
        }
    }

    #[test]
    fn sidebar_navigation_and_approval_flow_work_via_pointer() {
        let mut rt = Runtime::new(
            AppState::init_demo(),
            ui::view,
            (1280.0, 820.0),
            Env::new(true, 1.0),
        );
        rt.frame(0.0);
        assert_eq!(rt.state.nav, NavTarget::Runs);
        assert_eq!(rt.state.pending_approvals(), 2);

        // Navigate to Approvals via sidebar click
        let (ax, ay) = rt.find("nav-approvals").expect("nav-approvals exists");
        rt.pointer_down(ax, ay);
        rt.pointer_up(ax, ay);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.nav, NavTarget::Approvals);

        // Approve first pending run
        let (bx, by) = rt
            .find("appr-ok-run-demo-approval-1")
            .expect("approve button exists");
        rt.pointer_down(bx, by);
        rt.pointer_up(bx, by);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.pending_approvals(), 1);
        let run1 = rt
            .state
            .snapshot
            .runs
            .iter()
            .find(|r| r.id == "run-demo-approval-1")
            .unwrap();
        assert_eq!(run1.status, RunStatus::Success);
        assert!(!rt.state.toasts.entries().is_empty());

        // Click "+ Novo fluxo" in sidebar to open Workflow Editor modal
        let (nx, ny) = rt.find("nav-new-wf").expect("nav-new-wf exists");
        rt.pointer_down(nx, ny);
        rt.pointer_up(nx, ny);
        rt.frame(1.0 / 60.0);
        assert!(matches!(rt.state.modal, ModalState::WorkflowEditor(_)));

        // First Escape blurs the focused TextInput inside the modal, second Escape dismisses modal
        rt.key(KeyEvent {
            key: Key::Escape,
            mods: rune::Mods::default(),
        });
        rt.frame(1.0 / 60.0);
        rt.key(KeyEvent {
            key: Key::Escape,
            mods: rune::Mods::default(),
        });
        rt.frame(1.0 / 60.0);
        assert!(matches!(rt.state.modal, ModalState::None));
    }

    #[test]
    fn settings_tabs_vps_modals_and_github_binding_work() {
        let mut rt = Runtime::new(
            AppState::init_demo(),
            ui::view,
            (1280.0, 820.0),
            Env::new(true, 1.0),
        );
        rt.frame(0.0);

        // Token draft must never expose stored token in plaintext
        assert!(rt.state.settings_draft.github_token.text.is_empty());
        assert!(rt.state.snapshot.settings.has_github_token);

        // Navigate to Configurações (starts on GitHub tab)
        let (sx, sy) = rt.find("nav-settings").expect("nav-settings exists");
        rt.pointer_down(sx, sy);
        rt.pointer_up(sx, sy);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.nav, NavTarget::Settings);
        assert_eq!(rt.state.settings_tab, SettingsTab::GitHub);

        // Switch to VPS & Motor tab
        let (vx, vy) = rt
            .find("tab-settings-vps")
            .expect("tab-settings-vps exists");
        rt.pointer_down(vx, vy);
        rt.pointer_up(vx, vy);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.settings_tab, SettingsTab::Vps);

        // Open ServerEditor via "+ Adicionar VPS"
        let (ax, ay) = rt.find("btn-add-vps").expect("btn-add-vps exists");
        rt.pointer_down(ax, ay);
        rt.pointer_up(ax, ay);
        rt.frame(1.0 / 60.0);
        assert!(matches!(rt.state.modal, ModalState::ServerEditor(_)));

        // Close modal
        let (cx, cy) = rt.find("srv-cancel").expect("srv-cancel exists");
        rt.pointer_down(cx, cy);
        rt.pointer_up(cx, cy);
        rt.frame(1.0 / 60.0);
        assert!(matches!(rt.state.modal, ModalState::None));

        // Open ProvisionModal for VPS Produção
        let (px, py) = rt
            .find("btn-provision-vps-prod")
            .expect("btn-provision-vps-prod exists");
        rt.pointer_down(px, py);
        rt.pointer_up(px, py);
        rt.frame(1.0 / 60.0);
        assert!(matches!(rt.state.modal, ModalState::ProvisionModal(_)));

        // Start demo provision and verify workploy-runner log appears
        let (psx, psy) = rt.find("prov-start").expect("prov-start exists");
        rt.pointer_down(psx, psy);
        rt.pointer_up(psx, psy);
        rt.frame(1.0 / 60.0);
        assert!(rt.state.snapshot.provision_log.contains("workploy-runner"));

        let (clx, cly) = rt.find("prov-close").expect("prov-close exists");
        rt.pointer_down(clx, cly);
        rt.pointer_up(clx, cly);
        rt.frame(1.0 / 60.0);
        assert!(matches!(rt.state.modal, ModalState::None));

        // Switch back to GitHub tab and open BindingEditor via "+ Novo vínculo"
        let (gx, gy) = rt
            .find("tab-settings-github")
            .expect("tab-settings-github exists");
        rt.pointer_down(gx, gy);
        rt.pointer_up(gx, gy);
        rt.frame(1.0 / 60.0);

        for _ in 0..8 {
            let Some((_, by)) = rt.find("btn-new-binding") else { break };
            if by > 120.0 && by < 780.0 {
                break;
            }
            rt.wheel(640.0, 400.0, -240.0);
            rt.frame(1.0 / 60.0);
        }
        let (bx, by) = rt
            .find("btn-new-binding")
            .expect("btn-new-binding exists");
        rt.pointer_down(bx, by);
        rt.pointer_up(bx, by);
        rt.frame(1.0 / 60.0);
        assert!(matches!(rt.state.modal, ModalState::BindingEditor(_)));

        // Verify BindingEditor options and tooltips exist
        let els = rt.elements();
        for key in [
            "bind-srv-local",
            "bind-srv-vps-prod",
            "bind-mode-webhook",
            "bind-mode-pollvps",
            "bind-mode-polldesktop",
            "bind-wf-auto",
        ] {
            let el = els
                .iter()
                .find(|e| e.key.as_deref() == Some(key))
                .unwrap_or_else(|| panic!("expected BindingEditor option {key}"));
            assert!(
                el.help.as_deref().is_some_and(|h| !h.is_empty()),
                "expected non-empty tooltip (.help) on {key}"
            );
        }
    }

    #[test]
    fn github_search_two_column_grid_and_pagination_work() {
        let mut state = AppState::init_demo();
        state.open_settings(SettingsTab::GitHub);
        let mut rt = Runtime::new(state, ui::view, (1280.0, 820.0), Env::new(true, 1.0));
        rt.frame(0.0);
        rt.frame(1.0 / 60.0);

        // Wide viewport (>= 820.0): repo 0 and repo 1 sit side by side on the same row
        let (x0, y0) = rt.find("gh-repo-0").expect("gh-repo-0 exists");
        let (x1, y1) = rt.find("gh-repo-1").expect("gh-repo-1 exists");
        assert!(
            (y0 - y1).abs() < 2.0 && x1 > x0 + 100.0,
            "expected 2-column grid at 1280px: ({x0},{y0}) vs ({x1},{y1})"
        );
        assert!(rt.find("gh-repo-7").is_some());
        assert!(rt.find("gh-repo-8").is_none());

        // Scroll down and click "Próxima" to move to page 2
        for _ in 0..8 {
            let Some((_, py)) = rt.find("btn-gh-next") else { break };
            if py > 120.0 && py < 780.0 {
                break;
            }
            rt.wheel(640.0, 400.0, -240.0);
            rt.frame(1.0 / 60.0);
        }
        let (nx, ny) = rt.find("btn-gh-next").expect("btn-gh-next exists");
        rt.pointer_down(nx, ny);
        rt.pointer_up(nx, ny);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.github_repo_page, 1);
        assert!(rt.find("gh-repo-8").is_some());
        assert!(rt.find("gh-repo-9").is_some());
        assert!(rt.find("gh-repo-0").is_none());

        // Scroll back up to search input and type "faturamento" -> on_change resets page from 1 to 0
        for _ in 0..8 {
            let Some((_, sy)) = rt.find("gh-repo-search") else { break };
            if sy > 80.0 && sy < 760.0 {
                break;
            }
            rt.wheel(640.0, 400.0, 240.0);
            rt.frame(1.0 / 60.0);
        }
        let (sx, sy) = rt.find("gh-repo-search").expect("gh-repo-search exists");
        rt.pointer_down(sx, sy);
        rt.pointer_up(sx, sy);
        rt.frame(1.0 / 60.0);
        rt.key(KeyEvent {
            key: Key::Text("faturamento".into()),
            mods: rune::Mods::default(),
        });
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.github_repo_page, 0);
        assert!(rt.find("gh-repo-3").is_some());
        assert!(rt.find("gh-repo-0").is_none());

        // Narrow viewport (< 820.0): collapses to 1 column
        let mut narrow_state = AppState::init_demo();
        narrow_state.open_settings(SettingsTab::GitHub);
        narrow_state.sidebar_collapsed = true;
        let mut rt_narrow =
            Runtime::new(narrow_state, ui::view, (768.0, 820.0), Env::new(true, 1.0));
        rt_narrow.frame(0.0);
        rt_narrow.frame(1.0 / 60.0);
        let (_, ny0) = rt_narrow.find("gh-repo-0").expect("narrow gh-repo-0");
        let (_, ny1) = rt_narrow.find("gh-repo-1").expect("narrow gh-repo-1");
        assert!(
            ny1 > ny0 + 30.0,
            "expected 1-column stack at 768px: y0={ny0}, y1={ny1}"
        );
    }

    #[test]
    fn appearance_and_about_tabs_work_without_cadence_controls() {
        let mut state = AppState::init_demo();
        state.open_settings(SettingsTab::Appearance);
        let mut rt = Runtime::new(state, ui::view, (1280.0, 820.0), Env::new(true, 1.0));
        rt.frame(0.0);

        // Cadence/FPS/slider controls must not exist
        assert!(rt.find("timings-speed-slider").is_none());

        // Toggle theme Claro / Escuro
        let (lx, ly) = rt.find("app-theme-light").expect("app-theme-light exists");
        rt.pointer_down(lx, ly);
        rt.pointer_up(lx, ly);
        rt.frame(1.0 / 60.0);
        assert!(!rt.state.dark);

        let (dx, dy) = rt.find("app-theme-dark").expect("app-theme-dark exists");
        rt.pointer_down(dx, dy);
        rt.pointer_up(dx, dy);
        rt.frame(1.0 / 60.0);
        assert!(rt.state.dark);

        // Toggle animations 0x / 1x with phase continuity
        let p_before = rt.state.fx_phase.at(2.0);
        let (ax0, ay0) = rt.find("app-anim-0x").expect("app-anim-0x exists");
        rt.pointer_down(ax0, ay0);
        rt.pointer_up(ax0, ay0);
        rt.frame(1.0 / 60.0);
        assert!(rt.state.reduce_motion);
        assert!(rt.reduced_motion);

        let (ax1, ay1) = rt.find("app-anim-1x").expect("app-anim-1x exists");
        rt.pointer_down(ax1, ay1);
        rt.pointer_up(ax1, ay1);
        rt.frame(1.0 / 60.0);
        assert!(!rt.state.reduce_motion);
        assert!(!rt.reduced_motion);
        let _ = p_before;

        // Switch to Sobre tab and check for updates
        let (abx, aby) = rt
            .find("tab-settings-about")
            .expect("tab-settings-about exists");
        rt.pointer_down(abx, aby);
        rt.pointer_up(abx, aby);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.settings_tab, SettingsTab::About);
        assert!(rt.find("btn-open-repo").is_some());

        let (ux, uy) = rt
            .find("btn-check-updates")
            .expect("btn-check-updates exists");
        rt.pointer_down(ux, uy);
        rt.pointer_up(ux, uy);
        rt.frame(1.0 / 60.0);
        assert!(rt.state.snapshot.latest_release.is_some());
        assert!(!rt.state.snapshot.update_status.is_empty());
        assert!(rt.find("btn-open-release").is_some());
    }

    #[test]
    fn connections_diagnostic_screen_tests_ssh_github_and_purge() {
        let mut rt = Runtime::new(
            AppState::init_demo(),
            ui::view,
            (1280.0, 820.0),
            Env::new(true, 1.0),
        );
        rt.frame(0.0);

        let (cx, cy) = rt
            .find("nav-connections")
            .expect("nav-connections exists");
        rt.pointer_down(cx, cy);
        rt.pointer_up(cx, cy);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.nav, NavTarget::Connections);

        // Trigger GitHub test and VPS probe
        let (ghx, ghy) = rt
            .find("conn-test-github")
            .expect("conn-test-github exists");
        rt.pointer_down(ghx, ghy);
        rt.pointer_up(ghx, ghy);
        rt.frame(1.0 / 60.0);
        assert!(rt.state.snapshot.github_status.contains("Conectado"));

        let (px, py) = rt
            .find("conn-test-vps-vps-prod")
            .expect("conn-test-vps-vps-prod exists");
        rt.pointer_down(px, py);
        rt.pointer_up(px, py);
        rt.frame(1.0 / 60.0);
        assert!(
            rt.state
                .snapshot
                .ssh_test
                .as_ref()
                .is_some_and(|(ok, _)| *ok)
        );

        let (lx, ly) = rt
            .find("conn-purge-vps-vps-prod")
            .expect("conn-purge-vps-vps-prod exists");
        rt.pointer_down(lx, ly);
        rt.pointer_up(lx, ly);
        rt.frame(1.0 / 60.0);
        assert!(
            rt.state
                .snapshot
                .runner_sync_status
                .contains("Limpeza concluída")
        );
        assert!(rt.find("conn-provision-log").is_some());
    }

    #[test]
    fn onboarding_scene_navigates_to_settings_connections_and_runs() {
        let mut state = AppState::init_demo();
        demo::scene_onboarding(&mut state);
        assert!(state.should_show_onboarding());
        assert_eq!(state.nav, NavTarget::Onboarding);

        let mut rt = Runtime::new(state, ui::view, (1280.0, 820.0), Env::new(true, 1.0));
        rt.frame(0.0);
        assert!(rt.find("onboarding-card").is_some());

        // Shortcut to Configurações -> GitHub
        let (gx, gy) = rt.find("onb-btn-github").expect("onb-btn-github exists");
        rt.pointer_down(gx, gy);
        rt.pointer_up(gx, gy);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.nav, NavTarget::Settings);
        assert_eq!(rt.state.settings_tab, SettingsTab::GitHub);

        // Return to Onboarding and test VPS shortcut
        let (ox, oy) = rt.find("nav-onboarding").expect("nav-onboarding exists");
        rt.pointer_down(ox, oy);
        rt.pointer_up(ox, oy);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.nav, NavTarget::Onboarding);

        let (vx, vy) = rt.find("onb-btn-vps").expect("onb-btn-vps exists");
        rt.pointer_down(vx, vy);
        rt.pointer_up(vx, vy);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.nav, NavTarget::Settings);
        assert_eq!(rt.state.settings_tab, SettingsTab::Vps);

        // Return to Onboarding and test Conexões & Diagnóstico shortcut
        let (ox2, oy2) = rt.find("nav-onboarding").expect("nav-onboarding exists");
        rt.pointer_down(ox2, oy2);
        rt.pointer_up(ox2, oy2);
        rt.frame(1.0 / 60.0);

        let (cx, cy) = rt
            .find("onb-btn-connections")
            .expect("onb-btn-connections exists");
        rt.pointer_down(cx, cy);
        rt.pointer_up(cx, cy);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.nav, NavTarget::Connections);

        // Return to Onboarding and test Execuções shortcut
        let (ox3, oy3) = rt.find("nav-onboarding").expect("nav-onboarding exists");
        rt.pointer_down(ox3, oy3);
        rt.pointer_up(ox3, oy3);
        rt.frame(1.0 / 60.0);

        let (rx, ry) = rt.find("onb-btn-runs").expect("onb-btn-runs exists");
        rt.pointer_down(rx, ry);
        rt.pointer_up(rx, ry);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.nav, NavTarget::Runs);
    }

    #[test]
    fn reduced_motion_and_phase_preserve_continuity() {
        let mut state = AppState::init_demo();
        state.fx_phase.set_speed(2.0, 1.0);
        let p_before = state.fx_phase.at(2.0);
        state.set_animations_enabled(false, 2.0);
        let p_after = state.fx_phase.at(2.0);
        assert!((p_before - p_after).abs() < 1e-5);
        assert!(state.reduce_motion);

        let mut rt = Runtime::new(state, ui::view, (1280.0, 820.0), Env::new(true, 1.0));
        rt.frame(0.0);
        assert!(rt.reduced_motion);
    }

    #[test]
    fn purge_rule_modal_and_manual_vps_purge_work() {
        let mut rt = Runtime::new(
            AppState::init_demo(),
            ui::view,
            (1280.0, 820.0),
            Env::new(true, 1.0),
        );
        rt.frame(0.0);

        let (sx, sy) = rt.find("nav-settings").expect("nav-settings exists");
        rt.pointer_down(sx, sy);
        rt.pointer_up(sx, sy);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.nav, NavTarget::Settings);

        let (vx, vy) = rt
            .find("tab-settings-vps")
            .expect("tab-settings-vps exists");
        rt.pointer_down(vx, vy);
        rt.pointer_up(vx, vy);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.settings_tab, SettingsTab::Vps);

        // Open PurgeRuleEditor modal
        let (px, py) = rt.find("btn-purge-rule").expect("btn-purge-rule exists");
        rt.pointer_down(px, py);
        rt.pointer_up(px, py);
        rt.frame(1.0 / 60.0);
        assert!(matches!(rt.state.modal, ModalState::PurgeRuleEditor(_)));

        // Switch unit to MB and back to GB, checking conversion (default 5120 MB == 5 GB)
        let (mx, my) = rt.find("purge-unit-mb").expect("purge-unit-mb exists");
        rt.pointer_down(mx, my);
        rt.pointer_up(mx, my);
        rt.frame(1.0 / 60.0);
        if let ModalState::PurgeRuleEditor(d) = &rt.state.modal {
            assert_eq!(d.size_value.text, "5120");
            assert_eq!(d.parsed_max_size_mb(), Some(5120));
        } else {
            panic!("expected PurgeRuleEditor");
        }

        let (gx, gy) = rt.find("purge-unit-gb").expect("purge-unit-gb exists");
        rt.pointer_down(gx, gy);
        rt.pointer_up(gx, gy);
        rt.frame(1.0 / 60.0);
        if let ModalState::PurgeRuleEditor(d) = &mut rt.state.modal {
            assert_eq!(d.size_value.text, "5");
            d.enabled = true;
            d.every_n_runs = rune::TextEdit::new("4");
            d.size_value = rune::TextEdit::new("3");
        }

        // Save rule
        let (psx, psy) = rt.find("purge-save").expect("purge-save exists");
        rt.pointer_down(psx, psy);
        rt.pointer_up(psx, psy);
        rt.frame(1.0 / 60.0);
        assert!(matches!(rt.state.modal, ModalState::None));
        assert!(rt.state.snapshot.settings.purge_rule.enabled);
        assert_eq!(rt.state.snapshot.settings.purge_rule.every_n_runs, 4);
        assert_eq!(rt.state.snapshot.settings.purge_rule.max_size_mb, 3072);

        // Saving general settings must preserve purge_rule
        let (ssx, ssy) = rt
            .find("btn-save-settings")
            .expect("btn-save-settings exists");
        rt.pointer_down(ssx, ssy);
        rt.pointer_up(ssx, ssy);
        rt.frame(1.0 / 60.0);
        assert!(rt.state.snapshot.settings.purge_rule.enabled);
        assert_eq!(rt.state.snapshot.settings.purge_rule.max_size_mb, 3072);

        // Trigger "Limpar agora" on VPS Produção
        let (lx, ly) = rt
            .find("btn-purge-vps-vps-prod")
            .expect("btn-purge-vps-vps-prod exists");
        rt.pointer_down(lx, ly);
        rt.pointer_up(lx, ly);
        rt.frame(1.0 / 60.0);
        assert!(
            rt.state
                .snapshot
                .runner_sync_status
                .contains("Limpeza concluída")
        );
    }

    #[test]
    fn github_and_update_events_trigger_automatic_toasts_on_poll() {
        let mut state = AppState::init_demo();
        state.snapshot.github_last_event = Some((
            1,
            false,
            "Token do GitHub inválido ou expirado (HTTP 401).".into(),
        ));
        let mut rt = Runtime::new(state, ui::view, (1280.0, 820.0), Env::new(true, 1.0));
        rt.frame(0.0);

        // Simulate a new GitHub event arriving after wake_gen bump
        rt.state.snapshot.github_last_event = Some((
            2,
            false,
            "Token do GitHub inválido ou expirado (HTTP 401).".into(),
        ));
        let mut tx = Tx::default();
        assert!(poll_app(&mut rt.state, 0.1, &mut tx));
        rt.frame(0.1);
        assert_eq!(rt.state.last_github_event_seq, 2);
        let last = rt.state.toasts.entries().last().expect("danger toast emitted");
        assert_eq!(last.options.kind, state::ToastKind::Danger);
        assert!(
            last.options
                .description
                .as_deref()
                .is_some_and(|d| d.contains("401"))
        );

        // Now simulate GitHub success event
        rt.state.snapshot.github_last_event = Some((
            3,
            true,
            "Conectado como @matheuswidder · 10 repositórios carregados".into(),
        ));
        assert!(poll_app(&mut rt.state, 0.2, &mut tx));
        rt.frame(0.2);
        assert_eq!(rt.state.last_github_event_seq, 3);
        let last = rt
            .state
            .toasts
            .entries()
            .last()
            .expect("success toast emitted");
        assert_eq!(last.options.kind, state::ToastKind::Success);

        // Simulate update check error -> Danger toast
        rt.state.snapshot.update_last_event =
            Some((1, false, "Não foi possível verificar atualizações".into()));
        assert!(poll_app(&mut rt.state, 0.3, &mut tx));
        rt.frame(0.3);
        assert_eq!(rt.state.last_update_event_seq, 1);
        let last = rt
            .state
            .toasts
            .entries()
            .last()
            .expect("update danger toast emitted");
        assert_eq!(last.options.kind, state::ToastKind::Danger);

        // Simulate update available -> Success toast
        rt.state.snapshot.latest_release = Some(ReleaseInfo {
            tag_name: "v0.2.0".into(),
            version: "0.2.0".into(),
            html_url: "https://github.com/matheuswidder/w-workploy/releases/tag/v0.2.0".into(),
            installer_url: "https://github.com/matheuswidder/w-workploy/releases/download/v0.2.0/Workploy-Setup-0.2.0.exe".into(),
            published_at: "2026-11-18T12:00:00Z".into(),
            notes: "Nova release".into(),
            has_update: true,
        });
        rt.state.snapshot.update_last_event =
            Some((2, true, "Nova versão disponível: v0.2.0".into()));
        assert!(poll_app(&mut rt.state, 0.4, &mut tx));
        rt.frame(0.4);
        assert_eq!(rt.state.last_update_event_seq, 2);
        let last = rt
            .state
            .toasts
            .entries()
            .last()
            .expect("update success toast emitted");
        assert_eq!(last.options.kind, state::ToastKind::Success);

        // Simulate already up-to-date -> Info toast
        rt.state.snapshot.latest_release = Some(ReleaseInfo {
            has_update: false,
            ..Default::default()
        });
        rt.state.snapshot.update_last_event =
            Some((3, true, "Você já está na versão mais recente.".into()));
        assert!(poll_app(&mut rt.state, 0.5, &mut tx));
        rt.frame(0.5);
        assert_eq!(rt.state.last_update_event_seq, 3);
        let last = rt
            .state
            .toasts
            .entries()
            .last()
            .expect("update info toast emitted");
        assert_eq!(last.options.kind, state::ToastKind::Info);
    }
}
