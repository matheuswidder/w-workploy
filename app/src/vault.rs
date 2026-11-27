use std::sync::OnceLock;
#[cfg(any(test, not(target_os = "windows")))]
use std::{collections::HashMap, sync::Mutex};

pub const SERVICE_NAME: &str = "workploy";
const LEGACY_SERVICE_NAME: &str = "widder-ci";
pub const KEY_GITHUB_PAT: &str = "github:pat";

pub fn vps_password_key(vps_id: &str) -> String {
    format!("vps:{vps_id}:password")
}

pub fn vps_webhook_secret_key(vps_id: &str) -> String {
    format!("vps:{vps_id}:webhook")
}

pub fn vps_runner_token_key(vps_id: &str) -> String {
    format!("vps:{vps_id}:runner-token")
}

/// Chave de pareamento de um par de VPS (idA, idB). O par é normalizado
/// (ordem lexicográfica) para que os dois lados caiam na mesma chave. Serve
/// só para o repasse de fluxo encadeado entre runners — é mais estreita que o
/// `runner-token`, que concede o token admin do runner.
pub fn vps_pair_key(a: &str, b: &str) -> String {
    let (x, y) = if a <= b { (a, b) } else { (b, a) };
    format!("vps:pair:{x}:{y}")
}

#[cfg(test)]
fn memory_store() -> &'static Mutex<HashMap<String, String>> {
    static MEM: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    MEM.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[cfg(all(not(test), target_os = "windows"))]
fn win_store() -> Result<&'static std::sync::Arc<windows_native_keyring_store::Store>, String> {
    static STORE: OnceLock<Result<std::sync::Arc<windows_native_keyring_store::Store>, String>> =
        OnceLock::new();
    let res = STORE.get_or_init(|| {
        windows_native_keyring_store::Store::new()
            .map_err(|e| format!("Não foi possível abrir o Cofre do Windows: {e}"))
    });
    match res {
        Ok(s) => Ok(s),
        Err(e) => Err(e.clone()),
    }
}

#[cfg(any(test, not(target_os = "windows")))]
fn load_vault_file(path: &std::path::Path) -> HashMap<String, String> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    serde_json::from_str::<HashMap<String, String>>(&raw).unwrap_or_default()
}

#[cfg(any(test, not(target_os = "windows")))]
fn write_vault_file(path: &std::path::Path, map: &HashMap<String, String>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Falha ao preparar diretório do cofre: {e}"))?;
    }
    let json = serde_json::to_string_pretty(map)
        .map_err(|e| format!("Falha ao serializar cofre local: {e}"))?;
    let tmp = path.with_extension("json.tmp");
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| format!("Falha ao gravar cofre local: {e}"))?;
        f.write_all(json.as_bytes())
            .map_err(|e| format!("Falha ao escrever cofre local: {e}"))?;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&tmp, json.as_bytes())
            .map_err(|e| format!("Falha ao gravar cofre local: {e}"))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("Falha ao atualizar cofre local: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(all(not(test), not(target_os = "windows")))]
fn fallback_vault_path() -> std::path::PathBuf {
    crate::model::data_dir().join(".vault.json")
}

#[cfg(all(not(test), not(target_os = "windows")))]
fn fallback_store() -> &'static Mutex<HashMap<String, String>> {
    static MEM: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    MEM.get_or_init(|| Mutex::new(load_vault_file(&fallback_vault_path())))
}

pub fn save_secret(key: &str, value: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("Chave de segredo vazia não é permitida.".into());
    }

    #[cfg(test)]
    {
        let mut guard = memory_store()
            .lock()
            .map_err(|_| "Falha ao acessar o cofre em memória.".to_string())?;
        guard.insert(key.to_string(), value.to_string());
        Ok(())
    }

    #[cfg(all(not(test), target_os = "windows"))]
    {
        use keyring_core::api::CredentialStoreApi;
        let store = win_store()?;
        let entry = store
            .build(SERVICE_NAME, key, None)
            .map_err(|e| format!("Erro ao preparar credencial no Cofre do Windows: {e}"))?;
        entry
            .set_password(value)
            .map_err(|e| format!("Erro ao gravar segredo no Cofre do Windows: {e}"))
    }

    #[cfg(all(not(test), not(target_os = "windows")))]
    {
        let mut guard = fallback_store()
            .lock()
            .map_err(|_| "Falha ao acessar o cofre em memória.".to_string())?;
        guard.insert(key.to_string(), value.to_string());
        write_vault_file(&fallback_vault_path(), &guard)
    }
}

pub fn load_secret(key: &str) -> Result<Option<String>, String> {
    let key = key.trim();
    if key.is_empty() {
        return Ok(None);
    }

    #[cfg(test)]
    {
        let guard = memory_store()
            .lock()
            .map_err(|_| "Falha ao acessar o cofre em memória.".to_string())?;
        Ok(guard.get(key).cloned())
    }

    #[cfg(all(not(test), target_os = "windows"))]
    {
        use keyring_core::api::CredentialStoreApi;
        let store = win_store()?;
        let entry = store
            .build(SERVICE_NAME, key, None)
            .map_err(|e| format!("Erro ao preparar leitura no Cofre do Windows: {e}"))?;
        match entry.get_password() {
            Ok(val) => Ok(Some(val)),
            Err(keyring_core::Error::NoEntry) => {
                let legacy = store
                    .build(LEGACY_SERVICE_NAME, key, None)
                    .map_err(|e| format!("Erro ao preparar leitura legada no Cofre do Windows: {e}"))?;
                match legacy.get_password() {
                    Ok(val) => Ok(Some(val)),
                    Err(keyring_core::Error::NoEntry) => Ok(None),
                    Err(e) => Err(format!("Erro ao ler segredo do Cofre do Windows: {e}")),
                }
            }
            Err(e) => Err(format!("Erro ao ler segredo do Cofre do Windows: {e}")),
        }
    }

    #[cfg(all(not(test), not(target_os = "windows")))]
    {
        let guard = fallback_store()
            .lock()
            .map_err(|_| "Falha ao acessar o cofre em memória.".to_string())?;
        Ok(guard.get(key).cloned())
    }
}

pub fn delete_secret(key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Ok(());
    }

    #[cfg(test)]
    {
        let mut guard = memory_store()
            .lock()
            .map_err(|_| "Falha ao acessar o cofre em memória.".to_string())?;
        guard.remove(key);
        Ok(())
    }

    #[cfg(all(not(test), target_os = "windows"))]
    {
        use keyring_core::api::CredentialStoreApi;
        let store = win_store()?;
        let entry = store
            .build(SERVICE_NAME, key, None)
            .map_err(|e| format!("Erro ao preparar remoção no Cofre do Windows: {e}"))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("Erro ao remover segredo do Cofre do Windows: {e}")),
        }
    }

    #[cfg(all(not(test), not(target_os = "windows")))]
    {
        let mut guard = fallback_store()
            .lock()
            .map_err(|_| "Falha ao acessar o cofre em memória.".to_string())?;
        guard.remove(key);
        write_vault_file(&fallback_vault_path(), &guard)
    }
}

pub fn has_secret(key: &str) -> bool {
    matches!(load_secret(key), Ok(Some(v)) if !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_overwrite_and_delete_in_memory_vault() {
        let _guard = test_lock();
        let k = "test:vault:roundtrip";
        let _ = delete_secret(k);
        assert_eq!(load_secret(k).unwrap(), None);
        assert!(!has_secret(k));

        save_secret(k, "segredo-123").unwrap();
        assert_eq!(load_secret(k).unwrap(), Some("segredo-123".into()));
        assert!(has_secret(k));

        save_secret(k, "segredo-456").unwrap();
        assert_eq!(load_secret(k).unwrap(), Some("segredo-456".into()));

        delete_secret(k).unwrap();
        assert_eq!(load_secret(k).unwrap(), None);
        assert!(!has_secret(k));
        // Remoção idempotente não deve falhar
        delete_secret(k).unwrap();
    }

    #[test]
    fn vps_pair_key_normalizes_order_between_the_two_vps() {
        // O par (a, b) e o par (b, a) precisam cair na mesma chave: é o que
        // permite que as duas VPS compartilhem o segredo do repasse.
        assert_eq!(
            vps_pair_key("vps-a", "vps-b"),
            vps_pair_key("vps-b", "vps-a")
        );
        assert_eq!(vps_pair_key("vps-a", "vps-b"), "vps:pair:vps-a:vps-b");
        assert_eq!(vps_pair_key("vps-b", "vps-a"), "vps:pair:vps-a:vps-b");
        // Pares diferentes não podem colidir.
        assert_ne!(
            vps_pair_key("vps-a", "vps-b"),
            vps_pair_key("vps-a", "vps-c")
        );
        // A chave de par é distinta do token admin do runner.
        assert_ne!(vps_pair_key("vps-a", "vps-b"), vps_runner_token_key("vps-a"));
    }

    #[test]
    fn rejects_empty_key_and_formats_vps_keys() {
        let _guard = test_lock();
        assert_eq!(SERVICE_NAME, "workploy");
        assert_eq!(LEGACY_SERVICE_NAME, "widder-ci");
        assert!(save_secret("   ", "val").is_err());
        assert_eq!(vps_password_key("vps-1"), "vps:vps-1:password");
        assert_eq!(vps_webhook_secret_key("vps-1"), "vps:vps-1:webhook");
        assert_eq!(vps_runner_token_key("vps-1"), "vps:vps-1:runner-token");
    }

    #[test]
    fn file_vault_persists_and_reloads_secrets() {
        let tmp = std::env::temp_dir().join(format!("workploy-vault-test-{}", crate::model::new_id("dir")));
        let path = tmp.join(".vault.json");
        let mut map = HashMap::new();
        map.insert(KEY_GITHUB_PAT.into(), "ghp_test_token_123".into());
        map.insert(vps_password_key("vps-1"), "ssh_pass_456".into());

        write_vault_file(&path, &map).unwrap();
        let reloaded = load_vault_file(&path);
        assert_eq!(reloaded.get(KEY_GITHUB_PAT).map(String::as_str), Some("ghp_test_token_123"));
        assert_eq!(reloaded.get(&vps_password_key("vps-1")).map(String::as_str), Some("ssh_pass_456"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
