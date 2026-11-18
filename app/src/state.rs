use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use rune::{TextEdit, Tx};
use rune_fx::Phase;
use rune_kit::{ToastEffect, ToastEvent, ToastStack};
use workploy::engine::{self, SharedHandle};
use workploy::github;
use workploy::model::{
    self, AuthMethod, CapacityTier, DetectedWorkflow, GithubRepoSummary, PlatformTarget,
    ProvisionOptions, PurgeRule, ReleaseInfo, RepoBinding, RepoCommitItem, Run, RunStatus,
    RunnerKind, Settings, Shared, Step, StepRun, StepStatus, Trigger, TriggerMode,
    VpsProbeResult, VpsServer, Workflow,
};
use workploy::vault;
use workploy::vps;
use workploy::yaml;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum NavTarget {
    Onboarding,
    Runs,
    WorkflowRuns(String),
    Approvals,
    Connections,
    Settings,
    RunDetail { run_id: String, tab: DetailTab },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SettingsTab {
    #[default]
    GitHub,
    Vps,
    Appearance,
    About,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DetailTab {
    Summary,
    Logs { job_idx: usize },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JobFilter {
    All,
    Failed,
    Active,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StatusFilter {
    All,
    Success,
    Failed,
    Active,
    Cancelled,
}

impl StatusFilter {
    pub const ALL: [StatusFilter; 5] = [
        StatusFilter::All,
        StatusFilter::Success,
        StatusFilter::Failed,
        StatusFilter::Active,
        StatusFilter::Cancelled,
    ];

    pub fn label(self) -> &'static str {
        match self {
            StatusFilter::All => "Todos os status",
            StatusFilter::Success => "Sucesso",
            StatusFilter::Failed => "Falhou",
            StatusFilter::Active => "Em andamento",
            StatusFilter::Cancelled => "Cancelado",
        }
    }

    pub fn from_label(s: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|v| v.label() == s)
            .unwrap_or(StatusFilter::All)
    }

    pub fn matches(self, st: RunStatus) -> bool {
        match self {
            StatusFilter::All => true,
            StatusFilter::Success => st == RunStatus::Success,
            StatusFilter::Failed => st == RunStatus::Failed,
            StatusFilter::Active => st.active(),
            StatusFilter::Cancelled => st == RunStatus::Cancelled,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WorkflowPreset {
    NodePnpmCi,
    RustCargoCi,
    DockerComposeRedeploy,
    AuditAndSmoke,
}

impl WorkflowPreset {
    pub const ALL: [WorkflowPreset; 4] = [
        WorkflowPreset::NodePnpmCi,
        WorkflowPreset::RustCargoCi,
        WorkflowPreset::DockerComposeRedeploy,
        WorkflowPreset::AuditAndSmoke,
    ];

    pub fn label(self) -> &'static str {
        match self {
            WorkflowPreset::NodePnpmCi => "Node / pnpm CI",
            WorkflowPreset::RustCargoCi => "Rust Cargo CI",
            WorkflowPreset::DockerComposeRedeploy => "Docker Compose Redeploy",
            WorkflowPreset::AuditAndSmoke => "Auditoria & Smoke",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            WorkflowPreset::NodePnpmCi => {
                "Preenche nome e 3 passos reais de CI Node/pnpm: install, lint + typecheck e build."
            }
            WorkflowPreset::RustCargoCi => {
                "Preenche nome e 3 passos reais de CI Rust: cargo check, clippy estrito e cargo test."
            }
            WorkflowPreset::DockerComposeRedeploy => {
                "Preenche nome e 3 passos reais de redeploy: git pull, docker compose build e up -d."
            }
            WorkflowPreset::AuditAndSmoke => {
                "Preenche nome e 3 passos reais de auditoria e smoke: git status, docker ps e curl /health."
            }
        }
    }

    pub fn steps(self) -> &'static [(&'static str, &'static str)] {
        match self {
            WorkflowPreset::NodePnpmCi => &[
                ("Instalar dependências", "pnpm install --frozen-lockfile"),
                ("Lint e checagem de tipos", "pnpm lint && pnpm typecheck"),
                ("Build de produção", "pnpm build"),
            ],
            WorkflowPreset::RustCargoCi => &[
                ("Verificar compilação", "cargo check --all-targets"),
                ("Análise estática (Clippy)", "cargo clippy --no-deps -- -D warnings"),
                ("Executar suíte de testes", "cargo test"),
            ],
            WorkflowPreset::DockerComposeRedeploy => &[
                ("Atualizar código-fonte", "git pull --ff-only"),
                ("Construir imagens", "docker compose build"),
                ("Subir containers", "docker compose up -d --remove-orphans"),
            ],
            WorkflowPreset::AuditAndSmoke => &[
                ("Verificar estado do repositório", "git status -s"),
                (
                    "Listar containers ativos",
                    r#"docker ps --format 'table {{.Names}}\t{{.Status}}'"#,
                ),
                (
                    "Verificar endpoint de saúde",
                    "curl -fsS http://127.0.0.1:3000/health || true",
                ),
            ],
        }
    }
}

#[derive(Clone, Debug)]
pub struct StepDraft {
    pub uid: u64,
    pub name: TextEdit,
    pub run: TextEdit,
    pub approval: bool,
}

#[derive(Clone, Debug)]
pub struct WorkflowDraft {
    pub id: Option<String>,
    pub name: TextEdit,
    pub trigger: Trigger,
    pub runner: RunnerKind,
    pub workdir: TextEdit,
    pub on_success: Option<String>,
    pub steps: Vec<StepDraft>,
    pub error: Option<String>,
}

impl WorkflowDraft {
    pub fn apply_preset(&mut self, preset: WorkflowPreset, next_uid: &mut u64) {
        self.name = TextEdit::new(preset.label());
        self.steps = preset
            .steps()
            .iter()
            .map(|(name, cmd)| {
                *next_uid += 1;
                StepDraft {
                    uid: *next_uid,
                    name: TextEdit::new(*name),
                    run: TextEdit::new(*cmd),
                    approval: false,
                }
            })
            .collect();
        self.error = None;
    }

    pub fn new_empty(next_uid: &mut u64) -> Self {
        *next_uid += 1;
        Self {
            id: None,
            name: TextEdit::new("Novo fluxo"),
            trigger: Trigger::Manual,
            runner: RunnerKind::Local,
            workdir: TextEdit::new(""),
            on_success: None,
            steps: vec![StepDraft {
                uid: *next_uid,
                name: TextEdit::new("Build e testes"),
                run: TextEdit::new("echo Executando build... && echo Tudo certo"),
                approval: false,
            }],
            error: None,
        }
    }

    pub fn from_workflow(wf: &Workflow, next_uid: &mut u64) -> Self {
        let steps = wf
            .steps
            .iter()
            .map(|st| {
                *next_uid += 1;
                StepDraft {
                    uid: *next_uid,
                    name: TextEdit::new(st.name.clone()),
                    run: TextEdit::new(st.run.clone()),
                    approval: st.approval,
                }
            })
            .collect();
        Self {
            id: Some(wf.id.clone()),
            name: TextEdit::new(wf.name.clone()),
            trigger: wf.trigger,
            runner: wf.runner,
            workdir: TextEdit::new(wf.workdir.clone()),
            on_success: wf.on_success.clone(),
            steps,
            error: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct YamlDraft {
    pub wf_id: Option<String>,
    pub wf_name: String,
    pub edit: TextEdit,
    pub error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ServerAuthChoice {
    SshConfigAlias,
    PrivateKey,
    PasswordInVault,
}

#[derive(Clone, Debug)]
pub struct ServerDraft {
    pub id: Option<String>,
    pub name: TextEdit,
    pub host: TextEdit,
    pub port: TextEdit,
    pub user: TextEdit,
    pub auth_choice: ServerAuthChoice,
    pub key_path: TextEdit,
    pub password: TextEdit,
    pub has_saved_password: bool,
    pub generated_pubkey: Option<String>,
    pub runner_port: TextEdit,
    pub error: Option<String>,
}

impl ServerDraft {
    pub fn new_empty() -> Self {
        let mut password = TextEdit::new("");
        password.set_secure(true);
        Self {
            id: None,
            name: TextEdit::new(""),
            host: TextEdit::new(""),
            port: TextEdit::new("22"),
            user: TextEdit::new("root"),
            auth_choice: ServerAuthChoice::SshConfigAlias,
            key_path: TextEdit::new(""),
            password,
            has_saved_password: false,
            generated_pubkey: None,
            runner_port: TextEdit::new("8787"),
            error: None,
        }
    }

    pub fn from_server(srv: &VpsServer, demo_mode: bool) -> Self {
        let mut password = TextEdit::new("");
        password.set_secure(true);
        let (auth_choice, key_path) = match &srv.auth {
            AuthMethod::SshConfigAlias => (ServerAuthChoice::SshConfigAlias, String::new()),
            AuthMethod::PrivateKey { path } => (ServerAuthChoice::PrivateKey, path.clone()),
            AuthMethod::PasswordInVault => (ServerAuthChoice::PasswordInVault, String::new()),
        };
        let has_saved_password = if demo_mode {
            matches!(srv.auth, AuthMethod::PasswordInVault)
        } else {
            vault::has_secret(&vault::vps_password_key(&srv.id))
        };
        Self {
            id: Some(srv.id.clone()),
            name: TextEdit::new(srv.name.clone()),
            host: TextEdit::new(srv.host.clone()),
            port: TextEdit::new(if srv.port == 0 { 22 } else { srv.port }.to_string()),
            user: TextEdit::new(srv.user.clone()),
            auth_choice,
            key_path: TextEdit::new(key_path),
            password,
            has_saved_password,
            generated_pubkey: None,
            runner_port: TextEdit::new(
                if srv.runner_port == 0 {
                    8787
                } else {
                    srv.runner_port
                }
                .to_string(),
            ),
            error: None,
        }
    }
}

pub fn platform_compat_warning(
    platform: PlatformTarget,
    target_server_id: Option<&str>,
) -> Option<&'static str> {
    match platform {
        PlatformTarget::Windows if target_server_id.is_some() => Some(
            "Este repositório exige runner Windows — VPS Linux não gera binário Win32/.exe nativo. Prefira 'Local (Esta máquina)' ou ajuste a plataforma.",
        ),
        PlatformTarget::Linux if target_server_id.is_none() && cfg!(target_os = "windows") => Some(
            "Este repositório exige ambiente Linux (Docker/systemd) — no Windows Desktop use uma VPS Linux ou Docker.",
        ),
        PlatformTarget::MacOS => Some(
            "Este repositório exige macOS (Xcode/Apple) — não há runner macOS conectado neste ambiente.",
        ),
        _ => None,
    }
}

#[derive(Clone, Debug)]
pub struct BindingDraft {
    pub id: Option<String>,
    pub full_name: TextEdit,
    pub branch: TextEdit,
    pub target_server_id: Option<String>,
    pub trigger_mode: TriggerMode,
    pub workflow_id: Option<String>,
    pub platform: PlatformTarget,
    pub enabled: bool,
    pub error: Option<String>,
}

impl BindingDraft {
    pub fn new_empty(default_server_id: Option<String>) -> Self {
        Self {
            id: None,
            full_name: TextEdit::new(""),
            branch: TextEdit::new("main"),
            target_server_id: default_server_id,
            trigger_mode: TriggerMode::PollOnVps,
            workflow_id: None,
            platform: PlatformTarget::CrossPlatform,
            enabled: true,
            error: None,
        }
    }

    pub fn from_repo(repo: &GithubRepoSummary, default_server_id: Option<String>) -> Self {
        let branch = if repo.default_branch.trim().is_empty() {
            "main".to_string()
        } else {
            repo.default_branch.clone()
        };
        let trigger_mode = if default_server_id.is_some() {
            TriggerMode::WebhookOnVps
        } else {
            TriggerMode::PollOnDesktop
        };
        Self {
            id: None,
            full_name: TextEdit::new(repo.full_name.clone()),
            branch: TextEdit::new(branch),
            target_server_id: default_server_id,
            trigger_mode,
            workflow_id: None,
            platform: PlatformTarget::CrossPlatform,
            enabled: true,
            error: None,
        }
    }

    pub fn from_binding(b: &RepoBinding) -> Self {
        Self {
            id: Some(b.id.clone()),
            full_name: TextEdit::new(b.full_name.clone()),
            branch: TextEdit::new(if b.branch.trim().is_empty() {
                "main".into()
            } else {
                b.branch.clone()
            }),
            target_server_id: b.target_server_id.clone(),
            trigger_mode: b.trigger_mode,
            workflow_id: b.workflow_id.clone(),
            platform: b.platform,
            enabled: b.enabled,
            error: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProvisionDraft {
    pub server_id: String,
    pub server_name: String,
    pub install_git: bool,
    pub install_docker: bool,
    pub install_node: bool,
    pub install_rust: bool,
    pub install_runner_service: bool,
    pub runner_port: TextEdit,
    pub error: Option<String>,
}

impl ProvisionDraft {
    pub fn for_server(srv: &VpsServer) -> Self {
        let port = if srv.runner_port == 0 {
            8787
        } else {
            srv.runner_port
        };
        Self {
            server_id: srv.id.clone(),
            server_name: srv.name.clone(),
            install_git: true,
            install_docker: true,
            install_node: true,
            install_rust: false,
            install_runner_service: true,
            runner_port: TextEdit::new(port.to_string()),
            error: None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PurgeSizeUnit {
    Mb,
    Gb,
}

#[derive(Clone, Debug)]
pub struct PurgeRuleDraft {
    pub enabled: bool,
    pub every_n_runs: TextEdit,
    pub size_value: TextEdit,
    pub size_unit: PurgeSizeUnit,
    pub clean_workspaces: bool,
    pub clean_docker: bool,
    pub error: Option<String>,
}

impl PurgeRuleDraft {
    pub fn from_rule(rule: &PurgeRule) -> Self {
        let (size_value, size_unit) = if rule.max_size_mb == 0 {
            ("0".to_string(), PurgeSizeUnit::Gb)
        } else if rule.max_size_mb >= 1024 && rule.max_size_mb.is_multiple_of(1024) {
            ((rule.max_size_mb / 1024).to_string(), PurgeSizeUnit::Gb)
        } else if rule.max_size_mb >= 1024 {
            let gb = rule.max_size_mb as f64 / 1024.0;
            (format!("{gb:.1}"), PurgeSizeUnit::Gb)
        } else {
            (rule.max_size_mb.to_string(), PurgeSizeUnit::Mb)
        };
        Self {
            enabled: rule.enabled,
            every_n_runs: TextEdit::new(rule.every_n_runs.to_string()),
            size_value: TextEdit::new(size_value),
            size_unit,
            clean_workspaces: rule.clean_workspaces,
            clean_docker: rule.clean_docker,
            error: None,
        }
    }

    pub fn set_unit(&mut self, unit: PurgeSizeUnit) {
        if self.size_unit == unit {
            return;
        }
        let raw = self.size_value.text.trim().replace(',', ".");
        if let Ok(v) = raw.parse::<f64>()
            && v.is_finite()
            && v >= 0.0
        {
            let converted = match (self.size_unit, unit) {
                (PurgeSizeUnit::Mb, PurgeSizeUnit::Gb) => {
                    let gb = v / 1024.0;
                    if (gb - gb.round()).abs() < 1e-4 {
                        format!("{}", gb.round() as u64)
                    } else {
                        format!("{gb:.1}")
                    }
                }
                (PurgeSizeUnit::Gb, PurgeSizeUnit::Mb) => {
                    format!("{}", (v * 1024.0).round() as u64)
                }
                _ => raw,
            };
            self.size_value = TextEdit::new(converted);
        }
        self.size_unit = unit;
    }

    pub fn parsed_max_size_mb(&self) -> Option<u64> {
        let raw = self.size_value.text.trim().replace(',', ".");
        let v = raw.parse::<f64>().ok()?;
        if !v.is_finite() || v < 0.0 {
            return None;
        }
        let mb = match self.size_unit {
            PurgeSizeUnit::Mb => v.round(),
            PurgeSizeUnit::Gb => (v * 1024.0).round(),
        };
        if !mb.is_finite() || mb > u64::MAX as f64 {
            return None;
        }
        Some(mb as u64)
    }
}

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum ModalState {
    None,
    WorkflowEditor(Box<WorkflowDraft>),
    YamlModal(Box<YamlDraft>),
    DeleteConfirm { wf_id: String, wf_name: String },
    ServerEditor(Box<ServerDraft>),
    DeleteServerConfirm { server_id: String, server_name: String },
    BindingEditor(Box<BindingDraft>),
    /// Importação de receita direto do repositório vinculado: o repo vira a
    /// fonte da verdade. `selected` guarda o caminho escolhido na lista.
    ImportFromRepo {
        binding_id: String,
        loading: bool,
        paths: Vec<String>,
        status: String,
        selected: Option<String>,
    },
    RepoCommits {
        binding_id: String,
        full_name: String,
        branch_input: TextEdit,
    },
    ProvisionModal(Box<ProvisionDraft>),
    PurgeRuleEditor(Box<PurgeRuleDraft>),
}

#[derive(Clone, Debug)]
pub struct SettingsDraft {
    pub ssh_host: TextEdit,
    pub github_repo: TextEdit,
    pub github_branch: TextEdit,
    /// Campo sempre inicializado vazio para nunca exibir token salvo em texto puro.
    pub github_token: TextEdit,
    pub reveal_token: bool,
    pub poll_enabled: bool,
    pub poll_secs: TextEdit,
    pub watch_enabled: bool,
    pub watch_folder: TextEdit,
}

impl SettingsDraft {
    pub fn from_settings(s: &Settings) -> Self {
        let mut token = TextEdit::new("");
        token.set_secure(true);
        Self {
            ssh_host: TextEdit::new(s.ssh_host.clone()),
            github_repo: TextEdit::new(s.github_repo.clone()),
            github_branch: TextEdit::new(s.github_branch.clone()),
            github_token: token,
            reveal_token: false,
            poll_enabled: s.poll_enabled,
            poll_secs: TextEdit::new(s.poll_secs.to_string()),
            watch_enabled: s.watch_enabled,
            watch_folder: TextEdit::new(s.watch_folder.clone()),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConnectionsHealth {
    Healthy,
    Warning,
    Danger,
}

/// Caminhos de receita encontrados no repositório por `fetch_repo_yaml_candidates`
/// (rodando fora da thread de UI) e esperando o `poll` levar para o modal
/// `ImportFromRepo` aberto.
#[derive(Clone, Debug)]
pub struct RepoYamlCandidates {
    pub binding_id: String,
    pub paths: Vec<String>,
    pub status: String,
}

pub fn vps_ram_used_pct(probe: &VpsProbeResult) -> u8 {
    if probe.ram_total_mb == 0 {
        return 0;
    }
    let used = probe.ram_total_mb.saturating_sub(probe.ram_free_mb);
    (used.saturating_mul(100) / probe.ram_total_mb).min(100) as u8
}

pub fn vps_health_status(srv: &VpsServer) -> (ConnectionsHealth, Vec<String>) {
    let Some(probe) = &srv.last_probe else {
        return (ConnectionsHealth::Healthy, Vec::new());
    };
    let mut alerts = Vec::new();
    let mut health = ConnectionsHealth::Healthy;

    if !probe.ok {
        health = ConnectionsHealth::Danger;
        let msg = if probe.verdict.trim().is_empty() {
            "Falha na última verificação SSH.".to_string()
        } else {
            probe.verdict.trim().to_string()
        };
        alerts.push(msg);
        return (health, alerts);
    }

    if probe.has_widder_runner && !probe.runner_active {
        health = ConnectionsHealth::Danger;
        alerts.push("Agente workploy-runner instalado, mas inativo.".to_string());
    }

    let ram_pct = vps_ram_used_pct(probe);
    if ram_pct >= 80 {
        if health != ConnectionsHealth::Danger {
            health = ConnectionsHealth::Warning;
        }
        alerts.push(format!("Uso de RAM elevado: {ram_pct}%"));
    }
    if probe.cpu_usage_pct >= 80 {
        if health != ConnectionsHealth::Danger {
            health = ConnectionsHealth::Warning;
        }
        alerts.push(format!("Uso de CPU elevado: {}%", probe.cpu_usage_pct));
    }
    if probe.disk_used_pct >= 80 {
        if health != ConnectionsHealth::Danger {
            health = ConnectionsHealth::Warning;
        }
        alerts.push(format!("Uso de disco elevado: {}%", probe.disk_used_pct));
    }

    (health, alerts)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Danger,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToastMsg {
    pub title: String,
    pub description: Option<String>,
    pub kind: ToastKind,
}

pub struct AppState {
    pub shared: SharedHandle,
    pub snapshot: Shared,
    pub wake_gen: Arc<AtomicU64>,
    pub last_wake_gen: u64,
    pub last_github_event_seq: u64,
    pub last_update_event_seq: u64,
    pub last_sync_event_seq: u64,
    pub demo_mode: bool,

    pub dark: bool,
    pub sidebar_collapsed: bool,
    pub connections_seen_healthy: bool,
    pub nav: NavTarget,
    pub previous_list_nav: NavTarget,
    pub selected_project_id: Option<String>,
    pub selected_workflow_id: Option<String>,
    pub settings_tab: SettingsTab,

    pub open_select: Option<String>,
    pub search_runs: TextEdit,
    pub filter_status: StatusFilter,
    pub filter_event: Option<String>,
    pub filter_branch: Option<String>,
    pub filter_actor: Option<String>,
    pub auto_refresh: bool,

    pub github_repo_search: TextEdit,
    pub github_repo_page: usize,

    pub job_filter: JobFilter,
    pub annotations_expanded: bool,
    pub log_search: TextEdit,
    pub log_line_numbers: bool,
    pub log_wrap_lines: bool,
    pub collapsed_steps: HashSet<String>,
    /// Projetos (RepoBinding.id) com fluxos aninhados recolhidos na sidebar.
    pub collapsed_projects: HashSet<String>,

    pub settings_draft: SettingsDraft,
    pub modal: ModalState,
    pub next_uid: u64,
    pub toasts: ToastStack<ToastMsg>,
    pub probe_queue: Vec<String>,
    /// Resultado (rede) da busca de receitas no repo esperando o `poll`
    /// drenar para dentro do modal `ImportFromRepo` aberto.
    pub repo_yaml_inbox: Arc<Mutex<Option<RepoYamlCandidates>>>,

    // Motion & rune-fx timing state
    pub clock_t: f64,
    pub fx_phase: Phase,
    pub reduce_motion: bool,
    pub last_live_tick: f64,
}

impl AppState {
    pub fn init_live() -> Self {
        let is_headless = rune::tooling::is_headless();
        let demo_arg = rune::tooling::extra_args()
            .iter()
            .any(|a| a == "--demo" || a == "-demo");
        if is_headless || demo_arg {
            return Self::init_demo();
        }
        let loaded = model::load_shared();
        let settings_draft = SettingsDraft::from_settings(&loaded.settings);
        let shared = Arc::new(Mutex::new(loaded.clone()));
        Self::from_parts(shared, loaded, settings_draft, false)
    }

    pub fn init_demo() -> Self {
        let mut s = Shared::default();
        crate::demo::populate_demo(&mut s);
        let settings_draft = SettingsDraft::from_settings(&s.settings);
        let shared = Arc::new(Mutex::new(s.clone()));
        Self::from_parts(shared, s, settings_draft, true)
    }

    fn from_parts(
        shared: SharedHandle,
        snapshot: Shared,
        settings_draft: SettingsDraft,
        demo_mode: bool,
    ) -> Self {
        let last_github_event_seq = snapshot
            .github_last_event
            .as_ref()
            .map(|e| e.0)
            .unwrap_or(0);
        let last_update_event_seq = snapshot
            .update_last_event
            .as_ref()
            .map(|e| e.0)
            .unwrap_or(0);
        let last_sync_event_seq = snapshot
            .sync_last_event
            .as_ref()
            .map(|e| e.0)
            .unwrap_or(0);
        let mut st = Self {
            shared,
            snapshot,
            wake_gen: Arc::new(AtomicU64::new(0)),
            last_wake_gen: 0,
            last_github_event_seq,
            last_update_event_seq,
            last_sync_event_seq,
            demo_mode,
            dark: true,
            sidebar_collapsed: false,
            connections_seen_healthy: false,
            nav: NavTarget::Runs,
            previous_list_nav: NavTarget::Runs,
            selected_project_id: None,
            selected_workflow_id: None,
            settings_tab: SettingsTab::GitHub,
            open_select: None,
            search_runs: TextEdit::new(""),
            filter_status: StatusFilter::All,
            filter_event: None,
            filter_branch: None,
            filter_actor: None,
            auto_refresh: true,
            github_repo_search: TextEdit::new(""),
            github_repo_page: 0,
            job_filter: JobFilter::All,
            annotations_expanded: false,
            log_search: TextEdit::new(""),
            log_line_numbers: true,
            log_wrap_lines: true,
            collapsed_steps: HashSet::new(),
            collapsed_projects: HashSet::new(),
            settings_draft,
            modal: ModalState::None,
            next_uid: 100,
            toasts: ToastStack::default(),
            probe_queue: Vec::new(),
            repo_yaml_inbox: Arc::new(Mutex::new(None)),
            clock_t: 0.0,
            fx_phase: Phase::new(1.0),
            reduce_motion: false,
            last_live_tick: 0.0,
        };
        if !demo_mode && st.should_show_onboarding() {
            st.nav = NavTarget::Onboarding;
        }
        st
    }

    pub fn should_show_onboarding(&self) -> bool {
        !self.snapshot.settings.onboarding_completed
    }

    pub fn connections_health(&self) -> ConnectionsHealth {
        let mut worst = ConnectionsHealth::Healthy;
        for srv in &self.snapshot.settings.servers {
            let (h, _) = vps_health_status(srv);
            match h {
                ConnectionsHealth::Danger => return ConnectionsHealth::Danger,
                ConnectionsHealth::Warning => worst = ConnectionsHealth::Warning,
                ConnectionsHealth::Healthy => {}
            }
        }
        worst
    }

    pub fn sync_connections_health_seen(&mut self) {
        if self.connections_health() != ConnectionsHealth::Healthy {
            self.connections_seen_healthy = false;
        }
    }

    pub fn sidebar_connections_indicator(&self) -> Option<ConnectionsHealth> {
        match self.connections_health() {
            ConnectionsHealth::Danger => Some(ConnectionsHealth::Danger),
            ConnectionsHealth::Warning => Some(ConnectionsHealth::Warning),
            ConnectionsHealth::Healthy => {
                if self.connections_seen_healthy {
                    None
                } else {
                    Some(ConnectionsHealth::Healthy)
                }
            }
        }
    }

    pub fn complete_onboarding(&mut self, tx: &mut Tx) {
        if self.demo_mode {
            self.snapshot.settings.onboarding_completed = true;
        } else {
            let mut guard = self.shared.lock().unwrap();
            guard.settings.onboarding_completed = true;
            model::save_settings(&guard);
            self.snapshot = guard.clone();
        }
        self.navigate_to(NavTarget::Runs);
        self.push_toast(
            "onboarding-done",
            "Configuração inicial concluída",
            Some("Você pode reabrir o guia quando quiser em Configurações → Sobre.".into()),
            ToastKind::Success,
            tx,
        );
    }

    pub fn reopen_onboarding(&mut self) {
        self.navigate_to(NavTarget::Onboarding);
    }

    pub fn toggle_minimize_to_tray(&mut self, tx: &mut Tx) {
        let next = !self.snapshot.settings.minimize_to_tray;
        if self.demo_mode {
            self.snapshot.settings.minimize_to_tray = next;
        } else {
            let mut guard = self.shared.lock().unwrap();
            guard.settings.minimize_to_tray = next;
            model::save_settings(&guard);
            self.snapshot = guard.clone();
        }
        self.push_toast(
            "tray-toggle",
            if next {
                "Minimizar para a bandeja ativado"
            } else {
                "Minimizar para a bandeja desativado"
            },
            Some(if next {
                "Ao fechar a janela, o Workploy continuará ativo na bandeja do sistema.".into()
            } else {
                "Ao fechar a janela, o aplicativo será encerrado.".into()
            }),
            ToastKind::Info,
            tx,
        );
    }

    pub fn open_settings(&mut self, tab: SettingsTab) {
        self.settings_tab = tab;
        self.navigate_to(NavTarget::Settings);
    }

    pub fn set_animations_enabled(&mut self, enabled: bool, clock_t: f32) {
        self.reduce_motion = !enabled;
        self.fx_phase.set_speed(clock_t, if enabled { 1.0 } else { 0.0 });
    }

    pub fn push_toast(
        &mut self,
        key: impl Into<String>,
        title: impl Into<String>,
        description: Option<String>,
        kind: ToastKind,
        tx: &mut Tx,
    ) {
        let now = tx.clock();
        let msg = ToastMsg {
            title: title.into(),
            description,
            kind,
        };
        let effects = self.toasts.step(ToastEvent::Shown {
            key: key.into(),
            options: msg,
            secs: Some(3.8),
            now,
        });
        for eff in effects {
            if let ToastEffect::WakeAt(due) = eff {
                tx.wake_at(due);
            }
        }
    }

    pub fn pending_approvals(&self) -> usize {
        self.snapshot
            .runs
            .iter()
            .filter(|r| r.status == RunStatus::AwaitingApproval)
            .count()
    }

    pub fn has_active_runs(&self) -> bool {
        self.snapshot.runs.iter().any(|r| r.status.active())
            || self.snapshot.ssh_testing
            || self.snapshot.probing_server_id.is_some()
            || !self.probe_queue.is_empty()
            || self.snapshot.provisioning_server_id.is_some()
            || self.snapshot.purging_server_id.is_some()
            || self.snapshot.github_testing
            || self.snapshot.update_checking
            || self.snapshot.update_downloading
            || self.snapshot.repo_branches_loading
            || self.snapshot.repo_workflows_loading
            || self.snapshot.repo_commits_loading
    }

    pub fn navigate_to(&mut self, target: NavTarget) {
        self.open_select = None;
        if let NavTarget::WorkflowRuns(ref id) = target {
            self.selected_workflow_id = Some(id.clone());
        }
        if !matches!(target, NavTarget::RunDetail { .. }) {
            self.previous_list_nav = target.clone();
        }
        if matches!(target, NavTarget::Connections) {
            self.connections_seen_healthy =
                self.connections_health() == ConnectionsHealth::Healthy;
        }
        self.nav = target;
    }

    pub fn project_workflows<'a>(&'a self, b: &RepoBinding) -> Vec<&'a Workflow> {
        let base_ids = engine::matching_workflows_for_binding_with_bindings(
            &self.snapshot.workflows,
            b,
            &self.snapshot.settings.repo_bindings,
        );
        let mut out: Vec<&'a Workflow> = Vec::new();
        for wid in base_ids {
            let mut cur_id = Some(wid.as_str());
            while let Some(cid) = cur_id {
                if out.iter().any(|w| w.id == cid) {
                    break;
                }
                if let Some(wf) = self.snapshot.workflows.iter().find(|w| w.id == cid) {
                    out.push(wf);
                    cur_id = wf.on_success.as_deref();
                } else {
                    break;
                }
            }
        }
        out
    }

    pub fn open_run_summary(&mut self, run_id: &str) {
        self.open_select = None;
        self.annotations_expanded = false;
        self.job_filter = JobFilter::All;
        self.nav = NavTarget::RunDetail {
            run_id: run_id.to_string(),
            tab: DetailTab::Summary,
        };
    }

    pub fn open_run_logs(&mut self, run_id: &str, job_idx: usize) {
        self.open_select = None;
        self.collapsed_steps.clear();
        self.nav = NavTarget::RunDetail {
            run_id: run_id.to_string(),
            tab: DetailTab::Logs { job_idx },
        };
    }

    pub fn server_label(&self, server_id: Option<&str>) -> String {
        match server_id {
            None => "Local (Esta máquina)".into(),
            Some(id) => self
                .snapshot
                .settings
                .servers
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "VPS removida".into()),
        }
    }

    pub fn is_remote_vps_run(&self, run: &Run) -> Option<String> {
        if !run.id.starts_with("run-vps-") {
            return None;
        }
        let wf_srv = self
            .snapshot
            .workflows
            .iter()
            .find(|w| w.id == run.workflow_id)
            .and_then(|w| w.server_id.clone());
        if let Some(sid) = wf_srv
            && self.snapshot.settings.servers.iter().any(|s| s.id == sid)
        {
            return Some(sid);
        }
        self.snapshot
            .settings
            .servers
            .iter()
            .find(|s| s.runner_installed)
            .map(|s| s.id.clone())
    }

    pub fn start_workflow(&mut self, wf_id: &str, trigger: &str, tx: &mut Tx) {
        if self.demo_mode {
            let wf = self
                .snapshot
                .workflows
                .iter()
                .find(|w| w.id == wf_id)
                .cloned();
            if let Some(wf) = wf {
                let number = self
                    .snapshot
                    .runs
                    .iter()
                    .filter(|r| r.workflow_id == wf.id)
                    .count() as u64
                    + 1;
                let now = model::now_ts();
                let mut steps: Vec<StepRun> = wf
                    .steps
                    .iter()
                    .enumerate()
                    .map(|(i, st)| StepRun {
                        name: st.name.clone(),
                        status: if i == 0 {
                            if st.approval {
                                StepStatus::WaitingApproval
                            } else {
                                StepStatus::Running
                            }
                        } else {
                            StepStatus::Pending
                        },
                        log: if i == 0 {
                            format!("Iniciando passo '{}'...\nExecutando: {}\n", st.name, st.run)
                        } else {
                            String::new()
                        },
                        secs: 0.0,
                        substeps: Vec::new(),
                        started_ts: if i == 0 { Some(now) } else { None },
                    })
                    .collect();
                let status = if steps
                    .first()
                    .is_some_and(|s| s.status == StepStatus::WaitingApproval)
                {
                    if let Some(s0) = steps.first_mut() {
                        s0.log.push_str(
                            "[portão de aprovação] aguardando liberação manual para este passo...\n",
                        );
                    }
                    RunStatus::AwaitingApproval
                } else {
                    RunStatus::Running
                };
                let run = Run {
                    id: model::new_id("run"),
                    workflow_id: wf.id.clone(),
                    workflow_name: wf.name.clone(),
                    status,
                    trigger_label: trigger.to_string(),
                    title: format!("{} ({})", wf.name, trigger),
                    number,
                    commit: None,
                    started_at: model::stamp_list(now),
                    started_ts: Some(now),
                    finished_at: None,
                    steps,
                    approved: false,
                    cancel_requested: false,
                    secs: 0.0,
                    actor: "maestro".into(),
                    repo_name: None,
                    branch: "main".into(),
                    summary: String::new(),
                    annotations: Vec::new(),
                    vps_run_id: None,
                    vps_server_id: None,
                };
                let rid = run.id.clone();
                self.snapshot.runs.insert(0, run);
                self.open_run_summary(&rid);
                self.push_toast(
                    format!("run-{rid}"),
                    format!("Fluxo '{}' iniciado", wf.name),
                    Some(format!("Execução #{number} aberta no modo demonstração.")),
                    ToastKind::Success,
                    tx,
                );
            }
            return;
        }
        if let Some(rid) = engine::start_run(&self.shared, wf_id, trigger) {
            self.snapshot = self.shared.lock().unwrap().clone();
            self.open_run_summary(&rid);
            self.push_toast(
                format!("run-{rid}"),
                "Fluxo iniciado",
                Some("Acompanhando execução em tempo real.".into()),
                ToastKind::Success,
                tx,
            );
        }
    }

    pub fn resolve_primary_branch(branch_expr: &str) -> String {
        let raw = branch_expr.trim();
        if raw.is_empty() || raw == "*" {
            "main".to_string()
        } else {
            raw.split(',')
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty() && *s != "*")
                .unwrap_or("main")
                .to_string()
        }
    }

    pub fn run_binding_now(&mut self, binding_id: &str, tx: &mut Tx) {
        let Some(binding) = self
            .snapshot
            .settings
            .repo_bindings
            .iter()
            .find(|b| b.id == binding_id)
            .cloned()
        else {
            return;
        };

        if self.demo_mode {
            let mut wf_ids = engine::matching_workflows_for_binding(&self.snapshot.workflows, &binding);
            if wf_ids.is_empty()
                && let Some(first) = self.snapshot.workflows.first()
            {
                wf_ids.push(first.id.clone());
            }
            let Some(wf_id) = wf_ids.into_iter().next() else {
                self.push_toast(
                    format!("run-bind-empty-{binding_id}"),
                    "Nenhuma receita configurada",
                    Some("Crie uma receita de fluxo antes de executar este projeto.".into()),
                    ToastKind::Warning,
                    tx,
                );
                return;
            };
            let Some(wf) = self
                .snapshot
                .workflows
                .iter()
                .find(|w| w.id == wf_id)
                .cloned()
            else {
                return;
            };
            let number = self
                .snapshot
                .runs
                .iter()
                .filter(|r| r.workflow_id == wf.id)
                .count() as u64
                + 1;
            let now = model::now_ts();
            let mut steps: Vec<StepRun> = wf
                .steps
                .iter()
                .enumerate()
                .map(|(i, st)| StepRun {
                    name: st.name.clone(),
                    status: if i == 0 {
                        if st.approval {
                            StepStatus::WaitingApproval
                        } else {
                            StepStatus::Running
                        }
                    } else {
                        StepStatus::Pending
                    },
                    log: if i == 0 {
                        format!(
                            "Projeto: {}\nIniciando passo '{}'...\nExecutando: {}\n",
                            binding.full_name, st.name, st.run
                        )
                    } else {
                        String::new()
                    },
                    secs: 0.0,
                    substeps: Vec::new(),
                    started_ts: if i == 0 { Some(now) } else { None },
                })
                .collect();
            let status = if steps
                .first()
                .is_some_and(|s| s.status == StepStatus::WaitingApproval)
            {
                if let Some(s0) = steps.first_mut() {
                    s0.log.push_str(
                        "[portão de aprovação] aguardando liberação manual para este passo...\n",
                    );
                }
                RunStatus::AwaitingApproval
            } else {
                RunStatus::Running
            };
            let branch = Self::resolve_primary_branch(&binding.branch);
            let run = Run {
                id: model::new_id("run"),
                workflow_id: wf.id.clone(),
                workflow_name: wf.name.clone(),
                status,
                trigger_label: "disparo manual por projeto".into(),
                title: format!("{} · {}", binding.full_name, wf.name),
                number,
                commit: None,
                started_at: model::stamp_list(now),
                started_ts: Some(now),
                finished_at: None,
                steps,
                approved: false,
                cancel_requested: false,
                secs: 0.0,
                actor: "maestro".into(),
                repo_name: Some(binding.full_name.clone()),
                branch,
                summary: String::new(),
                annotations: Vec::new(),
                vps_run_id: None,
                vps_server_id: None,
            };
            let rid = run.id.clone();
            self.snapshot.runs.insert(0, run);
            self.push_toast(
                format!("run-bind-{rid}"),
                "Execução iniciada",
                Some(format!(
                    "{} ({}) · receita '{}'",
                    binding.full_name,
                    self.server_label(binding.target_server_id.as_deref()),
                    wf.name
                )),
                ToastKind::Success,
                tx,
            );
            return;
        }

        if let Some(rid) =
            engine::start_run_for_binding(&self.shared, binding_id, "disparo manual por projeto")
        {
            self.snapshot = self.shared.lock().unwrap().clone();
            tx.wake_after(std::time::Duration::from_millis(120));
            self.push_toast(
                format!("run-bind-{rid}"),
                "Execução iniciada",
                Some(format!(
                    "Projeto '{}' em execução.",
                    binding.full_name
                )),
                ToastKind::Success,
                tx,
            );
        } else {
            self.push_toast(
                format!("run-bind-fail-{binding_id}"),
                "Nenhuma receita associada",
                Some("Selecione uma receita no vínculo ou configure um fluxo com gatilho de push.".into()),
                ToastKind::Warning,
                tx,
            );
        }
    }

    pub fn approve_run(&mut self, run_id: &str, tx: &mut Tx) {
        if self.demo_mode {
            if let Some(r) = self.snapshot.runs.iter_mut().find(|r| r.id == run_id) {
                r.approved = true;
                r.status = RunStatus::Success;
                r.finished_at = Some(model::now_str());
                r.secs = r.elapsed_secs().max(12.0);
                for st in &mut r.steps {
                    if matches!(
                        st.status,
                        StepStatus::WaitingApproval | StepStatus::Pending | StepStatus::Running
                    ) {
                        st.status = StepStatus::Success;
                        st.secs = st.secs.max(4.0);
                        st.log.push_str("\n[aprovado] continuando na VPS...\nVerificação de saúde 200 OK · deploy finalizado.\n");
                    }
                }
                if r.summary.is_empty() {
                    r.summary = format!(
                        "# {} aprovado e concluído\n- Servidor: VPS Produção\n- Serviços: api=200 | web=200 | worker=ok",
                        r.workflow_name
                    );
                }
            }
            self.push_toast(
                format!("approve-{run_id}"),
                "Portão aprovado",
                Some("Execução aprovada e concluída com sucesso.".into()),
                ToastKind::Success,
                tx,
            );
            return;
        }

        let remote_srv = self
            .snapshot
            .runs
            .iter()
            .find(|r| r.id == run_id)
            .and_then(|r| self.is_remote_vps_run(r));

        if let Some(server_id) = remote_srv {
            vps::remote_approve_or_cancel(&self.shared, &server_id, run_id, true);
            self.snapshot = self.shared.lock().unwrap().clone();
            self.push_toast(
                format!("approve-{run_id}"),
                "Aprovação enviada à VPS",
                Some("O runner remoto continuará o fluxo automaticamente.".into()),
                ToastKind::Success,
                tx,
            );
        } else {
            engine::approve(&self.shared, run_id);
            self.snapshot = self.shared.lock().unwrap().clone();
            self.push_toast(
                format!("approve-{run_id}"),
                "Aprovação enviada",
                Some("O fluxo continuará a partir do passo pendente.".into()),
                ToastKind::Success,
                tx,
            );
        }
    }

    pub fn cancel_or_reject_run(&mut self, run_id: &str, reject_label: bool, tx: &mut Tx) {
        if self.demo_mode {
            if let Some(r) = self.snapshot.runs.iter_mut().find(|r| r.id == run_id) {
                r.cancel_requested = true;
                r.status = RunStatus::Cancelled;
                r.finished_at = Some(model::now_str());
                r.secs = r.elapsed_secs().max(1.0);
                for st in &mut r.steps {
                    if matches!(
                        st.status,
                        StepStatus::WaitingApproval | StepStatus::Pending | StepStatus::Running
                    ) {
                        st.status = StepStatus::Skipped;
                        st.log.push_str("\n[cancelado pelo usuário]\n");
                    }
                }
            }
            self.push_toast(
                format!("cancel-{run_id}"),
                if reject_label {
                    "Execução rejeitada"
                } else {
                    "Execução cancelada"
                },
                None,
                ToastKind::Warning,
                tx,
            );
            return;
        }

        let remote_srv = self
            .snapshot
            .runs
            .iter()
            .find(|r| r.id == run_id)
            .and_then(|r| self.is_remote_vps_run(r));

        if let Some(server_id) = remote_srv {
            vps::remote_approve_or_cancel(&self.shared, &server_id, run_id, false);
        } else {
            engine::cancel(&self.shared, run_id);
        }
        self.snapshot = self.shared.lock().unwrap().clone();
        self.push_toast(
            format!("cancel-{run_id}"),
            if reject_label {
                "Execução rejeitada"
            } else {
                "Cancelamento solicitado"
            },
            None,
            ToastKind::Warning,
            tx,
        );
    }

    pub fn open_new_server_modal(&mut self) {
        self.open_select = None;
        self.modal = ModalState::ServerEditor(Box::new(ServerDraft::new_empty()));
    }

    pub fn open_edit_server_modal(&mut self, server_id: &str) {
        self.open_select = None;
        if let Some(srv) = self
            .snapshot
            .settings
            .servers
            .iter()
            .find(|s| s.id == server_id)
            .cloned()
        {
            self.modal =
                ModalState::ServerEditor(Box::new(ServerDraft::from_server(&srv, self.demo_mode)));
        }
    }

    pub fn generate_server_keypair(&mut self, tx: &mut Tx) {
        let ModalState::ServerEditor(draft) = &mut self.modal else {
            return;
        };
        let label = if draft.name.text.trim().is_empty() {
            "workploy-vps".to_string()
        } else {
            draft.name.text.trim().to_string()
        };
        if self.demo_mode {
            let demo_path = format!("C:\\Users\\maestro\\.ssh\\{}.ed25519", label.to_lowercase().replace(' ', "-"));
            draft.key_path = TextEdit::new(demo_path);
            draft.generated_pubkey =
                Some("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDemoChaveWorkploy workploy".into());
            draft.error = None;
            self.push_toast(
                "keygen-demo",
                "Par de chaves gerado",
                Some("Adicione a chave pública ao ~/.ssh/authorized_keys da VPS.".into()),
                ToastKind::Success,
                tx,
            );
            return;
        }
        match vps::generate_ed25519_keypair(&label) {
            Ok((pub_key, priv_path)) => {
                draft.key_path = TextEdit::new(priv_path);
                draft.generated_pubkey = Some(pub_key);
                draft.error = None;
                self.push_toast(
                    "keygen-ok",
                    "Chave Ed25519 gerada",
                    Some("Copie a chave pública abaixo para a sua VPS.".into()),
                    ToastKind::Success,
                    tx,
                );
            }
            Err(err) => {
                draft.error = Some(err);
            }
        }
    }

    pub fn save_server_draft(&mut self, tx: &mut Tx) {
        let ModalState::ServerEditor(draft) = &mut self.modal else {
            return;
        };
        let name = draft.name.text.trim().to_string();
        let host = draft.host.text.trim().to_string();
        if name.is_empty() {
            draft.error = Some("Informe um nome para identificar o servidor.".into());
            return;
        }
        if host.is_empty() {
            draft.error = Some("Informe o endereço IP, domínio ou alias SSH da VPS.".into());
            return;
        }
        let port = match draft.port.text.trim().parse::<u16>() {
            Ok(p) if p > 0 => p,
            _ => {
                draft.error = Some("Porta SSH inválida (use um número entre 1 e 65535).".into());
                return;
            }
        };
        let runner_port = match draft.runner_port.text.trim().parse::<u16>() {
            Ok(p) if p > 0 => p,
            _ => {
                draft.error = Some("Porta do agente inválida (padrão: 8787).".into());
                return;
            }
        };
        let user = draft.user.text.trim().to_string();
        let id = draft.id.clone().unwrap_or_else(|| model::new_id("vps"));

        let auth = match draft.auth_choice {
            ServerAuthChoice::SshConfigAlias => AuthMethod::SshConfigAlias,
            ServerAuthChoice::PrivateKey => {
                let path = draft.key_path.text.trim().to_string();
                if path.is_empty() {
                    draft.error = Some("Informe ou gere o caminho da chave privada SSH.".into());
                    return;
                }
                AuthMethod::PrivateKey { path }
            }
            ServerAuthChoice::PasswordInVault => {
                let pwd = draft.password.text.clone();
                if pwd.trim().is_empty() && !draft.has_saved_password {
                    draft.error = Some("Informe a senha SSH para salvar no Cofre do Windows.".into());
                    return;
                }
                if !pwd.trim().is_empty() && !self.demo_mode {
                    let key = vault::vps_password_key(&id);
                    if let Err(err) = vault::save_secret(&key, pwd.trim()) {
                        draft.error = Some(err);
                        return;
                    }
                }
                AuthMethod::PasswordInVault
            }
        };

        let existing = self
            .snapshot
            .settings
            .servers
            .iter()
            .find(|s| s.id == id)
            .cloned();

        let server = VpsServer {
            id: id.clone(),
            name: name.clone(),
            host,
            port,
            user,
            auth,
            last_probe: existing.as_ref().and_then(|e| e.last_probe.clone()),
            runner_installed: existing.as_ref().is_some_and(|e| e.runner_installed),
            runner_port,
            webhook_secret_key: existing.and_then(|e| e.webhook_secret_key),
        };

        if self.demo_mode {
            if let Some(slot) = self
                .snapshot
                .settings
                .servers
                .iter_mut()
                .find(|s| s.id == id)
            {
                *slot = server;
            } else {
                self.snapshot.settings.servers.push(server);
            }
        } else {
            let mut guard = self.shared.lock().unwrap();
            if let Some(slot) = guard.settings.servers.iter_mut().find(|s| s.id == id) {
                *slot = server;
            } else {
                guard.settings.servers.push(server);
            }
            model::save_settings(&guard);
            self.snapshot = guard.clone();
        }
        self.sync_connections_health_seen();
        self.modal = ModalState::None;
        self.push_toast(
            format!("srv-save-{id}"),
            "Servidor salvo",
            Some(format!("'{name}' pronto para inspeção e provisionamento.")),
            ToastKind::Success,
            tx,
        );
    }

    pub fn confirm_delete_server(&mut self, tx: &mut Tx) {
        let ModalState::DeleteServerConfirm {
            server_id,
            server_name,
        } = self.modal.clone()
        else {
            return;
        };
        if !self.demo_mode {
            let _ = vault::delete_secret(&vault::vps_password_key(&server_id));
            let _ = vault::delete_secret(&vault::vps_webhook_secret_key(&server_id));
            let _ = vault::delete_secret(&vault::vps_runner_token_key(&server_id));
            let mut guard = self.shared.lock().unwrap();
            guard.settings.servers.retain(|s| s.id != server_id);
            for b in &mut guard.settings.repo_bindings {
                if b.target_server_id.as_deref() == Some(server_id.as_str()) {
                    b.target_server_id = None;
                }
            }
            for wf in &mut guard.workflows {
                if wf.server_id.as_deref() == Some(server_id.as_str()) {
                    wf.server_id = None;
                }
            }
            model::save_settings(&guard);
            model::save_workflows(&guard);
            self.snapshot = guard.clone();
        } else {
            self.snapshot.settings.servers.retain(|s| s.id != server_id);
            for b in &mut self.snapshot.settings.repo_bindings {
                if b.target_server_id.as_deref() == Some(server_id.as_str()) {
                    b.target_server_id = None;
                }
            }
            for wf in &mut self.snapshot.workflows {
                if wf.server_id.as_deref() == Some(server_id.as_str()) {
                    wf.server_id = None;
                }
            }
        }
        self.sync_connections_health_seen();
        self.modal = ModalState::None;
        self.push_toast(
            format!("srv-del-{server_id}"),
            "Servidor removido",
            Some(format!("'{server_name}' e suas credenciais foram removidos.")),
            ToastKind::Info,
            tx,
        );
    }

    pub fn probe_server(&mut self, server_id: &str, tx: &mut Tx) {
        if self.demo_mode {
            if let Some(srv) = self
                .snapshot
                .settings
                .servers
                .iter_mut()
                .find(|s| s.id == server_id)
            {
                let vcpus = srv.last_probe.as_ref().map(|p| p.vcpus).unwrap_or(4).max(2);
                let ram = srv
                    .last_probe
                    .as_ref()
                    .map(|p| p.ram_total_mb)
                    .unwrap_or(8192)
                    .max(4096);
                let disk = srv
                    .last_probe
                    .as_ref()
                    .map(|p| p.disk_free_gb)
                    .unwrap_or(52)
                    .max(32);
                let (tier, verdict) = vps::compute_capacity_tier(vcpus, ram, disk);
                srv.last_probe = Some(VpsProbeResult {
                    ok: true,
                    os: "Ubuntu 24.04.1 LTS".into(),
                    arch: "x86_64".into(),
                    vcpus,
                    ram_total_mb: ram,
                    ram_free_mb: ram * 3 / 4,
                    disk_free_gb: disk,
                    cpu_usage_pct: 18,
                    disk_used_pct: 38,
                    has_git: true,
                    has_docker: true,
                    has_node: true,
                    has_pnpm: true,
                    has_rust: tier == CapacityTier::Standard || tier == CapacityTier::Heavy,
                    has_systemd: true,
                    has_widder_runner: srv.runner_installed,
                    runner_active: srv.runner_installed,
                    tier,
                    verdict,
                    checked_at: "agora mesmo".into(),
                });
                let srv_name = srv.name.clone();
                self.snapshot.ssh_test = Some((
                    true,
                    format!(
                        "Inspeção concluída em '{srv_name}': {} · {vcpus} vCPU · {:.1} GB RAM · {disk} GB livres",
                        tier.label(),
                        ram as f64 / 1024.0
                    ),
                ));
                self.push_toast(
                    format!("probe-{server_id}"),
                    "VPS inspecionada",
                    Some(format!("'{srv_name}' avaliada como {}.", tier.label())),
                    ToastKind::Success,
                    tx,
                );
            }
            self.sync_connections_health_seen();
            return;
        }
        vps::probe_vps(&self.shared, server_id);
        self.snapshot = self.shared.lock().unwrap().clone();
        tx.wake_after(std::time::Duration::from_millis(120));
    }

    pub fn probe_all_servers(&mut self, tx: &mut Tx) {
        if self.snapshot.settings.servers.is_empty() {
            self.push_toast(
                "probe-all-empty",
                "Nenhuma VPS cadastrada",
                Some("Cadastre servidores em Configurações → VPS & Motor para diagnosticar.".into()),
                ToastKind::Info,
                tx,
            );
            return;
        }
        if self.demo_mode {
            let count = self.snapshot.settings.servers.len();
            for srv in &mut self.snapshot.settings.servers {
                let vcpus = srv.last_probe.as_ref().map(|p| p.vcpus).unwrap_or(4).max(2);
                let ram = srv
                    .last_probe
                    .as_ref()
                    .map(|p| p.ram_total_mb)
                    .unwrap_or(8192)
                    .max(4096);
                let ram_free = srv
                    .last_probe
                    .as_ref()
                    .map(|p| p.ram_free_mb)
                    .filter(|&f| f > 0 && f <= ram)
                    .unwrap_or(ram * 3 / 4);
                let disk = srv
                    .last_probe
                    .as_ref()
                    .map(|p| p.disk_free_gb)
                    .unwrap_or(52)
                    .max(32);
                let cpu_pct = srv.last_probe.as_ref().map(|p| p.cpu_usage_pct).unwrap_or(18);
                let disk_pct = srv.last_probe.as_ref().map(|p| p.disk_used_pct).unwrap_or(38);
                let (tier, verdict) = vps::compute_capacity_tier(vcpus, ram, disk);
                srv.last_probe = Some(VpsProbeResult {
                    ok: true,
                    os: srv
                        .last_probe
                        .as_ref()
                        .map(|p| p.os.clone())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "Ubuntu 24.04.1 LTS".into()),
                    arch: srv
                        .last_probe
                        .as_ref()
                        .map(|p| p.arch.clone())
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "x86_64".into()),
                    vcpus,
                    ram_total_mb: ram,
                    ram_free_mb: ram_free,
                    disk_free_gb: disk,
                    cpu_usage_pct: cpu_pct,
                    disk_used_pct: disk_pct,
                    has_git: true,
                    has_docker: true,
                    has_node: true,
                    has_pnpm: srv.last_probe.as_ref().is_some_and(|p| p.has_pnpm),
                    has_rust: srv.last_probe.as_ref().is_some_and(|p| p.has_rust)
                        || tier == CapacityTier::Standard
                        || tier == CapacityTier::Heavy,
                    has_systemd: true,
                    has_widder_runner: srv.runner_installed,
                    runner_active: srv.runner_installed,
                    tier,
                    verdict,
                    checked_at: "agora mesmo".into(),
                });
            }
            self.snapshot.ssh_test = Some((
                true,
                format!("Diagnóstico concluído em {count} VPS(s) cadastrada(s)."),
            ));
            self.sync_connections_health_seen();
            self.push_toast(
                "probe-all-demo",
                "Todas as VPSs inspecionadas",
                Some(format!("{count} servidor(es) verificado(s) com sucesso.")),
                ToastKind::Success,
                tx,
            );
            return;
        }

        let ids: Vec<String> = self
            .snapshot
            .settings
            .servers
            .iter()
            .map(|s| s.id.clone())
            .collect();
        let active_id = self.snapshot.probing_server_id.clone();
        self.probe_queue.clear();

        if let Some(cur) = active_id {
            for id in ids {
                if id != cur && !self.probe_queue.contains(&id) {
                    self.probe_queue.push(id);
                }
            }
        } else if let Some((first, rest)) = ids.split_first() {
            self.probe_queue.extend(rest.iter().cloned());
            vps::probe_vps(&self.shared, first);
            self.snapshot = self.shared.lock().unwrap().clone();
        }
        tx.wake_after(std::time::Duration::from_millis(120));
    }

    pub fn open_provision_modal(&mut self, server_id: &str) {
        self.open_select = None;
        let Some(draft) = self
            .snapshot
            .settings
            .servers
            .iter()
            .find(|s| s.id == server_id)
            .map(ProvisionDraft::for_server)
        else {
            return;
        };
        if self.snapshot.provisioning_server_id.is_none() {
            if !self.demo_mode
                && let Ok(mut s) = self.shared.lock()
                && s.provisioning_server_id.is_none()
            {
                s.provision_log.clear();
            }
            self.snapshot.provision_log.clear();
        }
        self.modal = ModalState::ProvisionModal(Box::new(draft));
    }

    pub fn start_provision(&mut self, tx: &mut Tx) {
        let ModalState::ProvisionModal(draft) = &mut self.modal else {
            return;
        };
        let port = match draft.runner_port.text.trim().parse::<u16>() {
            Ok(p) if p > 0 => p,
            _ => {
                draft.error = Some("Informe uma porta válida (ex: 8787).".into());
                return;
            }
        };
        let server_id = draft.server_id.clone();
        let server_name = draft.server_name.clone();
        let opts = ProvisionOptions {
            install_git: draft.install_git,
            install_docker: draft.install_docker,
            install_node: draft.install_node,
            install_rust: draft.install_rust,
            install_runner_service: draft.install_runner_service,
            runner_port: port,
        };
        draft.error = None;

        if self.demo_mode {
            self.snapshot.provision_log = format!(
                "=== [1/5] Verificando sistema em '{server_name}' ===\n\
                 Ubuntu 24.04.1 LTS (systemd ativo)\n\
                 === [2/5] Conferindo Git ===\n\
                 git version 2.43.0\n\
                 === [3/5] Conferindo Docker ===\n\
                 Docker version 27.3.1 · Compose v2.29.7\n\
                 === [4/5] Conferindo Node.js e pnpm ===\n\
                 v22.11.0 · pnpm 9.14.2\n\
                 === [5/5] Instalando serviço workploy-runner (porta {port}) ===\n\
                 systemctl enable --now workploy-runner -> active (running)\n\
                 === Provisionamento concluído! ==="
            );
            if let Some(srv) = self
                .snapshot
                .settings
                .servers
                .iter_mut()
                .find(|s| s.id == server_id)
            {
                srv.runner_installed = opts.install_runner_service;
                srv.runner_port = port;
                if let Some(p) = &mut srv.last_probe {
                    p.has_git |= opts.install_git;
                    p.has_docker |= opts.install_docker;
                    p.has_node |= opts.install_node;
                    p.has_rust |= opts.install_rust;
                    p.has_widder_runner = opts.install_runner_service;
                    p.runner_active = opts.install_runner_service;
                }
            }
            self.snapshot.runner_sync_status =
                format!("'{server_name}' provisionada com workploy-runner ativo na porta {port}.");
            self.sync_connections_health_seen();
            self.push_toast(
                format!("prov-{server_id}"),
                "Motor provisionado",
                Some(format!("workploy-runner ativo em '{server_name}'.")),
                ToastKind::Success,
                tx,
            );
            return;
        }

        vps::provision_vps(&self.shared, &server_id, opts);
        self.snapshot = self.shared.lock().unwrap().clone();
        tx.wake_after(std::time::Duration::from_millis(120));
    }

    pub fn sync_server_runs(&mut self, server_id: &str, tx: &mut Tx) {
        if self.demo_mode {
            let name = self.server_label(Some(server_id));
            self.snapshot.runner_sync_status =
                format!("Execuções sincronizadas com '{name}' agora mesmo.");
            self.push_toast(
                format!("sync-{server_id}"),
                "Execuções sincronizadas",
                Some(format!("Histórico de '{name}' atualizado.")),
                ToastKind::Success,
                tx,
            );
            return;
        }
        vps::sync_vps_runs(&self.shared, server_id);
        self.snapshot = self.shared.lock().unwrap().clone();
        tx.wake_after(std::time::Duration::from_millis(120));
    }

    pub fn test_github_and_list(&mut self, tx: &mut Tx) {
        let entered = self.settings_draft.github_token.text.trim().to_string();
        if self.demo_mode {
            if !entered.is_empty() {
                self.snapshot.settings.github_token = entered;
                self.snapshot.settings.has_github_token = true;
                self.settings_draft.github_token.clear();
            }
            if !self.snapshot.settings.has_github_token {
                self.snapshot.github_status =
                    "Informe um Token (PAT) do GitHub para conectar.".into();
                self.push_toast(
                    "gh-missing",
                    "Token necessário",
                    Some("Cole seu Token Pessoal do GitHub para listar repositórios.".into()),
                    ToastKind::Warning,
                    tx,
                );
                return;
            }
            self.snapshot.settings.github_user = "matheuswidder".into();
            self.snapshot.github_status = format!(
                "Conectado como @matheuswidder · {} repositórios carregados",
                self.snapshot.github_repos.len()
            );
            self.push_toast(
                "gh-ok",
                "GitHub conectado",
                Some("Lista de repositórios atualizada com sucesso.".into()),
                ToastKind::Success,
                tx,
            );
            return;
        }

        let token_opt = if entered.is_empty() {
            None
        } else {
            self.settings_draft.github_token.clear();
            Some(entered)
        };
        github::test_and_list_repos(&self.shared, token_opt);
        self.snapshot = self.shared.lock().unwrap().clone();
        tx.wake_after(std::time::Duration::from_millis(120));
    }

    pub fn clear_github_token(&mut self, tx: &mut Tx) {
        self.settings_draft.github_token.clear();
        self.github_repo_page = 0;
        if self.demo_mode {
            self.snapshot.settings.github_token.clear();
            self.snapshot.settings.has_github_token = false;
            self.snapshot.settings.github_user.clear();
            self.snapshot.github_repos.clear();
            self.snapshot.github_last_event = None;
            self.snapshot.github_status = "Token removido da sessão.".into();
        } else {
            let _ = vault::delete_secret(vault::KEY_GITHUB_PAT);
            let mut guard = self.shared.lock().unwrap();
            guard.settings.github_token.clear();
            guard.settings.has_github_token = false;
            guard.settings.github_user.clear();
            guard.github_repos.clear();
            guard.github_last_event = None;
            guard.github_status = "Token removido do Cofre do Windows.".into();
            model::save_settings(&guard);
            self.snapshot = guard.clone();
        }
        self.push_toast(
            "gh-cleared",
            "Token removido",
            Some("Credencial apagada com segurança do Cofre do Windows.".into()),
            ToastKind::Info,
            tx,
        );
    }

    pub fn toggle_binding_branch(branch_edit: &mut TextEdit, branch_name: &str) {
        let target = branch_name.trim();
        if target.is_empty() {
            return;
        }
        let current = branch_edit.text.trim();
        if current.is_empty() || current == "*" {
            *branch_edit = TextEdit::new(target);
            return;
        }
        let mut items: Vec<String> = current
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "*")
            .map(str::to_string)
            .collect();
        if let Some(pos) = items.iter().position(|x| x == target) {
            items.remove(pos);
        } else {
            items.push(target.to_string());
        }
        *branch_edit = TextEdit::new(items.join(","));
    }

    pub fn fetch_binding_branches(&mut self, tx: &mut Tx) {
        let repo = match &self.modal {
            ModalState::BindingEditor(d) => d.full_name.text.trim().to_string(),
            _ => return,
        };
        if self.demo_mode {
            self.snapshot.repo_branches_loading = false;
            if repo.is_empty() || !repo.contains('/') {
                self.snapshot.repo_branches_repo.clear();
                self.snapshot.repo_branches.clear();
                self.snapshot.repo_branches_status =
                    "Informe o repositório (dono/repo) para buscar branches.".into();
                return;
            }
            self.snapshot.repo_branches_repo = repo.clone();
            self.snapshot.repo_branches = vec!["main".into(), "dev".into(), "staging".into()];
            self.snapshot.repo_branches_status =
                format!("3 branch(es) encontrada(s) em {repo}.");
            return;
        }
        github::fetch_repo_branches(&self.shared, repo);
        self.snapshot = self.shared.lock().unwrap().clone();
        tx.wake_after(std::time::Duration::from_millis(120));
    }

    pub fn scan_binding_workflows(&mut self, tx: &mut Tx) {
        let repo = match &self.modal {
            ModalState::BindingEditor(d) => d.full_name.text.trim().to_string(),
            _ => return,
        };
        if self.demo_mode {
            self.snapshot.repo_workflows_loading = false;
            if repo.is_empty() || !repo.contains('/') {
                self.snapshot.repo_workflows_repo.clear();
                self.snapshot.repo_workflows.clear();
                self.snapshot.repo_workflows_status =
                    "Informe o repositório (dono/repo) para buscar fluxos.".into();
                return;
            }
            self.snapshot.repo_workflows_repo = repo.clone();
            self.snapshot.repo_workflows = vec![
                DetectedWorkflow {
                    path: ".github/workflows/ci.yml".into(),
                    name: "CI".into(),
                    kind: "github-actions".into(),
                },
                DetectedWorkflow {
                    path: ".github/workflows/deploy.yml".into(),
                    name: "Deploy VPS".into(),
                    kind: "github-actions".into(),
                },
                DetectedWorkflow {
                    path: "Dockerfile".into(),
                    name: "Dockerfile".into(),
                    kind: "docker".into(),
                },
            ];
            let lower = repo.to_ascii_lowercase();
            let det_plat = if lower.contains("workploy") || lower.contains("win") {
                PlatformTarget::Windows
            } else if lower.contains("macos") || lower.contains("xcode") {
                PlatformTarget::MacOS
            } else {
                PlatformTarget::Linux
            };
            self.snapshot.detected_platform = Some((repo.clone(), det_plat));
            if let ModalState::BindingEditor(d) = &mut self.modal
                && det_plat != PlatformTarget::CrossPlatform
            {
                d.platform = det_plat;
            }
            self.snapshot.repo_workflows_status =
                "2 fluxo(s) encontrado(s) em .github/workflows + 1 arquivo(s) na raiz".into();
            return;
        }
        if let Ok(mut guard) = self.shared.lock() {
            guard.detected_platform = None;
        }
        self.snapshot.detected_platform = None;
        github::scan_repo_workflows(&self.shared, repo);
        self.snapshot = self.shared.lock().unwrap().clone();
        tx.wake_after(std::time::Duration::from_millis(120));
    }

    pub fn generate_workflow_for_binding(&mut self, binding_id: &str, tx: &mut Tx) {
        let repo_name = self
            .snapshot
            .settings
            .repo_bindings
            .iter()
            .find(|b| b.id == binding_id)
            .map(|b| b.full_name.clone())
            .unwrap_or_else(|| "repositório".to_string());

        if self.demo_mode {
            self.push_toast(
                format!("gen-wf-demo-{binding_id}"),
                "Receita simulada do repositório",
                Some(format!(
                    "Análise simulada de '{repo_name}': no modo real lê package.json, Cargo.toml e docker-compose.yml para criar passos automáticos."
                )),
                ToastKind::Info,
                tx,
            );
            return;
        }

        github::generate_workflow_from_repo(&self.shared, binding_id.to_string());
        self.snapshot = self.shared.lock().unwrap().clone();
        tx.wake_after(std::time::Duration::from_millis(120));
        self.push_toast(
            format!("gen-wf-{binding_id}"),
            "Gerando receita do repositório",
            Some("Analisando repositório e gerando receita...".into()),
            ToastKind::Info,
            tx,
        );
    }

    /// Resolve o vínculo dono de uma receita: o `repo_id` gravado, o repo da
    /// proveniência (`source.repo`) ou o vínculo que aponta direto para ela.
    pub fn binding_id_for_workflow(&self, wf: &workploy::model::Workflow) -> Option<String> {
        if let Some(rid) = wf.repo_id.as_deref().filter(|r| !r.is_empty())
            && self
                .snapshot
                .settings
                .repo_bindings
                .iter()
                .any(|b| b.id == rid)
        {
            return Some(rid.to_string());
        }
        if let Some(src) = &wf.source {
            let repo = src.repo.trim();
            if !repo.is_empty()
                && let Some(b) = self
                    .snapshot
                    .settings
                    .repo_bindings
                    .iter()
                    .find(|b| b.full_name.eq_ignore_ascii_case(repo))
            {
                return Some(b.id.clone());
            }
        }
        self.snapshot
            .settings
            .repo_bindings
            .iter()
            .find(|b| b.workflow_id.as_deref() == Some(wf.id.as_str()))
            .map(|b| b.id.clone())
    }

    /// Abre o modal "Importar do repositório" e dispara em background a busca
    /// dos caminhos de receita no repo vinculado (`.workploy.yml` na raiz ou
    /// `infra/workploy/*.yml`). O resultado chega pelo `repo_yaml_inbox` e o
    /// `poll` move para dentro do modal aberto.
    pub fn open_import_from_repo_modal(&mut self, binding_id: &str, tx: &mut Tx) {
        self.open_select = None;
        let Some(binding) = self
            .snapshot
            .settings
            .repo_bindings
            .iter()
            .find(|b| b.id == binding_id)
            .cloned()
        else {
            return;
        };
        let repo_label = binding.full_name.trim().to_string();
        if self.demo_mode {
            // Mesmos caminhos que o backend prioriza, sem tocar na rede.
            self.modal = ModalState::ImportFromRepo {
                binding_id: binding.id.clone(),
                loading: false,
                paths: vec![".workploy.yml".into(), "infra/workploy/deploy.yml".into()],
                status: format!(
                    "2 arquivos de receita encontrados em {repo_label} (modo demonstração)."
                ),
                selected: Some(".workploy.yml".into()),
            };
            return;
        }
        self.modal = ModalState::ImportFromRepo {
            binding_id: binding.id.clone(),
            loading: true,
            paths: Vec::new(),
            status: format!("Buscando receitas em {repo_label}..."),
            selected: None,
        };
        let sh = self.shared.clone();
        let inbox = self.repo_yaml_inbox.clone();
        let bid = binding.id;
        std::thread::spawn(move || {
            let paths = github::fetch_repo_yaml_candidates(&sh, &repo_label);
            let status = if paths.is_empty() {
                "Nenhum .workploy.yml ou infra/workploy/*.yml encontrado — commit o arquivo e tente de novo.".to_string()
            } else {
                format!("{n} arquivo(s) de receita encontrado(s) em {repo_label}.", n = paths.len())
            };
            *inbox.lock().unwrap() = Some(RepoYamlCandidates {
                binding_id: bid,
                paths,
                status,
            });
            // Acorda o loop de frames (o mesmo efeito de `engine::notify`).
            let wake = sh.lock().unwrap().wake.clone();
            if let Some(wake) = wake {
                wake();
            }
        });
        tx.wake_after(std::time::Duration::from_millis(400));
    }

    /// Importa (ou atualiza) a receita a partir do caminho escolhido no modal.
    /// O backend devolve a mensagem pt-BR e também a publica em
    /// `sync_last_event` — o `poll` transforma no toast padrão da sincronização.
    pub fn import_recipe_from_repo(&mut self, binding_id: &str, path: &str, tx: &mut Tx) {
        let Some(binding) = self
            .snapshot
            .settings
            .repo_bindings
            .iter()
            .find(|b| b.id == binding_id)
            .cloned()
        else {
            return;
        };
        let clean_path = path.trim().to_string();
        self.modal = ModalState::None;
        if self.demo_mode {
            let repo_label = binding.full_name.trim();
            let msg = format!(
                "Receita importada de {repo_label}:{clean_path} (modo demonstração — nada foi gravado)."
            );
            self.push_toast(
                format!("imp-repo-{binding_id}"),
                "Receita importada do repositório",
                Some(msg),
                ToastKind::Success,
                tx,
            );
            return;
        }
        let sh = self.shared.clone();
        let bid = binding_id.to_string();
        std::thread::spawn(move || {
            // A mensagem retornada também vai para `runner_sync_status` e
            // `sync_last_event`; o toast nasce do evento de sincronização.
            let _ = github::import_workflow_from_repo(&sh, &bid, &clean_path);
        });
        tx.wake_after(std::time::Duration::from_millis(150));
    }

    /// "Atualizar do repo": reimporta a receita do caminho registrado em
    /// `source`, preservando id, encadeamento e vínculos (o backend decide
    /// entre atualizar ou recriar) e limpando o aviso de drift quando dá certo.
    pub fn update_workflow_from_repo(&mut self, wf_id: &str, tx: &mut Tx) {
        let Some(wf) = self
            .snapshot
            .workflows
            .iter()
            .find(|w| w.id == wf_id)
            .cloned()
        else {
            return;
        };
        let Some(source) = wf.source.clone() else {
            self.push_toast(
                format!("upd-repo-missing-{wf_id}"),
                "Receita sem origem no repo",
                Some(
                    "Esta receita não foi importada do repositório — não há de onde atualizar."
                        .into(),
                ),
                ToastKind::Warning,
                tx,
            );
            return;
        };
        let Some(binding_id) = self.binding_id_for_workflow(&wf) else {
            self.push_toast(
                format!("upd-repo-nobinding-{wf_id}"),
                "Vínculo não encontrado",
                Some(format!(
                    "Nenhum vínculo aponta para o repositório '{}'.",
                    source.repo.trim()
                )),
                ToastKind::Warning,
                tx,
            );
            return;
        };
        if self.demo_mode {
            let repo_label = source.repo.trim();
            let msg = format!(
                "Receita '{}' atualizada a partir de {repo_label}:{} (modo demonstração).",
                wf.name,
                source.path.trim()
            );
            self.snapshot.workflow_drift.retain(|(id, _, _)| id != wf_id);
            self.push_toast(
                format!("upd-repo-{wf_id}"),
                "Receita atualizada do repo",
                Some(msg),
                ToastKind::Success,
                tx,
            );
            return;
        }
        let sh = self.shared.clone();
        let bid = binding_id;
        let path = source.path.trim().to_string();
        std::thread::spawn(move || {
            let _ = github::import_workflow_from_repo(&sh, &bid, &path);
        });
        tx.wake_after(std::time::Duration::from_millis(150));
    }

    pub fn clear_finished_runs_history(&mut self, tx: &mut Tx) {
        let removed = if self.demo_mode {
            let before = self.snapshot.runs.len();
            self.snapshot.runs.retain(|r| r.status.active());
            before.saturating_sub(self.snapshot.runs.len())
        } else {
            let removed = engine::clear_finished_runs(&self.shared);
            self.snapshot = self.shared.lock().unwrap().clone();
            removed
        };

        if removed > 0 {
            self.push_toast(
                "clear-runs-ok",
                "Histórico limpo",
                Some(format!("Histórico limpo ({removed} execuções removidas).")),
                ToastKind::Success,
                tx,
            );
        } else {
            self.push_toast(
                "clear-runs-empty",
                "Histórico sem alterações",
                Some("Nenhuma execução finalizada para limpar.".into()),
                ToastKind::Info,
                tx,
            );
        }
    }

    pub fn open_repo_commits_modal(&mut self, binding_id: &str, tx: &mut Tx) {
        let Some(binding) = self
            .snapshot
            .settings
            .repo_bindings
            .iter()
            .find(|b| b.id == binding_id)
            .cloned()
        else {
            return;
        };
        let branch = Self::resolve_primary_branch(&binding.branch);
        self.open_select = None;
        self.modal = ModalState::RepoCommits {
            binding_id: binding.id,
            full_name: binding.full_name,
            branch_input: TextEdit::new(branch),
        };
        self.fetch_repo_commits_for_modal(tx);
    }

    pub fn fetch_repo_commits_for_modal(&mut self, tx: &mut Tx) {
        let (full_name, branch) = match &mut self.modal {
            ModalState::RepoCommits {
                full_name,
                branch_input,
                ..
            } => {
                let br = branch_input.text.trim();
                let resolved = if br.is_empty() {
                    "main".to_string()
                } else {
                    br.to_string()
                };
                if br.is_empty() {
                    *branch_input = TextEdit::new(resolved.clone());
                }
                (full_name.trim().to_string(), resolved)
            }
            _ => return,
        };

        if self.demo_mode {
            self.snapshot.repo_commits_loading = false;
            if full_name.is_empty() || !full_name.contains('/') {
                self.snapshot.repo_commits_repo.clear();
                self.snapshot.repo_commits_branch.clear();
                self.snapshot.repo_commits.clear();
                self.snapshot.repo_commits_status =
                    "Informe o repositório (dono/repo) para buscar commits.".into();
                return;
            }
            let commits = if full_name.eq_ignore_ascii_case("widder/landing") && branch == "main" {
                vec![
                    RepoCommitItem {
                        sha: "f3e8d21a9c4b5e6f708192a3b4c5d6e7f8a9b0c1".into(),
                        short_sha: "f3e8d21".into(),
                        message: "feat(seo): melhora metadados OpenGraph e schema JSON-LD".into(),
                        author: "ana.silva".into(),
                        date: "2026-11-18T15:40:00Z".into(),
                        branch: branch.clone(),
                    },
                    RepoCommitItem {
                        sha: "c06c3c1d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b".into(),
                        short_sha: "c06c3c1".into(),
                        message: "feat(painel): consolida interface OpenCode em Rune com pulso discreto".into(),
                        author: "matheuswidder".into(),
                        date: "2026-11-18T12:15:00Z".into(),
                        branch: branch.clone(),
                    },
                    RepoCommitItem {
                        sha: "b21c440f8e7d6c5b4a3928172635445362718090".into(),
                        short_sha: "b21c440".into(),
                        message: "style(hero): ajusta espaçamento responsivo em telas 360px".into(),
                        author: "matheuswidder".into(),
                        date: "2026-11-17T19:05:00Z".into(),
                        branch: branch.clone(),
                    },
                ]
            } else if branch == "main" {
                vec![
                    RepoCommitItem {
                        sha: "f3e8d21a9c4b5e6f708192a3b4c5d6e7f8a9b0c1".into(),
                        short_sha: "f3e8d21".into(),
                        message: "feat(faturamento): adiciona conciliação automática de boletos".into(),
                        author: "matheuswidder".into(),
                        date: "2026-11-18T16:10:00Z".into(),
                        branch: branch.clone(),
                    },
                    RepoCommitItem {
                        sha: "a8f4c12f9b3c4d5e6f708192a3b4c5d6e7f8091a".into(),
                        short_sha: "a8f4c12".into(),
                        message: "release(v2.14.0): prepara pacote de produção após CI #691".into(),
                        author: "matheuswidder".into(),
                        date: "2026-11-18T14:25:00Z".into(),
                        branch: branch.clone(),
                    },
                    RepoCommitItem {
                        sha: "d41a8e9c7b6a5f4e3d2c1b0a9f8e7d6c5b4a3210".into(),
                        short_sha: "d41a8e9".into(),
                        message: "fix(api): corrige timeout no webhook de confirmação PIX".into(),
                        author: "ana.silva".into(),
                        date: "2026-11-17T21:40:00Z".into(),
                        branch: branch.clone(),
                    },
                ]
            } else {
                vec![
                    RepoCommitItem {
                        sha: "9c12ab34cd56ef78901234567890abcdef123456".into(),
                        short_sha: "9c12ab3".into(),
                        message: format!("feat({branch}): prepara ajustes de homologação na branch {branch}"),
                        author: "matheuswidder".into(),
                        date: "2026-11-18T15:10:00Z".into(),
                        branch: branch.clone(),
                    },
                    RepoCommitItem {
                        sha: "5e78cd90ef12ab34567890abcdef1234567890ab".into(),
                        short_sha: "5e78cd9".into(),
                        message: format!("fix({branch}): valida migrations em ambiente isolado"),
                        author: "ana.silva".into(),
                        date: "2026-11-18T10:30:00Z".into(),
                        branch: branch.clone(),
                    },
                    RepoCommitItem {
                        sha: "3a45bc67de89fa0123456789abcdef0123456789".into(),
                        short_sha: "3a45bc6".into(),
                        message: "chore: atualiza dependências do workspace".into(),
                        author: "matheuswidder".into(),
                        date: "2026-11-17T17:50:00Z".into(),
                        branch: branch.clone(),
                    },
                ]
            };
            self.snapshot.repo_commits_repo = full_name.clone();
            self.snapshot.repo_commits_branch = branch.clone();
            self.snapshot.repo_commits_status = format!(
                "{} commit(s) recente(s) em {} ({}).",
                commits.len(),
                full_name,
                branch
            );
            self.snapshot.repo_commits = commits;
            return;
        }

        github::fetch_repo_commits(&self.shared, full_name, branch);
        self.snapshot = self.shared.lock().unwrap().clone();
        tx.wake_after(std::time::Duration::from_millis(120));
    }

    pub fn run_binding_commit_now(
        &mut self,
        binding_id: &str,
        sha: &str,
        message: &str,
        branch: &str,
        tx: &mut Tx,
    ) {
        let Some(binding) = self
            .snapshot
            .settings
            .repo_bindings
            .iter()
            .find(|b| b.id == binding_id)
            .cloned()
        else {
            return;
        };
        let short: String = sha.chars().take(7).collect();
        let clean_branch = if branch.trim().is_empty() {
            Self::resolve_primary_branch(&binding.branch)
        } else {
            branch.trim().to_string()
        };

        if self.demo_mode {
            let mut wf_ids =
                engine::matching_workflows_for_binding(&self.snapshot.workflows, &binding);
            if wf_ids.is_empty()
                && let Some(first) = self.snapshot.workflows.first()
            {
                wf_ids.push(first.id.clone());
            }
            let Some(wf_id) = wf_ids.into_iter().next() else {
                self.push_toast(
                    format!("run-commit-empty-{binding_id}"),
                    "Nenhuma receita configurada",
                    Some("Crie uma receita de fluxo antes de executar este projeto.".into()),
                    ToastKind::Warning,
                    tx,
                );
                return;
            };
            let Some(wf) = self
                .snapshot
                .workflows
                .iter()
                .find(|w| w.id == wf_id)
                .cloned()
            else {
                return;
            };
            let number = self
                .snapshot
                .runs
                .iter()
                .filter(|r| r.workflow_id == wf.id)
                .count() as u64
                + 1;
            let now = model::now_ts();
            let mut steps: Vec<StepRun> = wf
                .steps
                .iter()
                .enumerate()
                .map(|(i, st)| StepRun {
                    name: st.name.clone(),
                    status: if i == 0 {
                        if st.approval {
                            StepStatus::WaitingApproval
                        } else {
                            StepStatus::Running
                        }
                    } else {
                        StepStatus::Pending
                    },
                    log: if i == 0 {
                        format!(
                            "Projeto: {} ({}@{})\nIniciando passo '{}'...\nExecutando: {}\n",
                            binding.full_name, clean_branch, short, st.name, st.run
                        )
                    } else {
                        String::new()
                    },
                    secs: 0.0,
                    substeps: Vec::new(),
                    started_ts: if i == 0 { Some(now) } else { None },
                })
                .collect();
            let status = if steps
                .first()
                .is_some_and(|s| s.status == StepStatus::WaitingApproval)
            {
                if let Some(s0) = steps.first_mut() {
                    s0.log.push_str(
                        "[portão de aprovação] aguardando liberação manual para este passo...\n",
                    );
                }
                RunStatus::AwaitingApproval
            } else {
                RunStatus::Running
            };
            let trigger_label = if short.is_empty() {
                format!("commit em {clean_branch}")
            } else {
                format!("commit {short} ({clean_branch})")
            };
            let title = message
                .lines()
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(&trigger_label)
                .to_string();
            let run = Run {
                id: model::new_id("run"),
                workflow_id: wf.id.clone(),
                workflow_name: wf.name.clone(),
                status,
                trigger_label,
                title,
                number,
                commit: Some(sha.to_string()),
                started_at: model::stamp_list(now),
                started_ts: Some(now),
                finished_at: None,
                steps,
                approved: false,
                cancel_requested: false,
                secs: 0.0,
                actor: "maestro".into(),
                repo_name: Some(binding.full_name.clone()),
                branch: clean_branch,
                summary: String::new(),
                annotations: Vec::new(),
                vps_run_id: None,
                vps_server_id: None,
            };
            let rid = run.id.clone();
            self.snapshot.runs.insert(0, run);
            self.modal = ModalState::None;
            self.navigate_to(NavTarget::Runs);
            self.push_toast(
                format!("run-commit-{rid}"),
                format!("Execução iniciada para commit {short}"),
                Some(format!(
                    "{} ({}) · receita '{}'",
                    binding.full_name,
                    self.server_label(binding.target_server_id.as_deref()),
                    wf.name
                )),
                ToastKind::Success,
                tx,
            );
            return;
        }

        if let Some(rid) =
            engine::start_run_for_binding_commit(&self.shared, binding_id, sha, message, &clean_branch)
        {
            self.snapshot = self.shared.lock().unwrap().clone();
            self.modal = ModalState::None;
            self.navigate_to(NavTarget::Runs);
            tx.wake_after(std::time::Duration::from_millis(120));
            self.push_toast(
                format!("run-commit-{rid}"),
                format!("Execução iniciada para commit {short}"),
                Some(format!(
                    "Projeto '{}' ({}) em execução.",
                    binding.full_name, clean_branch
                )),
                ToastKind::Success,
                tx,
            );
        } else {
            self.push_toast(
                format!("run-commit-fail-{binding_id}"),
                "Nenhuma receita associada",
                Some("Selecione uma receita no vínculo ou configure um fluxo com gatilho de push.".into()),
                ToastKind::Warning,
                tx,
            );
        }
    }

    pub fn open_new_binding_modal(&mut self, prefill: Option<&GithubRepoSummary>) {
        self.open_select = None;
        let default_srv = self
            .snapshot
            .settings
            .servers
            .first()
            .map(|s| s.id.clone());
        let draft = match prefill {
            Some(repo) => BindingDraft::from_repo(repo, default_srv),
            None => BindingDraft::new_empty(default_srv),
        };
        self.modal = ModalState::BindingEditor(Box::new(draft));
    }

    pub fn open_edit_binding_modal(&mut self, binding_id: &str) {
        self.open_select = None;
        if let Some(b) = self
            .snapshot
            .settings
            .repo_bindings
            .iter()
            .find(|x| x.id == binding_id)
        {
            self.modal = ModalState::BindingEditor(Box::new(BindingDraft::from_binding(b)));
        }
    }

    pub fn save_binding_draft(&mut self, tx: &mut Tx) {
        let ModalState::BindingEditor(draft) = &mut self.modal else {
            return;
        };
        let full_name = draft.full_name.text.trim().to_string();
        if full_name.is_empty() || !full_name.contains('/') {
            draft.error = Some("Informe o repositório no formato dono/repositorio.".into());
            return;
        }
        let branch = {
            let b = draft.branch.text.trim();
            if b.is_empty() {
                "main".to_string()
            } else {
                b.to_string()
            }
        };
        let id = draft.id.clone().unwrap_or_else(|| model::new_id("repo"));
        let prev = self
            .snapshot
            .settings
            .repo_bindings
            .iter()
            .find(|x| x.id == id)
            .cloned();

        let binding = RepoBinding {
            id: id.clone(),
            full_name: full_name.clone(),
            target_server_id: draft.target_server_id.clone(),
            branch,
            trigger_mode: draft.trigger_mode,
            workflow_id: draft.workflow_id.clone(),
            enabled: draft.enabled,
            last_sha: prev.as_ref().map(|p| p.last_sha.clone()).unwrap_or_default(),
            last_status: prev
                .as_ref()
                .map(|p| p.last_status.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Aguardando sincronização".into()),
            platform: draft.platform,
        };

        let sync_server_id = binding.target_server_id.clone();

        if self.demo_mode {
            if let Some(slot) = self
                .snapshot
                .settings
                .repo_bindings
                .iter_mut()
                .find(|x| x.id == id)
            {
                *slot = binding;
            } else {
                self.snapshot.settings.repo_bindings.push(binding);
            }
        } else {
            {
                let mut guard = self.shared.lock().unwrap();
                if let Some(slot) = guard
                    .settings
                    .repo_bindings
                    .iter_mut()
                    .find(|x| x.id == id)
                {
                    *slot = binding;
                } else {
                    guard.settings.repo_bindings.push(binding);
                }
                model::save_settings(&guard);
            }
            if let Some(sid) = sync_server_id {
                github::sync_repo_bindings_to_vps(&self.shared, &sid);
                tx.wake_after(std::time::Duration::from_millis(120));
            }
            self.snapshot = self.shared.lock().unwrap().clone();
        }
        self.modal = ModalState::None;
        self.push_toast(
            format!("bind-save-{id}"),
            "Vínculo salvo",
            Some(format!("Repositório '{full_name}' configurado.")),
            ToastKind::Success,
            tx,
        );
    }

    pub fn delete_binding(&mut self, binding_id: &str, tx: &mut Tx) {
        if self.selected_project_id.as_deref() == Some(binding_id) {
            self.selected_project_id = None;
        }
        if self.demo_mode {
            self.snapshot
                .settings
                .repo_bindings
                .retain(|b| b.id != binding_id);
        } else {
            let mut guard = self.shared.lock().unwrap();
            guard
                .settings
                .repo_bindings
                .retain(|b| b.id != binding_id);
            model::save_settings(&guard);
            self.snapshot = guard.clone();
        }
        self.push_toast(
            format!("bind-del-{binding_id}"),
            "Vínculo removido",
            None,
            ToastKind::Info,
            tx,
        );
    }

    pub fn sync_bindings_to_vps(&mut self, server_id: &str, tx: &mut Tx) {
        let srv_name = self.server_label(Some(server_id));
        if self.demo_mode {
            self.snapshot.runner_sync_status =
                format!("Regras e repositórios sincronizados em '{srv_name}'.");
            self.push_toast(
                format!("sync-bind-{server_id}"),
                "Sincronizado na VPS",
                Some(format!("Configuração aplicada em '{srv_name}'.")),
                ToastKind::Success,
                tx,
            );
            return;
        }
        github::sync_repo_bindings_to_vps(&self.shared, server_id);
        self.snapshot = self.shared.lock().unwrap().clone();
        tx.wake_after(std::time::Duration::from_millis(120));
    }

    pub fn copy_binding_webhook_url(&mut self, server_id: &str, tx: &mut Tx) {
        let Some(srv) = self
            .snapshot
            .settings
            .servers
            .iter()
            .find(|s| s.id == server_id)
        else {
            self.push_toast(
                format!("copy-wh-url-missing-{server_id}"),
                "VPS não encontrada",
                Some("Selecione uma VPS válida neste vínculo antes de copiar a URL.".into()),
                ToastKind::Warning,
                tx,
            );
            return;
        };
        let (url, _, unresolved_alias) = vps::webhook_endpoint_and_secret(srv);
        tx.copy(url.clone());
        if unresolved_alias {
            self.push_toast(
                format!("copy-wh-url-{server_id}"),
                "URL do Webhook copiada (alias SSH não resolvido)",
                Some(format!(
                    "{url} — o host configurado é um alias SSH que o GitHub não alcança. Use o IP/domínio público da VPS ou troque para 'Sondagem 24/7 na VPS'."
                )),
                ToastKind::Warning,
                tx,
            );
        } else {
            self.push_toast(
                format!("copy-wh-url-{server_id}"),
                "URL do Webhook copiada",
                Some(format!(
                    "{url} — cole em Settings → Webhooks no repositório GitHub."
                )),
                ToastKind::Success,
                tx,
            );
        }
    }

    pub fn copy_binding_webhook_secret(&mut self, server_id: &str, tx: &mut Tx) {
        let Some(srv) = self
            .snapshot
            .settings
            .servers
            .iter()
            .find(|s| s.id == server_id)
        else {
            self.push_toast(
                format!("copy-wh-sec-missing-{server_id}"),
                "VPS não encontrada",
                Some("Selecione uma VPS válida neste vínculo antes de copiar o segredo.".into()),
                ToastKind::Warning,
                tx,
            );
            return;
        };
        let (_, secret, _) = vps::webhook_endpoint_and_secret(srv);
        let resolved = if let Some(sec) = secret {
            Some(sec)
        } else if self.demo_mode && srv.webhook_secret_key.is_some() {
            Some("whsec_demo_workploy_secret_8787".to_string())
        } else {
            None
        };
        if let Some(sec) = resolved {
            tx.copy(sec);
            self.push_toast(
                format!("copy-wh-sec-{server_id}"),
                "Segredo do Webhook copiado",
                Some("Cole no campo Secret ao criar o webhook em Settings → Webhooks no GitHub.".into()),
                ToastKind::Success,
                tx,
            );
        } else {
            self.push_toast(
                format!("copy-wh-sec-none-{server_id}"),
                "Segredo ainda não configurado",
                Some("Provisione o workploy-runner ou sincronize esta VPS para gerar o segredo no Cofre.".into()),
                ToastKind::Warning,
                tx,
            );
        }
    }

    pub fn open_purge_rule_modal(&mut self) {
        self.open_select = None;
        let draft = PurgeRuleDraft::from_rule(&self.snapshot.settings.purge_rule);
        self.modal = ModalState::PurgeRuleEditor(Box::new(draft));
    }

    pub fn save_purge_rule_draft(&mut self, tx: &mut Tx) {
        let ModalState::PurgeRuleEditor(draft) = &mut self.modal else {
            return;
        };
        let every_n_runs = match draft.every_n_runs.text.trim().parse::<u32>() {
            Ok(n) => n,
            Err(_) => {
                draft.error = Some(
                    "Informe um número inteiro válido (0 ou maior) para a frequência de execuções."
                        .into(),
                );
                return;
            }
        };
        let Some(max_size_mb) = draft.parsed_max_size_mb() else {
            draft.error = Some("Informe um limite de espaço válido (ex: 5 GB ou 5120 MB).".into());
            return;
        };

        let rule = PurgeRule {
            enabled: draft.enabled,
            every_n_runs,
            max_size_mb,
            clean_workspaces: draft.clean_workspaces,
            clean_docker: draft.clean_docker,
        };

        let installed_servers: Vec<String> = self
            .snapshot
            .settings
            .servers
            .iter()
            .filter(|srv| srv.runner_installed)
            .map(|srv| srv.id.clone())
            .collect();

        if self.demo_mode {
            self.snapshot.settings.purge_rule = rule;
            if !installed_servers.is_empty() {
                self.snapshot.runner_sync_status = format!(
                    "Regra de limpeza sincronizada com {} VPS(s) ativa(s).",
                    installed_servers.len()
                );
            }
        } else {
            {
                let mut guard = self.shared.lock().unwrap();
                guard.settings.purge_rule = rule;
                model::save_settings(&guard);
            }
            for sid in &installed_servers {
                github::sync_repo_bindings_to_vps(&self.shared, sid);
            }
            self.snapshot = self.shared.lock().unwrap().clone();
            if !installed_servers.is_empty() {
                tx.wake_after(std::time::Duration::from_millis(120));
            }
        }

        self.modal = ModalState::None;
        let desc = if installed_servers.is_empty() {
            "Salva localmente. Será aplicada ao provisionar uma VPS.".to_string()
        } else {
            format!(
                "Sincronizando regra com {} servidor(es) com runner ativo.",
                installed_servers.len()
            )
        };
        self.push_toast(
            "purge-rule-saved",
            "Regra de limpeza salva",
            Some(desc),
            ToastKind::Success,
            tx,
        );
    }

    pub fn purge_server(&mut self, server_id: &str, tx: &mut Tx) {
        let srv_name = self.server_label(Some(server_id));
        if self.demo_mode {
            self.snapshot.runner_sync_status =
                format!("Limpeza concluída em '{srv_name}': 1420.0 MB liberados.");
            self.push_toast(
                format!("purge-{server_id}"),
                "Limpeza concluída",
                Some(format!("Espaço em disco liberado em '{srv_name}'.")),
                ToastKind::Success,
                tx,
            );
            return;
        }
        vps::trigger_vps_purge(&self.shared, server_id);
        self.snapshot = self.shared.lock().unwrap().clone();
        self.push_toast(
            format!("purge-{server_id}"),
            "Limpeza iniciada na VPS",
            Some(format!("Executando limpeza em '{srv_name}'...")),
            ToastKind::Info,
            tx,
        );
        tx.wake_after(std::time::Duration::from_millis(120));
    }

    pub fn save_settings(&mut self, tx: &mut Tx) {
        let secs = self
            .settings_draft
            .poll_secs
            .text
            .trim()
            .parse::<u64>()
            .unwrap_or(60)
            .max(15);

        let entered_tok = self.settings_draft.github_token.text.trim().to_string();
        let prev = &self.snapshot.settings;
        let (github_token, has_github_token) = if !entered_tok.is_empty() {
            if !self.demo_mode {
                let _ = vault::save_secret(vault::KEY_GITHUB_PAT, &entered_tok);
            }
            self.settings_draft.github_token.clear();
            (entered_tok, true)
        } else {
            (prev.github_token.clone(), prev.has_github_token)
        };

        let new_settings = Settings {
            ssh_host: self.settings_draft.ssh_host.text.trim().to_string(),
            github_repo: self.settings_draft.github_repo.text.trim().to_string(),
            github_branch: {
                let b = self.settings_draft.github_branch.text.trim();
                if b.is_empty() {
                    "main".into()
                } else {
                    b.into()
                }
            },
            github_token,
            has_github_token,
            poll_enabled: self.settings_draft.poll_enabled,
            poll_secs: secs,
            watch_enabled: self.settings_draft.watch_enabled,
            watch_folder: self.settings_draft.watch_folder.text.trim().to_string(),
            servers: prev.servers.clone(),
            repo_bindings: prev.repo_bindings.clone(),
            github_user: prev.github_user.clone(),
            purge_rule: prev.purge_rule.clone(),
            onboarding_completed: prev.onboarding_completed,
            minimize_to_tray: prev.minimize_to_tray,
        };
        if !self.demo_mode {
            let mut guard = self.shared.lock().unwrap();
            guard.settings = new_settings;
            model::save_settings(&guard);
            self.snapshot = guard.clone();
        } else {
            self.snapshot.settings = new_settings;
        }
        self.push_toast(
            "settings-saved",
            "Configurações salvas",
            Some("Preferências de Servidores, GitHub e Vigia atualizadas.".into()),
            ToastKind::Success,
            tx,
        );
    }

    pub fn open_new_workflow_modal(&mut self) {
        self.open_select = None;
        let draft = WorkflowDraft::new_empty(&mut self.next_uid);
        self.modal = ModalState::WorkflowEditor(Box::new(draft));
    }

    pub fn open_edit_workflow_modal(&mut self, wf_id: &str) {
        self.open_select = None;
        if let Some(wf) = self.snapshot.workflows.iter().find(|w| w.id == wf_id).cloned() {
            let draft = WorkflowDraft::from_workflow(&wf, &mut self.next_uid);
            self.modal = ModalState::WorkflowEditor(Box::new(draft));
        }
    }

    pub fn open_yaml_modal(&mut self, wf_id: Option<&str>) {
        self.open_select = None;
        let (id, name, content) = match wf_id
            .and_then(|id| self.snapshot.workflows.iter().find(|w| w.id == id))
        {
            Some(wf) => (Some(wf.id.clone()), wf.name.clone(), yaml::export_yaml(wf)),
            None => (
                None,
                "Novo fluxo".into(),
                "name: Novo fluxo\non: [push]\n\njobs:\n  build:\n    runs-on: local\n    steps:\n      - name: Verificar\n        run: echo ok\n".into(),
            ),
        };
        self.modal = ModalState::YamlModal(Box::new(YamlDraft {
            wf_id: id,
            wf_name: name,
            edit: TextEdit::new(content),
            error: None,
        }));
    }

    pub fn save_workflow_draft(&mut self, tx: &mut Tx) {
        let ModalState::WorkflowEditor(draft) = &mut self.modal else {
            return;
        };
        let name = draft.name.text.trim().to_string();
        if name.is_empty() {
            draft.error = Some("Informe um nome para o fluxo.".into());
            return;
        }
        let steps: Vec<Step> = draft
            .steps
            .iter()
            .filter(|st| !st.name.text.trim().is_empty() || !st.run.text.trim().is_empty())
            .map(|st| Step {
                name: {
                    let n = st.name.text.trim();
                    if n.is_empty() {
                        "Passo".into()
                    } else {
                        n.into()
                    }
                },
                run: st.run.text.clone(),
                approval: st.approval,
            })
            .collect();
        if steps.is_empty() {
            draft.error = Some("Adicione pelo menos um passo válido.".into());
            return;
        }
        let wf = Workflow {
            id: draft.id.clone().unwrap_or_else(|| model::new_id("wf")),
            name: name.clone(),
            trigger: draft.trigger,
            runner: draft.runner,
            workdir: draft.workdir.text.trim().to_string(),
            steps,
            on_success: draft.on_success.clone(),
            server_id: None,
            repo_id: None,
            // Editar uma receita não pode apagar a proveniência de importação
            // do repositório (mantida quando a receita já existia).
            source: draft
                .id
                .as_deref()
                .and_then(|id| self.snapshot.workflows.iter().find(|w| w.id == id))
                .and_then(|w| w.source.clone()),
        };
        let wf_id = wf.id.clone();
        if self.demo_mode {
            if let Some(existing) = self.snapshot.workflows.iter_mut().find(|w| w.id == wf_id) {
                *existing = wf;
            } else {
                self.snapshot.workflows.push(wf);
            }
        } else {
            let installed_servers: Vec<String> = {
                let mut guard = self.shared.lock().unwrap();
                if let Some(existing) = guard.workflows.iter_mut().find(|w| w.id == wf_id) {
                    *existing = wf;
                } else {
                    guard.workflows.push(wf);
                }
                model::save_workflows(&guard);
                guard
                    .settings
                    .servers
                    .iter()
                    .filter(|srv| srv.runner_installed)
                    .map(|srv| srv.id.clone())
                    .collect()
            };
            for sid in &installed_servers {
                github::sync_repo_bindings_to_vps(&self.shared, sid);
            }
            self.snapshot = self.shared.lock().unwrap().clone();
            if !installed_servers.is_empty() {
                tx.wake_after(std::time::Duration::from_millis(120));
            }
        }
        self.modal = ModalState::None;
        self.open_select = None;
        self.push_toast(
            format!("wf-save-{wf_id}"),
            "Fluxo salvo",
            Some(format!("'{name}' atualizado com sucesso.")),
            ToastKind::Success,
            tx,
        );
    }

    pub fn apply_yaml_draft(&mut self, tx: &mut Tx) {
        let ModalState::YamlModal(draft) = &mut self.modal else {
            return;
        };
        let target_id = draft
            .wf_id
            .clone()
            .unwrap_or_else(|| model::new_id("wf"));
        match yaml::import_yaml(&draft.edit.text, target_id.clone()) {
            Ok(wf) => {
                let wf_name = wf.name.clone();
                if self.demo_mode {
                    if let Some(existing) =
                        self.snapshot.workflows.iter_mut().find(|w| w.id == target_id)
                    {
                        *existing = wf;
                    } else {
                        self.snapshot.workflows.push(wf);
                    }
                } else {
                    let mut guard = self.shared.lock().unwrap();
                    if let Some(existing) = guard.workflows.iter_mut().find(|w| w.id == target_id) {
                        *existing = wf;
                    } else {
                        guard.workflows.push(wf);
                    }
                    model::save_workflows(&guard);
                    self.snapshot = guard.clone();
                }
                self.modal = ModalState::None;
                self.push_toast(
                    format!("yaml-ok-{target_id}"),
                    "YAML importado",
                    Some(format!("Fluxo '{wf_name}' atualizado a partir do YAML.")),
                    ToastKind::Success,
                    tx,
                );
            }
            Err(err) => {
                draft.error = Some(err);
            }
        }
    }

    pub fn confirm_delete_workflow(&mut self, tx: &mut Tx) {
        let ModalState::DeleteConfirm { wf_id, wf_name } = self.modal.clone() else {
            return;
        };
        if self.demo_mode {
            self.snapshot.workflows.retain(|w| w.id != wf_id);
        } else {
            let mut guard = self.shared.lock().unwrap();
            guard.workflows.retain(|w| w.id != wf_id);
            model::save_workflows(&guard);
            self.snapshot = guard.clone();
        }
        if self.selected_workflow_id.as_deref() == Some(wf_id.as_str()) {
            self.selected_workflow_id = None;
        }
        if matches!(&self.nav, NavTarget::WorkflowRuns(id) if id == &wf_id) {
            self.navigate_to(NavTarget::Runs);
        }
        self.modal = ModalState::None;
        self.push_toast(
            format!("wf-del-{wf_id}"),
            "Fluxo excluído",
            Some(format!("'{wf_name}' foi removido.")),
            ToastKind::Info,
            tx,
        );
    }

    pub fn check_for_updates(&mut self, tx: &mut Tx) {
        if self.demo_mode {
            let current = env!("CARGO_PKG_VERSION");
            let info = ReleaseInfo {
                tag_name: "v0.1.3".into(),
                version: "0.1.3".into(),
                html_url: "https://github.com/matheuswidder/w-workploy/releases/tag/v0.1.3".into(),
                installer_url: "https://github.com/matheuswidder/w-workploy/releases/download/v0.1.3/Workploy-Setup-0.1.3.exe".into(),
                published_at: "2026-11-18T14:00:00Z".into(),
                notes: "Melhorias na interface multi-VPS, busca paginada de repositórios e verificador de atualizações.".into(),
                has_update: github::is_newer_version(current, "0.1.3"),
            };
            let status = if info.has_update {
                format!("Nova versão disponível: v{} (atual: v{current})", info.version)
            } else {
                format!("Você já está na versão mais recente (v{current}).")
            };
            let seq = self
                .snapshot
                .update_last_event
                .as_ref()
                .map(|e| e.0)
                .unwrap_or(self.last_update_event_seq)
                + 1;
            let kind = if info.has_update {
                ToastKind::Success
            } else {
                ToastKind::Info
            };
            let title = if info.has_update {
                "Atualização disponível"
            } else {
                "Atualizações verificadas"
            };
            self.snapshot.update_checking = false;
            self.snapshot.latest_release = Some(info);
            self.snapshot.update_status = status.clone();
            self.snapshot.update_last_event = Some((seq, true, status.clone()));
            self.last_update_event_seq = seq;
            self.push_toast(format!("upd-demo-{seq}"), title, Some(status), kind, tx);
            return;
        }
        github::check_for_updates(&self.shared);
        self.snapshot = self.shared.lock().unwrap().clone();
        tx.wake_after(std::time::Duration::from_millis(120));
    }

    pub fn install_latest_update(&mut self, tx: &mut Tx) {
        let Some(rel) = self.snapshot.latest_release.clone() else {
            return;
        };
        if !rel.has_update || self.snapshot.update_downloading {
            return;
        }
        if self.demo_mode {
            self.snapshot.update_downloading = true;
            self.snapshot.update_progress = 0.42;
            self.snapshot.update_status = format!(
                "Baixando Workploy v{}... (simulação de progresso)",
                rel.version
            );
            self.push_toast(
                format!("upd-install-demo-{}", rel.version),
                format!("Baixando Workploy v{}", rel.version),
                Some("Download integrado iniciado no modo demonstração.".into()),
                ToastKind::Info,
                tx,
            );
            return;
        }
        if rel.installer_url.trim().is_empty() {
            self.push_toast(
                "upd-install-missing",
                "Instalador indisponível",
                Some("Não há URL direta do instalador para esta release.".into()),
                ToastKind::Warning,
                tx,
            );
            return;
        }
        github::download_and_install_update(&self.shared, rel.installer_url, rel.version);
        self.snapshot = self.shared.lock().unwrap().clone();
        tx.wake_after(std::time::Duration::from_millis(120));
    }

    pub fn open_external_url(url: &str) {
        let trimmed = url.trim();
        if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
            return;
        }
        #[cfg(target_os = "windows")]
        {
            let mut cmd = std::process::Command::new("cmd");
            cmd.args(["/C", "start", "", trimmed]);
            engine::apply_no_window(&mut cmd);
            let _ = cmd.spawn();
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = std::process::Command::new("xdg-open").arg(trimmed).spawn();
        }
    }
}

pub fn setup_app(s: &mut AppState, waker: rune::Waker) {
    let wake_counter = s.wake_gen.clone();
    let wake = waker.clone();
    engine::set_wake_hook(
        &s.shared,
        Some(Arc::new(move || {
            wake_counter.fetch_add(1, Ordering::Relaxed);
            wake();
        })),
    );
    if !s.demo_mode && !rune::tooling::is_headless() {
        engine::spawn_github_poller(s.shared.clone());
        engine::spawn_folder_watcher(s.shared.clone());
    }
}

pub fn poll_app(s: &mut AppState, t: f64, tx: &mut Tx) -> bool {
    s.clock_t = t;
    let mut changed = false;

    let g = s.wake_gen.load(Ordering::Relaxed);
    if g != s.last_wake_gen {
        s.last_wake_gen = g;
        if !s.demo_mode {
            let prev_detected = s.snapshot.detected_platform.clone();
            if let Ok(guard) = s.shared.lock() {
                s.snapshot = guard.clone();
            }
            if s.snapshot.detected_platform != prev_detected
                && let Some((ref det_repo, det_plat)) = s.snapshot.detected_platform
                && det_plat != PlatformTarget::CrossPlatform
                && let ModalState::BindingEditor(d) = &mut s.modal
                && d.full_name.text.trim().eq_ignore_ascii_case(det_repo)
            {
                d.platform = det_plat;
            }
            s.sync_connections_health_seen();
        }
        changed = true;
    }

    if !s.demo_mode && s.snapshot.probing_server_id.is_none() && !s.probe_queue.is_empty() {
        let next_id = s.probe_queue.remove(0);
        vps::probe_vps(&s.shared, &next_id);
        if let Ok(guard) = s.shared.lock() {
            s.snapshot = guard.clone();
        }
        tx.wake_after(std::time::Duration::from_millis(120));
        changed = true;
    }

    if let Some((seq, ok, msg)) = s.snapshot.github_last_event.clone()
        && seq > s.last_github_event_seq
    {
        s.last_github_event_seq = seq;
        let (title, kind) = if ok {
            ("GitHub conectado", ToastKind::Success)
        } else {
            ("Falha ao conectar ao GitHub", ToastKind::Danger)
        };
        s.push_toast(format!("gh-event-{seq}"), title, Some(msg), kind, tx);
        changed = true;
    }

    if let Some((seq, ok, msg)) = s.snapshot.update_last_event.clone()
        && seq > s.last_update_event_seq
    {
        s.last_update_event_seq = seq;
        let (title, kind) = if !ok {
            ("Falha ao buscar atualizações", ToastKind::Danger)
        } else if s
            .snapshot
            .latest_release
            .as_ref()
            .is_some_and(|r| r.has_update)
        {
            ("Atualização disponível", ToastKind::Success)
        } else {
            ("Atualizações verificadas", ToastKind::Info)
        };
        s.push_toast(format!("upd-event-{seq}"), title, Some(msg), kind, tx);
        changed = true;
    }

    if let Some((seq, ok, msg)) = s.snapshot.sync_last_event.clone()
        && seq > s.last_sync_event_seq
    {
        s.last_sync_event_seq = seq;
        let (title, kind) = if ok {
            ("Sincronizado na VPS", ToastKind::Success)
        } else {
            ("Falha ao sincronizar na VPS", ToastKind::Danger)
        };
        s.push_toast(format!("sync-event-{seq}"), title, Some(msg), kind, tx);
        changed = true;
    }

    // Busca de receitas no repo: drena o resultado da thread de rede para o
    // modal `ImportFromRepo` aberto (resultados tardios de modal já fechado
    // ou de outro vínculo são descartados).
    if let Some(cand) = s.repo_yaml_inbox.lock().unwrap().take()
        && let ModalState::ImportFromRepo {
            binding_id,
            loading,
            paths,
            status,
            selected,
            ..
        } = &mut s.modal
        && cand.binding_id == *binding_id
    {
        *loading = false;
        if cand.paths.is_empty() {
            *selected = None;
        } else if selected.is_none() {
            *selected = Some(cand.paths[0].clone());
        }
        *paths = cand.paths;
        *status = cand.status;
        changed = true;
    }

    let has_active = s.has_active_runs();

    // Toast timers
    if let Some(due) = s.toasts.next_due() {
        if t + 1e-3 >= due {
            let before = s.toasts.entries().len();
            let effects = s.toasts.step(ToastEvent::Tick { now: t });
            for eff in effects {
                if let ToastEffect::WakeAt(next_due) = eff {
                    tx.wake_at(next_due);
                }
            }
            if s.toasts.entries().len() != before {
                changed = true;
            }
        } else {
            tx.wake_at(due);
        }
    }

    // Live elapsed duration tick while any run is active, or Connections telemetry is visible
    if (has_active && !rune::tooling::is_headless() && s.auto_refresh)
        || (matches!(s.nav, NavTarget::Connections) && t - s.last_live_tick >= 0.25)
    {
        if t - s.last_live_tick >= (if matches!(s.nav, NavTarget::Connections) { 0.25 } else { 1.0 }) {
            s.last_live_tick = t;
            changed = true;
        }
        if !rune::tooling::is_headless() {
            tx.wake_after(std::time::Duration::from_millis(500));
        }
    }

    changed
}
