#![cfg_attr(not(test), windows_subsystem = "windows")]

use flate2::read::GzDecoder;
use std::env;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoTaskMemFree, CoUninitialize, IPersistFile,
};
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Programs, IShellLinkW, KF_FLAG_DEFAULT, SHGetKnownFolderPath,
    ShellLink,
};
use windows::core::{Interface, PCWSTR};

use windows_sys::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HINSTANCE, HWND, LPARAM, LRESULT, RECT, RPC_E_CHANGED_MODE,
    S_FALSE, S_OK, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, CLEARTYPE_QUALITY, COLOR_WINDOW, CreateFontW, CreateSolidBrush, DT_LEFT,
    DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, EndPaint, FillRect, HBRUSH, HDC, HFONT,
    InvalidateRect, PAINTSTRUCT, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ,
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows_sys::Win32::UI::Controls::{
    BST_CHECKED, BST_UNCHECKED, ICC_PROGRESS_CLASS, ICC_STANDARD_CLASSES, INITCOMMONCONTROLSEX,
    InitCommonControlsEx, PBM_SETPOS, PBM_SETRANGE32, PROGRESS_CLASSW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CW_USEDEFAULT,
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, DrawIconEx, ES_AUTOHSCROLL,
    GWLP_USERDATA, GetClientRect, GetMessageW, GetWindowLongPtrW, GetWindowTextLengthW,
    GetWindowTextW, HICON, IDC_ARROW, IDI_APPLICATION, IDYES, IMAGE_ICON, LR_DEFAULTSIZE,
    LR_SHARED, LoadCursorW, LoadIconW, LoadImageW, MB_ICONERROR, MB_ICONINFORMATION,
    MB_ICONQUESTION, MB_OK, MB_YESNO, MSG, MessageBoxW, PostMessageW, PostQuitMessage,
    RegisterClassExW, SPI_GETWORKAREA, SW_SHOW, SendMessageW, SetWindowLongPtrW, SetWindowPos,
    SetWindowTextW, ShowWindow, SystemParametersInfoW, TranslateMessage, WM_APP, WM_CLOSE,
    WM_COMMAND, WM_CREATE, WM_CTLCOLORSTATIC, WM_DESTROY, WM_PAINT, WM_SETFONT, WNDCLASSEXW,
    WS_BORDER, WS_CAPTION, WS_CHILD, WS_EX_CLIENTEDGE, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
    WS_TABSTOP, WS_VISIBLE,
};

const fn rgb(r: u8, g: u8, b: u8) -> u32 {
    (r as u32) | ((g as u32) << 8) | ((b as u32) << 16)
}

const APP_NAME: &str = "Workploy";
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const PUBLISHER: &str = "Workploy";
const EXE_NAME: &str = "workploy.exe";
const ICON_NAME: &str = "workploy.ico";
const UNINSTALL_EXE_NAME: &str = "uninstall.exe";
const SHORTCUT_NAME: &str = "Workploy.lnk";
const SHORTCUT_DESC: &str = "Workploy — Plataforma CI/CD & Deploy self-hosted";
const UNINSTALL_REG_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Workploy";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static PAYLOAD_GZ: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/workploy.exe.gz"));
static PAYLOAD_ICO: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/workploy.ico"));

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliArgs {
    silent: bool,
    uninstall: bool,
    dir: Option<PathBuf>,
    no_desktop: bool,
    no_shortcuts: bool,
    no_registry: bool,
    pub launch: bool,
    help: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstallOptions {
    target_dir: PathBuf,
    create_desktop_shortcut: bool,
    create_shortcuts: bool,
    register_uninstall: bool,
    launch_after: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UninstallOptions {
    target_dir: PathBuf,
    remove_shortcuts: bool,
    remove_registry: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstallReport {
    target_dir: PathBuf,
    exe_path: PathBuf,
    exe_bytes: u64,
    ico_written: bool,
    uninstaller_written: bool,
    shortcuts_created: Vec<PathBuf>,
    registry_written: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UninstallReport {
    target_dir: PathBuf,
    removed_files: Vec<PathBuf>,
    removed_shortcuts: Vec<PathBuf>,
    registry_removed: bool,
    self_delete_scheduled: bool,
}

fn main() {
    let args = match parse_args(env::args_os().skip(1)) {
        Ok(parsed) => parsed,
        Err(err) => {
            show_message(
                std::ptr::null_mut(),
                "Workploy — Erro de argumentos",
                &err,
                MB_OK | MB_ICONERROR,
            );
            std::process::exit(2);
        }
    };

    if args.help {
        show_message(
            std::ptr::null_mut(),
            "Workploy — Ajuda do Instalador",
            "Uso: workploy-setup.exe [opções]\n\n\
             --silent           Instala ou desinstala sem interface gráfica\n\
             --dir <caminho>    Define a pasta de instalação\n\
             --no-desktop       Não cria atalho na Área de Trabalho\n\
             --no-shortcuts     Não cria atalhos no Menu Iniciar nem Área de Trabalho\n\
             --no-registry      Não registra no Adicionar/Remover Programas\n\
             --launch           Inicia o Workploy após instalação silenciosa\n\
             --uninstall        Desinstala o Workploy",
            MB_OK | MB_ICONINFORMATION,
        );
        return;
    }

    if args.uninstall {
        run_uninstall_flow(&args);
        return;
    }

    if args.silent {
        let opts = InstallOptions {
            target_dir: args.dir.unwrap_or_else(default_install_dir),
            create_desktop_shortcut: !args.no_desktop && !args.no_shortcuts,
            create_shortcuts: !args.no_shortcuts,
            register_uninstall: !args.no_registry,
            launch_after: args.launch,
        };
        if let Err(err) = run_silent_install(&opts) {
            eprintln!("Erro na instalação: {err}");
            std::process::exit(1);
        }
        return;
    }

    let initial = InstallOptions {
        target_dir: args.dir.unwrap_or_else(default_install_dir),
        create_desktop_shortcut: !args.no_desktop && !args.no_shortcuts,
        create_shortcuts: !args.no_shortcuts,
        register_uninstall: !args.no_registry,
        launch_after: true,
    };
    if let Err(err) = run_gui(initial) {
        show_message(
            std::ptr::null_mut(),
            "Workploy — Erro",
            &format!("Não foi possível abrir o instalador:\n\n{err}"),
            MB_OK | MB_ICONERROR,
        );
        std::process::exit(1);
    }
}

fn parse_args<I, S>(args: I) -> Result<CliArgs, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut parsed = CliArgs {
        silent: false,
        uninstall: false,
        dir: None,
        no_desktop: false,
        no_shortcuts: false,
        no_registry: false,
        launch: false,
        help: false,
    };

    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        let s = arg.as_ref().to_string_lossy();
        match s.as_ref() {
            "--silent" | "/S" | "/silent" => parsed.silent = true,
            "--uninstall" => parsed.uninstall = true,
            "--no-desktop" => parsed.no_desktop = true,
            "--no-shortcuts" => parsed.no_shortcuts = true,
            "--no-registry" => parsed.no_registry = true,
            "--launch" => parsed.launch = true,
            "--help" | "-h" | "/?" => parsed.help = true,
            "--dir" => {
                let next = iter
                    .next()
                    .ok_or_else(|| "A opção --dir requer um caminho de destino.".to_string())?;
                let val = next.as_ref();
                if val.is_empty() {
                    return Err("O caminho passado em --dir não pode ser vazio.".to_string());
                }
                parsed.dir = Some(PathBuf::from(val));
            }
            _ if s.starts_with("--dir=") => {
                let val = &s["--dir=".len()..];
                if val.is_empty() {
                    return Err("O caminho passado em --dir= não pode ser vazio.".to_string());
                }
                parsed.dir = Some(PathBuf::from(val));
            }
            other => {
                return Err(format!("Argumento desconhecido: {other}"));
            }
        }
    }

    Ok(parsed)
}

fn default_install_dir() -> PathBuf {
    default_install_dir_from(env::var_os("LOCALAPPDATA").as_deref())
}

fn default_install_dir_from(local_app_data: Option<&OsStr>) -> PathBuf {
    match local_app_data {
        Some(lad) if !lad.is_empty() => PathBuf::from(lad).join("Programs").join(APP_NAME),
        _ => PathBuf::from("Programs").join(APP_NAME),
    }
}

fn resolve_uninstall_dir(explicit_dir: Option<PathBuf>) -> PathBuf {
    if let Some(dir) = explicit_dir {
        return dir;
    }
    if let Some(reg_dir) = read_registry_install_location() {
        return reg_dir;
    }
    if let Ok(current) = env::current_exe()
        && let Some(parent) = current.parent()
    {
        let is_uninstaller = current
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case(UNINSTALL_EXE_NAME));
        if is_uninstaller && parent.join(EXE_NAME).exists() {
            return parent.to_path_buf();
        }
    }
    default_install_dir()
}

fn run_silent_install(opts: &InstallOptions) -> Result<(), String> {
    let report = install_with_payload(opts, PAYLOAD_GZ, PAYLOAD_ICO, &mut |_, _| {})?;
    if opts.launch_after {
        launch_installed_app(&report.exe_path)?;
    }
    Ok(())
}

fn run_uninstall_flow(args: &CliArgs) {
    let target_dir = resolve_uninstall_dir(args.dir.clone());
    if !args.silent {
        let prompt = format!(
            "Deseja desinstalar o Workploy de:\n{}\n\nSeus dados e configurações serão preservados.",
            target_dir.display()
        );
        let answer = show_message(
            std::ptr::null_mut(),
            "Workploy — Desinstalação",
            &prompt,
            MB_YESNO | MB_ICONQUESTION,
        );
        if answer != IDYES {
            return;
        }
    }

    let opts = UninstallOptions {
        target_dir,
        remove_shortcuts: !args.no_shortcuts,
        remove_registry: !args.no_registry,
    };

    match uninstall_with_options(&opts, &mut |_| {}) {
        Ok(_) => {
            if !args.silent {
                show_message(
                    std::ptr::null_mut(),
                    "Workploy — Desinstalação",
                    "O Workploy foi desinstalado com sucesso.",
                    MB_OK | MB_ICONINFORMATION,
                );
            }
        }
        Err(err) => {
            if !args.silent {
                show_message(
                    std::ptr::null_mut(),
                    "Workploy — Erro na Desinstalação",
                    &format!("Ocorreu um erro durante a desinstalação:\n\n{err}"),
                    MB_OK | MB_ICONERROR,
                );
            } else {
                eprintln!("Erro na desinstalação: {err}");
            }
            std::process::exit(1);
        }
    }
}

fn install_with_payload(
    opts: &InstallOptions,
    payload_gz: &[u8],
    icon_bytes: &[u8],
    progress: &mut dyn FnMut(u8, &str),
) -> Result<InstallReport, String> {
    if !cfg!(has_payload) && payload_gz.is_empty() {
        return Err(
            "Payload do workploy.exe não está embutido neste executável. Compile com scripts/build-installer.ps1."
                .to_string(),
        );
    }
    if payload_gz.is_empty() {
        return Err("Payload compactado está vazio.".to_string());
    }

    let target_dir = opts.target_dir.clone();
    if target_dir.as_os_str().is_empty() {
        return Err("A pasta de instalação não pode ser vazia.".to_string());
    }

    progress(12, "Preparando pasta de destino...");
    fs::create_dir_all(&target_dir)
        .map_err(|e| format!("Falha ao criar pasta '{}': {e}", target_dir.display()))?;

    let exe_path = target_dir.join(EXE_NAME);
    let ico_path = target_dir.join(ICON_NAME);
    let uninstall_path = target_dir.join(UNINSTALL_EXE_NAME);

    progress(25, "Verificando se o Workploy está em execução...");
    ensure_target_unlocked(&exe_path)?;

    progress(45, "Extraindo arquivos do Workploy...");
    let exe_bytes = decompress_to_file(payload_gz, &exe_path)?;

    let mut ico_written = false;
    if !icon_bytes.is_empty() {
        fs::write(&ico_path, icon_bytes)
            .map_err(|e| format!("Falha ao gravar ícone '{}': {e}", ico_path.display()))?;
        ico_written = true;
    }

    progress(68, "Gravando desinstalador...");
    let uninstaller_written = copy_self_as_uninstaller(&uninstall_path)?;

    let mut shortcuts_created = Vec::new();
    if opts.create_shortcuts {
        progress(82, "Criando atalhos no Windows...");
        let icon_for_shortcut = if ico_written { &ico_path } else { &exe_path };
        shortcuts_created =
            create_shortcuts(&exe_path, icon_for_shortcut, opts.create_desktop_shortcut)?;
    }

    let mut registry_written = false;
    if opts.register_uninstall {
        progress(94, "Registrando aplicativo no Windows...");
        let total_bytes = exe_bytes.saturating_add(icon_bytes.len() as u64);
        let size_kb = compute_estimated_size_kb(total_bytes);
        write_uninstall_registry(&target_dir, &ico_path, &uninstall_path, size_kb)?;
        registry_written = true;
    }

    progress(100, "Instalação concluída com sucesso!");

    Ok(InstallReport {
        target_dir,
        exe_path,
        exe_bytes,
        ico_written,
        uninstaller_written,
        shortcuts_created,
        registry_written,
    })
}

fn uninstall_with_options(
    opts: &UninstallOptions,
    progress: &mut dyn FnMut(&str),
) -> Result<UninstallReport, String> {
    let mut removed_shortcuts = Vec::new();
    if opts.remove_shortcuts {
        progress("Removendo atalhos...");
        removed_shortcuts = remove_shortcuts()?;
    }

    let mut registry_removed = false;
    if opts.remove_registry {
        progress("Removendo registro do Windows...");
        registry_removed = delete_uninstall_registry()?;
    }

    let exe_path = opts.target_dir.join(EXE_NAME);
    let ico_path = opts.target_dir.join(ICON_NAME);
    let uninstall_path = opts.target_dir.join(UNINSTALL_EXE_NAME);

    progress("Encerrando instâncias em execução...");
    ensure_target_unlocked(&exe_path)?;

    let mut removed_files = Vec::new();
    for file in [&exe_path, &ico_path] {
        if file.exists() {
            fs::remove_file(file)
                .map_err(|e| format!("Falha ao remover '{}': {e}", file.display()))?;
            removed_files.push(file.to_path_buf());
        }
    }

    let mut self_delete_scheduled = false;
    if uninstall_path.exists() {
        let running_from_target = env::current_exe()
            .ok()
            .is_some_and(|cur| paths_equal(&cur, &uninstall_path));
        if running_from_target {
            schedule_self_delete(&uninstall_path, &opts.target_dir)?;
            self_delete_scheduled = true;
        } else {
            fs::remove_file(&uninstall_path)
                .map_err(|e| format!("Falha ao remover '{}': {e}", uninstall_path.display()))?;
            removed_files.push(uninstall_path);
            let _ = fs::remove_dir(&opts.target_dir);
        }
    } else if opts.target_dir.exists() {
        let _ = fs::remove_dir(&opts.target_dir);
    }

    Ok(UninstallReport {
        target_dir: opts.target_dir.clone(),
        removed_files,
        removed_shortcuts,
        registry_removed,
        self_delete_scheduled,
    })
}

fn compute_estimated_size_kb(total_bytes: u64) -> u32 {
    let kb = total_bytes.div_ceil(1024).max(1);
    u32::try_from(kb).unwrap_or(u32::MAX)
}

fn decompress_to_file(gz_bytes: &[u8], dest: &Path) -> Result<u64, String> {
    let tmp_path = dest.with_extension("exe.tmp");
    let mut decoder = GzDecoder::new(gz_bytes);
    let out_file = File::create(&tmp_path)
        .map_err(|e| format!("Falha ao criar arquivo temporário '{}': {e}", tmp_path.display()))?;
    let mut writer = BufWriter::new(out_file);
    let copied = io::copy(&mut decoder, &mut writer).map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        format!("Falha ao descompactar payload: {e}")
    })?;
    writer.flush().map_err(|e| {
        let _ = fs::remove_file(&tmp_path);
        format!("Falha ao gravar arquivo extraído: {e}")
    })?;
    drop(writer);

    if copied == 0 {
        let _ = fs::remove_file(&tmp_path);
        return Err("O executável extraído resultou em 0 bytes.".to_string());
    }

    // Read trailing bytes to ensure complete gzip stream validation.
    let mut tail = [0u8; 1];
    if let Ok(n) = decoder.read(&mut tail)
        && n > 0
    {
        let _ = fs::remove_file(&tmp_path);
        return Err("Fluxo gzip inesperado após leitura completa.".to_string());
    }

    replace_file_with_retry(&tmp_path, dest, 10, Duration::from_millis(300))?;

    Ok(copied)
}

fn is_retryable_fs_lock(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::PermissionDenied
        || matches!(err.raw_os_error(), Some(5 | 32 | 33))
}

fn replace_file_with_retry(
    tmp_path: &Path,
    dest: &Path,
    max_attempts: usize,
    delay: Duration,
) -> Result<(), String> {
    let attempts = max_attempts.max(1);
    let mut last_err = String::new();
    for attempt in 0..attempts {
        if dest.exists()
            && let Err(e) = fs::remove_file(dest)
        {
            last_err = format!("Falha ao substituir '{}': {e}", dest.display());
            if is_retryable_fs_lock(&e) && attempt + 1 < attempts {
                thread::sleep(delay);
                continue;
            }
            let _ = fs::remove_file(tmp_path);
            return Err(last_err);
        }
        match fs::rename(tmp_path, dest) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = format!(
                    "Falha ao mover '{}' para '{}': {e}",
                    tmp_path.display(),
                    dest.display()
                );
                if is_retryable_fs_lock(&e) && attempt + 1 < attempts {
                    thread::sleep(delay);
                    continue;
                }
                let _ = fs::remove_file(tmp_path);
                return Err(last_err);
            }
        }
    }
    let _ = fs::remove_file(tmp_path);
    Err(last_err)
}

fn copy_self_as_uninstaller(dest: &Path) -> Result<bool, String> {
    let current = env::current_exe()
        .map_err(|e| format!("Não foi possível localizar o executável atual: {e}"))?;
    if paths_equal(&current, dest) {
        return Ok(false);
    }
    fs::copy(&current, dest).map_err(|e| {
        format!("Falha ao copiar desinstalador para '{}': {e}", dest.display())
    })?;
    Ok(true)
}

fn ensure_target_unlocked(target_exe: &Path) -> Result<(), String> {
    if !target_exe.exists() {
        return Ok(());
    }
    if OpenOptions::new().write(true).open(target_exe).is_ok() {
        return Ok(());
    }

    // O executável está em uso; encerra o processo em execução sem janela de console.
    let _ = Command::new("taskkill")
        .args(["/IM", EXE_NAME, "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .status();

    for _ in 0..10 {
        thread::sleep(Duration::from_millis(300));
        if OpenOptions::new().write(true).open(target_exe).is_ok() {
            return Ok(());
        }
    }

    Err(format!(
        "O arquivo '{}' está em uso e não pôde ser substituído. Feche o Workploy e tente novamente.",
        target_exe.display()
    ))
}

fn build_self_delete_raw_arg(uninstall_exe: &Path, install_dir: &Path) -> String {
    let exe_str = uninstall_exe.display();
    let dir_str = install_dir.display();
    format!(
        "/C \"ping 127.0.0.1 -n 3 > NUL & del /F /Q \"{exe_str}\" > NUL 2>&1 & rmdir \"{dir_str}\" > NUL 2>&1\""
    )
}

fn schedule_self_delete(uninstall_exe: &Path, install_dir: &Path) -> Result<(), String> {
    let raw = build_self_delete_raw_arg(uninstall_exe, install_dir);
    Command::new("cmd")
        .raw_arg(raw)
        .current_dir(env::temp_dir())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("Falha ao agendar remoção do desinstalador: {e}"))?;
    Ok(())
}

fn launch_installed_app(exe_path: &Path) -> Result<(), String> {
    let mut cmd = Command::new(exe_path);
    if let Some(parent) = exe_path.parent() {
        cmd.current_dir(parent);
    }
    cmd.spawn()
        .map_err(|e| format!("Não foi possível iniciar '{}': {e}", exe_path.display()))?;
    Ok(())
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    let ca = fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let cb = fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    ca.to_string_lossy()
        .eq_ignore_ascii_case(&cb.to_string_lossy())
}

fn known_folder_path(folder_id: &windows::core::GUID) -> Option<PathBuf> {
    unsafe {
        let raw = SHGetKnownFolderPath(folder_id, KF_FLAG_DEFAULT, None).ok()?;
        let s = raw.to_string().ok();
        CoTaskMemFree(Some(raw.as_ptr() as *const _));
        s.map(PathBuf::from)
    }
}

fn start_menu_shortcut_paths() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(programs) = known_folder_path(&FOLDERID_Programs) {
        candidates.push(programs.join(APP_NAME).join(SHORTCUT_NAME));
    }
    if let Some(appdata) = env::var_os("APPDATA") {
        let fallback = PathBuf::from(appdata)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join(APP_NAME)
            .join(SHORTCUT_NAME);
        if !candidates.iter().any(|p| paths_equal(p, &fallback)) {
            candidates.push(fallback);
        }
    }
    candidates
}

fn desktop_shortcut_paths() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(desktop) = known_folder_path(&FOLDERID_Desktop) {
        candidates.push(desktop.join(SHORTCUT_NAME));
    }
    if let Some(profile) = env::var_os("USERPROFILE") {
        let fallback = PathBuf::from(profile).join("Desktop").join(SHORTCUT_NAME);
        if !candidates.iter().any(|p| paths_equal(p, &fallback)) {
            candidates.push(fallback);
        }
    }
    candidates
}

struct ComGuard {
    should_uninit: bool,
}

impl ComGuard {
    fn init() -> Result<Self, String> {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if hr == windows::core::HRESULT(S_OK) || hr == windows::core::HRESULT(S_FALSE) {
            Ok(Self {
                should_uninit: true,
            })
        } else if hr == windows::core::HRESULT(RPC_E_CHANGED_MODE) {
            Ok(Self {
                should_uninit: false,
            })
        } else {
            Err(format!("Falha ao inicializar COM (HRESULT 0x{:08X}).", hr.0 as u32))
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.should_uninit {
            unsafe { CoUninitialize() };
        }
    }
}

fn create_shortcuts(
    exe_path: &Path,
    icon_path: &Path,
    create_desktop: bool,
) -> Result<Vec<PathBuf>, String> {
    let _com = ComGuard::init()?;
    let mut created = Vec::new();

    if let Some(start_menu_lnk) = start_menu_shortcut_paths().into_iter().next() {
        if let Some(parent) = start_menu_lnk.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                format!("Falha ao criar pasta no Menu Iniciar '{}': {e}", parent.display())
            })?;
        }
        create_shell_link(&start_menu_lnk, exe_path, icon_path)?;
        created.push(start_menu_lnk);
    }

    if create_desktop
        && let Some(desktop_lnk) = desktop_shortcut_paths().into_iter().next()
    {
        if let Some(parent) = desktop_lnk.parent() {
            let _ = fs::create_dir_all(parent);
        }
        create_shell_link(&desktop_lnk, exe_path, icon_path)?;
        created.push(desktop_lnk);
    }

    Ok(created)
}

fn create_shell_link(lnk_path: &Path, target_exe: &Path, icon_path: &Path) -> Result<(), String> {
    let target_w = to_wide_null(target_exe.as_os_str());
    let workdir_w = to_wide_null(
        target_exe
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .as_os_str(),
    );
    let desc_w = to_wide_null(OsStr::new(SHORTCUT_DESC));
    let icon_w = to_wide_null(icon_path.as_os_str());
    let lnk_w = to_wide_null(lnk_path.as_os_str());

    unsafe {
        let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| format!("Falha ao criar IShellLinkW: {e}"))?;
        shell_link
            .SetPath(PCWSTR::from_raw(target_w.as_ptr()))
            .map_err(|e| format!("Falha em IShellLinkW::SetPath: {e}"))?;
        shell_link
            .SetWorkingDirectory(PCWSTR::from_raw(workdir_w.as_ptr()))
            .map_err(|e| format!("Falha em IShellLinkW::SetWorkingDirectory: {e}"))?;
        shell_link
            .SetDescription(PCWSTR::from_raw(desc_w.as_ptr()))
            .map_err(|e| format!("Falha em IShellLinkW::SetDescription: {e}"))?;
        shell_link
            .SetIconLocation(PCWSTR::from_raw(icon_w.as_ptr()), 0)
            .map_err(|e| format!("Falha em IShellLinkW::SetIconLocation: {e}"))?;

        let persist: IPersistFile = shell_link
            .cast()
            .map_err(|e| format!("Falha ao obter IPersistFile: {e}"))?;
        persist
            .Save(PCWSTR::from_raw(lnk_w.as_ptr()), true)
            .map_err(|e| format!("Falha ao salvar atalho '{}': {e}", lnk_path.display()))?;
    }

    Ok(())
}

fn remove_shortcuts() -> Result<Vec<PathBuf>, String> {
    let mut removed = Vec::new();
    for lnk in start_menu_shortcut_paths() {
        if lnk.exists() {
            fs::remove_file(&lnk)
                .map_err(|e| format!("Falha ao remover atalho '{}': {e}", lnk.display()))?;
            removed.push(lnk.clone());
        }
        if let Some(folder) = lnk.parent()
            && folder.exists()
        {
            let _ = fs::remove_dir(folder);
        }
    }

    for lnk in desktop_shortcut_paths() {
        if lnk.exists() {
            fs::remove_file(&lnk)
                .map_err(|e| format!("Falha ao remover atalho '{}': {e}", lnk.display()))?;
            removed.push(lnk);
        }
    }

    Ok(removed)
}

fn write_uninstall_registry(
    install_dir: &Path,
    icon_path: &Path,
    uninstall_exe: &Path,
    estimated_size_kb: u32,
) -> Result<(), String> {
    let subkey_w = to_wide_null(OsStr::new(UNINSTALL_REG_SUBKEY));
    let mut hkey: HKEY = std::ptr::null_mut();

    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey_w.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            std::ptr::null(),
            &mut hkey,
            std::ptr::null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!(
            "Falha ao criar chave de registro Uninstall\\Workploy (código {status})."
        ));
    }

    let uninstall_cmd = format!("\"{}\" --uninstall", uninstall_exe.display());
    let quiet_cmd = format!("\"{}\" --uninstall --silent", uninstall_exe.display());
    let install_loc = install_dir.display().to_string();
    let display_icon = icon_path.display().to_string();

    let write_res = (|| -> Result<(), String> {
        reg_set_sz(hkey, "DisplayName", APP_NAME)?;
        reg_set_sz(hkey, "DisplayVersion", APP_VERSION)?;
        reg_set_sz(hkey, "Publisher", PUBLISHER)?;
        reg_set_sz(hkey, "DisplayIcon", &display_icon)?;
        reg_set_sz(hkey, "InstallLocation", &install_loc)?;
        reg_set_sz(hkey, "UninstallString", &uninstall_cmd)?;
        reg_set_sz(hkey, "QuietUninstallString", &quiet_cmd)?;
        reg_set_dword(hkey, "EstimatedSize", estimated_size_kb)?;
        reg_set_dword(hkey, "NoModify", 1)?;
        reg_set_dword(hkey, "NoRepair", 1)?;
        Ok(())
    })();

    unsafe {
        RegCloseKey(hkey);
    }
    write_res
}

fn reg_set_sz(hkey: HKEY, name: &str, value: &str) -> Result<(), String> {
    let name_w = to_wide_null(OsStr::new(name));
    let value_w = to_wide_null(OsStr::new(value));
    let byte_len = u32::try_from(value_w.len() * std::mem::size_of::<u16>())
        .map_err(|_| format!("Valor longo demais para '{name}'."))?;
    let status = unsafe {
        RegSetValueExW(
            hkey,
            name_w.as_ptr(),
            0,
            REG_SZ,
            value_w.as_ptr() as *const u8,
            byte_len,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!("Falha ao gravar '{name}' no registro (código {status})."));
    }
    Ok(())
}

fn reg_set_dword(hkey: HKEY, name: &str, value: u32) -> Result<(), String> {
    let name_w = to_wide_null(OsStr::new(name));
    let bytes = value.to_le_bytes();
    let status = unsafe {
        RegSetValueExW(
            hkey,
            name_w.as_ptr(),
            0,
            REG_DWORD,
            bytes.as_ptr(),
            bytes.len() as u32,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!("Falha ao gravar DWORD '{name}' no registro (código {status})."));
    }
    Ok(())
}

fn delete_uninstall_registry() -> Result<bool, String> {
    let subkey_w = to_wide_null(OsStr::new(UNINSTALL_REG_SUBKEY));
    let status = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, subkey_w.as_ptr()) };
    if status == ERROR_SUCCESS {
        Ok(true)
    } else if status == ERROR_FILE_NOT_FOUND {
        Ok(false)
    } else {
        Err(format!(
            "Falha ao remover entrada do registro (código {status})."
        ))
    }
}

fn read_registry_install_location() -> Option<PathBuf> {
    let subkey_w = to_wide_null(OsStr::new(UNINSTALL_REG_SUBKEY));
    let mut hkey: HKEY = std::ptr::null_mut();
    let open_status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            subkey_w.as_ptr(),
            0,
            KEY_READ,
            &mut hkey,
        )
    };
    if open_status != ERROR_SUCCESS {
        return None;
    }

    let value_name = to_wide_null(OsStr::new("InstallLocation"));
    let mut reg_type: u32 = 0;
    let mut byte_len: u32 = 0;
    let query_len = unsafe {
        RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            &mut reg_type,
            std::ptr::null_mut(),
            &mut byte_len,
        )
    };
    if query_len != ERROR_SUCCESS || reg_type != REG_SZ || byte_len == 0 {
        unsafe { RegCloseKey(hkey) };
        return None;
    }

    let mut buf = vec![0u16; (byte_len as usize).div_ceil(2)];
    let query_val = unsafe {
        RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            &mut reg_type,
            buf.as_mut_ptr() as *mut u8,
            &mut byte_len,
        )
    };
    unsafe { RegCloseKey(hkey) };
    if query_val != ERROR_SUCCESS {
        return None;
    }

    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let text = String::from_utf16_lossy(&buf[..end]);
    if text.trim().is_empty() {
        None
    } else {
        Some(PathBuf::from(text))
    }
}

fn to_wide_null(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

fn to_wide_str(s: &str) -> Vec<u16> {
    to_wide_null(OsStr::new(s))
}

fn show_message(hwnd: HWND, title: &str, text: &str, flags: u32) -> i32 {
    let title_w = to_wide_str(title);
    let text_w = to_wide_str(text);
    unsafe { MessageBoxW(hwnd, text_w.as_ptr(), title_w.as_ptr(), flags) }
}

// ============================================================================
// Interface Gráfica Win32 Nativa
// ============================================================================

const WIN_W: i32 = 480;
const WIN_H: i32 = 360;
const HEADER_H: i32 = 76;

const ID_PATH_EDIT: usize = 1001;
const ID_CHK_DESKTOP: usize = 1002;
const ID_CHK_LAUNCH: usize = 1003;
const ID_PROGRESS: usize = 1004;
const ID_STATUS_LBL: usize = 1005;
const ID_BTN_INSTALL: usize = 1006;
const ID_BTN_CANCEL: usize = 1007;

const WM_INSTALL_PROGRESS: u32 = WM_APP + 1;
const WM_INSTALL_DONE: u32 = WM_APP + 2;

struct GuiState {
    initial: InstallOptions,
    installing: bool,
    completed: bool,
    status_text: Arc<Mutex<String>>,
    result_box: Arc<Mutex<Option<Result<InstallReport, String>>>>,
    hwnd_path: HWND,
    hwnd_chk_desktop: HWND,
    hwnd_chk_launch: HWND,
    hwnd_progress: HWND,
    hwnd_status: HWND,
    hwnd_btn_install: HWND,
    hwnd_btn_cancel: HWND,
    font_regular: HFONT,
    font_bold: HFONT,
    bg_brush: HBRUSH,
    header_icon: HICON,
}

impl GuiState {
    fn new(initial: InstallOptions) -> Self {
        Self {
            initial,
            installing: false,
            completed: false,
            status_text: Arc::new(Mutex::new("Pronto para instalar o Workploy.".to_string())),
            result_box: Arc::new(Mutex::new(None)),
            hwnd_path: std::ptr::null_mut(),
            hwnd_chk_desktop: std::ptr::null_mut(),
            hwnd_chk_launch: std::ptr::null_mut(),
            hwnd_progress: std::ptr::null_mut(),
            hwnd_status: std::ptr::null_mut(),
            hwnd_btn_install: std::ptr::null_mut(),
            hwnd_btn_cancel: std::ptr::null_mut(),
            font_regular: std::ptr::null_mut(),
            font_bold: std::ptr::null_mut(),
            bg_brush: std::ptr::null_mut(),
            header_icon: std::ptr::null_mut(),
        }
    }
}

fn run_gui(initial: InstallOptions) -> Result<(), String> {
    unsafe {
        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_PROGRESS_CLASS | ICC_STANDARD_CLASSES,
        };
        InitCommonControlsEx(&icc);

        let hinstance: HINSTANCE = GetModuleHandleW(std::ptr::null());
        let class_name = to_wide_str("WorkployInstallerWindow");
        let app_icon = LoadImageW(
            hinstance,
            std::ptr::without_provenance::<u16>(1),
            IMAGE_ICON,
            0,
            0,
            LR_DEFAULTSIZE | LR_SHARED,
        );
        let hicon: HICON = if app_icon.is_null() {
            LoadIconW(std::ptr::null_mut(), IDI_APPLICATION)
        } else {
            app_icon
        };

        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: hicon,
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: hicon,
        };

        if RegisterClassExW(&wc) == 0 {
            return Err("Falha ao registrar classe de janela Win32.".to_string());
        }

        let mut state = Box::new(GuiState::new(initial));
        state.header_icon = hicon;

        let title = to_wide_str(&format!("{APP_NAME} v{APP_VERSION} — Instalador"));
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;

        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            title.as_ptr(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            WIN_W,
            WIN_H,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            &mut *state as *mut GuiState as *const core::ffi::c_void,
        );
        if hwnd.is_null() {
            return Err("Falha ao criar janela principal do instalador.".to_string());
        }

        // Mantém GuiState vivo até WM_DESTROY.
        let _raw_state = Box::into_raw(state);

        center_window(hwnd, WIN_W, WIN_H);
        ShowWindow(hwnd, SW_SHOW);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

unsafe fn center_window(hwnd: HWND, width: i32, height: i32) {
    let mut work: RECT = unsafe { std::mem::zeroed() };
    let ok = unsafe { SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut _ as *mut _, 0) };
    if ok != 0 {
        let x = work.left + ((work.right - work.left) - width) / 2;
        let y = work.top + ((work.bottom - work.top) - height) / 2;
        unsafe { SetWindowPos(hwnd, std::ptr::null_mut(), x, y, width, height, 0) };
    }
}

unsafe fn create_ui_font(height: i32, weight: i32) -> HFONT {
    let face = to_wide_str("Segoe UI");
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            1, // DEFAULT_CHARSET
            0,
            0,
            CLEARTYPE_QUALITY as u32,
            0,
            face.as_ptr(),
        )
    }
}

unsafe fn set_ctrl_font(hwnd: HWND, font: HFONT) {
    if !hwnd.is_null() && !font.is_null() {
        unsafe { SendMessageW(hwnd, WM_SETFONT, font as WPARAM, 1) };
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_CREATE {
        let cs = unsafe { &*(lparam as *const windows_sys::Win32::UI::WindowsAndMessaging::CREATESTRUCTW) };
        let state_ptr = cs.lpCreateParams as *mut GuiState;
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);
            init_controls(hwnd, &mut *state_ptr);
        }
        return 0;
    }

    let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GuiState };
    if state_ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    let state = unsafe { &mut *state_ptr };

    match msg {
        WM_PAINT => {
            unsafe { paint_header(hwnd, state) };
            0
        }
        WM_CTLCOLORSTATIC => {
            let hdc = wparam as HDC;
            unsafe {
                SetBkMode(hdc, TRANSPARENT as i32);
                SetTextColor(hdc, rgb(30, 41, 59));
            }
            state.bg_brush as LRESULT
        }
        WM_COMMAND => {
            let ctrl_id = wparam & 0xFFFF;
            match ctrl_id {
                ID_BTN_INSTALL => {
                    if state.completed {
                        unsafe { DestroyWindow(hwnd) };
                    } else if !state.installing {
                        start_gui_install(hwnd, state);
                    }
                    0
                }
                ID_BTN_CANCEL => {
                    if !state.installing {
                        unsafe { DestroyWindow(hwnd) };
                    }
                    0
                }
                _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
            }
        }
        WM_INSTALL_PROGRESS => {
            let pct = wparam.min(100);
            unsafe { SendMessageW(state.hwnd_progress, PBM_SETPOS, pct, 0) };
            if let Ok(txt) = state.status_text.lock() {
                let wide = to_wide_str(&txt);
                unsafe { SetWindowTextW(state.hwnd_status, wide.as_ptr()) };
            }
            0
        }
        WM_INSTALL_DONE => {
            state.installing = false;
            let outcome = state.result_box.lock().ok().and_then(|mut g| g.take());
            match outcome {
                Some(Ok(report)) => {
                    state.completed = true;
                    unsafe {
                        SendMessageW(state.hwnd_progress, PBM_SETPOS, 100, 0);
                        let done_txt = to_wide_str("Instalação concluída com sucesso!");
                        SetWindowTextW(state.hwnd_status, done_txt.as_ptr());
                    }

                    let launch_checked = unsafe {
                        SendMessageW(state.hwnd_chk_launch, BM_GETCHECK, 0, 0) as u32 == BST_CHECKED
                    };
                    if launch_checked {
                        if let Err(err) = launch_installed_app(&report.exe_path) {
                            show_message(
                                hwnd,
                                "Workploy — Aviso",
                                &format!("Instalação concluída, mas não foi possível iniciar o aplicativo:\n\n{err}"),
                                MB_OK | MB_ICONERROR,
                            );
                        }
                        unsafe { DestroyWindow(hwnd) };
                    } else {
                        unsafe {
                            let close_txt = to_wide_str("Concluir");
                            SetWindowTextW(state.hwnd_btn_install, close_txt.as_ptr());
                            EnableWindow(state.hwnd_btn_install, 1);
                            EnableWindow(state.hwnd_btn_cancel, 0);
                        }
                        show_message(
                            hwnd,
                            "Workploy — Instalação Concluída",
                            &format!(
                                "O Workploy foi instalado com sucesso em:\n{}",
                                report.target_dir.display()
                            ),
                            MB_OK | MB_ICONINFORMATION,
                        );
                    }
                }
                Some(Err(err)) => {
                    unsafe {
                        SendMessageW(state.hwnd_progress, PBM_SETPOS, 0, 0);
                        let err_lbl = to_wide_str("Falha na instalação.");
                        SetWindowTextW(state.hwnd_status, err_lbl.as_ptr());
                        set_controls_enabled(state, true);
                    }
                    show_message(
                        hwnd,
                        "Workploy — Erro na Instalação",
                        &err,
                        MB_OK | MB_ICONERROR,
                    );
                }
                None => {}
            }
            0
        }
        WM_CLOSE => {
            if !state.installing {
                unsafe { DestroyWindow(hwnd) };
            }
            0
        }
        WM_DESTROY => {
            unsafe {
                if !state.font_regular.is_null() {
                    DeleteObject(state.font_regular as *mut _);
                }
                if !state.font_bold.is_null() {
                    DeleteObject(state.font_bold as *mut _);
                }
                if !state.bg_brush.is_null() {
                    DeleteObject(state.bg_brush as *mut _);
                }
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(state_ptr));
                PostQuitMessage(0);
            }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

unsafe fn init_controls(hwnd: HWND, state: &mut GuiState) {
    state.font_regular = unsafe { create_ui_font(-14, 400) };
    state.font_bold = unsafe { create_ui_font(-20, 600) };
    state.bg_brush = unsafe { CreateSolidBrush(rgb(248, 250, 252)) };

    let mut client: RECT = unsafe { std::mem::zeroed() };
    unsafe { GetClientRect(hwnd, &mut client) };
    let cw = client.right - client.left;
    let margin = 24;
    let full_w = cw - margin * 2;

    let cls_static = to_wide_str("STATIC");
    let cls_edit = to_wide_str("EDIT");
    let cls_button = to_wide_str("BUTTON");

    let lbl_dest = to_wide_str("Pasta de instalação:");
    let hwnd_lbl_dest = unsafe {
        CreateWindowExW(
            0,
            cls_static.as_ptr(),
            lbl_dest.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            margin,
            HEADER_H + 16,
            full_w,
            20,
            hwnd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    unsafe { set_ctrl_font(hwnd_lbl_dest, state.font_regular) };

    let initial_path = to_wide_null(state.initial.target_dir.as_os_str());
    state.hwnd_path = unsafe {
        CreateWindowExW(
            WS_EX_CLIENTEDGE,
            cls_edit.as_ptr(),
            initial_path.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
            margin,
            HEADER_H + 40,
            full_w,
            26,
            hwnd,
            ID_PATH_EDIT as *mut _,
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    unsafe { set_ctrl_font(state.hwnd_path, state.font_regular) };

    let txt_chk_desktop = to_wide_str("Criar atalho na Área de Trabalho");
    state.hwnd_chk_desktop = unsafe {
        CreateWindowExW(
            0,
            cls_button.as_ptr(),
            txt_chk_desktop.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            margin,
            HEADER_H + 78,
            full_w,
            22,
            hwnd,
            ID_CHK_DESKTOP as *mut _,
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    unsafe {
        set_ctrl_font(state.hwnd_chk_desktop, state.font_regular);
        let checked = if state.initial.create_desktop_shortcut {
            BST_CHECKED
        } else {
            BST_UNCHECKED
        };
        SendMessageW(state.hwnd_chk_desktop, BM_SETCHECK, checked as WPARAM, 0);
    }

    let txt_chk_launch = to_wide_str("Abrir o Workploy após concluir");
    state.hwnd_chk_launch = unsafe {
        CreateWindowExW(
            0,
            cls_button.as_ptr(),
            txt_chk_launch.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            margin,
            HEADER_H + 106,
            full_w,
            22,
            hwnd,
            ID_CHK_LAUNCH as *mut _,
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    unsafe {
        set_ctrl_font(state.hwnd_chk_launch, state.font_regular);
        let checked = if state.initial.launch_after {
            BST_CHECKED
        } else {
            BST_UNCHECKED
        };
        SendMessageW(state.hwnd_chk_launch, BM_SETCHECK, checked as WPARAM, 0);
    }

    let txt_status = to_wide_str("Pronto para instalar o Workploy.");
    state.hwnd_status = unsafe {
        CreateWindowExW(
            0,
            cls_static.as_ptr(),
            txt_status.as_ptr(),
            WS_CHILD | WS_VISIBLE,
            margin,
            HEADER_H + 144,
            full_w,
            20,
            hwnd,
            ID_STATUS_LBL as *mut _,
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    unsafe { set_ctrl_font(state.hwnd_status, state.font_regular) };

    state.hwnd_progress = unsafe {
        CreateWindowExW(
            0,
            PROGRESS_CLASSW,
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE,
            margin,
            HEADER_H + 168,
            full_w,
            18,
            hwnd,
            ID_PROGRESS as *mut _,
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    unsafe {
        SendMessageW(state.hwnd_progress, PBM_SETRANGE32, 0, 100);
        SendMessageW(state.hwnd_progress, PBM_SETPOS, 0, 0);
    }

    let btn_y = client.bottom - 48;
    let btn_install_w = 132;
    let btn_cancel_w = 100;
    let btn_h = 30;
    let cancel_x = cw - margin - btn_cancel_w;
    let install_x = cancel_x - 10 - btn_install_w;

    let txt_install = to_wide_str("Instalar agora");
    state.hwnd_btn_install = unsafe {
        CreateWindowExW(
            0,
            cls_button.as_ptr(),
            txt_install.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
            install_x,
            btn_y,
            btn_install_w,
            btn_h,
            hwnd,
            ID_BTN_INSTALL as *mut _,
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    unsafe { set_ctrl_font(state.hwnd_btn_install, state.font_regular) };

    let txt_cancel = to_wide_str("Cancelar");
    state.hwnd_btn_cancel = unsafe {
        CreateWindowExW(
            0,
            cls_button.as_ptr(),
            txt_cancel.as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON as u32,
            cancel_x,
            btn_y,
            btn_cancel_w,
            btn_h,
            hwnd,
            ID_BTN_CANCEL as *mut _,
            std::ptr::null_mut(),
            std::ptr::null(),
        )
    };
    unsafe { set_ctrl_font(state.hwnd_btn_cancel, state.font_regular) };
    unsafe { InvalidateRect(hwnd, std::ptr::null(), 1) };
}

unsafe fn paint_header(hwnd: HWND, state: &GuiState) {
    let mut ps: PAINTSTRUCT = unsafe { std::mem::zeroed() };
    let hdc = unsafe { BeginPaint(hwnd, &mut ps) };

    let mut client: RECT = unsafe { std::mem::zeroed() };
    unsafe { GetClientRect(hwnd, &mut client) };

    let header_rect = RECT {
        left: 0,
        top: 0,
        right: client.right,
        bottom: HEADER_H,
    };
    let body_rect = RECT {
        left: 0,
        top: HEADER_H,
        right: client.right,
        bottom: client.bottom,
    };

    let header_brush = unsafe { CreateSolidBrush(rgb(15, 23, 42)) };
    unsafe {
        FillRect(hdc, &header_rect, header_brush);
        DeleteObject(header_brush as *mut _);
        FillRect(hdc, &body_rect, state.bg_brush);
    }

    if !state.header_icon.is_null() {
        unsafe {
            DrawIconEx(
                hdc,
                22,
                18,
                state.header_icon,
                40,
                40,
                0,
                std::ptr::null_mut(),
                0x0003, // DI_NORMAL
            );
        }
    }

    unsafe {
        SetBkMode(hdc, TRANSPARENT as i32);
        let prev_font = windows_sys::Win32::Graphics::Gdi::SelectObject(hdc, state.font_bold as *mut _);
        SetTextColor(hdc, rgb(248, 250, 252));
        let mut title_rect = RECT {
            left: 74,
            top: 16,
            right: client.right - 20,
            bottom: 42,
        };
        let mut title = to_wide_str("Workploy");
        DrawTextW(
            hdc,
            title.as_mut_ptr(),
            -1,
            &mut title_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        windows_sys::Win32::Graphics::Gdi::SelectObject(hdc, state.font_regular as *mut _);
        SetTextColor(hdc, rgb(148, 163, 184));
        let mut sub_rect = RECT {
            left: 74,
            top: 40,
            right: client.right - 20,
            bottom: 62,
        };
        let mut sub = to_wide_str(&format!("Versão {APP_VERSION} • Assistente de Instalação"));
        DrawTextW(
            hdc,
            sub.as_mut_ptr(),
            -1,
            &mut sub_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );

        windows_sys::Win32::Graphics::Gdi::SelectObject(hdc, prev_font);
        EndPaint(hwnd, &ps);
    }
}

unsafe fn set_controls_enabled(state: &GuiState, enabled: bool) {
    let flag = if enabled { 1 } else { 0 };
    unsafe {
        EnableWindow(state.hwnd_path, flag);
        EnableWindow(state.hwnd_chk_desktop, flag);
        EnableWindow(state.hwnd_chk_launch, flag);
        EnableWindow(state.hwnd_btn_install, flag);
        EnableWindow(state.hwnd_btn_cancel, flag);
    }
}

fn read_window_text(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; len as usize + 1];
    let copied = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if copied <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..copied as usize])
}

fn start_gui_install(hwnd: HWND, state: &mut GuiState) {
    let raw_path = read_window_text(state.hwnd_path);
    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        show_message(
            hwnd,
            "Workploy — Pasta inválida",
            "Informe uma pasta de instalação válida.",
            MB_OK | MB_ICONERROR,
        );
        return;
    }

    let desktop_checked = unsafe {
        SendMessageW(state.hwnd_chk_desktop, BM_GETCHECK, 0, 0) as u32 == BST_CHECKED
    };
    let launch_checked = unsafe {
        SendMessageW(state.hwnd_chk_launch, BM_GETCHECK, 0, 0) as u32 == BST_CHECKED
    };

    let opts = InstallOptions {
        target_dir: PathBuf::from(trimmed),
        create_desktop_shortcut: desktop_checked && state.initial.create_shortcuts,
        create_shortcuts: state.initial.create_shortcuts,
        register_uninstall: state.initial.register_uninstall,
        launch_after: launch_checked,
    };

    state.installing = true;
    unsafe { set_controls_enabled(state, false) };

    let hwnd_raw = hwnd as usize;
    let status_arc = Arc::clone(&state.status_text);
    let result_arc = Arc::clone(&state.result_box);

    thread::spawn(move || {
        let target_hwnd = hwnd_raw as HWND;
        let res = install_with_payload(&opts, PAYLOAD_GZ, PAYLOAD_ICO, &mut |pct, msg| {
            if let Ok(mut guard) = status_arc.lock() {
                *guard = msg.to_string();
            }
            unsafe {
                PostMessageW(target_hwnd, WM_INSTALL_PROGRESS, pct as WPARAM, 0);
            }
        });
        if let Ok(mut guard) = result_arc.lock() {
            *guard = Some(res);
        }
        unsafe {
            PostMessageW(target_hwnd, WM_INSTALL_DONE, 0, 0);
        }
    });
}

// ============================================================================
// Testes Unitários
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(prefix: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let path = env::temp_dir().join(format!(
                "workploy-setup-test-{prefix}-{}-{nanos}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("criar temp dir");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn gzip_bytes(data: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(data).expect("write gz");
        encoder.finish().expect("finish gz")
    }

    #[test]
    fn parses_cli_flags_correctly() {
        let parsed = parse_args([
            "--silent",
            "--dir",
            r"C:\Custom\Widder",
            "--no-desktop",
            "--no-shortcuts",
            "--no-registry",
            "--launch",
        ])
        .expect("parse args");

        assert_eq!(
            parsed,
            CliArgs {
                silent: true,
                uninstall: false,
                dir: Some(PathBuf::from(r"C:\Custom\Widder")),
                no_desktop: true,
                no_shortcuts: true,
                no_registry: true,
                launch: true,
                help: false,
            }
        );
    }

    #[test]
    fn parses_equals_dir_and_uninstall() {
        let parsed = parse_args(["--uninstall", "--silent", r"--dir=D:\Apps\Widder"])
            .expect("parse equals dir");
        assert!(parsed.uninstall);
        assert!(parsed.silent);
        assert_eq!(parsed.dir, Some(PathBuf::from(r"D:\Apps\Widder")));
    }

    #[test]
    fn rejects_missing_dir_and_unknown_flags() {
        assert!(parse_args(["--dir"]).is_err());
        assert!(parse_args(["--dir="]).is_err());
        assert!(parse_args(["--nao-existe"]).is_err());
    }

    #[test]
    fn default_install_dir_uses_localappdata() {
        let dir = default_install_dir_from(Some(OsStr::new(r"C:\Users\dev\AppData\Local")));
        assert_eq!(dir, PathBuf::from(r"C:\Users\dev\AppData\Local\Programs\Workploy"));
    }

    #[test]
    fn gzip_roundtrip_and_size_kb() {
        let original = b"MZ-workploy-payload-test-1234567890".repeat(250);
        let compressed = gzip_bytes(&original);
        let tmp = TempDir::new("gz");
        let dest = tmp.path.join("out.exe");
        let written = decompress_to_file(&compressed, &dest).expect("decompress");
        assert_eq!(written, original.len() as u64);
        assert_eq!(fs::read(&dest).expect("read out"), original);
        assert!(compute_estimated_size_kb(written) >= 8);
    }

    #[test]
    fn install_upgrade_and_uninstall_preserve_data_in_sandbox() {
        let tmp = TempDir::new("sandbox");
        let target = tmp.path.join("Workploy");
        let fake_exe_v1 = b"MZ-FAKE-WORKPLOY-V1".repeat(64);
        let fake_exe_v2 = b"MZ-FAKE-WORKPLOY-V2-UPGRADED".repeat(64);
        let fake_ico = b"\x00\x00\x01\x00ICON";

        let opts = InstallOptions {
            target_dir: target.clone(),
            create_desktop_shortcut: false,
            create_shortcuts: false,
            register_uninstall: false,
            launch_after: false,
        };

        let gz_v1 = gzip_bytes(&fake_exe_v1);
        let mut stages = Vec::new();
        let report_v1 = install_with_payload(&opts, &gz_v1, fake_ico, &mut |pct, msg| {
            stages.push((pct, msg.to_string()));
        })
        .expect("install v1");

        assert_eq!(report_v1.exe_bytes, fake_exe_v1.len() as u64);
        assert!(report_v1.ico_written);
        assert!(report_v1.uninstaller_written);
        assert!(report_v1.shortcuts_created.is_empty());
        assert!(!report_v1.registry_written);
        assert_eq!(fs::read(target.join(EXE_NAME)).expect("read v1"), fake_exe_v1);
        assert_eq!(fs::read(target.join(ICON_NAME)).expect("read ico"), fake_ico);
        assert!(target.join(UNINSTALL_EXE_NAME).exists());
        assert_eq!(stages.last().map(|s| s.0), Some(100));

        // Simula pasta de dados dentro do diretório de teste para garantir preservação na desinstalação.
        let data_dir = target.join("data");
        fs::create_dir_all(&data_dir).expect("create data dir");
        fs::write(data_dir.join("config.json"), b"{}").expect("write config");

        // Upgrade sobre instalação existente.
        let gz_v2 = gzip_bytes(&fake_exe_v2);
        let report_v2 = install_with_payload(&opts, &gz_v2, fake_ico, &mut |_, _| {})
            .expect("upgrade v2");
        assert_eq!(report_v2.exe_bytes, fake_exe_v2.len() as u64);
        assert_eq!(fs::read(target.join(EXE_NAME)).expect("read v2"), fake_exe_v2);

        // Desinstalação limpa sem remover dados.
        let un_opts = UninstallOptions {
            target_dir: target.clone(),
            remove_shortcuts: false,
            remove_registry: false,
        };
        let un_report = uninstall_with_options(&un_opts, &mut |_| {}).expect("uninstall");
        assert!(!un_report.registry_removed);
        assert!(!un_report.self_delete_scheduled);
        assert!(!target.join(EXE_NAME).exists());
        assert!(!target.join(ICON_NAME).exists());
        assert!(!target.join(UNINSTALL_EXE_NAME).exists());
        assert!(data_dir.join("config.json").exists());
    }

    #[test]
    fn rejects_empty_payload() {
        let tmp = TempDir::new("empty");
        let opts = InstallOptions {
            target_dir: tmp.path.join("Workploy"),
            create_desktop_shortcut: false,
            create_shortcuts: false,
            register_uninstall: false,
            launch_after: false,
        };
        assert!(install_with_payload(&opts, &[], &[], &mut |_, _| {}).is_err());
    }

    #[test]
    fn parses_launch_flag_default_and_enabled() {
        let off = parse_args(["--silent"]).expect("parse silent only");
        assert!(off.silent);
        assert!(!off.launch);

        let on = parse_args(["--silent", "--launch"]).expect("parse silent + launch");
        assert!(on.silent);
        assert!(on.launch);
    }

    #[test]
    fn replace_file_with_retry_succeeds_after_lock_release() {
        let tmp = TempDir::new("retry-lock");
        let dest = tmp.path.join("workploy.exe");
        let staged = tmp.path.join("workploy.exe.tmp");
        fs::write(&dest, b"old-binary").expect("write old");
        fs::write(&staged, b"new-binary").expect("write staged");

        // Mantém o arquivo aberto exclusivamente em Windows para forçar retry e libera em seguida.
        let handle = OpenOptions::new().read(true).write(true).open(&dest).expect("open lock");
        let releaser = thread::spawn(move || {
            thread::sleep(Duration::from_millis(180));
            drop(handle);
        });

        replace_file_with_retry(&staged, &dest, 10, Duration::from_millis(100))
            .expect("retry should succeed once handle is dropped");
        releaser.join().expect("join releaser");
        assert_eq!(fs::read(&dest).expect("read replaced"), b"new-binary");
    }

    #[test]
    fn self_delete_raw_arg_preserves_spaces_without_escapes() {
        let dir = Path::new(r"C:\Users\Maria Silva\AppData\Local\Programs\Workploy");
        let exe = dir.join("uninstall.exe");
        let raw = build_self_delete_raw_arg(&exe, dir);

        assert_eq!(
            raw,
            r#"/C "ping 127.0.0.1 -n 3 > NUL & del /F /Q "C:\Users\Maria Silva\AppData\Local\Programs\Workploy\uninstall.exe" > NUL 2>&1 & rmdir "C:\Users\Maria Silva\AppData\Local\Programs\Workploy" > NUL 2>&1""#
        );
        assert!(!raw.contains(r#"\""#));
    }
}
