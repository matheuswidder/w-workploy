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

#[cfg(all(not(test), not(target_os = "windows")))]
fn fallback_store() -> &'static Mutex<HashMap<String, String>> {
    static MEM: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    MEM.get_or_init(|| Mutex::new(HashMap::new()))
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
        Ok(())
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
        Ok(())
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
    fn rejects_empty_key_and_formats_vps_keys() {
        let _guard = test_lock();
        assert_eq!(SERVICE_NAME, "workploy");
        assert_eq!(LEGACY_SERVICE_NAME, "widder-ci");
        assert!(save_secret("   ", "val").is_err());
        assert_eq!(vps_password_key("vps-1"), "vps:vps-1:password");
        assert_eq!(vps_webhook_secret_key("vps-1"), "vps:vps-1:webhook");
        assert_eq!(vps_runner_token_key("vps-1"), "vps:vps-1:runner-token");
    }
}
