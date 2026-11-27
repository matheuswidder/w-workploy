use crate::engine::{self, shell_single_quote, SharedHandle};
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
    // Keepalive explícito: sem isso, uma conexão que cai silenciosamente
    // (NAT, reboot da VPS, troca de rede) deixa o cliente ssh pendurado
    // "em andamento" para sempre — nem linha nova, nem término. Com
    // ServerAlive 15s x 4, o próprio ssh detecta a conexão morta em ~1 min
    // e sai (exit 255), inclusive nos caminhos de captura (`cmd.output()`).
    cmd.arg("-o")
        .arg("ConnectTimeout=12")
        .arg("-o")
        .arg("ServerAliveInterval=15")
        .arg("-o")
        .arg("ServerAliveCountMax=4")
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

/// Tempo máximo de inatividade numa sessão SSH de um passo: nenhuma linha de
/// log e nenhum término do comando remoto. Uma conexão que perde a rota no
/// meio de um build longo fica exatamente nesse estado; sem esse limite o
/// passo permanece 'em andamento' para sempre, sem linha nova de log.
/// Padrão: 20 min. `WORKPLOY_SSH_IDLE_TIMEOUT_SECS` sobrescreve (60 s a 24 h).
pub fn ssh_idle_timeout() -> Duration {
    const DEFAULT_SECS: u64 = 20 * 60;
    let secs = std::env::var("WORKPLOY_SSH_IDLE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_SECS)
        .clamp(60, 24 * 60 * 60);
    Duration::from_secs(secs)
}

/// Resultado do bombeamento de uma sessão de streaming.
#[derive(Debug)]
pub(crate) enum StreamOutcome {
    /// O processo terminou normalmente.
    Exited(std::process::ExitStatus),
    /// Cancelamento pedido — o cliente ssh foi morto localmente.
    Cancelled,
    /// Inatividade: sem linha nova e sem término por `idle_timeout`.
    /// O cliente ssh foi morto localmente para destravar o passo.
    TimedOut(Duration),
    /// O watch do processo falhou (caso degenerado de pid inválido).
    WatchFailed,
}

/// Bombeia as linhas do canal enquanto aguarda o término do processo, com
/// três saídas além do término natural: cancelamento, falha de watch e — o
/// corte de segurança contra sessões mudas — o timeout de inatividade.
/// Linha recebida reinicia o cronômetro; build longo que emite log de tempos
/// em tempos nunca é cortado.
fn stream_until_exit<F>(
    child: &mut std::process::Child,
    rx: &std::sync::mpsc::Receiver<String>,
    cancel: Option<&AtomicBool>,
    idle_timeout: Duration,
    mut on_line: F,
) -> StreamOutcome
where
    F: FnMut(&str),
{
    let mut last_activity = Instant::now();
    loop {
        let mut got_line = false;
        while let Ok(line) = rx.try_recv() {
            on_line(&line);
            got_line = true;
        }
        if got_line {
            last_activity = Instant::now();
        }
        match child.try_wait() {
            Ok(Some(st)) => return StreamOutcome::Exited(st),
            Ok(None) => {}
            Err(_) => return StreamOutcome::WatchFailed,
        }
        if last_activity.elapsed() >= idle_timeout {
            let silent_for = last_activity.elapsed();
            let _ = child.kill();
            let _ = child.wait();
            return StreamOutcome::TimedOut(silent_for);
        }
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            let _ = child.kill();
            let _ = child.wait();
            return StreamOutcome::Cancelled;
        }
        std::thread::sleep(Duration::from_millis(80));
    }
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
    idle_timeout: Duration,
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

    // Polling temporal independente de linhas: o cancelamento é checado a
    // cada ~80 ms mesmo que o comando remoto fique minutos sem emitir nada,
    // e o timeout de inatividade corta sessões mudas (rede caída no meio de
    // um build longo — sem isso o passo fica 'em andamento' para sempre).
    // Ao cancelar ou esgotar, o cliente ssh local é morto na hora — a sessão
    // remota recebe SIGHUP quando a conexão cai.
    let outcome = stream_until_exit(&mut child, &rx, cancel.map(|c| c.as_ref()), idle_timeout, |line| {
        on_line(line);
    });

    for h in readers {
        let _ = h.join();
    }
    while let Ok(line) = rx.try_recv() {
        on_line(&line);
    }

    match outcome {
        StreamOutcome::Exited(status) => Ok(status.success()),
        StreamOutcome::Cancelled => Ok(false),
        StreamOutcome::WatchFailed => Ok(false),
        StreamOutcome::TimedOut(silent_for) => Err(format!(
            "[tempo esgotado] a conexão com a VPS ficou muda por {} (nenhuma linha de log e nenhum término) — o passo foi interrompido para não travar o fluxo. Verifique a rede/a VPS e rode o passo novamente.",
            fmt_dur(silent_for.as_secs_f64())
        )),
    }
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
            "CPU_PCT" => res.cpu_usage_pct = val.parse::<u16>().unwrap_or(0).min(100) as u8,
            "DISK_USED_PCT" => res.disk_used_pct = val.parse::<u16>().unwrap_or(0).min(100) as u8,
            "HAS_GIT" => res.has_git = val == "1" || val.eq_ignore_ascii_case("true"),
            "HAS_DOCKER" => res.has_docker = val == "1" || val.eq_ignore_ascii_case("true"),
            "HAS_NODE" => res.has_node = val == "1" || val.eq_ignore_ascii_case("true"),
            "HAS_PNPM" => res.has_pnpm = val == "1" || val.eq_ignore_ascii_case("true"),
            "HAS_RUST" => res.has_rust = val == "1" || val.eq_ignore_ascii_case("true"),
            "HAS_SYSTEMD" => res.has_systemd = val == "1" || val.eq_ignore_ascii_case("true"),
            "HAS_RUNNER" => res.has_beacon_runner = val == "1" || val.eq_ignore_ascii_case("true"),
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
export PATH="$HOME/.cargo/bin:/usr/local/cargo/bin:$PATH"
OS=$(. /etc/os-release 2>/dev/null && printf "%s" "$PRETTY_NAME" || uname -s)
ARCH=$(uname -m 2>/dev/null || echo unknown)
VCPUS=$(nproc 2>/dev/null || grep -c ^processor /proc/cpuinfo 2>/dev/null || echo 1)
MEM_TOT=$(awk "/^MemTotal:/{printf \"%d\", \$2/1024}" /proc/meminfo 2>/dev/null || echo 0)
MEM_AVAIL=$(awk "/^MemAvailable:/{printf \"%d\", \$2/1024}" /proc/meminfo 2>/dev/null || echo 0)
DISK_FREE=$(df -BG --output=avail / 2>/dev/null | tail -n 1 | tr -dc "0-9")
[ -z "$DISK_FREE" ] && DISK_FREE=$(df -k / 2>/dev/null | awk "NR==2{printf \"%d\", \$4/1048576}")
DISK_USED_PCT=$(df -P / 2>/dev/null | awk "NR==2{gsub(\"%\",\"\"); print \$5}" | tr -dc "0-9")
[ -z "$DISK_USED_PCT" ] && DISK_USED_PCT=0
CPU_PCT=$( (awk "/^cpu /{print \$2+\$3+\$4+\$5+\$6+\$7+\$8+\$9, \$5+\$6; exit}" /proc/stat 2>/dev/null; (sleep 0.2 2>/dev/null || sleep 1); awk "/^cpu /{print \$2+\$3+\$4+\$5+\$6+\$7+\$8+\$9, \$5+\$6; exit}" /proc/stat 2>/dev/null) | awk "NR==1{t1=\$1;i1=\$2} NR==2{dt=\$1-t1; di=\$2-i1; if(dt>0) printf \"%d\", (100*(dt-di))/dt; else print 0}" )
[ -z "$CPU_PCT" ] && CPU_PCT=0
has_cmd() { command -v "$1" >/dev/null 2>&1 && echo 1 || echo 0; }
HAS_GIT=$(has_cmd git)
HAS_DOCKER=$(has_cmd docker)
HAS_NODE=$(has_cmd node)
HAS_PNPM=$(has_cmd pnpm)
HAS_RUST=$(has_cmd cargo)
HAS_SYSTEMD=$([ -d /run/systemd/system ] && command -v systemctl >/dev/null 2>&1 && echo 1 || echo 0)
HAS_RUNNER=$([ -f /opt/workploy/workploy_runner.py ] && echo 1 || echo 0)
RUNNER_ACT=$(systemctl is-active --quiet workploy-runner 2>/dev/null && echo 1 || echo 0)
printf "OS=%s\nARCH=%s\nVCPUS=%s\nRAM_TOTAL_MB=%s\nRAM_FREE_MB=%s\nDISK_FREE_GB=%s\nCPU_PCT=%s\nDISK_USED_PCT=%s\nHAS_GIT=%s\nHAS_DOCKER=%s\nHAS_NODE=%s\nHAS_PNPM=%s\nHAS_RUST=%s\nHAS_SYSTEMD=%s\nHAS_RUNNER=%s\nRUNNER_ACTIVE=%s\n" \
"$OS" "$ARCH" "$VCPUS" "$MEM_TOT" "$MEM_AVAIL" "${DISK_FREE:-0}" "${CPU_PCT:-0}" "${DISK_USED_PCT:-0}" "$HAS_GIT" "$HAS_DOCKER" "$HAS_NODE" "$HAS_PNPM" "$HAS_RUST" "$HAS_SYSTEMD" "$HAS_RUNNER" "$RUNNER_ACT"
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
                    crate::diagnostic::humanize_transport_error(err)
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
                verdict: crate::diagnostic::humanize_transport_error(&e),
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
                    srv.runner_installed = probe.has_beacon_runner;
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

fn push_provision_log_line(s: &mut Shared, line: &str) {
    s.provision_log.push_str(line);
    if !line.ends_with('\n') {
        s.provision_log.push('\n');
    }
    const CAP: usize = 200_000;
    if s.provision_log.len() > CAP {
        let mut cut = s.provision_log.len() - CAP;
        while cut < s.provision_log.len() && !s.provision_log.is_char_boundary(cut) {
            cut += 1;
        }
        s.provision_log.drain(..cut);
    }
}

fn append_provision_log(sh: &SharedHandle, line: &str) {
    {
        let mut s = sh.lock().unwrap();
        push_provision_log_line(&mut s, line);
    }
    engine::notify(sh);
}

pub(crate) fn generate_random_hex(prefix: &str) -> String {
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_ok() {
        let mut hex = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            use std::fmt::Write;
            let _ = write!(hex, "{b:02x}");
        }
        return format!("{prefix}-{hex}");
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let mixed = nanos ^ (pid.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    format!("{prefix}-{nanos:x}-{mixed:016x}")
}

pub const RUNNER_SCRIPT_MARKER: &str = "WORKPLOY_RUNNER_VERSION = \"3\"";

/// Artefatos da página mobile, embutidos no binário: o runner os serve de
/// dentro do próprio `workploy_runner.py`, sem depender de arquivo externo
/// na VPS. Ver `runner_mobile_injection_block` para o formato da injeção.
pub const MOBILE_INDEX_HTML: &str = include_str!("../assets/mobile/index.html");
pub const MOBILE_APP_CSS: &str = include_str!("../assets/mobile/app.css");
pub const MOBILE_APP_JS: &str = include_str!("../assets/mobile/app.js");

/// Payload sintético hostil, publicado por `build.rs` só quando a compilação
/// é de teste (`WORKPLOY_TEST_BUILD=1`). Vazio em qualquer build normal — e é
/// exatamente essa lista que `install_runner` recusa publicar na VPS
/// (mensagem `manual:` no provisionamento e erro em
/// `ensure_runner_script_current`, o caminho de auto-update).
const MOBILE_HOSTILE_ASSETS: &[(&str, &str)] =
    include!(concat!(env!("OUT_DIR"), "/mobile_mock_assets.rs"));

/// Escapa o payload para um literal Python delimitado por `"""`.
///
/// O literal não é bruto: `\"` continua aspas, `\\` vira barra e `\uXXXX`
/// vira o caractere. Por isso só quatro coisas saem da forma crua:
/// - `"""` e `'''` crus fechariam o literal ou abririam outro (viram
///   `\u0022\u0022\u0022` e `\u0027\u0027\u0027`);
/// - `\` cru apagaria a barra do JS ou criaria escape inválido (vira `\\`);
/// - `\r` sozinho é normalizado pelo compilador Python (vira `\u000d`).
///
/// Crase, `${`, `\n`, `\t` e o resto entram como estão: dentro de `"""` não
/// têm efeito, e o arquivo servido fica byte a byte igual ao original.
fn py_escape_hostile(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 64);
    let mut from = 0;
    let mut i = 0;
    while i < text.len() {
        let rest = &text.as_bytes()[i..];
        let third = if rest.len() >= 3 {
            rest[..3].to_vec()
        } else {
            Vec::new()
        };
        let (len, replacement) = if third.as_slice() == b"\"\"\"" || third.as_slice() == b"'''" {
            let ch = format!(r"\u{:04x}", rest[0]);
            (3, ch.repeat(3))
        } else {
            match rest[0] {
                b'\\' => (1, "\\\\".to_string()),
                b'\r' if rest.len() < 2 || rest[1] != b'\n' => (1, "\\u000d".to_string()),
                _ => (1, String::new()),
            }
        };
        if len != 1 || replacement != String::new() {
            out.push_str(&text[from..i]);
            out.push_str(&replacement);
            from = i + len;
        }
        i += len;
    }
    out.push_str(&text[from..]);
    out
}
/// Bloco Python que embute a página mobile no runner, pronto para entrar no
/// template. Sempre com os arquivos reais; os testes provam `ast.parse` do
/// script gerado e o payload hostil entra por `MOBILE_HOSTILE_ASSETS`.
fn runner_mobile_injection_block() -> String {
    let (index, css, js) = (MOBILE_INDEX_HTML, MOBILE_APP_CSS, MOBILE_APP_JS);
    format!(
        "# Página mobile servida em /ui: os três arquivos de app/assets/mobile\n\
         # vêm embutidos aqui (o gerador injeta; ver vps.rs) e são\n\
         # estáticos — não contêm segredo. As rotas de DADOS continuam\n\
         # exigindo Authorization: Bearer.\n\
         MOBILE_ASSETS = {{\n\
         \x20   \"index\": \"\"\"{}```,\n\
         \x20   \"css\": \"\"\"{}```,\n\
         \x20   \"js\": \"\"\"{}```,\n\
         }}\n\
         MOBILE_CONTENT_TYPES = {{\n\
         \x20   \"/ui\": \"text/html; charset=utf-8\",\n\
         \x20   \"/ui/app.css\": \"text/css; charset=utf-8\",\n\
         \x20   \"/ui/app.js\": \"text/javascript; charset=utf-8\",\n\
         }}",
        py_escape_hostile(index),
        py_escape_hostile(css),
        py_escape_hostile(js),
    )
    .replace("```", "\"\"\"")
}

/// O runner gerado (JSON-compatível, um único arquivo para a VPS) já vem com
/// os artefatos mobile injetados. Todo acesso ao script passa por aqui.
pub fn runner_py_source() -> String {
    WORKPLOY_RUNNER_PY.replace("__WORKPLOY_MOBILE_ASSETS__", &runner_mobile_injection_block())
}

pub const WORKPLOY_RUNNER_PY: &str = r##"#!/usr/bin/env python3
# WORKPLOY_RUNNER_VERSION = "3"
__WORKPLOY_MOBILE_ASSETS__
import argparse
import hashlib
import hmac
import http.server
import json
import os
import re
import shutil
import signal
import socketserver
import subprocess
import threading
import time
import urllib.error
import urllib.request
from datetime import datetime
from urllib.parse import parse_qs, urlsplit

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
SUBSTEP_MONO = {}


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
                        for sub in st.get("substeps", []):
                            if sub.get("status") in ("Pending", "Running", "WaitingApproval"):
                                sub["status"] = "Skipped"
            RUNS = data[:100]
            save_runs_locked()
    except json.JSONDecodeError:
        RUNS = []


def save_runs_locked(seen_approval_seq=None):
    # Uma decisão de portão (aprovar/recusar) pode chegar de OUTRA thread
    # (o celular posta em `/runs/<id>/approve`) enquanto o executor monta a
    # sua escrita. Quando quem grava informa a decisão que já viu
    # (`seen_approval_seq`), toda decisão mais nova é reaplicada antes do
    # dump — sem isso a aprovação da thread principal seria sobrescrita.
    if seen_approval_seq is not None:
        for r in RUNS:
            gate = r.get("approval")
            if isinstance(gate, dict) and (gate.get("seq") or 0) > seen_approval_seq:
                r["approved"] = gate.get("approved", False)
                r["cancel_requested"] = gate.get("cancel", False)
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


def prepare_workspace(repo_full_name, branch, custom_workdir, commit_sha=None):
    if custom_workdir and custom_workdir.strip():
        wd = custom_workdir.strip()
        os.makedirs(wd, exist_ok=True)
        return wd, "", True
    os.makedirs(WORKSPACES_DIR, exist_ok=True)
    if not repo_full_name:
        return WORKSPACES_DIR, "", True
    wd = os.path.join(WORKSPACES_DIR, safe_slug(repo_full_name))
    gh_tok = read_text(GITHUB_TOKEN_PATH, "")
    if gh_tok:
        remote_url = f"https://x-access-token:{gh_tok}@github.com/{repo_full_name}.git"
    else:
        remote_url = f"https://github.com/{repo_full_name}.git"
    logs = []
    target_branch = (branch or "main").strip() or "main"
    try:
        if not os.path.isdir(os.path.join(wd, ".git")):
            os.makedirs(wd, exist_ok=True)
            res = subprocess.run(
                ["git", "clone", "--branch", target_branch, remote_url, wd],
                capture_output=True,
                text=True,
                timeout=300,
            )
            logs.append(f"[git clone] exit={res.returncode}")
            if res.stdout:
                logs.append(res.stdout)
            if res.stderr:
                logs.append(res.stderr.replace(gh_tok, "***") if gh_tok else res.stderr)
            if res.returncode != 0:
                return wd, "\n".join(logs) + "\n", False
        else:
            subprocess.run(["git", "-C", wd, "remote", "set-url", "origin", remote_url], capture_output=True, text=True)
            res = subprocess.run(
                ["git", "-C", wd, "fetch", "origin"],
                capture_output=True,
                text=True,
                timeout=180,
            )
            if res.returncode != 0:
                err_txt = (res.stderr or res.stdout or "").replace(gh_tok, "***") if gh_tok else (res.stderr or res.stdout or "")
                logs.append(f"[git fetch erro] exit={res.returncode}\n{err_txt}")
                return wd, "\n".join(logs) + "\n", False
        if commit_sha and commit_sha.strip():
            res2 = subprocess.run(
                ["git", "-C", wd, "checkout", "--force", commit_sha.strip()],
                capture_output=True,
                text=True,
                timeout=120,
            )
        else:
            subprocess.run(["git", "-C", wd, "checkout", "-B", target_branch, f"origin/{target_branch}"], capture_output=True, text=True)
            res2 = subprocess.run(
                ["git", "-C", wd, "reset", "--hard", f"origin/{target_branch}"],
                capture_output=True,
                text=True,
                timeout=120,
            )
        if res2.returncode != 0:
            err_txt = (res2.stderr or res2.stdout or "").replace(gh_tok, "***") if gh_tok else (res2.stderr or res2.stdout or "")
            logs.append(f"[git checkout erro] exit={res2.returncode}\n{err_txt}")
            return wd, "\n".join(logs) + "\n", False
        rev = subprocess.run(["git", "-C", wd, "rev-parse", "--short", "HEAD"], capture_output=True, text=True)
        logs.append(f"[git sync] workspace={wd} ref={(rev.stdout or '?').strip()}")
    except Exception as exc:
        err_s = str(exc).replace(gh_tok, "***") if gh_tok else str(exc)
        logs.append(f"[git sync erro] {err_s}")
        return wd, "\n".join(logs) + "\n", False
    return wd, "\n".join(logs) + ("\n" if logs else ""), True


def _diagnose_failure(step_name, log_text):
    low = (log_text or "").lower()
    sub_name = None
    for line in reversed((log_text or "").splitlines()):
        t = line.strip()
        if t.startswith("=== ") and t.endswith(" ==="):
            sub_name = t[4:-4].strip()
            break
    job_label = f"{step_name} › {sub_name}" if sub_name else step_name
    if "err_pnpm_no_pkg_manifest" in low or "no package.json" in low:
        headline = "Diretório sem package.json — verifique workspace/workdir."
    elif "command not found" in low:
        headline = "Comando não encontrado no PATH da VPS."
    elif "no space left on device" in low:
        headline = "Disco da VPS sem espaço livre."
    elif "error ts" in low:
        headline = "Erro de tipagem TypeScript."
    elif "fail " in low or "assertionerror" in low or "test timed out" in low:
        headline = f"Falha nos testes em '{sub_name or step_name}'."
    elif "docker compose" in low or "copy failed" in low:
        headline = "Falha no build ou subida dos containers Docker."
    elif "[git " in low or "authentication failed" in low:
        headline = "Falha ao sincronizar repositório do GitHub."
    else:
        headline = f"Etapa '{sub_name or step_name}' retornou código de erro."
    tail = [l.strip()[:180] for l in (log_text or "").splitlines() if l.strip() and not l.strip().startswith("=== ")][-4:]
    msg = headline + ("\nEvidência:\n• " + "\n• ".join(tail) if tail else "")
    return headline, [{"kind": "Error", "job": job_label, "msg": msg}]


def _finalize_running_substep(run_id, step_idx, step_obj, final_status):
    subs = step_obj.setdefault("substeps", [])
    if subs and subs[-1].get("status") == "Running":
        subs[-1]["status"] = final_status
        key = (run_id, step_idx, len(subs) - 1)
        t0 = SUBSTEP_MONO.pop(key, None)
        if t0 is not None:
            subs[-1]["secs"] = max(0.0, time.monotonic() - t0)
        else:
            st_ts = subs[-1].get("started_ts")
            subs[-1]["secs"] = max(0.0, float(int(time.time()) - int(st_ts))) if st_ts else 0.0


def append_step_log(run_id, step_idx, text):
    with STATE_LOCK:
        r = find_run_locked(run_id)
        if r and 0 <= step_idx < len(r["steps"]):
            step_obj = r["steps"][step_idx]
            subs = step_obj.setdefault("substeps", [])
            for seg in text.splitlines(keepends=True) or [text]:
                stripped = seg.strip()
                if stripped.startswith("::workploy-substep::"):
                    name = stripped[len("::workploy-substep::"):].strip() or "Sub-etapa"
                    _finalize_running_substep(run_id, step_idx, step_obj, "Success")
                    subs.append({
                        "name": name,
                        "status": "Running",
                        "log": "",
                        "secs": 0.0,
                        "substeps": [],
                        "started_ts": int(time.time()),
                    })
                    SUBSTEP_MONO[(run_id, step_idx, len(subs) - 1)] = time.monotonic()
                    cur = step_obj.get("log", "") + f"=== {name} ===\n"
                    if len(cur) > 120000:
                        cur = cur[-120000:]
                    step_obj["log"] = cur
                else:
                    cur = step_obj.get("log", "") + seg
                    if len(cur) > 120000:
                        cur = cur[-120000:]
                    step_obj["log"] = cur
                    if subs and subs[-1].get("status") == "Running":
                        sub_cur = subs[-1].get("log", "") + seg
                        if len(sub_cur) > 120000:
                            sub_cur = sub_cur[-120000:]
                        subs[-1]["log"] = sub_cur
            save_runs_locked()


def _limit_prefix(cpu_limit, mem_limit_mb):
    """Comandos de shell que aplicam os limites de recursos do Workploy.

    CPU: `taskset` fixa o processo em N nucleos (deixa os demais livres para a
    producao que convive na mesma VPS).
    RAM: `systemd-run --scope` cria um cgroup com MemoryMax (limite real de
    memoria, com OOM previsivel); sem systemd, cai para `ulimit -v`.
    """
    parts = []
    if cpu_limit and int(cpu_limit) > 0:
        cores = int(cpu_limit)
        try:
            total = os.cpu_count() or cores
        except Exception:
            total = cores
        if cores < total and shutil.which("taskset"):
            parts.append(f"taskset -c 0-{cores - 1}")
    if mem_limit_mb and int(mem_limit_mb) > 0:
        mb = int(mem_limit_mb)
        if shutil.which("systemd-run"):
            parts.append(f"systemd-run --scope --quiet -p MemoryMax={mb}M --")
        else:
            parts.append(f"bash -c 'ulimit -v {mb * 1024}; exec \"$0\"'")
    return parts


def _apply_limits(cmd, cpu_limit, mem_limit_mb):
    parts = _limit_prefix(cpu_limit, mem_limit_mb)
    if not parts:
        return cmd
    return " ".join(parts) + " " + cmd


def _measure_children():
    try:
        import resource
        usage = resource.getrusage(resource.RUSAGE_CHILDREN)
        return usage.ru_utime + usage.ru_stime, int(usage.ru_maxrss / 1024)
    except Exception:
        return 0.0, 0


def _build_step_env(workdir="", repo="", branch=""):
    env = os.environ.copy()
    # HOME pode não vir no ambiente do serviço (systemd) — sem ele, `~` e
    # `$HOME/.cargo/env` quebram e o PATH perde os diretórios do usuário.
    home = env.get("HOME") or "/root"
    env["HOME"] = home
    # PATH com os diretórios do sistema primeiro: `free`, `df`, `git`, `dnf` e
    # `apt-get` vivem em /usr/bin e /bin (que pode ser symlink de /usr/bin).
    system = ["/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"]
    user = [
        os.path.join(home, ".local/share/pnpm"),
        "/usr/local/share/pnpm",
        os.path.join(home, ".cargo/bin"),
        "/usr/local/cargo/bin",
    ]
    nvm_dir = os.path.join(home, ".nvm/versions/node")
    if os.path.isdir(nvm_dir):
        try:
            vers = sorted(os.listdir(nvm_dir))
            if vers:
                user.insert(0, os.path.join(nvm_dir, vers[-1], "bin"))
        except OSError:
            pass
    cur = env.get("PATH", "")
    env["PATH"] = ":".join([p for p in (system + user) if p] + ([cur] if cur else []))
    # Contexto do repositório para as receitas: a raiz real do workspace e o
    # repo/branch disparados.
    if workdir:
        env["WORKPLOY_WORKDIR"] = workdir
    if repo:
        env["WORKPLOY_REPO"] = repo
    if branch:
        env["WORKPLOY_BRANCH"] = branch
    return env


def execute_run(run_id, wf, repo_full_name="", branch="main", commit_sha=None):
    t_start = time.monotonic()
    # `None` = ainda não li nenhuma decisão; a primeira decisão que o executor
    # ver (no portão ou no fechamento) vira a linha de base e todo o resto do
    # run continua com ela. Decisão de OUTRA thread (celular) com id maior é
    # reaplicada em `save_runs_locked`, senão a aprovação seria sobrescrita.
    seen_approval_seq = None
    workdir, prep_log, sync_ok = prepare_workspace(repo_full_name, branch, wf.get("workdir", ""), commit_sha)
    with STATE_LOCK:
        ACTIVE_WORKDIRS[run_id] = workdir
    try:
        if prep_log and len(wf.get("steps", [])) > 0:
            append_step_log(run_id, 0, prep_log)
        if not sync_ok:
            with STATE_LOCK:
                r = find_run_locked(run_id)
                if r:
                    if r["steps"]:
                        r["steps"][0]["status"] = "Failed"
                        headline, anns = _diagnose_failure(r["steps"][0].get("name", "Git sync"), r["steps"][0].get("log", ""))
                        r["annotations"] = anns
                        r["summary"] = f"# Falha em {r.get('workflow_name', 'Fluxo')}\n- Etapa: {r['steps'][0].get('name')}\n- Motivo: {headline}\n"
                    for st in r["steps"][1:]:
                        if st["status"] == "Pending":
                            st["status"] = "Skipped"
                    r["status"] = "Failed"
                    r["finished_at"] = now_str()
                    r["secs"] = time.monotonic() - t_start
                    save_runs_locked()
            return

        steps = wf.get("steps", [])
        step_env = _build_step_env(workdir, repo_full_name, branch)
        extras = wf.get("extras") or {}
        if not extras:
            try:
                extras = (load_config().get("extras") or {}).get(wf.get("id"), {}) or {}
            except Exception:
                extras = {}
        skipped = [str(s) for s in (extras.get("skipped_steps") or [])]
        cpu_limit = extras.get("cpu_limit")
        mem_limit_mb = extras.get("mem_limit_mb")
        limits_desc = []
        if cpu_limit:
            limits_desc.append(f"{cpu_limit} vCPU")
        if mem_limit_mb:
            limits_desc.append(f"{mem_limit_mb} MB RAM")
        metrics = {"cpu_secs": 0.0, "peak_rss_mb": 0, "oom_step": None}
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

            step_name = step.get("name", "")
            if skipped and step_name in skipped:
                with STATE_LOCK:
                    r = find_run_locked(run_id)
                    if r:
                        r["steps"][idx]["status"] = "Skipped"
                        r["steps"][idx]["secs"] = 0.0
                        save_runs_locked()
                append_step_log(run_id, idx, "[pulado] este passo está marcado como 'Pular' na receita.\n")
                continue

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
                limited_cmd = _apply_limits(cmd, cpu_limit, mem_limit_mb)
                if limits_desc:
                    append_step_log(
                        run_id,
                        idx,
                        "[limites] aplicado neste passo: " + " · ".join(limits_desc) + "\n",
                    )
                cpu_before, _rss_before = _measure_children()
                # start_new_session: o comando ganha seu próprio grupo de
                # processos, permitindo matar shell + subprocessos de uma vez
                # no cancelamento (sem isso, filhos como cargo/node sobrevivem
                # órfãos à morte do shell).
                proc = subprocess.Popen(
                    [shell_bin, "-lc", limited_cmd],
                    cwd=workdir,
                    env=step_env,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.STDOUT,
                    text=True,
                    errors="replace",
                    start_new_session=True,
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
                        # Mata o grupo inteiro (shell + subprocessos): builds
                        # longos não continuam rodando na VPS após o cancel.
                        try:
                            os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
                        except OSError:
                            proc.terminate()
                        try:
                            proc.wait(timeout=3)
                        except subprocess.TimeoutExpired:
                            try:
                                os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
                            except OSError:
                                proc.kill()
                        break
                    time.sleep(0.2)
                t_pump.join(timeout=2)
                with STATE_LOCK:
                    PROCESSES.pop(run_id, None)
                cpu_after, rss_after = _measure_children()
                metrics["cpu_secs"] = round(metrics["cpu_secs"] + max(0.0, cpu_after - cpu_before), 2)
                metrics["peak_rss_mb"] = max(metrics["peak_rss_mb"], rss_after)
                ok = (not cancelled) and (proc.returncode == 0)
                if cancelled:
                    append_step_log(run_id, idx, "[cancelado pelo usuário]\n")
                elif not ok:
                    # 137 / -9 com limite de memoria = cgroup OOM.
                    if mem_limit_mb and (proc.returncode in (137, -9, 9)):
                        metrics["oom_step"] = step_name
                        append_step_log(
                            run_id,
                            idx,
                            f"[limite de memória] o passo ultrapassou o teto de {mem_limit_mb} MB e foi encerrado pelo sistema.\n",
                        )
                    append_step_log(run_id, idx, "[passo falhou — o fluxo para aqui]\n")

            s_secs = time.monotonic() - s_start
            with STATE_LOCK:
                r = find_run_locked(run_id)
                if not r:
                    return
                r["steps"][idx]["secs"] = s_secs
                if seen_approval_seq is None:
                    _gate = r.get("approval")
                    seen_approval_seq = _gate.get("seq") if isinstance(_gate, dict) else -1
                if cancelled:
                    _finalize_running_substep(run_id, idx, r["steps"][idx], "Skipped")
                    r["status"] = "Cancelled"
                    r["finished_at"] = now_str()
                    r["secs"] = time.monotonic() - t_start
                    for st in r["steps"][idx:]:
                        if st["status"] in ("Pending", "WaitingApproval", "Running"):
                            st["status"] = "Skipped"
                    save_runs_locked(seen_approval_seq)
                    return
                final_sub_status = "Success" if ok else "Failed"
                _finalize_running_substep(run_id, idx, r["steps"][idx], final_sub_status)
                r["steps"][idx]["status"] = final_sub_status
                r["metrics"] = dict(metrics)
                if not ok:
                    headline, anns = _diagnose_failure(r["steps"][idx].get("name", "Passo"), r["steps"][idx].get("log", ""))
                    r["annotations"] = r.get("annotations", []) + anns
                    r["summary"] = f"# Falha em {r.get('workflow_name', 'Fluxo')}\n- Etapa: {r['steps'][idx].get('name')}\n- Motivo: {headline}\n"
                    r["status"] = "Failed"
                    r["finished_at"] = now_str()
                    r["secs"] = time.monotonic() - t_start
                    for st in r["steps"][idx + 1:]:
                        if st["status"] == "Pending":
                            st["status"] = "Skipped"
                    save_runs_locked(seen_approval_seq)
                    return
                save_runs_locked(seen_approval_seq)

        usage_line = f"- Consumo: {metrics['peak_rss_mb']} MB de pico de RAM · {metrics['cpu_secs']:.0f} s de CPU"
        if metrics.get("oom_step"):
            usage_line += f" · OOM no passo '{metrics['oom_step']}'"
        with STATE_LOCK:
            r = find_run_locked(run_id)
            if r:
                r["status"] = "Success"
                r["finished_at"] = now_str()
                r["secs"] = time.monotonic() - t_start
                r["metrics"] = dict(metrics)
                if not r.get("summary"):
                    r["summary"] = f"# {r.get('workflow_name', 'Fluxo')} concluído com sucesso\n- Etapas concluídas: {len(steps)}/{len(steps)}\n{usage_line}\n"
                save_runs_locked(seen_approval_seq)

        next_id = wf.get("on_success")
        if next_id and next_id != wf.get("id"):
            cfg = load_config()
            remote_chains = cfg.get("remote_chains") or {}
            chain_targets = cfg.get("chain_targets") or {}
            self_sid = (cfg.get("self_server_id") or "").strip()
            # O desktop já sabe onde cada encadeado roda (x-server ou a VPS de
            # CI/deploy do vínculo). Essa tabela vence o mapa de cabos: o
            # repasse precisa acontecer também quando o par só tem HTTP.
            target_sid = ""
            if isinstance(chain_targets, dict):
                target_sid = (chain_targets.get(next_id) or "").strip()
            if not target_sid and isinstance(remote_chains, dict):
                cable = remote_chains.get(next_id)
                if isinstance(cable, list) and cable:
                    first = cable[0] if isinstance(cable[0], dict) else {}
                    target_sid = (first.get("peer_id") or first.get("runner_server_id") or "").strip()
            if target_sid and target_sid != self_sid:
                ok_fwd, fwd_msg = _dispatch_remote_chain(next_id, target_sid, repo_full_name, branch, cfg)
                with STATE_LOCK:
                    r = find_run_locked(run_id)
                    if r:
                        r["summary"] = (r.get("summary") or "") + f"- Encadeamento remoto: {fwd_msg}\n"
                        if not ok_fwd:
                            r["annotations"] = r.get("annotations", []) + [{
                                "kind": "Warning",
                                "job": "Encadeamento cross-VPS",
                                "msg": fwd_msg,
                            }]
                        save_runs_locked()
            else:
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


def _find_peer(cfg, target_sid, transport):
    # Dois vocabulários convivem no arquivo: o desktop grava `peers` (cabo SSH
    # no grafo, com `peer_id`/`pair_key`) e o formato legado usava
    # `server_id`/`key`. Aceitamos os dois, e `transport` ("ssh"|"http")
    # filtra pelos que têm aquele caminho montado.
    for p in cfg.get("peers") or []:
        if not isinstance(p, dict):
            continue
        pid = (p.get("peer_id") or p.get("server_id") or "").strip()
        if pid != target_sid:
            continue
        if transport == "ssh" and not isinstance(p.get("ssh"), dict):
            continue
        if transport == "http" and not ((p.get("host") or "").strip()):
            continue
        return p
    return None


def _peer_field(peer, primary, legacy, default=""):
    val = (peer.get(primary) or "").strip()
    if not val:
        val = (peer.get(legacy) or "").strip()
    return val or default


def _dispatch_remote_chain(next_id, target_sid, repo_full_name, branch, cfg):
    peer = _find_peer(cfg, target_sid, "ssh") or _find_peer(cfg, target_sid, "http")
    if not peer:
        return False, f"VPS de destino '{target_sid}' não está configurada em peers para '{next_id}'."
    host = _peer_field(peer, "host", "host")
    port = int(peer.get("port") or 8787)
    key = _peer_field(peer, "pair_key", "key")
    peer_name = _peer_field(peer, "name", "name", target_sid)
    self_sid = (cfg.get("self_server_id") or "").strip()
    payload = json.dumps({
        "workflow_id": next_id,
        "trigger_label": "encadeado: fluxo anterior concluído",
        "repo": repo_full_name or "",
        "branch": branch or "main",
        "actor": "workploy-runner",
    }).encode("utf-8")
    # Transporte único: o gatilho pareado por HTTP no runner da VPS de
    # destino. A malha SSH saiu do produto (v0.1.22) — manter dois transportes
    # fazia o repasse depender de conta dedicada e authorized_keys sem
    # necessidade. O runner do destino continua executando o fluxo localmente:
    # é ele que autoriza, aprova e registra a execução.
    ssh_note = ""
    if not host:
        return False, f"Par '{peer_name}' sem host alcançável (runner da outra VPS)."
    if not key:
        return False, f"Par '{peer_name}' sem chave de pareamento no cofre desta VPS."
    url = f"http://{host}:{port}/trigger"
    req = urllib.request.Request(
        url,
        data=payload,
        method="POST",
        headers={
            "Content-Type": "application/json",
            "Authorization": f"Bearer {key}",
            "X-Workploy-Peer": self_sid,
            "User-Agent": "workploy-runner",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            raw = resp.read().decode("utf-8", errors="replace")
            data = json.loads(raw or "{}")
            if data.get("ok"):
                rid = data.get("run_id") or "?"
                return True, f"fluxo '{next_id}' disparado na VPS '{peer_name}' ({rid})."
            err = data.get("error") or "erro desconhecido"
            if err == "workflow_not_allowed":
                err = "o fluxo não está na lista de liberados deste par (allowed_workflows)"
            return False, f"VPS '{peer_name}' recusou '{next_id}': {err}{ssh_note}"
    except urllib.error.HTTPError as he:
        try:
            body = he.read().decode("utf-8", errors="replace")
        except Exception:
            body = ""
        if he.code == 403:
            human = "o par não autorizou este fluxo (confira a chave de pareamento e a lista de fluxos liberados)"
            return False, f"VPS '{peer_name}' recusou '{next_id}': {human}{ssh_note}"
        return False, f"HTTP {he.code} ao encaminhar '{next_id}' para '{peer_name}' ({host}:{port}): {body[:160]}{ssh_note}"
    except Exception as exc:
        return False, f"Falha de rede ao encaminhar '{next_id}' para '{peer_name}' ({host}:{port}): {exc}{ssh_note}"


def queue_workflow(wf, trigger_label, repo_full_name="", branch="main", commit_info=None, actor="workploy-runner"):
    now_ts = int(time.time())
    commit_sha = None
    title = trigger_label
    if commit_info:
        commit_sha, title = commit_info
    with STATE_LOCK:
        wf_id = wf.get("id", "")
        clean_repo = (repo_full_name or "").strip()
        clean_branch = (branch or "main").strip() or "main"
        if clean_repo:
            for existing in RUNS:
                if (
                    existing.get("status") in ("Queued", "Running", "AwaitingApproval")
                    and existing.get("workflow_id") == wf_id
                    and (existing.get("repo_name") or "").strip().lower() == clean_repo.lower()
                    and (existing.get("branch") or "main").strip() == clean_branch
                ):
                    existing["cancel_requested"] = True
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
            "repo_name": repo_full_name or None,
            "branch": branch or "main",
            "summary": "",
            "annotations": [],
        }
        RUNS.insert(0, run_obj)
        save_runs_locked()
    threading.Thread(
        target=execute_run,
        args=(run_id, wf, repo_full_name, branch or "main", commit_sha),
        daemon=True,
    ).start()
    return run_id


def branch_matches(pattern, branch):
    pat = (pattern or "").strip()
    if not pat or pat == "*":
        return True
    target = (branch or "").strip()
    for item in pat.split(","):
        cand = item.strip()
        if cand == "*" or (cand and cand == target):
            return True
    return False


def _is_windows_only_workflow(w):
    wd = (w.get("workdir") or "").strip()
    if ":\\" in wd or ":/" in wd:
        return True
    if "windows" in (w.get("name") or "").lower():
        return True
    for st in w.get("steps", []):
        if "powershell" in (st.get("run") or "").lower():
            return True
    return False


def resolve_workflows_for_binding(cfg, b):
    wf_list = cfg.get("workflows", [])
    workflows = {w.get("id"): w for w in wf_list if w.get("id")}
    wf_id = b.get("workflow_id")
    if wf_id and wf_id in workflows:
        return [workflows[wf_id]]
    bid = b.get("id")
    if bid:
        by_repo = [w for w in wf_list if w.get("repo_id") == bid]
        if by_repo:
            sourced_push = [w for w in by_repo if w.get("trigger") == "Push" and w.get("source")]
            if sourced_push:
                return sourced_push
            push_only = [w for w in by_repo if w.get("trigger") == "Push"]
            if push_only:
                return push_only
            chained_targets = {w.get("on_success") for w in by_repo if w.get("on_success")}
            roots = [w for w in by_repo if w.get("id") not in chained_targets]
            return roots or by_repo
    all_bindings = cfg.get("bindings", [])
    claimed = {
        other.get("workflow_id")
        for other in all_bindings
        if other.get("id") != bid and other.get("workflow_id")
    }
    matched = []
    for w in wf_list:
        if w.get("trigger") != "Push":
            continue
        if (w.get("repo_id") or "").strip():
            continue
        if w.get("id") in claimed:
            continue
        if _is_windows_only_workflow(w):
            continue
        matched.append(w)
    return matched


def resolve_workflow_for_binding(cfg, b):
    wfs = resolve_workflows_for_binding(cfg, b)
    return wfs[0] if wfs else None


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

        for b in bindings:
            if not b.get("enabled") or b.get("trigger_mode") != "PollOnVps":
                continue
            repo = (b.get("full_name") or "").strip()
            raw_branch = b.get("branch")
            branch_pat = (raw_branch if raw_branch is not None else "main").strip()
            wfs = resolve_workflows_for_binding(cfg, b)
            if not repo or not wfs:
                continue
            headers = {
                "User-Agent": "workploy-runner",
                "Accept": "application/vnd.github+json",
            }
            if gh_tok:
                headers["Authorization"] = f"Bearer {gh_tok}"

            targets = []
            has_wildcard = (not branch_pat) or branch_pat == "*" or any(
                item.strip() == "*" for item in branch_pat.split(",")
            )
            if has_wildcard:
                branches_url = f"https://api.github.com/repos/{repo}/branches?per_page=100"
                req_b = urllib.request.Request(branches_url, headers=headers)
                try:
                    with urllib.request.urlopen(req_b, timeout=20) as resp:
                        blist = json.loads(resp.read().decode("utf-8"))
                    if isinstance(blist, list):
                        for item in blist:
                            bname = (item.get("name") or "").strip()
                            bsha = ((item.get("commit") or {}).get("sha") or "").strip()
                            if bname and bsha:
                                targets.append((bname, bsha))
                except Exception:
                    pass
                if not targets:
                    targets.append(("main", None))
            else:
                seen = set()
                for item in branch_pat.split(","):
                    br = item.strip()
                    if br and br not in seen:
                        seen.add(br)
                        targets.append((br, None))

            for br, known_sha in targets:
                try:
                    if known_sha:
                        sha = known_sha
                        msg = f"push em {br}"
                    else:
                        url = f"https://api.github.com/repos/{repo}/commits/{br}"
                        req = urllib.request.Request(url, headers=headers)
                        with urllib.request.urlopen(req, timeout=20) as resp:
                            payload = json.loads(resp.read().decode("utf-8"))
                        sha = payload.get("sha", "")
                        msg = ((payload.get("commit") or {}).get("message") or "commit").splitlines()[0]
                    key = f"{repo}:{br}"
                    prev = poll_state.get(key)
                    if sha and prev and prev != sha:
                        if known_sha:
                            try:
                                c_url = f"https://api.github.com/repos/{repo}/commits/{sha}"
                                c_req = urllib.request.Request(c_url, headers=headers)
                                with urllib.request.urlopen(c_req, timeout=15) as c_resp:
                                    c_payload = json.loads(c_resp.read().decode("utf-8"))
                                    msg = ((c_payload.get("commit") or {}).get("message") or msg).splitlines()[0]
                            except Exception:
                                pass
                        poll_state[key] = sha
                        for wf in wfs:
                            queue_workflow(wf, "sondagem 24/7 na VPS", repo, br, (sha, msg), "github-poll")
                    elif sha and not prev:
                        poll_state[key] = sha
                    with open(POLL_STATE_PATH, "w", encoding="utf-8") as f:
                        json.dump(poll_state, f)
                except Exception:
                    pass
        time.sleep(every)


RUN_LOG_DEFAULT_TAIL = 200_000


def _int_query(query, name, default):
    # Query inválida cai no padrão em vez de virar 500: o celular nunca vê
    # stack trace por causa de `?offset=abc`.
    raw = (query.get(name) or [""])[0]
    if raw == "":
        return default
    try:
        return int(raw)
    except (TypeError, ValueError):
        return default


def api_runs_limit(query):
    # `?limit=` na lista: padrão 50, teto 100 (o mesmo teto de `RUNS`).
    n = _int_query(query, "limit", 50)
    return max(1, min(100, n))


def api_run_log_payload(run, query):
    # Log incremental do celular. Offset/tail são calculados sobre o log
    # CONCATENADO dos passos (ou do passo escolhido em `?step=<index>`), para
    # o app pedir só o que falta em vez de rebaixar a lista inteira de runs.
    tail = _int_query(query, "tail", RUN_LOG_DEFAULT_TAIL)
    if tail <= 0:
        tail = RUN_LOG_DEFAULT_TAIL
    tail = min(tail, RUN_LOG_DEFAULT_TAIL)
    offset = max(0, _int_query(query, "offset", 0))
    steps = run.get("steps") or []
    step_q = _int_query(query, "step", -1)
    if step_q >= 0:
        selected = [steps[step_q]] if step_q < len(steps) else []
    else:
        selected = steps
    full = "".join((st.get("log") or "") for st in selected)
    size = len(full)
    truncated = False
    if offset > size:
        # O celular pediu além do fim (log trocou de run ou foi reciclado):
        # devolve vazio e avisa, sem erro.
        chunk = ""
        truncated = True
    else:
        chunk = full[offset:offset + tail]
        truncated = offset + len(chunk) < size
    out_steps = []
    for st in steps:
        log = st.get("log") or ""
        out_steps.append({
            "name": st.get("name") or "",
            "status": st.get("status") or "",
            "bytes": len(log.encode("utf-8")),
            "lines": log.count("\n"),
        })
    return {
        "run_id": run.get("id") or "",
        "status": run.get("status") or "",
        "steps": out_steps,
        "log": chunk,
        "offset": offset,
        "size": size,
        "truncated": truncated,
    }


def api_approval_decision(body):
    # `POST /runs/<id>/approve`: só duas decisões, nunca implícitas. Corpo
    # ausente/inválido vira `approve` porque é o que a página manda pelo
    # caminho dedicado (o botão de recusar usa `/cancel`).
    decision = "approve"
    note = ""
    if isinstance(body, dict):
        raw = str(body.get("decision") or "").strip().lower()
        if raw:
            decision = raw
        note = str(body.get("note") or "").strip()[:500]
    return decision, note


def _step_awaiting_index(run):
    steps = run.get("steps") or []
    for idx in range(len(steps) - 1, -1, -1):
        if steps[idx].get("status") == "WaitingApproval":
            return idx
    return -1


def api_apply_approval(run, decision, note):
    # Registro único da decisão: é ESTA função que escreve no run, tanto no
    # `/runs/<id>/approve` quanto no `/trigger` com `{run_id, approve}`.
    # Não acompanha o run (o Run do desktop ignora campos extras); quem avança
    # é o executor, que espera `approved` e deriva do status.
    stamp = now_str()
    entry = {
        # Monotônico: é o que o executor usa para saber que a decisão mudou
        # depois da última leitura dele (ver `save_runs_locked`).
        "seq": time.time_ns(),
        "decision": decision,
        "note": note,
        "by": "maestro",
        "at": stamp,
        "approved": decision == "approve",
        "cancel": decision != "approve",
    }
    run["approved"] = entry["approved"]
    run["cancel_requested"] = entry["cancel"]
    run["approval"] = entry
    if decision == "approve":
        run["approved_at"] = stamp
    idx = _step_awaiting_index(run)
    if idx >= 0:
        run["steps"][idx]["log"] = (run["steps"][idx].get("log") or "") + (
            "[aprovado pelo mobile] " + (note or "sem observação") + "\n"
            if decision == "approve"
            else "[recusado pelo mobile] " + (note or "sem observação") + "\n"
        )
    return entry


class _RunnerHTTPRequestHandler(http.server.BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        return

    def send_json(self, code, data):
        body = json.dumps(data, ensure_ascii=False).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def mobile_asset_path(self):
        # `/ui/` é o mesmo documento de `/ui`; o resto precisa casar exato.
        path = urlsplit(self.path or "/").path
        if path in ("/ui", "/ui/"):
            return "/ui"
        return path if path in MOBILE_CONTENT_TYPES else None

    def mobile_asset_key(self, path):
        # O dicionário usa as chaves curtas ("index"/"css"/"js"); as rotas
        # públicas ficam em MOBILE_CONTENT_TYPES.
        return {"/ui": "index", "/ui/app.css": "css", "/ui/app.js": "js"}[path]

    def serve_mobile_asset(self):
        # Assets estáticos da página mobile, SEM Bearer: HTML/CSS/JS não têm
        # segredo. As rotas de dados (`/runs`, `/run/...`, ações) continuam
        # exigindo o token — o atalho só vale para estes três caminhos.
        path = self.mobile_asset_path()
        if path is None:
            return False
        data = MOBILE_ASSETS[self.mobile_asset_key(path)].encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", MOBILE_CONTENT_TYPES[path])
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(data)
        return True

    def check_bearer(self):
        expected = read_text(TOKEN_PATH, "")
        auth = self.headers.get("Authorization", "")
        if not expected or not auth.startswith("Bearer "):
            return False
        got = auth[7:].strip()
        return hmac.compare_digest(got, expected)

    def check_peer(self, cfg):
        # Quem chama decide o que autorizar: `/trigger` força a allowlist do
        # par; `/status` (diagnóstico "Testar gatilho do par") só comprova a
        # chave e devolve o estado, sem disparar nada.
        peer_id = (self.headers.get("X-Workploy-Peer") or "").strip()
        auth = self.headers.get("Authorization", "")
        if not peer_id or not auth.startswith("Bearer "):
            return None
        got = auth[7:].strip()
        if not got:
            return None
        for entry in cfg.get("peer_keys") or []:
            if not isinstance(entry, dict):
                continue
            if (entry.get("peer_id") or entry.get("server_id") or "").strip() != peer_id:
                continue
            expected = (entry.get("pair_key") or entry.get("key") or "").strip()
            if expected and hmac.compare_digest(got, expected):
                return entry
        return None

    def read_body(self):
        n = int(self.headers.get("Content-Length", "0") or 0)
        if n <= 0 or n > 4_194_304:
            return b""
        return self.rfile.read(n)

    def do_GET(self):
        if self.serve_mobile_asset():
            # `/ui`, `/ui/app.css`, `/ui/app.js`: página pública (HTML/CSS/JS
            # não têm segredo). Qualquer outra rota continua exigindo admin.
            return
        cfg = load_config()
        is_admin = self.check_bearer()
        peer = None
        if not is_admin:
            # O par pode checar `/status` com a chave de pareamento: é o que o
            # botão "Testar gatilho do par" usa. Nada de leitura de runs aqui.
            peer = self.check_peer(cfg)
            if peer is None or self.path != "/status":
                return self.send_json(401, {"error": "unauthorized"})
        if self.path == "/status":
            with STATE_LOCK:
                active = sum(1 for r in RUNS if r.get("status") in ("Queued", "Running", "AwaitingApproval"))
                total = len(RUNS)
            out = {
                "ok": True,
                "service": "workploy-runner",
                "runner_version": 3,
                "active_runs": active,
                "total_runs": total,
                "bindings": len(cfg.get("bindings", [])),
            }
            if peer is not None:
                # Diagnóstico do par: ele vê, e só ele, quais fluxos pode disparar aqui.
                allowed = [str(x) for x in (peer.get("allowed_workflows") or [])]
                out["peer_id"] = (peer.get("peer_id") or peer.get("server_id") or "").strip()
                out["allowed_workflows"] = allowed
            return self.send_json(200, out)
        # Tudo daqui para baixo é leitura de execução: só admin (o par já saiu
        # no 401 acima, porque só `/status` é liberado para ele).
        parsed = urlsplit(self.path or "/")
        if parsed.path == "/runs":
            limit = api_runs_limit(parse_qs(parsed.query))
            with STATE_LOCK:
                rows = list(RUNS)[:limit]
            return self.send_json(200, rows)
        m_run = re.fullmatch(r"/run/([A-Za-z0-9._:-]+)", parsed.path)
        if m_run:
            # Execução completa, identificada por `id` — o celular vê um run
            # sem baixar a lista inteira a cada ciclo de polling.
            with STATE_LOCK:
                run = find_run_locked(m_run.group(1))
            if not run:
                return self.send_json(404, {"error": "not_found"})
            return self.send_json(200, run)
        m_log = re.fullmatch(r"/run/([A-Za-z0-9._:-]+)/log", parsed.path)
        if m_log:
            query = parse_qs(parsed.query)
            with STATE_LOCK:
                run = find_run_locked(m_log.group(1))
                payload = api_run_log_payload(run, query) if run else None
            if payload is None:
                return self.send_json(404, {"error": "not_found"})
            return self.send_json(200, payload)
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
            triggered = []
            for b in cfg.get("bindings", []):
                if not b.get("enabled") or b.get("trigger_mode") != "WebhookOnVps":
                    continue
                b_repo = (b.get("full_name") or "").strip()
                if b_repo.lower() != repo:
                    continue
                raw_branch = b.get("branch")
                pat = raw_branch if raw_branch is not None else "main"
                if not branch_matches(pat, branch):
                    continue
                for wf in resolve_workflows_for_binding(cfg, b):
                    rid = queue_workflow(wf, "webhook no GitHub", b_repo or repo, branch, (sha, msg), actor)
                    triggered.append(rid)
            return self.send_json(200, {"ok": True, "triggered": triggered})

        cfg = load_config()
        is_admin = self.check_bearer()
        peer_entry = None if is_admin else self.check_peer(cfg)
        if not is_admin and not (self.path == "/trigger" and peer_entry is not None):
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
            if body.get("approve") and peer_entry is None:
                # Mesma decisão de `/runs/<id>/approve`, mesma função de
                # registro, sem criar run novo. O run vem de `run_id` ou, quando
                # só o `workflow_id` viaja, do run mais novo desse fluxo que
                # ainda esteja no portão.
                wf_id = (body.get("workflow_id") or "").strip()
                rid = str(body.get("run_id") or "").strip()
                with STATE_LOCK:
                    gate = find_run_locked(rid) if rid else None
                    if gate is None:
                        gate = next(
                            (
                                x
                                for x in RUNS
                                if x.get("status") == "AwaitingApproval"
                                and x.get("workflow_id") == wf_id
                            ),
                            None,
                        )
                    if gate is None:
                        return self.send_json(409, {"error": "not_awaiting_approval"})
                    api_apply_approval(
                        gate,
                        "approve",
                        str(body.get("note") or "aprovado pelo gatilho").strip()[:500],
                    )
                    save_runs_locked()
                return self.send_json(200, {"ok": True, "run_id": gate.get("id"), "run": gate})
            wf_id = (body.get("workflow_id") or "").strip()
            inline_wf = body.get("workflow") if isinstance(body.get("workflow"), dict) else None
            inline_extras = body.get("extras") if isinstance(body.get("extras"), dict) else {}
            if peer_entry is not None:
                # Par autenticado nunca envia comandos inline e só dispara
                # fluxos explicitamente liberados na allowlist desse par.
                inline_wf = None
                allowed = [str(x) for x in (peer_entry.get("allowed_workflows") or [])]
                if wf_id not in allowed:
                    return self.send_json(403, {"error": "workflow_not_allowed"})
            target_wf = None
            for w in cfg.get("workflows", []):
                if w.get("id") == wf_id:
                    target_wf = dict(w)
                    break
            if inline_wf is not None:
                target_wf = dict(inline_wf)
            if target_wf is None:
                return self.send_json(404, {"error": "workflow_not_found"})
            extras = (cfg.get("extras") or {}).get(wf_id, {}) or {}
            if not extras and is_admin and isinstance(inline_extras, dict):
                extras = inline_extras.get(wf_id, {}) or {}
            target_wf["extras"] = extras
            commit_sha = (body.get("commit_sha") or "").strip()
            commit_msg = (body.get("commit_msg") or body.get("trigger_label") or "manual").strip()
            commit_info = (commit_sha, commit_msg) if commit_sha else None
            rid = queue_workflow(
                target_wf,
                body.get("trigger_label", "manual"),
                body.get("repo", ""),
                body.get("branch", "main"),
                commit_info,
                body.get("actor", "maestro"),
            )
            with STATE_LOCK:
                created_run = find_run_locked(rid)
            return self.send_json(200, {"ok": True, "run_id": rid, "run": created_run})

        m_approve = re.fullmatch(r"/runs/([^/]+)/approve", self.path)
        if m_approve:
            try:
                decision_body = json.loads(raw.decode("utf-8") or "{}")
            except Exception:
                return self.send_json(400, {"error": "invalid_json"})
            decision, note = api_approval_decision(decision_body)
            if decision not in ("approve", "reject"):
                return self.send_json(400, {"error": "invalid_decision"})
            rid = m_approve.group(1)
            with STATE_LOCK:
                r = find_run_locked(rid)
                if not r:
                    return self.send_json(404, {"error": "run_not_found"})
                # Só aprova o que ainda espera aprovação: repetir a decisão
                # num run que já seguiu (Running/Success/Failed/Cancelled)
                # seria uma mutação fora de contexto.
                if r.get("status") != "AwaitingApproval":
                    return self.send_json(409, {"error": "not_awaiting_approval"})
                entry = api_apply_approval(r, decision, note)
                save_runs_locked()
            return self.send_json(200, {"ok": True, "decision": entry["decision"]})

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


Handler = _RunnerHTTPRequestHandler


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
"##;

pub fn build_provision_script(
    opts: &ProvisionOptions,
    runner_token: &str,
    webhook_secret: &str,
) -> String {
    let port = if opts.runner_port == 0 { 8787 } else { opts.runner_port };
    let mut s = String::from(
        "set -e\n\
         export PATH=\"$HOME/.cargo/bin:/usr/local/cargo/bin:$PATH\"\n\
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
             fi\n\
             rustc --version && cargo --version\n",
        );
    }

    if opts.install_runner_service {
        // Nunca publica o runner fake do build de teste. Num build normal a
        // lista é vazia e este caminho morre aqui.
        if !MOBILE_HOSTILE_ASSETS.is_empty() {
            return "echo '[erro] runner gerado com payload de teste; recuso instalar na VPS.' >&2; exit 1\n"
                .to_string();
        }
        let runner_py = runner_py_source();
        s.push_str(&format!(
            "echo '=== [5/5] Instalando serviço workploy-runner (systemd) ==='\n\
             $SUDO mkdir -p /opt/workploy /etc/workploy /var/lib/workploy/workspaces /var/lib/workploy/runs\n\
             $SUDO sh -c \"printf '%s' '{runner_token}' > /etc/workploy/token && chmod 600 /etc/workploy/token\"\n\
             $SUDO sh -c \"printf '%s' '{webhook_secret}' > /etc/workploy/webhook_secret && chmod 600 /etc/workploy/webhook_secret\"\n\
             if [ ! -f /etc/workploy/config.json ]; then\n\
               $SUDO sh -c \"printf '{{\\\"bindings\\\":[],\\\"workflows\\\":[],\\\"poll_secs\\\":60}}' > /etc/workploy/config.json\"\n\
             fi\n\
             $SUDO chmod 600 /etc/workploy/config.json\n\
             $SUDO tee /opt/workploy/workploy_runner.py >/dev/null <<'WORKPLOY_RUNNER_EOF'\n\
             {runner_py}\n\
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
             Environment=\"PATH=/root/.cargo/bin:/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\"\n\
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
        let ok = match ssh_run_with_stdin_streaming(
            &server,
            "sh -s",
            script.as_bytes(),
            ssh_idle_timeout(),
            |line| {
                append_provision_log(&sh_log, line);
            },
        ) {
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
    idle_timeout: Duration,
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

    let outcome = stream_until_exit(&mut child, &rx, None, idle_timeout, |line| {
        on_line(line);
    });

    for h in readers {
        let _ = h.join();
    }
    while let Ok(line) = rx.try_recv() {
        on_line(&line);
    }

    match outcome {
        StreamOutcome::Exited(status) => Ok(status.success()),
        StreamOutcome::Cancelled => Ok(false),
        StreamOutcome::WatchFailed => Ok(false),
        StreamOutcome::TimedOut(silent_for) => Err(format!(
            "[tempo esgotado] o provisionamento ficou mudo por {} (nenhuma linha de log e nenhum término) — a sessão SSH foi encerrada. Verifique a rede/a VPS e tente novamente.",
            fmt_dur(silent_for.as_secs_f64())
        )),
    }
}

pub fn merge_remote_runs(existing: &mut Vec<Run>, remote: Vec<Run>, vps_server_id: &str) {
    for r in remote {
        // O espelho carrega o vínculo com o runner da VPS: o id remoto é a
        // própria chave de vinculação e o servidor dono é quem sincronizou.
        // O JSON do runner não traz esses campos — `#[serde(default)]` os
        // entrega como `None` e eles são preenchidos aqui.
        let remote_cancel = r.cancel_requested;
        let remote_approved = r.approved;
        let mut incoming = r;
        let remote_id = incoming.id.clone();
        incoming.vps_run_id = Some(remote_id.clone());
        incoming.vps_server_id = Some(vps_server_id.to_string());
        if let Some(slot) = existing
            .iter_mut()
            .find(|e| e.id == remote_id || e.vps_run_id.as_deref() == Some(remote_id.as_str()))
        {
            // Cancelamento é pedido, não estado consolidado: um pedido já
            // registrado em qualquer um dos lados permanece até o runner
            // confirmar na sincronização seguinte. Sem isso, uma sincronização
            // apagaria o pedido feito no desktop antes do runner processar.
            // Aprovação segue a mesma regra: o desktop marcou `approved` ao
            // enviar o POST ao runner; até o runner consolidar e devolver o
            // estado, a sincronização não pode apagar o flag local — senão o
            // espelho volta a parecer "aguardando aprovação" eternamente.
            let local_id = slot.id.clone();
            let local_cancel = slot.cancel_requested;
            let local_approved = slot.approved;
            *slot = incoming;
            slot.id = local_id;
            slot.cancel_requested = local_cancel || remote_cancel;
            slot.approved = local_approved || remote_approved;
        } else {
            existing.push(incoming);
        }
    }
    existing.sort_by(|a, b| {
        b.started_ts
            .unwrap_or(0)
            .cmp(&a.started_ts.unwrap_or(0))
            .then(b.number.cmp(&a.number))
    });
}

/// Porta HTTP do runner na VPS (`runner_port` 0 = padrão 8787).
pub fn runner_http_port(server: &VpsServer) -> u16 {
    if server.runner_port == 0 {
        8787
    } else {
        server.runner_port
    }
}

/// Id de execução seguro para interpolar num comando ssh (sem shell metachars).
fn sanitize_run_id(run_id: &str) -> String {
    run_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// Monta o comando ssh que faz `POST /runs/{id}/{action}` no runner da VPS
/// via curl local à máquina (o token nunca trafega pelo desktop).
fn build_runner_action_cmd(port: u16, safe_run_id: &str, action: &str) -> String {
    format!(
        "sh -c 'TOKEN=$(cat /etc/workploy/token 2>/dev/null) && curl -fsS -X POST -H \"Authorization: Bearer $TOKEN\" http://127.0.0.1:{port}/runs/{safe_run_id}/{action}'"
    )
}

/// Envia o cancelamento de uma execução ao runner da VPS
/// (`POST /runs/{id}/cancel`). Bloqueante — chame numa thread separada.
/// Marca `cancel_requested` no run remoto; o runner derruba o processo do
/// passo atual em até ~0,2 s e encerra o run como "Cancelled".
/// O resultado é registrado em `runner_sync_status` e, em sucesso, uma
/// sincronização puxa o estado atualizado do runner.
pub fn cancel_vps_run(sh: &SharedHandle, server: &VpsServer, vps_run_id: &str) -> Result<(), String> {
    let safe_rid = sanitize_run_id(vps_run_id);
    let result = if safe_rid.is_empty() {
        Err("Cancelamento na VPS ignorado: id de execução inválido.".into())
    } else {
        let cmd = build_runner_action_cmd(runner_http_port(server), &safe_rid, "cancel");
        match ssh_run_capture(server, &cmd) {
            Ok((true, _, _)) => Ok(()),
            Ok((false, _, stderr)) => Err(format!(
                "Falha ao cancelar a execução na VPS '{}': {}",
                server.name,
                stderr.trim()
            )),
            Err(e) => Err(format!(
                "Falha ao cancelar a execução na VPS '{}': {e}",
                server.name
            )),
        }
    };
    {
        let mut s = sh.lock().unwrap();
        s.runner_sync_status = match &result {
            Ok(()) => format!("Cancelamento enviado ao runner da '{}'.", server.name),
            Err(msg) => msg.clone(),
        };
    }
    engine::notify(sh);
    if result.is_ok() {
        sync_vps_runs(sh, &server.id);
    }
    result
}

pub const UPLOAD_CONFIG_CMD: &str = "sh -c 'if [ \"$(id -u)\" -eq 0 ]; then SUDO=\"\"; else SUDO=\"sudo -n\"; fi; $SUDO mkdir -p /etc/workploy && $SUDO tee /etc/workploy/config.json >/dev/null && $SUDO chmod 600 /etc/workploy/config.json'";

/// Garante que `/opt/workploy/workploy_runner.py` na VPS está na versão atual
/// (capaz de encadear cross-VPS). Se divergir e não houver execuções ativas no
/// runner, grava o script novo e reinicia o serviço `workploy-runner`.
pub fn ensure_runner_script_current(server: &VpsServer) -> Result<bool, String> {
    if !server.runner_installed || server.host.trim().is_empty() {
        return Ok(false);
    }
    let port = runner_http_port(server);
    let check_cmd = format!(
        "sh -c 'if grep -qF {marker} /opt/workploy/workploy_runner.py 2>/dev/null; then echo CURRENT; else TOKEN=$(cat /etc/workploy/token 2>/dev/null); ST=$(curl -fsS -H \"Authorization: Bearer $TOKEN\" http://127.0.0.1:{port}/status 2>/dev/null || true); echo \"OUTDATED:$ST\"; fi'",
        marker = shell_single_quote(RUNNER_SCRIPT_MARKER),
    );
    let (ok, stdout, stderr) = ssh_run_capture(server, &check_cmd)?;
    if !ok {
        return Err(stderr.trim().to_string());
    }
    let trimmed = stdout.trim();
    if trimmed == "CURRENT" {
        return Ok(false);
    }
    if let Some(status_json) = trimmed.strip_prefix("OUTDATED:")
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(status_json)
        && v.get("active_runs").and_then(|x| x.as_u64()).unwrap_or(0) > 0
    {
        return Ok(false);
    }
    let update_cmd = "sh -c 'if [ \"$(id -u)\" -eq 0 ]; then SUDO=\"\"; else SUDO=\"sudo -n\"; fi; $SUDO mkdir -p /opt/workploy && $SUDO tee /opt/workploy/workploy_runner.py >/dev/null && $SUDO chmod 755 /opt/workploy/workploy_runner.py && $SUDO systemctl restart workploy-runner'";
    // Mesma regra do provisionamento: um runner com payload de teste nunca
    // vira o arquivo da VPS.
    if !MOBILE_HOSTILE_ASSETS.is_empty() {
        return Err(format!(
            "runner gerado com payload de teste; recuso atualizar '{}'",
            server.name
        ));
    }
    let runner_py = runner_py_source();
    let (uok, _, uerr) = ssh_run_with_stdin(server, update_cmd, runner_py.as_bytes())?;
    if !uok {
        return Err(format!(
            "Falha ao atualizar workploy-runner em '{}': {}",
            server.name,
            uerr.trim()
        ));
    }
    Ok(true)
}

/// "Testar gatilho do par" a partir do desktop.
///
/// O runner escuta em `127.0.0.1:<porta>` na própria VPS. O teste entra por SSH
/// (a credencial que já existe para aquela VPS), e de lá faz `GET /status`
/// com a chave de pareamento do cofre — exatamente o caminho que o runner de
/// origem usa (`Authorization: Bearer` + `X-Workploy-Peer`). O resultado vira
/// mensagem humanizada em `runner_sync_status` + `sync_last_event`, que a UI
/// transforma no toast padrão. A chave nunca entra na mensagem.
pub fn probe_peer_trigger(sh: &SharedHandle, origin_id: &str, dest_id: &str) {
    let (dest, key) = {
        let s = sh.lock().unwrap();
        let Some(dest) = s.settings.servers.iter().find(|v| v.id == dest_id).cloned() else {
            drop(s);
            finish_probe(sh, false, "A VPS de destino não está mais cadastrada.".into());
            return;
        };
        let key = vault::load_secret(&vault::vps_pair_key(origin_id, dest_id))
            .ok()
            .flatten()
            .filter(|k| !k.trim().is_empty());
        (dest, key)
    };
    if crate::github::peer_http_host(&dest).is_none() {
        finish_probe(
            sh,
            false,
            format!(
                "Informe o endereço (host ou alias SSH) da VPS '{}' para testar o gatilho do par.",
                dest.name
            ),
        );
        return;
    }
    let Some(key) = key else {
        finish_probe(
            sh,
            false,
            format!(
                "Falta a chave de pareamento com '{}' no cofre. O mesmo segredo precisa existir nas duas VPS.",
                dest.name
            ),
        );
        return;
    };

    let sh2 = sh.clone();
    let origin = origin_id.to_string();
    std::thread::spawn(move || {
        let dest_name = dest.name.clone();
        let port = runner_http_port(&dest);
        // Corpo estruturado (JSON, sem shell) com a chave em variável de
        // ambiente do comando remoto: ela não aparece em `ps` nem no log.
        // `sh -c '<script>' -- <chave>`: a chave chega como argumento
        // posicional ($1), não no texto do comando — não aparece em `ps` nem
        // no journal do SSH.
        let remote = format!(
            "sh -c 'K=$1; curl -sS -m 12 -o - -w \"\\nHTTP:%{{http_code}}\" \
             -H \"Authorization: Bearer $K\" -H \"X-Workploy-Peer: {origin}\" \
             http://127.0.0.1:{port}/status' -- {k}",
            k = shell_single_quote(&key),
        );
        let (ok, msg) = match ssh_run_capture(&dest, &remote) {
            Ok((_, stdout, _)) => {
                let (body, code) = split_http_code(&stdout);
                match code {
                    200 => {
                        let allowed = allowed_workflows_from_status(&body);
                        let version = runner_version_from_status(&body);
                        if allowed.is_empty() {
                            (
                                false,
                                format!(
                                    "'{dest_name}' respondeu (runner v{version}), mas não reconhece esta VPS como par. Atualize o runner e sincronize o par."
                                ),
                            )
                        } else {
                            (
                                true,
                                format!(
                                    "'{dest_name}' respondeu (runner v{version}) e aceita este par. Fluxos liberados: {}.",
                                    allowed.join(", ")
                                ),
                            )
                        }
                    }
                    401 => (
                        false,
                        format!(
                            "'{dest_name}' recusou a chave de pareamento (401). Grave o mesmo segredo nas duas VPS."
                        ),
                    ),
                    0 => (
                        false,
                        format!(
                            "'{dest_name}' não tem o runner respondendo na porta {port}. O runner 24/7 está instalado e ativo?"
                        ),
                    ),
                    other => (
                        false,
                        format!("'{dest_name}' respondeu HTTP {other} ao diagnóstico do par."),
                    ),
                }
            }
            Err(e) => (
                false,
                format!(
                    "Não foi possível alcançar '{dest_name}' por SSH: {}",
                    crate::diagnostic::humanize_transport_error(&e)
                ),
            ),
        };
        finish_probe(&sh2, ok, msg);
    });
}

/// Separa o corpo da resposta do `HTTP:<código>` que o `curl -w` anexa.
fn split_http_code(raw: &str) -> (String, u16) {
    let mut body = raw.to_string();
    let mut code = 0u16;
    if let Some(pos) = raw.rfind("HTTP:") {
        code = raw[pos + 5..].trim().parse::<u16>().unwrap_or(0);
        body = raw[..pos].trim_end().to_string();
    }
    (body, code)
}

fn status_value(body: &str, field: &str) -> Option<serde_json::Value> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get(field).cloned())
}

fn allowed_workflows_from_status(body: &str) -> Vec<String> {
    status_value(body, "allowed_workflows")
        .and_then(|v| v.as_array().cloned())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn runner_version_from_status(body: &str) -> u64 {
    status_value(body, "runner_version")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
}

/// Dispara um fluxo na VPS de destino pelo caminho do par.
///
/// Usa o mesmo contrato do runner de origem (`POST /trigger` com
/// `Authorization: Bearer <chave do par>` + `X-Workploy-Peer`), mas executado
/// pela VPS de origem: o desktop entra por SSH e o `curl` sai de lá, então o
/// destino vê o par certo e a regra de allowlist dele vale.
///
/// O token vai como argumento posicional do `sh -c` (não no texto do comando
/// remoto), nunca em log e nunca em JSON de texto puro.
///
/// Devolve a mensagem humanizada de sucesso para o log do run pai.
pub fn trigger_peer_workflow(
    origin: &VpsServer,
    dest: VpsServer,
    wf: &Workflow,
    trigger_label: &str,
    repo: Option<&str>,
    branch: Option<&str>,
) -> Result<String, String> {
    // Quem assina o repasse é a VPS de origem do par — a VPS do fluxo pai.
    // O segredo vem do cofre; se não existir, o repasse não tenta outra
    // credencial: o destino responderia 401 e o erro ficaria confuso. Falta
    // de chave é erro de configuração, não de rede.
    let key = vault::load_secret(&vault::vps_pair_key(&origin.id, &dest.id))
        .ok()
        .flatten()
        .filter(|k| !k.trim().is_empty());
    let Some(key) = key else {
        return Err(format!(
            "nenhuma chave de pareamento com '{}' no cofre",
            dest.name
        ));
    };
    let port = runner_http_port(&dest);
    let payload = serde_json::json!({
        "workflow_id": wf.id,
        "trigger_label": trigger_label,
        "repo": repo.unwrap_or(""),
        "branch": branch.unwrap_or("main"),
        "actor": "workploy-runner",
    });
    let body = serde_json::to_vec(&payload)
        .map_err(|e| format!("não foi possível montar o gatilho: {e}"))?;
    let remote = format!(
        "sh -c 'K=$1; curl -sS -m 20 -o - -w \"\\nHTTP:%{{http_code}}\" -X POST \
         -H \"Authorization: Bearer $K\" -H \"X-Workploy-Peer: {origin}\" \
         -H \"Content-Type: application/json\" --data-binary @- \
         http://127.0.0.1:{port}/trigger' -- {k}",
        origin = origin.id,
        k = shell_single_quote(&key),
    );
    let (ok, stdout, stderr) = ssh_run_with_stdin(&dest, &remote, &body)?;
    if !ok {
        return Err(crate::diagnostic::humanize_transport_error(stderr.trim()));
    }
    let (resp_body, code) = split_http_code(&stdout);
    match code {
        200 => {
            let run_id = status_value(&resp_body, "run_id")
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_else(|| "?".into());
            Ok(format!(
                "fluxo '{}' disparado na VPS '{}' pelo par ({run_id}).",
                wf.name, dest.name
            ))
        }
        403 => Err(format!(
            "'{}' recusou '{}': não está na lista de fluxos liberados para este par",
            dest.name, wf.name
        )),
        404 => Err(format!(
            "'{}' não tem a receita '{}' sincronizada",
            dest.name, wf.name
        )),
        401 => Err(format!(
            "'{}' recusou a chave de pareamento (401)",
            dest.name
        )),
        0 => Err(format!(
            "o runner de '{}' não respondeu na porta {port}",
            dest.name
        )),
        other => Err(format!(
            "'{}' respondeu HTTP {other} ao gatilho",
            dest.name
        )),
    }
}

fn finish_probe(sh: &SharedHandle, ok: bool, msg: String) {
    {
        let mut s = sh.lock().unwrap();
        s.runner_sync_status = msg.clone();
        let seq = s.sync_last_event.as_ref().map(|e| e.0).unwrap_or(0) + 1;
        s.sync_last_event = Some((seq, ok, msg));
    }
    crate::engine::notify(sh);
}

pub fn trigger_vps_run(
    sh: &SharedHandle,
    server: &VpsServer,
    wf: &Workflow,
    trigger_label: &str,
    repo: Option<&str>,
    branch: Option<&str>,
    commit_sha: Option<&str>,
) -> Result<Run, String> {
    let _ = ensure_runner_script_current(server);
    // Garante que /etc/workploy/config.json tenha as receitas e encadeamentos
    // atualizados para que `on_success` continue funcionando 24/7 na VPS.
    let (cfg_opt, gh_tok_opt) = {
        let s = sh.lock().unwrap();
        let cfg = crate::github::build_runner_config_json_with_extras(
            &s.settings,
            &s.workflows,
            &s.workflow_extras,
            &server.id,
        )
        .ok();
        let needs_tok = crate::github::server_needs_github_token(&s.settings, &server.id);
        let tok = if needs_tok {
            if !s.settings.github_token.trim().is_empty() {
                Some(s.settings.github_token.trim().to_string())
            } else {
                vault::load_secret(vault::KEY_GITHUB_PAT).ok().flatten()
            }
        } else {
            None
        };
        (cfg, tok)
    };
    if let Some(cfg_json) = cfg_opt {
        let _ = ssh_run_with_stdin(server, UPLOAD_CONFIG_CMD, cfg_json.as_bytes());
    }
    if let Some(tok) = gh_tok_opt.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        let upload_tok_cmd = "sh -c 'if [ \"$(id -u)\" -eq 0 ]; then SUDO=\"\"; else SUDO=\"sudo -n\"; fi; $SUDO tee /etc/workploy/github_token >/dev/null && $SUDO chmod 600 /etc/workploy/github_token'";
        let _ = ssh_run_with_stdin(server, upload_tok_cmd, tok.as_bytes());
    }

    let port = runner_http_port(server);
    let extras_for_wf = {
        let s = sh.lock().unwrap();
        s.workflow_extras.get(&wf.id).cloned()
    };
    let payload = serde_json::json!({
        "workflow_id": wf.id,
        "workflow": wf,
        "extras": match extras_for_wf {
            Some(e) => serde_json::json!({ wf.id.clone(): e }),
            None => serde_json::json!({}),
        },
        "trigger_label": trigger_label,
        "repo": repo.unwrap_or(""),
        "branch": branch.unwrap_or("main"),
        "commit_sha": commit_sha.unwrap_or(""),
        "actor": "maestro",
    });
    let payload_bytes = serde_json::to_vec(&payload)
        .map_err(|e| format!("Falha ao serializar disparo para VPS: {e}"))?;
    let cmd = format!(
        "sh -c 'TOKEN=$(cat /etc/workploy/token 2>/dev/null) && curl -fsS -X POST -H \"Authorization: Bearer $TOKEN\" -H \"Content-Type: application/json\" --data-binary @- http://127.0.0.1:{port}/trigger'"
    );
    let (ok, stdout, stderr) = ssh_run_with_stdin(server, &cmd, &payload_bytes)?;
    if !ok {
        let detail = crate::diagnostic::humanize_transport_error(stderr.trim());
        return Err(detail);
    }
    let v: serde_json::Value = serde_json::from_str(&stdout)
        .map_err(|e| format!("Resposta inválida do runner em '{}': {e}", server.name))?;
    if !v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false) {
        let err = v.get("error").and_then(|x| x.as_str()).unwrap_or("erro desconhecido");
        return Err(format!("Runner '{}' recusou o disparo: {err}", server.name));
    }
    if let Some(run_val) = v.get("run").filter(|x| x.is_object())
        && let Ok(mut run) = serde_json::from_value::<Run>(run_val.clone())
    {
        run.vps_run_id = Some(run.id.clone());
        run.vps_server_id = Some(server.id.clone());
        return Ok(run);
    }
    let rid = v
        .get("run_id")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "Runner não retornou run_id.".to_string())?;
    Ok(Run {
        id: rid.to_string(),
        workflow_id: wf.id.clone(),
        workflow_name: wf.name.clone(),
        status: RunStatus::Queued,
        trigger_label: trigger_label.to_string(),
        title: trigger_label.to_string(),
        number: 1,
        commit: commit_sha.map(str::to_string),
        started_at: stamp_list(now_ts()),
        started_ts: Some(now_ts()),
        finished_at: None,
        steps: Vec::new(),
        approved: false,
        cancel_requested: false,
        secs: 0.0,
        actor: "workploy-runner".into(),
        repo_name: repo.map(str::to_string),
        branch: branch.unwrap_or("main").to_string(),
        summary: String::new(),
        annotations: Vec::new(),
        vps_run_id: Some(rid.to_string()),
        vps_server_id: Some(server.id.clone()),
    })
}

/// Converte a resposta `/runs` do runner em execuções + métricas de consumo.
/// O runner devolve um array de runs; cada run pode trazer um objeto
/// `metrics` (medido pelo próprio daemon) que o `Run` do Rust ignora.
pub fn parse_runs_with_metrics(
    stdout: &str,
) -> (Vec<Run>, std::collections::HashMap<String, RunMetrics>) {
    let mut runs = Vec::new();
    let mut metrics = std::collections::HashMap::new();
    let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(stdout) else {
        return (runs, metrics);
    };
    for value in values {
        let Ok(run) = serde_json::from_value::<Run>(value.clone()) else {
            continue;
        };
        if let Some(m) = value.get("metrics")
            && let Ok(parsed) = serde_json::from_value::<RunMetrics>(m.clone())
        {
            metrics.insert(run.id.clone(), parsed);
        }
        runs.push(run);
    }
    (runs, metrics)
}

pub fn sync_vps_runs_blocking_silent(sh: &SharedHandle, server: &VpsServer) -> bool {
    let port = runner_http_port(server);
    let cmd = format!(
        "sh -c 'TOKEN=$(cat /etc/workploy/token 2>/dev/null) && curl -fsS -H \"Authorization: Bearer $TOKEN\" http://127.0.0.1:{port}/runs'"
    );
    let Ok((true, stdout, _)) = ssh_run_capture(server, &cmd) else {
        return false;
    };
    let (remote_runs, metrics) = parse_runs_with_metrics(&stdout);
    if remote_runs.is_empty() {
        return false;
    }
    {
        let mut s = sh.lock().unwrap();
        let remote_runs: Vec<Run> = remote_runs
            .into_iter()
            .filter(|r| !s.run_is_hidden(r))
            .collect();
        merge_remote_runs(&mut s.runs, remote_runs, &server.id);
        for (id, m) in metrics {
            s.run_metrics.insert(id, m);
        }
        save_runs(&s);
    }
    engine::notify(sh);
    true
}

pub fn spawn_vps_runs_poller(sh: SharedHandle) {
    std::thread::spawn(move || {
        let mut last_idle_poll = Instant::now()
            .checked_sub(Duration::from_secs(60))
            .unwrap_or_else(Instant::now);
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let (active_servers, idle_servers) = {
                let s = sh.lock().unwrap();
                let mut active_ids: Vec<String> = Vec::new();
                for r in &s.runs {
                    if r.status.active()
                        && r.vps_run_id.is_some()
                        && let Some(sid) = &r.vps_server_id
                        && !active_ids.contains(sid)
                    {
                        active_ids.push(sid.clone());
                    }
                }
                let active: Vec<VpsServer> = s
                    .settings
                    .servers
                    .iter()
                    .filter(|srv| {
                        !srv.host.trim().is_empty()
                            && active_ids.iter().any(|id| id == &srv.id)
                    })
                    .cloned()
                    .collect();
                let idle: Vec<VpsServer> = s
                    .settings
                    .servers
                    .iter()
                    .filter(|srv| {
                        srv.runner_installed
                            && !srv.host.trim().is_empty()
                            && !active_ids.iter().any(|id| id == &srv.id)
                    })
                    .cloned()
                    .collect();
                (active, idle)
            };

            for srv in active_servers {
                let _ = sync_vps_runs_blocking_silent(&sh, &srv);
            }
            if !idle_servers.is_empty() && last_idle_poll.elapsed() >= Duration::from_secs(20) {
                last_idle_poll = Instant::now();
                for srv in idle_servers {
                    let _ = sync_vps_runs_blocking_silent(&sh, &srv);
                }
            }
        }
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
            Ok((true, stdout, _)) => {
                let (remote_runs, metrics) = parse_runs_with_metrics(&stdout);
                let count = remote_runs.len();
                let mut s = sh2.lock().unwrap();
                let remote_runs: Vec<Run> = remote_runs
                    .into_iter()
                    .filter(|r| !s.run_is_hidden(r))
                    .collect();
                merge_remote_runs(&mut s.runs, remote_runs, &server.id);
                for (id, m) in metrics {
                    s.run_metrics.insert(id, m);
                }
                save_runs(&s);
                format!("Sincronizadas {count} execuções de '{}'.", server.name)
            }
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
    let (server, remote_rid) = {
        let s = sh.lock().unwrap();
        let Some(srv) = s.settings.servers.iter().find(|v| v.id == server_id).cloned() else {
            return;
        };
        let remote_id = s
            .runs
            .iter()
            .find(|r| r.id == run_id)
            .and_then(|r| r.vps_run_id.clone())
            .unwrap_or_else(|| run_id.to_string());
        (srv, remote_id)
    };

    let sh2 = sh.clone();
    let rid = run_id.to_string();
    let sid = server_id.to_string();
    std::thread::spawn(move || {
        let action = if approve { "approve" } else { "cancel" };
        let cmd = build_runner_action_cmd(runner_http_port(&server), &sanitize_run_id(&remote_rid), action);
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

#[cfg(test)]
thread_local! {
    static TEST_SSH_CONFIG_PATH: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

fn ssh_config_path() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(custom) = TEST_SSH_CONFIG_PATH.with(|c| c.borrow().clone()) {
        return Some(custom);
    }
    let home = std::env::var_os("USERPROFILE")
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var_os("HOME").filter(|s| !s.is_empty()))
        .map(PathBuf::from)
        .or_else(|| {
            match (
                std::env::var_os("HOMEDRIVE"),
                std::env::var_os("HOMEPATH"),
            ) {
                (Some(d), Some(p)) if !d.is_empty() && !p.is_empty() => {
                    Some(PathBuf::from(d).join(p))
                }
                _ => None,
            }
        })?;
    Some(home.join(".ssh").join("config"))
}

fn parse_ssh_config_hostname(config_text: &str, alias: &str) -> Option<String> {
    let alias = alias.trim();
    if alias.is_empty() {
        return None;
    }
    let mut in_matching_host = false;
    for raw_line in config_text.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let Some(sep_idx) = line.find(|c: char| c.is_whitespace() || c == '=') else {
            continue;
        };
        let keyword = line[..sep_idx].trim();
        let rest = line[sep_idx..]
            .trim_start_matches(|c: char| c.is_whitespace() || c == '=')
            .trim();
        if rest.is_empty() {
            continue;
        }
        if keyword.eq_ignore_ascii_case("Host") {
            in_matching_host = rest
                .split_whitespace()
                .any(|pattern| pattern.trim_matches('"') == alias);
        } else if keyword.eq_ignore_ascii_case("Match") {
            in_matching_host = false;
        } else if in_matching_host
            && keyword.eq_ignore_ascii_case("HostName")
            && let Some(val) = rest.split_whitespace().next()
        {
            let cleaned = val.trim_matches('"').trim();
            if !cleaned.is_empty() {
                return Some(cleaned.to_string());
            }
        }
    }
    None
}

pub fn resolve_ssh_alias_host(alias_or_host: &str) -> Option<String> {
    let target = alias_or_host.trim();
    if target.is_empty() {
        return None;
    }
    if target.contains('.')
        || target.contains(':')
        || target.parse::<std::net::IpAddr>().is_ok()
    {
        return Some(target.to_string());
    }
    let path = ssh_config_path()?;
    let content = std::fs::read_to_string(path).ok()?;
    parse_ssh_config_hostname(&content, target)
}

pub fn webhook_endpoint_and_secret(server: &VpsServer) -> (String, Option<String>, bool) {
    let raw_host = server.host.trim();
    let (host, unresolved_alias) = match resolve_ssh_alias_host(raw_host) {
        Some(resolved) => (resolved, false),
        None => (raw_host.to_string(), !raw_host.is_empty()),
    };
    let port = if server.runner_port == 0 {
        8787
    } else {
        server.runner_port
    };
    let url = format!("http://{host}:{port}/webhook/github");
    let secret = vault::load_secret(&vault::vps_webhook_secret_key(&server.id))
        .ok()
        .flatten()
        .filter(|s| !s.is_empty());
    (url, secret, unresolved_alias)
}

fn begin_vps_purge(s: &mut Shared, server_id: &str) -> Option<VpsServer> {
    if s.purging_server_id.is_some() {
        return None;
    }
    let srv = s.settings.servers.iter().find(|v| v.id == server_id).cloned()?;
    s.purging_server_id = Some(server_id.to_string());
    s.runner_sync_status = format!("Limpando caches e workspaces em '{}'...", srv.name);
    push_provision_log_line(s, &format!("[purge] Iniciando limpeza em '{}'...", srv.name));
    Some(srv)
}

fn parse_purge_response(server_name: &str, stdout: &str) -> (bool, String) {
    match serde_json::from_str::<serde_json::Value>(stdout) {
        Ok(v) => {
            let ok = v.get("ok").and_then(|x| x.as_bool()).unwrap_or(true);
            let freed = v.get("freed_mb").and_then(|x| x.as_f64()).unwrap_or(0.0);
            let detail = v
                .get("message")
                .and_then(|x| x.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if ok {
                let msg = match detail {
                    Some(d) => format!(
                        "Limpeza concluída em '{server_name}': {d} ({freed:.1} MB liberados)"
                    ),
                    None => format!(
                        "Limpeza concluída em '{server_name}': {freed:.1} MB liberados"
                    ),
                };
                (true, msg)
            } else {
                let msg = match detail {
                    Some(d) => format!(
                        "Limpeza falhou em '{server_name}': {d} ({freed:.1} MB liberados)"
                    ),
                    None => format!("Limpeza falhou em '{server_name}'."),
                };
                (false, msg)
            }
        }
        Err(_) => (true, format!("Limpeza concluída em '{server_name}'.")),
    }
}

fn finish_vps_purge(s: &mut Shared, ok: bool, msg: String, log_line: &str) {
    push_provision_log_line(s, log_line);
    s.purging_server_id = None;
    s.runner_sync_status = msg.clone();
    let seq = s.sync_last_event.as_ref().map(|e| e.0).unwrap_or(0) + 1;
    s.sync_last_event = Some((seq, ok, msg));
}

pub fn trigger_vps_purge(sh: &SharedHandle, server_id: &str) {
    let server = {
        let mut s = sh.lock().unwrap();
        let Some(srv) = begin_vps_purge(&mut s, server_id) else {
            return;
        };
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
        let (ok, msg, log_line) = match ssh_run_capture(&server, &cmd) {
            Ok((true, stdout, _)) => {
                let (ok, msg) = parse_purge_response(&server.name, &stdout);
                let log_line = if serde_json::from_str::<serde_json::Value>(&stdout).is_ok() {
                    format!("[purge] {msg}")
                } else if !stdout.trim().is_empty() {
                    format!("[purge] {msg} (resposta: {})", stdout.trim())
                } else {
                    format!("[purge] {msg}")
                };
                (ok, msg, log_line)
            }
            Ok((false, stdout, stderr)) => {
                let err_detail = match (stderr.trim(), stdout.trim()) {
                    ("", "") => "erro desconhecido ao executar POST /purge".to_string(),
                    (e, "") => e.to_string(),
                    ("", o) => o.to_string(),
                    (e, o) => format!("{e} | saída: {o}"),
                };
                let msg = format!(
                    "Falha ao executar limpeza em '{}': {err_detail}",
                    server.name
                );
                let log_line = format!("[purge erro] {msg}");
                (false, msg, log_line)
            }
            Err(e) => {
                let msg = format!("Falha ao executar limpeza em '{}': {e}", server.name);
                let log_line = format!("[purge erro] {msg}");
                (false, msg, log_line)
            }
        };
        {
            let mut s = sh2.lock().unwrap();
            finish_vps_purge(&mut s, ok, msg, &log_line);
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
CPU_PCT=37\r\n\
DISK_USED_PCT=62\r\n\
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
        assert_eq!(res.cpu_usage_pct, 37);
        assert_eq!(res.disk_used_pct, 62);
        assert!(res.has_git);
        assert!(res.has_docker);
        assert!(res.has_node);
        assert!(!res.has_pnpm);
        assert!(res.has_rust);
        assert!(res.has_systemd);
        assert!(res.has_beacon_runner);
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

    /// Regressão do sintoma "limpei o histórico e voltou": o poller reenvia
    /// o histórico do runner a cada ciclo. Uma execução limpa pelo usuário
    /// (tombstone em `hidden_run_keys`) não pode voltar pela sincronização.
    #[test]
    fn cleared_runs_do_not_come_back_through_vps_sync() {
        let mk = |id: &str, status: RunStatus| Run {
            id: id.into(),
            workflow_id: "wf-ci".into(),
            workflow_name: "CI Principal".into(),
            status,
            trigger_label: "push".into(),
            title: "execução".into(),
            number: 1,
            commit: None,
            started_at: String::new(),
            started_ts: Some(10),
            finished_at: Some(String::new()),
            steps: Vec::new(),
            approved: false,
            cancel_requested: false,
            secs: 1.0,
            actor: "maestro".into(),
            repo_name: Some("octocat/acme".into()),
            branch: "main".into(),
            summary: String::new(),
            annotations: Vec::new(),
            vps_run_id: None,
            vps_server_id: None,
        };

        let mut shared = Shared::default();
        shared
            .runs
            .push(mk("run-vps-old", RunStatus::Success));
        shared
            .runs
            .push(mk("run-vps-keep", RunStatus::Success));

        // Usuário limpa: o mais antigo sai e fica marcado como apagado.
        let removed: Vec<Run> = shared.runs.iter().take(1).cloned().collect();
        shared.hide_runs(&removed);
        let hidden = shared.hidden_run_keys.clone();
        shared
            .runs
            .retain(|r| !Shared::run_visibility_keys(r).iter().any(|k| hidden.contains(k)));
        assert_eq!(shared.runs.len(), 1);

        // Próxima sincronização: o runner devolve os dois de novo.
        let remote = vec![mk("run-vps-old", RunStatus::Success), mk("run-vps-keep", RunStatus::Success)];
        let remote: Vec<Run> = remote
            .into_iter()
            .filter(|r| !shared.run_is_hidden(r))
            .collect();
        merge_remote_runs(&mut shared.runs, remote, "vps-a");

        let ids: Vec<&str> = shared.runs.iter().map(|r| r.id.as_str()).collect();
        assert!(
            !ids.contains(&"run-vps-old"),
            "execução limpa voltou pela sincronização: {ids:?}"
        );
        assert!(ids.contains(&"run-vps-keep"), "a execução mantida some: {ids:?}");
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
            repo_name: None,
            branch: "main".into(),
            summary: String::new(),
            annotations: vec![],
            vps_run_id: None,
            vps_server_id: None,
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

        merge_remote_runs(&mut local, remote, "vps-a");
        assert_eq!(local.len(), 2);
        assert_eq!(local[0].id, "run-vps-2");
        assert_eq!(local[1].status, RunStatus::Success);
        // O espelho carrega o vínculo com a VPS após a sincronização.
        assert_eq!(local[0].vps_run_id.as_deref(), Some("run-vps-2"));
        assert_eq!(local[0].vps_server_id.as_deref(), Some("vps-a"));
        assert_eq!(local[1].vps_run_id.as_deref(), Some("run-vps-1"));
    }

    fn run_for_merge(id: &str, status: RunStatus, cancel_requested: bool) -> Run {
        Run {
            id: id.into(),
            workflow_id: "wf-1".into(),
            workflow_name: "CI".into(),
            status,
            trigger_label: "webhook".into(),
            title: "run".into(),
            number: 1,
            commit: None,
            started_at: "18/11/2026 10:00".into(),
            started_ts: Some(100),
            finished_at: None,
            steps: vec![],
            approved: false,
            cancel_requested,
            secs: 0.0,
            actor: "workploy-runner".into(),
            repo_name: None,
            branch: "main".into(),
            summary: String::new(),
            annotations: vec![],
            vps_run_id: None,
            vps_server_id: None,
        }
    }

    #[test]
    fn merge_remote_runs_propagates_cancel_and_keeps_local_request() {
        // Desktop pediu cancelamento; o runner ainda não processou (false).
        let mut local = vec![run_for_merge("run-vps-1", RunStatus::Running, true)];
        let remote = vec![run_for_merge("run-vps-1", RunStatus::Running, false)];
        merge_remote_runs(&mut local, remote, "vps-a");
        assert!(
            local[0].cancel_requested,
            "pedido de cancelamento local não pode ser apagado pela sincronização"
        );
        assert_eq!(local[0].vps_run_id.as_deref(), Some("run-vps-1"));
        assert_eq!(local[0].vps_server_id.as_deref(), Some("vps-a"));

        // VPS → desktop: cancelamento confirmado no runner chega ao espelho.
        let remote_cancelled = vec![run_for_merge("run-vps-2", RunStatus::Cancelled, true)];
        merge_remote_runs(&mut local, remote_cancelled, "vps-a");
        let mirror = local.iter().find(|r| r.id == "run-vps-2").unwrap();
        assert!(mirror.cancel_requested);
        assert_eq!(mirror.status, RunStatus::Cancelled);
        assert_eq!(mirror.vps_run_id.as_deref(), Some("run-vps-2"));
        assert_eq!(mirror.vps_server_id.as_deref(), Some("vps-a"));
    }

    #[test]
    fn merge_remote_runs_keeps_local_approval_until_remote_confirms() {
        // Desktop aprovou (POST /approve enviado ao runner); o runner ainda
        // não consolidou o estado no runs.json dele. A sincronização periódica
        // não pode apagar o flag local — senão o espelho volta a parecer
        // "aguardando aprovação" enquanto o pedido já viajou.
        let mut local = vec![run_for_merge("run-vps-1", RunStatus::AwaitingApproval, false)];
        local[0].approved = true;
        let mut remote = run_for_merge("run-vps-1", RunStatus::AwaitingApproval, false);
        remote.approved = false;
        merge_remote_runs(&mut local, vec![remote], "vps-a");
        assert!(
            local[0].approved,
            "aprovação local não pode ser apagada pela sincronização"
        );

        // Caminho inverso: runner consolidou a aprovação — chega ao espelho
        // recém-sincronizado.
        let mut fresh: Vec<Run> = Vec::new();
        let mut remote_approved = run_for_merge("run-vps-2", RunStatus::Running, false);
        remote_approved.approved = true;
        merge_remote_runs(&mut fresh, vec![remote_approved], "vps-a");
        assert!(fresh[0].approved, "aprovação consolidada no runner chega ao espelho");
    }

    #[test]
    fn merge_remote_runs_matches_by_vps_run_id_and_preserves_local_id() {
        let mut local = vec![Run {
            id: "run-local-placeholder".into(),
            vps_run_id: Some("run-vps-99".into()),
            vps_server_id: Some("vps-a".into()),
            ..run_for_merge("run-local-placeholder", RunStatus::Queued, false)
        }];
        let mut remote = run_for_merge("run-vps-99", RunStatus::Running, false);
        remote.title = "Executando na VPS".into();
        merge_remote_runs(&mut local, vec![remote], "vps-a");
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].id, "run-local-placeholder");
        assert_eq!(local[0].vps_run_id.as_deref(), Some("run-vps-99"));
        assert_eq!(local[0].status, RunStatus::Running);
        assert_eq!(local[0].title, "Executando na VPS");
    }

    #[test]
    fn build_ssh_command_adds_keepalives_for_all_auth_modes() {
        // Sem ServerAlive explícito, uma conexão que cai silenciosamente
        // (NAT/reboot/troca de rede) deixa o ssh pendurado para sempre.
        let key_path =
            std::env::temp_dir().join(format!("workploy-ssh-key-{}", crate::model::new_id("k")));
        std::fs::write(&key_path, b"chave-de-teste").unwrap();

        let alias_srv = VpsServer {
            host: "aurora-vps".into(),
            auth: AuthMethod::SshConfigAlias,
            ..Default::default()
        };

        let key_srv = VpsServer {
            host: "203.0.113.5".into(),
            auth: AuthMethod::PrivateKey {
                path: key_path.to_string_lossy().into(),
            },
            ..Default::default()
        };

        for srv in [&alias_srv, &key_srv] {
            let (cmd, _guard) = build_ssh_command(srv, "echo ok").unwrap();
            let args: Vec<String> = cmd
                .get_args()
                .map(|a| a.to_string_lossy().to_string())
                .collect();
            let joined = args.join(" ");
            assert!(
                joined.contains("ServerAliveInterval=15"),
                "keepalive ausente para {:?}: {joined}",
                srv.auth
            );
            assert!(
                joined.contains("ServerAliveCountMax=4"),
                "keepalive count ausente para {:?}: {joined}",
                srv.auth
            );
        }
        let _ = std::fs::remove_file(&key_path);
    }

    fn silent_child() -> std::process::Child {
        // Processo que não emite NADA e viveria minutos: o mesmo estado de
        // um ssh cuja conexão morreu no meio do passo.
        let mut c = if cfg!(windows) {
            let mut c = Command::new("ping");
            c.arg("-n").arg("30").arg("127.0.0.1");
            c
        } else {
            let mut c = Command::new("sleep");
            c.arg("30");
            c
        };
        engine::apply_no_window(&mut c);
        c.stdout(Stdio::piped()).stderr(Stdio::piped());
        c.spawn().expect("child mudo deve subir")
    }

    fn chatty_then_silent_child() -> std::process::Child {
        // Emite duas linhas com pausa curtas entre elas e depois silencia
        // para sempre: linha nova reinicia o cronômetro de inatividade.
        let mut c = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.arg("/C")
                .arg("echo linha-um& ping -n 3 127.0.0.1 > nul & echo linha-dois& ping -n 30 127.0.0.1 > nul");
            c
        } else {
            let mut c = Command::new("sh");
            c.arg("-c").arg("echo linha-um; sleep 2; echo linha-dois; sleep 30");
            c
        };
        engine::apply_no_window(&mut c);
        c.stdout(Stdio::piped()).stderr(Stdio::piped());
        c.spawn().expect("child falante deve subir")
    }

    fn echo_child() -> std::process::Child {
        let mut c = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.arg("/C").arg("echo linha-ok");
            c
        } else {
            let mut c = Command::new("sh");
            c.arg("-c").arg("echo linha-ok");
            c
        };
        engine::apply_no_window(&mut c);
        c.stdout(Stdio::piped()).stderr(Stdio::piped());
        c.spawn().expect("child de echo deve subir")
    }

    fn forward_stdout_to_channel(child: &mut std::process::Child) -> std::sync::mpsc::Receiver<String> {
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        if let Some(out) = child.stdout.take() {
            std::thread::spawn(move || {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    let _ = tx.send(line);
                }
            });
        }
        rx
    }

    #[test]
    fn stream_until_exit_kills_silent_hang_after_idle_timeout() {
        // Antes do timeout de inatividade, um ssh mudo (rede caída no meio do
        // passo) deixava o loop de streaming rodando para sempre — passo 'em
        // andamento' eterno, sem linha nova de log. O timeout o mata.
        let mut child = silent_child();
        // Canal vazio = conexão morta: nada chega, processo nunca termina.
        let (tx, rx) = std::sync::mpsc::channel::<String>();
        drop(tx);
        let started = Instant::now();
        let lines = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = lines.clone();
        let outcome = stream_until_exit(
            &mut child,
            &rx,
            None,
            Duration::from_millis(400),
            move |_l| {
                seen.fetch_add(1, Ordering::Relaxed);
            },
        );
        assert!(
            matches!(outcome, StreamOutcome::TimedOut(_)),
            "child mudo deve ser abortado por inatividade, veio {outcome:?}"
        );
        assert_eq!(
            lines.load(Ordering::Relaxed),
            0,
            "nenhuma linha deveria ter chegado"
        );
        assert!(
            child.try_wait().unwrap().is_some(),
            "o child precisa estar morto após o timeout"
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "o abort não pode esperar o child inteiro (30 s)"
        );
    }

    #[test]
    fn stream_until_exit_survives_lines_flow_and_exits_normally() {
        // Linha nova reinicia o cronômetro: pausas curtas entre linhas não
        // matam a sessão; silêncio longo depois da última linha, sim.
        let mut child = chatty_then_silent_child();
        let rx = forward_stdout_to_channel(&mut child);
        let lines = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = lines.clone();
        let started = Instant::now();
        let outcome = stream_until_exit(
            &mut child,
            &rx,
            None,
            Duration::from_millis(5000),
            move |_l| {
                seen.fetch_add(1, Ordering::Relaxed);
            },
        );
        assert_eq!(
            lines.load(Ordering::Relaxed),
            2,
            "as duas linhas devem fluir antes do corte"
        );
        assert!(matches!(outcome, StreamOutcome::TimedOut(_)));
        assert!(
            started.elapsed() >= Duration::from_secs(4),
            "deve sobreviver à pausa de ~2 s entre linhas e só expirar após o silêncio de 5 s"
        );

        // Término natural continua funcionando: nada de timeout em exit limpo.
        let mut child = echo_child();
        let rx = forward_stdout_to_channel(&mut child);
        let mut got = Vec::new();
        let outcome = stream_until_exit(&mut child, &rx, None, Duration::from_secs(60), |l| {
            got.push(l.to_string());
        });
        match outcome {
            StreamOutcome::Exited(status) => assert!(status.success()),
            other => panic!("echo deve terminar normalmente, veio {other:?}"),
        }
        assert!(got.iter().any(|l| l.contains("linha-ok")));
    }

    #[test]
    fn cancel_vps_run_builds_expected_curl_command_with_sanitized_id() {
        let mut srv = VpsServer::default();
        assert_eq!(runner_http_port(&srv), 8787);
        srv.runner_port = 9090;
        assert_eq!(runner_http_port(&srv), 9090);

        let dirty = sanitize_run_id("run-vps-1; rm -rf /");
        assert_eq!(dirty, "run-vps-1rm-rf");
        assert!(!dirty.contains(';'));
        assert!(!dirty.contains(char::is_whitespace));

        let cmd = build_runner_action_cmd(8787, &sanitize_run_id("run-vps-abc_1"), "cancel");
        assert!(cmd.contains("curl -fsS -X POST"));
        assert!(cmd.contains("-H \"Authorization: Bearer $TOKEN\""));
        assert!(cmd.contains("$(cat /etc/workploy/token"));
        assert!(cmd.contains("http://127.0.0.1:8787/runs/run-vps-abc_1/cancel"));

        // Mesma montagem usada pela aprovação — só a ação muda.
        let approve_cmd = build_runner_action_cmd(8787, "run-vps-abc_1", "approve");
        assert!(approve_cmd.contains("http://127.0.0.1:8787/runs/run-vps-abc_1/approve"));
    }

    #[test]
    fn provision_script_includes_systemd_and_port() {
        if !MOBILE_HOSTILE_ASSETS.is_empty() {
            // Build de teste (WORKPLOY_TEST_BUILD=1): o provisionamento recusa
            // publicar o runner fake, por design. O conteúdo real é coberto
            // pelo build normal e pelos testes de injeção/HTTP.
            eprintln!("[skip] build de teste: provisionamento recusa o runner fake");
            return;
        }
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
        assert!(script.contains("def branch_matches("));
        assert!(script.contains("def resolve_workflow_for_binding("));
        assert!(script.contains("::workploy-substep::"));
        assert!(script.contains("def _finalize_running_substep("));
        assert!(script.contains("\"repo_name\": repo_full_name or None"));
        assert!(script.contains("start_new_session=True"));
        assert!(script.contains("os.killpg(os.getpgid(proc.pid), signal.SIGTERM)"));
        assert!(script.contains("os.killpg(os.getpgid(proc.pid), signal.SIGKILL)"));
        // Guarda: o runner gerado nao pode carregar nome de projeto privado.
        assert!(!script.contains("acme-erp"));
        assert!(!script.contains("docker.io"));

        assert!(PROBE_REMOTE_SCRIPT.contains("/opt/workploy/workploy_runner.py"));
        assert!(PROBE_REMOTE_SCRIPT.contains("workploy-runner"));
        assert!(PROBE_REMOTE_SCRIPT.contains("CPU_PCT"));
        assert!(PROBE_REMOTE_SCRIPT.contains("DISK_USED_PCT"));
        assert!(PROBE_REMOTE_SCRIPT.contains("export PATH=\"$HOME/.cargo/bin:/usr/local/cargo/bin:$PATH\""));
        assert!(!PROBE_REMOTE_SCRIPT.contains("acme-erp"));
        assert!(script.contains("export PATH=\"$HOME/.cargo/bin:/usr/local/cargo/bin:$PATH\""));
        assert!(script.contains(
            "Environment=\"PATH=/root/.cargo/bin:/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\""
        ));
    }

    #[test]
    fn provision_script_verifies_rust_installation_without_silencing_errors() {
        let opts = ProvisionOptions {
            install_git: false,
            install_docker: false,
            install_node: false,
            install_rust: true,
            install_runner_service: false,
            runner_port: 8787,
        };
        let script = build_provision_script(&opts, "tok123", "whsec456");
        assert!(script.starts_with("set -e\nexport PATH=\"$HOME/.cargo/bin:/usr/local/cargo/bin:$PATH\""));
        assert!(script.contains("https://sh.rustup.rs"));
        assert!(script.contains("rustc --version && cargo --version"));
        assert!(!script.contains("cargo --version || true"));
    }

    struct TempSshConfigGuard {
        path: PathBuf,
    }

    impl TempSshConfigGuard {
        fn new(contents: &str) -> Self {
            let path = std::env::temp_dir().join(format!("workploy-test-ssh-cfg-{}", new_id("cfg")));
            std::fs::write(&path, contents).unwrap();
            TEST_SSH_CONFIG_PATH.with(|c| *c.borrow_mut() = Some(path.clone()));
            Self { path }
        }
    }

    impl Drop for TempSshConfigGuard {
        fn drop(&mut self) {
            TEST_SSH_CONFIG_PATH.with(|c| *c.borrow_mut() = None);
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[test]
    fn resolve_ssh_alias_host_handles_direct_hosts_and_ssh_config_aliases() {
        let _cfg = TempSshConfigGuard::new(
            "\
# Comentário global
Host *
    ServerAliveInterval 30

Host aurora-vps
    HostName 203.0.113.10
    User root
    Port 22

Host staging-box prod-alias
    HostName = 198.51.100.77
    User deploy
",
        );

        // Domínio direto
        assert_eq!(
            resolve_ssh_alias_host("ci.workploy.com.br"),
            Some("ci.workploy.com.br".into())
        );
        // IPv4 direto
        assert_eq!(
            resolve_ssh_alias_host("203.0.113.10"),
            Some("203.0.113.10".into())
        );
        // IPv6 com ':'
        assert_eq!(
            resolve_ssh_alias_host("2001:db8::1"),
            Some("2001:db8::1".into())
        );
        // Alias resolvido via bloco Host exato
        assert_eq!(
            resolve_ssh_alias_host("aurora-vps"),
            Some("203.0.113.10".into())
        );
        // Alias resolvido em lista separada por espaço
        assert_eq!(
            resolve_ssh_alias_host("prod-alias"),
            Some("198.51.100.77".into())
        );
        // Alias ausente -> None
        assert_eq!(resolve_ssh_alias_host("alias-inexistente"), None);
    }

    #[test]
    fn webhook_endpoint_and_secret_formats_url_with_port_fallback_and_reads_vault() {
        let _guard = vault::test_lock();
        let _cfg = TempSshConfigGuard::new(
            "\
Host aurora-vps
    HostName 203.0.113.10
    User root
",
        );
        let mut srv = VpsServer {
            id: "vps-webhook-helper-test".into(),
            name: "Prod".into(),
            host: " 203.0.113.42 ".into(),
            port: 22,
            user: "deploy".into(),
            auth: AuthMethod::SshConfigAlias,
            last_probe: None,
            runner_installed: true,
            runner_port: 0,
            webhook_secret_key: None,
        };
        let key = vault::vps_webhook_secret_key(&srv.id);
        let _ = vault::delete_secret(&key);

        let (url_default, sec_none, unresolved_default) = webhook_endpoint_and_secret(&srv);
        assert_eq!(url_default, "http://203.0.113.42:8787/webhook/github");
        assert_eq!(sec_none, None);
        assert!(!unresolved_default);

        srv.runner_port = 9095;
        vault::save_secret(&key, "whsec-test-123").unwrap();
        let (url_custom, sec_some, unresolved_custom) = webhook_endpoint_and_secret(&srv);
        assert_eq!(url_custom, "http://203.0.113.42:9095/webhook/github");
        assert_eq!(sec_some, Some("whsec-test-123".into()));
        assert!(!unresolved_custom);

        // Alias resolvido do ~/.ssh/config deve usar o HostName real e unresolved = false
        srv.host = "aurora-vps".into();
        srv.runner_port = 8787;
        let (url_alias_ok, _, unresolved_alias_ok) = webhook_endpoint_and_secret(&srv);
        assert_eq!(url_alias_ok, "http://203.0.113.10:8787/webhook/github");
        assert!(!unresolved_alias_ok);

        // Alias ausente no ~/.ssh/config faz fallback para o texto original e sinaliza unresolved = true
        srv.host = "alias-sem-config".into();
        let (url_alias_missing, _, unresolved_alias_missing) = webhook_endpoint_and_secret(&srv);
        assert_eq!(
            url_alias_missing,
            "http://alias-sem-config:8787/webhook/github"
        );
        assert!(unresolved_alias_missing);

        let _ = vault::delete_secret(&key);
    }

    #[test]
    fn purge_feedback_updates_state_logs_and_monotonic_events() {
        let srv = VpsServer {
            id: "vps-purge-1".into(),
            name: "Produção".into(),
            host: "203.0.113.50".into(),
            port: 22,
            user: "deploy".into(),
            auth: AuthMethod::SshConfigAlias,
            last_probe: None,
            runner_installed: true,
            runner_port: 8787,
            webhook_secret_key: None,
        };
        let mut s = Shared::default();
        s.settings.servers.push(srv);
        s.sync_last_event = Some((4, true, "evento anterior".into()));

        assert!(begin_vps_purge(&mut s, "vps-purge-1").is_some());
        assert_eq!(s.purging_server_id.as_deref(), Some("vps-purge-1"));
        assert_eq!(
            s.runner_sync_status,
            "Limpando caches e workspaces em 'Produção'..."
        );
        assert!(s
            .provision_log
            .contains("[purge] Iniciando limpeza em 'Produção'..."));

        // Segunda chamada concorrente deve ser ignorada enquanto purging_server_id está preenchido
        assert!(begin_vps_purge(&mut s, "vps-purge-1").is_none());

        let (ok, ok_msg) = parse_purge_response(
            "Produção",
            r#"{"ok": true, "freed_mb": 420.5, "message": "workspaces: 1 removidos, 2 caches limpos"}"#,
        );
        assert!(ok);
        assert_eq!(
            ok_msg,
            "Limpeza concluída em 'Produção': workspaces: 1 removidos, 2 caches limpos (420.5 MB liberados)"
        );

        finish_vps_purge(&mut s, ok, ok_msg.clone(), &format!("[purge] {ok_msg}"));
        assert_eq!(s.purging_server_id, None);
        assert_eq!(s.runner_sync_status, ok_msg);
        assert_eq!(s.sync_last_event, Some((5, true, ok_msg.clone())));
        assert!(s.provision_log.contains(&format!("[purge] {ok_msg}")));

        let (fail_ok, fail_msg) = parse_purge_response(
            "Produção",
            r#"{"ok": false, "freed_mb": 0.0, "message": "docker daemon offline"}"#,
        );
        assert!(!fail_ok);
        assert_eq!(
            fail_msg,
            "Limpeza falhou em 'Produção': docker daemon offline (0.0 MB liberados)"
        );

        finish_vps_purge(
            &mut s,
            false,
            fail_msg.clone(),
            &format!("[purge erro] {fail_msg}"),
        );
        assert_eq!(s.purging_server_id, None);
        assert_eq!(s.sync_last_event, Some((6, false, fail_msg.clone())));
        assert!(s.provision_log.contains(&format!("[purge erro] {fail_msg}")));
    }

    #[test]
    fn runner_script_supports_cross_vps_forwarding_and_peer_auth() {
        let py = WORKPLOY_RUNNER_PY;
        // Versão: o desktop compara este marcador antes de reescrever o script.
        assert!(py.contains(RUNNER_SCRIPT_MARKER));
        assert!(py.contains("WORKPLOY_RUNNER_VERSION = \"3\""));
        // Saída cross-VPS: o runner encaminha o encadeamento para a VPS destino.
        assert!(py.contains("def _dispatch_remote_chain("));
        assert!(py.contains("remote_chains"));
        assert!(py.contains("self_server_id"));
        assert!(py.contains("\"X-Workploy-Peer\": self_sid"));
        assert!(py.contains("\"Authorization\": f\"Bearer {key}\""));
        // Malha SSH removida: o repasse é só por HTTP no runner do par.
        assert!(!py.contains("def _dispatch_via_ssh("));
        assert!(!py.contains("ssh_cfg.get(\"helper\")"));
        assert!(py.contains("http://{host}:{port}/trigger"));
        // Entrada cross-VPS: par autenticado só dispara fluxo da allowlist.
        assert!(py.contains("def check_peer(self, cfg):"));
        assert!(py.contains("peer_keys"));
        assert!(py.contains("hmac.compare_digest(got, expected)"));
        assert!(py.contains("workflow_not_allowed"));
        // O par autenticado nunca injeta comando inline.
        assert!(py.contains("inline_wf = None"));
        // Config com permissão restrita.
        assert!(UPLOAD_CONFIG_CMD.contains("chmod 600 /etc/workploy/config.json"));
    }

    /// O repasse cross-VPS fala os dois vocabulários do arquivo de config:
    /// `peers` (grafo SSH, `peer_id`/`pair_key`) e o legado (`server_id`/`key`).
    #[test]
    fn runner_script_dispatches_outbound_peer_with_both_config_vocabularies() {
        let py = WORKPLOY_RUNNER_PY;
        // Seleção do par por id, aceitando os dois nomes de campo.
        assert!(py.contains("def _find_peer(cfg, target_sid, transport):"));
        assert!(py.contains("p.get(\"peer_id\") or p.get(\"server_id\")"));
        assert!(py.contains("def _peer_field(peer, primary, legacy, default=\"\"):"));
        assert!(py.contains("_peer_field(peer, \"pair_key\", \"key\")"));
        // Destino vem da tabela do desktop (resolvida com o vínculo) e cai no
        // mapa de cabos quando ela não existe.
        assert!(py.contains("chain_targets = cfg.get(\"chain_targets\") or {}"));
        assert!(py.contains("target_sid = (chain_targets.get(next_id) or \"\").strip()"));
        // Saída HTTP autenticada como par.
        assert!(py.contains("\"X-Workploy-Peer\": self_sid"));
        assert!(py.contains("http://{host}:{port}/trigger"));
        // 403 do destino vira mensagem acionável (allowlist/chave), não código.
        assert!(py.contains("he.code == 403"));
        assert!(py.contains("lista de fluxos liberados"));
        // Recebimento: allowlist do par força a lista e ignora workflow inline.
        assert!(py.contains("peer_entry.get(\"allowed_workflows\")"));
        assert!(py.contains("if wf_id not in allowed:"));
        assert!(py.contains("return self.send_json(403, {\"error\": \"workflow_not_allowed\"})"));
        // `/status` autentica por par (diagnóstico) mas não expõe runs.
        assert!(py.contains("peer = self.check_peer(cfg)"));
        assert!(py.contains("out[\"allowed_workflows\"] = allowed"));
    }

    #[test]
    fn parse_http_code_from_curl_write_out() {
        let (body, code) = super::split_http_code("{\"ok\": true}\nHTTP:200");
        assert_eq!(code, 200);
        assert_eq!(body, "{\"ok\": true}");
        let (body, code) = super::split_http_code("");
        assert_eq!(code, 0);
        assert_eq!(body, "");
        let (body, code) = super::split_http_code("{\"error\":\"unauthorized\"}\nHTTP:401\n");
        assert_eq!(code, 401);
        assert!(body.contains("unauthorized"));
    }

    // ---------------------------------------------------------------------
    // Rotas de leitura do runner: `/run/<id>`, `/run/<id>/log`, `?limit=`.
    //
    // A lógica (offset/tail, limite da lista) é testada de verdade: o script
    // Python gerado é escrito num arquivo temporário e executado com
    // `importlib`, chamando as funções puras. Se faltar Python na máquina,
    // o teste pula com aviso explícito em vez de fingir cobertura.
    // ---------------------------------------------------------------------

    /// Escreve o runner gerado num `.py` temporário e roda `body` no Python
    /// local (`WORKPLOY_TEST_PYTHON` para forçar o interpretador). Devolve o
    /// stdout ou `None` quando não há interpretador disponível.
    fn run_generated_runner_python(body: &str) -> Option<String> {
        run_python_source_script(&runner_py_source(), body)
    }

    /// Mesmo caminho do teste acima, mas com um script arbitrário (para provar
    /// a injeção hostil sem passar pelo build de teste).
    fn run_python_source_script(script_source: &str, body: &str) -> Option<String> {
        use std::process::{Command, Stdio};

        let dir = std::env::temp_dir().join(format!(
            "workploy-runner-py-{}",
            crate::model::new_id("t")
        ));
        std::fs::create_dir_all(&dir).ok()?;
        let script = dir.join("workploy_runner_probe.py");
        std::fs::write(&script, script_source).ok()?;

        let driver = format!(
            "import importlib.util, json, sys\n\
             from urllib.parse import parse_qs\n\
             __file__ = {path:?}\n\
             spec = importlib.util.spec_from_file_location('wr_probe', __file__)\n\
             mod = importlib.util.module_from_spec(spec)\n\
             spec.loader.exec_module(mod)\n\
             {body}\n",
            path = script.to_string_lossy()
        );

        let candidates: Vec<String> = std::env::var("WORKPLOY_TEST_PYTHON")
            .ok()
            .filter(|v| !v.trim().is_empty())
            .into_iter()
            .chain(["python3", "python", "py"].iter().map(|s| s.to_string()))
            .collect();

        for exe in candidates {
            let mut cmd = Command::new(&exe);
            // `py` no Windows é o launcher e não lê o script do stdin.
            if exe == "py" {
                cmd.arg("-3");
            }
            let child = match cmd
                .arg("-c")
                .arg(&driver)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
            {
                Ok(c) => c,
                Err(_) => continue,
            };
            let out = match child.wait_with_output() {
                Ok(o) => o,
                Err(_) => continue,
            };
            if !out.status.success() {
                let err = String::from_utf8_lossy(&out.stderr);
                panic!("runner.py no Python '{exe}' falhou: {err}");
            }
            let _ = std::fs::remove_dir_all(&dir);
            return Some(String::from_utf8_lossy(&out.stdout).into_owned());
        }
        let _ = std::fs::remove_dir_all(&dir);
        None
    }

    #[test]
    fn runner_script_has_single_run_and_run_log_read_routes() {
        let py = WORKPLOY_RUNNER_PY;
        // Rotas novas, admin-only, dentro do `do_GET`.
        assert!(py.contains("m_run = re.fullmatch(r\"/run/([A-Za-z0-9._:-]+)\", parsed.path)"));
        assert!(py.contains("m_log = re.fullmatch(r\"/run/([A-Za-z0-9._:-]+)/log\", parsed.path)"));
        assert!(py.contains("def api_run_log_payload(run, query):"));
        assert!(py.contains("def api_runs_limit(query):"));
        // Autorização: o 401 do par continua antes de qualquer leitura de run.
        let get_body = py
            .split("    def do_GET(self):")
            .nth(1)
            .expect("do_GET presente")
            .split("\n    def do_POST(self):")
            .next()
            .unwrap();
        let guard = get_body
            .find("return self.send_json(401, {\"error\": \"unauthorized\"})")
            .expect("guard de auth no do_GET");
        let status = get_body.find("if self.path == \"/status\":").expect("/status no do_GET");
        let runs = get_body.find("if parsed.path == \"/runs\":").expect("/runs no do_GET");
        let log = get_body.find("m_log = re.fullmatch(").expect("/run/<id>/log no do_GET");
        assert!(guard < status && status < runs && runs < log, "ordem do do_GET mudou");
        // Campos espelham o schema Rust (`Run` / `StepRun`).
        for field in ["\"run_id\"", "\"status\"", "\"steps\"", "\"name\"", "\"bytes\"", "\"lines\"", "\"log\""] {
            assert!(py.contains(field), "campo {field} sumiu do payload do log");
        }
        // `?limit=` na lista: teto 100 e padrão 50.
        assert!(py.contains("return max(1, min(100, n))"));
        assert!(py.contains("n = _int_query(query, \"limit\", 50)"));
        // Nenhum erro devolve stack trace: `send_json` só serializa o dict.
        assert!(!get_body.contains("traceback"));
    }

    #[test]
    fn runner_log_route_slices_by_offset_and_tail() {
        let body = r#"
import json
run = {
    "id": "run-vps-abc-1",
    "status": "Running",
    "steps": [
        {"name": "build", "status": "Success", "log": "abcdefghij"},
        {"name": "test", "status": "Running", "log": "0123456789"},
    ],
}
cases = {
    "full": mod.api_run_log_payload(run, parse_qs("")),
    "offset": mod.api_run_log_payload(run, parse_qs("offset=4&tail=6")),
    "past_end": mod.api_run_log_payload(run, parse_qs("offset=999")),
    "at_end": mod.api_run_log_payload(run, parse_qs("offset=20")),
    "step1": mod.api_run_log_payload(run, parse_qs("step=1&offset=2&tail=3")),
    "step_oob": mod.api_run_log_payload(run, parse_qs("step=7")),
    "bad_int": mod.api_run_log_payload(run, parse_qs("offset=abc&tail=-5")),
    "tail_cap": mod.api_run_log_payload(
        {"id": "r", "status": "Success", "steps": [{"name": "s", "status": "Success", "log": "x" * 10}]},
        parse_qs("offset=0&tail=999999999"),
    ),
    "limit_default": {"n": mod.api_runs_limit(parse_qs(""))},
    "limit_hi": {"n": mod.api_runs_limit(parse_qs("limit=500"))},
    "limit_lo": {"n": mod.api_runs_limit(parse_qs("limit=0"))},
    "limit_bad": {"n": mod.api_runs_limit(parse_qs("limit=xyz"))},
}
print(json.dumps(cases))
"#;
        let Some(stdout) = run_generated_runner_python(body) else {
            eprintln!("[skip] sem interpretador Python: teste real de offset/tail não rodou");
            return;
        };
        let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("JSON de probe");

        let full = &v["full"];
        assert_eq!(full["run_id"], "run-vps-abc-1");
        assert_eq!(full["status"], "Running");
        assert_eq!(full["log"], "abcdefghij0123456789");
        assert_eq!(full["size"], 20);
        assert_eq!(full["truncated"], false);
        assert_eq!(full["steps"][0]["name"], "build");
        assert_eq!(full["steps"][0]["bytes"], 10);
        assert_eq!(full["steps"][1]["status"], "Running");
        assert_eq!(full["steps"][1]["lines"], 0);

        // offset + tail recortam do log concatenado. `truncated` aqui é
        // verdadeiro porque o teto de 6 bytes deixou os últimos 10 para trás —
        // é o que o celular usa para saber que ainda falta buscar.
        assert_eq!(v["offset"]["log"], "efghij");
        assert_eq!(v["offset"]["offset"], 4);
        assert_eq!(v["offset"]["size"], 20);
        assert_eq!(v["offset"]["truncated"], true);
        // Pedir além do fim: vazio + truncated, nunca erro.
        assert_eq!(v["past_end"]["log"], "");
        assert_eq!(v["past_end"]["truncated"], true);
        // Exatamente no fim: vazio, sem truncar (offset == tamanho).
        assert_eq!(v["at_end"]["log"], "");
        assert_eq!(v["at_end"]["truncated"], false);
        // Um passo só: log e tamanho do passo escolhido.
        assert_eq!(v["step1"]["log"], "234");
        assert_eq!(v["step1"]["size"], 10);
        assert_eq!(v["step1"]["steps"].as_array().unwrap().len(), 2);
        // Índice fora da faixa: nenhum passo selecionado, log vazio.
        assert_eq!(v["step_oob"]["log"], "");
        assert_eq!(v["step_oob"]["size"], 0);
        // Query inválida cai no padrão (offset 0, tail padrão).
        assert_eq!(v["bad_int"]["log"], "abcdefghij0123456789");
        assert_eq!(v["bad_int"]["offset"], 0);
        // `tail` acima do teto é limitado a 200_000; com log de 10 bytes vem
        // inteiro e sem truncar.
        assert_eq!(v["tail_cap"]["log"], "x".repeat(10));
        assert_eq!(v["tail_cap"]["truncated"], false);

        // `?limit=`: padrão 50, teto 100, nunca zero.
        assert_eq!(v["limit_default"]["n"], 50);
        assert_eq!(v["limit_hi"]["n"], 100);
        assert_eq!(v["limit_lo"]["n"], 1);
        assert_eq!(v["limit_bad"]["n"], 50);
    }

    /// Contrato das rotas sobre HTTP real, com o `Handler` de verdade num
    /// socket local: status code, JSON e autorização. Prova o comportamento
    /// que o app mobile vai consumir, não só a presença de substring.
    #[test]
    fn runner_http_routes_serve_single_run_and_incremental_log() {
        let body = r#"
import json, os, tempfile, threading, urllib.error, urllib.request
root = tempfile.mkdtemp(prefix="wr-http-")
mod.CONFIG_PATH = os.path.join(root, "config.json")
mod.TOKEN_PATH = os.path.join(root, "token")
mod.RUNS_PATH = os.path.join(root, "runs.json")
with open(mod.TOKEN_PATH, "w", encoding="utf-8") as f:
    f.write("admintoken-123")
with open(mod.CONFIG_PATH, "w", encoding="utf-8") as f:
    json.dump({"bindings": [], "workflows": [], "poll_secs": 60,
               "peer_keys": [{"peer_id": "par-1", "pair_key": "peerkey-abc"}]}, f)
runs = []
for i in range(3):
    runs.append({"id": f"run-vps-{i}", "workflow_id": "wf", "workflow_name": "CI",
                 "status": "Success", "trigger_label": "push", "title": "t",
                 "number": i, "commit": None, "started_at": "01/01/2026 00:00",
                 "started_ts": i, "finished_at": None,
                 "steps": [{"name": "build", "status": "Success", "log": "log-" + str(i) + "\n",
                            "secs": 0.1, "substeps": [], "started_ts": None}],
                 "approved": False, "cancel_requested": False, "secs": 0.1,
                 "actor": "x", "repo_name": None, "branch": "main",
                 "summary": "", "annotations": []})
mod.RUNS[:] = runs
srv = mod.ThreadingHTTPServer(("127.0.0.1", 0), mod.Handler)
port = srv.server_address[1]
threading.Thread(target=srv.serve_forever, daemon=True).start()

def call(path, headers):
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=5) as resp:
            return resp.status, json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as he:
        return he.code, json.loads(he.read().decode("utf-8") or "{}")

adm = {"Authorization": "Bearer admintoken-123"}
par = {"Authorization": "Bearer peerkey-abc", "X-Workploy-Peer": "par-1"}
res = {}
res["run"] = call("/run/run-vps-1", adm)
res["run_missing"] = call("/run/run-nao-existe", adm)
res["log"] = call("/run/run-vps-2/log?offset=2", adm)
res["log_missing"] = call("/run/run-nao-existe/log", adm)
res["runs_default"] = call("/runs", adm)
res["runs_limit"] = call("/runs?limit=2", adm)
res["runs_limit_cap"] = call("/runs?limit=999", adm)
res["run_noauth"] = call("/run/run-vps-1", {})
res["log_noauth"] = call("/run/run-vps-1/log", {})
res["run_peer"] = call("/run/run-vps-1", par)
res["log_peer"] = call("/run/run-vps-1/log", par)
srv.shutdown()
print(json.dumps(res))
"#;
        let Some(stdout) = run_generated_runner_python(body) else {
            eprintln!("[skip] sem interpretador Python: teste HTTP do runner não rodou");
            return;
        };
        let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("JSON do probe HTTP");

        // `/run/<id>`: objeto completo com o schema do Rust.
        assert_eq!(v["run"][0], 200);
        assert_eq!(v["run"][1]["id"], "run-vps-1");
        assert_eq!(v["run"][1]["steps"][0]["name"], "build");
        assert_eq!(v["run"][1]["steps"][0]["log"], "log-1\n");
        assert_eq!(v["run_missing"][0], 404);
        assert_eq!(v["run_missing"][1]["error"], "not_found");

        // `/run/<id>/log`: payload incremental + 404 estável.
        assert_eq!(v["log"][0], 200);
        assert_eq!(v["log"][1]["run_id"], "run-vps-2");
        assert_eq!(v["log"][1]["status"], "Success");
        assert_eq!(v["log"][1]["log"], "g-2\n");
        assert_eq!(v["log"][1]["steps"][0]["name"], "build");
        assert_eq!(v["log"][1]["steps"][0]["status"], "Success");
        assert_eq!(v["log"][1]["steps"][0]["bytes"], 6);
        assert_eq!(v["log"][1]["steps"][0]["lines"], 1);
        assert_eq!(v["log_missing"][0], 404);
        assert_eq!(v["log_missing"][1]["error"], "not_found");

        // `?limit=` na lista.
        assert_eq!(v["runs_default"][1].as_array().unwrap().len(), 3);
        assert_eq!(v["runs_limit"][1].as_array().unwrap().len(), 2);
        assert_eq!(v["runs_limit_cap"][1].as_array().unwrap().len(), 3);

        // Autorização: sem chave e com chave de par (fora de `/status`) = 401.
        for key in ["run_noauth", "log_noauth", "run_peer", "log_peer"] {
            assert_eq!(v[key][0], 401, "{key} deveria exigir admin");
            assert_eq!(v[key][1]["error"], "unauthorized");
        }
    }

    // ---------------------------------------------------------------------
    // Página mobile: `/ui` + assets embutidos + `/runs/<id>/approve`.
    //
    // O texto do arquivo é byte-identico ao que está em `app/assets/mobile`;
    // o HTML/JS/CSS é servido SEM Bearer (não há segredo), mas as rotas de
    // dados continuam 401 sem token.
    // ---------------------------------------------------------------------

    /// Marcador do bloco de injeção: só o template, nunca o script servido.
    #[test]
    fn runner_mobile_assets_are_embedded_once_and_marked() {
        assert!(!WORKPLOY_RUNNER_PY.contains("MOBILE_ASSETS = {"));
        assert_eq!(WORKPLOY_RUNNER_PY.matches("__WORKPLOY_MOBILE_ASSETS__").count(), 1);

        let py = runner_py_source();
        assert!(!py.contains("__WORKPLOY_MOBILE_ASSETS__"), "placeholder sobrou no script gerado");
        assert_eq!(py.matches("MOBILE_ASSETS = {").count(), 1);
        assert!(py.contains("MOBILE_CONTENT_TYPES = {"));
        assert!(py.contains("\"/ui\": \"text/html; charset=utf-8\""));
        assert!(py.contains("\"/ui/app.css\": \"text/css; charset=utf-8\""));
        assert!(py.contains("\"/ui/app.js\": \"text/javascript; charset=utf-8\""));
        // O HTML servido é o arquivo real (comparação com o `include_str!`).
        assert!(py.contains(MOBILE_INDEX_HTML.trim_end()));
        assert!(py.contains("<title>Workploy</title>"));
    }

    /// Instalação e auto-update mandam o script GERADO (com os assets), nunca o
    /// template sem eles; num build de teste, recusam publicar o runner fake.
    #[test]
    fn provision_installs_generated_runner_but_never_the_mock() {
        let opts = ProvisionOptions {
            install_git: false,
            install_docker: false,
            install_node: false,
            install_rust: false,
            install_runner_service: true,
            runner_port: 8787,
        };
        let script = build_provision_script(&opts, "tok123", "whsec456");
        assert!(!script.contains("__WORKPLOY_MOBILE_ASSETS__"));
        if MOBILE_HOSTILE_ASSETS.is_empty() {
            assert!(script.contains("MOBILE_ASSETS = {"));
            assert!(!script.contains("recuso instalar na VPS"));
        } else {
            // Build de teste: o runner fake nunca é publicado.
            assert!(script.contains("recuso instalar na VPS"));
            assert!(!script.contains("MOBILE_ASSETS = {"));
        }
    }

    #[test]
    fn py_escape_hostile_handles_unterminated_literals() {
        // `"""`/`'''` crus fechariam o literal: viram escapes. Barra vira
        // `\\` (senão o `\` do JS some) e `\r` sozinho vira `\u000d`.
        let raw = "a\"\"\"b'''c\\d\re";
        assert_eq!(py_escape_hostile(raw), "a\\u0022\\u0022\\u0022b\\u0027\\u0027\\u0027c\\\\d\\u000de");
        // Crase, `${` e `\n` não mudam (o literal não é bruto, nenhum deles
        // tem efeito dentro de `"""`).
        assert_eq!(py_escape_hostile("`x ${y}`\n"), "`x ${y}`\n");
        // CRLF fica: só o `\r` sozinho seria normalizado pelo Python.
        assert_eq!(py_escape_hostile("a\r\nb"), "a\r\nb");
    }

    #[test]
    fn runner_mobile_assets_install_clean_runner_in_normal_build() {
        if !MOBILE_HOSTILE_ASSETS.is_empty() {
            eprintln!("[skip] build de teste: caminho do binário real não roda aqui");
            return;
        }
        let opts = ProvisionOptions {
            install_git: false,
            install_docker: false,
            install_node: false,
            install_rust: false,
            install_runner_service: true,
            runner_port: 8787,
        };
        let script = build_provision_script(&opts, "tok123", "whsec456");
        assert!(script.contains("workploy-runner.service"));
        assert!(script.contains("MOBILE_ASSETS = {"));
        assert!(!script.contains("recuso instalar na VPS"));
        assert!(!script.contains("__WORKPLOY_MOBILE_ASSETS__"));
    }

    #[test]
    fn runner_script_parses_with_python_ast() {
        let body = "import ast\n\
             src = open(__file__, 'r', encoding='utf-8').read()\n\
             tree = ast.parse(src)\n\
             names = [n.name for n in tree.body if isinstance(n, ast.FunctionDef)]\n\
             print(json.dumps({'ast_ok': True, 'api_funcs': sorted(n for n in names if n.startswith('api_'))}))";
        let Some(stdout) = run_python_source_script(&runner_py_source(), body) else {
            eprintln!("[skip] sem interpretador Python: ast.parse do runner não rodou");
            return;
        };
        let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("JSON do probe");
        assert_eq!(v["ast_ok"], true);
        let funcs = v["api_funcs"].as_array().expect("lista de funções");
        for want in ["api_apply_approval", "api_approval_decision"] {
            assert!(
                funcs.iter().any(|f| f == want),
                "{want} não existe no script que o ast.parse aceitou"
            );
        }
    }

    /// O compilador Python é a prova: `compile()` do script gerado com o
    /// payload REAL da página (aspas, crases, `"""` nos comentários do
    /// `app.js`, `\` em regex, `$` em selects) e com um payload sintético
    /// hostil. Se a injeção vazasse um `"""` ou uma crase, o compile falharia.
    #[test]
    fn runner_script_stays_parseable_with_hostile_mobile_payload() {
        // Fração hostil: também roda no build normal, com um payload embutido no
        // teste. O gerador (`WORKPLOY_TEST_BUILD=1`) acrescenta o payload
        // completo do `app.js` por cima.
        let fallback = [
            "const tpl = `a ${b} c`;",
            "const r = /\\/+$/;",
            "const d = \"\"\"triple\"\"\" and '''also''';",
            "// `y` \\n literal \\\\ \\\\\\\\ crase ` e $ e } %",
            "$(\"#a\").val(1);",
            "carriage\rreturn",
        ]
        .join("\n");
        let (hostile_src, css_src, js_src): (String, String, String) = if MOBILE_HOSTILE_ASSETS.is_empty() {
            (fallback.clone(), "body{}".to_string(), format!("{fallback}\n{fallback}"))
        } else {
            let pick = |name: &str| {
                MOBILE_HOSTILE_ASSETS
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| *v)
                    .expect("asset hostil")
            };
            (pick("index").to_string(), pick("css").to_string(), pick("js").to_string())
        };
        let hostile = hostile_src.as_str();
        let css = css_src.as_str();
        let js = js_src.as_str();
        assert!(hostile.contains("${"), "payload precisa ter interpolação");
        assert!(hostile.contains("\"\"\"") && hostile.contains("'''"));
        assert!(hostile.contains('`'));
        assert!(hostile.contains("\\\\"));

        let marker = "__WORKPLOY_MOBILE_MARKER__";
        let block = format!(
            "MOBILE_ASSETS = {{\n    \"index\": \"\"\"{}\"\"\",\n    \"css\": \"\"\"{}\"\"\",\n    \"js\": \"\"\"{}\"\"\",\n}}\n# {}",
            py_escape_hostile(hostile),
            py_escape_hostile(css),
            py_escape_hostile(js),
            marker,
        );
        let generated = runner_py_source();
        assert!(!generated.contains(marker), "marcador de teste não pode ir para o runner");
        let synthetic = WORKPLOY_RUNNER_PY.replace("__WORKPLOY_MOBILE_ASSETS__", &block);
        assert!(synthetic.contains(marker), "bloco hostil não entrou no runner");

        let probe = |src: &str| -> serde_json::Value {
            let body = r#"
import ast
src = open(__file__, 'r', encoding='utf-8').read()
tree = ast.parse(src)
compile(tree, 'workploy_runner_probe.py', 'exec')
print(json.dumps({
    "assets": sorted(mod.MOBILE_ASSETS.keys()),
    "index": mod.MOBILE_ASSETS["index"],
    "css": mod.MOBILE_ASSETS["css"],
    "js": mod.MOBILE_ASSETS["js"],
}))
"#;
            match run_python_source_script(src, body) {
                Some(stdout) => serde_json::from_str(stdout.trim()).expect("JSON do probe"),
                None => serde_json::Value::Null,
            }
        };

        let v = probe(&synthetic);
        if v.is_null() {
            eprintln!("[skip] sem interpretador Python: prova hostil não rodou");
            return;
        }
        assert_eq!(v["assets"], serde_json::json!(["css", "index", "js"]));
        assert_eq!(v["index"], hostile, "o payload hostil não sobreviveu à injeção");
        assert_eq!(v["css"], css);
        assert_eq!(v["js"], js, "o payload hostil não sobreviveu à injeção");

        // E com os arquivos REAIS: o texto lido de volta é exatamente o
        // `include_str!`, o mesmo que o teste HTTP compara no corpo servido.
        let real = probe(&runner_py_source());
        if real.is_null() {
            eprintln!("[skip] sem interpretador Python: prova dos arquivos reais não rodou");
            return;
        }
        // O interpretador Python normaliza CRLF para LF ao abrir o arquivo
        // (`open(..., "r")` em modo texto). O `include_str!` entrega os bytes
        // crus do disco, que no Windows têm CRLF. A comparação é sobre o
        // conteúdo normalizado, que é o que o `ast`/`compile` enxergam e o
        // que o servidor devolve depois de escrito na VPS (sempre LF).
        let lf = |t: &str| t.replace("\r\n", "\n");
        assert_eq!(real["index"], lf(MOBILE_INDEX_HTML));
        assert_eq!(real["css"], lf(MOBILE_APP_CSS));
        assert_eq!(real["js"], lf(MOBILE_APP_JS));

        // O bloco com os arquivos REAIS: sem crase e os únicos `"""`
        // são os 6 delimitadores. O `app.js` tem escapes de regex (`\/`), então
        // o texto dentro do literal não é o arquivo cru — quem prova que o
        // servido é o arquivo é o `probe` acima e o teste HTTP.
        let real_block = runner_mobile_injection_block();
        assert!(real_block.contains(MOBILE_INDEX_HTML.trim_end()));
        let escaped_css = py_escape_hostile(MOBILE_APP_CSS);
        assert!(real_block.contains(&escaped_css));
        assert_eq!(real_block.matches("\"\"\"").count(), 6);
        // Literais fechados: cada terço é o texto entre delimitadores, com
        // `"""`/`'''` já escapados (nenhum delimitador perdido dentro deles).
        for (i, chunk) in real_block.split("\"\"\"").enumerate().skip(1).take(3) {
            assert!(
                !chunk.contains("\"\"\"") && !chunk.contains("'''"),
                "aspas triplas cruas dentro do literal {i}"
            );
        }
    }

    /// Rotas `/ui` + `/ui/app.css` + `/ui/app.js` e a decisão em
    /// `/runs/<id>/approve`, sobre HTTP real com o `Handler` de verdade.
    #[test]
    fn runner_http_serves_mobile_page_without_token_and_approves_gate() {
        let body = r#"
import json, os, tempfile, threading, urllib.error, urllib.request
root = tempfile.mkdtemp(prefix="wr-ui-")
mod.CONFIG_PATH = os.path.join(root, "config.json")
mod.TOKEN_PATH = os.path.join(root, "token")
mod.RUNS_PATH = os.path.join(root, "runs.json")
with open(mod.TOKEN_PATH, "w", encoding="utf-8") as f:
    f.write("admintoken-123")
with open(mod.CONFIG_PATH, "w", encoding="utf-8") as f:
    json.dump({"bindings": [], "workflows": [], "poll_secs": 60}, f)

def mk(rid, status):
    return {"id": rid, "workflow_id": "wf", "workflow_name": "CI",
            "status": status, "trigger_label": "push", "title": "t",
            "number": 1, "commit": None, "started_at": "01/01/2026 00:00",
            "started_ts": 1, "finished_at": None,
            "steps": [{"name": "deploy", "status": "WaitingApproval", "log": "",
                       "secs": 0.0, "substeps": [], "started_ts": None}],
            "approved": False, "cancel_requested": False, "secs": 0.0,
            "actor": "x", "repo_name": None, "branch": "main",
            "summary": "", "annotations": []}

mod.RUNS[:] = [mk("run-gate", "AwaitingApproval"), mk("run-running", "Running")]
srv = mod.ThreadingHTTPServer(("127.0.0.1", 0), mod.Handler)
port = srv.server_address[1]
threading.Thread(target=srv.serve_forever, daemon=True).start()

def call(path, headers=None, data=None, method=None):
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}",
                                 headers=headers or {}, data=data, method=method)
    try:
        with urllib.request.urlopen(req, timeout=5) as resp:
            return resp.status, dict(resp.headers), resp.read()
    except urllib.error.HTTPError as he:
        return he.code, dict(he.headers), he.read()

adm = {"Authorization": "Bearer admintoken-123"}
res = {}
st, hd, raw = call("/ui")
res["ui"] = [st, hd.get("Content-Type"), hd.get("Cache-Control") is not None,
             hd.get("Cache-Control"), raw == mod.MOBILE_ASSETS["index"].encode("utf-8")]
st, hd, raw = call("/ui/app.css")
res["css"] = [st, hd.get("Content-Type"), raw == mod.MOBILE_ASSETS["css"].encode("utf-8")]
st, hd, raw = call("/ui/app.js")
res["js"] = [st, hd.get("Content-Type"), raw == mod.MOBILE_ASSETS["js"].encode("utf-8")]
res["ui_404"] = call("/ui/nope", adm)[0]
res["runs_noauth"] = call("/runs")[0]

def approve(path, headers, payload):
    data = json.dumps(payload).encode("utf-8")
    h = dict(headers)
    h["Content-Type"] = "application/json"
    st, _hd, raw = call(path, h, data, "POST")
    return st, json.loads(raw.decode("utf-8") or "{}")

res["approve_noauth"] = approve("/runs/run-gate/approve", {}, {"decision": "approve"})[0]
res["approve_wrong_status"] = approve("/runs/run-running/approve", adm, {"decision": "approve"})
res["approve_missing"] = approve("/runs/run-nao-existe/approve", adm, {"decision": "approve"})[0]
res["approve"] = approve("/runs/run-gate/approve", adm, {"decision": "approve", "note": "ok no celular"})
run = [r for r in mod.RUNS if r["id"] == "run-gate"][0]
res["run_after"] = {"status": run["status"], "approved": run["approved"],
                    "approval": run.get("approval"), "log": run["steps"][0]["log"]}
res["approve_again"] = approve("/runs/run-gate/approve", adm, {"decision": "approve"})
mod.RUNS.append(mk("run-gate-2", "AwaitingApproval"))
res["reject"] = approve("/runs/run-gate-2/approve", adm, {"decision": "reject", "note": "nao"})
run2 = [r for r in mod.RUNS if r["id"] == "run-gate-2"][0]
res["run_reject"] = {"approved": run2["approved"], "cancel_requested": run2["cancel_requested"],
                     "decision": run2["approval"]["decision"]}
res["bad_decision"] = approve("/runs/run-gate-2/approve", adm, {"decision": "maybe"})[0]
# Espelho do desktop: o run aprovado sobrevive a `parse_runs_with_metrics`.
srv.shutdown()
print(json.dumps(res))
"#;
        let Some(stdout) = run_generated_runner_python(body) else {
            eprintln!("[skip] sem interpretador Python: teste HTTP da página não rodou");
            return;
        };
        let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("JSON do probe HTTP");

        // `/ui` sem Bearer: 200, HTML, no-store, corpo = index.html real.
        assert_eq!(v["ui"][0], 200);
        assert_eq!(v["ui"][1], "text/html; charset=utf-8");
        assert_eq!(v["ui"][2], true, "Cache-Control ausente");
        assert_eq!(v["ui"][3], "no-store");
        assert_eq!(v["ui"][4], true, "corpo não é o index.html embutido");
        assert_eq!(v["css"][0], 200);
        assert_eq!(v["css"][1], "text/css; charset=utf-8");
        assert_eq!(v["css"][2], true);
        assert_eq!(v["js"][0], 200);
        assert_eq!(v["js"][1], "text/javascript; charset=utf-8");
        assert_eq!(v["js"][2], true);
        // `/ui/<desconhecido>` e `/runs` sem token não passam pelo atalho.
        assert_eq!(v["ui_404"], 404);
        assert_eq!(v["runs_noauth"], 401);

        // `/runs/<id>/approve`
        assert_eq!(v["approve_noauth"], 401);
        assert_eq!(v["approve_wrong_status"][0], 409);
        assert_eq!(v["approve_wrong_status"][1]["error"], "not_awaiting_approval");
        assert_eq!(v["approve_missing"], 404);
        assert_eq!(v["approve"][0], 200);
        assert_eq!(v["approve"][1]["decision"], "approve");
        assert_eq!(v["run_after"]["approved"], true);
        assert_eq!(v["run_after"]["status"], "AwaitingApproval", "quem avança o run é o executor");
        assert_eq!(v["run_after"]["approval"]["note"], "ok no celular");
        assert_eq!(v["run_after"]["approval"]["decision"], "approve");
        assert!(
            v["run_after"]["log"].as_str().unwrap().contains("[aprovado pelo mobile] ok no celular"),
            "log do portão sem o registro da decisão: {}",
            v["run_after"]["log"]
        );
        // Repetir a decisão: a mesma função regrava o registro, sem run novo
        // e sem erro (o portão em si segue Aberto até o executor acordar).
        assert_eq!(v["approve_again"][0], 200);
        assert_eq!(v["approve_again"][1]["decision"], "approve");
        assert_eq!(v["reject"][0], 200);
        assert_eq!(v["run_reject"]["approved"], false);
        assert_eq!(v["run_reject"]["cancel_requested"], true);
        assert_eq!(v["run_reject"]["decision"], "reject");
        assert_eq!(v["bad_decision"], 400);

        // O run aprovado continua sendo um `Run` válido para o espelho Rust.
        let py_run = serde_json::json!({
            "id": "run-gate",
            "workflow_id": "wf",
            "workflow_name": "CI",
            "status": "AwaitingApproval",
            "trigger_label": "push",
            "title": "t",
            "number": 1,
            "started_at": "01/01/2026 00:00",
            "started_ts": 1,
            "steps": [],
            "approved": true,
            "cancel_requested": false,
            "secs": 0.0,
            "actor": "x",
            "branch": "main",
            "approval": {"decision": "approve", "note": "ok"},
        });
        let (runs, _) = parse_runs_with_metrics(&serde_json::to_string(&vec![py_run]).unwrap());
        assert_eq!(runs.len(), 1, "campo extra `approval` não pode invalidar o Run");
        assert!(runs[0].approved);

        // Corrida real: o executor está numa thread, o celular aprova em
        // outra. A escrita do executor (com a cópia antiga) precisa reaplicar
        // a decisão — senão a aprovação some no próximo poll.
        let body = r#"
import os, tempfile
root = tempfile.mkdtemp(prefix="wr-race-")
mod.RUNS_PATH = os.path.join(root, "runs.json")
run = {"id": "run-race", "status": "Running", "approved": False,
       "cancel_requested": False, "steps": [
           {"name": "deploy", "status": "Running", "log": "", "secs": 0.0,
            "substeps": [], "started_ts": None}]}
mod.RUNS[:] = [run]
seen = None   # executor nasceu antes de qualquer decisão
# ... o celular aprova (outra thread, sem o lock do executor):
mod.api_apply_approval(run, "approve", "pelo celular")
# ... e agora o executor grava o estado dele:
mod.save_runs_locked(seen)
print(json.dumps({"approved": run["approved"],
                  "cancel": run["cancel_requested"],
                  "seq": run["approval"]["seq"] > 0}))
"#;
        let Some(stdout) = run_generated_runner_python(body) else {
            eprintln!("[skip] sem interpretador Python: prova da corrida não rodou");
            return;
        };
        let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("JSON do probe");
        assert_eq!(v["approved"], true, "a escrita do executor apagou a aprovação");
        assert_eq!(v["cancel"], false);
        assert_eq!(v["seq"], true);
    }
}


