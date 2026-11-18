use crate::engine::{self, SharedHandle};
use crate::model::*;
use crate::vault;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub(crate) struct TempAskpassGuard {
    path: PathBuf,
}

impl Drop for TempAskpassGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn sanitize_slug(raw: &str, fallback: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn generate_ed25519_keypair(name: &str) -> Result<(String, String), String> {
    let keys_dir = data_dir().join("keys");
    std::fs::create_dir_all(&keys_dir)
        .map_err(|e| format!("Não foi possível criar a pasta de chaves: {e}"))?;

    let slug = sanitize_slug(name, "workploy-vps");
    let mut priv_path = keys_dir.join(format!("{slug}.ed25519"));
    if priv_path.exists() {
        priv_path = keys_dir.join(format!("{slug}-{}.ed25519", new_id("k")));
    }

    let mut cmd = Command::new("ssh-keygen");
    engine::apply_no_window(&mut cmd);
    cmd.arg("-t")
        .arg("ed25519")
        .arg("-N")
        .arg("")
        .arg("-C")
        .arg("workploy")
        .arg("-f")
        .arg(&priv_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let out = cmd
        .output()
        .map_err(|e| format!("Não foi possível executar ssh-keygen: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(format!("Falha ao gerar chave SSH Ed25519: {err}"));
    }

    let pub_path = PathBuf::from(format!("{}.pub", priv_path.display()));
    let pub_key = std::fs::read_to_string(&pub_path)
        .map_err(|e| format!("Chave gerada, mas não foi possível ler a chave pública: {e}"))?;

    Ok((
        pub_key.trim().to_string(),
        priv_path.to_string_lossy().to_string(),
    ))
}

fn create_askpass_guard() -> Result<TempAskpassGuard, String> {
    let tmp = std::env::temp_dir();
    #[cfg(target_os = "windows")]
    let path = tmp.join(format!("workploy-askpass-{}.cmd", new_id("sh")));
    #[cfg(not(target_os = "windows"))]
    let path = tmp.join(format!("workploy-askpass-{}.sh", new_id("sh")));

    #[cfg(target_os = "windows")]
    let script = "@echo off\r\npowershell -NoProfile -NonInteractive -Command \"[Console]::Write($env:WORKPLOY_SSH_PASS)\"\r\n";
    #[cfg(not(target_os = "windows"))]
    let script = "#!/bin/sh\nprintf '%s\\n' \"$WORKPLOY_SSH_PASS\"\n";

    std::fs::write(&path, script)
        .map_err(|e| format!("Falha ao preparar autenticador SSH temporário: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700));
    }

    Ok(TempAskpassGuard { path })
}

fn ssh_target(server: &VpsServer) -> String {
    let host = server.host.trim();
    let user = server.user.trim();
    if user.is_empty() || host.contains('@') {
        host.to_string()
    } else {
        format!("{user}@{host}")
    }
}

pub(crate) fn build_ssh_command(
    server: &VpsServer,
    remote_cmd: &str,
) -> Result<(Command, Option<TempAskpassGuard>), String> {
    let host = server.host.trim();
    if host.is_empty() {
        return Err("Endereço ou alias SSH da VPS não informado.".into());
    }

    let mut cmd = Command::new("ssh");
    engine::apply_no_window(&mut cmd);
    cmd.arg("-o")
        .arg("ConnectTimeout=12")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new");

    let mut askpass_guard = None;

    match &server.auth {
        AuthMethod::SshConfigAlias => {
            cmd.arg("-o").arg("BatchMode=yes");
            if server.port != 0 && server.port != 22 {
                cmd.arg("-p").arg(server.port.to_string());
            }
            if !server.user.trim().is_empty() && !host.contains('@') {
                cmd.arg("-l").arg(server.user.trim());
            }
            cmd.arg(host);
        }
        AuthMethod::PrivateKey { path } => {
            let trimmed = path.trim();
            if trimmed.is_empty() {
                return Err("Caminho da chave privada SSH não informado.".into());
            }
            if !Path::new(trimmed).exists() {
                return Err(format!("Chave privada não encontrada: {trimmed}"));
            }
            cmd.arg("-o")
                .arg("BatchMode=yes")
                .arg("-o")
                .arg("IdentitiesOnly=yes")
                .arg("-i")
                .arg(trimmed);
            if server.port != 0 {
                cmd.arg("-p").arg(server.port.to_string());
            }
            cmd.arg(ssh_target(server));
        }
        AuthMethod::PasswordInVault => {
            let key = vault::vps_password_key(&server.id);
            let pass = vault::load_secret(&key)?
                .filter(|p| !p.is_empty())
                .ok_or_else(|| {
                    "Nenhuma senha SSH salva no Cofre do Windows para esta VPS.".to_string()
                })?;
            let guard = create_askpass_guard()?;
            cmd.arg("-o")
                .arg("NumberOfPasswordPrompts=1")
                .arg("-o")
                .arg("PreferredAuthentications=publickey,password,keyboard-interactive");
            if server.port != 0 {
                cmd.arg("-p").arg(server.port.to_string());
            }
            cmd.arg(ssh_target(server));
            cmd.env("SSH_ASKPASS", &guard.path);
            cmd.env("SSH_ASKPASS_REQUIRE", "force");
            cmd.env("DISPLAY", "workploy:0");
            cmd.env("WORKPLOY_SSH_PASS", pass);
            askpass_guard = Some(guard);
        }
    }

    cmd.arg(remote_cmd);
    Ok((cmd, askpass_guard))
}

pub fn ssh_run_capture(server: &VpsServer, remote_cmd: &str) -> Result<(bool, String, String), String> {
    let (mut cmd, _guard) = build_ssh_command(server, remote_cmd)?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let out = cmd
        .output()
        .map_err(|e| format!("Não foi possível iniciar o cliente ssh: {e}"))?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}

pub fn ssh_run_with_stdin(
    server: &VpsServer,
    remote_cmd: &str,
    stdin_bytes: &[u8],
) -> Result<(bool, String, String), String> {
    let (mut cmd, _guard) = build_ssh_command(server, remote_cmd)?;
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Não foi possível iniciar o cliente ssh: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(stdin_bytes)
            .map_err(|e| format!("Falha ao enviar dados via SSH: {e}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("Falha ao aguardar retorno do SSH: {e}"))?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}

pub fn ssh_run_streaming<F>(
    server: &VpsServer,
    remote_cmd: &str,
    cancel: Option<&Arc<AtomicBool>>,
    mut on_line: F,
) -> Result<bool, String>
where
    F: FnMut(&str) + Send,
{
    let (mut cmd, _guard) = build_ssh_command(server, remote_cmd)?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Não foi possível iniciar o comando SSH: {e}"))?;

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        let tx2 = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx2.send(line);
            }
        }));
    }
    if let Some(err) = child.stderr.take() {
        let tx2 = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                let _ = tx2.send(line);
            }
        }));
    }
    drop(tx);

    let mut cancelled = false;
    let status = loop {
        while let Ok(line) = rx.try_recv() {
            on_line(&line);
        }
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {}
            Err(_) => break None,
        }
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            cancelled = true;
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(80));
    };

    for h in readers {
        let _ = h.join();
    }
    while let Ok(line) = rx.try_recv() {
        on_line(&line);
    }

    if cancelled {
        return Ok(false);
    }
    Ok(status.is_some_and(|s| s.success()))
}

pub fn compute_capacity_tier(
    vcpus: u32,
    ram_total_mb: u64,
    disk_free_gb: u64,
) -> (CapacityTier, String) {
    if vcpus >= 4 && ram_total_mb >= 7500 && disk_free_gb >= 60 {
        (
            CapacityTier::Heavy,
            "Capacidade Pesada — excelente para builds paralelos Rust/Android e múltiplos containers.".into(),
        )
    } else if vcpus >= 2 && ram_total_mb >= 3500 && disk_free_gb >= 30 {
        (
            CapacityTier::Standard,
            "Capacidade Padrão — recomendada para monorepos Node/Docker e CI contínuo.".into(),
        )
    } else if vcpus >= 1 && ram_total_mb >= 1500 && disk_free_gb >= 12 {
        (
            CapacityTier::Light,
            "Capacidade Leve — ideal para APIs leves, webhooks e deploys rápidos.".into(),
        )
    } else {
        (
            CapacityTier::Insufficient,
            "Recursos insuficientes para builds estáveis — recomendado pelo menos 1-2 vCPU, 2 GB RAM e 25 GB SSD.".into(),
        )
    }
}

pub fn parse_probe_output(raw: &str) -> VpsProbeResult {
    let mut res = VpsProbeResult::default();
    for line in raw.lines() {
        let trimmed = line.trim();
        let Some((k, v)) = trimmed.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let val = v.trim();
        match key {
            "OS" => res.os = val.to_string(),
            "ARCH" => res.arch = val.to_string(),
            "VCPUS" => res.vcpus = val.parse().unwrap_or(0),
            "RAM_TOTAL_MB" => res.ram_total_mb = val.parse().unwrap_or(0),
            "RAM_FREE_MB" => res.ram_free_mb = val.parse().unwrap_or(0),
            "DISK_FREE_GB" => res.disk_free_gb = val.parse().unwrap_or(0),
            "HAS_GIT" => res.has_git = val == "1" || val.eq_ignore_ascii_case("true"),
            "HAS_DOCKER" => res.has_docker = val == "1" || val.eq_ignore_ascii_case("true"),
            "HAS_NODE" => res.has_node = val == "1" || val.eq_ignore_ascii_case("true"),
            "HAS_PNPM" => res.has_pnpm = val == "1" || val.eq_ignore_ascii_case("true"),
            "HAS_RUST" => res.has_rust = val == "1" || val.eq_ignore_ascii_case("true"),
            "HAS_SYSTEMD" => res.has_systemd = val == "1" || val.eq_ignore_ascii_case("true"),
            "HAS_RUNNER" => res.has_widder_runner = val == "1" || val.eq_ignore_ascii_case("true"),
            "RUNNER_ACTIVE" => res.runner_active = val == "1" || val.eq_ignore_ascii_case("true"),
            _ => {}
        }
    }

    let (tier, verdict) = compute_capacity_tier(res.vcpus, res.ram_total_mb, res.disk_free_gb);
    res.tier = tier;
    res.verdict = verdict;
    res.checked_at = now_str();
    res.ok = !res.os.is_empty() && res.vcpus > 0;
    res
}

const PROBE_REMOTE_SCRIPT: &str = r#"sh -c '
OS=$(. /etc/os-release 2>/dev/null && printf "%s" "$PRETTY_NAME" || uname -s)
ARCH=$(uname -m 2>/dev/null || echo unknown)
VCPUS=$(nproc 2>/dev/null || grep -c ^processor /proc/cpuinfo 2>/dev/null || echo 1)
MEM_TOT=$(awk "/^MemTotal:/{printf \"%d\", \$2/1024}" /proc/meminfo 2>/dev/null || echo 0)
MEM_AVAIL=$(awk "/^MemAvailable:/{printf \"%d\", \$2/1024}" /proc/meminfo 2>/dev/null || echo 0)
DISK_FREE=$(df -BG --output=avail / 2>/dev/null | tail -n 1 | tr -dc "0-9")
[ -z "$DISK_FREE" ] && DISK_FREE=$(df -k / 2>/dev/null | awk "NR==2{printf \"%d\", \$4/1048576}")
has_cmd() { command -v "$1" >/dev/null 2>&1 && echo 1 || echo 0; }
HAS_GIT=$(has_cmd git)
HAS_DOCKER=$(has_cmd docker)
HAS_NODE=$(has_cmd node)
HAS_PNPM=$(has_cmd pnpm)
HAS_RUST=$(has_cmd cargo)
HAS_SYSTEMD=$([ -d /run/systemd/system ] && command -v systemctl >/dev/null 2>&1 && echo 1 || echo 0)
HAS_RUNNER=$([ -f /opt/workploy/workploy_runner.py ] && echo 1 || echo 0)
RUNNER_ACT=$(systemctl is-active --quiet workploy-runner 2>/dev/null && echo 1 || echo 0)
printf "OS=%s\nARCH=%s\nVCPUS=%s\nRAM_TOTAL_MB=%s\nRAM_FREE_MB=%s\nDISK_FREE_GB=%s\nHAS_GIT=%s\nHAS_DOCKER=%s\nHAS_NODE=%s\nHAS_PNPM=%s\nHAS_RUST=%s\nHAS_SYSTEMD=%s\nHAS_RUNNER=%s\nRUNNER_ACTIVE=%s\n" \
"$OS" "$ARCH" "$VCPUS" "$MEM_TOT" "$MEM_AVAIL" "${DISK_FREE:-0}" "$HAS_GIT" "$HAS_DOCKER" "$HAS_NODE" "$HAS_PNPM" "$HAS_RUST" "$HAS_SYSTEMD" "$HAS_RUNNER" "$RUNNER_ACT"
'"#;

pub fn probe_vps(sh: &SharedHandle, server_id: &str) {
    let server = {
        let mut s = sh.lock().unwrap();
        if s.probing_server_id.is_some() {
            return;
        }
        let Some(srv) = s.settings.servers.iter().find(|v| v.id == server_id).cloned() else {
            return;
        };
        s.probing_server_id = Some(server_id.to_string());
        s.ssh_testing = true;
        s.ssh_test = None;
        srv
    };
    engine::notify(sh);

    let sh2 = sh.clone();
    let sid = server_id.to_string();
    std::thread::spawn(move || {
        let started = Instant::now();
        let probe = match ssh_run_capture(&server, PROBE_REMOTE_SCRIPT) {
            Ok((true, stdout, _)) => {
                let parsed = parse_probe_output(&stdout);
                if parsed.ok {
                    parsed
                } else {
                    VpsProbeResult {
                        ok: false,
                        verdict: "Conexão SSH aberta, mas a VPS não retornou métricas válidas.".into(),
                        checked_at: now_str(),
                        ..Default::default()
                    }
                }
            }
            Ok((false, _, stderr)) => {
                let err = stderr.trim();
                let msg = if err.is_empty() {
                    "Falha ao executar diagnóstico na VPS.".into()
                } else {
                    format!("Falha SSH: {err}")
                };
                VpsProbeResult {
                    ok: false,
                    verdict: msg,
                    checked_at: now_str(),
                    ..Default::default()
                }
            }
            Err(e) => VpsProbeResult {
                ok: false,
                verdict: e,
                checked_at: now_str(),
                ..Default::default()
            },
        };

        let ms = started.elapsed().as_millis();
        let summary_msg = if probe.ok {
            format!(
                "{} · {} vCPU · {:.1} GB RAM · {} GB livres ({} ms)",
                probe.tier.label(),
                probe.vcpus,
                probe.ram_total_mb as f64 / 1024.0,
                probe.disk_free_gb,
                ms
            )
        } else {
            probe.verdict.clone()
        };

        {
            let mut s = sh2.lock().unwrap();
            if let Some(srv) = s.settings.servers.iter_mut().find(|v| v.id == sid) {
                if probe.ok {
                    srv.runner_installed = probe.has_widder_runner;
                }
                srv.last_probe = Some(probe.clone());
            }
            s.probing_server_id = None;
            s.ssh_testing = false;
            s.ssh_test = Some((probe.ok, summary_msg));
            save_settings(&s);
        }
        engine::notify(&sh2);
    });
}

fn append_provision_log(sh: &SharedHandle, line: &str) {
    {
        let mut s = sh.lock().unwrap();
        s.provision_log.push_str(line);
        if !line.ends_with('\n') {
            s.provision_log.push('\n');
        }
        const CAP: usize = 200_000;
        if s.provision_log.len() > CAP {
            let cut = s.provision_log.len() - CAP;
            s.provision_log.drain(..cut);
        }
    }
    engine::notify(sh);
}

fn generate_random_hex(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let mixed = nanos ^ (pid.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    format!("{prefix}-{nanos:x}-{mixed:016x}")
}

pub const WORKPLOY_RUNNER_PY: &str = r#"#!/usr/bin/env python3
import argparse
import hashlib
import hmac
import http.server
import json
import os
import re
import shutil
import socketserver
import subprocess
import threading
import time
import urllib.error
import urllib.request
from datetime import datetime

CONFIG_PATH = "/etc/workploy/config.json"
TOKEN_PATH = "/etc/workploy/token"
WEBHOOK_SECRET_PATH = "/etc/workploy/webhook_secret"
GITHUB_TOKEN_PATH = "/etc/workploy/github_token"
RUNS_PATH = "/var/lib/workploy/runs.json"
POLL_STATE_PATH = "/var/lib/workploy/poll_state.json"
PURGE_STATE_PATH = "/var/lib/workploy/purge_state.json"
WORKSPACES_DIR = "/var/lib/workploy/workspaces"
BUILD_CACHE_DIRS = ("target", "node_modules", ".next", ".turbo", ".parcel-cache", ".cache", "dist", "build")

STATE_LOCK = threading.Lock()
PURGE_LOCK = threading.Lock()
RUNS = []
PROCESSES = {}
ACTIVE_WORKDIRS = {}


def now_str():
    return datetime.now().strftime("%d/%m/%Y %H:%M:%S")


def stamp_list(ts):
    return datetime.fromtimestamp(ts).strftime("%d/%m/%Y %H:%M")


def read_text(path, default=""):
    try:
        with open(path, "r", encoding="utf-8") as f:
            return f.read().strip()
    except OSError:
        return default


def load_config():
    raw = read_text(CONFIG_PATH, "{}")
    try:
        data = json.loads(raw)
        if isinstance(data, dict):
            return data
    except json.JSONDecodeError:
        pass
    return {"bindings": [], "workflows": [], "poll_secs": 60}


def load_runs():
    global RUNS
    raw = read_text(RUNS_PATH, "[]")
    try:
        data = json.loads(raw)
        if isinstance(data, list):
            for r in data:
                if r.get("status") in ("Queued", "Running", "AwaitingApproval"):
                    r["status"] = "Cancelled"
                    r["finished_at"] = now_str()
                    for st in r.get("steps", []):
                        if st.get("status") in ("Pending", "Running", "WaitingApproval"):
                            st["status"] = "Skipped"
            RUNS = data[:100]
            save_runs_locked()
    except json.JSONDecodeError:
        RUNS = []


def save_runs_locked():
    os.makedirs(os.path.dirname(RUNS_PATH), exist_ok=True)
    tmp = RUNS_PATH + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(RUNS[:100], f, ensure_ascii=False, indent=2)
    os.replace(tmp, RUNS_PATH)


def find_run_locked(run_id):
    for r in RUNS:
        if r.get("id") == run_id:
            return r
    return None


def safe_slug(text):
    cleaned = re.sub(r"[^a-zA-Z0-9._-]+", "-", text or "workspace").strip("-")
    return cleaned or "workspace"


def dir_size_mb(path):
    if not path or not os.path.exists(path):
        return 0.0
    total = 0
    for root, _dirs, files in os.walk(path, followlinks=False):
        for name in files:
            fp = os.path.join(root, name)
            try:
                if not os.path.islink(fp):
                    total += os.path.getsize(fp)
            except OSError:
                pass
    return total / (1024.0 * 1024.0)


def load_purge_state():
    raw = read_text(PURGE_STATE_PATH, "{}")
    try:
        data = json.loads(raw)
        if isinstance(data, dict):
            return {
                "runs_since_purge": int(data.get("runs_since_purge", 0) or 0),
                "last_purge_ts": int(data.get("last_purge_ts", 0) or 0),
                "last_freed_mb": float(data.get("last_freed_mb", 0.0) or 0.0),
            }
    except Exception:
        pass
    return {"runs_since_purge": 0, "last_purge_ts": 0, "last_freed_mb": 0.0}


def save_purge_state(st):
    os.makedirs(os.path.dirname(PURGE_STATE_PATH), exist_ok=True)
    tmp = PURGE_STATE_PATH + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(st, f, ensure_ascii=False, indent=2)
    os.replace(tmp, PURGE_STATE_PATH)


def _parse_docker_reclaimed_mb(output):
    m = re.search(r"Total reclaimed space:\s*([0-9]+(?:\.[0-9]+)?)\s*([kMGT]?B)", output or "", re.IGNORECASE)
    if not m:
        return 0.0
    val = float(m.group(1))
    unit = m.group(2).upper()
    if unit == "B":
        return val / (1024.0 * 1024.0)
    if unit == "KB":
        return val / 1024.0
    if unit == "MB":
        return val
    if unit == "GB":
        return val * 1024.0
    if unit == "TB":
        return val * 1024.0 * 1024.0
    return 0.0


def run_purge(cfg):
    rule = (cfg or {}).get("purge_rule") or {}
    clean_ws = bool(rule.get("clean_workspaces", True))
    clean_docker = bool(rule.get("clean_docker", True))
    freed_mb = 0.0
    notes = []

    if clean_ws and os.path.isdir(WORKSPACES_DIR):
        before_mb = dir_size_mb(WORKSPACES_DIR)
        with STATE_LOCK:
            active_paths = {os.path.realpath(p) for p in ACTIVE_WORKDIRS.values() if p}
        configured_paths = set()
        for b in (cfg or {}).get("bindings", []):
            if b.get("enabled") and b.get("full_name"):
                configured_paths.add(os.path.realpath(os.path.join(WORKSPACES_DIR, safe_slug(b.get("full_name")))))

        removed_dirs = 0
        cleaned_caches = 0
        for entry in os.listdir(WORKSPACES_DIR):
            full = os.path.realpath(os.path.join(WORKSPACES_DIR, entry))
            if full in active_paths:
                continue
            if full not in configured_paths:
                shutil.rmtree(full, ignore_errors=True)
                removed_dirs += 1
            elif os.path.isdir(full):
                for root, dirs, _files in os.walk(full, topdown=True):
                    if ".git" in dirs:
                        dirs.remove(".git")
                    for dname in list(dirs):
                        if dname in BUILD_CACHE_DIRS:
                            shutil.rmtree(os.path.join(root, dname), ignore_errors=True)
                            dirs.remove(dname)
                            cleaned_caches += 1
        after_mb = dir_size_mb(WORKSPACES_DIR)
        ws_freed = max(0.0, before_mb - after_mb)
        freed_mb += ws_freed
        notes.append(f"workspaces: {removed_dirs} removidos, {cleaned_caches} caches limpos ({ws_freed:.1f} MB)")

    if clean_docker and shutil.which("docker"):
        docker_freed = 0.0
        try:
            res = subprocess.run(
                ["docker", "system", "prune", "-af", "--filter", "until=24h"],
                capture_output=True,
                text=True,
                timeout=120,
            )
            docker_freed += _parse_docker_reclaimed_mb(res.stdout)
            res2 = subprocess.run(
                ["docker", "builder", "prune", "-f", "--filter", "until=24h"],
                capture_output=True,
                text=True,
                timeout=120,
            )
            if res2.returncode != 0:
                res2 = subprocess.run(
                    ["docker", "builder", "prune", "-f"],
                    capture_output=True,
                    text=True,
                    timeout=120,
                )
            docker_freed += _parse_docker_reclaimed_mb(res2.stdout)
            freed_mb += docker_freed
            notes.append(f"docker: {docker_freed:.1f} MB liberados")
        except Exception as exc:
            notes.append(f"docker: erro ({exc})")

    msg = "; ".join(notes) if notes else "nada configurado para limpar"
    return {"ok": True, "freed_mb": round(freed_mb, 2), "message": msg}


def record_run_and_maybe_purge():
    cfg = load_config()
    rule = cfg.get("purge_rule") or {}
    with PURGE_LOCK:
        st = load_purge_state()
        st["runs_since_purge"] = int(st.get("runs_since_purge", 0)) + 1
        enabled = bool(rule.get("enabled", False))
        every_n = int(rule.get("every_n_runs", 10) or 0)
        max_mb = int(rule.get("max_size_mb", 5120) or 0)
        should_purge = False
        if enabled:
            if every_n > 0 and st["runs_since_purge"] >= every_n:
                should_purge = True
            elif max_mb > 0 and dir_size_mb(WORKSPACES_DIR) >= float(max_mb):
                should_purge = True
        if should_purge:
            res = run_purge(cfg)
            st["runs_since_purge"] = 0
            st["last_purge_ts"] = int(time.time())
            st["last_freed_mb"] = float(res.get("freed_mb", 0.0))
        save_purge_state(st)


def run_manual_purge():
    cfg = load_config()
    with PURGE_LOCK:
        res = run_purge(cfg)
        st = load_purge_state()
        st["runs_since_purge"] = 0
        st["last_purge_ts"] = int(time.time())
        st["last_freed_mb"] = float(res.get("freed_mb", 0.0))
        save_purge_state(st)
        return res


def prepare_workspace(repo_full_name, branch, custom_workdir):
    if custom_workdir and custom_workdir.strip():
        wd = custom_workdir.strip()
        os.makedirs(wd, exist_ok=True)
        return wd, ""
    os.makedirs(WORKSPACES_DIR, exist_ok=True)
    if not repo_full_name:
        return WORKSPACES_DIR, ""
    wd = os.path.join(WORKSPACES_DIR, safe_slug(repo_full_name))
    gh_tok = read_text(GITHUB_TOKEN_PATH, "")
    if gh_tok:
        remote_url = f"https://x-access-token:{gh_tok}@github.com/{repo_full_name}.git"
    else:
        remote_url = f"https://github.com/{repo_full_name}.git"
    logs = []
    try:
        if not os.path.isdir(os.path.join(wd, ".git")):
            os.makedirs(wd, exist_ok=True)
            res = subprocess.run(
                ["git", "clone", "--branch", branch or "main", remote_url, wd],
                capture_output=True,
                text=True,
                timeout=300,
            )
            logs.append(f"[git clone] exit={res.returncode}")
            if res.stdout:
                logs.append(res.stdout)
            if res.stderr:
                logs.append(res.stderr.replace(gh_tok, "***") if gh_tok else res.stderr)
        else:
            res = subprocess.run(
                ["git", "-C", wd, "fetch", "origin", branch or "main"],
                capture_output=True,
                text=True,
                timeout=180,
            )
            subprocess.run(["git", "-C", wd, "checkout", branch or "main"], capture_output=True, text=True)
            res2 = subprocess.run(
                ["git", "-C", wd, "pull", "--ff-only", "origin", branch or "main"],
                capture_output=True,
                text=True,
                timeout=180,
            )
            logs.append(f"[git sync] fetch={res.returncode} pull={res2.returncode}")
            if res2.stdout:
                logs.append(res2.stdout)
    except Exception as exc:
        logs.append(f"[git sync erro] {exc}")
    return wd, "\n".join(logs) + ("\n" if logs else "")


def append_step_log(run_id, step_idx, text):
    with STATE_LOCK:
        r = find_run_locked(run_id)
        if r and 0 <= step_idx < len(r["steps"]):
            cur = r["steps"][step_idx].get("log", "") + text
            if len(cur) > 120000:
                cur = cur[-120000:]
            r["steps"][step_idx]["log"] = cur
            save_runs_locked()


def execute_run(run_id, wf, repo_full_name="", branch="main"):
    t_start = time.monotonic()
    workdir, prep_log = prepare_workspace(repo_full_name, branch, wf.get("workdir", ""))
    with STATE_LOCK:
        ACTIVE_WORKDIRS[run_id] = workdir
    try:
        if prep_log and len(wf.get("steps", [])) > 0:
            append_step_log(run_id, 0, prep_log)

        steps = wf.get("steps", [])
        for idx, step in enumerate(steps):
            with STATE_LOCK:
                r = find_run_locked(run_id)
                if not r or r.get("cancel_requested"):
                    if r:
                        r["status"] = "Cancelled"
                        r["finished_at"] = now_str()
                        r["secs"] = time.monotonic() - t_start
                        for st in r["steps"][idx:]:
                            if st["status"] in ("Pending", "WaitingApproval", "Running"):
                                st["status"] = "Skipped"
                        save_runs_locked()
                    return

            if step.get("approval"):
                with STATE_LOCK:
                    r = find_run_locked(run_id)
                    if r:
                        r["status"] = "AwaitingApproval"
                        r["steps"][idx]["status"] = "WaitingApproval"
                        save_runs_locked()
                append_step_log(run_id, idx, "[portão de aprovação] aguardando aprovação do maestro...\n")
                while True:
                    time.sleep(0.4)
                    with STATE_LOCK:
                        r = find_run_locked(run_id)
                        if not r:
                            return
                        if r.get("cancel_requested"):
                            r["status"] = "Cancelled"
                            r["finished_at"] = now_str()
                            r["secs"] = time.monotonic() - t_start
                            for st in r["steps"][idx:]:
                                st["status"] = "Skipped"
                            save_runs_locked()
                            return
                        if r.get("approved"):
                            break
                append_step_log(run_id, idx, "[aprovado] continuando.\n")

            with STATE_LOCK:
                r = find_run_locked(run_id)
                if r:
                    r["status"] = "Running"
                    r["steps"][idx]["status"] = "Running"
                    r["steps"][idx]["started_ts"] = int(time.time())
                    save_runs_locked()

            cmd = step.get("run", "").strip()
            s_start = time.monotonic()
            ok = True
            cancelled = False
            if not cmd:
                append_step_log(run_id, idx, "[passo sem comando — nada a fazer]\n")
            else:
                shell_bin = "/bin/bash" if os.path.exists("/bin/bash") else "/bin/sh"
                proc = subprocess.Popen(
                    [shell_bin, "-lc", cmd],
                    cwd=workdir,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    text=True,
                    errors="replace",
                )
                with STATE_LOCK:
                    PROCESSES[run_id] = proc

                def pump_output():
                    if proc.stdout:
                        for line in proc.stdout:
                            append_step_log(run_id, idx, line)

                t_pump = threading.Thread(target=pump_output, daemon=True)
                t_pump.start()
                while proc.poll() is None:
                    with STATE_LOCK:
                        r = find_run_locked(run_id)
                        if r and r.get("cancel_requested"):
                            cancelled = True
                    if cancelled:
                        proc.terminate()
                        try:
                            proc.wait(timeout=3)
                        except subprocess.TimeoutExpired:
                            proc.kill()
                        break
                    time.sleep(0.2)
                t_pump.join(timeout=2)
                with STATE_LOCK:
                    PROCESSES.pop(run_id, None)
                ok = (not cancelled) and (proc.returncode == 0)
                if cancelled:
                    append_step_log(run_id, idx, "[cancelado pelo usuário]\n")
                elif not ok:
                    append_step_log(run_id, idx, "[passo falhou — o fluxo para aqui]\n")

            s_secs = time.monotonic() - s_start
            with STATE_LOCK:
                r = find_run_locked(run_id)
                if not r:
                    return
                r["steps"][idx]["secs"] = s_secs
                if cancelled:
                    r["status"] = "Cancelled"
                    r["finished_at"] = now_str()
                    r["secs"] = time.monotonic() - t_start
                    for st in r["steps"][idx:]:
                        if st["status"] in ("Pending", "WaitingApproval", "Running"):
                            st["status"] = "Skipped"
                    save_runs_locked()
                    return
                r["steps"][idx]["status"] = "Success" if ok else "Failed"
                if not ok:
                    r["status"] = "Failed"
                    r["finished_at"] = now_str()
                    r["secs"] = time.monotonic() - t_start
                    for st in r["steps"][idx + 1:]:
                        if st["status"] == "Pending":
                            st["status"] = "Skipped"
                    save_runs_locked()
                    return
                save_runs_locked()

        with STATE_LOCK:
            r = find_run_locked(run_id)
            if r:
                r["status"] = "Success"
                r["finished_at"] = now_str()
                r["secs"] = time.monotonic() - t_start
                save_runs_locked()

        next_id = wf.get("on_success")
        if next_id and next_id != wf.get("id"):
            cfg = load_config()
            for cand in cfg.get("workflows", []):
                if cand.get("id") == next_id:
                    queue_workflow(cand, "encadeado: fluxo anterior concluído", repo_full_name, branch, None, "workploy-runner")
                    break
    finally:
        with STATE_LOCK:
            ACTIVE_WORKDIRS.pop(run_id, None)
        try:
            record_run_and_maybe_purge()
        except Exception:
            pass


def queue_workflow(wf, trigger_label, repo_full_name="", branch="main", commit_info=None, actor="workploy-runner"):
    now_ts = int(time.time())
    commit_sha = None
    title = trigger_label
    if commit_info:
        commit_sha, title = commit_info
    with STATE_LOCK:
        wf_id = wf.get("id", "")
        number = sum(1 for r in RUNS if r.get("workflow_id") == wf_id) + 1
        run_id = f"run-vps-{now_ts:x}-{number}"
        run_obj = {
            "id": run_id,
            "workflow_id": wf_id,
            "workflow_name": wf.get("name", "Fluxo VPS"),
            "status": "Queued",
            "trigger_label": trigger_label,
            "title": title,
            "number": number,
            "commit": commit_sha,
            "started_at": stamp_list(now_ts),
            "started_ts": now_ts,
            "finished_at": None,
            "steps": [
                {
                    "name": s.get("name", "Passo"),
                    "status": "Pending",
                    "log": "",
                    "secs": 0.0,
                    "substeps": [],
                    "started_ts": None,
                }
                for s in wf.get("steps", [])
            ],
            "approved": False,
            "cancel_requested": False,
            "secs": 0.0,
            "actor": actor,
            "branch": branch or "main",
            "summary": "",
            "annotations": [],
        }
        RUNS.insert(0, run_obj)
        save_runs_locked()
    threading.Thread(
        target=execute_run,
        args=(run_id, wf, repo_full_name, branch or "main"),
        daemon=True,
    ).start()
    return run_id


def poller_loop():
    poll_state = {}
    try:
        poll_state = json.loads(read_text(POLL_STATE_PATH, "{}"))
    except Exception:
        poll_state = {}

    while True:
        cfg = load_config()
        every = max(15, int(cfg.get("poll_secs", 60) or 60))
        gh_tok = read_text(GITHUB_TOKEN_PATH, "")
        bindings = cfg.get("bindings", [])
        workflows = {w.get("id"): w for w in cfg.get("workflows", []) if w.get("id")}

        for b in bindings:
            if not b.get("enabled") or b.get("trigger_mode") != "PollOnVps":
                continue
            repo = (b.get("full_name") or "").strip()
            branch = (b.get("branch") or "main").strip()
            wf_id = b.get("workflow_id")
            if not repo or not wf_id or wf_id not in workflows:
                continue
            url = f"https://api.github.com/repos/{repo}/commits/{branch}"
            headers = {
                "User-Agent": "workploy-runner",
                "Accept": "application/vnd.github+json",
            }
            if gh_tok:
                headers["Authorization"] = f"Bearer {gh_tok}"
            req = urllib.request.Request(url, headers=headers)
            try:
                with urllib.request.urlopen(req, timeout=20) as resp:
                    payload = json.loads(resp.read().decode("utf-8"))
                sha = payload.get("sha", "")
                msg = ((payload.get("commit") or {}).get("message") or "commit").splitlines()[0]
                key = f"{repo}:{branch}"
                prev = poll_state.get(key)
                if sha and prev and prev != sha:
                    poll_state[key] = sha
                    queue_workflow(workflows[wf_id], "sondagem 24/7 na VPS", repo, branch, (sha, msg), "github-poll")
                elif sha and not prev:
                    poll_state[key] = sha
                with open(POLL_STATE_PATH, "w", encoding="utf-8") as f:
                    json.dump(poll_state, f)
            except Exception:
                pass
        time.sleep(every)


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        return

    def send_json(self, code, data):
        body = json.dumps(data, ensure_ascii=False).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def check_bearer(self):
        expected = read_text(TOKEN_PATH, "")
        auth = self.headers.get("Authorization", "")
        if not expected or not auth.startswith("Bearer "):
            return False
        got = auth[7:].strip()
        return hmac.compare_digest(got, expected)

    def read_body(self):
        n = int(self.headers.get("Content-Length", "0") or 0)
        if n <= 0 or n > 4_194_304:
            return b""
        return self.rfile.read(n)

    def do_GET(self):
        if not self.check_bearer():
            return self.send_json(401, {"error": "unauthorized"})
        if self.path == "/status":
            with STATE_LOCK:
                active = sum(1 for r in RUNS if r.get("status") in ("Queued", "Running", "AwaitingApproval"))
                total = len(RUNS)
            cfg = load_config()
            return self.send_json(200, {
                "ok": True,
                "service": "workploy-runner",
                "active_runs": active,
                "total_runs": total,
                "bindings": len(cfg.get("bindings", [])),
            })
        if self.path == "/runs":
            with STATE_LOCK:
                return self.send_json(200, RUNS[:50])
        self.send_json(404, {"error": "not_found"})

    def do_POST(self):
        raw = self.read_body()
        if self.path == "/webhook/github":
            secret = read_text(WEBHOOK_SECRET_PATH, "")
            sig = self.headers.get("X-Hub-Signature-256", "")
            if secret:
                expected_sig = "sha256=" + hmac.new(secret.encode("utf-8"), raw, hashlib.sha256).hexdigest()
                if not hmac.compare_digest(sig, expected_sig):
                    return self.send_json(401, {"error": "invalid_signature"})
            ev = self.headers.get("X-GitHub-Event", "push")
            if ev == "ping":
                return self.send_json(200, {"ok": True, "pong": True})
            try:
                payload = json.loads(raw.decode("utf-8") or "{}")
            except Exception:
                return self.send_json(400, {"error": "invalid_json"})
            repo = ((payload.get("repository") or {}).get("full_name") or "").strip().lower()
            ref = payload.get("ref", "")
            branch = ref.replace("refs/heads/", "", 1) if ref.startswith("refs/heads/") else ref
            sha = payload.get("after", "")
            msg = ((payload.get("head_commit") or {}).get("message") or "webhook push").splitlines()[0]
            actor = ((payload.get("pusher") or {}).get("name") or "github").strip()
            cfg = load_config()
            workflows = {w.get("id"): w for w in cfg.get("workflows", []) if w.get("id")}
            triggered = []
            for b in cfg.get("bindings", []):
                if not b.get("enabled") or b.get("trigger_mode") != "WebhookOnVps":
                    continue
                if (b.get("full_name") or "").strip().lower() != repo:
                    continue
                if (b.get("branch") or "main").strip() != branch:
                    continue
                wf_id = b.get("workflow_id")
                if wf_id and wf_id in workflows:
                    rid = queue_workflow(workflows[wf_id], "webhook no GitHub", repo, branch, (sha, msg), actor)
                    triggered.append(rid)
            return self.send_json(200, {"ok": True, "triggered": triggered})

        if not self.check_bearer():
            return self.send_json(401, {"error": "unauthorized"})

        if self.path == "/reload":
            cfg = load_config()
            return self.send_json(200, {"ok": True, "bindings": len(cfg.get("bindings", []))})

        if self.path == "/purge":
            res = run_manual_purge()
            return self.send_json(200, res)

        if self.path == "/trigger":
            try:
                body = json.loads(raw.decode("utf-8") or "{}")
            except Exception:
                return self.send_json(400, {"error": "invalid_json"})
            wf_id = body.get("workflow_id", "")
            cfg = load_config()
            for w in cfg.get("workflows", []):
                if w.get("id") == wf_id:
                    rid = queue_workflow(
                        w,
                        body.get("trigger_label", "manual"),
                        body.get("repo", ""),
                        body.get("branch", "main"),
                        None,
                        "maestro",
                    )
                    return self.send_json(200, {"ok": True, "run_id": rid})
            return self.send_json(404, {"error": "workflow_not_found"})

        m_app = re.fullmatch(r"/runs/([^/]+)/approve", self.path)
        if m_app:
            rid = m_app.group(1)
            with STATE_LOCK:
                r = find_run_locked(rid)
                if not r:
                    return self.send_json(404, {"error": "run_not_found"})
                r["approved"] = True
                save_runs_locked()
            return self.send_json(200, {"ok": True})

        m_can = re.fullmatch(r"/runs/([^/]+)/cancel", self.path)
        if m_can:
            rid = m_can.group(1)
            with STATE_LOCK:
                r = find_run_locked(rid)
                if not r:
                    return self.send_json(404, {"error": "run_not_found"})
                r["cancel_requested"] = True
                save_runs_locked()
            return self.send_json(200, {"ok": True})

        self.send_json(404, {"error": "not_found"})


class ThreadingHTTPServer(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=8787)
    args = parser.parse_args()
    load_runs()
    threading.Thread(target=poller_loop, daemon=True).start()
    srv = ThreadingHTTPServer(("0.0.0.0", args.port), Handler)
    srv.serve_forever()


if __name__ == "__main__":
    main()
"#;

pub fn build_provision_script(
    opts: &ProvisionOptions,
    runner_token: &str,
    webhook_secret: &str,
) -> String {
    let port = if opts.runner_port == 0 { 8787 } else { opts.runner_port };
    let mut s = String::from(
        "set -e\n\
         if [ \"$(id -u)\" -eq 0 ]; then SUDO=''; \
         elif command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null; then SUDO='sudo -n'; \
         else echo '[erro] Esta conta não tem privilégios root nem sudo sem senha.' >&2; exit 1; fi\n\
         echo '=== [1/5] Verificando sistema e pacotes base ==='\n\
         if command -v apt-get >/dev/null 2>&1; then\n\
           export DEBIAN_FRONTEND=noninteractive\n\
           $SUDO apt-get update -y\n\
           $SUDO apt-get install -y ca-certificates curl python3\n\
         fi\n",
    );

    if opts.install_git {
        s.push_str(
            "echo '=== [2/5] Conferindo Git ==='\n\
             if ! command -v git >/dev/null 2>&1 && command -v apt-get >/dev/null 2>&1; then\n\
               $SUDO apt-get install -y git\n\
             fi\n\
             git --version || true\n",
        );
    }

    if opts.install_docker {
        s.push_str(
            "echo '=== [3/5] Conferindo Docker ==='\n\
             if ! command -v docker >/dev/null 2>&1; then\n\
               if command -v apt-get >/dev/null 2>&1; then\n\
                 $SUDO apt-get install -y docker.io docker-compose-plugin || curl -fsSL https://get.docker.com | $SUDO sh\n\
               fi\n\
               $SUDO systemctl enable --now docker || true\n\
             fi\n\
             docker --version || true\n",
        );
    }

    if opts.install_node {
        s.push_str(
            "echo '=== [4/5] Conferindo Node.js e pnpm ==='\n\
             if ! command -v node >/dev/null 2>&1; then\n\
               if command -v apt-get >/dev/null 2>&1; then\n\
                 curl -fsSL https://deb.nodesource.com/setup_22.x | $SUDO -E bash - || true\n\
                 $SUDO apt-get install -y nodejs || true\n\
               fi\n\
             fi\n\
             if command -v corepack >/dev/null 2>&1 && ! command -v pnpm >/dev/null 2>&1; then\n\
               $SUDO corepack enable || true\n\
             fi\n\
             node -v || true\n",
        );
    }

    if opts.install_rust {
        s.push_str(
            "echo '=== [Extra] Conferindo Rust ==='\n\
             if ! command -v cargo >/dev/null 2>&1; then\n\
               curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y\n\
             fi\n",
        );
    }

    if opts.install_runner_service {
        s.push_str(&format!(
            "echo '=== [5/5] Instalando serviço workploy-runner (systemd) ==='\n\
             $SUDO mkdir -p /opt/workploy /etc/workploy /var/lib/workploy/workspaces /var/lib/workploy/runs\n\
             $SUDO sh -c \"printf '%s' '{runner_token}' > /etc/workploy/token && chmod 600 /etc/workploy/token\"\n\
             $SUDO sh -c \"printf '%s' '{webhook_secret}' > /etc/workploy/webhook_secret && chmod 600 /etc/workploy/webhook_secret\"\n\
             if [ ! -f /etc/workploy/config.json ]; then\n\
               $SUDO sh -c \"printf '{{\\\"bindings\\\":[],\\\"workflows\\\":[],\\\"poll_secs\\\":60}}' > /etc/workploy/config.json\"\n\
             fi\n\
             $SUDO tee /opt/workploy/workploy_runner.py >/dev/null <<'WORKPLOY_RUNNER_EOF'\n\
             {WORKPLOY_RUNNER_PY}\n\
             WORKPLOY_RUNNER_EOF\n\
             $SUDO chmod 755 /opt/workploy/workploy_runner.py\n\
             $SUDO tee /etc/systemd/system/workploy-runner.service >/dev/null <<'WORKPLOY_SERVICE_EOF'\n\
             [Unit]\n\
             Description=Workploy Runner Daemon\n\
             After=network-online.target\n\
             Wants=network-online.target\n\
             \n\
             [Service]\n\
             Type=simple\n\
             ExecStart=/usr/bin/python3 /opt/workploy/workploy_runner.py --port {port}\n\
             Restart=always\n\
             RestartSec=3\n\
             WorkingDirectory=/var/lib/workploy\n\
             \n\
             [Install]\n\
             WantedBy=multi-user.target\n\
             WORKPLOY_SERVICE_EOF\n\
             $SUDO systemctl daemon-reload\n\
             $SUDO systemctl enable --now workploy-runner\n\
             $SUDO systemctl restart workploy-runner\n\
             $SUDO systemctl is-active workploy-runner\n\
             echo '=== Provisionamento concluído! ==='\n"
        ));
    }
    s
}

pub fn provision_vps(sh: &SharedHandle, server_id: &str, opts: ProvisionOptions) {
    let server = {
        let mut s = sh.lock().unwrap();
        if s.provisioning_server_id.is_some() {
            return;
        }
        let Some(srv) = s.settings.servers.iter().find(|v| v.id == server_id).cloned() else {
            return;
        };
        s.provisioning_server_id = Some(server_id.to_string());
        s.provision_log = format!("Iniciando provisionamento em '{}' ({})...\n", srv.name, srv.host);
        srv
    };
    engine::notify(sh);

    let sh2 = sh.clone();
    let sid = server_id.to_string();
    std::thread::spawn(move || {
        let token_key = vault::vps_runner_token_key(&sid);
        let webhook_key = vault::vps_webhook_secret_key(&sid);

        let runner_token = match vault::load_secret(&token_key) {
            Ok(Some(t)) if !t.is_empty() => t,
            _ => {
                let t = generate_random_hex("wtok");
                let _ = vault::save_secret(&token_key, &t);
                t
            }
        };
        let webhook_secret = match vault::load_secret(&webhook_key) {
            Ok(Some(sec)) if !sec.is_empty() => sec,
            _ => {
                let sec = generate_random_hex("whsec");
                let _ = vault::save_secret(&webhook_key, &sec);
                sec
            }
        };

        let script = build_provision_script(&opts, &runner_token, &webhook_secret);
        let sh_log = sh2.clone();
        let ok = match ssh_run_with_stdin_streaming(&server, "sh -s", script.as_bytes(), |line| {
            append_provision_log(&sh_log, line);
        }) {
            Ok(success) => success,
            Err(e) => {
                append_provision_log(&sh2, &format!("[erro] {e}"));
                false
            }
        };

        {
            let mut s = sh2.lock().unwrap();
            if let Some(srv) = s.settings.servers.iter_mut().find(|v| v.id == sid)
                && opts.install_runner_service
                && ok
            {
                srv.runner_installed = true;
                srv.runner_port = if opts.runner_port == 0 { 8787 } else { opts.runner_port };
                srv.webhook_secret_key = Some(webhook_key);
            }
            s.provisioning_server_id = None;
            s.runner_sync_status = if ok {
                "VPS provisionada e workploy-runner ativo.".into()
            } else {
                "Provisionamento falhou — consulte o log.".into()
            };
            save_settings(&s);
        }
        engine::notify(&sh2);
        if ok {
            probe_vps(&sh2, &sid);
        }
    });
}

fn ssh_run_with_stdin_streaming<F>(
    server: &VpsServer,
    remote_cmd: &str,
    stdin_bytes: &[u8],
    mut on_line: F,
) -> Result<bool, String>
where
    F: FnMut(&str) + Send,
{
    let (mut cmd, _guard) = build_ssh_command(server, remote_cmd)?;
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Não foi possível iniciar o comando SSH: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(stdin_bytes)
            .map_err(|e| format!("Falha ao enviar script de provisionamento: {e}"))?;
    }

    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        let tx2 = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                let _ = tx2.send(line);
            }
        }));
    }
    if let Some(err) = child.stderr.take() {
        let tx2 = tx.clone();
        readers.push(std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                let _ = tx2.send(line);
            }
        }));
    }
    drop(tx);

    let status = loop {
        while let Ok(line) = rx.try_recv() {
            on_line(&line);
        }
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => std::thread::sleep(Duration::from_millis(80)),
            Err(_) => break None,
        }
    };
    for h in readers {
        let _ = h.join();
    }
    while let Ok(line) = rx.try_recv() {
        on_line(&line);
    }

    Ok(status.is_some_and(|s| s.success()))
}

pub fn merge_remote_runs(existing: &mut Vec<Run>, remote: Vec<Run>) {
    for r in remote {
        if let Some(slot) = existing.iter_mut().find(|e| e.id == r.id) {
            *slot = r;
        } else {
            existing.push(r);
        }
    }
    existing.sort_by(|a, b| {
        b.started_ts
            .unwrap_or(0)
            .cmp(&a.started_ts.unwrap_or(0))
            .then(b.number.cmp(&a.number))
    });
}

pub fn sync_vps_runs(sh: &SharedHandle, server_id: &str) {
    let server = {
        let s = sh.lock().unwrap();
        let Some(srv) = s.settings.servers.iter().find(|v| v.id == server_id).cloned() else {
            return;
        };
        srv
    };

    let sh2 = sh.clone();
    std::thread::spawn(move || {
        let port = if server.runner_port == 0 {
            8787
        } else {
            server.runner_port
        };
        let cmd = format!(
            "sh -c 'TOKEN=$(cat /etc/workploy/token 2>/dev/null) && curl -fsS -H \"Authorization: Bearer $TOKEN\" http://127.0.0.1:{port}/runs'"
        );
        let msg = match ssh_run_capture(&server, &cmd) {
            Ok((true, stdout, _)) => match serde_json::from_str::<Vec<Run>>(&stdout) {
                Ok(remote_runs) => {
                    let count = remote_runs.len();
                    let mut s = sh2.lock().unwrap();
                    merge_remote_runs(&mut s.runs, remote_runs);
                    save_runs(&s);
                    format!("Sincronizadas {count} execuções de '{}'.", server.name)
                }
                Err(e) => format!("Resposta inválida do runner em '{}': {e}", server.name),
            },
            Ok((false, _, stderr)) => {
                format!(
                    "Não foi possível consultar execuções de '{}': {}",
                    server.name,
                    stderr.trim()
                )
            }
            Err(e) => e,
        };
        {
            let mut s = sh2.lock().unwrap();
            s.runner_sync_status = msg;
        }
        engine::notify(&sh2);
    });
}

pub fn remote_approve_or_cancel(sh: &SharedHandle, server_id: &str, run_id: &str, approve: bool) {
    let server = {
        let s = sh.lock().unwrap();
        let Some(srv) = s.settings.servers.iter().find(|v| v.id == server_id).cloned() else {
            return;
        };
        srv
    };

    let sh2 = sh.clone();
    let rid = run_id.to_string();
    let sid = server_id.to_string();
    std::thread::spawn(move || {
        let port = if server.runner_port == 0 {
            8787
        } else {
            server.runner_port
        };
        let action = if approve { "approve" } else { "cancel" };
        let safe_rid: String = rid
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        let cmd = format!(
            "sh -c 'TOKEN=$(cat /etc/workploy/token 2>/dev/null) && curl -fsS -X POST -H \"Authorization: Bearer $TOKEN\" http://127.0.0.1:{port}/runs/{safe_rid}/{action}'"
        );
        let status_msg = match ssh_run_capture(&server, &cmd) {
            Ok((true, _, _)) => {
                let mut s = sh2.lock().unwrap();
                if let Some(r) = s.runs.iter_mut().find(|r| r.id == rid) {
                    if approve {
                        r.approved = true;
                    } else {
                        r.cancel_requested = true;
                    }
                }
                save_runs(&s);
                if approve {
                    "Aprovação enviada ao runner da VPS.".into()
                } else {
                    "Cancelamento enviado ao runner da VPS.".into()
                }
            }
            Ok((false, _, stderr)) => {
                format!("Falha ao enviar comando para a VPS: {}", stderr.trim())
            }
            Err(e) => e,
        };
        {
            let mut s = sh2.lock().unwrap();
            s.runner_sync_status = status_msg;
        }
        engine::notify(&sh2);
        sync_vps_runs(&sh2, &sid);
    });
}

pub fn trigger_vps_purge(sh: &SharedHandle, server_id: &str) {
    let server = {
        let mut s = sh.lock().unwrap();
        let Some(srv) = s.settings.servers.iter().find(|v| v.id == server_id).cloned() else {
            return;
        };
        s.runner_sync_status = format!("Executando limpeza na VPS '{}'...", srv.name);
        srv
    };
    engine::notify(sh);

    let sh2 = sh.clone();
    std::thread::spawn(move || {
        let port = if server.runner_port == 0 {
            8787
        } else {
            server.runner_port
        };
        let cmd = format!(
            "sh -c 'TOKEN=$(cat /etc/workploy/token 2>/dev/null) && curl -fsS -X POST -H \"Authorization: Bearer $TOKEN\" http://127.0.0.1:{port}/purge'"
        );
        let msg = match ssh_run_capture(&server, &cmd) {
            Ok((true, stdout, _)) => match serde_json::from_str::<serde_json::Value>(&stdout) {
                Ok(v) => {
                    let freed = v.get("freed_mb").and_then(|x| x.as_f64()).unwrap_or(0.0);
                    let detail = v
                        .get("message")
                        .and_then(|x| x.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty());
                    match detail {
                        Some(d) => format!(
                            "Limpeza concluída em '{}': {:.1} MB liberados ({d}).",
                            server.name, freed
                        ),
                        None => format!(
                            "Limpeza concluída em '{}': {:.1} MB liberados.",
                            server.name, freed
                        ),
                    }
                }
                Err(_) => format!("Limpeza concluída em '{}'.", server.name),
            },
            Ok((false, _, stderr)) => format!(
                "Falha ao executar limpeza em '{}': {}",
                server.name,
                stderr.trim()
            ),
            Err(e) => e,
        };
        {
            let mut s = sh2.lock().unwrap();
            s.runner_sync_status = msg;
        }
        engine::notify(&sh2);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_capacity_tier_classifies_all_tiers() {
        let (t0, _) = compute_capacity_tier(1, 512, 8);
        assert_eq!(t0, CapacityTier::Insufficient);

        let (t1, _) = compute_capacity_tier(1, 2048, 20);
        assert_eq!(t1, CapacityTier::Light);

        let (t2, _) = compute_capacity_tier(2, 4096, 40);
        assert_eq!(t2, CapacityTier::Standard);

        let (t3, _) = compute_capacity_tier(4, 8192, 80);
        assert_eq!(t3, CapacityTier::Heavy);
    }

    #[test]
    fn parse_probe_output_parses_real_linux_metrics() {
        let sample = "\
OS=Ubuntu 24.04.1 LTS\r\n\
ARCH=x86_64\r\n\
VCPUS=4\r\n\
RAM_TOTAL_MB=7952\r\n\
RAM_FREE_MB=6120\r\n\
DISK_FREE_GB=74\r\n\
HAS_GIT=1\r\n\
HAS_DOCKER=1\r\n\
HAS_NODE=1\r\n\
HAS_PNPM=0\r\n\
HAS_RUST=1\r\n\
HAS_SYSTEMD=1\r\n\
HAS_RUNNER=1\r\n\
RUNNER_ACTIVE=1\r\n";

        let res = parse_probe_output(sample);
        assert!(res.ok);
        assert_eq!(res.os, "Ubuntu 24.04.1 LTS");
        assert_eq!(res.arch, "x86_64");
        assert_eq!(res.vcpus, 4);
        assert_eq!(res.ram_total_mb, 7952);
        assert_eq!(res.ram_free_mb, 6120);
        assert_eq!(res.disk_free_gb, 74);
        assert!(res.has_git);
        assert!(res.has_docker);
        assert!(res.has_node);
        assert!(!res.has_pnpm);
        assert!(res.has_rust);
        assert!(res.has_systemd);
        assert!(res.has_widder_runner);
        assert!(res.runner_active);
        assert_eq!(res.tier, CapacityTier::Heavy);
    }

    #[test]
    fn password_auth_does_not_leak_secret_into_ssh_args() {
        let _guard = vault::test_lock();
        let srv = VpsServer {
            id: "vps-askpass-test".into(),
            name: "Teste".into(),
            host: "192.0.2.10".into(),
            port: 2222,
            user: "deploy".into(),
            auth: AuthMethod::PasswordInVault,
            last_probe: None,
            runner_installed: false,
            runner_port: 8787,
            webhook_secret_key: None,
        };
        let key = vault::vps_password_key(&srv.id);
        vault::save_secret(&key, "S3nh@Sup3rS3cr3t!").unwrap();

        let (cmd, askpass) = build_ssh_command(&srv, "echo ok").unwrap();
        assert!(askpass.is_some());
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert!(!args.iter().any(|a| a.contains("S3nh@Sup3rS3cr3t!")));
        assert!(args.contains(&"deploy@192.0.2.10".to_string()));
        assert!(args.contains(&"2222".to_string()));

        let askpass_path = askpass.as_ref().unwrap().path.clone();
        assert!(askpass_path.exists());
        drop(askpass);
        assert!(!askpass_path.exists());
        let _ = vault::delete_secret(&key);
    }

    #[test]
    fn merge_remote_runs_updates_and_sorts_newest_first() {
        let mut local = vec![Run {
            id: "run-vps-1".into(),
            workflow_id: "wf-1".into(),
            workflow_name: "CI".into(),
            status: RunStatus::Running,
            trigger_label: "webhook".into(),
            title: "Antigo".into(),
            number: 1,
            commit: None,
            started_at: "17/11/2026 10:00".into(),
            started_ts: Some(100),
            finished_at: None,
            steps: vec![],
            approved: false,
            cancel_requested: false,
            secs: 0.0,
            actor: "github".into(),
            branch: "main".into(),
            summary: String::new(),
            annotations: vec![],
        }];

        let remote = vec![
            Run {
                id: "run-vps-1".into(),
                status: RunStatus::Success,
                ..local[0].clone()
            },
            Run {
                id: "run-vps-2".into(),
                number: 2,
                started_ts: Some(200),
                ..local[0].clone()
            },
        ];

        merge_remote_runs(&mut local, remote);
        assert_eq!(local.len(), 2);
        assert_eq!(local[0].id, "run-vps-2");
        assert_eq!(local[1].status, RunStatus::Success);
    }

    #[test]
    fn provision_script_includes_systemd_and_port() {
        let opts = ProvisionOptions {
            install_git: true,
            install_docker: false,
            install_node: true,
            install_rust: false,
            install_runner_service: true,
            runner_port: 9090,
        };
        let script = build_provision_script(&opts, "tok123", "whsec456");
        assert!(script.contains("workploy-runner.service"));
        assert!(script.contains("Workploy Runner Daemon"));
        assert!(script.contains("--port 9090"));
        assert!(script.contains("/opt/workploy/workploy_runner.py"));
        assert!(script.contains("WORKPLOY_RUNNER_EOF"));
        assert!(script.contains("WORKPLOY_SERVICE_EOF"));
        assert!(script.contains("\"User-Agent\": \"workploy-runner\""));
        assert!(script.contains("def dir_size_mb("));
        assert!(script.contains("def run_purge("));
        assert!(script.contains("/var/lib/workploy/purge_state.json"));
        assert!(script.contains("/etc/workploy/config.json"));
        assert!(script.contains("\"/purge\""));
        assert!(!script.contains("widder"));
        assert!(!script.contains("docker.io"));

        assert!(PROBE_REMOTE_SCRIPT.contains("/opt/workploy/workploy_runner.py"));
        assert!(PROBE_REMOTE_SCRIPT.contains("workploy-runner"));
        assert!(!PROBE_REMOTE_SCRIPT.contains("widder"));
    }
}
