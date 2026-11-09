//! Chunked Windows credentials with read compatibility for pre-0.2.2 accounts.
use crate::Result;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

const CHUNK_SIZE: usize = 1024;
const MAX_CHUNKS: usize = 64;

#[derive(Serialize, Deserialize)]
struct Manifest {
    generation: String,
    count: usize,
}

trait Vault {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}

struct WindowsVault;
impl Vault for WindowsVault {
    fn get(&self, key: &str) -> Result<Option<String>> {
        match crate::entry(key)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err("Windows could not read saved mail credentials. Sign in again.".into()),
        }
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        crate::entry(key)?.set_password(value).map_err(|_| {
            "Windows could not save mail credentials. Please try signing in again.".into()
        })
    }
    fn delete(&self, key: &str) -> Result<()> {
        match crate::entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => {
                Err("Windows could not clear the saved mail credentials. Please try again.".into())
            }
        }
    }
}

fn manifest_key(key: &str) -> String {
    format!("mail-v2:{key}:manifest")
}
fn part_key(key: &str, manifest: &Manifest, part: usize) -> String {
    format!("mail-v2:{key}:{}:{part}", manifest.generation)
}
fn read_manifest(vault: &impl Vault, key: &str) -> Result<Option<Manifest>> {
    let Some(value) = vault.get(&manifest_key(key))? else {
        return Ok(None);
    };
    parse_manifest(&value).map(Some)
}
fn parse_manifest(value: &str) -> Result<Manifest> {
    let manifest: Manifest = serde_json::from_str(value)
        .map_err(|_| "Saved mail credentials are invalid. Sign in again.")?;
    if manifest.count == 0
        || manifest.count > MAX_CHUNKS
        || manifest.generation.len() != 16
        || !manifest
            .generation
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("Saved mail credentials are invalid. Sign in again.".into());
    }
    Ok(manifest)
}
fn previous_for_replacement(vault: &impl Vault, key: &str) -> Result<Option<Manifest>> {
    // Access errors still propagate. A malformed manifest can be replaced by a
    // fresh successful sign-in, but cannot be trusted to name old chunks.
    Ok(vault
        .get(&manifest_key(key))?
        .and_then(|value| parse_manifest(&value).ok()))
}

fn load_from<T: DeserializeOwned>(vault: &impl Vault, key: &str) -> Result<T> {
    if let Some(manifest) = read_manifest(vault, key)? {
        let mut encoded = String::new();
        for part in 0..manifest.count {
            let value = vault
                .get(&part_key(key, &manifest, part))?
                .ok_or("Saved mail credentials are incomplete. Sign in again.")?;
            if value.len() > CHUNK_SIZE || !value.is_ascii() {
                return Err("Saved mail credentials are invalid. Sign in again.".into());
            }
            encoded.push_str(&value);
        }
        let bytes = STANDARD
            .decode(encoded)
            .map_err(|_| "Saved mail credentials are invalid. Sign in again.")?;
        serde_json::from_slice(&bytes)
            .map_err(|_| "Saved mail credentials are invalid. Sign in again.".into())
    } else {
        // Preserve the original account's application ID and refresh token.
        // The first successful refresh migrates storage atomically.
        let legacy = vault.get(key)?.ok_or("Sign in to this account again.")?;
        serde_json::from_str(&legacy)
            .map_err(|_| "Saved mail credentials are invalid. Sign in again.".into())
    }
}

fn cleanup(vault: &impl Vault, key: &str, manifest: &Manifest) {
    for part in 0..manifest.count {
        let _ = vault.delete(&part_key(key, manifest, part));
    }
}

fn save_to<T: Serialize>(vault: &impl Vault, key: &str, value: &T) -> Result<()> {
    let encoded = STANDARD
        .encode(serde_json::to_vec(value).map_err(|_| "Could not encode mail credentials")?);
    let count = encoded.len().div_ceil(CHUNK_SIZE);
    if count == 0 || count > MAX_CHUNKS {
        return Err("The mail provider returned credentials too large to save.".into());
    }
    let previous = previous_for_replacement(vault, key)?;
    let manifest = Manifest {
        generation: format!("{:016x}", rand::random::<u64>()),
        count,
    };
    for (part, bytes) in encoded.as_bytes().chunks(CHUNK_SIZE).enumerate() {
        let text = std::str::from_utf8(bytes).expect("base64 is ASCII");
        if let Err(error) = vault.set(&part_key(key, &manifest, part), text) {
            cleanup(vault, key, &manifest);
            return Err(error);
        }
    }
    let encoded_manifest = serde_json::to_string(&manifest)
        .map_err(|_| "Could not encode the mail credential manifest")?;
    if let Err(error) = vault.set(&manifest_key(key), &encoded_manifest) {
        cleanup(vault, key, &manifest);
        return Err(error);
    }
    if let Some(previous) = previous {
        cleanup(vault, key, &previous);
    }
    let _ = vault.delete(key);
    Ok(())
}

fn delete_from(vault: &impl Vault, key: &str) -> Result<()> {
    if let Some(manifest) = previous_for_replacement(vault, key)? {
        for part in 0..manifest.count {
            vault.delete(&part_key(key, &manifest, part))?;
        }
    }
    vault.delete(&manifest_key(key))?;
    vault.delete(key)
}

pub fn load<T: DeserializeOwned>(key: &str) -> Result<T> {
    load_from(&WindowsVault, key)
}
pub fn save<T: Serialize>(key: &str, value: &T) -> Result<()> {
    save_to(&WindowsVault, key, value)
}
pub fn delete(key: &str) -> Result<()> {
    delete_from(&WindowsVault, key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::{cell::RefCell, collections::HashMap};

    #[derive(Default)]
    struct MemoryVault {
        data: RefCell<HashMap<String, String>>,
        fail_key: RefCell<Option<String>>,
    }
    impl Vault for MemoryVault {
        fn get(&self, key: &str) -> Result<Option<String>> {
            Ok(self.data.borrow().get(key).cloned())
        }
        fn set(&self, key: &str, value: &str) -> Result<()> {
            if self
                .fail_key
                .borrow()
                .as_deref()
                .is_some_and(|suffix| key.ends_with(suffix))
            {
                return Err("Simulated credential write failure".into());
            }
            assert!(value.encode_utf16().count() <= CHUNK_SIZE);
            self.data.borrow_mut().insert(key.into(), value.into());
            Ok(())
        }
        fn delete(&self, key: &str) -> Result<()> {
            self.data.borrow_mut().remove(key);
            Ok(())
        }
    }

    #[test]
    fn long_mail_tokens_round_trip_rotate_and_delete_all_parts() {
        let vault = MemoryVault::default();
        let value = json!({"access_token":"a".repeat(8000),"refresh_token":"r".repeat(6000),"expires_at":123});
        save_to(&vault, "test-account", &value).unwrap();
        assert!(vault.data.borrow().len() > 10);
        assert_eq!(load_from::<Value>(&vault, "test-account").unwrap(), value);
        let next = json!({"access_token":"new","refresh_token":"rotated"});
        save_to(&vault, "test-account", &next).unwrap();
        assert_eq!(vault.data.borrow().len(), 2);
        assert_eq!(load_from::<Value>(&vault, "test-account").unwrap(), next);
        delete_from(&vault, "test-account").unwrap();
        assert!(vault.data.borrow().is_empty());
    }

    #[test]
    fn legacy_account_credentials_load_and_migrate_without_losing_refresh_tokens() {
        let vault = MemoryVault::default();
        let value = json!({"access_token":"legacy","refresh_token":"keep-me"});
        vault
            .data
            .borrow_mut()
            .insert("test-account".into(), value.to_string());
        assert_eq!(load_from::<Value>(&vault, "test-account").unwrap(), value);
        save_to(&vault, "test-account", &value).unwrap();
        assert!(!vault.data.borrow().contains_key("test-account"));
        assert_eq!(load_from::<Value>(&vault, "test-account").unwrap(), value);
    }

    #[test]
    fn failed_new_generation_keeps_previous_credentials_and_cleans_partial_parts() {
        let vault = MemoryVault::default();
        let original = json!({"refresh_token":"original"});
        save_to(&vault, "test-account", &original).unwrap();
        let count = vault.data.borrow().len();
        for failed_suffix in [":1", ":manifest"] {
            *vault.fail_key.borrow_mut() = Some(failed_suffix.into());
            assert!(save_to(
                &vault,
                "test-account",
                &json!({"refresh_token":"new".repeat(2000)})
            )
            .is_err());
            assert_eq!(
                load_from::<Value>(&vault, "test-account").unwrap(),
                original
            );
            assert_eq!(vault.data.borrow().len(), count);
        }
    }

    #[test]
    fn corrupted_manifest_blocks_stale_load_but_allows_explicit_sign_in_and_disconnect() {
        let vault = MemoryVault::default();
        vault.data.borrow_mut().insert(
            "test-account".into(),
            json!({"refresh_token":"stale"}).to_string(),
        );
        vault
            .data
            .borrow_mut()
            .insert(manifest_key("test-account"), "{bad}".into());
        assert!(load_from::<Value>(&vault, "test-account").is_err());
        *vault.fail_key.borrow_mut() = Some(":manifest".into());
        assert!(save_to(&vault, "test-account", &json!({"refresh_token":"new"})).is_err());
        assert!(load_from::<Value>(&vault, "test-account").is_err());
        assert!(vault.data.borrow().contains_key("test-account"));
        *vault.fail_key.borrow_mut() = None;
        let fresh = json!({"refresh_token":"fresh"});
        save_to(&vault, "test-account", &fresh).unwrap();
        assert_eq!(load_from::<Value>(&vault, "test-account").unwrap(), fresh);
        assert!(!vault.data.borrow().contains_key("test-account"));
        vault
            .data
            .borrow_mut()
            .insert(manifest_key("test-account"), "{bad}".into());
        vault
            .data
            .borrow_mut()
            .insert("test-account".into(), "legacy".into());
        vault
            .data
            .borrow_mut()
            .insert("mail-v2:test-account:unknown:0".into(), "untouched".into());
        delete_from(&vault, "test-account").unwrap();
        assert!(!vault
            .data
            .borrow()
            .contains_key(&manifest_key("test-account")));
        assert!(!vault.data.borrow().contains_key("test-account"));
        assert!(vault
            .data
            .borrow()
            .contains_key("mail-v2:test-account:unknown:0"));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "Opt-in Windows Credential Manager smoke; writes and removes only a unique synthetic test ID"]
    fn windows_credential_manager_long_mail_token_round_trip() {
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = delete(&self.0);
            }
        }
        let key = format!(
            "yas-mail-token-test-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        );
        let _cleanup = Cleanup(key.clone());
        let original = json!({"access_token":"a".repeat(8000),"refresh_token":"r".repeat(6000),"expires_at":123});
        save(&key, &original).expect("Could not save synthetic mail credentials");
        let manifest = read_manifest(&WindowsVault, &key)
            .expect("Could not read synthetic manifest")
            .expect("Synthetic manifest missing");
        assert!(
            manifest.count > 10,
            "Long synthetic credentials must span safe-size chunks"
        );
        let restored: Value = load(&key).expect("Could not load synthetic mail credentials");
        assert!(
            restored == original,
            "Synthetic mail credential round trip mismatch"
        );
        delete(&key).expect("Could not delete synthetic mail credentials");
        assert!(
            WindowsVault.get(&manifest_key(&key)).unwrap().is_none(),
            "Synthetic manifest was not removed"
        );
        assert!(
            WindowsVault.get(&key).unwrap().is_none(),
            "Synthetic legacy credential was not removed"
        );
        for part in 0..manifest.count {
            assert!(
                WindowsVault
                    .get(&part_key(&key, &manifest, part))
                    .unwrap()
                    .is_none(),
                "A synthetic credential chunk was not removed"
            );
        }
    }
}
