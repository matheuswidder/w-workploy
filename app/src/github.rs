use crate::engine::{self, SharedHandle};
use crate::model::*;
use crate::vault;
use crate::vps;
use serde::Serialize;
use std::collections::HashSet;

#[derive(Serialize)]
struct RunnerConfigPayload {
    poll_secs: u64,
    bindings: Vec<RepoBinding>,
    workflows: Vec<Workflow>,
    purge_rule: PurgeRule,
}

pub fn parse_github_user_json(v: &serde_json::Value) -> Result<String, String> {
    v.get("login")
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Resposta do GitHub não trouxe o usuário autenticado.".into())
}

pub fn parse_github_repos_json(v: &serde_json::Value) -> Result<Vec<GithubRepoSummary>, String> {
    let arr = v
        .as_array()
        .ok_or_else(|| "Resposta inesperada ao listar repositórios do GitHub.".to_string())?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let Some(full_name) = item.get("full_name").and_then(|x| x.as_str()) else {
            continue;
        };
        out.push(GithubRepoSummary {
            full_name: full_name.to_string(),
            default_branch: item
                .get("default_branch")
                .and_then(|x| x.as_str())
                .unwrap_or("main")
                .to_string(),
            private: item
                .get("private")
                .and_then(|x| x.as_bool())
                .unwrap_or(false),
            description: item
                .get("description")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            html_url: item
                .get("html_url")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            updated_at: item
                .get("updated_at")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
        });
    }
    Ok(out)
}

pub fn resolve_github_token_with_warning(
    sh: &SharedHandle,
    token_opt: Option<&str>,
) -> (Option<String>, Option<String>) {
    if let Some(tok) = token_opt {
        let trimmed = tok.trim();
        if !trimmed.is_empty() {
            let vault_warn = match vault::save_secret(vault::KEY_GITHUB_PAT, trimmed) {
                Ok(()) => None,
                Err(e) => Some(format!(
                    "Token mantido apenas em memória (falha ao salvar no Cofre do Windows: {e})"
                )),
            };
            let mut s = sh.lock().unwrap();
            s.settings.github_token = trimmed.to_string();
            s.settings.has_github_token = true;
            if let Some(w) = &vault_warn {
                s.github_status = w.clone();
            }
            return (Some(trimmed.to_string()), vault_warn);
        }
    }
    {
        let s = sh.lock().unwrap();
        let in_mem = s.settings.github_token.trim();
        if !in_mem.is_empty() {
            return (Some(in_mem.to_string()), None);
        }
    }
    match vault::load_secret(vault::KEY_GITHUB_PAT) {
        Ok(Some(from_vault)) => {
            let trimmed = from_vault.trim().to_string();
            if !trimmed.is_empty() {
                let mut s = sh.lock().unwrap();
                s.settings.github_token = trimmed.clone();
                s.settings.has_github_token = true;
                return (Some(trimmed), None);
            }
        }
        Ok(None) => {}
        Err(e) => {
            let warn = format!("Não foi possível ler o token no Cofre do Windows: {e}");
            let mut s = sh.lock().unwrap();
            s.github_status = warn.clone();
            return (None, Some(warn));
        }
    }
    (None, None)
}

pub fn resolve_github_token(sh: &SharedHandle, token_opt: Option<&str>) -> Option<String> {
    resolve_github_token_with_warning(sh, token_opt).0
}

fn record_github_event(s: &mut Shared, ok: bool, msg: String) {
    let seq = s.github_last_event.as_ref().map(|e| e.0).unwrap_or(0) + 1;
    s.github_last_event = Some((seq, ok, msg));
}

pub fn test_and_list_repos(sh: &SharedHandle, token_opt: Option<String>) {
    {
        let mut s = sh.lock().unwrap();
        if s.github_testing {
            return;
        }
        s.github_testing = true;
        s.github_status = "Consultando conta e repositórios no GitHub...".into();
    }
    engine::notify(sh);

    let sh2 = sh.clone();
    std::thread::spawn(move || {
        let (token_res, vault_warn) =
            resolve_github_token_with_warning(&sh2, token_opt.as_deref());
        let Some(token) = token_res else {
            let msg = vault_warn
                .unwrap_or_else(|| "Informe um Token (PAT) do GitHub para conectar.".into());
            finish_github_error(&sh2, msg);
            return;
        };

        let user_resp = ureq::get("https://api.github.com/user")
            .set("Authorization", &format!("Bearer {token}"))
            .set("User-Agent", "workploy")
            .set("Accept", "application/vnd.github+json")
            .call();

        let login = match user_resp {
            Ok(r) => match r.into_json::<serde_json::Value>() {
                Ok(v) => match parse_github_user_json(&v) {
                    Ok(u) => u,
                    Err(e) => {
                        finish_github_error(&sh2, e);
                        return;
                    }
                },
                Err(_) => {
                    finish_github_error(&sh2, "Não foi possível ler os dados do usuário no GitHub.".into());
                    return;
                }
            },
            Err(ureq::Error::Status(401, _)) => {
                finish_github_error(&sh2, "Token do GitHub inválido ou expirado (HTTP 401).".into());
                return;
            }
            Err(ureq::Error::Status(code, _)) => {
                finish_github_error(&sh2, format!("GitHub retornou erro HTTP {code} ao autenticar."));
                return;
            }
            Err(ureq::Error::Transport(t)) => {
                finish_github_error(&sh2, format!("Não foi possível conectar ao GitHub: {t}"));
                return;
            }
        };

        let repos_resp = ureq::get("https://api.github.com/user/repos?per_page=100&sort=updated")
            .set("Authorization", &format!("Bearer {token}"))
            .set("User-Agent", "workploy")
            .set("Accept", "application/vnd.github+json")
            .call();

        let repos = match repos_resp {
            Ok(r) => match r.into_json::<serde_json::Value>() {
                Ok(v) => match parse_github_repos_json(&v) {
                    Ok(list) => list,
                    Err(e) => {
                        finish_github_error(&sh2, e);
                        return;
                    }
                },
                Err(_) => {
                    finish_github_error(&sh2, "Não foi possível ler a lista de repositórios do GitHub.".into());
                    return;
                }
            },
            Err(ureq::Error::Status(code, _)) => {
                finish_github_error(&sh2, format!("Erro HTTP {code} ao listar repositórios."));
                return;
            }
            Err(ureq::Error::Transport(t)) => {
                finish_github_error(
                    &sh2,
                    format!("Falha de rede ao listar repositórios no GitHub: {t}"),
                );
                return;
            }
        };

        {
            let mut s = sh2.lock().unwrap();
            s.settings.github_user = login.clone();
            s.github_repos = repos;
            let status = if let Some(warn) = vault_warn {
                format!(
                    "Conectado como @{} · {} repositórios carregados · {}",
                    login,
                    s.github_repos.len(),
                    warn
                )
            } else {
                format!(
                    "Conectado como @{} · {} repositórios carregados",
                    login,
                    s.github_repos.len()
                )
            };
            s.github_status = status.clone();
            s.github_testing = false;
            record_github_event(&mut s, true, status);
            save_settings(&s);
        }
        engine::notify(&sh2);
    });
}

fn finish_github_error(sh: &SharedHandle, msg: String) {
    {
        let mut s = sh.lock().unwrap();
        s.github_testing = false;
        s.github_status = msg.clone();
        record_github_event(&mut s, false, msg);
    }
    engine::notify(sh);
}

pub fn build_runner_config_json(settings: &Settings, workflows: &[Workflow], server_id: &str) -> Result<String, String> {
    let bindings: Vec<RepoBinding> = settings
        .repo_bindings
        .iter()
        .filter(|b| b.target_server_id.as_deref() == Some(server_id))
        .cloned()
        .collect();

    let mut needed_wf_ids: HashSet<String> = HashSet::new();
    for b in &bindings {
        if let Some(wid) = &b.workflow_id {
            needed_wf_ids.insert(wid.clone());
        }
    }
    for wf in workflows {
        if wf.server_id.as_deref() == Some(server_id) {
            needed_wf_ids.insert(wf.id.clone());
        }
    }
    // Inclui fluxos encadeados (`on_success`) para que redeploy automático funcione na VPS.
    let mut changed = true;
    while changed {
        changed = false;
        for wf in workflows {
            if needed_wf_ids.contains(&wf.id)
                && let Some(next) = &wf.on_success
                && needed_wf_ids.insert(next.clone())
            {
                changed = true;
            }
        }
    }

    let selected_wfs: Vec<Workflow> = workflows
        .iter()
        .filter(|w| needed_wf_ids.contains(&w.id))
        .cloned()
        .collect();

    let payload = RunnerConfigPayload {
        poll_secs: settings.poll_secs.max(15),
        bindings,
        workflows: selected_wfs,
        purge_rule: settings.purge_rule.clone(),
    };

    serde_json::to_string_pretty(&payload)
        .map_err(|e| format!("Falha ao serializar configuração da VPS: {e}"))
}

pub fn server_needs_github_token(settings: &Settings, server_id: &str) -> bool {
    settings.repo_bindings.iter().any(|b| {
        b.enabled
            && b.target_server_id.as_deref() == Some(server_id)
            && matches!(
                b.trigger_mode,
                TriggerMode::PollOnVps | TriggerMode::WebhookOnVps
            )
    })
}

pub fn sync_repo_bindings_to_vps(sh: &SharedHandle, server_id: &str) {
    let (server, config_json, needs_gh_token, gh_token) = {
        let s = sh.lock().unwrap();
        let Some(srv) = s.settings.servers.iter().find(|v| v.id == server_id).cloned() else {
            return;
        };
        let cfg = match build_runner_config_json(&s.settings, &s.workflows, server_id) {
            Ok(c) => c,
            Err(e) => {
                drop(s);
                let mut g = sh.lock().unwrap();
                g.runner_sync_status = e;
                drop(g);
                engine::notify(sh);
                return;
            }
        };
        let needs_tok = server_needs_github_token(&s.settings, server_id);
        let tok = if !s.settings.github_token.trim().is_empty() {
            Some(s.settings.github_token.trim().to_string())
        } else {
            vault::load_secret(vault::KEY_GITHUB_PAT).ok().flatten()
        };
        (srv, cfg, needs_tok, tok)
    };

    let sh2 = sh.clone();
    std::thread::spawn(move || {
        let port = if server.runner_port == 0 {
            8787
        } else {
            server.runner_port
        };

        let upload_cfg_cmd = "sh -c 'if [ \"$(id -u)\" -eq 0 ]; then SUDO=\"\"; else SUDO=\"sudo -n\"; fi; $SUDO mkdir -p /etc/workploy && $SUDO tee /etc/workploy/config.json >/dev/null'";
        match vps::ssh_run_with_stdin(&server, upload_cfg_cmd, config_json.as_bytes()) {
            Ok((true, _, _)) => {}
            Ok((false, _, err)) => {
                set_sync_status(&sh2, format!("Falha ao enviar configuração para '{}': {}", server.name, err.trim()));
                return;
            }
            Err(e) => {
                set_sync_status(&sh2, e);
                return;
            }
        }

        if needs_gh_token
            && let Some(tok) = gh_token
            && !tok.trim().is_empty()
        {
            let upload_tok_cmd = "sh -c 'if [ \"$(id -u)\" -eq 0 ]; then SUDO=\"\"; else SUDO=\"sudo -n\"; fi; $SUDO tee /etc/workploy/github_token >/dev/null && $SUDO chmod 600 /etc/workploy/github_token'";
            match vps::ssh_run_with_stdin(&server, upload_tok_cmd, tok.trim().as_bytes()) {
                Ok((true, _, _)) => {}
                Ok((false, _, err)) => {
                    set_sync_status(
                        &sh2,
                        format!(
                            "Configuração enviada, mas falhou ao gravar github_token em '{}': {}",
                            server.name,
                            err.trim()
                        ),
                    );
                    return;
                }
                Err(e) => {
                    set_sync_status(&sh2, e);
                    return;
                }
            }
        }

        let reload_cmd = format!(
            "sh -c 'TOKEN=$(cat /etc/workploy/token 2>/dev/null) && curl -fsS -X POST -H \"Authorization: Bearer $TOKEN\" http://127.0.0.1:{port}/reload'"
        );
        let msg = match vps::ssh_run_capture(&server, &reload_cmd) {
            Ok((true, _, _)) => format!("Regras e repositórios sincronizados em '{}'.", server.name),
            Ok((false, _, err)) => format!("Configuração enviada para '{}', mas o reload falhou: {}", server.name, err.trim()),
            Err(e) => e,
        };
        set_sync_status(&sh2, msg);
    });
}

fn set_sync_status(sh: &SharedHandle, msg: String) {
    {
        let mut s = sh.lock().unwrap();
        s.runner_sync_status = msg;
    }
    engine::notify(sh);
}

fn parse_semver_tuple(raw: &str) -> Option<(u64, u64, u64)> {
    let trimmed = raw.trim().trim_start_matches(['v', 'V']);
    let core = trimmed.split(['-', '+']).next()?.trim();
    if core.is_empty() {
        return None;
    }
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = match parts.next() {
        Some(p) => p.parse::<u64>().ok()?,
        None => 0,
    };
    let patch = match parts.next() {
        Some(p) => p.parse::<u64>().ok()?,
        None => 0,
    };
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

pub fn is_newer_version(current: &str, latest: &str) -> bool {
    match (parse_semver_tuple(current), parse_semver_tuple(latest)) {
        (Some(cur), Some(lat)) => lat > cur,
        _ => false,
    }
}

pub fn parse_release_json(v: &serde_json::Value, current_version: &str) -> Result<ReleaseInfo, String> {
    let tag_name = v
        .get("tag_name")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Resposta de release do GitHub sem tag_name.".to_string())?
        .to_string();

    let version = tag_name.trim_start_matches(['v', 'V']).to_string();
    let html_url = v
        .get("html_url")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let published_at = v
        .get("published_at")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let notes = v
        .get("body")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();

    let assets = v.get("assets").and_then(|x| x.as_array());
    let installer_url = assets
        .and_then(|list| {
            list.iter()
                .find(|a| {
                    a.get("name")
                        .and_then(|n| n.as_str())
                        .is_some_and(|n| {
                            let lower = n.to_ascii_lowercase();
                            lower.starts_with("workploy-setup") && lower.ends_with(".exe")
                        })
                })
                .or_else(|| {
                    list.iter().find(|a| {
                        a.get("name")
                            .and_then(|n| n.as_str())
                            .is_some_and(|n| n.to_ascii_lowercase().ends_with(".exe"))
                    })
                })
                .and_then(|a| a.get("browser_download_url"))
                .and_then(|u| u.as_str())
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| html_url.clone());

    let has_update = is_newer_version(current_version, &tag_name);

    Ok(ReleaseInfo {
        tag_name,
        version,
        html_url,
        installer_url,
        published_at,
        notes,
        has_update,
    })
}

fn record_update_event(s: &mut Shared, ok: bool, msg: String) {
    let seq = s.update_last_event.as_ref().map(|e| e.0).unwrap_or(0) + 1;
    s.update_last_event = Some((seq, ok, msg));
}

pub fn check_for_updates(sh: &SharedHandle) {
    {
        let mut s = sh.lock().unwrap();
        if s.update_checking {
            return;
        }
        s.update_checking = true;
        s.update_status = "Verificando atualizações no GitHub...".into();
    }
    engine::notify(sh);

    let sh2 = sh.clone();
    std::thread::spawn(move || {
        let url = "https://api.github.com/repos/matheuswidder/w-workploy/releases/latest";
        let resp = ureq::get(url)
            .set("User-Agent", "workploy")
            .set("Accept", "application/vnd.github+json")
            .call();

        let current_ver = env!("CARGO_PKG_VERSION");

        match resp {
            Ok(r) => match r.into_json::<serde_json::Value>() {
                Ok(v) => match parse_release_json(&v, current_ver) {
                    Ok(info) => {
                        let status = if info.has_update {
                            format!(
                                "Nova versão disponível: v{} (atual: v{current_ver})",
                                info.version
                            )
                        } else {
                            format!("Você já está na versão mais recente (v{current_ver}).")
                        };
                        let mut s = sh2.lock().unwrap();
                        s.update_checking = false;
                        s.latest_release = Some(info);
                        s.update_status = status.clone();
                        record_update_event(&mut s, true, status);
                    }
                    Err(e) => {
                        let mut s = sh2.lock().unwrap();
                        s.update_checking = false;
                        s.update_status = e.clone();
                        record_update_event(&mut s, false, e);
                    }
                },
                Err(_) => {
                    let msg = "Não foi possível ler os dados da release no GitHub.".to_string();
                    let mut s = sh2.lock().unwrap();
                    s.update_checking = false;
                    s.update_status = msg.clone();
                    record_update_event(&mut s, false, msg);
                }
            },
            Err(ureq::Error::Status(404, _)) => {
                let msg = "Nenhuma release publicada ainda em matheuswidder/w-workploy.".to_string();
                let mut s = sh2.lock().unwrap();
                s.update_checking = false;
                s.latest_release = None;
                s.update_status = msg.clone();
                record_update_event(&mut s, true, msg);
            }
            Err(ureq::Error::Status(code, _)) => {
                let msg = format!("GitHub retornou erro HTTP {code} ao verificar atualizações.");
                let mut s = sh2.lock().unwrap();
                s.update_checking = false;
                s.update_status = msg.clone();
                record_update_event(&mut s, false, msg);
            }
            Err(ureq::Error::Transport(t)) => {
                let msg = format!("Não foi possível verificar atualizações: {t}");
                let mut s = sh2.lock().unwrap();
                s.update_checking = false;
                s.update_status = msg.clone();
                record_update_event(&mut s, false, msg);
            }
        }
        engine::notify(&sh2);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_github_user_and_repos_payload() {
        let user_json = serde_json::json!({ "login": "matheuswidder" });
        assert_eq!(parse_github_user_json(&user_json).unwrap(), "matheuswidder");

        let repos_json = serde_json::json!([
            {
                "full_name": "matheuswidder/w-erp-central",
                "default_branch": "main",
                "private": true,
                "description": "ERP principal",
                "html_url": "https://github.com/matheuswidder/w-erp-central",
                "updated_at": "2026-11-17T10:00:00Z"
            },
            {
                "full_name": "matheuswidder/widder-site",
                "default_branch": "master",
                "private": false,
                "description": null
            }
        ]);
        let repos = parse_github_repos_json(&repos_json).unwrap();
        assert_eq!(repos.len(), 2);
        assert_eq!(repos[0].full_name, "matheuswidder/w-erp-central");
        assert!(repos[0].private);
        assert_eq!(repos[1].default_branch, "master");
        assert!(!repos[1].private);
        assert_eq!(repos[1].description, "");
    }

    #[test]
    fn runner_config_includes_only_target_vps_bindings_and_chained_workflows() {
        let mut settings = Settings {
            poll_secs: 10, // deve respeitar mínimo 15s
            ..Settings::default()
        };
        settings.repo_bindings.push(RepoBinding {
            id: "rb-1".into(),
            full_name: "widder/api".into(),
            target_server_id: Some("vps-a".into()),
            branch: "main".into(),
            trigger_mode: TriggerMode::WebhookOnVps,
            workflow_id: Some("wf-ci".into()),
            enabled: true,
            last_sha: String::new(),
            last_status: String::new(),
        });
        settings.repo_bindings.push(RepoBinding {
            id: "rb-2".into(),
            full_name: "widder/other".into(),
            target_server_id: Some("vps-b".into()),
            branch: "main".into(),
            trigger_mode: TriggerMode::PollOnVps,
            workflow_id: Some("wf-other".into()),
            enabled: true,
            last_sha: String::new(),
            last_status: String::new(),
        });

        let workflows = vec![
            Workflow {
                id: "wf-ci".into(),
                name: "CI".into(),
                trigger: Trigger::Push,
                runner: RunnerKind::Ssh,
                workdir: String::new(),
                steps: vec![Step { name: "Test".into(), run: "echo ok".into(), approval: false }],
                on_success: Some("wf-deploy".into()),
                server_id: Some("vps-a".into()),
                repo_id: Some("rb-1".into()),
            },
            Workflow {
                id: "wf-deploy".into(),
                name: "Deploy".into(),
                trigger: Trigger::Manual,
                runner: RunnerKind::Ssh,
                workdir: String::new(),
                steps: vec![Step { name: "Up".into(), run: "docker compose up -d".into(), approval: true }],
                on_success: None,
                server_id: Some("vps-a".into()),
                repo_id: None,
            },
            Workflow {
                id: "wf-other".into(),
                name: "Other".into(),
                trigger: Trigger::Push,
                runner: RunnerKind::Ssh,
                workdir: String::new(),
                steps: vec![],
                on_success: None,
                server_id: Some("vps-b".into()),
                repo_id: Some("rb-2".into()),
            },
        ];

        let cfg_str = build_runner_config_json(&settings, &workflows, "vps-a").unwrap();
        let cfg: serde_json::Value = serde_json::from_str(&cfg_str).unwrap();
        assert_eq!(cfg["poll_secs"].as_u64(), Some(15));
        assert_eq!(cfg["bindings"].as_array().unwrap().len(), 1);
        assert_eq!(cfg["bindings"][0]["full_name"], "widder/api");
        let wfs = cfg["workflows"].as_array().unwrap();
        assert_eq!(wfs.len(), 2);
        assert!(wfs.iter().any(|w| w["id"] == "wf-ci"));
        assert!(wfs.iter().any(|w| w["id"] == "wf-deploy"));
        assert!(!wfs.iter().any(|w| w["id"] == "wf-other"));
        assert_eq!(cfg["purge_rule"]["enabled"].as_bool(), Some(false));
        assert_eq!(cfg["purge_rule"]["every_n_runs"].as_u64(), Some(10));
        assert_eq!(cfg["purge_rule"]["max_size_mb"].as_u64(), Some(5120));
        assert_eq!(cfg["purge_rule"]["clean_workspaces"].as_bool(), Some(true));
        assert_eq!(cfg["purge_rule"]["clean_docker"].as_bool(), Some(true));
    }

    #[test]
    fn runner_config_serializes_custom_purge_rule_and_token_needed_for_webhook_and_poll() {
        let mut settings = Settings {
            purge_rule: PurgeRule {
                enabled: true,
                every_n_runs: 4,
                max_size_mb: 1024,
                clean_workspaces: true,
                clean_docker: false,
            },
            ..Settings::default()
        };
        settings.repo_bindings.push(RepoBinding {
            id: "rb-wh".into(),
            full_name: "widder/private-wh".into(),
            target_server_id: Some("vps-wh".into()),
            branch: "main".into(),
            trigger_mode: TriggerMode::WebhookOnVps,
            workflow_id: None,
            enabled: true,
            last_sha: String::new(),
            last_status: String::new(),
        });
        settings.repo_bindings.push(RepoBinding {
            id: "rb-poll".into(),
            full_name: "widder/private-poll".into(),
            target_server_id: Some("vps-poll".into()),
            branch: "main".into(),
            trigger_mode: TriggerMode::PollOnVps,
            workflow_id: None,
            enabled: true,
            last_sha: String::new(),
            last_status: String::new(),
        });
        settings.repo_bindings.push(RepoBinding {
            id: "rb-disabled".into(),
            full_name: "widder/disabled".into(),
            target_server_id: Some("vps-off".into()),
            branch: "main".into(),
            trigger_mode: TriggerMode::WebhookOnVps,
            workflow_id: None,
            enabled: false,
            last_sha: String::new(),
            last_status: String::new(),
        });
        settings.repo_bindings.push(RepoBinding {
            id: "rb-desk".into(),
            full_name: "widder/desktop".into(),
            target_server_id: Some("vps-desk".into()),
            branch: "main".into(),
            trigger_mode: TriggerMode::PollOnDesktop,
            workflow_id: None,
            enabled: true,
            last_sha: String::new(),
            last_status: String::new(),
        });

        assert!(server_needs_github_token(&settings, "vps-wh"));
        assert!(server_needs_github_token(&settings, "vps-poll"));
        assert!(!server_needs_github_token(&settings, "vps-off"));
        assert!(!server_needs_github_token(&settings, "vps-desk"));

        let cfg_str = build_runner_config_json(&settings, &[], "vps-wh").unwrap();
        let cfg: serde_json::Value = serde_json::from_str(&cfg_str).unwrap();
        assert_eq!(cfg["purge_rule"]["enabled"].as_bool(), Some(true));
        assert_eq!(cfg["purge_rule"]["every_n_runs"].as_u64(), Some(4));
        assert_eq!(cfg["purge_rule"]["max_size_mb"].as_u64(), Some(1024));
        assert_eq!(cfg["purge_rule"]["clean_workspaces"].as_bool(), Some(true));
        assert_eq!(cfg["purge_rule"]["clean_docker"].as_bool(), Some(false));
    }

    #[test]
    fn github_event_counter_increments_monotonically() {
        let sh = std::sync::Arc::new(std::sync::Mutex::new(Shared::default()));
        finish_github_error(&sh, "falha 1".into());
        {
            let s = sh.lock().unwrap();
            assert_eq!(
                s.github_last_event,
                Some((1, false, "falha 1".to_string()))
            );
            assert_eq!(s.github_status, "falha 1");
        }
        {
            let mut s = sh.lock().unwrap();
            record_github_event(&mut s, true, "ok 2".into());
            assert_eq!(s.github_last_event, Some((2, true, "ok 2".to_string())));
        }
    }

    #[test]
    fn is_newer_version_compares_semver_with_and_without_v_prefix() {
        assert!(is_newer_version("0.1.2", "0.1.3"));
        assert!(is_newer_version("0.1.2", "v0.1.3"));
        assert!(is_newer_version("v0.1.2", "0.2.0"));
        assert!(is_newer_version("0.1.2", "V1.0.0"));
        assert!(is_newer_version("0.2.0", "0.10.0"));

        assert!(!is_newer_version("0.1.2", "0.1.2"));
        assert!(!is_newer_version("0.1.2", "v0.1.2"));
        assert!(!is_newer_version("0.1.2", "0.1.1"));
        assert!(!is_newer_version("0.10.0", "0.2.0"));
        assert!(!is_newer_version("1.0.0", "v0.9.9"));
        assert!(!is_newer_version("0.1.2", ""));
        assert!(!is_newer_version("0.1.2", "invalid"));
        assert!(!is_newer_version("invalid", "0.1.3"));
    }

    #[test]
    fn parse_release_json_extracts_installer_exe_and_falls_back_to_html_url() {
        let release_with_exe = serde_json::json!({
            "tag_name": "v0.2.0",
            "html_url": "https://github.com/matheuswidder/w-workploy/releases/tag/v0.2.0",
            "published_at": "2026-11-18T12:00:00Z",
            "body": "Novidades da versão 0.2.0",
            "assets": [
                {
                    "name": "workploy-0.2.0-win64-portable.zip",
                    "browser_download_url": "https://github.com/matheuswidder/w-workploy/releases/download/v0.2.0/workploy-0.2.0-win64-portable.zip"
                },
                {
                    "name": "Workploy-Setup-0.2.0.exe",
                    "browser_download_url": "https://github.com/matheuswidder/w-workploy/releases/download/v0.2.0/Workploy-Setup-0.2.0.exe"
                }
            ]
        });
        let info = parse_release_json(&release_with_exe, "0.1.2").unwrap();
        assert_eq!(info.tag_name, "v0.2.0");
        assert_eq!(info.version, "0.2.0");
        assert_eq!(
            info.installer_url,
            "https://github.com/matheuswidder/w-workploy/releases/download/v0.2.0/Workploy-Setup-0.2.0.exe"
        );
        assert_eq!(info.published_at, "2026-11-18T12:00:00Z");
        assert_eq!(info.notes, "Novidades da versão 0.2.0");
        assert!(info.has_update);

        let release_without_exe = serde_json::json!({
            "tag_name": "v0.1.2",
            "html_url": "https://github.com/matheuswidder/w-workploy/releases/tag/v0.1.2",
            "assets": []
        });
        let same = parse_release_json(&release_without_exe, "0.1.2").unwrap();
        assert_eq!(same.installer_url, same.html_url);
        assert!(!same.has_update);
    }
}
