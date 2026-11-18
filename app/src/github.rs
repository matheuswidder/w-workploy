use crate::engine::{self, SharedHandle};
use crate::model::*;
use crate::vault;
use crate::vps;
use serde::Serialize;
use std::collections::HashSet;
use std::ffi::OsString;
use std::io::{BufWriter, Read, Write};
use std::path::Path;

#[derive(Serialize)]
struct RunnerConfigPayload {
    poll_secs: u64,
    bindings: Vec<RepoBinding>,
    workflows: Vec<Workflow>,
    purge_rule: PurgeRule,
}

pub fn branch_matches(pattern: &str, branch: &str) -> bool {
    let pat = pattern.trim();
    if pat.is_empty() || pat == "*" {
        return true;
    }
    let target = branch.trim();
    pat.split(',').any(|item| {
        let cand = item.trim();
        cand == "*" || (!cand.is_empty() && cand == target)
    })
}

pub fn expand_branch_targets(pattern: &str, default_branch: &str) -> Vec<String> {
    let def = if default_branch.trim().is_empty() {
        "main"
    } else {
        default_branch.trim()
    };
    let pat = pattern.trim();
    if pat.is_empty() || pat == "*" {
        return vec![def.to_string()];
    }
    let mut out = Vec::new();
    for item in pat.split(',') {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        let resolved = if trimmed == "*" { def } else { trimmed };
        if !out.iter().any(|x| x == resolved) {
            out.push(resolved.to_string());
        }
    }
    if out.is_empty() {
        out.push(def.to_string());
    }
    out
}

pub fn parse_repo_branches_json(v: &serde_json::Value) -> Result<Vec<String>, String> {
    let arr = v
        .as_array()
        .ok_or_else(|| "Resposta inesperada ao listar branches do GitHub.".to_string())?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        if let Some(name) = item.get("name").and_then(|x| x.as_str()).map(str::trim)
            && !name.is_empty()
        {
            out.push(name.to_string());
        }
    }
    Ok(out)
}

pub fn parse_github_workflows_dir_json(v: &serde_json::Value) -> Vec<DetectedWorkflow> {
    let Some(arr) = v.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in arr {
        let kind_type = item.get("type").and_then(|x| x.as_str()).unwrap_or("file");
        if kind_type != "file" {
            continue;
        }
        let Some(name) = item.get("name").and_then(|x| x.as_str()).map(str::trim) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".yml") || lower.ends_with(".yaml") {
            let path = item
                .get("path")
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!(".github/workflows/{name}"));
            out.push(DetectedWorkflow {
                path,
                name: name.to_string(),
                kind: "github-actions".into(),
            });
        }
    }
    out
}

pub fn parse_repo_root_manifests_json(v: &serde_json::Value) -> Vec<DetectedWorkflow> {
    let Some(arr) = v.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in arr {
        let kind_type = item.get("type").and_then(|x| x.as_str()).unwrap_or("file");
        if kind_type != "file" {
            continue;
        }
        let Some(name) = item.get("name").and_then(|x| x.as_str()).map(str::trim) else {
            continue;
        };
        let kind = match name {
            "workploy.yml" | "workploy.yaml" => Some("workploy"),
            "Dockerfile" => Some("docker"),
            "docker-compose.yml" | "docker-compose.yaml" => Some("docker-compose"),
            "package.json" => Some("node"),
            "Cargo.toml" => Some("rust"),
            _ => None,
        };
        if let Some(kind) = kind {
            let path = item
                .get("path")
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| name.to_string());
            out.push(DetectedWorkflow {
                path,
                name: name.to_string(),
                kind: kind.into(),
            });
        }
    }
    out
}

fn github_get_request(url: &str, token: Option<&str>) -> ureq::Request {
    let mut req = ureq::get(url)
        .set("User-Agent", "workploy")
        .set("Accept", "application/vnd.github+json")
        .set("X-GitHub-Api-Version", "2022-11-28");
    if let Some(tok) = token {
        let trimmed = tok.trim();
        if !trimmed.is_empty() {
            req = req.set("Authorization", &format!("Bearer {trimmed}"));
        }
    }
    req
}

pub fn fetch_repo_branches(sh: &SharedHandle, full_name: String) {
    let repo = full_name.trim().to_string();
    if repo.is_empty() {
        let mut s = sh.lock().unwrap();
        s.repo_branches_loading = false;
        s.repo_branches_repo.clear();
        s.repo_branches.clear();
        s.repo_branches_status = "Informe o repositório (dono/repo) para buscar branches.".into();
        drop(s);
        engine::notify(sh);
        return;
    }
    {
        let mut s = sh.lock().unwrap();
        if s.repo_branches_loading && s.repo_branches_repo == repo {
            return;
        }
        s.repo_branches_loading = true;
        s.repo_branches_repo = repo.clone();
        s.repo_branches_status = format!("Buscando branches de {repo}...");
    }
    engine::notify(sh);

    let sh2 = sh.clone();
    std::thread::spawn(move || {
        let token = resolve_github_token(&sh2, None);
        let url = format!("https://api.github.com/repos/{repo}/branches?per_page=100");
        let result = match github_get_request(&url, token.as_deref()).call() {
            Ok(resp) => match resp.into_json::<serde_json::Value>() {
                Ok(v) => parse_repo_branches_json(&v),
                Err(_) => Err("Não foi possível ler a lista de branches do GitHub.".into()),
            },
            Err(ureq::Error::Status(404, _)) => {
                Err(format!("Repositório '{repo}' não encontrado ou sem permissão."))
            }
            Err(ureq::Error::Status(401, _)) => {
                Err("Token do GitHub inválido ou expirado (HTTP 401).".into())
            }
            Err(ureq::Error::Status(code, _)) => {
                Err(format!("GitHub retornou erro HTTP {code} ao buscar branches."))
            }
            Err(ureq::Error::Transport(t)) => {
                Err(format!("Falha de rede ao buscar branches: {t}"))
            }
        };

        {
            let mut s = sh2.lock().unwrap();
            if s.repo_branches_repo == repo {
                s.repo_branches_loading = false;
                match result {
                    Ok(branches) => {
                        s.repo_branches_status = if branches.is_empty() {
                            format!("Nenhuma branch encontrada em {repo}.")
                        } else {
                            format!("{} branch(es) encontrada(s) em {repo}.", branches.len())
                        };
                        s.repo_branches = branches;
                    }
                    Err(err) => {
                        s.repo_branches_status = err;
                    }
                }
            }
        }
        engine::notify(&sh2);
    });
}

pub fn detect_platform_from_paths(paths: &[String]) -> PlatformTarget {
    let norm: Vec<String> = paths
        .iter()
        .map(|p| p.trim().replace('\\', "/").to_ascii_lowercase())
        .filter(|p| !p.is_empty())
        .collect();

    if norm.iter().any(|p| {
        p.contains(".xcodeproj")
            || p.contains(".xcworkspace")
            || p == "info.plist"
            || p.ends_with("/info.plist")
            || p.contains("macos")
    }) {
        return PlatformTarget::MacOS;
    }

    if norm.iter().any(|p| {
        p == "installer"
            || p.starts_with("installer/")
            || p.contains("/installer/")
            || p.ends_with("/installer")
            || p.ends_with(".ps1")
            || p.contains("winresource")
            || p.contains("windows-sys")
            || p.ends_with(".sln")
            || p.ends_with(".vcxproj")
    }) {
        return PlatformTarget::Windows;
    }

    if norm.iter().any(|p| {
        p == "dockerfile"
            || p.ends_with("/dockerfile")
            || p.contains("dockerfile.")
            || p == "docker-compose.yml"
            || p.ends_with("/docker-compose.yml")
            || p == "docker-compose.yaml"
            || p.ends_with("/docker-compose.yaml")
            || p.ends_with(".service")
            || p == "install.sh"
            || p.ends_with("/install.sh")
    }) {
        return PlatformTarget::Linux;
    }

    PlatformTarget::CrossPlatform
}

fn collect_content_paths(v: &serde_json::Value) -> Vec<String> {
    let Some(arr) = v.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let kind_type = item.get("type").and_then(|x| x.as_str()).unwrap_or("file");
        let path = item
            .get("path")
            .and_then(|x| x.as_str())
            .or_else(|| item.get("name").and_then(|x| x.as_str()))
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if let Some(p) = path {
            if kind_type == "dir" && !p.ends_with('/') {
                out.push(format!("{p}/"));
            } else {
                out.push(p.to_string());
            }
        }
    }
    out
}

pub fn apply_detected_platform(s: &mut Shared, repo: &str, platform: PlatformTarget) {
    s.detected_platform = Some((repo.to_string(), platform));
    if platform == PlatformTarget::CrossPlatform {
        return;
    }
    let mut changed = false;
    for b in &mut s.settings.repo_bindings {
        if b.full_name.trim().eq_ignore_ascii_case(repo)
            && b.platform == PlatformTarget::CrossPlatform
        {
            b.platform = platform;
            changed = true;
        }
    }
    if changed {
        save_settings(s);
    }
}

pub fn parse_repo_commits_json(
    v: &serde_json::Value,
    branch: &str,
) -> Result<Vec<RepoCommitItem>, String> {
    let arr = v
        .as_array()
        .ok_or_else(|| "Resposta inesperada ao listar commits do GitHub.".to_string())?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let Some(sha) = item
            .get("sha")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let short_sha: String = sha.chars().take(7).collect();
        let message = item
            .pointer("/commit/message")
            .and_then(|x| x.as_str())
            .and_then(|m| m.lines().next())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("commit")
            .to_string();
        let author = item
            .pointer("/author/login")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                item.pointer("/commit/author/name")
                    .and_then(|x| x.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or("desconhecido")
            .to_string();
        let date = item
            .pointer("/commit/author/date")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        out.push(RepoCommitItem {
            sha: sha.to_string(),
            short_sha,
            message,
            author,
            date,
            branch: branch.to_string(),
        });
    }
    Ok(out)
}

fn encode_query_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for b in raw.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn fetch_repo_commits(sh: &SharedHandle, full_name: String, branch: String) {
    let repo = full_name.trim().to_string();
    let br = {
        let trimmed = branch.trim();
        if trimmed.is_empty() {
            "main".to_string()
        } else {
            trimmed.to_string()
        }
    };
    if repo.is_empty() {
        let mut s = sh.lock().unwrap();
        s.repo_commits_loading = false;
        s.repo_commits_repo.clear();
        s.repo_commits_branch.clear();
        s.repo_commits.clear();
        s.repo_commits_status = "Informe o repositório (dono/repo) para buscar commits.".into();
        drop(s);
        engine::notify(sh);
        return;
    }
    {
        let mut s = sh.lock().unwrap();
        if s.repo_commits_loading && s.repo_commits_repo == repo && s.repo_commits_branch == br {
            return;
        }
        s.repo_commits_loading = true;
        s.repo_commits_repo = repo.clone();
        s.repo_commits_branch = br.clone();
        s.repo_commits_status = format!("Buscando commits de {repo} ({br})...");
    }
    engine::notify(sh);

    let sh2 = sh.clone();
    std::thread::spawn(move || {
        let token = resolve_github_token(&sh2, None);
        let encoded_br = encode_query_component(&br);
        let url = format!("https://api.github.com/repos/{repo}/commits?sha={encoded_br}&per_page=8");
        let result = match github_get_request(&url, token.as_deref()).call() {
            Ok(resp) => match resp.into_json::<serde_json::Value>() {
                Ok(v) => parse_repo_commits_json(&v, &br),
                Err(_) => Err("Não foi possível ler a lista de commits do GitHub.".into()),
            },
            Err(ureq::Error::Status(404, _)) => Err(format!(
                "Repositório '{repo}' ou branch '{br}' não encontrado."
            )),
            Err(ureq::Error::Status(401, _)) => {
                Err("Token do GitHub inválido ou expirado (HTTP 401).".into())
            }
            Err(ureq::Error::Status(code, _)) => Err(format!(
                "GitHub retornou erro HTTP {code} ao buscar commits."
            )),
            Err(ureq::Error::Transport(t)) => {
                Err(format!("Falha de rede ao buscar commits: {t}"))
            }
        };

        {
            let mut s = sh2.lock().unwrap();
            if s.repo_commits_repo == repo && s.repo_commits_branch == br {
                s.repo_commits_loading = false;
                match result {
                    Ok(commits) => {
                        s.repo_commits_status = if commits.is_empty() {
                            format!("Nenhum commit encontrado em {repo} ({br}).")
                        } else {
                            format!("{} commit(s) recente(s) em {repo} ({br}).", commits.len())
                        };
                        s.repo_commits = commits;
                    }
                    Err(err) => {
                        s.repo_commits_status = err;
                    }
                }
            }
        }
        engine::notify(&sh2);
    });
}

pub fn scan_repo_workflows(sh: &SharedHandle, full_name: String) {
    let repo = full_name.trim().to_string();
    if repo.is_empty() {
        let mut s = sh.lock().unwrap();
        s.repo_workflows_loading = false;
        s.repo_workflows_repo.clear();
        s.repo_workflows.clear();
        s.repo_workflows_status = "Informe o repositório (dono/repo) para buscar fluxos.".into();
        drop(s);
        engine::notify(sh);
        return;
    }
    {
        let mut s = sh.lock().unwrap();
        if s.repo_workflows_loading && s.repo_workflows_repo == repo {
            return;
        }
        s.repo_workflows_loading = true;
        s.repo_workflows_repo = repo.clone();
        s.repo_workflows_status = format!("Buscando fluxos automatizados em {repo}...");
    }
    engine::notify(sh);

    let sh2 = sh.clone();
    std::thread::spawn(move || {
        let token = resolve_github_token(&sh2, None);

        let wf_url = format!("https://api.github.com/repos/{repo}/contents/.github/workflows");
        let mut gh_workflows = Vec::new();
        let mut all_paths = Vec::new();
        let mut wf_err: Option<String> = None;
        match github_get_request(&wf_url, token.as_deref()).call() {
            Ok(resp) => {
                if let Ok(v) = resp.into_json::<serde_json::Value>() {
                    all_paths.extend(collect_content_paths(&v));
                    gh_workflows = parse_github_workflows_dir_json(&v);
                }
            }
            Err(ureq::Error::Status(404, _)) => {}
            Err(ureq::Error::Status(code, _)) => {
                wf_err = Some(format!("GitHub retornou erro HTTP {code} ao verificar fluxos."));
            }
            Err(ureq::Error::Transport(t)) => {
                wf_err = Some(format!("Falha de rede ao verificar fluxos: {t}"));
            }
        }

        let root_url = format!("https://api.github.com/repos/{repo}/contents/");
        let mut root_items = Vec::new();
        let mut root_err: Option<String> = None;
        match github_get_request(&root_url, token.as_deref()).call() {
            Ok(resp) => {
                if let Ok(v) = resp.into_json::<serde_json::Value>() {
                    all_paths.extend(collect_content_paths(&v));
                    root_items = parse_repo_root_manifests_json(&v);
                }
            }
            Err(ureq::Error::Status(404, _)) => {
                root_err = Some(format!("Repositório '{repo}' não encontrado ou vazio."));
            }
            Err(ureq::Error::Status(code, _)) => {
                root_err = Some(format!("GitHub retornou erro HTTP {code} ao listar arquivos."));
            }
            Err(ureq::Error::Transport(t)) => {
                root_err = Some(format!("Falha de rede ao listar arquivos: {t}"));
            }
        }

        let gh_count = gh_workflows.len();
        let root_count = root_items.len();
        let mut all = gh_workflows;
        all.extend(root_items);
        let detected_platform = detect_platform_from_paths(&all_paths);

        {
            let mut s = sh2.lock().unwrap();
            if s.repo_workflows_repo == repo {
                s.repo_workflows_loading = false;
                if all.is_empty() && (root_err.is_some() || wf_err.is_some()) {
                    s.repo_workflows.clear();
                    s.repo_workflows_status = root_err
                        .or(wf_err)
                        .unwrap_or_else(|| "Não foi possível analisar o repositório.".into());
                } else {
                    s.repo_workflows = all;
                    apply_detected_platform(&mut s, &repo, detected_platform);
                    s.repo_workflows_status = match (gh_count, root_count) {
                        (0, 0) => "Nenhum fluxo automatizado encontrado no repositório.".into(),
                        (g, 0) => format!("{g} fluxo(s) encontrado(s) em .github/workflows"),
                        (0, r) => format!("{r} arquivo(s) de automação encontrado(s) na raiz"),
                        (g, r) => format!(
                            "{g} fluxo(s) encontrado(s) em .github/workflows + {r} arquivo(s) na raiz"
                        ),
                    };
                }
            }
        }
        engine::notify(&sh2);
    });
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
        for wid in engine::matching_workflows_for_binding_with_bindings(
            workflows,
            b,
            &settings.repo_bindings,
        ) {
            needed_wf_ids.insert(wid);
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
        let mut s = sh.lock().unwrap();
        let Some(srv) = s.settings.servers.iter().find(|v| v.id == server_id).cloned() else {
            return;
        };
        let cfg = match build_runner_config_json(&s.settings, &s.workflows, server_id) {
            Ok(c) => c,
            Err(e) => {
                drop(s);
                finish_sync_status(sh, server_id, false, e);
                return;
            }
        };
        let needs_tok = server_needs_github_token(&s.settings, server_id);
        let tok = if !s.settings.github_token.trim().is_empty() {
            Some(s.settings.github_token.trim().to_string())
        } else {
            vault::load_secret(vault::KEY_GITHUB_PAT).ok().flatten()
        };
        mark_sync_started(&mut s, server_id, &srv.name);
        (srv, cfg, needs_tok, tok)
    };
    engine::notify(sh);

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
                let detail = err.trim();
                let msg = if detail.is_empty() {
                    format!("Falha ao enviar configuração para '{}'.", server.name)
                } else {
                    format!("Falha ao enviar configuração para '{}': {detail}", server.name)
                };
                finish_sync_status(&sh2, &server.id, false, msg);
                return;
            }
            Err(e) => {
                finish_sync_status(&sh2, &server.id, false, e);
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
                    let detail = err.trim();
                    let msg = if detail.is_empty() {
                        format!(
                            "Configuração enviada, mas falhou ao gravar github_token em '{}'.",
                            server.name
                        )
                    } else {
                        format!(
                            "Configuração enviada, mas falhou ao gravar github_token em '{}': {detail}",
                            server.name
                        )
                    };
                    finish_sync_status(&sh2, &server.id, false, msg);
                    return;
                }
                Err(e) => {
                    finish_sync_status(&sh2, &server.id, false, e);
                    return;
                }
            }
        }

        let reload_cmd = format!(
            "sh -c 'TOKEN=$(cat /etc/workploy/token 2>/dev/null) && curl -fsS -X POST -H \"Authorization: Bearer $TOKEN\" http://127.0.0.1:{port}/reload'"
        );
        let (ok, msg) = match vps::ssh_run_capture(&server, &reload_cmd) {
            Ok((true, _, _)) => (
                true,
                format!("Regras e repositórios sincronizados em '{}'.", server.name),
            ),
            Ok((false, _, err)) => {
                let detail = err.trim();
                let m = if detail.is_empty() {
                    format!(
                        "Configuração enviada para '{}', mas o reload falhou.",
                        server.name
                    )
                } else {
                    format!(
                        "Configuração enviada para '{}', mas o reload falhou: {detail}",
                        server.name
                    )
                };
                (false, m)
            }
            Err(e) => (false, e),
        };
        finish_sync_status(&sh2, &server.id, ok, msg);
    });
}

fn record_sync_event(s: &mut Shared, ok: bool, msg: String) {
    let seq = s.sync_last_event.as_ref().map(|e| e.0).unwrap_or(0) + 1;
    s.sync_last_event = Some((seq, ok, msg));
}

fn short_sync_error(msg: &str) -> String {
    let first_line = msg
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("Falha ao sincronizar na VPS.");
    const MAX_CHARS: usize = 96;
    if first_line.chars().count() > MAX_CHARS {
        let truncated: String = first_line.chars().take(MAX_CHARS - 1).collect();
        format!("{truncated}…")
    } else {
        first_line.to_string()
    }
}

fn mark_sync_started(s: &mut Shared, server_id: &str, server_name: &str) {
    let status = format!("Sincronizando em '{server_name}'...");
    for b in &mut s.settings.repo_bindings {
        if b.enabled && b.target_server_id.as_deref() == Some(server_id) {
            b.last_status = status.clone();
        }
    }
    s.runner_sync_status = status;
}

fn apply_sync_status(s: &mut Shared, server_id: &str, ok: bool, msg: String) {
    let binding_status = if ok {
        "Sincronizado na VPS".to_string()
    } else {
        short_sync_error(&msg)
    };
    for b in &mut s.settings.repo_bindings {
        if b.target_server_id.as_deref() == Some(server_id)
            && (b.enabled || b.last_status.starts_with("Sincronizando"))
        {
            b.last_status = binding_status.clone();
        }
    }
    s.runner_sync_status = msg.clone();
    record_sync_event(s, ok, msg);
}

fn finish_sync_status(sh: &SharedHandle, server_id: &str, ok: bool, msg: String) {
    {
        let mut s = sh.lock().unwrap();
        apply_sync_status(&mut s, server_id, ok, msg);
        save_settings(&s);
    }
    engine::notify(sh);
}

pub fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    fn val(b: u8) -> Option<u8> {
        match b {
            b'A'..=b'Z' => Some(b - b'A'),
            b'a'..=b'z' => Some(b - b'a' + 26),
            b'0'..=b'9' => Some(b - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .collect();
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if !bytes.len().is_multiple_of(4) {
        return Err("Conteúdo base64 com tamanho inválido.".into());
    }
    let (chunks, _) = bytes.as_chunks::<4>();
    let mut out = Vec::with_capacity(chunks.len() * 3);
    for chunk in chunks {
        let a = val(chunk[0]).ok_or_else(|| "Caractere base64 inválido.".to_string())?;
        let b = val(chunk[1]).ok_or_else(|| "Caractere base64 inválido.".to_string())?;
        out.push((a << 2) | (b >> 4));
        if chunk[2] != b'=' {
            let c = val(chunk[2]).ok_or_else(|| "Caractere base64 inválido.".to_string())?;
            out.push((b << 4) | (c >> 2));
            if chunk[3] != b'=' {
                let d = val(chunk[3]).ok_or_else(|| "Caractere base64 inválido.".to_string())?;
                out.push((c << 6) | d);
            }
        }
    }
    Ok(out)
}

pub fn decode_github_file_response(raw: &str) -> Result<String, String> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(raw)
        && v.get("encoding").and_then(|e| e.as_str()) == Some("base64")
        && let Some(content) = v.get("content").and_then(|c| c.as_str())
    {
        let bytes = decode_base64(content)?;
        return String::from_utf8(bytes)
            .map_err(|_| "Arquivo do GitHub não está em UTF-8 válido.".to_string());
    }
    Ok(raw.to_string())
}

pub fn build_steps_from_package_json_scripts(
    package_json: &serde_json::Value,
    has_pnpm_lock: bool,
) -> Vec<Step> {
    let uses_pnpm = has_pnpm_lock
        || package_json
            .get("packageManager")
            .and_then(|v| v.as_str())
            .is_some_and(|pm| pm.trim().to_ascii_lowercase().starts_with("pnpm"));

    let (pm, install_cmd) = if uses_pnpm {
        ("pnpm", "pnpm install --frozen-lockfile")
    } else {
        ("npm", "npm ci")
    };

    let mut steps = vec![Step {
        name: "Instalar dependências".into(),
        run: install_cmd.into(),
        approval: false,
    }];

    if let Some(scripts) = package_json.get("scripts").and_then(|v| v.as_object()) {
        const KNOWN_SCRIPTS: &[(&str, &str)] = &[
            ("typecheck", "Verificação de tipos (typecheck)"),
            ("lint", "Lint"),
            ("test", "Testes"),
            ("test:unit", "Testes unitários"),
            ("test:e2e", "Testes E2E"),
            ("build", "Build"),
        ];
        for &(script_key, label) in KNOWN_SCRIPTS {
            if scripts
                .get(script_key)
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.trim().is_empty())
            {
                steps.push(Step {
                    name: label.into(),
                    run: format!("{pm} run {script_key}"),
                    approval: false,
                });
            }
        }
    }

    steps
}

pub fn build_rust_ci_steps() -> Vec<Step> {
    vec![
        Step {
            name: "Verificação de tipos (cargo check)".into(),
            run: "cargo check --all-targets".into(),
            approval: false,
        },
        Step {
            name: "Lint Rust (clippy)".into(),
            run: "cargo clippy --no-deps -- -D warnings".into(),
            approval: false,
        },
        Step {
            name: "Testes unitários (cargo test)".into(),
            run: "cargo test".into(),
            approval: false,
        },
    ]
}

pub fn build_generated_workflows_for_repo(
    binding: &RepoBinding,
    package_json: Option<&serde_json::Value>,
    has_pnpm_lock: bool,
    has_cargo_toml: bool,
    has_compose: bool,
    has_dockerfile: bool,
) -> Result<(Workflow, Option<Workflow>), String> {
    let mut ci_steps: Vec<Step> = Vec::new();
    if let Some(pkg) = package_json {
        ci_steps.extend(build_steps_from_package_json_scripts(pkg, has_pnpm_lock));
    }
    if has_cargo_toml {
        ci_steps.extend(build_rust_ci_steps());
    }
    if ci_steps.is_empty() {
        if has_compose {
            ci_steps.push(Step {
                name: "Validar Docker Compose".into(),
                run: "docker compose config -q".into(),
                approval: false,
            });
        } else if has_dockerfile {
            ci_steps.push(Step {
                name: "Validar build Docker".into(),
                run: "docker build -t workploy-ci .".into(),
                approval: false,
            });
        }
    }
    if ci_steps.is_empty() {
        return Err(format!(
            "Não foi possível detectar package.json, Cargo.toml ou Docker em '{}'.",
            binding.full_name
        ));
    }

    let clean_repo = binding.full_name.trim();
    let repo_short = clean_repo.rsplit('/').next().unwrap_or(clean_repo);
    let runner = if binding.target_server_id.is_some() {
        RunnerKind::Ssh
    } else {
        RunnerKind::Local
    };
    let base_id = new_id("wf");

    let deploy_wf = if has_compose || has_dockerfile || binding.target_server_id.is_some() {
        Some(Workflow {
            id: format!("{base_id}-deploy"),
            name: format!("Deploy — {repo_short}"),
            trigger: Trigger::Manual,
            runner,
            workdir: String::new(),
            steps: vec![
                Step {
                    name: "Construir containers (docker compose build)".into(),
                    run: "docker compose build".into(),
                    approval: false,
                },
                Step {
                    name: "Subir serviços (docker compose up)".into(),
                    run: "docker compose up -d --remove-orphans".into(),
                    approval: true,
                },
            ],
            on_success: None,
            server_id: binding.target_server_id.clone(),
            repo_id: Some(binding.id.clone()),
        })
    } else {
        None
    };

    let ci_wf = Workflow {
        id: format!("{base_id}-ci"),
        name: format!("CI — {repo_short}"),
        trigger: Trigger::Push,
        runner,
        workdir: String::new(),
        steps: ci_steps,
        on_success: deploy_wf.as_ref().map(|d| d.id.clone()),
        server_id: binding.target_server_id.clone(),
        repo_id: Some(binding.id.clone()),
    };

    Ok((ci_wf, deploy_wf))
}

fn finish_generate_workflow_error(sh: &SharedHandle, binding_id: &str, msg: String) {
    {
        let mut s = sh.lock().unwrap();
        if let Some(b) = s
            .settings
            .repo_bindings
            .iter_mut()
            .find(|x| x.id == binding_id)
        {
            b.last_status = short_sync_error(&msg);
        }
        s.runner_sync_status = msg.clone();
        record_sync_event(&mut s, false, msg);
    }
    engine::notify(sh);
}

pub fn generate_workflow_from_repo(sh: &SharedHandle, binding_id: String) {
    let (binding, default_branch) = {
        let mut s = sh.lock().unwrap();
        let Some(b) = s
            .settings
            .repo_bindings
            .iter()
            .find(|x| x.id == binding_id)
            .cloned()
        else {
            record_sync_event(&mut s, false, "Vínculo de repositório não encontrado.".into());
            drop(s);
            engine::notify(sh);
            return;
        };
        if b.full_name.trim().is_empty() {
            record_sync_event(
                &mut s,
                false,
                "Informe o repositório (dono/repo) antes de gerar a receita.".into(),
            );
            drop(s);
            engine::notify(sh);
            return;
        }
        let def_br = s
            .github_repos
            .iter()
            .find(|r| r.full_name.eq_ignore_ascii_case(b.full_name.trim()))
            .map(|r| r.default_branch.clone())
            .filter(|br| !br.trim().is_empty())
            .unwrap_or_else(|| "main".into());
        let status = format!("Analisando {} para gerar receita...", b.full_name.trim());
        s.runner_sync_status = status.clone();
        if let Some(entry) = s
            .settings
            .repo_bindings
            .iter_mut()
            .find(|x| x.id == binding_id)
        {
            entry.last_status = "Gerando receita...".into();
        }
        (b, def_br)
    };
    engine::notify(sh);

    let sh2 = sh.clone();
    std::thread::spawn(move || {
        let repo = binding.full_name.trim().to_string();
        let token = resolve_github_token(&sh2, None);
        let branch = expand_branch_targets(&binding.branch, &default_branch)
            .into_iter()
            .next()
            .unwrap_or_else(|| default_branch.clone());
        let ref_query = format!("?ref={}", encode_query_component(&branch));

        let root_url = format!("https://api.github.com/repos/{repo}/contents/{ref_query}");
        let mut all_paths = Vec::new();
        let root_entries = match github_get_request(&root_url, token.as_deref()).call() {
            Ok(resp) => match resp.into_json::<serde_json::Value>() {
                Ok(v) => {
                    all_paths.extend(collect_content_paths(&v));
                    v
                }
                Err(_) => {
                    finish_generate_workflow_error(
                        &sh2,
                        &binding_id,
                        format!("Não foi possível ler os arquivos da raiz de {repo}."),
                    );
                    return;
                }
            },
            Err(ureq::Error::Status(401, _)) => {
                finish_generate_workflow_error(
                    &sh2,
                    &binding_id,
                    "Token do GitHub inválido ou expirado (HTTP 401).".into(),
                );
                return;
            }
            Err(ureq::Error::Status(404, _)) => {
                finish_generate_workflow_error(
                    &sh2,
                    &binding_id,
                    format!("Repositório '{repo}' ou branch '{branch}' não encontrado."),
                );
                return;
            }
            Err(ureq::Error::Status(code, _)) => {
                finish_generate_workflow_error(
                    &sh2,
                    &binding_id,
                    format!("GitHub retornou HTTP {code} ao analisar {repo}."),
                );
                return;
            }
            Err(ureq::Error::Transport(t)) => {
                finish_generate_workflow_error(
                    &sh2,
                    &binding_id,
                    format!("Falha de rede ao analisar {repo}: {t}"),
                );
                return;
            }
        };

        let wf_url =
            format!("https://api.github.com/repos/{repo}/contents/.github/workflows{ref_query}");
        let mut gh_workflows = Vec::new();
        if let Ok(resp) = github_get_request(&wf_url, token.as_deref()).call()
            && let Ok(v) = resp.into_json::<serde_json::Value>()
        {
            all_paths.extend(collect_content_paths(&v));
            gh_workflows = parse_github_workflows_dir_json(&v);
        }

        let root_names: Vec<String> = root_entries
            .as_array()
            .into_iter()
            .flatten()
            .filter(|item| item.get("type").and_then(|x| x.as_str()).unwrap_or("file") == "file")
            .filter_map(|item| item.get("name").and_then(|x| x.as_str()))
            .map(|s| s.trim().to_string())
            .collect();

        let has_file = |target: &str| root_names.iter().any(|n| n == target);
        let has_package_json = has_file("package.json");
        let has_pnpm_lock = has_file("pnpm-lock.yaml");
        let has_cargo_toml = has_file("Cargo.toml");
        let has_compose = has_file("docker-compose.yml")
            || has_file("docker-compose.yaml")
            || has_file("compose.yml")
            || has_file("compose.yaml");
        let has_dockerfile = root_names
            .iter()
            .any(|n| n.eq_ignore_ascii_case("Dockerfile"));

        let mut package_json: Option<serde_json::Value> = None;
        if has_package_json {
            let pkg_url =
                format!("https://api.github.com/repos/{repo}/contents/package.json{ref_query}");
            let req = github_get_request(&pkg_url, token.as_deref())
                .set("Accept", "application/vnd.github.raw+json");
            match req.call() {
                Ok(resp) => match resp.into_string() {
                    Ok(raw) => match decode_github_file_response(&raw) {
                        Ok(decoded) => match serde_json::from_str::<serde_json::Value>(&decoded) {
                            Ok(parsed) => package_json = Some(parsed),
                            Err(e) => {
                                finish_generate_workflow_error(
                                    &sh2,
                                    &binding_id,
                                    format!("Falha ao interpretar package.json de {repo}: {e}"),
                                );
                                return;
                            }
                        },
                        Err(e) => {
                            finish_generate_workflow_error(&sh2, &binding_id, e);
                            return;
                        }
                    },
                    Err(e) => {
                        finish_generate_workflow_error(
                            &sh2,
                            &binding_id,
                            format!("Falha ao ler package.json de {repo}: {e}"),
                        );
                        return;
                    }
                },
                Err(ureq::Error::Status(code, _)) => {
                    finish_generate_workflow_error(
                        &sh2,
                        &binding_id,
                        format!("GitHub retornou HTTP {code} ao ler package.json de {repo}."),
                    );
                    return;
                }
                Err(ureq::Error::Transport(t)) => {
                    finish_generate_workflow_error(
                        &sh2,
                        &binding_id,
                        format!("Falha de rede ao ler package.json de {repo}: {t}"),
                    );
                    return;
                }
            }
        }

        let (ci_wf, deploy_wf) = match build_generated_workflows_for_repo(
            &binding,
            package_json.as_ref(),
            has_pnpm_lock,
            has_cargo_toml,
            has_compose,
            has_dockerfile,
        ) {
            Ok(pair) => pair,
            Err(err) => {
                finish_generate_workflow_error(&sh2, &binding_id, err);
                return;
            }
        };

        let detected_platform = detect_platform_from_paths(&all_paths);
        let ci_id = ci_wf.id.clone();
        let ci_name = ci_wf.name.clone();
        let deploy_name = deploy_wf.as_ref().map(|d| d.name.clone());
        let target_server_id = binding.target_server_id.clone();

        {
            let mut s = sh2.lock().unwrap();
            s.workflows.push(ci_wf);
            if let Some(dwf) = deploy_wf {
                s.workflows.push(dwf);
            }
            save_workflows(&s);

            if let Some(entry) = s
                .settings
                .repo_bindings
                .iter_mut()
                .find(|x| x.id == binding_id)
            {
                entry.workflow_id = Some(ci_id);
                entry.last_status = "Receita gerada".into();
            }
            if !gh_workflows.is_empty() && s.repo_workflows_repo == repo {
                s.repo_workflows = gh_workflows;
            }
            apply_detected_platform(&mut s, &repo, detected_platform);
            save_settings(&s);

            let msg = if let Some(dn) = deploy_name {
                format!("Receita '{ci_name}' + '{dn}' gerada para {repo}.")
            } else {
                format!("Receita '{ci_name}' gerada para {repo}.")
            };
            s.runner_sync_status = msg.clone();
            record_sync_event(&mut s, true, msg);
        }
        engine::notify(&sh2);

        if let Some(srv_id) = target_server_id {
            sync_repo_bindings_to_vps(&sh2, &srv_id);
        }
    });
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

pub fn parse_release_from_redirect_location(
    location: &str,
    current_version: &str,
) -> Result<Option<ReleaseInfo>, String> {
    let trimmed = location.trim();
    if trimmed.is_empty() {
        return Err("Resposta de redirecionamento do GitHub sem cabeçalho Location.".into());
    }
    let Some((_, after_tag)) = trimmed.split_once("/releases/tag/") else {
        // Quando não há nenhuma release publicada, /releases/latest redireciona para /releases
        if trimmed.ends_with("/releases") || trimmed.contains("/releases?") {
            return Ok(None);
        }
        return Err("Não foi possível identificar a versão da release no redirecionamento.".into());
    };
    let tag_name = after_tag
        .split(['?', '#', '/'])
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Tag de release vazia no redirecionamento do GitHub.".to_string())?
        .to_string();

    let version = tag_name.trim_start_matches(['v', 'V']).to_string();
    let html_url = format!("https://github.com/matheuswidder/w-workploy/releases/tag/{tag_name}");
    let installer_url = format!(
        "https://github.com/matheuswidder/w-workploy/releases/download/{tag_name}/Workploy-Setup-{version}.exe"
    );
    let has_update = is_newer_version(current_version, &tag_name);

    Ok(Some(ReleaseInfo {
        tag_name,
        version,
        html_url,
        installer_url,
        published_at: String::new(),
        notes: String::new(),
        has_update,
    }))
}

fn fetch_latest_release_via_redirect(current_version: &str) -> Result<Option<ReleaseInfo>, String> {
    let agent = ureq::AgentBuilder::new().redirects(0).build();
    let resp = agent
        .get("https://github.com/matheuswidder/w-workploy/releases/latest")
        .set("User-Agent", "workploy")
        .call();

    match resp {
        Ok(r) => {
            if let Some(loc) = r.header("Location") {
                parse_release_from_redirect_location(loc, current_version)
            } else {
                Err("GitHub não retornou redirecionamento para a última release.".into())
            }
        }
        Err(ureq::Error::Status(301 | 302 | 303 | 307 | 308, r)) => {
            if let Some(loc) = r.header("Location") {
                parse_release_from_redirect_location(loc, current_version)
            } else {
                Err("GitHub não retornou cabeçalho Location ao buscar release.".into())
            }
        }
        Err(ureq::Error::Status(404, _)) => Ok(None),
        Err(ureq::Error::Status(code, _)) => Err(format!(
            "GitHub retornou erro HTTP {code} ao verificar atualizações."
        )),
        Err(ureq::Error::Transport(t)) => {
            Err(format!("Não foi possível verificar atualizações: {t}"))
        }
    }
}

fn apply_release_outcome(sh: &SharedHandle, current_ver: &str, outcome: Result<Option<ReleaseInfo>, String>) {
    let mut s = sh.lock().unwrap();
    s.update_checking = false;
    match outcome {
        Ok(Some(info)) => {
            let status = if info.has_update {
                format!(
                    "Nova versão disponível: v{} (atual: v{current_ver})",
                    info.version
                )
            } else {
                format!("Você já está na versão mais recente (v{current_ver}).")
            };
            s.latest_release = Some(info);
            s.update_status = status.clone();
            record_update_event(&mut s, true, status);
        }
        Ok(None) => {
            let msg = "Nenhuma release publicada ainda em matheuswidder/w-workploy.".to_string();
            s.latest_release = None;
            s.update_status = msg.clone();
            record_update_event(&mut s, true, msg);
        }
        Err(msg) => {
            s.update_status = msg.clone();
            record_update_event(&mut s, false, msg);
        }
    }
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
        let token = resolve_github_token(&sh2, None);
        let url = "https://api.github.com/repos/matheuswidder/w-workploy/releases/latest";
        let resp = github_get_request(url, token.as_deref()).call();
        let current_ver = env!("CARGO_PKG_VERSION");

        let outcome = match resp {
            Ok(r) => match r.into_json::<serde_json::Value>() {
                Ok(v) => parse_release_json(&v, current_ver).map(Some),
                Err(_) => Err("Não foi possível ler os dados da release no GitHub.".to_string()),
            },
            Err(ureq::Error::Status(403 | 429, _)) => {
                fetch_latest_release_via_redirect(current_ver)
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(ureq::Error::Status(code, _)) => Err(format!(
                "GitHub retornou erro HTTP {code} ao verificar atualizações."
            )),
            Err(ureq::Error::Transport(t)) => {
                Err(format!("Não foi possível verificar atualizações: {t}"))
            }
        };

        apply_release_outcome(&sh2, current_ver, outcome);
        engine::notify(&sh2);
    });
}

const MIN_INSTALLER_BYTES: u64 = 500 * 1024;
const DOWNLOAD_CHUNK_BYTES: usize = 64 * 1024;
const PROGRESS_NOTIFY_BYTES: u64 = 256 * 1024;

pub fn sanitize_installer_version(version: &str) -> String {
    let cleaned: String = version
        .trim()
        .trim_start_matches(['v', 'V'])
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect();
    if cleaned.is_empty() {
        "latest".to_string()
    } else {
        cleaned
    }
}

pub fn is_dev_target_dir(exe_dir: &Path) -> bool {
    exe_dir.components().any(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("target"))
    })
}

pub fn build_installer_args(current_exe: Option<&Path>) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    if let Some(exe) = current_exe
        && let Some(parent) = exe.parent()
        && !parent.as_os_str().is_empty()
        && !is_dev_target_dir(parent)
    {
        args.push("--dir".into());
        args.push(parent.as_os_str().to_os_string());
    }
    args.push("--silent".into());
    args.push("--launch".into());
    args
}

pub fn validate_downloaded_installer(path: &Path) -> Result<u64, String> {
    let meta = std::fs::metadata(path)
        .map_err(|e| format!("Não foi possível ler o instalador baixado: {e}"))?;
    let len = meta.len();
    if len <= MIN_INSTALLER_BYTES {
        return Err(format!(
            "Arquivo de atualização inválido ou incompleto ({} KB).",
            len / 1024
        ));
    }
    #[cfg(target_os = "windows")]
    {
        let mut file = std::fs::File::open(path)
            .map_err(|e| format!("Falha ao abrir instalador baixado: {e}"))?;
        let mut magic = [0u8; 2];
        file.read_exact(&mut magic)
            .map_err(|e| format!("Falha ao validar cabeçalho do instalador: {e}"))?;
        if &magic != b"MZ" {
            return Err(
                "O arquivo baixado não é um executável Windows válido (assinatura MZ ausente)."
                    .into(),
            );
        }
    }
    Ok(len)
}

fn format_download_progress(version: &str, downloaded: u64, total_bytes: Option<u64>) -> (f32, String) {
    let dl_mb = downloaded as f64 / (1024.0 * 1024.0);
    if let Some(total) = total_bytes
        && total > 0
    {
        let frac = (downloaded as f64 / total as f64).clamp(0.0, 1.0) as f32;
        let pct = (frac * 100.0).round() as u32;
        let tot_mb = total as f64 / (1024.0 * 1024.0);
        (
            frac,
            format!("Baixando Workploy v{version}... {pct}% ({dl_mb:.1} / {tot_mb:.1} MB)"),
        )
    } else {
        (
            0.0,
            format!("Baixando Workploy v{version}... ({dl_mb:.1} MB)"),
        )
    }
}

fn finish_update_download_error(sh: &SharedHandle, msg: String) {
    {
        let mut s = sh.lock().unwrap();
        s.update_downloading = false;
        s.update_progress = 0.0;
        s.update_status = msg.clone();
        record_update_event(&mut s, false, msg);
    }
    engine::notify(sh);
}

pub fn download_installer_stream(
    sh: &SharedHandle,
    installer_url: &str,
    version: &str,
    dest_path: &Path,
    token: Option<&str>,
) -> Result<u64, String> {
    let url = installer_url.trim();
    if url.is_empty() {
        return Err("URL do instalador não informada.".into());
    }

    let mut req = ureq::get(url).set("User-Agent", "workploy");
    if let Some(tok) = token {
        let trimmed = tok.trim();
        if !trimmed.is_empty() {
            req = req.set("Authorization", &format!("Bearer {trimmed}"));
        }
    }

    let resp = req.call().map_err(|e| match e {
        ureq::Error::Status(code, _) => {
            format!("Falha ao baixar atualização (HTTP {code}).")
        }
        ureq::Error::Transport(t) => {
            format!("Falha de rede ao baixar atualização: {t}")
        }
    })?;

    let total_bytes = resp
        .header("Content-Length")
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&n| n > 0);

    if let Some(parent) = dest_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Falha ao criar pasta temporária: {e}"))?;
    }

    let part_path = dest_path.with_extension("exe.part");
    let file = std::fs::File::create(&part_path)
        .map_err(|e| format!("Falha ao criar arquivo temporário da atualização: {e}"))?;
    let mut writer = BufWriter::new(file);
    let mut reader = resp.into_reader();
    let mut buf = vec![0u8; DOWNLOAD_CHUNK_BYTES];
    let mut downloaded: u64 = 0;
    let mut last_notified: u64 = 0;

    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                drop(writer);
                let _ = std::fs::remove_file(&part_path);
                return Err(format!("Falha durante o download da atualização: {e}"));
            }
        };
        if let Err(e) = writer.write_all(&buf[..n]) {
            drop(writer);
            let _ = std::fs::remove_file(&part_path);
            return Err(format!("Falha ao gravar instalador temporário: {e}"));
        }
        downloaded += n as u64;
        if downloaded - last_notified >= PROGRESS_NOTIFY_BYTES {
            last_notified = downloaded;
            let (progress, status) = format_download_progress(version, downloaded, total_bytes);
            {
                let mut s = sh.lock().unwrap();
                s.update_progress = progress;
                s.update_status = status;
            }
            engine::notify(sh);
        }
    }

    if let Err(e) = writer.flush() {
        drop(writer);
        let _ = std::fs::remove_file(&part_path);
        return Err(format!("Falha ao finalizar gravação do instalador: {e}"));
    }
    drop(writer);

    if dest_path.exists() {
        let _ = std::fs::remove_file(dest_path);
    }
    if let Err(e) = std::fs::rename(&part_path, dest_path) {
        let _ = std::fs::remove_file(&part_path);
        return Err(format!("Falha ao preparar arquivo do instalador: {e}"));
    }

    match validate_downloaded_installer(dest_path) {
        Ok(len) => {
            {
                let mut s = sh.lock().unwrap();
                s.update_progress = 1.0;
            }
            engine::notify(sh);
            Ok(len)
        }
        Err(e) => {
            let _ = std::fs::remove_file(dest_path);
            Err(e)
        }
    }
}

pub fn download_and_install_update(sh: &SharedHandle, installer_url: String, version: String) {
    {
        let mut s = sh.lock().unwrap();
        if s.update_downloading {
            return;
        }
        s.update_downloading = true;
        s.update_progress = 0.0;
        s.update_status = format!("Baixando Workploy v{version}...");
    }
    engine::notify(sh);

    let sh2 = sh.clone();
    std::thread::spawn(move || {
        let url = installer_url.trim().to_string();
        if url.is_empty() {
            finish_update_download_error(&sh2, "URL do instalador não informada.".into());
            return;
        }

        let clean_ver = sanitize_installer_version(&version);
        let temp_exe = std::env::temp_dir().join(format!("Workploy-Setup-{clean_ver}.exe"));
        let token = resolve_github_token(&sh2, None);

        if let Err(err) = download_installer_stream(&sh2, &url, &version, &temp_exe, token.as_deref()) {
            finish_update_download_error(&sh2, err);
            return;
        }

        #[cfg(target_os = "windows")]
        {
            let current_exe = std::env::current_exe().ok();
            let args = build_installer_args(current_exe.as_deref());
            match std::process::Command::new(&temp_exe).args(&args).spawn() {
                Ok(_) => {
                    {
                        let mut s = sh2.lock().unwrap();
                        s.update_progress = 1.0;
                        s.update_status = "Reiniciando para concluir atualização...".into();
                    }
                    engine::notify(&sh2);
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    std::process::exit(0);
                }
                Err(e) => {
                    finish_update_download_error(
                        &sh2,
                        format!("Não foi possível iniciar o instalador da atualização: {e}"),
                    );
                }
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            {
                let mut s = sh2.lock().unwrap();
                s.update_downloading = false;
                s.update_progress = 1.0;
                let msg = format!(
                    "Atualização v{version} baixada em {}.",
                    temp_exe.display()
                );
                s.update_status = msg.clone();
                record_update_event(&mut s, true, msg);
            }
            engine::notify(&sh2);
        }
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
            platform: PlatformTarget::CrossPlatform,
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
            platform: PlatformTarget::CrossPlatform,
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
            platform: PlatformTarget::CrossPlatform,
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
            platform: PlatformTarget::CrossPlatform,
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
            platform: PlatformTarget::CrossPlatform,
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
            platform: PlatformTarget::CrossPlatform,
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

    #[test]
    fn parse_release_redirect_location_builds_release_and_handles_no_releases() {
        let info = parse_release_from_redirect_location(
            "https://github.com/matheuswidder/w-workploy/releases/tag/v0.1.4",
            "0.1.3",
        )
        .unwrap()
        .expect("should parse release");
        assert_eq!(info.tag_name, "v0.1.4");
        assert_eq!(info.version, "0.1.4");
        assert_eq!(
            info.html_url,
            "https://github.com/matheuswidder/w-workploy/releases/tag/v0.1.4"
        );
        assert_eq!(
            info.installer_url,
            "https://github.com/matheuswidder/w-workploy/releases/download/v0.1.4/Workploy-Setup-0.1.4.exe"
        );
        assert!(info.has_update);

        let same = parse_release_from_redirect_location("/matheuswidder/w-workploy/releases/tag/v0.1.3?foo=1", "0.1.3")
            .unwrap()
            .unwrap();
        assert_eq!(same.tag_name, "v0.1.3");
        assert!(!same.has_update);

        let none = parse_release_from_redirect_location(
            "https://github.com/matheuswidder/w-workploy/releases",
            "0.1.3",
        )
        .unwrap();
        assert!(none.is_none());
    }

    #[test]
    fn branch_matching_and_expansion_support_wildcard_and_lists() {
        assert!(branch_matches("*", "main"));
        assert!(branch_matches("*", "feature/xyz"));
        assert!(branch_matches("", "develop"));
        assert!(branch_matches("main, dev, staging", "dev"));
        assert!(branch_matches("main,dev", "main"));
        assert!(!branch_matches("main,dev", "staging"));
        assert!(branch_matches("main", "main"));
        assert!(!branch_matches("main", "master"));

        assert_eq!(expand_branch_targets("*", "master"), vec!["master"]);
        assert_eq!(expand_branch_targets("", ""), vec!["main"]);
        assert_eq!(
            expand_branch_targets("main, dev, main, staging", "main"),
            vec!["main", "dev", "staging"]
        );
    }

    #[test]
    fn parses_repo_branches_and_detected_workflows() {
        let branches_json = serde_json::json!([
            { "name": "main" },
            { "name": "staging" },
            { "name": "dev" }
        ]);
        assert_eq!(
            parse_repo_branches_json(&branches_json).unwrap(),
            vec!["main", "staging", "dev"]
        );

        let wf_dir_json = serde_json::json!([
            { "name": "ci.yml", "path": ".github/workflows/ci.yml", "type": "file" },
            { "name": "deploy.yaml", "path": ".github/workflows/deploy.yaml", "type": "file" },
            { "name": "README.md", "path": ".github/workflows/README.md", "type": "file" },
            { "name": "subdir", "path": ".github/workflows/subdir", "type": "dir" }
        ]);
        let ghs = parse_github_workflows_dir_json(&wf_dir_json);
        assert_eq!(ghs.len(), 2);
        assert_eq!(ghs[0].path, ".github/workflows/ci.yml");
        assert_eq!(ghs[0].kind, "github-actions");
        assert_eq!(ghs[1].name, "deploy.yaml");

        let root_json = serde_json::json!([
            { "name": "workploy.yml", "path": "workploy.yml", "type": "file" },
            { "name": "Dockerfile", "path": "Dockerfile", "type": "file" },
            { "name": "docker-compose.yml", "path": "docker-compose.yml", "type": "file" },
            { "name": "package.json", "path": "package.json", "type": "file" },
            { "name": "Cargo.toml", "path": "Cargo.toml", "type": "file" },
            { "name": "README.md", "path": "README.md", "type": "file" }
        ]);
        let roots = parse_repo_root_manifests_json(&root_json);
        assert_eq!(roots.len(), 5);
        assert_eq!(roots[0].kind, "workploy");
        assert_eq!(roots[1].kind, "docker");
        assert_eq!(roots[2].kind, "docker-compose");
        assert_eq!(roots[3].kind, "node");
        assert_eq!(roots[4].kind, "rust");
    }

    #[test]
    fn installer_args_include_dir_for_installed_exe_and_omit_for_target() {
        let installed = Path::new(r"C:\Users\maestro\AppData\Local\Programs\Workploy\workploy.exe");
        let args_installed = build_installer_args(Some(installed));
        assert_eq!(
            args_installed,
            vec![
                OsString::from("--dir"),
                OsString::from(r"C:\Users\maestro\AppData\Local\Programs\Workploy"),
                OsString::from("--silent"),
                OsString::from("--launch"),
            ]
        );

        let dev_debug = Path::new(r"C:\Projects\w-workploy\app\target\debug\workploy.exe");
        assert_eq!(
            build_installer_args(Some(dev_debug)),
            vec![OsString::from("--silent"), OsString::from("--launch")]
        );

        assert_eq!(
            build_installer_args(None),
            vec![OsString::from("--silent"), OsString::from("--launch")]
        );
        assert_eq!(sanitize_installer_version("v0.1.4"), "0.1.4");
    }

    #[test]
    fn download_installer_stream_validates_mz_size_and_updates_progress() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let mut valid_payload = vec![b'A'; 600 * 1024];
        valid_payload[0] = b'M';
        valid_payload[1] = b'Z';

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let addr = listener.local_addr().unwrap();
        let body = valid_payload.clone();
        let srv = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().expect("accept");
                let mut req_buf = [0u8; 1024];
                let _ = stream.read(&mut req_buf);
                let req_str = String::from_utf8_lossy(&req_buf);
                if req_str.contains("GET /bad.exe") {
                    let bad = b"<html>not an exe</html>";
                    let hdr = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        bad.len()
                    );
                    stream.write_all(hdr.as_bytes()).unwrap();
                    stream.write_all(bad).unwrap();
                } else {
                    let hdr = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    stream.write_all(hdr.as_bytes()).unwrap();
                    stream.write_all(&body).unwrap();
                }
            }
        });

        let tmp = std::env::temp_dir().join(format!("workploy-dl-test-{}", new_id("dir")));
        std::fs::create_dir_all(&tmp).unwrap();

        let sh = std::sync::Arc::new(std::sync::Mutex::new(Shared::default()));
        let ok_path = tmp.join("Workploy-Setup-0.1.4.exe");
        let len = download_installer_stream(
            &sh,
            &format!("http://{addr}/Workploy-Setup-0.1.4.exe"),
            "0.1.4",
            &ok_path,
            None,
        )
        .expect("valid exe should download");
        assert_eq!(len, valid_payload.len() as u64);
        assert!(ok_path.exists());
        assert_eq!(sh.lock().unwrap().update_progress, 1.0);

        let bad_path = tmp.join("bad.exe");
        let err = download_installer_stream(
            &sh,
            &format!("http://{addr}/bad.exe"),
            "0.1.4",
            &bad_path,
            None,
        )
        .expect_err("small/html response should fail validation");
        assert!(err.contains("inválido") || err.contains("MZ"));
        assert!(!bad_path.exists());

        srv.join().unwrap();
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn download_and_install_update_records_error_event_on_http_failure() {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind 404 server");
        let addr = listener.local_addr().unwrap();
        let srv = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut req_buf = [0u8; 1024];
            let _ = stream.read(&mut req_buf);
            let hdr = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream.write_all(hdr.as_bytes()).unwrap();
        });

        let sh = std::sync::Arc::new(std::sync::Mutex::new(Shared::default()));
        download_and_install_update(&sh, format!("http://{addr}/missing.exe"), "0.1.4".into());

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            {
                let s = sh.lock().unwrap();
                if !s.update_downloading
                    && let Some((seq, ok, msg)) = s.update_last_event.as_ref()
                {
                    assert_eq!(s.update_progress, 0.0);
                    assert_eq!(*seq, 1);
                    assert!(!ok);
                    assert!(msg.contains("404"));
                    assert_eq!(s.update_status, *msg);
                    break;
                }
            }
            assert!(std::time::Instant::now() < deadline, "timed out waiting for error event");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        srv.join().unwrap();
    }

    #[test]
    fn sync_status_updates_bindings_and_records_monotonic_events() {
        let mut s = Shared::default();
        s.settings.repo_bindings.push(RepoBinding {
            id: "rb-1".into(),
            full_name: "widder/api".into(),
            target_server_id: Some("vps-a".into()),
            enabled: true,
            last_status: String::new(),
            ..RepoBinding::default()
        });
        s.settings.repo_bindings.push(RepoBinding {
            id: "rb-disabled".into(),
            full_name: "widder/paused".into(),
            target_server_id: Some("vps-a".into()),
            enabled: false,
            last_status: "Pausado antes".into(),
            ..RepoBinding::default()
        });
        s.settings.repo_bindings.push(RepoBinding {
            id: "rb-other".into(),
            full_name: "widder/other".into(),
            target_server_id: Some("vps-b".into()),
            enabled: true,
            last_status: "Monitorando".into(),
            ..RepoBinding::default()
        });

        mark_sync_started(&mut s, "vps-a", "Prod");
        assert_eq!(
            s.settings.repo_bindings[0].last_status,
            "Sincronizando em 'Prod'..."
        );
        assert_eq!(s.settings.repo_bindings[1].last_status, "Pausado antes");
        assert_eq!(s.settings.repo_bindings[2].last_status, "Monitorando");
        assert_eq!(s.runner_sync_status, "Sincronizando em 'Prod'...");

        let ok_msg = "Regras e repositórios sincronizados em 'Prod'.".to_string();
        apply_sync_status(&mut s, "vps-a", true, ok_msg.clone());
        assert_eq!(s.settings.repo_bindings[0].last_status, "Sincronizado na VPS");
        assert_eq!(s.settings.repo_bindings[1].last_status, "Pausado antes");
        assert_eq!(s.settings.repo_bindings[2].last_status, "Monitorando");
        assert_eq!(s.runner_sync_status, ok_msg);
        assert_eq!(s.sync_last_event, Some((1, true, ok_msg)));

        let err_msg = "Falha ao enviar configuração para 'Prod': Permission denied\ndetalhe extra".to_string();
        apply_sync_status(&mut s, "vps-a", false, err_msg.clone());
        assert_eq!(
            s.settings.repo_bindings[0].last_status,
            "Falha ao enviar configuração para 'Prod': Permission denied"
        );
        assert_eq!(s.settings.repo_bindings[1].last_status, "Pausado antes");
        assert_eq!(s.settings.repo_bindings[2].last_status, "Monitorando");
        assert_eq!(s.runner_sync_status, err_msg);
        assert_eq!(s.sync_last_event, Some((2, false, err_msg)));
    }

    #[test]
    fn runner_config_includes_generic_push_workflows_and_chained_when_binding_workflow_is_none() {
        let mut settings = Settings::default();
        settings.repo_bindings.push(RepoBinding {
            id: "rb-auto".into(),
            full_name: "widder/site".into(),
            target_server_id: Some("vps-prod".into()),
            branch: "main".into(),
            trigger_mode: TriggerMode::WebhookOnVps,
            workflow_id: None,
            enabled: true,
            last_sha: String::new(),
            last_status: String::new(),
            platform: PlatformTarget::CrossPlatform,
        });

        let workflows = vec![
            Workflow {
                id: "wf-ci-generic".into(),
                name: "CI Principal".into(),
                trigger: Trigger::Push,
                runner: RunnerKind::Local,
                workdir: String::new(),
                steps: vec![Step { name: "Test".into(), run: "echo ok".into(), approval: false }],
                on_success: Some("wf-deploy-generic".into()),
                server_id: None,
                repo_id: None,
            },
            Workflow {
                id: "wf-deploy-generic".into(),
                name: "Deploy VPS".into(),
                trigger: Trigger::Manual,
                runner: RunnerKind::Ssh,
                workdir: String::new(),
                steps: vec![Step { name: "Deploy".into(), run: "echo deploy".into(), approval: true }],
                on_success: None,
                server_id: None,
                repo_id: None,
            },
            Workflow {
                id: "wf-unrelated-manual".into(),
                name: "Isolamento RLS".into(),
                trigger: Trigger::Manual,
                runner: RunnerKind::Local,
                workdir: String::new(),
                steps: vec![],
                on_success: None,
                server_id: None,
                repo_id: None,
            },
        ];

        let cfg_str = build_runner_config_json(&settings, &workflows, "vps-prod").unwrap();
        let cfg: serde_json::Value = serde_json::from_str(&cfg_str).unwrap();
        let wfs = cfg["workflows"].as_array().unwrap();
        assert_eq!(wfs.len(), 2);
        assert!(wfs.iter().any(|w| w["id"] == "wf-ci-generic"));
        assert!(wfs.iter().any(|w| w["id"] == "wf-deploy-generic"));
        assert!(!wfs.iter().any(|w| w["id"] == "wf-unrelated-manual"));
    }

    #[test]
    fn detect_platform_from_paths_identifies_macos_windows_linux_and_cross_platform() {
        assert_eq!(
            detect_platform_from_paths(&[
                "README.md".into(),
                "ios/App.xcodeproj".into(),
                "Dockerfile".into(),
            ]),
            PlatformTarget::MacOS
        );
        assert_eq!(
            detect_platform_from_paths(&["App/Info.plist".into()]),
            PlatformTarget::MacOS
        );
        assert_eq!(
            detect_platform_from_paths(&["macos/Runner.xcworkspace".into()]),
            PlatformTarget::MacOS
        );

        assert_eq!(
            detect_platform_from_paths(&[
                "app/Cargo.toml".into(),
                "installer/".into(),
                "scripts/build-installer.ps1".into(),
            ]),
            PlatformTarget::Windows
        );
        assert_eq!(
            detect_platform_from_paths(&["installer".into()]),
            PlatformTarget::Windows
        );
        assert_eq!(
            detect_platform_from_paths(&["MySolution.sln".into()]),
            PlatformTarget::Windows
        );
        assert_eq!(
            detect_platform_from_paths(&["deps/winresource".into()]),
            PlatformTarget::Windows
        );

        assert_eq!(
            detect_platform_from_paths(&["src/main.ts".into(), "Dockerfile".into()]),
            PlatformTarget::Linux
        );
        assert_eq!(
            detect_platform_from_paths(&["deploy/docker-compose.yml".into()]),
            PlatformTarget::Linux
        );
        assert_eq!(
            detect_platform_from_paths(&["systemd/app.service".into()]),
            PlatformTarget::Linux
        );
        assert_eq!(
            detect_platform_from_paths(&["install.sh".into()]),
            PlatformTarget::Linux
        );

        assert_eq!(
            detect_platform_from_paths(&["README.md".into(), "package.json".into(), "src/index.ts".into()]),
            PlatformTarget::CrossPlatform
        );
        assert_eq!(detect_platform_from_paths(&[]), PlatformTarget::CrossPlatform);
    }

    #[test]
    fn parse_repo_commits_json_extracts_commits_and_fallbacks() {
        let payload = serde_json::json!([
            {
                "sha": "0123456789abcdef0123456789abcdef01234567",
                "commit": {
                    "message": "feat(backend): detecção de plataforma\n\ncorpo detalhado",
                    "author": {
                        "name": "Matheus Widder",
                        "date": "2026-11-18T10:15:00Z"
                    }
                },
                "author": {
                    "login": "matheuswidder"
                }
            },
            {
                "sha": "abcdef1234567",
                "commit": {
                    "message": "fix: ajuste rápido",
                    "author": {
                        "name": "CI Bot",
                        "date": "2026-11-17T09:00:00Z"
                    }
                },
                "author": null
            },
            {
                "sha": "",
                "commit": { "message": "ignorar sem sha" }
            }
        ]);

        let commits = parse_repo_commits_json(&payload, "main").unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].sha, "0123456789abcdef0123456789abcdef01234567");
        assert_eq!(commits[0].short_sha, "0123456");
        assert_eq!(commits[0].message, "feat(backend): detecção de plataforma");
        assert_eq!(commits[0].author, "matheuswidder");
        assert_eq!(commits[0].date, "2026-11-18T10:15:00Z");
        assert_eq!(commits[0].branch, "main");

        assert_eq!(commits[1].short_sha, "abcdef1");
        assert_eq!(commits[1].author, "CI Bot");

        assert!(parse_repo_commits_json(&serde_json::json!({"message": "Not Found"}), "main").is_err());
    }

    #[test]
    fn apply_detected_platform_updates_only_cross_platform_bindings() {
        let mut s = Shared::default();
        s.settings.repo_bindings.push(RepoBinding {
            id: "rb-auto".into(),
            full_name: "matheuswidder/w-workploy".into(),
            platform: PlatformTarget::CrossPlatform,
            ..RepoBinding::default()
        });
        s.settings.repo_bindings.push(RepoBinding {
            id: "rb-manual".into(),
            full_name: "matheuswidder/w-workploy".into(),
            platform: PlatformTarget::Linux,
            ..RepoBinding::default()
        });

        apply_detected_platform(&mut s, "matheuswidder/w-workploy", PlatformTarget::Windows);
        assert_eq!(
            s.detected_platform,
            Some(("matheuswidder/w-workploy".into(), PlatformTarget::Windows))
        );
        assert_eq!(s.settings.repo_bindings[0].platform, PlatformTarget::Windows);
        assert_eq!(s.settings.repo_bindings[1].platform, PlatformTarget::Linux);
    }

    #[test]
    fn build_steps_from_package_json_scripts_detects_pnpm_and_npm_real_commands() {
        let pkg = serde_json::json!({
            "name": "w-erp",
            "scripts": {
                "build": "vite build",
                "test": "vitest run",
                "lint": "eslint .",
                "typecheck": "tsc --noEmit",
                "test:unit": "vitest run unit",
                "test:e2e": "playwright test",
                "dev": "vite"
            }
        });

        let pnpm_steps = build_steps_from_package_json_scripts(&pkg, true);
        let pnpm_cmds: Vec<&str> = pnpm_steps.iter().map(|s| s.run.as_str()).collect();
        assert_eq!(
            pnpm_cmds,
            vec![
                "pnpm install --frozen-lockfile",
                "pnpm run typecheck",
                "pnpm run lint",
                "pnpm run test",
                "pnpm run test:unit",
                "pnpm run test:e2e",
                "pnpm run build",
            ]
        );
        assert!(pnpm_steps.iter().all(|s| !s.run.starts_with("echo ")));

        let npm_steps = build_steps_from_package_json_scripts(&pkg, false);
        let npm_cmds: Vec<&str> = npm_steps.iter().map(|s| s.run.as_str()).collect();
        assert_eq!(
            npm_cmds,
            vec![
                "npm ci",
                "npm run typecheck",
                "npm run lint",
                "npm run test",
                "npm run test:unit",
                "npm run test:e2e",
                "npm run build",
            ]
        );

        let pkg_pm = serde_json::json!({
            "packageManager": "pnpm@9.1.0",
            "scripts": { "test": "vitest", "empty": "   " }
        });
        let pm_steps = build_steps_from_package_json_scripts(&pkg_pm, false);
        assert_eq!(pm_steps[0].run, "pnpm install --frozen-lockfile");
        assert_eq!(pm_steps[1].run, "pnpm run test");
        assert_eq!(pm_steps.len(), 2);
    }

    #[test]
    fn build_generated_workflows_for_repo_creates_chained_ci_and_deploy() {
        let vps_binding = RepoBinding {
            id: "rb-erp".into(),
            full_name: "matheuswidder/w-erp-central".into(),
            target_server_id: Some("vps-prod".into()),
            branch: "main".into(),
            ..RepoBinding::default()
        };
        let pkg = serde_json::json!({
            "scripts": { "typecheck": "tsc", "test": "vitest", "build": "vite build" }
        });

        let (ci, deploy_opt) = build_generated_workflows_for_repo(
            &vps_binding,
            Some(&pkg),
            true,
            false,
            true,
            true,
        )
        .unwrap();

        let deploy = deploy_opt.expect("should generate chained deploy workflow");
        assert_ne!(ci.id, deploy.id);
        assert_eq!(ci.name, "CI — w-erp-central");
        assert_eq!(ci.trigger, Trigger::Push);
        assert_eq!(ci.runner, RunnerKind::Ssh);
        assert_eq!(ci.workdir, "");
        assert_eq!(ci.on_success.as_deref(), Some(deploy.id.as_str()));
        assert_eq!(ci.server_id.as_deref(), Some("vps-prod"));
        assert_eq!(ci.repo_id.as_deref(), Some("rb-erp"));
        assert_eq!(ci.steps.len(), 4);

        assert_eq!(deploy.name, "Deploy — w-erp-central");
        assert_eq!(deploy.trigger, Trigger::Manual);
        assert_eq!(deploy.runner, RunnerKind::Ssh);
        assert_eq!(deploy.steps[0].run, "docker compose build");
        assert_eq!(deploy.steps[1].run, "docker compose up -d --remove-orphans");
        assert!(deploy.steps[1].approval);

        // Rust local sem Docker não gera deploy encadeado
        let local_binding = RepoBinding {
            id: "rb-rust".into(),
            full_name: "widder/cli-tool".into(),
            target_server_id: None,
            ..RepoBinding::default()
        };
        let (rust_ci, rust_deploy) =
            build_generated_workflows_for_repo(&local_binding, None, false, true, false, false)
                .unwrap();
        assert!(rust_deploy.is_none());
        assert_eq!(rust_ci.runner, RunnerKind::Local);
        assert_eq!(
            rust_ci
                .steps
                .iter()
                .map(|s| s.run.as_str())
                .collect::<Vec<_>>(),
            vec![
                "cargo check --all-targets",
                "cargo clippy --no-deps -- -D warnings",
                "cargo test",
            ]
        );

        // Sem stack detectável retorna erro
        assert!(
            build_generated_workflows_for_repo(&local_binding, None, false, false, false, false)
                .is_err()
        );
    }

    #[test]
    fn decode_github_file_response_handles_raw_and_base64_json() {
        let raw = r#"{"name":"app","scripts":{"test":"vitest"}}"#;
        assert_eq!(decode_github_file_response(raw).unwrap(), raw);

        // Base64 de `{"scripts":{"build":"vite build"}}`
        let b64_payload = serde_json::json!({
            "type": "file",
            "encoding": "base64",
            "content": "eyJzY3JpcHRzIjp7ImJ1aWxkIjoidml0ZSBidWlsZCJ9fQ==\n"
        })
        .to_string();
        assert_eq!(
            decode_github_file_response(&b64_payload).unwrap(),
            r#"{"scripts":{"build":"vite build"}}"#
        );
    }
}
