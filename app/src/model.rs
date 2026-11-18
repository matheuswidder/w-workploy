use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
pub type WakeFn = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub enum Trigger {
    #[default]
    Manual,
    Push,
    Folder,
}

impl Trigger {
    pub fn label(self) -> &'static str {
        match self {
            Trigger::Manual => "Manual (botão)",
            Trigger::Push => "push no GitHub",
            Trigger::Folder => "Mudança na pasta",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub enum RunnerKind {
    #[default]
    Local,
    Ssh,
}

impl RunnerKind {
    pub fn label(self) -> &'static str {
        match self {
            RunnerKind::Local => "Esta máquina",
            RunnerKind::Ssh => "VPS via SSH",
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Step {
    pub name: String,
    pub run: String,
    #[serde(default)]
    pub approval: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub trigger: Trigger,
    pub runner: RunnerKind,
    #[serde(default)]
    pub workdir: String,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub on_success: Option<String>,
    #[serde(default)]
    pub server_id: Option<String>,
    #[serde(default)]
    pub repo_id: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum RunStatus {
    Queued,
    Running,
    AwaitingApproval,
    Success,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn label(self) -> &'static str {
        match self {
            RunStatus::Queued => "Na fila",
            RunStatus::Running => "Executando",
            RunStatus::AwaitingApproval => "Aguardando aprovação",
            RunStatus::Success => "Sucesso",
            RunStatus::Failed => "Falhou",
            RunStatus::Cancelled => "Cancelado",
        }
    }
    pub fn active(self) -> bool {
        matches!(
            self,
            RunStatus::Queued | RunStatus::Running | RunStatus::AwaitingApproval
        )
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum StepStatus {
    Pending,
    WaitingApproval,
    Running,
    Success,
    Failed,
    Skipped,
}

impl StepStatus {
    pub fn label(self) -> &'static str {
        match self {
            StepStatus::Pending => "Pendente",
            StepStatus::WaitingApproval => "Aguardando aprovação",
            StepStatus::Running => "Executando",
            StepStatus::Success => "Sucesso",
            StepStatus::Failed => "Falhou",
            StepStatus::Skipped => "Ignorado",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
pub enum AnnKind {
    Error,
    Warning,
    Notice,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Annotation {
    pub kind: AnnKind,
    pub job: String,
    pub msg: String,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct StepRun {
    pub name: String,
    pub status: StepStatus,
    #[serde(default)]
    pub log: String,
    #[serde(default)]
    pub secs: f64,
    #[serde(default)]
    pub substeps: Vec<StepRun>,
    #[serde(default)]
    pub started_ts: Option<i64>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Run {
    pub id: String,
    pub workflow_id: String,
    pub workflow_name: String,
    pub status: RunStatus,
    pub trigger_label: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub number: u64,
    #[serde(default)]
    pub commit: Option<String>,
    pub started_at: String,
    /// Instante real (unix) para formatar a data conforme a tela. Execuções
    /// antigas, gravadas antes deste campo, seguem usando `started_at`.
    #[serde(default)]
    pub started_ts: Option<i64>,
    #[serde(default)]
    pub finished_at: Option<String>,
    pub steps: Vec<StepRun>,
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub cancel_requested: bool,
    #[serde(default)]
    pub secs: f64,
    #[serde(default)]
    pub actor: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub enum AuthMethod {
    #[default]
    SshConfigAlias,
    PrivateKey {
        path: String,
    },
    PasswordInVault,
}

impl AuthMethod {
    pub fn label(&self) -> &'static str {
        match self {
            AuthMethod::SshConfigAlias => "Alias do ~/.ssh/config",
            AuthMethod::PrivateKey { .. } => "Chave privada SSH",
            AuthMethod::PasswordInVault => "Senha no Cofre do Windows",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub enum CapacityTier {
    #[default]
    Insufficient,
    Light,
    Standard,
    Heavy,
}

impl CapacityTier {
    pub fn label(self) -> &'static str {
        match self {
            CapacityTier::Insufficient => "Insuficiente",
            CapacityTier::Light => "Leve",
            CapacityTier::Standard => "Padrão",
            CapacityTier::Heavy => "Pesada",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            CapacityTier::Insufficient => {
                "Abaixo do mínimo recomendado (menos de 1 vCPU, 2 GB RAM ou 12 GB livres)"
            }
            CapacityTier::Light => "Leve: 1-2 vCPU · 2 GB RAM · 25 GB SSD",
            CapacityTier::Standard => "Padrão: 2-4 vCPU · 4-8 GB RAM · 50+ GB SSD",
            CapacityTier::Heavy => "Pesada: 4-8 vCPU · 8-16 GB RAM · 80+ GB NVMe",
        }
    }

    pub fn recommendations() -> &'static [(CapacityTier, &'static str, &'static str)] {
        &[
            (
                CapacityTier::Light,
                "Leve: 1-2 vCPU · 2 GB RAM · 25 GB SSD",
                "Sites estáticos, webhooks e deploys leves",
            ),
            (
                CapacityTier::Standard,
                "Padrão: 2-4 vCPU · 4-8 GB RAM · 50+ GB SSD",
                "Monorepos Node/TypeScript, Docker e CI contínuo",
            ),
            (
                CapacityTier::Heavy,
                "Pesada: 4-8 vCPU · 8-16 GB RAM · 80+ GB NVMe",
                "Builds Rust/Android paralelos e múltiplos containers",
            ),
        ]
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq, Eq)]
pub struct VpsProbeResult {
    pub ok: bool,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub arch: String,
    #[serde(default)]
    pub vcpus: u32,
    #[serde(default)]
    pub ram_total_mb: u64,
    #[serde(default)]
    pub ram_free_mb: u64,
    #[serde(default)]
    pub disk_free_gb: u64,
    #[serde(default)]
    pub cpu_usage_pct: u8,
    #[serde(default)]
    pub disk_used_pct: u8,
    #[serde(default)]
    pub has_git: bool,
    #[serde(default)]
    pub has_docker: bool,
    #[serde(default)]
    pub has_node: bool,
    #[serde(default)]
    pub has_pnpm: bool,
    #[serde(default)]
    pub has_rust: bool,
    #[serde(default)]
    pub has_systemd: bool,
    #[serde(default)]
    pub has_widder_runner: bool,
    #[serde(default)]
    pub runner_active: bool,
    #[serde(default)]
    pub tier: CapacityTier,
    #[serde(default)]
    pub verdict: String,
    #[serde(default)]
    pub checked_at: String,
}

fn default_ssh_port() -> u16 {
    22
}

fn default_runner_port() -> u16 {
    8787
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct VpsServer {
    pub id: String,
    pub name: String,
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub auth: AuthMethod,
    #[serde(default)]
    pub last_probe: Option<VpsProbeResult>,
    #[serde(default)]
    pub runner_installed: bool,
    #[serde(default = "default_runner_port")]
    pub runner_port: u16,
    /// Nome da chave no Cofre do Windows (nunca o segredo em si).
    #[serde(default)]
    pub webhook_secret_key: Option<String>,
}

impl Default for VpsServer {
    fn default() -> Self {
        Self {
            id: new_id("vps"),
            name: String::new(),
            host: String::new(),
            port: 22,
            user: String::new(),
            auth: AuthMethod::SshConfigAlias,
            last_probe: None,
            runner_installed: false,
            runner_port: 8787,
            webhook_secret_key: None,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ProvisionOptions {
    pub install_git: bool,
    pub install_docker: bool,
    pub install_node: bool,
    pub install_rust: bool,
    pub install_runner_service: bool,
    pub runner_port: u16,
}

impl Default for ProvisionOptions {
    fn default() -> Self {
        Self {
            install_git: true,
            install_docker: true,
            install_node: true,
            install_rust: false,
            install_runner_service: true,
            runner_port: 8787,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Default)]
pub enum TriggerMode {
    WebhookOnVps,
    #[default]
    PollOnVps,
    PollOnDesktop,
}

impl TriggerMode {
    pub fn label(self) -> &'static str {
        match self {
            TriggerMode::WebhookOnVps => "Webhook 24/7 na VPS",
            TriggerMode::PollOnVps => "Sondagem 24/7 na VPS",
            TriggerMode::PollOnDesktop => "Sondagem no Desktop",
        }
    }
}

fn default_branch() -> String {
    "main".into()
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct RepoBinding {
    pub id: String,
    pub full_name: String,
    #[serde(default)]
    pub target_server_id: Option<String>,
    #[serde(default = "default_branch")]
    pub branch: String,
    #[serde(default)]
    pub trigger_mode: TriggerMode,
    #[serde(default)]
    pub workflow_id: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub last_sha: String,
    #[serde(default)]
    pub last_status: String,
}

impl Default for RepoBinding {
    fn default() -> Self {
        Self {
            id: new_id("repo"),
            full_name: String::new(),
            target_server_id: None,
            branch: "main".into(),
            trigger_mode: TriggerMode::PollOnVps,
            workflow_id: None,
            enabled: true,
            last_sha: String::new(),
            last_status: String::new(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq, Eq)]
pub struct GithubRepoSummary {
    pub full_name: String,
    #[serde(default)]
    pub default_branch: String,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub updated_at: String,
}

fn default_poll_secs() -> u64 {
    60
}

fn default_purge_every_n_runs() -> u32 {
    10
}

fn default_purge_max_size_mb() -> u64 {
    5120
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PurgeRule {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_purge_every_n_runs")]
    pub every_n_runs: u32,
    #[serde(default = "default_purge_max_size_mb")]
    pub max_size_mb: u64,
    #[serde(default = "default_true")]
    pub clean_workspaces: bool,
    #[serde(default = "default_true")]
    pub clean_docker: bool,
}

impl Default for PurgeRule {
    fn default() -> Self {
        Self {
            enabled: false,
            every_n_runs: 10,
            max_size_mb: 5120,
            clean_workspaces: true,
            clean_docker: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DetectedWorkflow {
    pub path: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub version: String,
    pub html_url: String,
    pub installer_url: String,
    pub published_at: String,
    pub notes: String,
    pub has_update: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Settings {
    #[serde(default)]
    pub ssh_host: String,
    #[serde(default)]
    pub github_repo: String,
    #[serde(default = "default_branch")]
    pub github_branch: String,
    /// Mantido apenas em memória; nunca é serializado em JSON. Tokens legados
    /// encontrados no disco são migrados automaticamente para o Cofre do Windows.
    #[serde(default, skip_serializing)]
    pub github_token: String,
    #[serde(skip)]
    pub has_github_token: bool,
    #[serde(default)]
    pub poll_enabled: bool,
    #[serde(default = "default_poll_secs")]
    pub poll_secs: u64,
    #[serde(default)]
    pub watch_enabled: bool,
    #[serde(default)]
    pub watch_folder: String,
    #[serde(default)]
    pub servers: Vec<VpsServer>,
    #[serde(default)]
    pub repo_bindings: Vec<RepoBinding>,
    #[serde(default)]
    pub github_user: String,
    #[serde(default)]
    pub purge_rule: PurgeRule,
    #[serde(default)]
    pub onboarding_completed: bool,
    #[serde(default = "default_true")]
    pub minimize_to_tray: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ssh_host: "workploy-vps".into(),
            github_repo: String::new(),
            github_branch: "main".into(),
            github_token: String::new(),
            has_github_token: false,
            poll_enabled: false,
            poll_secs: 60,
            watch_enabled: false,
            watch_folder: String::new(),
            servers: Vec::new(),
            repo_bindings: Vec::new(),
            github_user: String::new(),
            purge_rule: PurgeRule::default(),
            onboarding_completed: false,
            minimize_to_tray: true,
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Shared {
    pub workflows: Vec<Workflow>,
    pub runs: Vec<Run>,
    pub settings: Settings,
    #[serde(skip)]
    pub github_status: String,
    #[serde(skip)]
    pub last_sha: String,
    #[serde(skip)]
    pub ssh_test: Option<(bool, String)>,
    #[serde(skip)]
    pub ssh_testing: bool,
    #[serde(skip)]
    pub probing_server_id: Option<String>,
    #[serde(skip)]
    pub provisioning_server_id: Option<String>,
    #[serde(skip)]
    pub purging_server_id: Option<String>,
    #[serde(skip)]
    pub provision_log: String,
    #[serde(skip)]
    pub github_repos: Vec<GithubRepoSummary>,
    #[serde(skip)]
    pub github_testing: bool,
    #[serde(skip)]
    pub github_last_event: Option<(u64, bool, String)>,
    #[serde(skip)]
    pub runner_sync_status: String,
    #[serde(skip)]
    pub sync_last_event: Option<(u64, bool, String)>,
    #[serde(skip)]
    pub update_checking: bool,
    #[serde(skip)]
    pub update_downloading: bool,
    #[serde(skip)]
    pub update_progress: f32,
    #[serde(skip)]
    pub update_status: String,
    #[serde(skip)]
    pub latest_release: Option<ReleaseInfo>,
    #[serde(skip)]
    pub update_last_event: Option<(u64, bool, String)>,
    #[serde(skip)]
    pub repo_branches_loading: bool,
    #[serde(skip)]
    pub repo_branches_repo: String,
    #[serde(skip)]
    pub repo_branches: Vec<String>,
    #[serde(skip)]
    pub repo_branches_status: String,
    #[serde(skip)]
    pub repo_workflows_loading: bool,
    #[serde(skip)]
    pub repo_workflows_repo: String,
    #[serde(skip)]
    pub repo_workflows: Vec<DetectedWorkflow>,
    #[serde(skip)]
    pub repo_workflows_status: String,
    #[serde(skip)]
    pub wake: Option<WakeFn>,
}

pub fn now_str() -> String {
    chrono::Local::now().format("%d/%m/%Y %H:%M:%S").to_string()
}

pub fn now_ts() -> i64 {
    chrono::Local::now().timestamp()
}

pub fn stamp_list(ts: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|t| t.format("%d/%m/%Y %H:%M").to_string())
        .unwrap_or_default()
}

pub fn stamp_age(ts: i64) -> String {
    let mins = (now_ts() - ts).max(0) / 60;
    if mins < 1 {
        "agora mesmo".into()
    } else if mins < 60 {
        format!("há {mins} min")
    } else if mins < 1440 {
        format!("há {} h", mins / 60)
    } else {
        format!("há {} dias", mins / 1440)
    }
}

pub fn fmt_dur(secs: f64) -> String {
    let seconds = if secs.is_finite() {
        secs.max(0.0) as u64
    } else {
        0
    };
    let hours = seconds / 3600;
    let minutes = seconds % 3600 / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours} h {minutes} min {seconds} s")
    } else if minutes > 0 {
        format!("{minutes} min {seconds} s")
    } else {
        format!("{seconds} s")
    }
}

pub fn new_id(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{prefix}-{nanos:x}")
}

fn is_installed_location(exe_dir: &Path) -> bool {
    let lower = exe_dir.to_string_lossy().to_ascii_lowercase();
    lower.contains("\\program files")
        || lower.contains("/program files")
        || lower.contains("\\appdata\\local\\programs")
        || lower.contains("/appdata/local/programs")
}

pub fn resolve_data_dir(
    env_override: Option<&str>,
    exe_dir: Option<&Path>,
    local_app_data: Option<&Path>,
) -> PathBuf {
    if let Some(s) = env_override
        && !s.is_empty()
    {
        return PathBuf::from(s);
    }
    if let Some(dir) = exe_dir {
        if dir.join("portable.flag").exists() {
            return dir.join("data");
        }
        if is_installed_location(dir)
            && let Some(lad) = local_app_data
        {
            return lad.join("Workploy").join("data");
        }
        return dir.join("data");
    }
    if let Some(lad) = local_app_data {
        return lad.join("Workploy").join("data");
    }
    PathBuf::from("data")
}

pub fn data_dir() -> PathBuf {
    let env_override = std::env::var("WORKPLOY_DATA")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("WIDDER_CI_DATA").ok());
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));
    let local_app_data = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let dir = resolve_data_dir(
        env_override.as_deref(),
        exe_dir.as_deref(),
        local_app_data.as_deref(),
    );
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn read_json<T: for<'de> Deserialize<'de>>(path: PathBuf) -> Option<T> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_json<T: Serialize>(path: PathBuf, value: &T) {
    if let Ok(text) = serde_json::to_string_pretty(value) {
        let _ = std::fs::write(path, text);
    }
}

pub fn save_workflows(s: &Shared) {
    write_json(data_dir().join("workflows.json"), &s.workflows);
}

pub fn save_runs(s: &Shared) {
    write_json(data_dir().join("runs.json"), &s.runs);
}

pub fn save_settings_to_dir(dir: &std::path::Path, s: &Shared) {
    let tok = s.settings.github_token.trim();
    if !tok.is_empty() {
        let _ = crate::vault::save_secret(crate::vault::KEY_GITHUB_PAT, tok);
    }
    write_json(dir.join("settings.json"), &s.settings);
}

pub fn save_settings(s: &Shared) {
    save_settings_to_dir(&data_dir(), s);
}

pub fn load_shared_from_dir(dir: &std::path::Path) -> Shared {
    let settings_raw = std::fs::read_to_string(dir.join("settings.json")).ok();
    let has_explicit_onboarding = settings_raw
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|v| v.get("onboarding_completed").map(|_| ()))
        .is_some();
    let loaded_settings: Settings = settings_raw
        .as_deref()
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or_default();

    let mut s = Shared {
        workflows: read_json(dir.join("workflows.json")).unwrap_or_default(),
        runs: read_json(dir.join("runs.json")).unwrap_or_default(),
        settings: loaded_settings,
        ..Default::default()
    };

    // Migração segura: se um settings.json antigo tiver `github_token` em texto puro,
    // move o token para o Cofre do Windows e regrava o JSON sem o segredo.
    let legacy_token = s.settings.github_token.trim().to_string();
    if !legacy_token.is_empty() {
        let _ = crate::vault::save_secret(crate::vault::KEY_GITHUB_PAT, &legacy_token);
        s.settings.github_token = legacy_token;
        s.settings.has_github_token = true;
        save_settings_to_dir(dir, &s);
    } else if let Ok(Some(tok)) = crate::vault::load_secret(crate::vault::KEY_GITHUB_PAT)
        && !tok.trim().is_empty()
    {
        s.settings.github_token = tok;
        s.settings.has_github_token = true;
    }

    if !has_explicit_onboarding
        && (s.settings.has_github_token
            || !s.settings.servers.is_empty()
            || !s.settings.repo_bindings.is_empty())
    {
        s.settings.onboarding_completed = true;
    }

    // Execuções interrompidas (app fechado no meio) não ficam "executando" para sempre.
    for r in &mut s.runs {
        if r.status.active() {
            r.status = RunStatus::Cancelled;
            r.finished_at = Some(now_str());
            for st in &mut r.steps {
                if matches!(
                    st.status,
                    StepStatus::Running | StepStatus::WaitingApproval | StepStatus::Pending
                ) {
                    st.status = StepStatus::Skipped;
                }
            }
        }
    }
    if s.workflows.is_empty() {
        seed(&mut s);
        write_json(dir.join("workflows.json"), &s.workflows);
    }
    s
}

pub fn load_shared() -> Shared {
    load_shared_from_dir(&data_dir())
}

fn seed(s: &mut Shared) {
    let redeploy_id = new_id("wf");
    let ci_id = new_id("wf");
    let rls_id = new_id("wf");
    s.workflows.push(Workflow {
        id: ci_id.clone(),
        name: "CI Principal".into(),
        trigger: Trigger::Push,
        runner: RunnerKind::Local,
        workdir: String::new(),
        steps: vec![
            Step {
                name: "Verificação de tipos + Testes + Smoke".into(),
                run: "echo Verificando tipos... && echo Executando testes... && echo Smoke concluído".into(),
                approval: false,
            },
            Step {
                name: "Varredura de segredos (repo local)".into(),
                run: "echo Procurando segredos no repositório... && echo Nenhum segredo encontrado".into(),
                approval: false,
            },
            Step {
                name: "Validação de contratos (Zod)".into(),
                run: "echo Validando contratos Zod... && echo Nenhum erro encontrado".into(),
                approval: false,
            },
        ],
        on_success: Some(redeploy_id.clone()),
        server_id: None,
        repo_id: None,
    });
    s.workflows.push(Workflow {
        id: redeploy_id,
        name: "Deploy VPS".into(),
        trigger: Trigger::Manual,
        runner: RunnerKind::Ssh,
        workdir: String::new(),
        steps: vec![
            Step {
                name: "Preparar ambiente na VPS".into(),
                run: "echo Preparando ambiente na VPS...".into(),
                approval: false,
            },
            Step {
                name: "Validar conexão SSH".into(),
                run: "echo Conferindo chave SSH...".into(),
                approval: false,
            },
            Step {
                name: "Atualizar código, reconstruir e validar saúde na VPS".into(),
                run: "echo Atualizando código... && echo Reconstruindo... && echo Validando saúde...".into(),
                approval: true,
            },
        ],
        on_success: None,
        server_id: None,
        repo_id: None,
    });
    s.workflows.push(Workflow {
        id: rls_id,
        name: "Isolamento RLS".into(),
        trigger: Trigger::Manual,
        runner: RunnerKind::Local,
        workdir: String::new(),
        steps: vec![
            Step {
                name: "Testes positivos de RLS".into(),
                run: "echo Verificando acesso permitido... OK".into(),
                approval: false,
            },
            Step {
                name: "Testes negativos de RLS".into(),
                run: "echo Verificando bloqueio de acesso indevido... OK".into(),
                approval: false,
            },
        ],
        on_success: None,
        server_id: None,
        repo_id: None,
    });
}

/// Instante de demonstração: `days_ago` dias atrás, no horário local indicado.
fn demo_ts(days_ago: i64, hh: u32, mm: u32) -> i64 {
    let now = chrono::Local::now();
    let day = now.date_naive() - chrono::Duration::days(days_ago);
    day.and_hms_opt(hh, mm, 0)
        .and_then(|d| d.and_local_timezone(chrono::Local).single())
        .map(|t| t.timestamp())
        .unwrap_or_else(|| now.timestamp())
}

fn sr(name: &str, secs: f64, log: &str) -> StepRun {
    StepRun {
        name: name.into(),
        status: StepStatus::Success,
        log: log.into(),
        secs,
        substeps: Vec::new(),
        started_ts: None,
    }
}

/// Amostras de demonstração usadas só no modo foto (dados isolados em diretório
/// temporário). Reproduzem o layout das telas de referência com copy de exemplo;
/// não são execuções reais e nunca tocam os dados da pessoa.
pub fn seed_photo_samples(s: &mut Shared) -> Option<String> {
    let ci = s
        .workflows
        .iter()
        .find(|w| w.name == "CI Principal" || w.name == "CI")?
        .id
        .clone();
    let dep = s.workflows.iter().find(|w| w.name == "Deploy VPS")?.id.clone();
    let actor = "matheuswidder";

    let install_log = "Run pnpm install --frozen-lockfile\nScope: all 32 workspace projects\n? Verifying lockfile against supply-chain policies (996 entries)...\n[WARN] The metadata of xlsx is missing the \"time\" field; skipping the minimumReleaseAge check for this package.\n\u{2713} Lockfile passes supply-chain policies (996 entries in 4.5s)\nLockfile is up to date, resolution step is skipped\nProgress: resolved 1, reused 0, downloaded 0, added 0\nPackages: +811\nProgress: resolved 811, reused 110, downloaded 29, added 8\nProgress: resolved 811, reused 110, downloaded 112, added 37\nProgress: resolved 811, reused 110, downloaded 217, added 73\nProgress: resolved 811, reused 110, downloaded 276, added 95\nProgress: resolved 811, reused 110, downloaded 344, added 117\nProgress: resolved 811, reused 110, downloaded 486, added 170\nProgress: resolved 811, reused 110, downloaded 695, added 244\nDone in 24s";
    let mut typecheck_job = sr("Typecheck + Tests + Smoke", 881.0, "");
    typecheck_job.substeps = vec![
        sr("Run actions/setup-node@v4", 4.0, "Run actions/setup-node@v4\nAttempting to download 24.x...\nAcquiring 24.11.0 - x64\nExtracting ...\nSuccessfully set up node 24.11.0"),
        sr("Install dependencies", 24.0, install_log),
        sr("Lint (if configured)", 1.0, "Run pnpm lint\nNo lint configured at root; skipping."),
        sr("Typecheck all packages", 348.0, "Run pnpm -r typecheck\nScope: 32 of 33 workspace projects\npackages/core typecheck$ tsc --noEmit\nmodules/engenharia typecheck$ tsc --noEmit\nDone in 5m 48s"),
        sr("Test packages (parallel; lista unica em scripts/ci-packages.json)", 485.0, "Run pnpm test:ci\n12 packages, 214 suites\nPASS modules/engenharia/src/__tests__/medicao-capex-acervo-historico.spec.ts\nPASS modules/engenharia/src/contratos/contratos.service.spec.ts\nTests: 1287 passed, 1287 total\nTime: 8m 5s"),
        sr("Readiness smoke", 1.0, "Run node scripts/readiness-smoke.mjs\napi /healthz 200\nweb / 200\nworker queue ok"),
        sr("Dependency audit (high+)", 1.0, "Run pnpm audit --audit-level high\n0 vulnerabilities (high+)"),
        sr("Patch / whitespace check", 0.0, "Run git diff --check HEAD~1 HEAD || git diff --check\nfatal: ambiguous argument 'HEAD~1': unknown revision or path not in the working tree.\nUse '--' to separate paths from revisions, like this:\n'git <command> [<revision>...] -- [<file>...]'"),
        sr("Post Run actions/setup-node@v4", 1.0, "Post job cleanup.\n(node:18068) [DEP0040] DeprecationWarning: The `punycode` module is deprecated. Please use a userland alternative instead.\n(Use `node --trace-deprecation ...` to show where the warning was created)\nCache hit occurred on the primary key node-cache-linux-x64-pnpm, not saving cache."),
        sr("Post Run pnpm/action-setup@v4", 0.0, "Post job cleanup.\nPruning is unnecessary."),
        sr("Post Run actions/checkout@v4", 0.0, "Post job cleanup."),
        sr("Complete job", 0.0, "Cleaning up orphan processes"),
    ];
    let secret_job = sr(
        "Secret scan (repo-local)",
        14.0,
        "Run gitleaks detect --source . --no-banner\n0 leaks found in 412 commits",
    );
    let contracts_job = sr(
        "Contracts typecheck (Zod)",
        27.0,
        "Run pnpm --filter contracts typecheck\nzod schemas: 148 exported, 0 errors",
    );

    let ts_ci_ok = demo_ts(1, 15, 40);
    let warn_msg = "Node.js 20 is deprecated. Actions targeting Node.js 20 are being forced to run on Node.js 24: actions/checkout@v4, actions/setup-node@v4.";
    let ci_ok = Run {
        id: new_id("run"),
        workflow_id: ci.clone(),
        workflow_name: "CI".into(),
        status: RunStatus::Success,
        trigger_label: "push".into(),
        title: "fix(gis,arquivo): aceita aliases de sortBy e refaz UX/UI de Evidencia...".into(),
        number: 671,
        commit: Some("c06c3c1d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b".into()),
        started_at: stamp_list(ts_ci_ok),
        started_ts: Some(ts_ci_ok),
        finished_at: Some(stamp_list(ts_ci_ok)),
        steps: vec![typecheck_job, secret_job, contracts_job],
        approved: false,
        cancel_requested: false,
        secs: 884.0,
        actor: actor.into(),
        branch: "main".into(),
        summary: String::new(),
        annotations: vec![
            Annotation { kind: AnnKind::Warning, job: "Secret scan (repo-local)".into(), msg: warn_msg.into() },
            Annotation { kind: AnnKind::Warning, job: "Contracts typecheck (Zod)".into(), msg: warn_msg.into() },
            Annotation { kind: AnnKind::Warning, job: "Typecheck + Tests + Smoke".into(), msg: warn_msg.into() },
            Annotation { kind: AnnKind::Notice, job: "Typecheck + Tests + Smoke".into(), msg: "Cache hit occurred on the primary key node-cache-linux-x64-pnpm, not saving cache.".into() },
            Annotation { kind: AnnKind::Notice, job: "Typecheck + Tests + Smoke".into(), msg: "Lockfile passes supply-chain policies (996 entries in 4.5s).".into() },
            Annotation { kind: AnnKind::Notice, job: "Readiness smoke".into(), msg: "api, web and worker responded 200.".into() },
        ],
    };
    let ci_ok_id = ci_ok.id.clone();

    let ts_err =
        "error TS2345: Argument of type 'string' is not assignable to parameter of type 'DateFilter'.";
    let mut failed_jobs = Vec::new();
    for n in [
        "Typecheck + Tests + Smoke",
        "Secret scan (repo-local)",
        "Contracts typecheck (Zod)",
    ] {
        let mut j = sr(
            n,
            2.0,
            &format!("Run pnpm -r typecheck\n{ts_err}\n[passo falhou — o fluxo para aqui]"),
        );
        j.status = StepStatus::Failed;
        failed_jobs.push(j);
    }
    let ts_ci_fail = demo_ts(0, 8, 18);
    let ci_fail = Run {
        id: new_id("run"),
        workflow_id: ci.clone(),
        workflow_name: "CI".into(),
        status: RunStatus::Failed,
        trigger_label: "push".into(),
        title: "fix(arquivo,gis): corrigir filtro SQL sem_ocr/erro/na_fila, selecao d...".into(),
        number: 689,
        commit: Some("7db1e72a9b8c7d6e5f4a3b2c1d0e9f8a7b6c5d4e".into()),
        started_at: stamp_list(ts_ci_fail),
        started_ts: Some(ts_ci_fail),
        finished_at: Some(stamp_list(ts_ci_fail)),
        steps: failed_jobs,
        approved: false,
        cancel_requested: false,
        secs: 4.0,
        actor: actor.into(),
        branch: "main".into(),
        summary: String::new(),
        annotations: vec![
            Annotation { kind: AnnKind::Error, job: "Typecheck + Tests + Smoke".into(), msg: ts_err.into() },
            Annotation { kind: AnnKind::Error, job: "Contracts typecheck (Zod)".into(), msg: ts_err.into() },
            Annotation { kind: AnnKind::Error, job: "Secret scan (repo-local)".into(), msg: "O passo não iniciou: o comando retornou erro (exit 1).".into() },
        ],
    };

    let mut deploy_job = sr("Deploy & Validate VPS", 590.0, "");
    deploy_job.substeps = vec![
        sr("Set up job", 0.0, "Initialize runner\nRunner: widder-vps (self-hosted)"),
        sr("Configure SSH", 0.0, "Run ssh -o BatchMode=yes widder-vps echo ok\nok"),
        sr("Pull, rebuild & validate healthchecks on VPS", 539.0, "Run set -euo pipefail\nFrom github.com:matheuswidder/w-erp-central\n * branch            main       -> FETCH_HEAD\n   551e6eb0..0915e2fa  main       -> origin/main\nArquivos alterados (551e6eb0..0915e2fa):\n  apps/web/components/engenharia/ContratoDetalhe.tsx\n  apps/web/lib/engenharia-api.ts\n  modules/engenharia/src/contratos/contratos.service.ts\n  packages/core/src/storage/path-builder.ts\n=== Rebuild & Validate: oktopus-api ===\n Image api-api Building\n Image api-api Built\n Container oktopus-api Recreate\n Container oktopus-api Started\nhealthcheck api: 200\nhealthcheck web: 200\nhealthcheck worker: ok"),
    ];
    let ts_deploy_ok = demo_ts(0, 8, 18);
    let deploy_ok = Run {
        id: new_id("run"),
        workflow_id: dep.clone(),
        workflow_name: "Deploy VPS".into(),
        status: RunStatus::Success,
        trigger_label: "workflow_run".into(),
        title: "Deploy VPS".into(),
        number: 37,
        commit: Some("0915e2faa46aaee78c3f7c82ba72c87ec01c07d0".into()),
        started_at: stamp_list(ts_deploy_ok),
        started_ts: Some(ts_deploy_ok),
        finished_at: Some(stamp_list(ts_deploy_ok)),
        steps: vec![deploy_job],
        approved: true,
        cancel_requested: false,
        secs: 593.0,
        actor: actor.into(),
        branch: "main".into(),
        summary: "# Deploy VPS concluído (0915e2faa46aaee78c3f7c82ba72c87ec01c07d0)\n- Serviços atualizados: api=true | web=true | worker=true\n```\nFilesystem      Size  Used Avail Use% Mounted on\n/dev/sda1        96G   56G   41G  59%\nTYPE            TOTAL   ACTIVE   SIZE     RECLAIMABLE\nImages          27      27       18.55GB  0B (0%)\nContainers      31      31       74.63MB  0B (0%)\nLocal Volumes   18      14       3.201GB  254kB (0%)\nBuild Cache     89      0        4.731GB  4.117GB\n```".into(),
        annotations: vec![
            Annotation { kind: AnnKind::Notice, job: "Deploy & Validate VPS".into(), msg: "Healthchecks 200 em api e web; worker ok.".into() },
        ],
    };

    let deploy_ok_id = deploy_ok.id.clone();
    let ci_ok_id_copy = ci_ok_id.clone();
    let mut history: Vec<Run> = Vec::new();
    let ci_titles = [
        "fix(comercial,engenharia,financeiro,planejamento,web): resumo-360, fi...",
        "fix(compras,inicio): proxy same-origin para dossie PDF e layout mobil...",
        "fix(engenharia,compras,configuracoes): aditivo 400, requisicao/AF/par...",
        "fix(engenharia): evitar 25P02 na categoria Medicoes/MED e isolar arqu...",
        "feat(compras,pdf-kit): refatora SC, Cotacao com dossie PDF e AF com V...",
        "fix(uploads,storage): bucket engenharia-medicoes (0165), auto-heal e ...",
        "fix(uploads): presign ponta a ponta, teto de admin e corpo sem truncar",
        "feat(planejamento,engenharia,shell,arquivo): kanban padrao, agenda mu...",
    ];
    let stamps_ci = [
        (0u64, 8u64, 18u64),
        (1, 21, 16),
        (1, 20, 10),
        (1, 18, 4),
        (1, 17, 28),
        (1, 17, 18),
        (1, 17, 2),
        (1, 16, 15),
    ];
    let shas_ci = [
        "1d94b5b", "7360b65", "bbdf4b3", "045fb8e", "4b490e9", "14fbe53", "c12536f", "cb117b0",
    ];
    for i in 0..ci_titles.len() {
        let mut j = sr(
            "Typecheck + Tests + Smoke",
            2.0,
            &format!("Run pnpm -r typecheck\n{ts_err}\n[passo falhou — o fluxo para aqui]"),
        );
        j.status = StepStatus::Failed;
        let (d, hh, mm) = stamps_ci[i];
        let ts = demo_ts(d as i64, hh as u32, mm as u32);
        history.push(Run {
            id: new_id("run"),
            workflow_id: ci.clone(),
            workflow_name: "CI".into(),
            status: RunStatus::Failed,
            trigger_label: "push".into(),
            title: ci_titles[i].into(),
            number: 688 - i as u64,
            commit: Some(format!("{}0000000000000000000000000000000000000", shas_ci[i])),
            started_at: stamp_list(ts),
            started_ts: Some(ts),
            finished_at: Some(stamp_list(ts)),
            steps: vec![j],
            approved: false,
            cancel_requested: false,
            secs: [4.0, 4.0, 5.0, 4.0, 4.0, 4.0, 5.0, 4.0][i],
            actor: actor.into(),
            branch: "main".into(),
            summary: String::new(),
            annotations: Vec::new(),
        });
    }
    for (k, (num, hh, mm)) in [
        (670u64, 14u32, 58u32),
        (669, 14, 2),
        (668, 12, 41),
        (667, 11, 7),
    ]
    .iter()
    .enumerate()
    {
        let ts = demo_ts(1, *hh, *mm);
        history.push(Run {
            id: new_id("run"),
            workflow_id: ci.clone(),
            workflow_name: "CI".into(),
            status: RunStatus::Success,
            trigger_label: "push".into(),
            title: format!("fix(engenharia): lote anterior de correcoes #{num}"),
            number: *num,
            commit: Some(format!(
                "{:07x}0000000000000000000000000000000000000",
                0xa1b2c0 + k
            )),
            started_at: stamp_list(ts),
            started_ts: Some(ts),
            finished_at: Some(stamp_list(ts)),
            steps: vec![sr(
                "Typecheck + Tests + Smoke",
                860.0,
                "Run pnpm test:ci\nTests: 1287 passed, 1287 total",
            )],
            approved: false,
            cancel_requested: false,
            secs: 862.0,
            actor: actor.into(),
            branch: "main".into(),
            summary: String::new(),
            annotations: Vec::new(),
        });
    }
    let deploy_stamps = [
        (1u64, 21u64, 17u64),
        (1, 20, 10),
        (1, 18, 4),
        (1, 17, 28),
        (1, 17, 18),
        (1, 17, 2),
        (1, 16, 15),
        (1, 14, 58),
        (1, 14, 2),
        (1, 12, 41),
    ];
    for (i, &(d, hh, mm)) in deploy_stamps.iter().enumerate() {
        let ts = demo_ts(d as i64, hh as u32, mm as u32);
        history.push(Run {
            id: new_id("run"),
            workflow_id: dep.clone(),
            workflow_name: "Deploy VPS".into(),
            status: RunStatus::Success,
            trigger_label: if i == 0 {
                "workflow_run".into()
            } else {
                "manual".into()
            },
            title: "Deploy VPS".into(),
            number: 36 - i as u64,
            commit: None,
            started_at: stamp_list(ts),
            started_ts: Some(ts),
            finished_at: Some(stamp_list(ts)),
            steps: vec![sr(
                "Deploy & Validate VPS",
                1.0,
                "Configure SSH\nssh -o BatchMode=yes widder-vps echo ok\nok",
            )],
            approved: true,
            cancel_requested: false,
            secs: if i == 1 { 2.0 } else { 1.0 },
            actor: actor.into(),
            branch: "main".into(),
            summary: String::new(),
            annotations: Vec::new(),
        });
    }
    let _ = (deploy_ok_id, ci_ok_id_copy);
    history.push(ci_fail);
    history.push(ci_ok);
    history.push(deploy_ok);
    history.sort_by(|a, b| {
        b.started_ts
            .unwrap_or(0)
            .cmp(&a.started_ts.unwrap_or(0))
            .then(b.number.cmp(&a.number))
    });
    s.runs = history;
    Some(ci_ok_id)
}

impl Run {
    pub fn elapsed_secs(&self) -> f64 {
        if self.status.active() {
            self.started_ts
                .map(|t| (now_ts() - t).max(0) as f64)
                .unwrap_or(self.secs)
        } else {
            self.secs
        }
    }
}
impl StepRun {
    pub fn elapsed_secs(&self) -> f64 {
        if self.status == StepStatus::Running {
            self.started_ts
                .map(|t| (now_ts() - t).max(0) as f64)
                .unwrap_or(self.secs)
        } else {
            self.secs
        }
    }
}

#[cfg(test)]
mod duration_tests {
    use super::*;
    use crate::vault;

    #[test]
    fn fluxos_iniciais_em_portugues_e_compativeis_com_amostras() {
        let mut shared = Shared::default();
        seed(&mut shared);
        assert_eq!(shared.workflows[0].name, "CI Principal");
        assert_eq!(
            shared.workflows[0].steps[0].name,
            "Verificação de tipos + Testes + Smoke"
        );
        assert_eq!(
            shared.workflows[1].steps[2].name,
            "Atualizar código, reconstruir e validar saúde na VPS"
        );
        assert_eq!(shared.workflows[2].name, "Isolamento RLS");
        assert!(seed_photo_samples(&mut shared).is_some());
    }

    #[test]
    fn format_and_live_step_duration() {
        assert_eq!(fmt_dur(0.0), "0 s");
        assert_eq!(fmt_dur(65.0), "1 min 5 s");
        let mut step = StepRun {
            name: "teste".into(),
            status: StepStatus::Running,
            log: String::new(),
            secs: 0.0,
            substeps: vec![],
            started_ts: Some(now_ts() - 4),
        };
        assert!(step.elapsed_secs() >= 4.0);
        step.status = StepStatus::Success;
        step.secs = 1.25;
        assert_eq!(step.elapsed_secs(), 1.25);
    }

    #[test]
    fn secrets_never_appear_in_serialized_settings_json() {
        let mut settings = Settings {
            github_token: "ghp_secret_never_serialize_999".into(),
            ..Settings::default()
        };
        settings.servers.push(VpsServer {
            id: "vps-1".into(),
            name: "Prod".into(),
            host: "10.0.0.1".into(),
            port: 22,
            user: "root".into(),
            auth: AuthMethod::PasswordInVault,
            last_probe: None,
            runner_installed: true,
            runner_port: 8787,
            webhook_secret_key: Some(vault::vps_webhook_secret_key("vps-1")),
        });
        let json = serde_json::to_string_pretty(&settings).unwrap();
        assert!(!json.contains("ghp_secret_never_serialize_999"));
        assert!(!json.contains("github_token"));
    }

    #[test]
    fn legacy_plaintext_token_migrates_into_vault_and_clears_disk() {
        let _guard = vault::test_lock();
        let _ = vault::delete_secret(vault::KEY_GITHUB_PAT);

        let tmp = std::env::temp_dir().join(format!("workploy-model-test-{}", new_id("dir")));
        std::fs::create_dir_all(&tmp).unwrap();

        let legacy_json = r#"{
            "ssh_host": "legacy-vps",
            "github_repo": "matheuswidder/repo",
            "github_branch": "main",
            "github_token": "ghp_legacy_plaintext_token",
            "poll_enabled": true,
            "poll_secs": 45,
            "watch_enabled": false,
            "watch_folder": ""
        }"#;
        std::fs::write(tmp.join("settings.json"), legacy_json).unwrap();

        let loaded = load_shared_from_dir(&tmp);
        assert_eq!(loaded.settings.github_token, "ghp_legacy_plaintext_token");
        assert!(loaded.settings.has_github_token);
        assert_eq!(
            vault::load_secret(vault::KEY_GITHUB_PAT).unwrap(),
            Some("ghp_legacy_plaintext_token".into())
        );

        let cleaned_disk = std::fs::read_to_string(tmp.join("settings.json")).unwrap();
        assert!(!cleaned_disk.contains("ghp_legacy_plaintext_token"));
        assert!(!cleaned_disk.contains("github_token"));

        let _ = vault::delete_secret(vault::KEY_GITHUB_PAT);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn repo_binding_and_workflow_routing_roundtrip() {
        let binding = RepoBinding {
            id: "rb-1".into(),
            full_name: "widder/erp".into(),
            target_server_id: Some("vps-prod".into()),
            branch: "main".into(),
            trigger_mode: TriggerMode::WebhookOnVps,
            workflow_id: Some("wf-ci".into()),
            enabled: true,
            last_sha: "abc1234".into(),
            last_status: "Monitorando".into(),
        };
        let wf = Workflow {
            id: "wf-ci".into(),
            name: "CI".into(),
            trigger: Trigger::Push,
            runner: RunnerKind::Ssh,
            workdir: "/opt/app".into(),
            steps: vec![],
            on_success: None,
            server_id: Some("vps-prod".into()),
            repo_id: Some("rb-1".into()),
        };
        let json_b = serde_json::to_string(&binding).unwrap();
        let back_b: RepoBinding = serde_json::from_str(&json_b).unwrap();
        assert_eq!(back_b.target_server_id.as_deref(), Some("vps-prod"));
        assert_eq!(back_b.trigger_mode, TriggerMode::WebhookOnVps);

        let json_w = serde_json::to_string(&wf).unwrap();
        let back_w: Workflow = serde_json::from_str(&json_w).unwrap();
        assert_eq!(back_w.server_id.as_deref(), Some("vps-prod"));
        assert_eq!(back_w.repo_id.as_deref(), Some("rb-1"));
        assert!(!CapacityTier::recommendations().is_empty());
    }

    #[test]
    fn resolve_data_dir_routes_installed_portable_and_dev_paths() {
        let lad = Path::new(r"C:\Users\maestro\AppData\Local");

        // 1. WORKPLOY_DATA override tem prioridade máxima
        assert_eq!(
            resolve_data_dir(
                Some(r"D:\custom-ci-data"),
                Some(Path::new(r"C:\Program Files\Workploy")),
                Some(lad),
            ),
            PathBuf::from(r"D:\custom-ci-data")
        );

        // 2. Program Files sem portable.flag vai para %LOCALAPPDATA%\Workploy\data
        assert_eq!(
            resolve_data_dir(
                None,
                Some(Path::new(r"C:\Program Files\Workploy")),
                Some(lad),
            ),
            lad.join("Workploy").join("data")
        );

        // 3. Per-user AppData\Local\Programs sem portable.flag também vai para %LOCALAPPDATA%\Workploy\data
        assert_eq!(
            resolve_data_dir(
                Some(""),
                Some(Path::new(r"C:\Users\maestro\AppData\Local\Programs\Workploy")),
                Some(lad),
            ),
            lad.join("Workploy").join("data")
        );

        // 4. portable.flag força modo portátil mesmo dentro de diretório com "Program Files" no nome
        let tmp = std::env::temp_dir().join(format!("workploy-portable-{}", new_id("dir")));
        let fake_prog_files = tmp.join("Program Files").join("Workploy");
        std::fs::create_dir_all(&fake_prog_files).unwrap();
        std::fs::write(fake_prog_files.join("portable.flag"), b"").unwrap();
        assert_eq!(
            resolve_data_dir(None, Some(&fake_prog_files), Some(lad)),
            fake_prog_files.join("data")
        );
        let _ = std::fs::remove_dir_all(&tmp);

        // 5. target\debug e builds locais preservam exe_dir\data
        let dev_dir = Path::new(r"C:\Projects\w-workploy\app\target\debug");
        assert_eq!(
            resolve_data_dir(None, Some(dev_dir), Some(lad)),
            dev_dir.join("data")
        );

        // 6. Sem exe_dir mas com LOCALAPPDATA usa %LOCALAPPDATA%\Workploy\data
        assert_eq!(
            resolve_data_dir(None, None, Some(lad)),
            lad.join("Workploy").join("data")
        );
    }

    #[test]
    fn purge_rule_defaults_and_serde_roundtrip() {
        let def = PurgeRule::default();
        assert!(!def.enabled);
        assert_eq!(def.every_n_runs, 10);
        assert_eq!(def.max_size_mb, 5120);
        assert!(def.clean_workspaces);
        assert!(def.clean_docker);

        // Partial JSON preserves defaults for omitted fields
        let partial: PurgeRule = serde_json::from_str(r#"{"enabled": true}"#).unwrap();
        assert!(partial.enabled);
        assert_eq!(partial.every_n_runs, 10);
        assert_eq!(partial.max_size_mb, 5120);
        assert!(partial.clean_workspaces);
        assert!(partial.clean_docker);

        // Settings without purge_rule deserializes with defaults and roundtrips cleanly
        let legacy_settings: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy_settings.purge_rule, def);

        let custom = PurgeRule {
            enabled: true,
            every_n_runs: 5,
            max_size_mb: 2048,
            clean_workspaces: true,
            clean_docker: false,
        };
        let settings = Settings {
            purge_rule: custom.clone(),
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.purge_rule, custom);
        assert!(legacy_settings.minimize_to_tray);
        assert!(!legacy_settings.onboarding_completed);
    }

    #[test]
    fn onboarding_auto_completes_for_existing_installations_unless_explicit() {
        let _guard = vault::test_lock();
        let _ = vault::delete_secret(vault::KEY_GITHUB_PAT);

        let tmp = std::env::temp_dir().join(format!("workploy-onboard-test-{}", new_id("dir")));
        std::fs::create_dir_all(&tmp).unwrap();

        // 1. Sem configuração prévia -> onboarding_completed = false
        std::fs::write(tmp.join("settings.json"), r#"{}"#).unwrap();
        let fresh = load_shared_from_dir(&tmp);
        assert!(!fresh.settings.onboarding_completed);
        assert!(fresh.settings.minimize_to_tray);

        // 2. JSON legado com servidor cadastrado e sem campo onboarding_completed -> true
        std::fs::write(
            tmp.join("settings.json"),
            r#"{"servers":[{"id":"vps-1","name":"Prod","host":"10.0.0.1"}]}"#,
        )
        .unwrap();
        let migrated = load_shared_from_dir(&tmp);
        assert!(migrated.settings.onboarding_completed);

        // 3. Explícito false deve ser respeitado mesmo com servidores cadastrados
        std::fs::write(
            tmp.join("settings.json"),
            r#"{"onboarding_completed":false,"servers":[{"id":"vps-1","name":"Prod","host":"10.0.0.1"}]}"#,
        )
        .unwrap();
        let explicit_false = load_shared_from_dir(&tmp);
        assert!(!explicit_false.settings.onboarding_completed);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
