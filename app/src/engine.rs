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

fn append_log(sh: &SharedHandle, run_id: &str, step_idx: usize, text: &str) {
    let mut s = sh.lock().unwrap();
    if let Some(r) = s.runs.iter_mut().find(|r| r.id == run_id)
        && let Some(st) = r.steps.get_mut(step_idx)
    {
        st.log.push_str(text);
        const CAP: usize = 120_000;
        if st.log.len() > CAP {
            let cut = st.log.len() - CAP;
            st.log.drain(..cut);
        }
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

pub fn cancel(sh: &SharedHandle, run_id: &str) {
    update_run(sh, run_id, |r| r.cancel_requested = true);
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
        branch: "main".into(),
        summary: String::new(),
        annotations: Vec::new(),
    };
    let run_id = run.id.clone();
    {
        let mut s = sh.lock().unwrap();
        s.runs.insert(0, run);
        save_runs(&s);
    }
    notify(sh);
    let sh2 = sh.clone();
    let rid = run_id.clone();
    std::thread::spawn(move || execute_run(sh2, rid, wf));
    Some(run_id)
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
                st.status = StepStatus::Skipped;
            }
        }
    });
}

fn execute_run(sh: SharedHandle, run_id: String, wf: Workflow) {
    let total = Instant::now();
    for (i, step) in wf.steps.iter().enumerate() {
        if is_cancelled(&sh, &run_id) {
            finish_cancelled(&sh, &run_id, i, total.elapsed().as_secs_f64(), None);
            return;
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
        let (runner, ssh_host, target_server, workdir) = {
            let s = sh.lock().unwrap();
            let srv = wf
                .server_id
                .as_ref()
                .and_then(|sid| s.settings.servers.iter().find(|v| &v.id == sid).cloned());
            (
                wf.runner,
                s.settings.ssh_host.clone(),
                srv,
                wf.workdir.clone(),
            )
        };
        let (ok, secs) = exec_step(
            &sh,
            &run_id,
            i,
            runner,
            &ssh_host,
            target_server.as_ref(),
            &workdir,
            &step.run,
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
                st.status = if ok {
                    StepStatus::Success
                } else {
                    StepStatus::Failed
                };
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
            }
        });
        if !ok {
            return;
        }
    }
    update_run(&sh, &run_id, |r| {
        r.status = RunStatus::Success;
        r.finished_at = Some(now_str());
        r.secs = total.elapsed().as_secs_f64();
    });
    if let Some(next) = &wf.on_success
        && next != &wf.id
    {
        start_run(&sh, next, "encadeado: fluxo anterior concluído");
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
) -> (bool, f64) {
    let start = Instant::now();
    if cmd.trim().is_empty() {
        append_log(sh, run_id, idx, "[passo sem comando — nada a fazer]\n");
        return (true, 0.0);
    }

    if runner == RunnerKind::Ssh
        && let Some(srv) = target_server
    {
        let remote_cmd = if workdir.trim().is_empty() {
            cmd.to_string()
        } else {
            let escaped = workdir.trim().replace('\'', "'\"'\"'");
            format!("cd '{escaped}' && {cmd}")
        };
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
            let res = vps::ssh_run_streaming(srv, &remote_cmd, Some(&cancel_flag), |line| {
                append_log(sh, run_id, idx, &format!("{line}\n"));
            });
            done_flag.store(true, Ordering::Relaxed);
            match res {
                Ok(success) => success,
                Err(e) => {
                    append_log(sh, run_id, idx, &format!("[erro SSH] {e}\n"));
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
            c.arg("-o")
                .arg("BatchMode=yes")
                .arg("-o")
                .arg("ConnectTimeout=15")
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

pub fn matching_workflows_for_binding(workflows: &[Workflow], binding: &RepoBinding) -> Vec<String> {
    if let Some(wid) = &binding.workflow_id
        && workflows.iter().any(|w| &w.id == wid)
    {
        return vec![wid.clone()];
    }
    let by_repo: Vec<String> = workflows
        .iter()
        .filter(|w| w.repo_id.as_deref() == Some(binding.id.as_str()))
        .map(|w| w.id.clone())
        .collect();
    if !by_repo.is_empty() {
        return by_repo;
    }
    workflows
        .iter()
        .filter(|w| w.trigger == Trigger::Push && w.server_id == binding.target_server_id)
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
        if let Some(b) = s.settings.repo_bindings.iter_mut().find(|x| x.id == binding_id) {
            let repo_name = b.full_name.clone();
            if b.last_sha.is_empty() {
                b.last_sha = sha.to_string();
                b.last_status = format!("Monitorando — commit atual {short}");
            } else if b.last_sha != sha {
                b.last_sha = sha.to_string();
                b.last_status = format!("Novo commit {short} — disparando execução");
                for wf_id in matching_workflows_for_binding(&workflows, b) {
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
                    for wf_id in matching_workflows_for_binding(&workflows, b) {
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
                                start_run_full(
                                    &sh,
                                    &wf_id,
                                    "push no GitHub",
                                    Some((sha.clone(), message.clone())),
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
}
