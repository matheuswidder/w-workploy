use crate::model::*;
use crate::vault;
use crate::vps;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type SharedHandle = Arc<Mutex<Shared>>;

#[cfg(target_os = "windows")]
pub fn apply_no_window(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW)
}

#[cfg(not(target_os = "windows"))]
pub fn apply_no_window(cmd: &mut Command) -> &mut Command {
    cmd
}

pub fn set_wake_hook(sh: &SharedHandle, wake: Option<WakeFn>) {
    sh.lock().unwrap().wake = wake;
}

pub(crate) fn notify(sh: &SharedHandle) {
    let wake = sh.lock().unwrap().wake.clone();
    if let Some(wake) = wake {
        wake();
    }
}

fn update_run(sh: &SharedHandle, id: &str, f: impl FnOnce(&mut Run)) {
    let mut s = sh.lock().unwrap();
    if let Some(r) = s.runs.iter_mut().find(|r| r.id == id) {
        f(r);
    }
    save_runs(&s);
    drop(s);
    notify(sh);
}

fn push_capped_log(buf: &mut String, chunk: &str) {
    buf.push_str(chunk);
    const CAP: usize = 120_000;
    if buf.len() > CAP {
        let mut cut = buf.len() - CAP;
        while cut < buf.len() && !buf.is_char_boundary(cut) {
            cut += 1;
        }
        buf.drain(..cut);
    }
}

pub(crate) fn apply_log_to_step(st: &mut StepRun, text: &str) {
    let now = now_ts();
    for seg in text.split_inclusive('\n') {
        let trimmed = seg.trim();
        let marker_cand = trimmed.trim_matches('"');
        if let Some(raw_name) = marker_cand.strip_prefix("::workploy-substep::") {
            let name = raw_name.trim();
            let sub_name = if name.is_empty() {
                "Sub-etapa".to_string()
            } else {
                name.to_string()
            };
            if let Some(prev) = st.substeps.last_mut()
                && prev.status == StepStatus::Running
            {
                prev.status = StepStatus::Success;
                prev.secs = prev.started_ts.map(|t| (now - t).max(0) as f64).unwrap_or(0.0);
            }
            st.substeps.push(StepRun {
                name: sub_name.clone(),
                status: StepStatus::Running,
                log: String::new(),
                secs: 0.0,
                substeps: Vec::new(),
                started_ts: Some(now),
            });
            push_capped_log(&mut st.log, &format!("=== {sub_name} ===\n"));
        } else {
            push_capped_log(&mut st.log, seg);
            if let Some(active) = st.substeps.last_mut()
                && active.status == StepStatus::Running
            {
                push_capped_log(&mut active.log, seg);
            }
        }
    }
}

pub(crate) fn finalize_step_substeps(st: &mut StepRun, final_status: StepStatus) {
    let now = now_ts();
    if let Some(last) = st.substeps.last_mut()
        && last.status == StepStatus::Running
    {
        last.status = final_status;
        last.secs = last.started_ts.map(|t| (now - t).max(0) as f64).unwrap_or(0.0);
    }
}

fn append_log(sh: &SharedHandle, run_id: &str, step_idx: usize, text: &str) {
    let mut s = sh.lock().unwrap();
    if let Some(r) = s.runs.iter_mut().find(|r| r.id == run_id)
        && let Some(st) = r.steps.get_mut(step_idx)
    {
        apply_log_to_step(st, text);
    }
    drop(s);
    notify(sh);
}

fn is_cancelled(sh: &SharedHandle, run_id: &str) -> bool {
    let s = sh.lock().unwrap();
    s.runs
        .iter()
        .find(|r| r.id == run_id)
        .map(|r| r.cancel_requested)
        .unwrap_or(false)
}

pub fn approve(sh: &SharedHandle, run_id: &str) {
    update_run(sh, run_id, |r| r.approved = true);
}

/// Destino do cancelamento na VPS para uma execução espelhada no desktop:
/// `(servidor dono, id da execução no runner)`. `None` quando a execução não
/// tem origem no runner de uma VPS — nesses casos o cancelamento local basta
/// (execuções que rodam passos via SSH direto morrem quando o loop de
/// `exec_step`/`ssh_run_streaming` derruba o cliente ssh ativo).
fn vps_cancel_target(s: &Shared, run_id: &str) -> Option<(VpsServer, String)> {
    let r = s.runs.iter().find(|r| r.id == run_id)?;
    let vps_run_id = r.vps_run_id.clone()?;
    let srv = s
        .settings
        .servers
        .iter()
        .find(|v| Some(&v.id) == r.vps_server_id.as_ref())?;
    Some((srv.clone(), vps_run_id))
}

/// Dispara, cada qual em sua thread, o `POST /runs/{id}/cancel` no runner da
/// VPS para cada destino resolvido. Falhas só registram status — nunca
/// bloqueiam o cancelamento local já marcado.
fn spawn_vps_cancel(sh: &SharedHandle, cancels: Vec<(VpsServer, String)>) {
    for (server, vps_run_id) in cancels {
        let sh2 = sh.clone();
        std::thread::spawn(move || {
            let _ = vps::cancel_vps_run(&sh2, &server, &vps_run_id);
        });
    }
}

pub fn cancel(sh: &SharedHandle, run_id: &str) {
    // Cancelar uma execução espelhada precisa avisar a VPS: só a flag local
    // deixaria o run remoto rodando até o fim. A flag é marcada primeiro
    // (efeito imediato na UI), e o pedido viaja ao runner em segundo plano.
    let target = {
        let mut s = sh.lock().unwrap();
        let target = vps_cancel_target(&s, run_id);
        if let Some(r) = s.runs.iter_mut().find(|r| r.id == run_id) {
            r.cancel_requested = true;
        }
        save_runs(&s);
        drop(s);
        target
    };
    notify(sh);
    if let Some((server, vps_run_id)) = target {
        spawn_vps_cancel(sh, vec![(server, vps_run_id)]);
    }
}

pub fn clear_finished_runs(sh: &SharedHandle) -> usize {
    let removed = {
        let mut s = sh.lock().unwrap();
        let before = s.runs.len();
        s.runs.retain(|r| r.status.active());
        let removed = before.saturating_sub(s.runs.len());
        save_runs(&s);
        removed
    };
    notify(sh);
    removed
}

pub fn start_run(sh: &SharedHandle, workflow_id: &str, trigger_label: &str) -> Option<String> {
    start_run_full(sh, workflow_id, trigger_label, None)
}

pub fn start_run_full(
    sh: &SharedHandle,
    workflow_id: &str,
    trigger_label: &str,
    commit: Option<(String, String)>,
) -> Option<String> {
    start_run_with_target(sh, workflow_id, trigger_label, commit, None, None, None)
}

pub fn start_run_with_target(
    sh: &SharedHandle,
    workflow_id: &str,
    trigger_label: &str,
    commit: Option<(String, String)>,
    target_server_override: Option<String>,
    repo_name: Option<String>,
    branch: Option<String>,
) -> Option<String> {
    let wf = {
        let s = sh.lock().unwrap();
        s.workflows.iter().find(|w| w.id == workflow_id).cloned()
    }?;
    let number = {
        let s = sh.lock().unwrap();
        s.runs
            .iter()
            .filter(|r| r.workflow_id == workflow_id)
            .count() as u64
            + 1
    };
    let (commit_sha, title) = match &commit {
        Some((sha, msg)) => (
            Some(sha.clone()),
            msg.lines().next().unwrap_or(trigger_label).to_string(),
        ),
        None => (None, trigger_label.to_string()),
    };
    let clean_repo = repo_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let clean_branch = branch
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("main")
        .to_string();
    let run = Run {
        id: new_id("run"),
        workflow_id: wf.id.clone(),
        workflow_name: wf.name.clone(),
        status: RunStatus::Queued,
        trigger_label: trigger_label.to_string(),
        title,
        number,
        commit: commit_sha,
        started_at: stamp_list(now_ts()),
        started_ts: Some(now_ts()),
        finished_at: None,
        steps: wf
            .steps
            .iter()
            .map(|s| StepRun {
                name: s.name.clone(),
                status: StepStatus::Pending,
                log: String::new(),
                secs: 0.0,
                substeps: Vec::new(),
                started_ts: None,
            })
            .collect(),
        approved: false,
        cancel_requested: false,
        secs: 0.0,
        actor: "maestro".into(),
        repo_name: clean_repo.clone(),
        branch: clean_branch.clone(),
        summary: String::new(),
        annotations: Vec::new(),
        vps_run_id: None,
        vps_server_id: None,
    };
    let run_id = run.id.clone();
    let commit_sha_for_exec = run.commit.clone();
    let mut vps_cancels: Vec<(VpsServer, String)> = Vec::new();
    {
        let mut guard = sh.lock().unwrap();
        // Split borrow: s.runs mutável + s.settings somente leitura.
        let s = &mut *guard;
        if let Some(ref repo) = clean_repo {
            for r in &mut s.runs {
                if r.status.active()
                    && r.workflow_id == wf.id
                    && r.repo_name
                        .as_deref()
                        .is_some_and(|existing| existing.eq_ignore_ascii_case(repo))
                    && r.branch == clean_branch
                {
                    r.cancel_requested = true;
                    // Execução espelhada: o aviso precisa chegar ao runner da
                    // VPS, senão ela continua rodando lá.
                    if let (Some(sid), Some(vid)) = (r.vps_server_id.clone(), r.vps_run_id.clone())
                        && let Some(srv) = s.settings.servers.iter().find(|v| v.id == sid)
                    {
                        vps_cancels.push((srv.clone(), vid));
                    }
                }
            }
        }
        s.runs.insert(0, run);
        save_runs(s);
    }
    notify(sh);
    spawn_vps_cancel(sh, vps_cancels);
    let sh2 = sh.clone();
    let rid = run_id.clone();
    std::thread::spawn(move || {
        execute_run(
            sh2,
            rid,
            wf,
            target_server_override,
            clean_repo,
            Some(clean_branch),
            commit_sha_for_exec,
        )
    });
    Some(run_id)
}

pub fn start_run_for_binding(
    sh: &SharedHandle,
    binding_id: &str,
    trigger_label: &str,
) -> Option<String> {
    let (binding, workflows, all_bindings) = {
        let s = sh.lock().unwrap();
        let b = s
            .settings
            .repo_bindings
            .iter()
            .find(|x| x.id == binding_id)
            .cloned()?;
        (b, s.workflows.clone(), s.settings.repo_bindings.clone())
    };
    let wf_ids = matching_workflows_for_binding_with_bindings(&workflows, &binding, &all_bindings);
    let primary_branch = crate::github::expand_branch_targets(&binding.branch, "main")
        .into_iter()
        .next()
        .unwrap_or_else(|| "main".into());
    let mut first_run_id = None;
    for wf_id in wf_ids {
        if let Some(rid) = start_run_with_target(
            sh,
            &wf_id,
            trigger_label,
            None,
            binding.target_server_id.clone(),
            Some(binding.full_name.clone()),
            Some(primary_branch.clone()),
        ) && first_run_id.is_none()
        {
            first_run_id = Some(rid);
        }
    }
    first_run_id
}

pub fn start_run_for_binding_commit(
    sh: &SharedHandle,
    binding_id: &str,
    sha: &str,
    message: &str,
    branch: &str,
) -> Option<String> {
    let (binding, workflows, all_bindings) = {
        let s = sh.lock().unwrap();
        let b = s
            .settings
            .repo_bindings
            .iter()
            .find(|x| x.id == binding_id)
            .cloned()?;
        (b, s.workflows.clone(), s.settings.repo_bindings.clone())
    };
    let wf_ids = matching_workflows_for_binding_with_bindings(&workflows, &binding, &all_bindings);
    let short: String = sha.chars().take(7).collect();
    let trigger_label = if short.is_empty() {
        format!("commit em {branch}")
    } else {
        format!("commit {short} ({branch})")
    };
    let mut first_run_id = None;
    for wf_id in wf_ids {
        if let Some(rid) = start_run_with_target(
            sh,
            &wf_id,
            &trigger_label,
            Some((sha.to_string(), message.to_string())),
            binding.target_server_id.clone(),
            Some(binding.full_name.clone()),
            Some(branch.to_string()),
        ) && first_run_id.is_none()
        {
            first_run_id = Some(rid);
        }
    }
    first_run_id
}

fn finish_cancelled(
    sh: &SharedHandle,
    run_id: &str,
    from: usize,
    elapsed: f64,
    step_secs: Option<f64>,
) {
    update_run(sh, run_id, |r| {
        r.status = RunStatus::Cancelled;
        r.finished_at = Some(now_str());
        r.secs = elapsed;
        if let Some(secs) = step_secs
            && let Some(step) = r.steps.get_mut(from)
        {
            step.secs = secs;
        }
        for st in r.steps.iter_mut().skip(from) {
            if matches!(
                st.status,
                StepStatus::Pending | StepStatus::WaitingApproval | StepStatus::Running
            ) {
                finalize_step_substeps(st, StepStatus::Skipped);
                st.status = StepStatus::Skipped;
            }
        }
    });
}

pub fn safe_slug(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_dash = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        "workspace".to_string()
    } else {
        trimmed
    }
}

fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

fn redact_between(text: &mut String, prefix: &str, stop_at: char, stop_before_slash: bool) {
    let mut from = 0usize;
    while let Some(rel) = text[from..].find(prefix) {
        let start = from + rel + prefix.len();
        let rest = &text[start..];
        if let Some(at_rel) = rest.find(stop_at) {
            let segment = &rest[..at_rel];
            if !stop_before_slash
                || (!segment.contains('/') && !segment.chars().any(char::is_whitespace))
            {
                text.replace_range(start..start + at_rel, "***");
                from = start + 3 + 1;
                continue;
            }
        }
        from = start;
    }
}

pub fn mask_secrets_in_log(text: &str, known_tokens: &[&str]) -> String {
    let mut out = text.to_string();
    for tok in known_tokens {
        let t = tok.trim();
        if !t.is_empty() {
            out = out.replace(t, "***");
        }
    }
    redact_between(&mut out, "x-access-token:", '@', true);
    redact_between(&mut out, "https://", '@', true);
    out
}

pub fn build_vps_sync_script(
    repo: &str,
    branch: &str,
    commit_sha: Option<&str>,
) -> (String, String) {
    let clean_repo = repo.trim();
    let wd = format!("/var/lib/workploy/workspaces/{}", safe_slug(clean_repo));
    let clean_branch = crate::github::expand_branch_targets(branch, "main")
        .into_iter()
        .next()
        .unwrap_or_else(|| "main".into());
    let wd_q = shell_single_quote(&wd);
    let repo_q = shell_single_quote(clean_repo);
    let branch_q = shell_single_quote(&clean_branch);
    let checkout_part = if let Some(sha) = commit_sha.map(str::trim).filter(|s| !s.is_empty()) {
        let sha_q = shell_single_quote(sha);
        format!("git -C \"$WD\" checkout --force {sha_q}")
    } else {
        format!(
            "(git -C \"$WD\" checkout {branch_q} 2>/dev/null || git -C \"$WD\" checkout -b {branch_q} \"origin/\"{branch_q}) && git -C \"$WD\" pull --ff-only origin {branch_q}"
        )
    };

    let script = format!(
        "set -e\n\
         WS_ROOT='/var/lib/workploy/workspaces'\n\
         WD={wd_q}\n\
         REPO={repo_q}\n\
         mkdir -p \"$WS_ROOT\"\n\
         TOK=\"\"\n\
         if [ -r /etc/workploy/github_token ]; then\n\
           TOK=$(tr -d '\\r\\n' < /etc/workploy/github_token)\n\
         elif command -v sudo >/dev/null 2>&1 && sudo -n test -r /etc/workploy/github_token 2>/dev/null; then\n\
           TOK=$(sudo -n cat /etc/workploy/github_token 2>/dev/null | tr -d '\\r\\n')\n\
         fi\n\
         if [ -n \"$TOK\" ]; then\n\
           REMOTE_URL=\"https://x-access-token:${{TOK}}@github.com/${{REPO}}.git\"\n\
         else\n\
           REMOTE_URL=\"https://github.com/${{REPO}}.git\"\n\
         fi\n\
         if [ ! -d \"$WD/.git\" ]; then\n\
           git clone \"$REMOTE_URL\" \"$WD\"\n\
         else\n\
           git -C \"$WD\" remote set-url origin \"$REMOTE_URL\" 2>/dev/null || true\n\
         fi\n\
         git -C \"$WD\" fetch origin\n\
         {checkout_part}\n\
         echo \"[git sync] workspace=$WD ref=$(git -C \"$WD\" rev-parse --short HEAD 2>/dev/null || echo '?')\""
    );
    (wd, script)
}

pub fn build_vps_sync_command(
    repo: &str,
    branch: &str,
    commit_sha: Option<&str>,
) -> (String, String) {
    let (wd, script) = build_vps_sync_script(repo, branch, commit_sha);
    (wd, format!("sh -c {}", shell_single_quote(&script)))
}

fn run_vps_sync_step(
    sh: &SharedHandle,
    run_id: &str,
    srv: &VpsServer,
    sync_cmd: &str,
    known_token: &str,
) -> (bool, bool, f64) {
    let start = Instant::now();
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let done_flag = Arc::new(AtomicBool::new(false));
    let ok = std::thread::scope(|scope| {
        let cf = cancel_flag.clone();
        let df = done_flag.clone();
        scope.spawn(move || {
            while !df.load(Ordering::Relaxed) {
                if is_cancelled(sh, run_id) {
                    cf.store(true, Ordering::Relaxed);
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let res = vps::ssh_run_streaming(srv, sync_cmd, Some(&cancel_flag), vps::ssh_idle_timeout(), |line| {
            let masked = mask_secrets_in_log(line, &[known_token]);
            append_log(sh, run_id, 0, &format!("{masked}\n"));
        });
        done_flag.store(true, Ordering::Relaxed);
        match res {
            Ok(success) => success,
            Err(e) => {
                let masked = mask_secrets_in_log(&e, &[known_token]);
                let human = crate::diagnostic::humanize_transport_error(&masked);
                append_log(sh, run_id, 0, &format!("[erro SSH] {human}\n"));
                false
            }
        }
    });
    let secs = start.elapsed().as_secs_f64();
    let cancelled = cancel_flag.load(Ordering::Relaxed);
    if cancelled {
        append_log(sh, run_id, 0, "[cancelado pelo usuário]\n");
    } else if !ok {
        append_log(sh, run_id, 0, "[git sync falhou — o fluxo para aqui]\n");
    }
    (ok, cancelled, secs)
}

fn execute_run(
    sh: SharedHandle,
    run_id: String,
    wf: Workflow,
    target_server_override: Option<String>,
    repo_name: Option<String>,
    branch: Option<String>,
    commit_sha: Option<String>,
) {
    execute_run_inner(
        sh,
        run_id,
        wf,
        target_server_override,
        repo_name,
        branch,
        commit_sha,
    );
    // Persistência final garantida: o último estado do run (sucesso, falha
    // ou cancelamento) chega ao disco antes desta thread encerrar — a
    // gravação de rotina é assíncrona, mas o fim de run espera o flush.
    crate::model::flush_runs_to_disk();
}

fn execute_run_inner(
    sh: SharedHandle,
    run_id: String,
    wf: Workflow,
    target_server_override: Option<String>,
    repo_name: Option<String>,
    branch: Option<String>,
    commit_sha: Option<String>,
) {
    let total = Instant::now();
    // Extras da receita: limites de CPU/RAM aplicados na VPS e passos a pular.
    let extras: crate::model::WorkflowExtras = {
        let s = sh.lock().unwrap();
        s.workflow_extras.get(&wf.id).cloned().unwrap_or_default()
    };
    // Quando o destino é uma VPS com o daemon `workploy-runner` instalado e
    // ativo, delega a execução ao runner 24/7 via `POST /trigger`: o build
    // sobrevive se o desktop dormir/fechar, respeita concorrência na VPS e
    // o desktop acompanha logs e sub-etapas ao vivo via auto-sync. Se o
    // daemon não responder, cai de forma transparente para SSH streaming direto.
    let daemon_candidate = {
        let s = sh.lock().unwrap();
        let override_sid = target_server_override
            .as_deref()
            .map(str::trim)
            .filter(|x| !x.is_empty());
        let trigger_label = s
            .runs
            .iter()
            .find(|r| r.id == run_id)
            .map(|r| r.trigger_label.clone())
            .unwrap_or_else(|| "manual".to_string());
        if let Some(sid) = override_sid {
            s.settings
                .servers
                .iter()
                .find(|v| v.id == sid)
                .filter(|v| v.runner_installed && !v.host.trim().is_empty())
                .map(|v| (v.clone(), trigger_label))
        } else if wf.runner == RunnerKind::Ssh {
            wf.server_id
                .as_ref()
                .and_then(|sid| s.settings.servers.iter().find(|v| &v.id == sid))
                .filter(|v| v.runner_installed && !v.host.trim().is_empty())
                .map(|v| (v.clone(), trigger_label))
        } else {
            None
        }
    };
    if let Some((srv, trig_lbl)) = daemon_candidate
        && !wf.steps.is_empty()
    {
        append_log(
            &sh,
            &run_id,
            0,
            &format!(
                "[workploy-runner] enviando execução para o daemon 24/7 em '{}'...\n",
                srv.name
            ),
        );
        match vps::trigger_vps_run(
            &sh,
            &srv,
            &wf,
            &trig_lbl,
            repo_name.as_deref(),
            branch.as_deref(),
            commit_sha.as_deref(),
        ) {
            Ok(remote_run) => {
                update_run(&sh, &run_id, |r| {
                    r.vps_run_id = Some(remote_run.id.clone());
                    r.vps_server_id = Some(srv.id.clone());
                    r.actor = "workploy-runner".into();
                    r.status = remote_run.status;
                    r.number = remote_run.number;
                    if !remote_run.steps.is_empty() {
                        r.steps = remote_run.steps;
                    }
                });
                return;
            }
            Err(err) => {
                append_log(
                    &sh,
                    &run_id,
                    0,
                    &format!(
                        "[aviso] daemon workploy-runner indisponível ({err}) — executando via SSH direto...\n"
                    ),
                );
            }
        }
    }

    let mut synced_vps_workdir: Option<String> = None;
    for (i, step) in wf.steps.iter().enumerate() {
        if is_cancelled(&sh, &run_id) {
            finish_cancelled(&sh, &run_id, i, total.elapsed().as_secs_f64(), None);
            return;
        }
        // Passo marcado como "Pular" na receita: ignorado sem quebrar o fluxo.
        if extras.skips(&step.name) {
            update_run(&sh, &run_id, |r| {
                if let Some(st) = r.steps.get_mut(i) {
                    st.status = StepStatus::Skipped;
                    st.secs = 0.0;
                }
            });
            append_log(
                &sh,
                &run_id,
                i,
                "[pulado] este passo está marcado como 'Pular' na receita.\n",
            );
            continue;
        }
        if step.approval {
            update_run(&sh, &run_id, |r| {
                r.status = RunStatus::AwaitingApproval;
                if let Some(st) = r.steps.get_mut(i) {
                    st.status = StepStatus::WaitingApproval;
                }
            });
            append_log(
                &sh,
                &run_id,
                i,
                "[portão de aprovação] aguardando o maestro aprovar este passo...\n",
            );
            loop {
                std::thread::sleep(Duration::from_millis(200));
                let s = sh.lock().unwrap();
                let r = s.runs.iter().find(|r| r.id == run_id);
                match r {
                    Some(r) if r.cancel_requested => {
                        drop(s);
                        finish_cancelled(&sh, &run_id, i, total.elapsed().as_secs_f64(), None);
                        return;
                    }
                    Some(r) if r.approved => {
                        drop(s);
                        break;
                    }
                    None => {
                        drop(s);
                        return;
                    }
                    _ => {}
                }
            }
            append_log(&sh, &run_id, i, "[aprovado] continuando.\n");
        }
        update_run(&sh, &run_id, |r| {
            r.status = RunStatus::Running;
            if let Some(st) = r.steps.get_mut(i) {
                st.status = StepStatus::Running;
                st.started_ts = Some(now_ts());
            }
        });
        let (runner, ssh_host, target_server, mut workdir, github_token) = {
            let s = sh.lock().unwrap();
            let override_sid = target_server_override
                .as_deref()
                .map(str::trim)
                .filter(|x| !x.is_empty());
            let tok = s.settings.github_token.clone();
            if let Some(sid) = override_sid {
                let srv = s.settings.servers.iter().find(|v| v.id == sid).cloned();
                (
                    RunnerKind::Ssh,
                    s.settings.ssh_host.clone(),
                    srv,
                    wf.workdir.clone(),
                    tok,
                )
            } else {
                let srv = wf
                    .server_id
                    .as_ref()
                    .and_then(|sid| s.settings.servers.iter().find(|v| &v.id == sid).cloned());
                (
                    wf.runner,
                    s.settings.ssh_host.clone(),
                    srv,
                    wf.workdir.clone(),
                    tok,
                )
            }
        };
        if runner == RunnerKind::Ssh
            && target_server.is_none()
            && ssh_host.trim().is_empty()
        {
            append_log(
                &sh,
                &run_id,
                i,
                "[erro de configuração] Este fluxo exige execução em VPS (self-hosted), mas nenhuma VPS está configurada no vínculo do projeto ou no fluxo. Selecione a VPS em Configurações → GitHub → Vínculo.\n",
            );
            update_run(&sh, &run_id, |r| {
                if let Some(st) = r.steps.get_mut(i) {
                    finalize_step_substeps(st, StepStatus::Failed);
                    st.status = StepStatus::Failed;
                    let (_headline, anns) = crate::diagnostic::diagnose_step_failure(st);
                    r.annotations.extend(anns);
                }
                r.status = RunStatus::Failed;
                r.finished_at = Some(now_str());
                r.secs = total.elapsed().as_secs_f64();
                for st in r.steps.iter_mut().skip(i + 1) {
                    if st.status == StepStatus::Pending {
                        st.status = StepStatus::Skipped;
                    }
                }
                r.summary = crate::diagnostic::build_run_summary(r, "VPS não configurada");
            });
            return;
        }
        let step_server_label = target_server
            .as_ref()
            .map(|srv| srv.name.clone())
            .unwrap_or_else(|| {
                if runner == RunnerKind::Ssh {
                    ssh_host.clone()
                } else {
                    "Local (Esta máquina)".to_string()
                }
            });
        if let Some(srv) = target_server.as_ref() {
            let sid = srv.id.clone();
            update_run(&sh, &run_id, |r| {
                if r.vps_server_id.is_none() {
                    r.vps_server_id = Some(sid);
                }
            });
        }
        if runner == RunnerKind::Ssh
            && let Some(srv) = target_server.as_ref()
            && let Some(repo) = repo_name.as_deref().map(str::trim).filter(|s| !s.is_empty())
            && wf.workdir.trim().is_empty()
        {
            if let Some(ref existing_wd) = synced_vps_workdir {
                workdir = existing_wd.clone();
            } else {
                let (effective_wd, sync_cmd) = build_vps_sync_command(
                    repo,
                    branch.as_deref().unwrap_or("main"),
                    commit_sha.as_deref(),
                );
                workdir = effective_wd.clone();
                if i == 0 {
                    append_log(
                        &sh,
                        &run_id,
                        0,
                        &format!("[git sync] preparando workspace {effective_wd} em '{}'...\n", srv.name),
                    );
                    let (sync_ok, cancelled, sync_secs) =
                        run_vps_sync_step(&sh, &run_id, srv, &sync_cmd, &github_token);
                    if cancelled || is_cancelled(&sh, &run_id) {
                        finish_cancelled(
                            &sh,
                            &run_id,
                            0,
                            total.elapsed().as_secs_f64(),
                            Some(sync_secs),
                        );
                        return;
                    }
                    if !sync_ok {
                        update_run(&sh, &run_id, |r| {
                            if let Some(st) = r.steps.get_mut(0) {
                                st.secs = sync_secs;
                                st.status = StepStatus::Failed;
                                let (_headline, anns) = crate::diagnostic::diagnose_step_failure(st);
                                r.annotations.extend(anns);
                            }
                            r.status = RunStatus::Failed;
                            r.finished_at = Some(now_str());
                            r.secs = total.elapsed().as_secs_f64();
                            for st in r.steps.iter_mut().skip(1) {
                                if st.status == StepStatus::Pending {
                                    st.status = StepStatus::Skipped;
                                }
                            }
                            r.summary = crate::diagnostic::build_run_summary(r, &step_server_label);
                        });
                        return;
                    }
                    synced_vps_workdir = Some(effective_wd);
                }
            }
        }
        let (ok, secs) = exec_step(
            &sh,
            &run_id,
            i,
            runner,
            &ssh_host,
            target_server.as_ref(),
            &workdir,
            &step.run,
            &github_token,
            repo_name.as_deref(),
            branch.as_deref(),
            extras.cpu_limit,
            extras.mem_limit_mb,
        );
        if is_cancelled(&sh, &run_id) {
            finish_cancelled(
                &sh,
                &run_id,
                i,
                total.elapsed().as_secs_f64(),
                Some(secs),
            );
            return;
        }
        update_run(&sh, &run_id, |r| {
            if let Some(st) = r.steps.get_mut(i) {
                st.secs = secs;
                let final_status = if ok {
                    StepStatus::Success
                } else {
                    StepStatus::Failed
                };
                finalize_step_substeps(st, final_status);
                st.status = final_status;
                if !ok {
                    let (_headline, anns) = crate::diagnostic::diagnose_step_failure(st);
                    r.annotations.extend(anns);
                }
            }
            if !ok {
                r.status = RunStatus::Failed;
                r.finished_at = Some(now_str());
                r.secs = total.elapsed().as_secs_f64();
                for st in r.steps.iter_mut().skip(i + 1) {
                    if st.status == StepStatus::Pending {
                        st.status = StepStatus::Skipped;
                    }
                }
                r.summary = crate::diagnostic::build_run_summary(r, &step_server_label);
            }
        });
        if !ok {
            return;
        }
    }
    let final_server_label = {
        let s = sh.lock().unwrap();
        let override_sid = target_server_override
            .as_deref()
            .map(str::trim)
            .filter(|x| !x.is_empty());
        let sid = override_sid.or(wf.server_id.as_deref());
        sid.and_then(|id| s.settings.servers.iter().find(|v| v.id == id))
            .map(|v| v.name.clone())
            .unwrap_or_else(|| {
                if wf.runner == RunnerKind::Ssh {
                    "VPS via SSH".to_string()
                } else {
                    "Local (Esta máquina)".to_string()
                }
            })
    };
    update_run(&sh, &run_id, |r| {
        r.status = RunStatus::Success;
        r.finished_at = Some(now_str());
        r.secs = total.elapsed().as_secs_f64();
        if r.summary.is_empty() {
            r.summary = crate::diagnostic::build_run_summary(r, &final_server_label);
        }
    });
    if let Some(next) = &wf.on_success
        && next != &wf.id
    {
        start_run_with_target(
            &sh,
            next,
            "encadeado: fluxo anterior concluído",
            None,
            target_server_override,
            repo_name,
            branch,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn exec_step(
    sh: &SharedHandle,
    run_id: &str,
    idx: usize,
    runner: RunnerKind,
    ssh_host: &str,
    target_server: Option<&VpsServer>,
    workdir: &str,
    cmd: &str,
    known_token: &str,
    repo_name: Option<&str>,
    branch: Option<&str>,
    cpu_limit: Option<u32>,
    mem_limit_mb: Option<u32>,
) -> (bool, f64) {
    let start = Instant::now();
    if cmd.trim().is_empty() {
        append_log(sh, run_id, idx, "[passo sem comando — nada a fazer]\n");
        return (true, 0.0);
    }

    if runner == RunnerKind::Ssh
        && let Some(srv) = target_server
    {
        let workdir_q = shell_single_quote(workdir.trim());
        let repo_q = shell_single_quote(repo_name.unwrap_or("").trim());
        let branch_q = shell_single_quote(branch.unwrap_or("").trim());
        let inner_q = shell_single_quote(cmd);
        // Limites de recursos na VPS:
        //  - `taskset` prende o passo em N vCPUs (deixa os demais para a
        //    produção que convive na máquina);
        //  - `systemd-run --scope` cria um cgroup com teto de memória, o que
        //    dá um OOM previsível em vez de o servidor inteiro trocar.
        let mut limits_desc: Vec<String> = Vec::new();
        let mut limits_setup = String::new();
        if let Some(cores) = cpu_limit.filter(|c| *c > 0) {
            limits_setup.push_str(&format!("export WORKPLOY_CPU_LIMIT={cores}; "));
            limits_desc.push(format!("{cores} vCPU"));
        }
        if let Some(mb) = mem_limit_mb.filter(|m| *m > 0) {
            limits_setup.push_str(&format!("export WORKPLOY_MEM_LIMIT_MB={mb}; "));
            limits_desc.push(format!("{mb} MB RAM"));
        }
        let limits_apply = if limits_setup.is_empty() {
            String::new()
        } else {
            "CPUS_TOTAL=$(nproc 2>/dev/null || echo 1); LIMCPU=''; \
             if [ -n \"${WORKPLOY_CPU_LIMIT:-}\" ] && [ \"$CPUS_TOTAL\" -gt \"$WORKPLOY_CPU_LIMIT\" ]; then LIMCPU=\"taskset -c 0-$((WORKPLOY_CPU_LIMIT-1))\"; fi; \
             LIMMEM=''; if [ -n \"${WORKPLOY_MEM_LIMIT_MB:-}\" ] && command -v systemd-run >/dev/null 2>&1; then LIMMEM=\"systemd-run --scope --quiet -p MemoryMax=${WORKPLOY_MEM_LIMIT_MB}M --\"; fi; \
             "
                .to_string()
        };
        let cd_part = if workdir.trim().is_empty() {
            String::new()
        } else {
            format!("cd {workdir_q} && ")
        };
        let body = format!(
            "{cd_part}{limits_setup}{limits_apply}$LIMCPU $LIMMEM /bin/bash -lc {inner_q}"
        );
        let wrapped = format!(
            "export HOME=\"${{HOME:-/root}}\"; \
             export PATH=\"/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:$HOME/.local/share/pnpm:/usr/local/share/pnpm:$HOME/.cargo/bin:/usr/local/cargo/bin:$PATH\"; \
             if [ -d \"$HOME/.nvm/versions/node\" ]; then NVM_BIN=$(ls -d \"$HOME/.nvm/versions/node\"/*/bin 2>/dev/null | tail -n 1); [ -n \"$NVM_BIN\" ] && export PATH=\"$NVM_BIN:$PATH\"; fi; \
             export WORKPLOY_WORKDIR='{workdir_q}'; \
             export WORKPLOY_REPO='{repo_q}'; \
             export WORKPLOY_BRANCH='{branch_q}'; \
             {body}"
        );
        if !limits_desc.is_empty() {
            append_log(
                sh,
                run_id,
                idx,
                &format!(
                    "[limites] aplicado neste passo: {}\n",
                    limits_desc.join(" · ")
                ),
            );
        }
        let remote_cmd = format!(
            "if [ -x /bin/bash ]; then /bin/bash -lc {}; else sh -c {}; fi",
            shell_single_quote(&wrapped),
            shell_single_quote(&wrapped)
        );
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let done_flag = Arc::new(AtomicBool::new(false));
        let ok = std::thread::scope(|scope| {
            let cf = cancel_flag.clone();
            let df = done_flag.clone();
            scope.spawn(move || {
                while !df.load(Ordering::Relaxed) {
                    if is_cancelled(sh, run_id) {
                        cf.store(true, Ordering::Relaxed);
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            });
            let res = vps::ssh_run_streaming(srv, &remote_cmd, Some(&cancel_flag), vps::ssh_idle_timeout(), |line| {
                let masked = mask_secrets_in_log(line, &[known_token]);
                append_log(sh, run_id, idx, &format!("{masked}\n"));
            });
            done_flag.store(true, Ordering::Relaxed);
            match res {
                Ok(success) => success,
                Err(e) => {
                    let masked = mask_secrets_in_log(&e, &[known_token]);
                    let human = crate::diagnostic::humanize_transport_error(&masked);
                    append_log(sh, run_id, idx, &format!("[erro SSH] {human}\n"));
                    false
                }
            }
        });
        let secs = start.elapsed().as_secs_f64();
        if cancel_flag.load(Ordering::Relaxed) {
            append_log(sh, run_id, idx, "[cancelado pelo usuário]\n");
            return (false, secs);
        }
        if !ok {
            append_log(sh, run_id, idx, "[passo falhou — o fluxo para aqui]\n");
        }
        return (ok, secs);
    }

    let mut command = match runner {
        RunnerKind::Ssh => {
            let mut c = Command::new("ssh");
            apply_no_window(&mut c);
            // Keepalive explícito (mesma razão do build_ssh_command): conexão
            // que cai silenciosamente no meio do passo não pode deixar o ssh
            // pendurado para sempre.
            c.arg("-o")
                .arg("BatchMode=yes")
                .arg("-o")
                .arg("ConnectTimeout=15")
                .arg("-o")
                .arg("ServerAliveInterval=15")
                .arg("-o")
                .arg("ServerAliveCountMax=4")
                .arg(ssh_host)
                .arg(cmd);
            c
        }
        RunnerKind::Local => {
            if cfg!(windows) {
                let mut c = Command::new("cmd");
                apply_no_window(&mut c);
                c.arg("/C").arg(cmd);
                c
            } else {
                let mut c = Command::new("sh");
                apply_no_window(&mut c);
                c.arg("-c").arg(cmd);
                c
            }
        }
    };
    if runner == RunnerKind::Local && !workdir.is_empty() && Path::new(workdir).is_dir() {
        command.current_dir(workdir);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            append_log(
                sh,
                run_id,
                idx,
                &format!("[erro] não foi possível iniciar o comando: {e}\n"),
            );
            return (false, start.elapsed().as_secs_f64());
        }
    };
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        let sh2 = sh.clone();
        let rid = run_id.to_string();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(out).lines() {
                match line {
                    Ok(l) => append_log(&sh2, &rid, idx, &format!("{l}\n")),
                    Err(_) => break,
                }
            }
        }));
    }
    if let Some(err) = child.stderr.take() {
        let sh2 = sh.clone();
        let rid = run_id.to_string();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(err).lines() {
                match line {
                    Ok(l) => append_log(&sh2, &rid, idx, &format!("{l}\n")),
                    Err(_) => break,
                }
            }
        }));
    }
    let mut cancelled = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {}
            Err(_) => break None,
        }
        if is_cancelled(sh, run_id) {
            cancelled = true;
        }
        if cancelled {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    for h in readers {
        let _ = h.join();
    }
    let secs = start.elapsed().as_secs_f64();
    if cancelled {
        append_log(sh, run_id, idx, "[cancelado pelo usuário]\n");
        return (false, secs);
    }
    let ok = status.map(|s| s.success()).unwrap_or(false);
    if !ok {
        append_log(sh, run_id, idx, "[passo falhou — o fluxo para aqui]\n");
    }
    (ok, secs)
}

pub fn spawn_ssh_test(sh: SharedHandle) {
    let host = {
        let mut s = sh.lock().unwrap();
        if s.ssh_testing {
            return;
        }
        s.ssh_testing = true;
        s.ssh_test = None;
        s.settings.ssh_host.clone()
    };
    notify(&sh);
    std::thread::spawn(move || {
        let mut cmd = Command::new("ssh");
        apply_no_window(&mut cmd);
        let out = cmd
            .arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("ConnectTimeout=8")
            .arg(&host)
            .arg("echo workploy-ok")
            .output();
        let (ok, msg) = match out {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
                let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
                if o.status.success() {
                    (true, format!("Conectou em '{host}': {stdout}"))
                } else {
                    (false, format!("Falhou: {stderr}"))
                }
            }
            Err(e) => (false, format!("Não foi possível chamar o ssh: {e}")),
        };
        let mut s = sh.lock().unwrap();
        s.ssh_testing = false;
        s.ssh_test = Some((ok, msg));
        drop(s);
        notify(&sh);
    });
}

fn is_windows_workdir(workdir: &str) -> bool {
    let w = workdir.trim();
    w.contains(":\\") || w.contains(":/")
}

fn is_windows_only_workflow(w: &Workflow) -> bool {
    is_windows_workdir(&w.workdir)
        || w.name.to_ascii_lowercase().contains("windows")
        || w.steps
            .iter()
            .any(|st| st.run.to_ascii_lowercase().contains("powershell"))
}

pub fn matching_workflows_for_binding(workflows: &[Workflow], binding: &RepoBinding) -> Vec<String> {
    matching_workflows_for_binding_with_bindings(workflows, binding, &[])
}

pub fn matching_workflows_for_binding_with_bindings(
    workflows: &[Workflow],
    binding: &RepoBinding,
    all_bindings: &[RepoBinding],
) -> Vec<String> {
    if let Some(wid) = &binding.workflow_id
        && workflows.iter().any(|w| &w.id == wid)
    {
        return vec![wid.clone()];
    }
    let by_repo_wfs: Vec<&Workflow> = workflows
        .iter()
        .filter(|w| w.repo_id.as_deref() == Some(binding.id.as_str()))
        .collect();
    if !by_repo_wfs.is_empty() {
        let sourced_push_wfs: Vec<String> = by_repo_wfs
            .iter()
            .filter(|w| w.trigger == Trigger::Push && w.source.is_some())
            .map(|w| w.id.clone())
            .collect();
        if !sourced_push_wfs.is_empty() {
            return sourced_push_wfs;
        }
        let push_wfs: Vec<String> = by_repo_wfs
            .iter()
            .filter(|w| w.trigger == Trigger::Push)
            .map(|w| w.id.clone())
            .collect();
        if !push_wfs.is_empty() {
            return push_wfs;
        }
        let chained_targets: std::collections::HashSet<&str> = by_repo_wfs
            .iter()
            .filter_map(|w| w.on_success.as_deref())
            .collect();
        let roots: Vec<String> = by_repo_wfs
            .iter()
            .filter(|w| !chained_targets.contains(w.id.as_str()))
            .map(|w| w.id.clone())
            .collect();
        if !roots.is_empty() {
            return roots;
        }
        return by_repo_wfs.into_iter().map(|w| w.id.clone()).collect();
    }
    let is_vps_target = binding.target_server_id.is_some();
    workflows
        .iter()
        .filter(|w| {
            if w.trigger != Trigger::Push {
                return false;
            }
            if w.repo_id
                .as_deref()
                .map(str::trim)
                .is_some_and(|rid| !rid.is_empty())
            {
                return false;
            }
            let claimed_by_other_binding = all_bindings.iter().any(|other| {
                other.id != binding.id && other.workflow_id.as_deref() == Some(w.id.as_str())
            });
            if claimed_by_other_binding {
                return false;
            }
            if is_vps_target && is_windows_only_workflow(w) {
                return false;
            }
            true
        })
        .map(|w| w.id.clone())
        .collect()
}

pub fn poll_binding_step(
    sh: &SharedHandle,
    binding_id: &str,
    sha: &str,
    message: &str,
) -> Vec<(String, String)> {
    let short = sha[..7.min(sha.len())].to_string();
    let mut to_trigger: Vec<(String, String)> = Vec::new();
    {
        let mut s = sh.lock().unwrap();
        let workflows = s.workflows.clone();
        let all_bindings = s.settings.repo_bindings.clone();
        if let Some(b) = s.settings.repo_bindings.iter_mut().find(|x| x.id == binding_id) {
            let repo_name = b.full_name.clone();
            if b.last_sha.is_empty() {
                b.last_sha = sha.to_string();
                b.last_status = format!("Monitorando — commit atual {short}");
            } else if b.last_sha != sha {
                b.last_sha = sha.to_string();
                b.last_status = format!("Novo commit {short} — disparando execução");
                for wf_id in
                    matching_workflows_for_binding_with_bindings(&workflows, b, &all_bindings)
                {
                    to_trigger.push((wf_id, format!("push em {repo_name} ({short}): {message}")));
                }
            } else {
                b.last_status = format!("Sem alterações ({short})");
            }
            save_settings(&s);
        }
    }
    notify(sh);
    to_trigger
}

pub fn poll_binding_branch_step(
    sh: &SharedHandle,
    branch_shas: &mut std::collections::HashMap<(String, String), String>,
    binding_id: &str,
    branch: &str,
    total_branches: usize,
    sha: &str,
    message: &str,
) -> Vec<(String, String)> {
    if total_branches <= 1 {
        return poll_binding_step(sh, binding_id, sha, message);
    }
    let short = sha[..7.min(sha.len())].to_string();
    let key = (binding_id.to_string(), branch.to_string());
    let prev = branch_shas.get(&key).cloned();
    let mut to_trigger: Vec<(String, String)> = Vec::new();
    {
        let mut s = sh.lock().unwrap();
        let workflows = s.workflows.clone();
        let all_bindings = s.settings.repo_bindings.clone();
        if let Some(b) = s.settings.repo_bindings.iter_mut().find(|x| x.id == binding_id) {
            let repo_name = b.full_name.clone();
            match prev {
                None => {
                    branch_shas.insert(key, sha.to_string());
                    b.last_sha = sha.to_string();
                    b.last_status =
                        format!("Monitorando {total_branches} branches — {branch}: {short}");
                }
                Some(ref old) if old != sha => {
                    branch_shas.insert(key, sha.to_string());
                    b.last_sha = sha.to_string();
                    b.last_status =
                        format!("Novo commit {short} em {branch} — disparando execução");
                    for wf_id in
                        matching_workflows_for_binding_with_bindings(&workflows, b, &all_bindings)
                    {
                        to_trigger.push((
                            wf_id,
                            format!("push em {repo_name}@{branch} ({short}): {message}"),
                        ));
                    }
                }
                Some(_) => {
                    b.last_status =
                        format!("Sem alterações em {total_branches} branches ({branch}: {short})");
                }
            }
            save_settings(&s);
        }
    }
    notify(sh);
    to_trigger
}

fn fetch_branch_head_sha(repo: &str, branch: &str, token: &str) -> Result<(String, String), String> {
    let url = format!("https://api.github.com/repos/{repo}/commits/{branch}");
    let mut req = ureq::get(&url)
        .set("User-Agent", "workploy")
        .set("Accept", "application/vnd.github+json");
    if !token.trim().is_empty() {
        req = req.set("Authorization", &format!("Bearer {}", token.trim()));
    }
    let resp = req.call().map_err(|e| match e {
        ureq::Error::Status(code, _) => format!("GitHub retornou erro HTTP {code}."),
        ureq::Error::Transport(t) => format!("Não foi possível consultar o GitHub: {t}"),
    })?;
    let v = resp
        .into_json::<serde_json::Value>()
        .map_err(|_| "Não foi possível ler a resposta do GitHub.".to_string())?;
    let sha = v
        .get("sha")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    if sha.is_empty() {
        return Err("Resposta do GitHub sem commit — confira repositório/branch.".into());
    }
    let message = v
        .pointer("/commit/message")
        .and_then(|x| x.as_str())
        .unwrap_or("commit")
        .to_string();
    Ok((sha, message))
}

pub fn spawn_github_poller(sh: SharedHandle) {
    std::thread::spawn(move || {
        let mut branch_shas: std::collections::HashMap<(String, String), String> =
            std::collections::HashMap::new();
        let mut last_check = Instant::now()
            .checked_sub(Duration::from_secs(3600))
            .unwrap_or_else(Instant::now);
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let (
                legacy_enabled,
                legacy_repo,
                legacy_branch,
                mem_token,
                every,
                desktop_bindings,
                default_branches,
            ) = {
                let s = sh.lock().unwrap();
                let bindings: Vec<RepoBinding> = s
                    .settings
                    .repo_bindings
                    .iter()
                    .filter(|b| {
                        b.enabled
                            && b.trigger_mode == TriggerMode::PollOnDesktop
                            && !b.full_name.trim().is_empty()
                    })
                    .cloned()
                    .collect();
                let defs: Vec<(String, String)> = s
                    .github_repos
                    .iter()
                    .map(|r| (r.full_name.to_ascii_lowercase(), r.default_branch.clone()))
                    .collect();
                (
                    s.settings.poll_enabled,
                    s.settings.github_repo.clone(),
                    s.settings.github_branch.clone(),
                    s.settings.github_token.clone(),
                    s.settings.poll_secs.max(15),
                    bindings,
                    defs,
                )
            };

            let token = if !mem_token.trim().is_empty() {
                mem_token
            } else {
                vault::load_secret(vault::KEY_GITHUB_PAT)
                    .ok()
                    .flatten()
                    .unwrap_or_default()
            };

            let has_legacy = legacy_enabled && !legacy_repo.is_empty() && !token.is_empty();
            let has_bindings = !desktop_bindings.is_empty();

            if !has_legacy && !has_bindings {
                let mut s = sh.lock().unwrap();
                if s.github_status.is_empty() || s.github_status.starts_with("Sondagem") {
                    s.github_status = if !legacy_enabled {
                        "Sondagem desligada.".into()
                    } else {
                        "Configure o repositório e o token para ligar.".into()
                    };
                }
                drop(s);
                notify(&sh);
                continue;
            }

            if last_check.elapsed() < Duration::from_secs(every) {
                continue;
            }
            last_check = Instant::now();

            if has_legacy {
                match fetch_branch_head_sha(&legacy_repo, &legacy_branch, &token) {
                    Ok((sha, message)) => {
                        let short = sha[..7.min(sha.len())].to_string();
                        let mut to_trigger: Vec<String> = Vec::new();
                        {
                            let mut s = sh.lock().unwrap();
                            if s.last_sha.is_empty() {
                                s.last_sha = sha.clone();
                                s.github_status =
                                    format!("Monitorando GitHub — commit atual {short}");
                            } else if s.last_sha != sha {
                                s.last_sha = sha.clone();
                                s.github_status =
                                    format!("Novo commit {short} — disparando fluxos de push");
                                to_trigger = s
                                    .workflows
                                    .iter()
                                    .filter(|w| w.trigger == Trigger::Push)
                                    .map(|w| w.id.clone())
                                    .collect();
                            } else {
                                s.github_status =
                                    format!("Sem mudanças no GitHub (commit {short})");
                            }
                        }
                        notify(&sh);
                        for id in to_trigger {
                            start_run_full(
                                &sh,
                                &id,
                                "push no GitHub",
                                Some((sha.clone(), message.clone())),
                            );
                        }
                    }
                    Err(msg) => {
                        let mut s = sh.lock().unwrap();
                        s.github_status = msg;
                        drop(s);
                        notify(&sh);
                    }
                }
            }

            for b in desktop_bindings {
                let def_branch = default_branches
                    .iter()
                    .find(|(name, _)| *name == b.full_name.trim().to_ascii_lowercase())
                    .map(|(_, br)| br.as_str())
                    .unwrap_or("main");
                let branches = crate::github::expand_branch_targets(&b.branch, def_branch);
                let total_branches = branches.len();
                for target_branch in branches {
                    match fetch_branch_head_sha(b.full_name.trim(), &target_branch, &token) {
                        Ok((sha, message)) => {
                            let triggers = poll_binding_branch_step(
                                &sh,
                                &mut branch_shas,
                                &b.id,
                                &target_branch,
                                total_branches,
                                &sha,
                                &message,
                            );
                            for (wf_id, _summary) in triggers {
                                start_run_with_target(
                                    &sh,
                                    &wf_id,
                                    "push no GitHub",
                                    Some((sha.clone(), message.clone())),
                                    b.target_server_id.clone(),
                                    Some(b.full_name.clone()),
                                    Some(target_branch.clone()),
                                );
                            }
                        }
                        Err(err) => {
                            let mut s = sh.lock().unwrap();
                            if let Some(entry) =
                                s.settings.repo_bindings.iter_mut().find(|x| x.id == b.id)
                            {
                                entry.last_status = if total_branches > 1 {
                                    format!("{target_branch}: {err}")
                                } else {
                                    err
                                };
                            }
                            drop(s);
                            notify(&sh);
                        }
                    }
                }
            }
            // Anti-drift: 1 consulta por receita com proveniência de repo;
            // registra aviso quando o yml no repo mudou desde a importação.
            crate::github::check_workflow_source_drift(&sh);
        }
    });
}

fn folder_signature(root: &Path) -> Option<(u64, u64)> {
    fn walk(dir: &Path, depth: usize, count: &mut u64, newest: &mut u64) {
        if depth > 5 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name == ".git" || name == "target" || name == "node_modules" || name == ".next" {
                continue;
            }
            if path.is_dir() {
                walk(&path, depth + 1, count, newest);
            } else if let Ok(meta) = e.metadata() {
                *count += 1;
                if let Ok(m) = meta.modified()
                    && let Ok(d) = m.duration_since(std::time::UNIX_EPOCH)
                {
                    let n = d.as_nanos() as u64;
                    if n > *newest {
                        *newest = n;
                    }
                }
            }
        }
    }
    if !root.is_dir() {
        return None;
    }
    let mut count = 0u64;
    let mut newest = 0u64;
    walk(root, 0, &mut count, &mut newest);
    Some((count, newest))
}

pub fn spawn_folder_watcher(sh: SharedHandle) {
    std::thread::spawn(move || {
        let mut prev: Option<(u64, u64)> = None;
        loop {
            std::thread::sleep(Duration::from_secs(3));
            let (enabled, folder) = {
                let s = sh.lock().unwrap();
                (s.settings.watch_enabled, s.settings.watch_folder.clone())
            };
            if !enabled || folder.is_empty() {
                prev = None;
                continue;
            }
            let sig = folder_signature(Path::new(&folder));
            let changed = match (prev, sig) {
                (Some(p), Some(c)) => p != c,
                _ => false,
            };
            prev = sig;
            if changed {
                let ids: Vec<String> = {
                    let s = sh.lock().unwrap();
                    s.workflows
                        .iter()
                        .filter(|w| w.trigger == Trigger::Folder)
                        .map(|w| w.id.clone())
                        .collect()
                };
                for id in ids {
                    start_run(&sh, &id, "mudança na pasta vigiada");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_status(sh: &SharedHandle, run_id: &str, want: RunStatus, ms: u64) -> bool {
        let start = Instant::now();
        while (start.elapsed().as_millis() as u64) < ms {
            {
                let s = sh.lock().unwrap();
                if let Some(r) = s.runs.iter().find(|r| r.id == run_id) {
                    if r.status == want {
                        return true;
                    }
                    if matches!(r.status, RunStatus::Failed | RunStatus::Cancelled)
                        && want != r.status
                    {
                        return false;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    fn echo_workflow(id: &str, approval: bool, on_success: Option<String>) -> Workflow {
        Workflow {
            id: id.into(),
            name: format!("teste {id}"),
            trigger: Trigger::Manual,
            runner: RunnerKind::Local,
            workdir: String::new(),
            steps: vec![Step {
                name: "echo".into(),
                run: "echo workploy-test-ok".into(),
                approval,
            }],
            on_success,
            server_id: None,
            repo_id: None,
            source: None,
        }
    }

    #[test]
    fn wake_hook_covers_mutations_and_runs_unlocked() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let sh = Arc::new(Mutex::new(Shared::default()));
        sh.lock()
            .unwrap()
            .workflows
            .push(echo_workflow("wake", true, None));
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        set_wake_hook(
            &sh,
            Some(Arc::new(move || {
                c.fetch_add(1, Ordering::SeqCst);
            })),
        );
        let id = start_run(&sh, "wake", "manual").unwrap();
        assert!(wait_status(&sh, &id, RunStatus::AwaitingApproval, 5000));
        let before = count.load(Ordering::SeqCst);
        append_log(&sh, &id, 0, "teste");
        assert!(count.load(Ordering::SeqCst) > before);
        let before = count.load(Ordering::SeqCst);
        cancel(&sh, &id);
        assert!(count.load(Ordering::SeqCst) > before);
        let before = count.load(Ordering::SeqCst);
        approve(&sh, &id);
        assert!(count.load(Ordering::SeqCst) > before);
        assert!(wait_status(&sh, &id, RunStatus::Cancelled, 5000));
    }

    #[test]
    fn cancellation_during_running_records_duration() {
        let sh = Arc::new(Mutex::new(Shared::default()));
        let mut wf = echo_workflow("long", false, None);
        wf.steps[0].run = if cfg!(windows) {
            "ping -n 5 127.0.0.1 > nul"
        } else {
            "sleep 5"
        }
        .into();
        sh.lock().unwrap().workflows.push(wf);
        let id = start_run(&sh, "long", "manual").unwrap();
        assert!(wait_status(&sh, &id, RunStatus::Running, 5000));
        std::thread::sleep(Duration::from_millis(350));
        {
            let s = sh.lock().unwrap();
            let r = &s.runs[0];
            assert!(r.elapsed_secs() >= 0.0);
            assert!(r.steps[0].elapsed_secs() >= 0.0);
        }
        cancel(&sh, &id);
        assert!(wait_status(&sh, &id, RunStatus::Cancelled, 10000));
        let s = sh.lock().unwrap();
        let r = &s.runs[0];
        assert!(r.secs >= 0.3);
        assert!(r.steps[0].secs >= 0.3);
    }

    #[test]
    fn run_local_echo_success() {
        let sh: SharedHandle = Arc::new(Mutex::new(Shared::default()));
        {
            let mut s = sh.lock().unwrap();
            s.workflows.push(echo_workflow("wf-a", false, None));
        }
        let rid = start_run(&sh, "wf-a", "teste").expect("run criada");
        assert!(
            wait_status(&sh, &rid, RunStatus::Success, 20_000),
            "execução deveria terminar com sucesso"
        );
        let s = sh.lock().unwrap();
        let r = s.runs.iter().find(|r| r.id == rid).unwrap();
        assert!(
            r.steps[0].log.contains("workploy-test-ok"),
            "log deveria conter a saída do comando"
        );
        assert!(r.secs > 0.0 && r.steps[0].secs > 0.0);
        assert_eq!(r.elapsed_secs(), r.secs);
    }

    #[test]
    fn matching_workflows_by_repo_prefers_push_over_manual_recipes() {
        let workflows = vec![
            Workflow {
                id: "wf-ci".into(),
                name: "CI Oktopus".into(),
                trigger: Trigger::Push,
                runner: RunnerKind::Ssh,
                workdir: String::new(),
                steps: vec![],
                on_success: Some("wf-deploy".into()),
                server_id: Some("vps-1".into()),
                repo_id: Some("rb-erp".into()),
                source: None,
            },
            Workflow {
                id: "wf-deploy".into(),
                name: "Deploy Oktopus VPS".into(),
                trigger: Trigger::Manual,
                runner: RunnerKind::Ssh,
                workdir: String::new(),
                steps: vec![],
                on_success: None,
                server_id: Some("vps-1".into()),
                repo_id: Some("rb-erp".into()),
                source: None,
            },
            Workflow {
                id: "wf-smoke".into(),
                name: "Smoke Produção Oktopus".into(),
                trigger: Trigger::Manual,
                runner: RunnerKind::Ssh,
                workdir: String::new(),
                steps: vec![],
                on_success: None,
                server_id: Some("vps-1".into()),
                repo_id: Some("rb-erp".into()),
                source: None,
            },
        ];
        let binding = RepoBinding {
            id: "rb-erp".into(),
            full_name: "matheuswidder/w-erp-central".into(),
            target_server_id: Some("vps-1".into()),
            branch: "main".into(),
            trigger_mode: TriggerMode::WebhookOnVps,
            workflow_id: None,
            enabled: true,
            ..RepoBinding::default()
        };
        let matched = matching_workflows_for_binding(&workflows, &binding);
        assert_eq!(matched, vec!["wf-ci"]);
    }

    #[test]
    fn failing_step_stops_run() {
        let sh: SharedHandle = Arc::new(Mutex::new(Shared::default()));
        {
            let mut s = sh.lock().unwrap();
            let mut wf = echo_workflow("wf-fail", false, None);
            wf.steps[0].run = "exit 1".into();
            wf.steps.push(Step {
                name: "nunca roda".into(),
                run: "echo nao".into(),
                approval: false,
            });
            s.workflows.push(wf);
        }
        let rid = start_run(&sh, "wf-fail", "teste").unwrap();
        assert!(wait_status(&sh, &rid, RunStatus::Failed, 20_000));
        let s = sh.lock().unwrap();
        let r = s.runs.iter().find(|r| r.id == rid).unwrap();
        assert_eq!(r.steps[1].status, StepStatus::Skipped);
    }

    #[test]
    fn approval_gate_then_chain_redeploy() {
        let sh: SharedHandle = Arc::new(Mutex::new(Shared::default()));
        {
            let mut s = sh.lock().unwrap();
            s.workflows
                .push(echo_workflow("wf-ci", false, Some("wf-deploy".into())));
            s.workflows.push(echo_workflow("wf-deploy", true, None));
        }
        let rid = start_run(&sh, "wf-ci", "teste").unwrap();
        assert!(
            wait_status(&sh, &rid, RunStatus::Success, 20_000),
            "CI deveria concluir"
        );
        let deploy_id = {
            let start = Instant::now();
            loop {
                {
                    let s = sh.lock().unwrap();
                    if let Some(r) = s.runs.iter().find(|r| r.workflow_id == "wf-deploy") {
                        break r.id.clone();
                    }
                }
                assert!(
                    start.elapsed().as_secs() < 20,
                    "redeploy encadeado não apareceu"
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        assert!(
            wait_status(&sh, &deploy_id, RunStatus::AwaitingApproval, 20_000),
            "redeploy deveria parar aguardando aprovação"
        );
        approve(&sh, &deploy_id);
        assert!(
            wait_status(&sh, &deploy_id, RunStatus::Success, 20_000),
            "aprovado, o redeploy deveria concluir"
        );
    }

    #[test]
    fn cancel_rejected_run() {
        let sh: SharedHandle = Arc::new(Mutex::new(Shared::default()));
        {
            let mut s = sh.lock().unwrap();
            s.workflows.push(echo_workflow("wf-c", true, None));
        }
        let rid = start_run(&sh, "wf-c", "teste").unwrap();
        assert!(wait_status(&sh, &rid, RunStatus::AwaitingApproval, 20_000));
        cancel(&sh, &rid);
        assert!(wait_status(&sh, &rid, RunStatus::Cancelled, 20_000));
    }

    fn mk_mirror_run(id: &str, status: RunStatus, vps_server_id: Option<&str>) -> Run {
        Run {
            id: id.into(),
            workflow_id: "wf-1".into(),
            workflow_name: "CI VPS".into(),
            status,
            trigger_label: "webhook".into(),
            title: "run vps".into(),
            number: 1,
            commit: None,
            started_at: "18/11/2026 09:00".into(),
            started_ts: None,
            finished_at: None,
            steps: Vec::new(),
            approved: false,
            cancel_requested: false,
            secs: 0.0,
            actor: "workploy-runner".into(),
            repo_name: None,
            branch: "main".into(),
            summary: String::new(),
            annotations: Vec::new(),
            vps_run_id: Some(id.into()),
            vps_server_id: vps_server_id.map(str::to_string),
        }
    }

    fn test_server(id: &str) -> VpsServer {
        VpsServer {
            id: id.into(),
            name: format!("VPS {id}"),
            // Host vazio: o SSH falha na hora, sem nenhum acesso de rede no teste.
            host: String::new(),
            port: 22,
            user: String::new(),
            auth: AuthMethod::SshConfigAlias,
            last_probe: None,
            runner_installed: true,
            runner_port: 8787,
            webhook_secret_key: None,
        }
    }

    #[test]
    fn cancel_resolves_vps_target_and_marks_flag_immediately() {
        let sh: SharedHandle = Arc::new(Mutex::new(Shared::default()));
        {
            let mut s = sh.lock().unwrap();
            s.settings.servers.push(test_server("vps-x"));
            s.runs
                .push(mk_mirror_run("run-vps-abc-1", RunStatus::Running, Some("vps-x")));
            s.runs.push(Run {
                id: "run-local-1".into(),
                vps_run_id: None,
                vps_server_id: None,
                ..mk_mirror_run("run-local-1", RunStatus::Running, None)
            });
        }

        // Run espelhado resolve o destino do cancelamento na VPS.
        let (srv, rid) = {
            let s = sh.lock().unwrap();
            vps_cancel_target(&s, "run-vps-abc-1").expect("espelho deve resolver destino")
        };
        assert_eq!(srv.id, "vps-x");
        assert_eq!(rid, "run-vps-abc-1");

        // Run puramente local (mesmo rodando passos via SSH direto) não tem
        // destino no runner: o cancel local já mata o cliente ssh do passo.
        {
            let s = sh.lock().unwrap();
            assert!(vps_cancel_target(&s, "run-local-1").is_none());
        }

        // Cancel marca a flag imediatamente; a propagação à VPS roda em thread.
        cancel(&sh, "run-vps-abc-1");
        let s = sh.lock().unwrap();
        assert!(s
            .runs
            .iter()
            .find(|r| r.id == "run-vps-abc-1")
            .unwrap()
            .cancel_requested);
    }

    #[test]
    fn start_run_superseding_mirror_requests_vps_cancel_too() {
        let sh: SharedHandle = Arc::new(Mutex::new(Shared::default()));
        {
            let mut s = sh.lock().unwrap();
            s.workflows.push(echo_workflow("wf-vps-ci", false, None));
            s.settings.servers.push(test_server("vps-y"));
            let mut mirror = mk_mirror_run("run-vps-m1", RunStatus::Running, Some("vps-y"));
            mirror.workflow_id = "wf-vps-ci".into();
            mirror.repo_name = Some("widder/erp".into());
            mirror.branch = "main".into();
            s.runs.push(mirror);
        }

        let rid = start_run_with_target(
            &sh,
            "wf-vps-ci",
            "push",
            None,
            None,
            Some("widder/erp".into()),
            Some("main".into()),
        )
        .expect("nova execução deve iniciar");

        let s = sh.lock().unwrap();
        let mirror = s.runs.iter().find(|r| r.id == "run-vps-m1").unwrap();
        assert!(
            mirror.cancel_requested,
            "espelho ativo da mesma repo/branch deve receber o pedido de cancelamento"
        );
        assert!(s.runs.iter().any(|r| r.id == rid));
    }

    #[test]
    fn yaml_roundtrip() {
        let wf = echo_workflow("wf-y", true, None);
        let text = crate::yaml::export_yaml(&wf);
        let back = crate::yaml::import_yaml(&text, "wf-y".into()).expect("import deveria funcionar");
        assert_eq!(back.steps.len(), 1);
        assert!(back.steps[0].approval);
        assert!(back.steps[0].run.contains("workploy-test-ok"));
    }

    #[test]
    fn desktop_repo_binding_polling_routes_to_linked_workflow() {
        let sh = Arc::new(Mutex::new(Shared::default()));
        {
            let mut s = sh.lock().unwrap();
            let mut wf = echo_workflow("wf-bound", false, None);
            wf.server_id = Some("vps-1".into());
            wf.repo_id = Some("rb-1".into());
            s.workflows.push(wf);
            s.settings.repo_bindings.push(RepoBinding {
                id: "rb-1".into(),
                full_name: "widder/app".into(),
                target_server_id: Some("vps-1".into()),
                branch: "main".into(),
                trigger_mode: TriggerMode::PollOnDesktop,
                workflow_id: Some("wf-bound".into()),
                enabled: true,
                last_sha: String::new(),
                last_status: String::new(),
                platform: PlatformTarget::CrossPlatform,
            });
        }

        // Primeira sondagem registra o SHA inicial sem disparar execução espúria
        let t1 = poll_binding_step(&sh, "rb-1", "1111111aaaaaaa", "init");
        assert!(t1.is_empty());

        // Mesmo SHA não dispara
        let t2 = poll_binding_step(&sh, "rb-1", "1111111aaaaaaa", "init");
        assert!(t2.is_empty());

        // Novo SHA dispara o workflow vinculado
        let t3 = poll_binding_step(&sh, "rb-1", "2222222bbbbbbb", "feat: deploy");
        assert_eq!(t3.len(), 1);
        assert_eq!(t3[0].0, "wf-bound");
    }

    #[test]
    fn desktop_repo_binding_multi_branch_polling_tracks_each_branch() {
        let sh = Arc::new(Mutex::new(Shared::default()));
        {
            let mut s = sh.lock().unwrap();
            s.workflows.push(echo_workflow("wf-multi", false, None));
            s.settings.repo_bindings.push(RepoBinding {
                id: "rb-multi".into(),
                full_name: "widder/app".into(),
                target_server_id: None,
                branch: "main,dev".into(),
                trigger_mode: TriggerMode::PollOnDesktop,
                workflow_id: Some("wf-multi".into()),
                enabled: true,
                last_sha: String::new(),
                last_status: String::new(),
                platform: PlatformTarget::CrossPlatform,
            });
        }

        let mut branch_shas = std::collections::HashMap::new();
        // Primeira sondagem em main e dev não dispara
        assert!(
            poll_binding_branch_step(&sh, &mut branch_shas, "rb-multi", "main", 2, "aaa1111", "init main")
                .is_empty()
        );
        assert!(
            poll_binding_branch_step(&sh, &mut branch_shas, "rb-multi", "dev", 2, "bbb2222", "init dev")
                .is_empty()
        );

        // Segunda sondagem sem mudança em nenhuma das duas não dispara
        assert!(
            poll_binding_branch_step(&sh, &mut branch_shas, "rb-multi", "main", 2, "aaa1111", "init main")
                .is_empty()
        );
        assert!(
            poll_binding_branch_step(&sh, &mut branch_shas, "rb-multi", "dev", 2, "bbb2222", "init dev")
                .is_empty()
        );

        // Novo commit apenas em dev dispara
        let trig = poll_binding_branch_step(
            &sh,
            &mut branch_shas,
            "rb-multi",
            "dev",
            2,
            "ccc3333",
            "feat: dev",
        );
        assert_eq!(trig.len(), 1);
        assert_eq!(trig[0].0, "wf-multi");
    }

    #[test]
    fn matching_workflows_uses_generic_push_regardless_of_workflow_server_id() {
        let workflows = vec![
            Workflow {
                id: "wf-generic-push".into(),
                name: "CI Principal".into(),
                trigger: Trigger::Push,
                runner: RunnerKind::Local,
                workdir: String::new(),
                steps: vec![],
                on_success: Some("wf-deploy".into()),
                server_id: None,
                repo_id: None,
                source: None,
            },
            Workflow {
                id: "wf-other-vps-push".into(),
                name: "CI Outro".into(),
                trigger: Trigger::Push,
                runner: RunnerKind::Ssh,
                workdir: String::new(),
                steps: vec![],
                on_success: None,
                server_id: Some("vps-b".into()),
                repo_id: None,
                source: None,
            },
            Workflow {
                id: "wf-manual".into(),
                name: "Deploy VPS".into(),
                trigger: Trigger::Manual,
                runner: RunnerKind::Ssh,
                workdir: String::new(),
                steps: vec![],
                on_success: None,
                server_id: None,
                repo_id: None,
                source: None,
            },
        ];

        let binding = RepoBinding {
            id: "rb-1".into(),
            full_name: "widder/app".into(),
            target_server_id: Some("vps-a".into()),
            branch: "main".into(),
            trigger_mode: TriggerMode::PollOnVps,
            workflow_id: None,
            enabled: true,
            last_sha: String::new(),
            last_status: String::new(),
            platform: PlatformTarget::CrossPlatform,
        };

        let matched = matching_workflows_for_binding(&workflows, &binding);
        assert_eq!(matched, vec!["wf-generic-push", "wf-other-vps-push"]);
    }

    #[test]
    fn start_run_with_target_propagates_repo_branch_and_server_override_across_chain() {
        let sh = Arc::new(Mutex::new(Shared::default()));
        {
            let mut s = sh.lock().unwrap();
            s.workflows
                .push(echo_workflow("wf-ci", false, Some("wf-deploy".into())));
            s.workflows.push(echo_workflow("wf-deploy", false, None));
        }

        let rid = start_run_with_target(
            &sh,
            "wf-ci",
            "push",
            None,
            None,
            Some("widder/erp".into()),
            Some("staging".into()),
        )
        .unwrap();

        assert!(wait_status(&sh, &rid, RunStatus::Success, 20_000));
        let deploy_run = {
            let start = Instant::now();
            loop {
                let found = {
                    let s = sh.lock().unwrap();
                    s.runs.iter().find(|r| r.workflow_id == "wf-deploy").cloned()
                };
                if let Some(r) = found {
                    break r;
                }
                assert!(start.elapsed().as_secs() < 20);
                std::thread::sleep(Duration::from_millis(50));
            }
        };
        assert!(wait_status(&sh, &deploy_run.id, RunStatus::Success, 20_000));

        let s = sh.lock().unwrap();
        let ci_run = s.runs.iter().find(|r| r.id == rid).unwrap();
        assert_eq!(ci_run.repo_name.as_deref(), Some("widder/erp"));
        assert_eq!(ci_run.branch, "staging");
        assert_eq!(deploy_run.repo_name.as_deref(), Some("widder/erp"));
        assert_eq!(deploy_run.branch, "staging");
    }

    #[test]
    fn start_run_for_binding_forces_ssh_on_target_server_even_for_local_recipe() {
        let sh = Arc::new(Mutex::new(Shared::default()));
        {
            let mut s = sh.lock().unwrap();
            let mut wf = echo_workflow("wf-generic", false, None);
            wf.trigger = Trigger::Push;
            wf.runner = RunnerKind::Local;
            wf.server_id = None;
            s.workflows.push(wf);
            s.settings.servers.push(VpsServer {
                id: "vps-target".into(),
                name: "VPS Alvo".into(),
                host: "192.0.2.55".into(),
                port: 22,
                user: "deploy".into(),
                auth: AuthMethod::PrivateKey {
                    path: "/nonexistent/workploy_test_missing_key".into(),
                },
                last_probe: None,
                runner_installed: false,
                runner_port: 8787,
                webhook_secret_key: None,
            });
            s.settings.repo_bindings.push(RepoBinding {
                id: "rb-ssh".into(),
                full_name: "widder/service".into(),
                target_server_id: Some("vps-target".into()),
                branch: "develop".into(),
                trigger_mode: TriggerMode::PollOnDesktop,
                workflow_id: None,
                enabled: true,
                last_sha: String::new(),
                last_status: String::new(),
                platform: PlatformTarget::CrossPlatform,
            });
        }

        let rid = start_run_for_binding(&sh, "rb-ssh", "manual").unwrap();
        assert!(wait_status(&sh, &rid, RunStatus::Failed, 10_000));
        let s = sh.lock().unwrap();
        let r = s.runs.iter().find(|r| r.id == rid).unwrap();
        assert_eq!(r.repo_name.as_deref(), Some("widder/service"));
        assert_eq!(r.branch, "develop");
        assert!(
            r.steps[0].log.contains("[erro SSH]"),
            "deveria ter forçado execução SSH na VPS do vínculo, log={}",
            r.steps[0].log
        );
    }

    #[test]
    fn start_run_for_binding_commit_populates_commit_repo_branch_and_target() {
        let sh = Arc::new(Mutex::new(Shared::default()));
        {
            let mut s = sh.lock().unwrap();
            let mut wf = echo_workflow("wf-commit", false, None);
            wf.trigger = Trigger::Push;
            s.workflows.push(wf);
            s.settings.repo_bindings.push(RepoBinding {
                id: "rb-commit".into(),
                full_name: "matheuswidder/w-workploy".into(),
                target_server_id: None,
                branch: "main".into(),
                trigger_mode: TriggerMode::PollOnDesktop,
                workflow_id: None,
                enabled: true,
                last_sha: String::new(),
                last_status: String::new(),
                platform: PlatformTarget::Windows,
            });
        }

        let rid = start_run_for_binding_commit(
            &sh,
            "rb-commit",
            "deadbeef1234567890",
            "fix(ci): ajusta build do instalador\n\ndetalhes",
            "release/0.1.8",
        )
        .unwrap();
        assert!(wait_status(&sh, &rid, RunStatus::Success, 10_000));
        let s = sh.lock().unwrap();
        let r = s.runs.iter().find(|r| r.id == rid).unwrap();
        assert_eq!(r.commit.as_deref(), Some("deadbeef1234567890"));
        assert_eq!(r.title, "fix(ci): ajusta build do instalador");
        assert_eq!(r.repo_name.as_deref(), Some("matheuswidder/w-workploy"));
        assert_eq!(r.branch, "release/0.1.8");
        assert!(r.trigger_label.contains("deadbee"));
    }

    #[test]
    fn matching_workflows_excludes_windows_and_other_repo_workflows_on_vps() {
        let workflows = vec![
            Workflow {
                id: "wf-win-workdir".into(),
                name: "Build Local".into(),
                trigger: Trigger::Push,
                runner: RunnerKind::Local,
                workdir: r"C:\Projects\w-workploy".into(),
                steps: vec![],
                on_success: None,
                server_id: None,
                repo_id: None,
                source: None,
            },
            Workflow {
                id: "wf-win-slash".into(),
                name: "Build Slash".into(),
                trigger: Trigger::Push,
                runner: RunnerKind::Local,
                workdir: "D:/Projects/outro".into(),
                steps: vec![],
                on_success: None,
                server_id: None,
                repo_id: None,
                source: None,
            },
            Workflow {
                id: "wf-win-release".into(),
                name: "Workploy Windows CI & Release".into(),
                trigger: Trigger::Push,
                runner: RunnerKind::Local,
                workdir: String::new(),
                steps: vec![Step {
                    name: "Build".into(),
                    run: "powershell -NoProfile -File scripts/build-installer.ps1".into(),
                    approval: false,
                }],
                on_success: None,
                server_id: None,
                repo_id: None,
                source: None,
            },
            Workflow {
                id: "wf-other-repo".into(),
                name: "CI Outro Projeto".into(),
                trigger: Trigger::Push,
                runner: RunnerKind::Ssh,
                workdir: String::new(),
                steps: vec![],
                on_success: None,
                server_id: None,
                repo_id: Some("rb-other".into()),
                source: None,
            },
            Workflow {
                id: "wf-claimed-by-binding".into(),
                name: "CI Vinculado".into(),
                trigger: Trigger::Push,
                runner: RunnerKind::Local,
                workdir: String::new(),
                steps: vec![],
                on_success: None,
                server_id: None,
                repo_id: None,
                source: None,
            },
            Workflow {
                id: "wf-generic-linux".into(),
                name: "CI Principal".into(),
                trigger: Trigger::Push,
                runner: RunnerKind::Local,
                workdir: String::new(),
                steps: vec![Step {
                    name: "Test".into(),
                    run: "cargo test".into(),
                    approval: false,
                }],
                on_success: None,
                server_id: None,
                repo_id: None,
                source: None,
            },
        ];

        let other_binding = RepoBinding {
            id: "rb-other".into(),
            full_name: "matheuswidder/w-workploy".into(),
            target_server_id: None,
            branch: "main".into(),
            trigger_mode: TriggerMode::PollOnDesktop,
            workflow_id: Some("wf-claimed-by-binding".into()),
            enabled: true,
            ..RepoBinding::default()
        };
        let target_binding = RepoBinding {
            id: "rb-vps".into(),
            full_name: "matheuswidder/w-erp".into(),
            target_server_id: Some("vps-1".into()),
            branch: "main".into(),
            trigger_mode: TriggerMode::PollOnDesktop,
            workflow_id: None,
            enabled: true,
            ..RepoBinding::default()
        };

        // Sem all_bindings, ainda exclui caminhos Windows, receita Windows e outro repo_id
        let basic = matching_workflows_for_binding(&workflows, &target_binding);
        assert_eq!(basic, vec!["wf-claimed-by-binding", "wf-generic-linux"]);

        // Com all_bindings, exclui também wf-claimed-by-binding (pertence a outro binding)
        let scoped = matching_workflows_for_binding_with_bindings(
            &workflows,
            &target_binding,
            &[other_binding, target_binding.clone()],
        );
        assert_eq!(scoped, vec!["wf-generic-linux"]);
    }

    #[test]
    fn build_vps_sync_command_builds_workspace_and_masks_tokens() {
        assert_eq!(safe_slug("matheuswidder/w-erp-central"), "matheuswidder-w-erp-central");
        assert_eq!(safe_slug("///"), "workspace");

        let (wd_branch, script_branch) =
            build_vps_sync_script("matheuswidder/w-erp", "main, dev", None);
        assert_eq!(wd_branch, "/var/lib/workploy/workspaces/matheuswidder-w-erp");
        assert!(script_branch.contains("mkdir -p \"$WS_ROOT\""));
        assert!(script_branch.contains("/etc/workploy/github_token"));
        assert!(script_branch.contains("git clone"));
        assert!(script_branch.contains("git -C \"$WD\" fetch origin"));
        assert!(script_branch.contains("git -C \"$WD\" checkout 'main'"));
        assert!(script_branch.contains("git -C \"$WD\" pull --ff-only origin 'main'"));
        assert!(!script_branch.contains("--force"));

        let (wd_cmd, cmd_branch) =
            build_vps_sync_command("matheuswidder/w-erp", "main, dev", None);
        assert_eq!(wd_cmd, wd_branch);
        assert!(cmd_branch.starts_with("sh -c "));

        let (wd_sha, script_sha) = build_vps_sync_script(
            "matheuswidder/w-erp",
            "main",
            Some("deadbeef1234567890"),
        );
        assert_eq!(wd_sha, wd_branch);
        assert!(script_sha.contains("git -C \"$WD\" checkout --force 'deadbeef1234567890'"));

        let raw_log = "fatal: could not read from https://x-access-token:ghp_secret123456@github.com/org/repo.git and ghp_secret123456";
        let masked = mask_secrets_in_log(raw_log, &["ghp_secret123456"]);
        assert!(!masked.contains("ghp_secret123456"));
        assert!(masked.contains("***"));
    }

    #[test]
    fn clear_finished_runs_removes_only_inactive_runs_and_notifies() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let sh = Arc::new(Mutex::new(Shared::default()));
        let wakes = Arc::new(AtomicUsize::new(0));
        let wakes_c = wakes.clone();
        set_wake_hook(
            &sh,
            Some(Arc::new(move || {
                wakes_c.fetch_add(1, Ordering::SeqCst);
            })),
        );

        let mk_run = |id: &str, status: RunStatus| Run {
            id: id.into(),
            workflow_id: "wf-1".into(),
            workflow_name: "CI".into(),
            status,
            trigger_label: "manual".into(),
            title: "run".into(),
            number: 1,
            commit: None,
            started_at: "18/11/2026 10:00".into(),
            started_ts: None,
            finished_at: None,
            steps: Vec::new(),
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

        {
            let mut s = sh.lock().unwrap();
            s.runs.push(mk_run("r-queued", RunStatus::Queued));
            s.runs.push(mk_run("r-running", RunStatus::Running));
            s.runs.push(mk_run("r-wait", RunStatus::AwaitingApproval));
            s.runs.push(mk_run("r-ok", RunStatus::Success));
            s.runs.push(mk_run("r-fail", RunStatus::Failed));
            s.runs.push(mk_run("r-cancel", RunStatus::Cancelled));
        }

        let before = wakes.load(Ordering::SeqCst);
        let removed = clear_finished_runs(&sh);
        assert_eq!(removed, 3);
        assert!(wakes.load(Ordering::SeqCst) > before);

        let s = sh.lock().unwrap();
        let remaining_ids: Vec<&str> = s.runs.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(remaining_ids, vec!["r-queued", "r-running", "r-wait"]);
    }

    #[test]
    fn substep_telemetry_tracks_live_substeps_and_finalizes_status() {
        let mut step = StepRun {
            name: "Typecheck + Tests".into(),
            status: StepStatus::Running,
            log: String::new(),
            secs: 0.0,
            substeps: Vec::new(),
            started_ts: Some(now_ts()),
        };

        apply_log_to_step(&mut step, "::workploy-substep::Install dependencies\n");
        assert_eq!(step.substeps.len(), 1);
        assert_eq!(step.substeps[0].name, "Install dependencies");
        assert_eq!(step.substeps[0].status, StepStatus::Running);

        apply_log_to_step(&mut step, "Packages: +811\nDone in 2s\n");
        assert!(step.substeps[0].log.contains("Packages: +811"));
        assert!(step.log.contains("Packages: +811"));

        apply_log_to_step(&mut step, "::workploy-substep::Run tests\n");
        assert_eq!(step.substeps.len(), 2);
        assert_eq!(step.substeps[0].status, StepStatus::Success);
        assert_eq!(step.substeps[1].name, "Run tests");
        assert_eq!(step.substeps[1].status, StepStatus::Running);

        apply_log_to_step(&mut step, "FAIL src/app.spec.ts\n");
        assert!(step.substeps[1].log.contains("FAIL src/app.spec.ts"));
        assert!(!step.substeps[0].log.contains("FAIL src/app.spec.ts"));

        finalize_step_substeps(&mut step, StepStatus::Failed);
        assert_eq!(step.substeps[0].status, StepStatus::Success);
        assert_eq!(step.substeps[1].status, StepStatus::Failed);
    }

    #[test]
    fn start_run_with_target_cancels_previous_active_run_on_same_repo_branch_and_workflow() {
        let sh = Arc::new(Mutex::new(Shared::default()));
        {
            let mut s = sh.lock().unwrap();
            let mut wf = echo_workflow("wf-ci", false, None);
            wf.steps[0].run = if cfg!(windows) {
                "ping -n 5 127.0.0.1 > nul"
            } else {
                "sleep 5"
            }
            .into();
            s.workflows.push(wf.clone());
            wf.id = "wf-other".into();
            s.workflows.push(wf);
        }

        let first_main = start_run_with_target(
            &sh,
            "wf-ci",
            "push 1",
            None,
            None,
            Some("widder/erp".into()),
            Some("main".into()),
        )
        .unwrap();
        let dev_run = start_run_with_target(
            &sh,
            "wf-ci",
            "push dev",
            None,
            None,
            Some("widder/erp".into()),
            Some("dev".into()),
        )
        .unwrap();
        let other_wf_run = start_run_with_target(
            &sh,
            "wf-other",
            "push other wf",
            None,
            None,
            Some("widder/erp".into()),
            Some("main".into()),
        )
        .unwrap();

        assert!(wait_status(&sh, &first_main, RunStatus::Running, 5000));

        // Nova execução na mesma (repo, branch, workflow_id) deve solicitar cancelamento da anterior
        let second_main = start_run_with_target(
            &sh,
            "wf-ci",
            "push 2",
            None,
            None,
            Some("WIDDER/ERP".into()),
            Some("main".into()),
        )
        .unwrap();

        {
            let s = sh.lock().unwrap();
            let r1 = s.runs.iter().find(|r| r.id == first_main).unwrap();
            let r_dev = s.runs.iter().find(|r| r.id == dev_run).unwrap();
            let r_other = s.runs.iter().find(|r| r.id == other_wf_run).unwrap();
            let r2 = s.runs.iter().find(|r| r.id == second_main).unwrap();

            assert!(r1.cancel_requested);
            assert!(!r_dev.cancel_requested);
            assert!(!r_other.cancel_requested);
            assert!(!r2.cancel_requested);
        }

        assert!(wait_status(&sh, &first_main, RunStatus::Cancelled, 10_000));
        cancel(&sh, &dev_run);
        cancel(&sh, &other_wf_run);
        cancel(&sh, &second_main);
    }

    #[test]
    fn skipped_step_is_not_executed_and_flow_continues() {
        let sh: SharedHandle = Arc::new(Mutex::new(Shared::default()));
        {
            let mut s = sh.lock().unwrap();
            let mut wf = echo_workflow("wf-skip", false, None);
            wf.steps[0].name = "Preparo".into();
            wf.steps.push(Step {
                name: "Rodar".into(),
                run: "echo rodou".into(),
                approval: false,
            });
            s.workflows.push(wf);
            s.workflow_extras.insert(
                "wf-skip".into(),
                crate::model::WorkflowExtras {
                    skipped_steps: vec!["Preparo".into()],
                    ..Default::default()
                },
            );
        }
        let rid = start_run(&sh, "wf-skip", "teste").expect("run criada");
        assert!(
            wait_status(&sh, &rid, RunStatus::Success, 20_000),
            "fluxo com passo pulado deve concluir"
        );
        let s = sh.lock().unwrap();
        let r = s.runs.iter().find(|r| r.id == rid).unwrap();
        assert_eq!(r.steps[0].status, StepStatus::Skipped);
        assert!(r.steps[0].log.contains("pulado"), "log: {}", r.steps[0].log);
        assert_eq!(r.steps[1].status, StepStatus::Success);
        assert!(r.steps[1].log.contains("rodou"));
    }
}

