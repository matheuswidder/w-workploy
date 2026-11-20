use crate::model::{AnnKind, Annotation, Run, RunStatus, StepRun, StepStatus};

/// Traduz erros crus de SSH, HTTP, Git e sistema para mensagens claras em pt-BR,
/// mantendo o detalhe técnico resumido quando ajuda a agir.
pub fn humanize_transport_error(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "Falha de comunicação desconhecida.".to_string();
    }
    let low = trimmed.to_ascii_lowercase();

    if low.contains("permission denied (publickey")
        || low.contains("permission denied, please try again")
    {
        return format!(
            "Autenticação SSH recusada na VPS (chave ou senha inválida). Confira usuário, chave privada ou alias em ~/.ssh/config. Detalhe: {}",
            first_meaningful_line(trimmed)
        );
    }
    if low.contains("could not resolve hostname") || low.contains("name or service not known") {
        return format!(
            "Endereço ou alias SSH da VPS não encontrado. Verifique o host em Configurações → VPS ou ~/.ssh/config. Detalhe: {}",
            first_meaningful_line(trimmed)
        );
    }
    if low.contains("connection refused") {
        return format!(
            "Conexão recusada pela VPS (porta SSH/runner fechada ou serviço parado). Detalhe: {}",
            first_meaningful_line(trimmed)
        );
    }
    if low.contains("connection timed out") || low.contains("operation timed out") {
        return format!(
            "Tempo esgotado ao conectar na VPS — verifique IP, porta e regras de firewall. Detalhe: {}",
            first_meaningful_line(trimmed)
        );
    }
    if low.contains("host key verification failed") {
        return "Chave do servidor SSH mudou ou não consta em known_hosts. Reconecte manualmente via terminal uma vez ou atualize ~/.ssh/known_hosts.".to_string();
    }
    if low.contains("bad credentials") || low.contains("returned status code 401") {
        return "Token do GitHub inválido ou expirado (HTTP 401). Gere ou atualize o PAT em Configurações → GitHub.".to_string();
    }
    if low.contains("rate limit") || low.contains("returned status code 403") {
        return "Acesso negado ou limite da API do GitHub atingido (HTTP 403). Verifique as permissões do token (repo / read:org).".to_string();
    }
    if low.contains("returned status code 404") || low.contains("the requested url returned error: 404") {
        return "Recurso não encontrado (HTTP 404) — confira dono/repositório, branch ou endpoint do runner na VPS.".to_string();
    }
    if low.contains("no space left on device") {
        return "Disco cheio no servidor! Use 'Limpar agora' em Conexões & Diagnóstico ou libere espaço em /var/lib/workploy e Docker.".to_string();
    }
    if low.contains("not a git repository") {
        return "A pasta de destino não é um repositório Git válido.".to_string();
    }

    trimmed.to_string()
}

fn first_meaningful_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or("")
        .chars()
        .take(160)
        .collect()
}

fn active_substep_name(step: &StepRun) -> Option<String> {
    if let Some(sub) = step
        .substeps
        .iter()
        .rev()
        .find(|s| matches!(s.status, StepStatus::Failed | StepStatus::Running))
    {
        return Some(sub.name.clone());
    }
    for line in step.log.lines().rev() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("=== ")
            && let Some(name) = rest.strip_suffix(" ===")
        {
            return Some(name.trim().to_string());
        }
    }
    None
}

fn collect_matching_lines(log: &str, pred: impl Fn(&str, &str) -> bool, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    for raw in log.lines() {
        let t = raw.trim();
        if t.is_empty()
            || t.starts_with("=== ")
            || t.starts_with("[passo falhou")
            || t.starts_with("[git sync falhou")
        {
            continue;
        }
        let low = t.to_ascii_lowercase();
        if pred(t, &low) {
            let clipped: String = t.chars().take(180).collect();
            if !out.contains(&clipped) {
                out.push(clipped);
                if out.len() >= max {
                    break;
                }
            }
        }
    }
    out
}

/// Analisa o log de um passo que falhou e retorna:
/// `(resumo_humano_curto, annotations)` com causa provável, evidência e ação sugerida.
pub fn diagnose_step_failure(step: &StepRun) -> (String, Vec<Annotation>) {
    let substep = active_substep_name(step);
    let scope_log = step
        .substeps
        .iter()
        .rev()
        .find(|s| s.status == StepStatus::Failed)
        .map(|s| s.log.as_str())
        .filter(|l| !l.trim().is_empty())
        .unwrap_or(step.log.as_str());

    let low = scope_log.to_ascii_lowercase();
    let job_label = match &substep {
        Some(sub) => format!("{} › {}", step.name, sub),
        None => step.name.clone(),
    };

    let (headline, hint, evidence) = if low.contains("err_pnpm_no_pkg_manifest")
        || low.contains("no package.json")
    {
        (
            "Diretório sem package.json — o comando rodou fora da pasta do repositório.".to_string(),
            "Confira se o fluxo está vinculado a um projeto ou defina a Pasta de trabalho correta.",
            collect_matching_lines(scope_log, |_, l| l.contains("package.json") || l.contains("err_pnpm"), 3),
        )
    } else if low.contains("command not found") || low.contains("is not recognized as an internal") {
        let lines = collect_matching_lines(
            scope_log,
            |_, l| l.contains("command not found") || l.contains("not recognized"),
            3,
        );
        let missing_cmd = lines
            .first()
            .and_then(|l| {
                l.split(':')
                    .rev()
                    .nth(1)
                    .map(|s| s.trim().to_string())
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "ferramenta".to_string());
        (
            format!("Comando não encontrado no PATH do servidor ('{missing_cmd}')."),
            "Provisione a VPS (Configurações → VPS → Provisionar) ou ajuste o PATH do serviço/script.",
            lines,
        )
    } else if low.contains("tempo esgotado") || low.contains("connection timed out") {
        (
            "Sessão SSH interrompida por inatividade ou queda de conexão com a VPS.".to_string(),
            "Verifique se a VPS reiniciou ou se o comando ficou preso aguardando entrada interativa.",
            collect_matching_lines(scope_log, |_, l| l.contains("tempo esgotado") || l.contains("ssh"), 3),
        )
    } else if low.contains("permission denied (publickey")
        || low.contains("could not resolve hostname")
        || low.contains("connection refused")
        || low.contains("host key verification failed")
        || low.contains("[erro ssh]")
    {
        let lines = collect_matching_lines(
            scope_log,
            |_, l| {
                l.contains("permission denied")
                    || l.contains("could not resolve")
                    || l.contains("connection refused")
                    || l.contains("[erro ssh]")
            },
            3,
        );
        let msg = lines
            .first()
            .map(|l| humanize_transport_error(l))
            .unwrap_or_else(|| "Falha de conexão SSH com a VPS.".to_string());
        (msg, "Teste a conexão em Configurações → VPS → Testar conexão.", lines)
    } else if low.contains("killed") && (low.contains("137") || low.contains("out of memory") || low.contains("heap"))
        || low.contains("javascript heap out of memory")
    {
        (
            "Processo encerrado por falta de memória RAM (OOM / heap limit).".to_string(),
            "Reduza a concorrência (--concurrency=1), use NODE_OPTIONS=--max-old-space-size ou ative swap na VPS.",
            collect_matching_lines(scope_log, |_, l| l.contains("heap") || l.contains("killed") || l.contains("137"), 3),
        )
    } else if low.contains("no space left on device") {
        (
            "Disco da VPS sem espaço livre durante a execução.".to_string(),
            "Execute 'Limpar agora' em Conexões & Diagnóstico para limpar caches Docker e workspaces antigos.",
            collect_matching_lines(scope_log, |_, l| l.contains("no space left"), 3),
        )
    } else if low.contains("error ts") {
        let lines = collect_matching_lines(scope_log, |_, l| l.contains("error ts"), 4);
        (
            format!("Erro de tipagem TypeScript ({} ocorrência(s) capturada(s)).", lines.len().max(1)),
            "Corrija os erros de tipo indicados acima antes de prosseguir para o deploy.",
            lines,
        )
    } else if low.contains("fail ")
        || low.contains("assertionerror")
        || low.contains("test timed out")
        || low.contains("failed tests")
        || low.contains("eliffecycle")
        || low.contains("test failed")
    {
        let lines = collect_matching_lines(
            scope_log,
            |t, l| {
                t.starts_with("FAIL ")
                    || l.contains("assertionerror")
                    || l.contains("test timed out")
                    || l.contains("failed ")
                    || l.contains("err_assertion")
                    || l.contains("error:")
            },
            4,
        );
        (
            if let Some(sub) = &substep {
                format!("Falha nos testes/checagens na sub-etapa '{sub}'.")
            } else {
                format!("Falha nos testes ou validações em '{}'.", step.name)
            },
            "Abra os logs do job para ver o stacktrace completo e corrigir as suítes que falharam.",
            lines,
        )
    } else if low.contains("docker compose")
        || low.contains("copy failed")
        || low.contains("cannot connect to the docker daemon")
        || low.contains("configuration file provided: not found")
    {
        (
            "Falha no build ou subida dos containers Docker.".to_string(),
            "Verifique se o daemon Docker está ativo, se o docker-compose.yml existe no caminho certo e se há espaço em disco.",
            collect_matching_lines(
                scope_log,
                |_, l| {
                    l.contains("error")
                        || l.contains("failed")
                        || l.contains("not found")
                        || l.contains("docker")
                },
                4,
            ),
        )
    } else if low.contains("git clone")
        || low.contains("git fetch")
        || low.contains("git pull")
        || low.contains("authentication failed")
        || low.contains("[git sync falhou")
    {
        (
            "Falha ao sincronizar código do GitHub na VPS.".to_string(),
            "Verifique o token PAT do GitHub e as permissões de leitura do repositório.",
            collect_matching_lines(
                scope_log,
                |_, l| l.contains("fatal:") || l.contains("error:") || l.contains("git"),
                3,
            ),
        )
    } else {
        let lines = collect_matching_lines(
            scope_log,
            |_, l| {
                l.contains("erro")
                    || l.contains("error")
                    || l.contains("falhou")
                    || l.contains("failed")
                    || l.contains("fatal")
            },
            4,
        );
        let fallback_lines = if lines.is_empty() {
            scope_log
                .lines()
                .map(str::trim)
                .filter(|l| {
                    !l.is_empty()
                        && !l.starts_with("=== ")
                        && !l.starts_with("[passo falhou")
                })
                .rev()
                .take(3)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(|s| s.chars().take(180).collect::<String>())
                .collect()
        } else {
            lines
        };
        (
            if let Some(sub) = &substep {
                format!("Etapa '{}' falhou em '{}'.", step.name, sub)
            } else {
                format!("Etapa '{}' retornou código de erro.", step.name)
            },
            "Confira as linhas de erro extraídas abaixo ou abra os logs completos do job.",
            fallback_lines,
        )
    };

    let mut anns = Vec::new();
    let mut full_msg = format!("{headline}\nSugestão: {hint}");
    if !evidence.is_empty() {
        full_msg.push_str("\nEvidência:\n• ");
        full_msg.push_str(&evidence.join("\n• "));
    }
    anns.push(Annotation {
        kind: AnnKind::Error,
        job: job_label,
        msg: full_msg,
    });

    (headline, anns)
}

/// Constrói um resumo Markdown objetivo para `Run.summary` após conclusão ou falha.
pub fn build_run_summary(run: &Run, server_label: &str) -> String {
    let total = run.steps.len();
    let passed = run
        .steps
        .iter()
        .filter(|s| s.status == StepStatus::Success)
        .count();
    let failed_step = run.steps.iter().find(|s| s.status == StepStatus::Failed);

    let mut md = String::new();
    match run.status {
        RunStatus::Success => {
            md.push_str(&format!("# {} concluído com sucesso\n", run.workflow_name));
            md.push_str(&format!("- Destino: {server_label}\n"));
            md.push_str(&format!("- Etapas concluídas: {passed}/{total}\n"));
            if let Some(repo) = &run.repo_name {
                md.push_str(&format!("- Repositório: {repo} ({})\n", run.branch));
            }
        }
        RunStatus::Failed => {
            md.push_str(&format!("# Falha em {}\n", run.workflow_name));
            md.push_str(&format!("- Destino: {server_label}\n"));
            if let Some(fs) = failed_step {
                let (headline, _) = diagnose_step_failure(fs);
                md.push_str(&format!("- Etapa com falha: {}\n", fs.name));
                if let Some(sub) = active_substep_name(fs) {
                    md.push_str(&format!("- Sub-etapa: {sub}\n"));
                }
                md.push_str(&format!("- Motivo: {headline}\n"));
            }
        }
        _ => {}
    }
    md
}

/// Resumo de 1 linha para exibir direto na lista de execuções quando uma run falhou.
pub fn short_failure_reason(run: &Run) -> Option<String> {
    if run.status != RunStatus::Failed {
        return None;
    }
    let fs = run.steps.iter().find(|s| s.status == StepStatus::Failed)?;
    let (headline, _) = diagnose_step_failure(fs);
    let prefix = match active_substep_name(fs) {
        Some(sub) => format!("{} › {}", fs.name, sub),
        None => fs.name.clone(),
    };
    Some(format!("{prefix}: {headline}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanizes_common_ssh_and_http_errors() {
        assert!(
            humanize_transport_error("git@github.com: Permission denied (publickey).")
                .contains("Autenticação SSH recusada")
        );
        assert!(
            humanize_transport_error("ssh: Could not resolve hostname widder-bad: Name or service not known")
                .contains("não encontrado")
        );
        assert!(
            humanize_transport_error("GitHub returned status code 401")
                .contains("Token do GitHub inválido")
        );
    }

    #[test]
    fn diagnoses_vitest_typescript_and_missing_pkg_failures() {
        let ts_step = StepRun {
            name: "Gate completo".into(),
            status: StepStatus::Failed,
            log: "=== Typecheck em todos os pacotes ===\napps/web/src/App.tsx(42,10): error TS2345: Argument of type 'string' is not assignable\n".into(),
            secs: 12.0,
            substeps: vec![StepRun {
                name: "Typecheck em todos os pacotes".into(),
                status: StepStatus::Failed,
                log: "apps/web/src/App.tsx(42,10): error TS2345: Argument of type 'string' is not assignable\n".into(),
                secs: 12.0,
                substeps: Vec::new(),
                started_ts: None,
            }],
            started_ts: None,
        };
        let (headline, anns) = diagnose_step_failure(&ts_step);
        assert!(headline.contains("TypeScript"));
        assert_eq!(anns.len(), 1);
        assert!(anns[0].job.contains("Typecheck em todos os pacotes"));
        assert!(anns[0].msg.contains("error TS2345"));

        let missing_pkg = StepRun {
            name: "Instalar".into(),
            status: StepStatus::Failed,
            log: "ERR_PNPM_NO_PKG_MANIFEST  No package.json was found in /root".into(),
            secs: 1.0,
            substeps: Vec::new(),
            started_ts: None,
        };
        let (h2, _) = diagnose_step_failure(&missing_pkg);
        assert!(h2.contains("package.json"));
    }
}
