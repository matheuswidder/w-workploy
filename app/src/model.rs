use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as RelaxedOrd};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex as StdMutex, OnceLock};
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

/// Proveniência de uma receita importada do repositório (anti-drift):
/// de onde ela veio e o SHA do arquivo no instante da importação, para
/// avisar quando o yml no repo mudar e a cópia local ficar desatualizada.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WorkflowSource {
    pub repo: String,
    pub path: String,
    pub sha: Option<String>,
    pub imported_at: String,
}

/// Configuração extra de uma receita, persistida **fora** do `Workflow` para
/// não alterar o schema já gravado em `workflows.json` (e no
/// `/etc/workploy/config.json` da VPS): limites de CPU/RAM aplicados aos passos
/// e nomes dos passos que devem ser pulados.
#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq)]
pub struct WorkflowExtras {
    /// Limite de vCPU (núcleos) por passo. `None` = sem limite.
    #[serde(default)]
    pub cpu_limit: Option<u32>,
    /// Limite de RAM em MB por passo. `None` = sem limite.
    #[serde(default)]
    pub mem_limit_mb: Option<u32>,
    /// Nomes dos passos ignorados nesta receita.
    #[serde(default)]
    pub skipped_steps: Vec<String>,
}

impl WorkflowExtras {
    pub fn is_empty(&self) -> bool {
        self.cpu_limit.is_none() && self.mem_limit_mb.is_none() && self.skipped_steps.is_empty()
    }

    pub fn skips(&self, step_name: &str) -> bool {
        self.skipped_steps
            .iter()
            .any(|n| n.trim() == step_name.trim())
    }
}

/// Consumo real de recursos medido pelo runner da VPS durante uma execução.
#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq)]
pub struct RunMetrics {
    /// CPU acumulada (segundos de CPU de todos os filhos).
    #[serde(default)]
    pub cpu_secs: f64,
    /// Pico de memória residente observado (MB).
    #[serde(default)]
    pub peak_rss_mb: u32,
    /// Passo encerrado pelo limite de memória (cgroup OOM).
    #[serde(default)]
    pub oom_step: Option<String>,
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
    /// Origem no repo quando a receita foi importada de lá. `#[serde(default)]`
    /// mantém o `workflows.json` legado abrindo sem migração.
    #[serde(default)]
    pub source: Option<WorkflowSource>,
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

/// Fase de uma execução ativa, derivada da fração de passos concluídos.
/// Guia qual spinner de progresso exibir (suave no comecinho, mais vivo no fim).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RunPhase {
    /// Acabou de sair da fila / primeiro passo (< 20% concluído).
    Starting,
    /// Avançando entre passos (20%–60%).
    Progress,
    /// Mais da metade, ainda com trabalho pela frente (60%–85%).
    Slow,
    /// Último passo em execução ou >= 85% concluído.
    Finishing,
}

impl RunPhase {
    pub fn label(self) -> &'static str {
        match self {
            RunPhase::Starting => "Iniciando",
            RunPhase::Progress => "Em progresso",
            RunPhase::Slow => "Avançado",
            RunPhase::Finishing => "Finalizando",
        }
    }
}

/// Família visual do spinner de execução (svg-spinners recriado em rune-fx).
/// Fluxos de CI/build/teste usam a família `pulse-*`; fluxos de deploy, a
/// família `blocks-*`; a fase da execução escolhe o membro da família.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpinnerKind {
    /// `pulse-2`: dois pontos suaves alternando (CI, começando).
    PulseSoft,
    /// `pulse-multiple`: três pontos em sequência (CI, em progresso).
    PulseMultiple,
    /// `pulse-rings-2`: dois anéis expandindo (CI, mais da metade).
    PulseRings2,
    /// `pulse-rings-3`: três anéis acelerando (CI, finalizando).
    PulseRings3,
    /// `blocks-scale`: blocos subindo em sequência (Deploy, começando).
    BlocksScale,
    /// `blocks-shuffle-2`: dois blocos alternando (Deploy, em progresso).
    BlocksShuffle2,
    /// `blocks-shuffle-3`: três blocos em rodízio (Deploy, mais da metade).
    BlocksShuffle3,
    /// `blocks-wave`: onda diagonal final de blocos (Deploy, finalizando).
    BlocksWave,
}

/// Receita de deploy: entrega/reinicia serviços (deploy, redeploy, compose, up -d).
pub fn is_deploy_workflow(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("deploy") || n.contains("docker compose") || n.contains("up -d")
}

/// Receita de CI/build/teste: valida código antes de liberar (ci, test, check,
/// lint, audit, smoke). Receitas sem palavra-chave seguem a família `pulse-*`.
pub fn is_ci_workflow(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("ci")
        || n.contains("test")
        || n.contains("check")
        || n.contains("lint")
        || n.contains("audit")
        || n.contains("smoke")
}

/// Spinner que representa a execução: família pelo tipo de fluxo, membro pela fase.
/// Deploy → família `blocks-*`; CI (e receitas genéricas) → família `pulse-*`.
pub fn run_phase_spinner_kind(run: &Run, workflow_name: &str) -> SpinnerKind {
    if is_deploy_workflow(workflow_name) {
        match run_phase(run) {
            RunPhase::Starting => SpinnerKind::BlocksScale,
            RunPhase::Progress => SpinnerKind::BlocksShuffle2,
            RunPhase::Slow => SpinnerKind::BlocksShuffle3,
            RunPhase::Finishing => SpinnerKind::BlocksWave,
        }
    } else {
        match run_phase(run) {
            RunPhase::Starting => SpinnerKind::PulseSoft,
            RunPhase::Progress => SpinnerKind::PulseMultiple,
            RunPhase::Slow => SpinnerKind::PulseRings2,
            RunPhase::Finishing => SpinnerKind::PulseRings3,
        }
    }
}

/// Fração de progresso da execução: passos concluídos sobre o total, refinada
/// pela última sub-etapa ativa (o passo em execução contribui parcialmente).
pub fn run_progress_frac(run: &Run) -> f32 {
    let total = run.steps.len();
    if total == 0 {
        return 0.0;
    }
    let done = run
        .steps
        .iter()
        .filter(|st| matches!(st.status, StepStatus::Success | StepStatus::Skipped))
        .count();
    let mut frac = done as f32 / total as f32;
    if let Some(active) = run.steps.iter().find(|st| st.status == StepStatus::Running)
        && !active.substeps.is_empty()
    {
        let sub_total = active.substeps.len();
        let sub_done = active
            .substeps
            .iter()
            .filter(|s| matches!(s.status, StepStatus::Success | StepStatus::Skipped))
            .count();
        frac += (sub_done as f32 / sub_total as f32) / total as f32;
    }
    frac.clamp(0.0, 1.0)
}

/// Fase da execução para escolher o spinner animado correspondente.
/// Execuções encerradas não têm fase de progresso e devolvem `Starting`.
pub fn run_phase(run: &Run) -> RunPhase {
    let total = run.steps.len();
    if total == 0 || !run.status.active() {
        return RunPhase::Starting;
    }
    // Último passo em execução: está acabando, mesmo antes dos 85%.
    if run.steps[total - 1].status == StepStatus::Running {
        return RunPhase::Finishing;
    }
    let frac = run_progress_frac(run);
    if frac < 0.2 {
        RunPhase::Starting
    } else if frac < 0.6 {
        RunPhase::Progress
    } else if frac < 0.85 {
        RunPhase::Slow
    } else {
        RunPhase::Finishing
    }
}

/// Percentual inteiro de passos concluídos (para rótulos "Running · 67%").
pub fn steps_completed_pct(run: &Run) -> u8 {
    let total = run.steps.len();
    if total == 0 {
        return 0;
    }
    let done = run
        .steps
        .iter()
        .filter(|st| matches!(st.status, StepStatus::Success | StepStatus::Skipped))
        .count();
    ((done as f32 / total as f32) * 100.0).round().clamp(0.0, 100.0) as u8
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
    pub repo_name: Option<String>,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    /// Vínculo com a execução no runner da VPS quando esta `Run` é espelho de
    /// uma execução criada pelo `workploy-runner` (o id remoto é a própria
    /// chave de vinculação). `#[serde(default)]` mantém o `runs.json` legado
    /// abrindo sem migração.
    #[serde(default)]
    pub vps_run_id: Option<String>,
    /// Servidor dono da execução espelhada (id em `settings.servers`), para
    /// onde o cancelamento do desktop é propagado.
    #[serde(default)]
    pub vps_server_id: Option<String>,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum PlatformTarget {
    #[default]
    CrossPlatform,
    Windows,
    Linux,
    MacOS,
}

impl PlatformTarget {
    pub fn label(self) -> &'static str {
        match self {
            PlatformTarget::CrossPlatform => "Multiplataforma",
            PlatformTarget::Windows => "Windows",
            PlatformTarget::Linux => "Linux",
            PlatformTarget::MacOS => "macOS",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoCommitItem {
    pub sha: String,
    pub short_sha: String,
    pub message: String,
    pub author: String,
    pub date: String,
    pub branch: String,
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
    #[serde(default)]
    pub platform: PlatformTarget,
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
            platform: PlatformTarget::CrossPlatform,
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
    /// Extras por receita (limites de CPU/RAM e passos pulados). Vive ao lado
    /// de `workflows` no `workflows.json` para não quebrar o schema legado.
    #[serde(default)]
    pub workflow_extras: std::collections::HashMap<String, WorkflowExtras>,
    /// Métricas de consumo por execução (medidas pelo runner da VPS).
    #[serde(default)]
    pub run_metrics: std::collections::HashMap<String, RunMetrics>,
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
    pub repo_commits_loading: bool,
    #[serde(skip)]
    pub repo_commits_repo: String,
    #[serde(skip)]
    pub repo_commits_branch: String,
    #[serde(skip)]
    pub repo_commits: Vec<RepoCommitItem>,
    #[serde(skip)]
    pub repo_commits_status: String,
    /// Avisos anti-drift por receita: `(workflow_id, workflow_name, msg)` —
    /// o arquivo no repo mudou desde a importação registrada em `source`.
    #[serde(skip)]
    pub workflow_drift: Vec<(String, String, String)>,
    #[serde(skip)]
    pub detected_platform: Option<(String, PlatformTarget)>,
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
    write_json(data_dir().join("workflows.json"), &WorkflowsFile::from(s));
}

/// Arquivo `workflows.json`: recipes + extras. O formato legado (array puro de
/// `Workflow`) continua abrindo — é migrado na primeira gravação.
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct WorkflowsFile {
    #[serde(default)]
    pub workflows: Vec<Workflow>,
    #[serde(default)]
    pub extras: std::collections::HashMap<String, WorkflowExtras>,
}

impl From<&Shared> for WorkflowsFile {
    fn from(s: &Shared) -> Self {
        Self {
            workflows: s.workflows.clone(),
            extras: s.workflow_extras.clone(),
        }
    }
}

/// Lê `workflows.json` aceitando o formato novo (objeto) e o legado (array).
pub fn read_workflows_file(path: &std::path::Path) -> (Vec<Workflow>, std::collections::HashMap<String, WorkflowExtras>) {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return (Vec::new(), Default::default());
    };
    if let Ok(file) = serde_json::from_str::<WorkflowsFile>(&raw) {
        return (file.workflows, file.extras);
    }
    match serde_json::from_str::<Vec<Workflow>>(&raw) {
        Ok(list) => (list, Default::default()),
        Err(_) => (Vec::new(), Default::default()),
    }
}

// ---------------------------------------------------------------------------
// Persistência de runs: serialização sob lock (CPU pura, sem syscall) e
// gravação em disco numa thread dedicada, FORA do lock global. Antes, cada
// `save_runs` regravava runs.json inteiro (logs acumulados incluídos) em
// `std::fs::write` síncrono enquanto o caller segurava o lock que a UI usa
// para snapshot de render — com runs.json de uso real, cada transição de
// passo bloqueava a UI por dezenas/centenas de ms.
//
// Contrato da camada:
// - escritor único (a thread dedicada) → sem corrupção por concorrência;
// - gravação atômica (.tmp + rename) → crash no meio não deixa runs.json
//   truncado (o caminho antigo deixava);
// - coalescing por destino: rajadas de saves viram 1 gravação,
//   última-versão-ganha; a versão mais recente NUNCA é descartada;
// - `flush_runs_to_disk()` garante o estado final em disco no fim de cada
//   run e no encerramento do app;
// - runs.json lido no mesmo formato (`read_json`), sem mudança de schema.
// ---------------------------------------------------------------------------

struct RunsSnapshot {
    dir: PathBuf,
    text: String,
    seq: u64,
}

struct RunsPersist {
    tx: Sender<RunsSnapshot>,
    written: Arc<(StdMutex<u64>, Condvar)>,
}

static RUNS_PERSIST: OnceLock<RunsPersist> = OnceLock::new();
static RUNS_PERSIST_SEQ: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static RUNS_PERSIST_TEST_DELAY_MS: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static RUNS_PERSIST_WRITES: AtomicU64 = AtomicU64::new(0);

fn runs_persist() -> &'static RunsPersist {
    RUNS_PERSIST.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<RunsSnapshot>();
        let written = Arc::new((StdMutex::new(0u64), Condvar::new()));
        let w2 = written.clone();
        std::thread::Builder::new()
            .name("workploy-runs-persist".into())
            .spawn(move || {
                while let Ok(first) = rx.recv() {
                    // Coalescing: acumula tudo que chegou enquanto a última
                    // gravação corria, por destino — última versão ganha.
                    let mut batch = std::collections::HashMap::new();
                    batch.insert(first.dir.clone(), first);
                    while let Ok(next) = rx.try_recv() {
                        batch.insert(next.dir.clone(), next);
                    }
                    #[cfg(test)]
                    let delay =
                        RUNS_PERSIST_TEST_DELAY_MS.load(RelaxedOrd::Relaxed);
                    #[cfg(test)]
                    if delay > 0 {
                        std::thread::sleep(std::time::Duration::from_millis(delay));
                    }
                    let mut max_seq = 0u64;
                    for snap in batch.into_values() {
                        write_runs_atomic(&snap.dir, &snap.text);
                        max_seq = max_seq.max(snap.seq);
                    }
                    #[cfg(test)]
                    {
                        RUNS_PERSIST_WRITES.fetch_add(1, RelaxedOrd::Relaxed);
                    }
                    if let Ok(mut g) = w2.0.lock() {
                        if max_seq > *g {
                            *g = max_seq;
                        }
                        w2.1.notify_all();
                    }
                }
            })
            .expect("thread de persistência de runs");
        RunsPersist { tx, written }
    })
}

/// Gravação atômica: escreve em `.tmp` e renomeia por cima do alvo. Se o
/// rename falhar (alvo aberto por antivírus/editor no Windows), cai para
/// gravação direta — continua saindo de um único escritor, então não há
/// corrupção por concorrência; perde-se apenas a atomicidade da rodada.
fn write_runs_atomic(dir: &Path, text: &str) {
    let _ = std::fs::create_dir_all(dir);
    let target = dir.join("runs.json");
    let tmp = dir.join("runs.json.tmp");
    if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, &target).is_ok() {
        return;
    }
    let _ = std::fs::write(&target, text);
    let _ = std::fs::remove_file(&tmp);
}

pub fn save_runs(s: &Shared) {
    save_runs_to_dir(&data_dir(), s);
}

pub(crate) fn save_runs_to_dir(dir: &Path, s: &Shared) {
    // Sob lock (o caller ainda o segura): apenas serialização (CPU, mesma
    // ordem de custo do clone de snapshot que a UI já faz). A gravação em
    // disco acontece na thread de persistência, sem tocar o lock global.
    if let Ok(text) = serde_json::to_string_pretty(&s.runs) {
        let snap = RunsSnapshot {
            dir: dir.to_path_buf(),
            text,
            seq: RUNS_PERSIST_SEQ.fetch_add(1, RelaxedOrd::Relaxed) + 1,
        };
        let _ = runs_persist().tx.send(snap);
    }
}

/// Bloqueia até o último snapshot enfileirado estar gravado em disco (com
/// teto de espera defensivo). Chame FORA do lock global — no fim de cada run
/// e no encerramento do app.
pub fn flush_runs_to_disk() {
    let p = runs_persist();
    let target = RUNS_PERSIST_SEQ.load(RelaxedOrd::Acquire);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut g = p.written.0.lock().unwrap();
    while *g < target {
        if std::time::Instant::now() >= deadline {
            return;
        }
        let (ng, _) = p
            .written
            .1
            .wait_timeout(g, std::time::Duration::from_millis(100))
            .unwrap();
        g = ng;
    }
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
        workflows: Vec::new(),
        runs: read_json(dir.join("runs.json")).unwrap_or_default(),
        settings: loaded_settings,
        ..Default::default()
    };
    let (wfs, extras) = read_workflows_file(&dir.join("workflows.json"));
    s.workflows = wfs;
    s.workflow_extras = extras;

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
        write_json(dir.join("workflows.json"), &WorkflowsFile::from(&s));
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
        source: None,
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
        source: None,
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
        source: None,
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
        repo_name: None,
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
        vps_run_id: None,
        vps_server_id: None,
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
        repo_name: None,
        branch: "main".into(),
        summary: String::new(),
        annotations: vec![
            Annotation { kind: AnnKind::Error, job: "Typecheck + Tests + Smoke".into(), msg: ts_err.into() },
            Annotation { kind: AnnKind::Error, job: "Contracts typecheck (Zod)".into(), msg: ts_err.into() },
            Annotation { kind: AnnKind::Error, job: "Secret scan (repo-local)".into(), msg: "O passo não iniciou: o comando retornou erro (exit 1).".into() },
        ],
        vps_run_id: None,
        vps_server_id: None,
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
        repo_name: None,
        branch: "main".into(),
        summary: "# Deploy VPS concluído (0915e2faa46aaee78c3f7c82ba72c87ec01c07d0)\n- Serviços atualizados: api=true | web=true | worker=true\n```\nFilesystem      Size  Used Avail Use% Mounted on\n/dev/sda1        96G   56G   41G  59%\nTYPE            TOTAL   ACTIVE   SIZE     RECLAIMABLE\nImages          27      27       18.55GB  0B (0%)\nContainers      31      31       74.63MB  0B (0%)\nLocal Volumes   18      14       3.201GB  254kB (0%)\nBuild Cache     89      0        4.731GB  4.117GB\n```".into(),
        annotations: vec![
            Annotation { kind: AnnKind::Notice, job: "Deploy & Validate VPS".into(), msg: "Healthchecks 200 em api e web; worker ok.".into() },
        ],
        vps_run_id: None,
        vps_server_id: None,
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
            repo_name: None,
            branch: "main".into(),
            summary: String::new(),
            annotations: Vec::new(),
            vps_run_id: None,
            vps_server_id: None,
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
            repo_name: None,
            branch: "main".into(),
            summary: String::new(),
            annotations: Vec::new(),
            vps_run_id: None,
            vps_server_id: None,
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
            repo_name: None,
            branch: "main".into(),
            summary: String::new(),
            annotations: Vec::new(),
            vps_run_id: None,
            vps_server_id: None,
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

/// Peso estimado de um passo na hora de sugerir limites de recursos.
/// Builds de container e suítes pesadas pesam mais que lint/smoke.
pub fn step_resource_weight(run: &str) -> u32 {
    let r = run.to_ascii_lowercase();
    let name = r.as_str();
    let heavy = [
        "docker compose up",
        "docker build",
        "docker-compose up",
        "build --",
        "pnpm build",
        "next build",
        "cargo build",
        "cargo test",
    ];
    let medium = [
        "test",
        "typecheck",
        "vitest",
        "jest",
        "playwright",
        "pnpm -r",
        "run-package-checks",
        "tsc",
    ];
    let light = ["install", "lint", "audit", "smoke", "curl", "git ", "echo"];
    if heavy.iter().any(|h| name.contains(h)) {
        3
    } else if medium.iter().any(|m| name.contains(m)) {
        2
    } else if light.iter().any(|l| name.contains(l)) {
        1
    } else {
        2
    }
}

/// Sugestão de limites de recursos para uma receita, calculada a partir do
/// peso dos passos, da capacidade da VPS de destino e do histórico medido.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LimitSuggestion {
    pub cpu_limit: Option<u32>,
    pub mem_limit_mb: Option<u32>,
    pub reason: String,
}

/// Calcula a sugestão de vCPU/RAM para uma receita.
///
/// Regras:
/// - **CPU:** metade dos "núcleos-equivalentes" dos passos, arredondado para
///   cima, com pelo menos 1 e sempre deixando **1 vCPU livre** para a VPS
///   (produção e serviços que convivem na mesma máquina).
/// - **RAM:** quando existe medição real (`peak_rss_mb`), usa 1,4× o pico
///   observado (margem para picos); sem medição, estima 512 MB por
///   núcleo-equivalente. O teto é 60% da RAM da VPS.
pub fn suggest_workflow_limits(
    wf: &Workflow,
    vps: Option<&VpsProbeResult>,
    history: &[&Run],
    metrics: Option<&RunMetrics>,
) -> LimitSuggestion {
    let weight: u32 = wf.steps.iter().map(|s| step_resource_weight(&s.run)).sum();
    let cores_equiv = weight.max(1);

    let vcpu_total = vps.map(|p| p.vcpus.max(1)).unwrap_or(4);
    let ram_total_mb = vps.map(|p| p.ram_total_mb).unwrap_or(4096);

    let cpu_cap = vcpu_total.saturating_sub(1).max(1);
    let cpu = cores_equiv.div_ceil(2).clamp(1, cpu_cap);

    let mem_cap = ((ram_total_mb as f64) * 0.6) as u64;
    let mem = match metrics {
        Some(m) if m.peak_rss_mb > 0 => {
            ((m.peak_rss_mb as f64) * 1.4).ceil() as u64
        }
        _ => (cores_equiv as u64) * 512,
    };
    let mem = mem.clamp(1024, mem_cap.max(1024));

    let finished: Vec<&&Run> = history
        .iter()
        .filter(|r| !r.status.active())
        .collect();
    let avg_secs = if finished.is_empty() {
        0.0
    } else {
        finished.iter().map(|r| r.secs.max(0.0)).sum::<f64>() / finished.len() as f64
    };
    let ok_pct = if finished.is_empty() {
        0
    } else {
        let ok = finished
            .iter()
            .filter(|r| r.status == RunStatus::Success)
            .count();
        (ok as f64 / finished.len() as f64 * 100.0).round() as usize
    };

    let mut reason = format!(
        "{} passo(s) com peso {} núcleo(s)-equivalente.",
        wf.steps.len(),
        cores_equiv
    );
    if let Some(m) = metrics
        && m.peak_rss_mb > 0
    {
        reason.push_str(&format!(
            " Últimas medições: pico de {:.1} GB de RAM e {:.0} s de CPU.",
            m.peak_rss_mb as f64 / 1024.0,
            m.cpu_secs
        ));
    }
    if !finished.is_empty() {
        reason.push_str(&format!(
            " Histórico: {} execuções, média {} e {}% de sucesso.",
            finished.len(),
            fmt_dur(avg_secs),
            ok_pct
        ));
    }
    reason.push_str(&format!(
        " A VPS tem {} vCPU e {:.1} GB de RAM — a sugestão deixa 1 vCPU e 40% de RAM livres para o resto da máquina.",
        vcpu_total,
        ram_total_mb as f64 / 1024.0
    ));

    LimitSuggestion {
        cpu_limit: Some(cpu),
        mem_limit_mb: Some(mem as u32),
        reason,
    }
}

pub fn compute_project_kpis(
    runs: &[Run],
    repo_full_name: &str,
    platform: PlatformTarget,
) -> (usize, usize, f64, &'static str) {
    let target = repo_full_name.trim();
    let matching: Vec<&Run> = if target.is_empty() {
        Vec::new()
    } else {
        runs.iter()
            .filter(|r| {
                r.repo_name
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|rn| rn.eq_ignore_ascii_case(target))
            })
            .collect()
    };

    let total_runs = matching.len();
    let finished: Vec<&Run> = matching
        .iter()
        .copied()
        .filter(|r| !r.status.active())
        .collect();

    let success_pct = if finished.is_empty() {
        0
    } else {
        let ok_count = finished
            .iter()
            .filter(|r| r.status == RunStatus::Success)
            .count();
        ((ok_count as f64 / finished.len() as f64) * 100.0).round() as usize
    };

    let avg_duration_secs = if finished.is_empty() {
        0.0
    } else {
        let sum: f64 = finished
            .iter()
            .map(|r| {
                if r.secs.is_finite() {
                    r.secs.max(0.0)
                } else {
                    0.0
                }
            })
            .sum();
        sum / finished.len() as f64
    };

    let estimated_resources_label = match platform {
        PlatformTarget::Windows => "4 vCPU · 8 GB RAM · 50 GB SSD (Windows)",
        PlatformTarget::MacOS => "4 vCPU · 8 GB RAM · 50 GB SSD (macOS)",
        PlatformTarget::Linux => {
            if avg_duration_secs >= 180.0 {
                "4 vCPU · 8 GB RAM · 60 GB SSD (Linux/Docker)"
            } else {
                "2 vCPU · 4 GB RAM · 30 GB SSD (Linux/Docker)"
            }
        }
        PlatformTarget::CrossPlatform => {
            if avg_duration_secs >= 180.0 {
                "4 vCPU · 8 GB RAM · 50 GB SSD"
            } else {
                "2 vCPU · 4 GB RAM · 25 GB SSD"
            }
        }
    };

    (
        total_runs,
        success_pct,
        avg_duration_secs,
        estimated_resources_label,
    )
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
    fn legacy_runs_json_opens_without_vps_link_and_roundtrips_when_set() {
        // runs.json gravado antes dos campos de vínculo com a VPS.
        let raw = r#"[{
            "id": "run-vps-abc-1",
            "workflow_id": "wf-1",
            "workflow_name": "CI",
            "status": "Success",
            "trigger_label": "push",
            "started_at": "17/11/2026 10:00",
            "steps": []
        }]"#;
        let runs: Vec<Run> = serde_json::from_str(raw).expect("runs.json legado deve abrir");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].vps_run_id, None);
        assert_eq!(runs[0].vps_server_id, None);

        // Preenchidos, os campos sobrevivem a um ciclo de gravação/leitura.
        let mut linked = runs[0].clone();
        linked.vps_run_id = Some("run-vps-abc-1".into());
        linked.vps_server_id = Some("vps-1".into());
        let back: Run = serde_json::from_str(&serde_json::to_string(&linked).unwrap()).unwrap();
        assert_eq!(back.vps_run_id.as_deref(), Some("run-vps-abc-1"));
        assert_eq!(back.vps_server_id.as_deref(), Some("vps-1"));
    }

    fn phase_step(status: StepStatus) -> StepRun {
        StepRun {
            name: "passo".into(),
            status,
            log: String::new(),
            secs: 0.0,
            substeps: Vec::new(),
            started_ts: None,
        }
    }

    fn phase_run(status: RunStatus, steps: Vec<StepRun>) -> Run {
        Run {
            id: "run-phase-test".into(),
            workflow_id: "wf-x".into(),
            workflow_name: "X".into(),
            status,
            trigger_label: "manual".into(),
            title: String::new(),
            number: 1,
            commit: None,
            started_at: String::new(),
            started_ts: None,
            finished_at: None,
            steps,
            approved: false,
            cancel_requested: false,
            secs: 0.0,
            actor: String::new(),
            repo_name: None,
            branch: "main".into(),
            summary: String::new(),
            annotations: Vec::new(),
            vps_run_id: None,
            vps_server_id: None,
        }
    }

    #[test]
    fn run_phase_maps_step_fraction_and_last_step() {
        // Fila, sem passo em execução → Starting.
        let queued = phase_run(
            RunStatus::Queued,
            vec![phase_step(StepStatus::Pending), phase_step(StepStatus::Pending)],
        );
        assert_eq!(run_phase(&queued), RunPhase::Starting);

        // Primeiro passo rodando, nada concluído (< 0.2) → Starting.
        let starting = phase_run(
            RunStatus::Running,
            vec![phase_step(StepStatus::Running), phase_step(StepStatus::Pending)],
        );
        assert_eq!(run_phase(&starting), RunPhase::Starting);
        assert_eq!(steps_completed_pct(&starting), 0);

        // 1 de 3 concluído (0.33) → Progress.
        let progress = phase_run(
            RunStatus::Running,
            vec![
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Running),
                phase_step(StepStatus::Pending),
            ],
        );
        assert_eq!(run_phase(&progress), RunPhase::Progress);
        assert_eq!(steps_completed_pct(&progress), 33);

        // 3 de 5 concluídos, passo ativo no meio (0.6) → Slow.
        let slow = phase_run(
            RunStatus::Running,
            vec![
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Running),
                phase_step(StepStatus::Pending),
            ],
        );
        assert_eq!(run_phase(&slow), RunPhase::Slow);
        assert_eq!(steps_completed_pct(&slow), 60);

        // Último passo rodando → Finishing, mesmo abaixo de 85% (0.5 aqui).
        let last_running = phase_run(
            RunStatus::Running,
            vec![
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Running),
            ],
        );
        assert_eq!(run_phase(&last_running), RunPhase::Finishing);

        // Sub-etapas refinam a fração: 2/3 + (1/2)/3 ≈ 0.83 → Slow (não 0.85).
        let mut sub_slow = phase_run(
            RunStatus::Running,
            vec![
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Running),
                phase_step(StepStatus::Pending),
            ],
        );
        sub_slow.steps[2].substeps = vec![
            phase_step(StepStatus::Success),
            phase_step(StepStatus::Running),
        ];
        assert_eq!(run_phase(&sub_slow), RunPhase::Slow);

        // Execuções encerradas não têm fase de progresso.
        let done = phase_run(
            RunStatus::Success,
            vec![
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Success),
            ],
        );
        assert_eq!(run_phase(&done), RunPhase::Starting);
        assert_eq!(steps_completed_pct(&done), 100);
    }

    #[test]
    fn spinner_kind_maps_family_by_workflow_type_and_phase() {
        // CI: família pulse-* pela fase.
        let ci_starting = phase_run(
            RunStatus::Running,
            vec![phase_step(StepStatus::Running), phase_step(StepStatus::Pending)],
        );
        assert_eq!(
            run_phase_spinner_kind(&ci_starting, "CI Principal"),
            SpinnerKind::PulseSoft
        );

        let ci_progress = phase_run(
            RunStatus::Running,
            vec![
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Running),
                phase_step(StepStatus::Pending),
            ],
        );
        assert_eq!(
            run_phase_spinner_kind(&ci_progress, "Node / pnpm CI"),
            SpinnerKind::PulseMultiple
        );

        let ci_slow = phase_run(
            RunStatus::Running,
            vec![
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Running),
                phase_step(StepStatus::Pending),
            ],
        );
        assert_eq!(
            run_phase_spinner_kind(&ci_slow, "Auditoria & Smoke"),
            SpinnerKind::PulseRings2
        );

        let ci_finishing = phase_run(
            RunStatus::Running,
            vec![
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Running),
            ],
        );
        assert_eq!(
            run_phase_spinner_kind(&ci_finishing, "Rust Cargo CI"),
            SpinnerKind::PulseRings3
        );

        // Deploy: família blocks-* pela fase.
        let dep_starting = phase_run(
            RunStatus::Running,
            vec![phase_step(StepStatus::Running), phase_step(StepStatus::Pending)],
        );
        assert_eq!(
            run_phase_spinner_kind(&dep_starting, "Deploy VPS"),
            SpinnerKind::BlocksScale
        );
        assert_eq!(
            run_phase_spinner_kind(&dep_starting, "Docker Compose Redeploy"),
            SpinnerKind::BlocksScale
        );

        let dep_progress = phase_run(
            RunStatus::Running,
            vec![
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Running),
                phase_step(StepStatus::Pending),
            ],
        );
        assert_eq!(
            run_phase_spinner_kind(&dep_progress, "Redeploy Produção"),
            SpinnerKind::BlocksShuffle2
        );

        let dep_slow = phase_run(
            RunStatus::Running,
            vec![
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Running),
                phase_step(StepStatus::Pending),
            ],
        );
        assert_eq!(
            run_phase_spinner_kind(&dep_slow, "Deploy VPS"),
            SpinnerKind::BlocksShuffle3
        );

        let dep_finishing = phase_run(
            RunStatus::Running,
            vec![
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Success),
                phase_step(StepStatus::Running),
            ],
        );
        assert_eq!(
            run_phase_spinner_kind(&dep_finishing, "Deploy VPS"),
            SpinnerKind::BlocksWave
        );

        // Receita genérica (nem CI nem deploy) segue a família pulse-*.
        assert_eq!(
            run_phase_spinner_kind(&dep_progress, "Build Mobile"),
            SpinnerKind::PulseMultiple
        );

        // Classificação por palavra-chave.
        assert!(is_deploy_workflow("docker compose up -d --remove-orphans"));
        assert!(is_deploy_workflow("Redeploy VPS"));
        assert!(!is_deploy_workflow("CI Principal"));
        assert!(is_ci_workflow("Isolamento RLS de testes"));
        assert!(!is_ci_workflow("Deploy VPS"));
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
            platform: PlatformTarget::CrossPlatform,
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
            source: None,
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

    #[test]
    fn compute_project_kpis_filters_repo_and_calculates_metrics() {
        let mk_run = |repo: Option<&str>, status: RunStatus, secs: f64| Run {
            id: new_id("run"),
            workflow_id: "wf-1".into(),
            workflow_name: "CI".into(),
            status,
            trigger_label: "push".into(),
            title: "commit".into(),
            number: 1,
            commit: None,
            started_at: "18/11/2026 10:00".into(),
            started_ts: None,
            finished_at: None,
            steps: vec![],
            approved: false,
            cancel_requested: false,
            secs,
            actor: "maestro".into(),
            repo_name: repo.map(str::to_string),
            branch: "main".into(),
            summary: String::new(),
            annotations: vec![],
            vps_run_id: None,
            vps_server_id: None,
        };

        let runs = vec![
            mk_run(Some("matheuswidder/w-erp"), RunStatus::Success, 120.0),
            mk_run(Some("MATHEUSWIDDER/w-erp"), RunStatus::Success, 240.0),
            mk_run(Some("matheuswidder/w-erp"), RunStatus::Failed, 60.0),
            mk_run(Some("matheuswidder/w-erp"), RunStatus::Running, 999.0),
            mk_run(Some("matheuswidder/other"), RunStatus::Failed, 10.0),
        ];

        let (total, pct, avg, res) =
            compute_project_kpis(&runs, "matheuswidder/w-erp", PlatformTarget::Linux);
        assert_eq!(total, 4);
        assert_eq!(pct, 67);
        assert!((avg - 140.0).abs() < 0.01);
        assert!(res.contains("Linux"));

        let (empty_total, empty_pct, empty_avg, empty_res) =
            compute_project_kpis(&runs, "matheuswidder/missing", PlatformTarget::Windows);
        assert_eq!(empty_total, 0);
        assert_eq!(empty_pct, 0);
        assert_eq!(empty_avg, 0.0);
        assert!(empty_res.contains("Windows"));
    }
}

#[cfg(test)]
mod provenance_tests {
    use super::*;

    fn sample_workflow(source: Option<WorkflowSource>) -> Workflow {
        Workflow {
            id: "wf-1".into(),
            name: "CI Principal".into(),
            trigger: Trigger::Push,
            runner: RunnerKind::Local,
            workdir: String::new(),
            steps: vec![Step {
                name: "Passo".into(),
                run: "echo ok".into(),
                approval: false,
            }],
            on_success: None,
            server_id: None,
            repo_id: None,
            source,
        }
    }

    #[test]
    fn workflows_json_legado_sem_source_abre_sem_migrar() {
        // Formato gravado antes da proveniência existir: sem campo `source`.
        let legacy = r#"{
            "id": "wf-legacy",
            "name": "CI Antigo",
            "trigger": "Push",
            "runner": "Ssh",
            "steps": [{"name": "Build", "run": "cargo build", "approval": false}]
        }"#;
        let wf: Workflow = serde_json::from_str(legacy).expect("workflows.json legado deve abrir");
        assert_eq!(wf.id, "wf-legacy");
        assert_eq!(wf.name, "CI Antigo");
        assert_eq!(wf.trigger, Trigger::Push);
        assert_eq!(wf.runner, RunnerKind::Ssh);
        assert_eq!(wf.source, None);
    }

    #[test]
    fn workflow_source_roundtrip_preserva_proveniencia() {
        let source = WorkflowSource {
            repo: "matheuswidder/w-workploy".into(),
            path: ".workploy.yml".into(),
            sha: Some("0123456789abcdef".into()),
            imported_at: "18/11/2026 10:00:00".into(),
        };
        let wf = sample_workflow(Some(source.clone()));
        let json = serde_json::to_string(&wf).unwrap();
        let back: Workflow = serde_json::from_str(&json).unwrap();
        assert_eq!(back.source, Some(source));
        assert_eq!(back.id, wf.id);
        assert_eq!(back.name, wf.name);
    }

    #[test]
    fn workflow_source_default_e_campos_ausentes() {
        // `sha` (Option) pode faltar no JSON; `imported_at` é obrigatório.
        let sem_sha: WorkflowSource = serde_json::from_str(
            r#"{"repo":"a/b","path":"infra/workploy/ci.yml","imported_at":"18/11/2026 10:00"}"#,
        )
        .unwrap();
        assert_eq!(sem_sha.sha, None);
        assert_eq!(sem_sha.imported_at, "18/11/2026 10:00");
        assert!(
            serde_json::from_str::<WorkflowSource>(r#"{"repo":"a/b","path":"x.yml"}"#).is_err(),
            "sem imported_at a leitura deve falhar explícito"
        );
        assert_eq!(
            WorkflowSource::default(),
            WorkflowSource {
                repo: String::new(),
                path: String::new(),
                sha: None,
                imported_at: String::new()
            }
        );
    }
}

#[cfg(test)]
mod persist_tests {
    use super::*;

    fn fixture_run(idx: usize, log_kb: usize) -> Run {
        let line = "x".repeat(80);
        let log = format!("{line}\n").repeat(log_kb * 1024 / 81 + 1);
        Run {
            id: format!("run-fx-{idx}"),
            workflow_id: "wf-fx".into(),
            workflow_name: "CI Fixture".into(),
            status: RunStatus::Success,
            trigger_label: "manual".into(),
            title: format!("fixture-{idx}"),
            number: idx as u64,
            commit: None,
            started_at: "18/11/2026 10:00".into(),
            started_ts: Some(1),
            finished_at: None,
            steps: vec![StepRun {
                name: "Build".into(),
                status: StepStatus::Success,
                log,
                secs: 540.0,
                substeps: vec![],
                started_ts: Some(1),
            }],
            approved: false,
            cancel_requested: false,
            secs: 540.0,
            actor: "maestro".into(),
            repo_name: Some("widder/erp".into()),
            branch: "main".into(),
            summary: String::new(),
            annotations: vec![],
            vps_run_id: None,
            vps_server_id: None,
        }
    }

    struct ResetPersistDelay;
    impl Drop for ResetPersistDelay {
        fn drop(&mut self) {
            RUNS_PERSIST_TEST_DELAY_MS.store(0, RelaxedOrd::Relaxed);
        }
    }

    /// Com runs.json de uso real (20 runs × ~200 KB de log cada), `save_runs`
    /// chamado sob lock não segura o lock durante o I/O de disco: a gravação
    /// roda na thread dedicada de persistência, coalesce rajadas de saves em
    /// poucas gravações (última versão ganha) e escreve de forma atômica; o
    /// flush garante o estado final em disco e o formato continua legível.
    #[test]
    fn save_runs_writes_off_lock_atomic_coalesced_and_flushable() {
        let _reset = ResetPersistDelay;
        let tmp = std::env::temp_dir().join(format!("workploy-persist-{}", new_id("dir")));
        std::fs::create_dir_all(&tmp).unwrap();

        let sh = Arc::new(StdMutex::new(Shared::default()));
        {
            let mut s = sh.lock().unwrap();
            for i in 0..20 {
                s.runs.push(fixture_run(i, 200));
            }
        }

        // 1) I/O fora do caminho do lock: worker pausado, save sob lock,
        //    lock re-adquirível na hora e arquivo AINDA não gravado.
        RUNS_PERSIST_TEST_DELAY_MS.store(600, RelaxedOrd::Relaxed);
        {
            let s = sh.lock().unwrap();
            save_runs_to_dir(&tmp, &s);
        }
        assert!(
            sh.try_lock().is_ok(),
            "lock deve estar livre logo após save_runs (I/O de disco fora do lock)"
        );
        assert!(
            !tmp.join("runs.json").exists(),
            "a gravação deve acontecer na thread de persistência, não no caller"
        );

        // 2) Coalescing: rajada de saves vira poucas gravações e o arquivo
        //    final carrega a ÚLTIMA versão.
        let writes_before = RUNS_PERSIST_WRITES.load(RelaxedOrd::Relaxed);
        for k in 0..12 {
            let mut s = sh.lock().unwrap();
            s.runs[0].title = format!("versao-{k}");
            save_runs_to_dir(&tmp, &s);
        }
        RUNS_PERSIST_TEST_DELAY_MS.store(0, RelaxedOrd::Relaxed);
        flush_runs_to_disk();

        assert!(tmp.join("runs.json").exists());
        let writes_delta = RUNS_PERSIST_WRITES.load(RelaxedOrd::Relaxed) - writes_before;
        assert!(writes_delta > 0, "pelo menos uma gravação deve ocorrer");
        assert!(
            writes_delta < 12,
            "rajada de 12 saves deve coalescer em menos gravações (delta={writes_delta})"
        );

        // 3) Atômico e legível no formato legado: sem .tmp sobrando e o JSON
        //    reabre como Vec<Run> com a última versão gravada.
        assert!(!tmp.join("runs.json.tmp").exists());
        let back = read_json::<Vec<Run>>(tmp.join("runs.json"))
            .expect("runs.json gravado deve reabrir no formato legado");
        assert_eq!(back.len(), 20);
        assert_eq!(back[0].title, "versao-11");
        assert!(back[0].steps[0].log.len() >= 200 * 1024);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

#[cfg(test)]
mod limit_suggestion_tests {
    use super::*;

    fn wf_with(steps: &[(&str, &str)]) -> Workflow {
        Workflow {
            id: "wf-sug".into(),
            name: "CI".into(),
            trigger: Trigger::Push,
            runner: RunnerKind::Ssh,
            workdir: String::new(),
            steps: steps
                .iter()
                .map(|(n, r)| Step {
                    name: (*n).into(),
                    run: (*r).into(),
                    approval: false,
                })
                .collect(),
            on_success: None,
            server_id: None,
            repo_id: None,
            source: None,
        }
    }

    #[test]
    fn sugestao_respeita_capacidade_da_vps_e_deixa_um_vcpu_livre() {
        let wf = wf_with(&[
            ("Instalar", "pnpm install --frozen-lockfile"),
            ("Testes", "node scripts/run-package-checks.mjs --concurrency=1"),
            ("Build", "docker compose up -d --build"),
        ]);
        let probe = VpsProbeResult {
            ok: true,
            vcpus: 4,
            ram_total_mb: 8192,
            ..Default::default()
        };
        let sug = suggest_workflow_limits(&wf, Some(&probe), &[], None);
        let cpu = sug.cpu_limit.expect("sugere cpu");
        let mem = sug.mem_limit_mb.expect("sugere memoria");
        assert!(cpu >= 1 && cpu <= 3, "cpu {cpu} deve deixar 1 vCPU livre");
        assert!(mem >= 1024, "memoria mínima de 1 GB");
        assert!(
            mem as u64 <= (8192_f64 * 0.6) as u64,
            "memoria {mem} não pode passar de 60% da VPS"
        );
        assert!(sug.reason.contains("vCPU"), "razão: {}", sug.reason);
    }

    #[test]
    fn sugestao_usa_pico_medido_quando_existe() {
        let wf = wf_with(&[("Testes", "pnpm test")]);
        let probe = VpsProbeResult {
            ok: true,
            vcpus: 8,
            ram_total_mb: 16384,
            ..Default::default()
        };
        let metrics = RunMetrics {
            cpu_secs: 120.0,
            peak_rss_mb: 2048,
            oom_step: None,
        };
        let sug = suggest_workflow_limits(&wf, Some(&probe), &[], Some(&metrics));
        // 2048 MB * 1.4 = 2867 -> arredondado para cima no teto de 1 GB.
        let mem = sug.mem_limit_mb.expect("sugere memoria");
        assert!(mem >= 2048, "memoria sugerida {mem} deve cobrir o pico medido");
        assert!(sug.reason.contains("medições"), "razão: {}", sug.reason);
    }
}
