#![cfg_attr(not(test), windows_subsystem = "windows")]

mod demo;
mod gfx;
mod state;
mod ui;

use rune::{App, AppSpec, Key, KeyEvent, Tray, TrayEvent, TrayIcon, TrayItem, TraySpec, Tx};
use state::{AppState, ModalState, poll_app, setup_app};

fn load_tray_icon() -> Option<TrayIcon> {
    let img = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .ok()?
        .into_rgba8();
    let (w, h) = img.dimensions();
    Some(TrayIcon::rgba(w, h, img.into_raw()))
}

fn on_close(s: &mut AppState, tx: &mut Tx) {
    if !s.snapshot.settings.minimize_to_tray {
        tx.quit();
    }
}

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

const SINGLE_INSTANCE_ADDR: &str = "127.0.0.1:48731";

fn is_demo_mode() -> bool {
    rune::tooling::extra_args()
        .iter()
        .any(|a| a == "--demo" || a == "-demo")
        || std::env::args().any(|a| a == "--demo" || a == "-demo")
}

fn try_acquire_single_instance(addr: &str) -> Result<std::net::TcpListener, bool> {
    match std::net::TcpListener::bind(addr) {
        Ok(listener) => Ok(listener),
        Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => {
            if let Ok(sock_addr) = addr.parse::<std::net::SocketAddr>()
                && let Ok(mut stream) = std::net::TcpStream::connect_timeout(
                    &sock_addr,
                    std::time::Duration::from_millis(500),
                )
            {
                let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(500)));
                let _ = std::io::Write::write_all(&mut stream, b"SHOW\n");
                let _ = std::io::Write::flush(&mut stream);
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(true)
        }
        Err(_) => Err(false),
    }
}

fn spawn_single_instance_listener<F>(listener: std::net::TcpListener, on_show: F)
where
    F: Fn() + Send + 'static,
{
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                break;
            };
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));
            let mut buf = [0u8; 64];
            if let Ok(n) = std::io::Read::read(&mut stream, &mut buf) {
                let cmd = String::from_utf8_lossy(&buf[..n]);
                if cmd.lines().any(|line| line.trim() == "SHOW") {
                    on_show();
                }
            }
        }
    });
}

fn main() {
    let is_headless = rune::tooling::is_headless();
    let demo_mode = is_demo_mode();

    let single_instance_listener = if !is_headless && !demo_mode {
        match try_acquire_single_instance(SINGLE_INSTANCE_ADDR) {
            Ok(listener) => Some(listener),
            Err(true) => return,
            Err(false) => None,
        }
    } else {
        None
    };

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

    let app = App::new(spec)
        .compositor(gfx::WorkployGfx::default())
        .keep_running()
        .on_close(on_close);

    let handle = app.handle();
    if let Some(listener) = single_instance_listener {
        let show_handle = handle.clone();
        spawn_single_instance_listener(listener, move || {
            show_handle.open();
        });
    }

    let _tray = if !is_headless {
        load_tray_icon().and_then(|tray_icon| {
            let tray_handle = handle.clone();
            let tray_spec = TraySpec::new("br.com.widder.workploy", "Workploy", tray_icon)
                .tooltip("Workploy — CI/CD & Deploy self-hosted")
                .menu(vec![
                    TrayItem::action("open", "Abrir Workploy"),
                    TrayItem::Separator,
                    TrayItem::action("quit", "Sair"),
                ]);
            Tray::new(tray_spec, move |ev| match ev {
                TrayEvent::Activate { .. } => tray_handle.open(),
                TrayEvent::Menu(id) if id == "open" => tray_handle.open(),
                TrayEvent::Menu(id) if id == "quit" => tray_handle.quit(),
                _ => {}
            })
            .ok()
        })
    } else {
        None
    };

    app.run();
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

    #[test]
    fn binding_editor_branch_chips_and_workflow_scan_work() {
        let mut state = AppState::init_demo();
        state.open_settings(SettingsTab::GitHub);
        let mut rt = Runtime::new(state, ui::view, (1280.0, 820.0), Env::new(true, 1.0));
        rt.frame(0.0);

        // Open BindingEditor prefilled with first repo ("widder/erp-central") via pointer click
        let (rx, ry) = rt
            .find("btn-bind-repo-0")
            .expect("btn-bind-repo-0 exists");
        rt.pointer_down(rx, ry);
        rt.pointer_up(rx, ry);
        rt.frame(1.0 / 60.0);
        assert!(matches!(rt.state.modal, ModalState::BindingEditor(_)));

        // Click "Buscar branches"
        let (bx, by) = rt
            .find("bind-fetch-branches")
            .expect("bind-fetch-branches exists");
        rt.pointer_down(bx, by);
        rt.pointer_up(bx, by);
        rt.frame(1.0 / 60.0);
        assert_eq!(
            rt.state.snapshot.repo_branches,
            vec!["main", "dev", "staging"]
        );

        // Click "Todas (*)"
        let (ax, ay) = rt
            .find("bind-branch-all")
            .expect("bind-branch-all exists");
        rt.pointer_down(ax, ay);
        rt.pointer_up(ax, ay);
        rt.frame(1.0 / 60.0);
        if let ModalState::BindingEditor(d) = &rt.state.modal {
            assert_eq!(d.branch.text, "*");
        } else {
            panic!("expected BindingEditor");
        }

        // Click "Nenhuma"
        let (nx, ny) = rt
            .find("bind-branch-none")
            .expect("bind-branch-none exists");
        rt.pointer_down(nx, ny);
        rt.pointer_up(nx, ny);
        rt.frame(1.0 / 60.0);
        if let ModalState::BindingEditor(d) = &rt.state.modal {
            assert_eq!(d.branch.text, "");
        }

        // Select multi-branch chips: main + dev -> "main,dev"
        let (b0x, b0y) = rt
            .find("bind-branch-opt-0")
            .expect("bind-branch-opt-0 exists");
        rt.pointer_down(b0x, b0y);
        rt.pointer_up(b0x, b0y);
        rt.frame(1.0 / 60.0);
        let (b1x, b1y) = rt
            .find("bind-branch-opt-1")
            .expect("bind-branch-opt-1 exists");
        rt.pointer_down(b1x, b1y);
        rt.pointer_up(b1x, b1y);
        rt.frame(1.0 / 60.0);
        if let ModalState::BindingEditor(d) = &rt.state.modal {
            assert_eq!(d.branch.text, "main,dev");
        }

        // Toggle "main" off -> "dev"
        let (b0x2, b0y2) = rt
            .find("bind-branch-opt-0")
            .expect("bind-branch-opt-0 exists");
        rt.pointer_down(b0x2, b0y2);
        rt.pointer_up(b0x2, b0y2);
        rt.frame(1.0 / 60.0);
        if let ModalState::BindingEditor(d) = &rt.state.modal {
            assert_eq!(d.branch.text, "dev");
        }

        // Click "Buscar fluxos no repo"
        let (wx, wy) = rt
            .find("bind-scan-workflows")
            .expect("bind-scan-workflows exists");
        rt.pointer_down(wx, wy);
        rt.pointer_up(wx, wy);
        rt.frame(1.0 / 60.0);
        assert!(!rt.state.snapshot.repo_workflows.is_empty());
        assert!(rt.find("bind-detected-workflows").is_some());
    }

    #[test]
    fn smart_sidebar_pulse_and_connections_alerts_work() {
        let mut rt = Runtime::new(
            AppState::init_demo(),
            ui::view,
            (1280.0, 820.0),
            Env::new(true, 1.0),
        );
        rt.frame(0.0);

        // Initially healthy & not seen -> green pulse shown
        assert_eq!(rt.state.connections_health(), state::ConnectionsHealth::Healthy);
        assert!(!rt.state.connections_seen_healthy);
        assert!(rt.find("sidebar-conn-pulse-healthy").is_some());

        // Click Conexões & Diagnóstico while healthy -> green pulse disappears
        let (cx, cy) = rt
            .find("nav-connections")
            .expect("nav-connections exists");
        rt.pointer_down(cx, cy);
        rt.pointer_up(cx, cy);
        rt.frame(1.0 / 60.0);
        assert!(rt.state.connections_seen_healthy);
        assert!(rt.find("sidebar-conn-pulse-healthy").is_none());

        // Trigger CPU >= 80% on VPS Staging -> Warning pulse + alert banner in Connections
        rt.update(|s, _| {
            if let Some(p) = &mut s.snapshot.settings.servers[1].last_probe {
                p.cpu_usage_pct = 86;
            }
            s.sync_connections_health_seen();
        });
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.connections_health(), state::ConnectionsHealth::Warning);
        assert!(!rt.state.connections_seen_healthy);
        assert!(rt.find("sidebar-conn-pulse-warning").is_some());
        assert!(rt.find("connections-health-alert-banner").is_some());

        // Trigger failed probe -> Danger pulse
        rt.update(|s, _| {
            if let Some(p) = &mut s.snapshot.settings.servers[0].last_probe {
                p.ok = false;
            }
            s.sync_connections_health_seen();
        });
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.connections_health(), state::ConnectionsHealth::Danger);
        assert!(rt.find("sidebar-conn-pulse-danger").is_some());
    }

    #[test]
    fn onboarding_completion_reopen_and_tray_toggle_work() {
        let mut state = AppState::init_demo();
        demo::scene_onboarding(&mut state);
        let mut rt = Runtime::new(state, ui::view, (1280.0, 820.0), Env::new(true, 1.0));
        rt.frame(0.0);
        assert_eq!(rt.state.nav, NavTarget::Onboarding);
        assert!(rt.find("nav-onboarding").is_some());

        // Scroll to bottom of onboarding if needed and click "Concluir configuração inicial"
        for _ in 0..8 {
            let Some((_, by)) = rt.find("onb-btn-complete") else { break };
            if by > 80.0 && by < 780.0 {
                break;
            }
            rt.wheel(640.0, 400.0, -240.0);
            rt.frame(1.0 / 60.0);
        }
        let (bx, by) = rt
            .find("onb-btn-complete")
            .expect("onb-btn-complete exists");
        rt.pointer_down(bx, by);
        rt.pointer_up(bx, by);
        rt.frame(1.0 / 60.0);
        assert!(rt.state.snapshot.settings.onboarding_completed);
        assert_eq!(rt.state.nav, NavTarget::Runs);
        assert!(rt.find("nav-onboarding").is_none());

        // Open Settings -> Appearance and toggle minimize_to_tray
        let (sx, sy) = rt.find("nav-settings").expect("nav-settings exists");
        rt.pointer_down(sx, sy);
        rt.pointer_up(sx, sy);
        rt.frame(1.0 / 60.0);

        let (apx, apy) = rt
            .find("tab-settings-appearance")
            .expect("tab-settings-appearance exists");
        rt.pointer_down(apx, apy);
        rt.pointer_up(apx, apy);
        rt.frame(1.0 / 60.0);

        assert!(rt.state.snapshot.settings.minimize_to_tray);
        let (tx, ty) = rt
            .find("app-minimize-to-tray")
            .expect("app-minimize-to-tray exists");
        rt.pointer_down(tx, ty);
        rt.pointer_up(tx, ty);
        rt.frame(1.0 / 60.0);
        assert!(!rt.state.snapshot.settings.minimize_to_tray);

        // Open About tab and reopen onboarding
        let (abx, aby) = rt
            .find("tab-settings-about")
            .expect("tab-settings-about exists");
        rt.pointer_down(abx, aby);
        rt.pointer_up(abx, aby);
        rt.frame(1.0 / 60.0);

        let (rx, ry) = rt
            .find("btn-reopen-onboarding")
            .expect("btn-reopen-onboarding exists");
        rt.pointer_down(rx, ry);
        rt.pointer_up(rx, ry);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.nav, NavTarget::Onboarding);
    }

    #[test]
    fn about_tab_integrated_auto_update_shows_install_button_and_progress() {
        let mut state = AppState::init_demo();
        state.open_settings(SettingsTab::About);
        state.snapshot.latest_release = Some(ReleaseInfo {
            tag_name: "v0.9.0".into(),
            version: "0.9.0".into(),
            html_url: "https://github.com/matheuswidder/w-workploy/releases/tag/v0.9.0".into(),
            installer_url: "https://github.com/matheuswidder/w-workploy/releases/download/v0.9.0/Workploy-Setup-0.9.0.exe".into(),
            published_at: "2026-11-18T14:00:00Z".into(),
            notes: "Atualização integrada".into(),
            has_update: true,
        });
        state.snapshot.update_status = "Nova versão disponível: v0.9.0".into();

        let mut rt = Runtime::new(state, ui::view, (1280.0, 820.0), Env::new(true, 1.0));
        rt.frame(0.0);
        rt.frame(1.0 / 60.0);

        assert!(rt.find("btn-update-install").is_some());
        assert!(rt.find("btn-update-open").is_some());
        assert!(rt.find("about-update-progress").is_none());
        assert!(!rt.state.has_active_runs() || rt.state.snapshot.runs.iter().any(|r| r.status.active()));

        // Click "Baixar e instalar v0.9.0 agora" in demo mode
        let (ix, iy) = rt
            .find("btn-update-install")
            .expect("btn-update-install exists");
        rt.pointer_down(ix, iy);
        rt.pointer_up(ix, iy);
        rt.frame(1.0 / 60.0);

        assert!(rt.state.snapshot.update_downloading);
        assert!(rt.state.has_active_runs());
        assert!(rt.find("about-update-progress").is_some());
        assert!(!rt.state.toasts.entries().is_empty());

        // Clicking disabled install button again must not reset or fail
        let (ix2, iy2) = rt
            .find("btn-update-install")
            .expect("disabled btn-update-install still exists");
        let prev_progress = rt.state.snapshot.update_progress;
        rt.pointer_down(ix2, iy2);
        rt.pointer_up(ix2, iy2);
        rt.frame(1.0 / 60.0);
        assert_eq!(rt.state.snapshot.update_progress, prev_progress);
    }

    #[test]
    fn connections_kpi_strip_test_all_and_log_visibility_work() {
        let mut state = AppState::init_demo();
        state.navigate_to(NavTarget::Connections);
        let mut rt = Runtime::new(state, ui::view, (1280.0, 820.0), Env::new(true, 1.0));
        rt.frame(0.0);
        rt.frame(1.0 / 60.0);

        for key in [
            "conn-kpi-grid",
            "kpi-servers-online",
            "kpi-cpu-avg",
            "kpi-ram-avg",
            "kpi-disk-avg",
            "kpi-github-auto",
            "conn-test-all-vps",
            "conn-test-github",
            "conn-provision-log",
        ] {
            assert!(rt.find(key).is_some(), "expected element {key}");
        }

        // Test all VPSs
        let (ax, ay) = rt
            .find("conn-test-all-vps")
            .expect("conn-test-all-vps exists");
        rt.pointer_down(ax, ay);
        rt.pointer_up(ax, ay);
        rt.frame(1.0 / 60.0);
        assert!(
            rt.state
                .snapshot
                .ssh_test
                .as_ref()
                .is_some_and(|(ok, msg)| *ok && msg.contains("2 VPS"))
        );

        // Clear activity log -> log card hides when both provision_log and runner_sync_status are empty
        for _ in 0..8 {
            let Some((_, cy)) = rt.find("btn-clear-prov-log") else { break };
            if cy > 80.0 && cy < 780.0 {
                break;
            }
            rt.wheel(640.0, 400.0, -240.0);
            rt.frame(1.0 / 60.0);
        }
        let (lx, ly) = rt
            .find("btn-clear-prov-log")
            .expect("btn-clear-prov-log exists");
        rt.pointer_down(lx, ly);
        rt.pointer_up(lx, ly);
        rt.frame(1.0 / 60.0);
        assert!(rt.find("conn-provision-log").is_none());
    }

    #[test]
    fn provision_modal_states_wrapping_and_responsive_viewports_work() {
        let long_log = "Created symlink /etc/systemd/system/multi-user.target.wants/workploy-runner-very-long-unit-name-without-spaces-to-test-wrap.service -> /etc/systemd/system/workploy-runner.service.\n=== Provisionamento concluído! ===";

        // 1. Check wrapping and container bounds across 360px, 768px, 1440px
        for vw in [360.0_f32, 768.0, 1440.0] {
            let mut state = AppState::init_demo();
            state.open_provision_modal("vps-prod");
            state.snapshot.provision_log = long_log.into();
            let mut rt = Runtime::new(state, ui::view, (vw, 820.0), Env::new(true, 1.0));
            rt.frame(0.0);
            rt.frame(1.0 / 60.0);

            let els = rt.elements();
            let dialog = els
                .iter()
                .find(|e| e.key.as_deref() == Some("provision-modal-dialog"))
                .expect("provision-modal-dialog exists");
            let log_box = els
                .iter()
                .find(|e| e.key.as_deref() == Some("prov-log-box"))
                .expect("prov-log-box exists");
            let log_text = els
                .iter()
                .find(|e| e.key.as_deref() == Some("prov-log-text"))
                .expect("prov-log-text exists");

            assert!(
                dialog.rect[2] <= vw + 0.5,
                "dialog wider than viewport {vw}: {:?}",
                dialog.rect
            );
            assert!(
                log_box.rect[0] + log_box.rect[2] <= dialog.rect[0] + dialog.rect[2] + 0.5,
                "log_box overflows dialog at {vw}px: box={:?}, dialog={:?}",
                log_box.rect,
                dialog.rect
            );
            assert!(
                log_text.rect[0] + log_text.rect[2] <= log_box.rect[0] + log_box.rect[2] + 0.5,
                "log_text overflows log_box at {vw}px: text={:?}, box={:?}",
                log_text.rect,
                log_box.rect
            );
        }

        // 2. While is_provisioning is true: animated indicator + disabled "Em andamento..." button
        let mut state_running = AppState::init_demo();
        state_running.open_provision_modal("vps-prod");
        state_running.snapshot.provisioning_server_id = Some("vps-prod".into());
        state_running.snapshot.provision_log = "Conectando via SSH...".into();
        let mut rt_running = Runtime::new(
            state_running,
            ui::view,
            (1280.0, 820.0),
            Env::new(true, 1.0),
        );
        rt_running.frame(0.0);
        rt_running.frame(1.0 / 60.0);

        assert!(rt_running.find("prov-progress-indicator").is_some());
        assert!(rt_running.find("prov-active-pulse").is_some());
        let els_run = rt_running.elements();
        let start_btn = els_run
            .iter()
            .find(|e| e.key.as_deref() == Some("prov-start"))
            .expect("prov-start exists while running");
        assert_eq!(start_btn.label.as_deref(), Some("Em andamento..."));
        assert!(!start_btn.focusable, "disabled button must not take focus");

        // Clicking disabled "Em andamento..." does not overwrite provision_log
        let (sx, sy) = rt_running.find("prov-start").unwrap();
        rt_running.pointer_down(sx, sy);
        rt_running.pointer_up(sx, sy);
        rt_running.frame(1.0 / 60.0);
        assert_eq!(
            rt_running.state.snapshot.provision_log,
            "Conectando via SSH..."
        );

        // 3. After completion ("=== Provisionamento concluído! ==="): badge, banner, "Executar novamente", "Concluir"
        let mut state_done = AppState::init_demo();
        state_done.open_provision_modal("vps-prod");
        let mut rt_done =
            Runtime::new(state_done, ui::view, (1280.0, 820.0), Env::new(true, 1.0));
        rt_done.frame(0.0);
        rt_done.frame(1.0 / 60.0);

        let (psx, psy) = rt_done.find("prov-start").expect("prov-start exists");
        rt_done.pointer_down(psx, psy);
        rt_done.pointer_up(psx, psy);
        for _ in 0..30 {
            rt_done.frame(1.0 / 60.0);
        }

        assert!(rt_done.find("prov-success-badge").is_some());
        assert!(rt_done.find("prov-success-banner").is_some());
        assert!(rt_done.find("prov-progress-indicator").is_none());

        let els_done = rt_done.elements();
        let rerun_btn = els_done
            .iter()
            .find(|e| e.key.as_deref() == Some("prov-rerun"))
            .expect("prov-rerun exists after completion");
        assert_eq!(rerun_btn.label.as_deref(), Some("Executar novamente"));
        let conclude_btn = els_done
            .iter()
            .find(|e| e.key.as_deref() == Some("prov-close"))
            .expect("prov-close (Concluir) exists after completion");
        assert_eq!(conclude_btn.label.as_deref(), Some("Concluir"));

        // Click "Executar novamente" -> stays open and keeps completion
        let (rx, ry) = rt_done.find("prov-rerun").unwrap();
        rt_done.pointer_down(rx, ry);
        rt_done.pointer_up(rx, ry);
        for _ in 0..30 {
            rt_done.frame(1.0 / 60.0);
        }
        assert!(matches!(rt_done.state.modal, ModalState::ProvisionModal(_)));

        // Click "Concluir" -> closes modal
        let (cx, cy) = rt_done.find("prov-close").unwrap();
        rt_done.pointer_down(cx, cy);
        rt_done.pointer_up(cx, cy);
        rt_done.frame(1.0 / 60.0);
        assert!(matches!(rt_done.state.modal, ModalState::None));

        // 4. Failure state ([erro] in log or "Provisionamento falhou" in runner_sync_status): error banner above "Tentar novamente"
        for (log, sync_status) in [
            ("[erro] Falha na conexão SSH", ""),
            ("Conectando...", "Provisionamento falhou — consulte o log."),
        ] {
            let mut state_err = AppState::init_demo();
            state_err.open_provision_modal("vps-prod");
            state_err.snapshot.provision_log = log.into();
            state_err.snapshot.runner_sync_status = sync_status.into();
            let mut rt_err =
                Runtime::new(state_err, ui::view, (1280.0, 820.0), Env::new(true, 1.0));
            rt_err.frame(0.0);
            rt_err.frame(1.0 / 60.0);

            let els_err = rt_err.elements();
            let banner = els_err
                .iter()
                .find(|e| e.key.as_deref() == Some("prov-error-banner"))
                .expect("prov-error-banner exists on failure");
            let retry_btn = els_err
                .iter()
                .find(|e| e.key.as_deref() == Some("prov-start"))
                .expect("prov-start (Tentar novamente) exists on failure");
            assert_eq!(retry_btn.label.as_deref(), Some("Tentar novamente"));
            assert!(
                banner.rect[1] + banner.rect[3] <= retry_btn.rect[1] + 0.5,
                "error banner should sit above Tentar novamente: banner={:?}, retry={:?}",
                banner.rect,
                retry_btn.rect
            );
        }
    }

    #[test]
    fn single_instance_notifies_primary_when_addr_in_use() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let listener = try_acquire_single_instance("127.0.0.1:0")
            .expect("ephemeral single-instance bind should succeed");
        let addr = listener.local_addr().unwrap().to_string();

        let show_calls = Arc::new(AtomicUsize::new(0));
        let show_calls_bg = Arc::clone(&show_calls);
        spawn_single_instance_listener(listener, move || {
            show_calls_bg.fetch_add(1, Ordering::SeqCst);
        });

        let second = try_acquire_single_instance(&addr);
        assert_eq!(second.err(), Some(true), "second bind must see AddrInUse");

        for _ in 0..20 {
            if show_calls.load(Ordering::SeqCst) > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(show_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn provision_modal_clears_stale_log_when_idle_and_keeps_active_log() {
        let mut state = AppState::init_demo();
        state.snapshot.provisioning_server_id = None;
        state.snapshot.provision_log = "=== Provisionamento concluído! ===\nlog antigo".into();

        state.open_provision_modal("vps-staging");
        assert!(
            state.snapshot.provision_log.is_empty(),
            "idle open_provision_modal must clear previous VPS log"
        );
        assert!(matches!(state.modal, ModalState::ProvisionModal(_)));

        // When a provision is actively running, opening the modal must preserve the active log
        state.snapshot.provisioning_server_id = Some("vps-prod".into());
        state.snapshot.provision_log = "Instalando Docker...".into();
        state.open_provision_modal("vps-staging");
        assert_eq!(state.snapshot.provision_log, "Instalando Docker...");
    }

    #[test]
    fn sync_last_event_triggers_automatic_toasts_on_poll() {
        let state = AppState::init_demo();
        let mut rt = Runtime::new(state, ui::view, (1280.0, 820.0), Env::new(true, 1.0));
        rt.frame(0.0);

        let mut tx = Tx::default();
        rt.state.snapshot.sync_last_event = Some((
            1,
            true,
            "Regras e repositórios sincronizados em 'VPS Produção'.".into(),
        ));
        assert!(poll_app(&mut rt.state, 0.1, &mut tx));
        rt.frame(0.1);
        assert_eq!(rt.state.last_sync_event_seq, 1);
        let ok_toast = rt
            .state
            .toasts
            .entries()
            .last()
            .expect("sync success toast emitted");
        assert_eq!(ok_toast.options.kind, state::ToastKind::Success);
        assert!(
            ok_toast
                .options
                .description
                .as_deref()
                .is_some_and(|d| d.contains("VPS Produção"))
        );

        rt.state.snapshot.sync_last_event = Some((
            2,
            false,
            "Falha ao enviar configuração para 'VPS Produção': timeout".into(),
        ));
        assert!(poll_app(&mut rt.state, 0.2, &mut tx));
        rt.frame(0.2);
        assert_eq!(rt.state.last_sync_event_seq, 2);
        let err_toast = rt
            .state
            .toasts
            .entries()
            .last()
            .expect("sync danger toast emitted");
        assert_eq!(err_toast.options.kind, state::ToastKind::Danger);
        assert!(
            err_toast
                .options
                .description
                .as_deref()
                .is_some_and(|d| d.contains("timeout"))
        );
    }

    #[test]
    fn binding_editor_branches_scroll_container_bounds_height_and_keeps_controls_fixed() {
        // 1. Without fetched branches, scroll box is hidden while quick controls stay visible
        let mut empty_state = AppState::init_demo();
        empty_state.open_new_binding_modal(None);
        let mut rt_empty = Runtime::new(
            empty_state,
            ui::view,
            (1280.0, 820.0),
            Env::new(true, 1.0),
        );
        rt_empty.frame(0.0);
        rt_empty.frame(1.0 / 60.0);
        assert!(rt_empty.find("bind-fetch-branches").is_some());
        assert!(rt_empty.find("bind-branch-all").is_some());
        assert!(rt_empty.find("bind-branch-none").is_some());
        assert!(rt_empty.find("bind-branches-scroll").is_none());

        // 2. With 24 branches across 360px, 768px, 1440px: compact scroll <= 104px, no overflow, controls above
        for vw in [360.0_f32, 768.0, 1440.0] {
            let mut state = AppState::init_demo();
            state.open_new_binding_modal(None);
            if let ModalState::BindingEditor(d) = &mut state.modal {
                d.full_name = rune::TextEdit::new("widder/erp-central");
            }
            state.snapshot.repo_branches_repo = "widder/erp-central".into();
            state.snapshot.repo_branches = (0..24)
                .map(|i| format!("feature/module-{i:02}-long-branch"))
                .collect();

            let mut rt = Runtime::new(state, ui::view, (vw, 820.0), Env::new(true, 1.0));
            rt.frame(0.0);
            rt.frame(1.0 / 60.0);

            let els = rt.elements();
            let dialog = els
                .iter()
                .find(|e| e.key.as_deref() == Some("binding-modal-dialog"))
                .expect("binding-modal-dialog exists");
            let scroll = els
                .iter()
                .find(|e| e.key.as_deref() == Some("bind-branches-scroll"))
                .expect("bind-branches-scroll exists");
            assert!(
                scroll.rect[3] <= 104.5,
                "branch scroll container exceeded max_h(104.0) at {vw}px: {:?}",
                scroll.rect
            );
            assert!(
                scroll.rect[0] + scroll.rect[2] <= dialog.rect[0] + dialog.rect[2] + 0.5,
                "branch scroll container overflowed dialog at {vw}px: scroll={:?}, dialog={:?}",
                scroll.rect,
                dialog.rect
            );
            for ctrl_key in ["bind-fetch-branches", "bind-branch-all", "bind-branch-none"] {
                let ctrl = els
                    .iter()
                    .find(|e| e.key.as_deref() == Some(ctrl_key))
                    .unwrap_or_else(|| panic!("{ctrl_key} exists"));
                assert!(
                    ctrl.rect[1] + ctrl.rect[3] <= scroll.rect[1] + 0.5,
                    "{ctrl_key} should sit above bind-branches-scroll at {vw}px: ctrl={:?}, scroll={:?}",
                    ctrl.rect,
                    scroll.rect
                );
            }
        }
    }

    #[test]
    fn workflow_editor_presets_ssh_targets_and_tooltips_work() {
        let mut state = AppState::init_demo();
        state.open_edit_workflow_modal("wf-deploy-vps");
        if let ModalState::WorkflowEditor(d) = &state.modal {
            assert_eq!(d.server_id.as_deref(), Some("vps-prod"));
            assert_eq!(d.repo_id.as_deref(), Some("rb-erp"));
        } else {
            panic!("expected WorkflowEditor");
        }

        let mut rt = Runtime::new(state, ui::view, (1280.0, 820.0), Env::new(true, 1.0));
        rt.frame(0.0);
        rt.frame(1.0 / 60.0);

        let els = rt.elements();
        for key in [
            "ed-preset-node",
            "ed-preset-rust",
            "ed-preset-docker",
            "ed-preset-audit",
            "ed-trig-manual",
            "ed-trig-push",
            "ed-trig-folder",
            "ed-run-local",
            "ed-run-ssh",
            "ed-srv-default",
            "ed-srv-vps-prod",
            "ed-repo-all",
            "ed-repo-rb-erp",
            "ed-chain-none",
            "btn-add-step",
        ] {
            let el = els
                .iter()
                .find(|e| e.key.as_deref() == Some(key))
                .unwrap_or_else(|| panic!("expected WorkflowEditor control {key}"));
            assert!(
                el.help.as_deref().is_some_and(|h| !h.is_empty()),
                "expected tooltip (.help) on {key}"
            );
        }

        // Click each quick preset and verify real steps without `echo`
        for (key, expected_name, expected_first_cmd) in [
            ("ed-preset-node", "Node / pnpm CI", "pnpm install --frozen-lockfile"),
            ("ed-preset-rust", "Rust Cargo CI", "cargo check --all-targets"),
            ("ed-preset-docker", "Docker Compose Redeploy", "git pull --ff-only"),
            ("ed-preset-audit", "Auditoria & Smoke", "git status -s"),
        ] {
            let (px, py) = rt.find(key).unwrap_or_else(|| panic!("{key} exists"));
            rt.pointer_down(px, py);
            rt.pointer_up(px, py);
            rt.frame(1.0 / 60.0);
            if let ModalState::WorkflowEditor(d) = &rt.state.modal {
                assert_eq!(d.name.text, expected_name);
                assert_eq!(d.steps.len(), 3);
                assert_eq!(d.steps[0].run.text, expected_first_cmd);
                for st in &d.steps {
                    assert!(
                        !st.run.text.contains("echo "),
                        "preset {key} should not contain echo: {}",
                        st.run.text
                    );
                }
            } else {
                panic!("expected WorkflowEditor");
            }
        }

        // Select VPS Staging + repo rb-landing and save
        let (sx, sy) = rt
            .find("ed-srv-vps-staging")
            .expect("ed-srv-vps-staging exists");
        rt.pointer_down(sx, sy);
        rt.pointer_up(sx, sy);
        rt.frame(1.0 / 60.0);

        let (rx, ry) = rt
            .find("ed-repo-rb-landing")
            .expect("ed-repo-rb-landing exists");
        rt.pointer_down(rx, ry);
        rt.pointer_up(rx, ry);
        rt.frame(1.0 / 60.0);

        let (svx, svy) = rt.find("ed-save").expect("ed-save exists");
        rt.pointer_down(svx, svy);
        rt.pointer_up(svx, svy);
        rt.frame(1.0 / 60.0);

        assert!(matches!(rt.state.modal, ModalState::None));
        let saved = rt
            .state
            .snapshot
            .workflows
            .iter()
            .find(|w| w.id == "wf-deploy-vps")
            .unwrap();
        assert_eq!(saved.server_id.as_deref(), Some("vps-staging"));
        assert_eq!(saved.repo_id.as_deref(), Some("rb-landing"));
    }

    #[test]
    fn workflow_runs_pipeline_summary_and_local_vps_warning_work() {
        for vw in [360.0_f32, 768.0, 1440.0] {
            let mut state = AppState::init_demo();
            if vw < 500.0 {
                state.sidebar_collapsed = true;
            }
            // Add an active automatic VPS binding so the smart warning triggers on wf-ci-principal (Local + Push)
            state.snapshot.settings.repo_bindings.push(workploy::model::RepoBinding {
                id: "rb-auto-vps".into(),
                full_name: "widder/portal-cliente".into(),
                target_server_id: Some("vps-prod".into()),
                branch: "main".into(),
                trigger_mode: workploy::model::TriggerMode::PollOnVps,
                workflow_id: None,
                enabled: true,
                last_sha: String::new(),
                last_status: String::new(),
            });
            state.navigate_to(NavTarget::WorkflowRuns("wf-ci-principal".into()));

            let mut rt = Runtime::new(state, ui::view, (vw, 820.0), Env::new(true, 1.0));
            rt.frame(0.0);
            rt.frame(1.0 / 60.0);

            let els = rt.elements();
            for key in ["btn-run-wf", "btn-edit-wf", "btn-yaml-wf", "btn-del-wf"] {
                let el = els
                    .iter()
                    .find(|e| e.key.as_deref() == Some(key))
                    .unwrap_or_else(|| panic!("expected header button {key}"));
                assert!(
                    el.help.as_deref().is_some_and(|h| !h.is_empty()),
                    "expected tooltip on {key}"
                );
            }
            let summary = els
                .iter()
                .find(|e| e.key.as_deref() == Some("wf-pipeline-summary"))
                .expect("wf-pipeline-summary exists");
            let warn = els
                .iter()
                .find(|e| e.key.as_deref() == Some("wf-local-vps-warning"))
                .expect("wf-local-vps-warning exists");
            assert!(
                summary.rect[0] + summary.rect[2] <= vw + 0.5,
                "pipeline summary overflowed {vw}px: {:?}",
                summary.rect
            );
            assert!(
                warn.rect[0] + warn.rect[2] <= vw + 0.5,
                "warning banner overflowed {vw}px: {:?}",
                warn.rect
            );

            // Clicking "Editar fluxo" on warning banner opens WorkflowEditor
            let (ex, ey) = rt
                .find("btn-warn-edit-wf")
                .expect("btn-warn-edit-wf exists");
            rt.pointer_down(ex, ey);
            rt.pointer_up(ex, ey);
            rt.frame(1.0 / 60.0);
            assert!(matches!(rt.state.modal, ModalState::WorkflowEditor(_)));
        }
    }

    #[test]
    fn settings_webhook_binding_copy_buttons_work() {
        let mut state = AppState::init_demo();
        state.open_settings(SettingsTab::GitHub);
        let mut rt = Runtime::new(state, ui::view, (1280.0, 820.0), Env::new(true, 1.0));
        rt.frame(0.0);
        rt.frame(1.0 / 60.0);

        for _ in 0..8 {
            let Some((_, by)) = rt.find("btn-copy-wh-url-rb-erp") else {
                break;
            };
            if by > 120.0 && by < 780.0 {
                break;
            }
            rt.wheel(640.0, 400.0, -240.0);
            rt.frame(1.0 / 60.0);
        }

        // Only WebhookOnVps binding with VPS has copy buttons; PollOnDesktop does not
        assert!(rt.find("btn-copy-wh-url-rb-erp").is_some());
        assert!(rt.find("btn-copy-wh-sec-rb-erp").is_some());
        assert!(rt.find("btn-copy-wh-url-rb-landing").is_none());

        let els = rt.elements();
        for key in ["btn-copy-wh-url-rb-erp", "btn-copy-wh-sec-rb-erp"] {
            let el = els
                .iter()
                .find(|e| e.key.as_deref() == Some(key))
                .unwrap();
            let help = el.help.as_deref().unwrap_or("");
            assert!(
                help.contains("Webhooks") && help.contains("Sondagem 24/7 na VPS"),
                "expected GitHub Webhooks + zero-config hint in tooltip of {key}: {help}"
            );
        }

        let (ux, uy) = rt.find("btn-copy-wh-url-rb-erp").unwrap();
        rt.pointer_down(ux, uy);
        rt.pointer_up(ux, uy);
        rt.frame(1.0 / 60.0);
        assert!(
            rt.state
                .toasts
                .entries()
                .iter()
                .any(|t| t.options.title == "URL do Webhook copiada")
        );

        let (sx, sy) = rt.find("btn-copy-wh-sec-rb-erp").unwrap();
        rt.pointer_down(sx, sy);
        rt.pointer_up(sx, sy);
        rt.frame(1.0 / 60.0);
        assert!(
            rt.state
                .toasts
                .entries()
                .iter()
                .any(|t| t.options.title == "Segredo do Webhook copiado")
        );
    }

    #[test]
    fn vps_purge_busy_state_disables_buttons_and_updates_label() {
        let mut state = AppState::init_demo();
        state.snapshot.runs.clear();
        assert!(!state.has_active_runs());
        state.snapshot.purging_server_id = Some("vps-prod".into());
        assert!(state.has_active_runs());

        // Connections screen: vps-prod shows "Limpando...", vps-staging disabled too
        state.navigate_to(NavTarget::Connections);
        let mut rt_conn = Runtime::new(state, ui::view, (1280.0, 820.0), Env::new(true, 1.0));
        rt_conn.frame(0.0);
        rt_conn.frame(1.0 / 60.0);
        let els_conn = rt_conn.elements();
        let prod_conn = els_conn
            .iter()
            .find(|e| e.key.as_deref() == Some("conn-purge-vps-vps-prod"))
            .expect("conn-purge-vps-vps-prod exists");
        assert_eq!(prod_conn.label.as_deref(), Some("Limpando..."));
        assert!(!prod_conn.focusable);
        let stg_conn = els_conn
            .iter()
            .find(|e| e.key.as_deref() == Some("conn-purge-vps-vps-staging"))
            .expect("conn-purge-vps-vps-staging exists");
        assert_eq!(stg_conn.label.as_deref(), Some("Limpar agora"));
        assert!(!stg_conn.focusable);

        // Settings -> VPS & Motor tab: same disabled / "Limpando..." behavior
        rt_conn.update(|s, _| s.open_settings(SettingsTab::Vps));
        rt_conn.frame(1.0 / 60.0);
        let els_set = rt_conn.elements();
        let prod_set = els_set
            .iter()
            .find(|e| e.key.as_deref() == Some("btn-purge-vps-vps-prod"))
            .expect("btn-purge-vps-vps-prod exists");
        assert_eq!(prod_set.label.as_deref(), Some("Limpando..."));
        assert!(!prod_set.focusable);
        let stg_set = els_set
            .iter()
            .find(|e| e.key.as_deref() == Some("btn-purge-vps-vps-staging"))
            .expect("btn-purge-vps-vps-staging exists");
        assert_eq!(stg_set.label.as_deref(), Some("Limpar agora"));
        assert!(!stg_set.focusable);
    }
}
