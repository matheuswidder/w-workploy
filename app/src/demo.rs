use workploy::model::{
    self, AnnKind, Annotation, AuthMethod, CapacityTier, GithubRepoSummary, PlatformTarget,
    PurgeRule, RepoBinding, Run, RunStatus, RunnerKind, Settings, Shared, Step, StepRun,
    StepStatus, Trigger, TriggerMode, VpsProbeResult, VpsServer, Workflow,
};

use crate::state::{AppState, ModalState, NavTarget, SettingsDraft, SettingsTab, WorkflowDraft};

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

pub fn populate_demo(s: &mut Shared) {
    let wf_ci = "wf-ci-principal".to_string();
    let wf_deploy = "wf-deploy-vps".to_string();
    let wf_rls = "wf-rls-isolamento".to_string();
    let wf_mobile = "wf-build-mobile".to_string();

    let vps_prod_id = "vps-prod".to_string();
    let vps_staging_id = "vps-staging".to_string();
    let rb_erp_id = "rb-erp".to_string();
    let rb_landing_id = "rb-landing".to_string();

    s.workflows = vec![
        Workflow {
            id: wf_ci.clone(),
            name: "CI Principal".into(),
            trigger: Trigger::Push,
            runner: RunnerKind::Local,
            workdir: "C:\\Projects\\w-erp".into(),
            steps: vec![
                Step {
                    name: "Verificação de tipos e testes".into(),
                    run: "pnpm -r typecheck && pnpm test:ci".into(),
                    approval: false,
                },
                Step {
                    name: "Varredura de segredos".into(),
                    run: "gitleaks detect --source . --no-banner".into(),
                    approval: false,
                },
                Step {
                    name: "Validação de contratos".into(),
                    run: "pnpm --filter contracts typecheck".into(),
                    approval: false,
                },
            ],
            on_success: Some(wf_deploy.clone()),
            server_id: None,
            repo_id: None,
            // Importada do repo (fonte da verdade) — e o yml no repo mudou
            // desde então, então o aviso anti-drift acende no resumo do fluxo.
            source: Some(workploy::model::WorkflowSource {
                repo: "octocat/landing".into(),
                path: ".workploy.yml".into(),
                sha: Some("c06c3c17a8c44d".into()),
                imported_at: "2026-11-18 09:12:00".into(),
            }),
        },
        Workflow {
            id: wf_deploy.clone(),
            name: "Deploy VPS".into(),
            trigger: Trigger::Manual,
            runner: RunnerKind::Ssh,
            workdir: "/srv/acme-erp".into(),
            steps: vec![
                Step {
                    name: "Preparar ambiente remoto".into(),
                    run: "git fetch origin && git checkout main && git pull --ff-only".into(),
                    approval: false,
                },
                Step {
                    name: "Reconstruir containers".into(),
                    run: "docker compose up -d --build api web worker".into(),
                    approval: true,
                },
                Step {
                    name: "Validar saúde dos serviços".into(),
                    run: "curl -fsS https://api.example.com/healthz && echo 200 OK".into(),
                    approval: false,
                },
            ],
            on_success: None,
            // Deploy numa VPS diferente do CI do mesmo vínculo (cross-VPS).
            server_id: Some(vps_staging_id.clone()),
            repo_id: None,
            source: None,
        },
        Workflow {
            id: wf_rls.clone(),
            name: "Isolamento RLS".into(),
            trigger: Trigger::Manual,
            runner: RunnerKind::Local,
            workdir: "C:\\Projects\\w-erp".into(),
            steps: vec![
                Step {
                    name: "Testes positivos de tenant".into(),
                    run: "pnpm test:rls:positive".into(),
                    approval: false,
                },
                Step {
                    name: "Aplicar políticas em homologação".into(),
                    run: "supabase db push --linked".into(),
                    approval: true,
                },
            ],
            on_success: None,
            server_id: None,
            repo_id: None,
            source: None,
        },
        Workflow {
            id: wf_mobile.clone(),
            name: "Build Mobile".into(),
            trigger: Trigger::Folder,
            runner: RunnerKind::Local,
            workdir: "C:\\Projects\\w-mobile".into(),
            steps: vec![
                Step {
                    name: "Compilar APK de homologação".into(),
                    run: "./gradlew assembleRelease".into(),
                    approval: false,
                },
            ],
            on_success: None,
            server_id: None,
            repo_id: None,
            source: None,
        },
    ];

    let servers = vec![
        VpsServer {
            id: vps_prod_id.clone(),
            name: "VPS Produção".into(),
            host: "prod.example.com".into(),
            port: 22,
            user: "deploy".into(),
            auth: AuthMethod::SshConfigAlias,
            runner_installed: true,
            runner_port: 8787,
            webhook_secret_key: Some("vps:vps-prod:webhook".into()),
            last_probe: Some(VpsProbeResult {
                ok: true,
                os: "Ubuntu 24.04.1 LTS".into(),
                arch: "x86_64".into(),
                vcpus: 4,
                ram_total_mb: 8192,
                ram_free_mb: 6180,
                disk_free_gb: 64,
                cpu_usage_pct: 22,
                disk_used_pct: 36,
                has_git: true,
                has_docker: true,
                has_node: true,
                has_pnpm: true,
                has_rust: true,
                has_systemd: true,
                has_beacon_runner: true,
                runner_active: true,
                tier: CapacityTier::Standard,
                verdict: "Capacidade Padrão — recomendada para monorepos Node/Docker e CI contínuo.".into(),
                checked_at: "agora mesmo".into(),
            }),
        },
        VpsServer {
            id: vps_staging_id.clone(),
            name: "VPS Staging".into(),
            host: "203.0.113.10".into(),
            port: 22,
            user: "ubuntu".into(),
            auth: AuthMethod::PrivateKey {
                path: "C:\\Users\\user\\.ssh\\beacon_staging.ed25519".into(),
            },
            runner_installed: false,
            runner_port: 8787,
            webhook_secret_key: None,
            last_probe: Some(VpsProbeResult {
                ok: true,
                os: "Ubuntu 22.04.5 LTS".into(),
                arch: "x86_64".into(),
                vcpus: 2,
                ram_total_mb: 4096,
                ram_free_mb: 2890,
                disk_free_gb: 34,
                cpu_usage_pct: 14,
                disk_used_pct: 42,
                has_git: true,
                has_docker: true,
                has_node: true,
                has_pnpm: false,
                has_rust: false,
                has_systemd: true,
                has_beacon_runner: false,
                runner_active: false,
                tier: CapacityTier::Light,
                verdict: "Capacidade Leve — ideal para APIs leves, webhooks e deploys rápidos.".into(),
                checked_at: "há 12 min".into(),
            }),
        },
    ];

    let repo_bindings = vec![
        RepoBinding {
            id: rb_erp_id,
            full_name: "octocat/erp-central".into(),
            target_server_id: Some(vps_prod_id.clone()),
            branch: "main".into(),
            trigger_mode: TriggerMode::WebhookOnVps,
            workflow_id: Some(wf_deploy.clone()),
            enabled: true,
            last_sha: "a8f4c12".into(),
            last_status: "Webhook 24/7 ativo · último commit a8f4c12".into(),
            platform: PlatformTarget::Linux,
            // CI (testes pesados) na VPS Produção, deploy na VPS Staging.
            ci_server_id: Some(vps_prod_id.clone()),
            deploy_server_id: Some(vps_staging_id.clone()),
        },
        RepoBinding {
            id: rb_landing_id,
            full_name: "octocat/landing".into(),
            target_server_id: None,
            branch: "main".into(),
            trigger_mode: TriggerMode::PollOnDesktop,
            workflow_id: Some(wf_ci.clone()),
            enabled: true,
            last_sha: "c06c3c1".into(),
            last_status: "Monitorando no Desktop · commit atual c06c3c1".into(),
            platform: PlatformTarget::CrossPlatform,
            ci_server_id: None,
            deploy_server_id: None,
        },
    ];

    s.settings = Settings {
        ssh_host: "beacon-vps".into(),
        github_repo: "octocat/erp-central".into(),
        github_branch: "main".into(),
        github_token: String::new(),
        has_github_token: true,
        poll_enabled: true,
        poll_secs: 60,
        watch_enabled: false,
        watch_folder: "C:\\Projects\\w-mobile".into(),
        servers,
        ssh_mesh: Vec::new(),
        repo_bindings,
        github_user: "octocat".into(),
        purge_rule: PurgeRule::default(),
        onboarding_completed: true,
        minimize_to_tray: true,
    };
    s.github_repos = vec![
        GithubRepoSummary {
            full_name: "octocat/erp-central".into(),
            default_branch: "main".into(),
            private: true,
            description: "ERP central multi-tenant Acme".into(),
            html_url: "https://github.com/octocat/erp-central".into(),
            updated_at: "2026-11-17T18:40:00Z".into(),
        },
        GithubRepoSummary {
            full_name: "octocat/landing".into(),
            default_branch: "main".into(),
            private: false,
            description: "Site institucional e página principal".into(),
            html_url: "https://github.com/octocat/landing".into(),
            updated_at: "2026-11-16T14:15:00Z".into(),
        },
        GithubRepoSummary {
            full_name: "acme/mobile-app".into(),
            default_branch: "develop".into(),
            private: true,
            description: "Aplicativo Android e iOS de campo".into(),
            html_url: "https://github.com/acme/mobile-app".into(),
            updated_at: "2026-11-15T09:20:00Z".into(),
        },
        GithubRepoSummary {
            full_name: "acme/faturamento-pix".into(),
            default_branch: "main".into(),
            private: true,
            description: "Serviço de cobrança recorrente e webhooks PIX".into(),
            html_url: "https://github.com/acme/faturamento-pix".into(),
            updated_at: "2026-11-14T16:05:00Z".into(),
        },
        GithubRepoSummary {
            full_name: "acme/audit-log".into(),
            default_branch: "main".into(),
            private: true,
            description: "Serviço append-only de auditoria e trilha de eventos".into(),
            html_url: "https://github.com/acme/audit-log".into(),
            updated_at: "2026-11-13T11:30:00Z".into(),
        },
        GithubRepoSummary {
            full_name: "octocat/portal-cliente".into(),
            default_branch: "main".into(),
            private: true,
            description: "Portal do cliente para faturas e chamados".into(),
            html_url: "https://github.com/octocat/portal-cliente".into(),
            updated_at: "2026-11-12T10:12:00Z".into(),
        },
        GithubRepoSummary {
            full_name: "acme/worker-media".into(),
            default_branch: "main".into(),
            private: true,
            description: "Fila assíncrona de processamento de mídia".into(),
            html_url: "https://github.com/acme/worker-media".into(),
            updated_at: "2026-11-11T19:45:00Z".into(),
        },
        GithubRepoSummary {
            full_name: "acme/contratos-sdk".into(),
            default_branch: "main".into(),
            private: false,
            description: "Schemas compartilhados e validação de contratos".into(),
            html_url: "https://github.com/acme/contratos-sdk".into(),
            updated_at: "2026-11-10T08:50:00Z".into(),
        },
        GithubRepoSummary {
            full_name: "acme/infra-caddy".into(),
            default_branch: "main".into(),
            private: true,
            description: "Configuração de proxy reverso e TLS automático".into(),
            html_url: "https://github.com/acme/infra-caddy".into(),
            updated_at: "2026-11-09T15:20:00Z".into(),
        },
        GithubRepoSummary {
            full_name: "octocat/w-workploy".into(),
            default_branch: "main".into(),
            private: false,
            description: "Workploy desktop para workflows de CI/CD e deploy".into(),
            html_url: "https://github.com/octocat/w-workploy".into(),
            updated_at: "2026-11-18T09:00:00Z".into(),
        },
    ];
    s.github_status = "Conectado como @octocat · 10 repositórios carregados".into();
    s.last_sha = "a8f4c12f9b3c4d5e6f708192a3b4c5d6e7f8091a".into();
    s.ssh_test = Some((true, "VPS Produção ativa · 4 vCPU · 8.0 GB RAM · 64 GB livres (18 ms)".into()));
    s.ssh_testing = false;
    s.runner_sync_status = "Regras sincronizadas com 'VPS Produção'.".into();

    let now = model::now_ts();

    // 1. AwaitingApproval — Deploy VPS #38
    let run_approval_1 = Run {
        id: "run-demo-approval-1".into(),
        workflow_id: wf_deploy.clone(),
        workflow_name: "Deploy VPS".into(),
        status: RunStatus::AwaitingApproval,
        trigger_label: "workflow_run".into(),
        title: "Deploy de produção após CI #691 (release v2.14.0)".into(),
        number: 38,
        commit: Some("a8f4c12f9b3c4d5e6f708192a3b4c5d6e7f8091a".into()),
        started_at: model::stamp_list(now - 95),
        started_ts: Some(now - 95),
        finished_at: None,
        steps: vec![
            sr(
                "Preparar ambiente remoto",
                6.0,
                "Conectando via SSH em beacon-vps...\nSincronizando branch main (a8f4c12)...\nRepositório pronto.",
            ),
            StepRun {
                name: "Reconstruir containers".into(),
                status: StepStatus::WaitingApproval,
                log: "[portão de aprovação] aguardando o maestro aprovar a reconstrução dos containers em produção...\nImagem alvo: ghcr.io/acme/erp-api:v2.14.0\nMigrações pendentes: 0".into(),
                secs: 0.0,
                substeps: Vec::new(),
                started_ts: Some(now - 89),
            },
            StepRun {
                name: "Validar healthchecks".into(),
                status: StepStatus::Pending,
                log: String::new(),
                secs: 0.0,
                substeps: Vec::new(),
                started_ts: None,
            },
        ],
        approved: false,
        cancel_requested: false,
        secs: 0.0,
        actor: "octocat".into(),
        repo_name: Some("octocat/erp-central".into()),
        branch: "main".into(),
        summary: String::new(),
        annotations: vec![Annotation {
            kind: AnnKind::Notice,
            job: "Reconstruir containers".into(),
            msg: "Portão obrigatório antes de reiniciar api, web e worker na VPS.".into(),
        }],
        vps_run_id: None,
        vps_server_id: None,
    };

    // 2. Running — CI Principal #692 (ao vivo, no projeto octocat/landing)
    let run_running = Run {
        id: "run-demo-running".into(),
        workflow_id: wf_ci.clone(),
        workflow_name: "CI Principal".into(),
        status: RunStatus::Running,
        trigger_label: "push".into(),
        title: "feat(faturamento): integra confirmação instantânea PIX e recibo PDF".into(),
        number: 692,
        commit: Some("e4d91a0b7c6d5e4f3a2b1c0d9e8f7a6b5c4d3e2f".into()),
        started_at: model::stamp_list(now - 38),
        started_ts: Some(now - 38),
        finished_at: None,
        steps: vec![
            sr(
                "Verificação de tipos e testes",
                24.0,
                "Executando pnpm -r typecheck...\n32 pacotes verificados sem erros.\nExecutando suíte de testes unitários...\n1.294 testes aprovados.",
            ),
            StepRun {
                name: "Varredura de segredos".into(),
                status: StepStatus::Running,
                log: "Executando gitleaks detect --source . --no-banner\nAnalisando 418 commits recentes...\nVerificando variáveis de ambiente e chaves...".into(),
                secs: 14.0,
                substeps: Vec::new(),
                started_ts: Some(now - 14),
            },
            StepRun {
                name: "Validação de contratos".into(),
                status: StepStatus::Pending,
                log: String::new(),
                secs: 0.0,
                substeps: Vec::new(),
                started_ts: None,
            },
        ],
        approved: false,
        cancel_requested: false,
        secs: 0.0,
        actor: "ana.silva".into(),
        repo_name: Some("octocat/landing".into()),
        branch: "feat/pix-recibo".into(),
        summary: String::new(),
        annotations: Vec::new(),
        vps_run_id: None,
        vps_server_id: None,
    };

    // 2b. Running — Deploy VPS #39 (última etapa, no projeto octocat/erp-central)
    let run_deploy_finishing = Run {
        id: "run-demo-deploy-39".into(),
        workflow_id: wf_deploy.clone(),
        workflow_name: "Deploy VPS".into(),
        status: RunStatus::Running,
        trigger_label: "workflow_run".into(),
        title: "Deploy de produção após CI #692 (release v2.15.0)".into(),
        number: 39,
        commit: Some("c4b81f3a2d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a".into()),
        started_at: model::stamp_list(now - 12),
        started_ts: Some(now - 12),
        finished_at: None,
        steps: vec![
            sr(
                "Preparar ambiente remoto",
                5.0,
                "Conectando via SSH em beacon-vps...\nRepositório atualizado (c4b81f3).",
            ),
            sr(
                "Reconstruir containers",
                42.0,
                "docker compose up -d --build api web worker\nImagens reconstruídas e containers reiniciados.",
            ),
            StepRun {
                name: "Validar saúde dos serviços".into(),
                status: StepStatus::Running,
                log: "Aguardando healthchecks...\napi: 200 OK\nweb: aguardando resposta...".into(),
                secs: 12.0,
                substeps: Vec::new(),
                started_ts: Some(now - 12),
            },
        ],
        approved: true,
        cancel_requested: false,
        secs: 0.0,
        actor: "octocat".into(),
        repo_name: Some("octocat/erp-central".into()),
        branch: "main".into(),
        summary: String::new(),
        annotations: Vec::new(),
        vps_run_id: None,
        vps_server_id: None,
    };

    // 3. AwaitingApproval 2 — Isolamento RLS #114
    let run_approval_2 = Run {
        id: "run-demo-approval-2".into(),
        workflow_id: wf_rls.clone(),
        workflow_name: "Isolamento RLS".into(),
        status: RunStatus::AwaitingApproval,
        trigger_label: "manual".into(),
        title: "Atualização de políticas multi-tenant (contratos e medições)".into(),
        number: 114,
        commit: Some("b73c90e1f2a3b4c5d6e7f8091a2b3c4d5e6f7a8b".into()),
        started_at: model::stamp_list(now - 240),
        started_ts: Some(now - 240),
        finished_at: None,
        steps: vec![
            sr(
                "Testes positivos de tenant",
                19.0,
                "Tenant A acessa seus contratos: OK\nTenant B bloqueado de ler dados de A: OK",
            ),
            StepRun {
                name: "Aplicar políticas em homologação".into(),
                status: StepStatus::WaitingApproval,
                log: "[portão de aprovação] aguardando liberação para aplicar migration RLS 0168 em homologação...".into(),
                secs: 0.0,
                substeps: Vec::new(),
                started_ts: Some(now - 220),
            },
        ],
        approved: false,
        cancel_requested: false,
        secs: 0.0,
        actor: "octocat".into(),
        repo_name: Some("acme/faturamento-pix".into()),
        branch: "main".into(),
        summary: String::new(),
        annotations: vec![Annotation {
            kind: AnnKind::Warning,
            job: "Aplicar políticas em homologação".into(),
            msg: "Altera políticas SELECT/INSERT na tabela medicoes_capex.".into(),
        }],
        vps_run_id: None,
        vps_server_id: None,
    };

    // 4. Failed — CI Principal #689
    let ts_err = "erro TS2345: Argumento do tipo 'string' não é atribuível ao parâmetro do tipo 'FiltroData'.";
    let run_failed = Run {
        id: "run-demo-failed".into(),
        workflow_id: wf_ci.clone(),
        workflow_name: "CI Principal".into(),
        status: RunStatus::Failed,
        trigger_label: "push".into(),
        title: "fix(arquivo,gis): corrige filtro SQL sem_ocr e ordenação por data".into(),
        number: 689,
        commit: Some("7db1e72a9b8c7d6e5f4a3b2c1d0e9f8a7b6c5d4e".into()),
        started_at: model::stamp_list(now - 1800),
        started_ts: Some(now - 1800),
        finished_at: Some(model::stamp_list(now - 1785)),
        steps: vec![
            StepRun {
                name: "Verificação de tipos e testes".into(),
                status: StepStatus::Failed,
                log: format!(
                    "Executando pnpm -r typecheck\nmodules/arquivo/src/filtros.ts(48,19): {ts_err}\n[passo falhou — o fluxo para aqui]"
                ),
                secs: 15.0,
                substeps: Vec::new(),
                started_ts: None,
            },
            StepRun {
                name: "Varredura de segredos".into(),
                status: StepStatus::Skipped,
                log: String::new(),
                secs: 0.0,
                substeps: Vec::new(),
                started_ts: None,
            },
            StepRun {
                name: "Validação de contratos".into(),
                status: StepStatus::Skipped,
                log: String::new(),
                secs: 0.0,
                substeps: Vec::new(),
                started_ts: None,
            },
        ],
        approved: false,
        cancel_requested: false,
        secs: 15.0,
        actor: "octocat".into(),
        repo_name: Some("octocat/w-workploy".into()),
        branch: "main".into(),
        summary: String::new(),
        annotations: vec![Annotation {
            kind: AnnKind::Error,
            job: "Verificação de tipos e testes".into(),
            msg: ts_err.into(),
        }],
        vps_run_id: None,
        vps_server_id: None,
    };

    // 5. Featured Success — CI Principal #671 (used in summary & logs scenes)
    let install_log = "\
Executando pnpm install --frozen-lockfile
Escopo: todos os 32 pacotes do workspace
Verificando lockfile contra políticas de segurança (996 pacotes)...
✓ Lockfile aprovado em 4,5 s
Progresso: resolvidos 811, reutilizados 110, baixados 695, adicionados 244
Dependências instaladas em 24 s";

    let mut job_build = sr("Verificação de tipos e testes", 412.0, "");
    job_build.substeps = vec![
        sr(
            "Preparar ambiente Node 24",
            4.0,
            "Configurando Node.js v24.11.0 (x64)...\nCache de pnpm restaurado com sucesso.",
        ),
        sr("Instalar dependências", 24.0, install_log),
        sr(
            "Checagem de tipos em todo o monorepo",
            148.0,
            "Executando pnpm -r typecheck\nEscopo: 32 de 32 projetos\npackages/core -> tsc --noEmit (0 erros)\nmodules/engenharia -> tsc --noEmit (0 erros)\nConcluído em 2 min 28 s",
        ),
        sr(
            "Executar suíte de testes (paralelo)",
            228.0,
            "Executando pnpm test:ci\n12 pacotes, 214 suítes\nPASS modules/engenharia/src/__tests__/medicao-capex.spec.ts\nPASS modules/financeiro/src/__tests__/cobranca-pix.spec.ts\nTestes: 1.287 aprovados, 1.287 no total\nTempo: 3 min 48 s",
        ),
        sr(
            "Smoke de prontidão local",
            8.0,
            "Executando node scripts/readiness-smoke.mjs\napi /healthz -> 200 OK\nweb / -> 200 OK\nfila worker -> pronta",
        ),
    ];

    let job_sec = sr(
        "Varredura de segredos",
        14.0,
        "Executando gitleaks detect --source . --no-banner\n0 vazamentos encontrados em 412 commits.\nAuditoria de dependências críticas: 0 vulnerabilidades.",
    );
    let job_contracts = sr(
        "Validação de contratos",
        27.0,
        "Executando pnpm --filter contracts typecheck\n148 schemas Zod exportados e compatíveis.\n0 quebras de contrato.",
    );

    let run_featured = Run {
        id: "run-demo-featured".into(),
        workflow_id: wf_ci.clone(),
        workflow_name: "CI Principal".into(),
        status: RunStatus::Success,
        trigger_label: "push".into(),
        title: "feat(painel): consolida interface OpenCode em Rune com pulso discreto".into(),
        number: 671,
        commit: Some("c06c3c1d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b".into()),
        started_at: model::stamp_list(now - 5400),
        started_ts: Some(now - 5400),
        finished_at: Some(model::stamp_list(now - 4947)),
        steps: vec![job_build, job_sec, job_contracts],
        approved: true,
        cancel_requested: false,
        secs: 453.0,
        actor: "octocat".into(),
        repo_name: Some("octocat/landing".into()),
        branch: "main".into(),
        summary: "\
# Resumo da execução #671 (c06c3c1)
- Pacotes validados: 32/32 | Suítes: 214 | Testes aprovados: 1.287
- Serviços prontos para encadeamento: api=true | web=true | worker=true
```
Sistema de arquivos  Tam  Usado  Disp  Uso%
/dev/sda1            96G   56G   41G   59%
Imagens Docker: 27 ativas (18,55 GB)
Containers saudáveis: 31/31 (api 200 · web 200 · worker ok)
```"
        .into(),
        annotations: vec![
            Annotation {
                kind: AnnKind::Notice,
                job: "Verificação de tipos e testes".into(),
                msg: "Cache primário restaurado com 100% de acerto (economizou 38 s).".into(),
            },
            Annotation {
                kind: AnnKind::Notice,
                job: "Smoke de prontidão local".into(),
                msg: "API, Web e Worker responderam HTTP 200 dentro do limite.".into(),
            },
            Annotation {
                kind: AnnKind::Warning,
                job: "Varredura de segredos".into(),
                msg: "Metadado de data ausente no pacote legado xlsx; verificação mínima ignorada.".into(),
            },
            Annotation {
                kind: AnnKind::Notice,
                job: "Validação de contratos".into(),
                msg: "148 contratos Zod validados sem alterações incompatíveis.".into(),
            },
        ],
        vps_run_id: None,
        vps_server_id: None,
    };

    // 6. Deploy VPS #37 Success
    let run_deploy_37 = Run {
        id: "run-demo-deploy-37".into(),
        workflow_id: wf_deploy.clone(),
        workflow_name: "Deploy VPS".into(),
        status: RunStatus::Success,
        trigger_label: "workflow_run".into(),
        title: "Deploy VPS — liberação automática v2.13.4".into(),
        number: 37,
        commit: Some("0915e2faa46aaee78c3f7c82ba72c87ec01c07d0".into()),
        started_at: model::stamp_list(now - 9600),
        started_ts: Some(now - 9600),
        finished_at: Some(model::stamp_list(now - 9007)),
        steps: vec![
            sr("Preparar ambiente remoto", 5.0, "Runner: beacon-vps (self-hosted)\nSSH OK"),
            sr(
                "Reconstruir containers",
                542.0,
                "Container acme-api atualizado\nContainer acme-web atualizado\nContainer acme-worker atualizado",
            ),
            sr("Validar healthchecks", 46.0, "healthcheck api: 200\nhealthcheck web: 200\nworker: ok"),
        ],
        approved: true,
        cancel_requested: false,
        secs: 593.0,
        actor: "octocat".into(),
        repo_name: Some("octocat/erp-central".into()),
        branch: "main".into(),
        summary: "Deploy VPS concluído em 9 min 53 s · todos os serviços saudáveis.".into(),
        annotations: Vec::new(),
        vps_run_id: None,
        vps_server_id: None,
    };

    // 7. Build Mobile #24 Cancelled
    let run_mobile_24 = Run {
        id: "run-demo-mobile-24".into(),
        workflow_id: wf_mobile.clone(),
        workflow_name: "Build Mobile".into(),
        status: RunStatus::Cancelled,
        trigger_label: "manual".into(),
        title: "Geração de pacote APK de homologação".into(),
        number: 24,
        commit: Some("91ab442000000000000000000000000000000000".into()),
        started_at: model::stamp_list(now - 14400),
        started_ts: Some(now - 14400),
        finished_at: Some(model::stamp_list(now - 14372)),
        steps: vec![StepRun {
            name: "Compilar APK de homologação".into(),
            status: StepStatus::Skipped,
            log: "[cancelado pelo maestro]".into(),
            secs: 28.0,
            substeps: Vec::new(),
            started_ts: None,
        }],
        approved: false,
        cancel_requested: true,
        secs: 28.0,
        actor: "ana.silva".into(),
        repo_name: Some("acme/mobile-app".into()),
        branch: "feat/mobile-offline".into(),
        summary: String::new(),
        annotations: Vec::new(),
        vps_run_id: None,
        vps_server_id: None,
    };

    s.runs = vec![
        run_approval_1,
        run_running,
        run_deploy_finishing,
        run_approval_2,
        run_failed,
        run_featured,
        run_deploy_37,
        run_mobile_24,
    ];

    // A receita "CI Principal" foi importada do repo e o yml mudou lá:
    // o aviso anti-drift acende no resumo do fluxo e no editor.
    s.workflow_drift = vec![(
        "wf-ci-principal".into(),
        "CI Principal".into(),
        "A receita mudou no repo — use Atualizar do repo.".into(),
    )];
}

pub fn scene_runs(s: &mut AppState) {
    *s = AppState::init_demo();
    s.navigate_to(NavTarget::Runs);
}

/// Visão de um fluxo importado do repo com aviso de drift: mostra o banner
/// amarelo, a linha de proveniência com "Atualizar do repo" e os botões de
/// importação do dashboard.
pub fn scene_wf_drift(s: &mut AppState) {
    *s = AppState::init_demo();
    s.selected_project_id = Some("rb-landing".into());
    s.selected_workflow_id = Some("wf-ci-principal".into());
    s.navigate_to(NavTarget::Runs);
}

pub fn scene_summary(s: &mut AppState) {
    *s = AppState::init_demo();
    s.open_run_summary("run-demo-featured");
}

pub fn scene_logs(s: &mut AppState) {
    *s = AppState::init_demo();
    s.open_run_logs("run-demo-featured", 0);
}

pub fn scene_approvals(s: &mut AppState) {
    *s = AppState::init_demo();
    s.navigate_to(NavTarget::Approvals);
}

pub fn scene_connections(s: &mut AppState) {
    *s = AppState::init_demo();
    s.navigate_to(NavTarget::Connections);
    s.settings_draft = SettingsDraft::from_settings(&s.snapshot.settings);
}

/// Cena de demonstração do editor "Configurar vínculo" (abas por assunto).
pub fn scene_binding_editor(s: &mut AppState) {
    scene_settings(s);
    if let Some(bid) = s
        .snapshot
        .settings
        .repo_bindings
        .first()
        .map(|b| b.id.clone())
    {
        s.open_edit_binding_modal(&bid);
    } else {
        s.open_new_binding_modal(None);
    }
}

pub fn scene_settings(s: &mut AppState) {
    *s = AppState::init_demo();
    s.open_settings(SettingsTab::GitHub);
    s.settings_draft = SettingsDraft::from_settings(&s.snapshot.settings);
}

pub fn scene_editor(s: &mut AppState) {
    *s = AppState::init_demo();
    s.navigate_to(NavTarget::Runs);
    if let Some(wf) = s.snapshot.workflows.get(1).cloned() {
        let extras = workploy::model::WorkflowExtras::default();
        let draft = WorkflowDraft::from_workflow(&wf, &extras, &mut s.next_uid);
        s.modal = ModalState::WorkflowEditor(Box::new(draft));
    }
}

pub fn scene_onboarding(s: &mut AppState) {
    *s = AppState::init_demo();
    s.snapshot.runs.clear();
    s.snapshot.settings.servers.clear();
    s.snapshot.settings.repo_bindings.clear();
    s.snapshot.settings.has_github_token = false;
    s.snapshot.settings.github_token.clear();
    s.snapshot.settings.github_user.clear();
    s.snapshot.settings.ssh_host.clear();
    s.snapshot.github_repos.clear();
    s.snapshot.github_status.clear();
    s.snapshot.runner_sync_status.clear();
    s.snapshot.settings.onboarding_completed = false;
    s.settings_draft = SettingsDraft::from_settings(&s.snapshot.settings);
    s.navigate_to(NavTarget::Onboarding);
}
