//! Public desktop OAuth registrations belong to the Yas Mail publisher.
//! End users only select an account provider; identifiers stay inside Rust.
use crate::Result;
use serde::Serialize;

pub struct Client {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Serialize)]
pub struct ProviderStatus {
    provider: &'static str,
    name: &'static str,
    configured: bool,
}

struct Settings<'a> {
    google_id: Option<&'a str>,
    google_secret: Option<&'a str>,
    microsoft_id: Option<&'a str>,
}

fn unavailable(provider: &str) -> String {
    format!("{} sign-in is unavailable in this build. Please use an updated Yas Mail release with sign-in enabled.", if matches!(provider, "gmail" | "google") { "Google" } else { "Microsoft" })
}

impl Settings<'_> {
    fn client(&self, provider: &str) -> Result<Client> {
        let (id, secret) = match provider {
            "gmail" | "google" => (self.google_id, self.google_secret.unwrap_or_default()),
            "outlook" | "onedrive" => (self.microsoft_id, ""),
            _ => return Err("Choose Gmail or Outlook to sign in".into()),
        };
        let id = id.unwrap_or_default().trim();
        let secret = secret.trim();
        crate::oauth_setup::validate_client_id(provider, id).map_err(|_| unavailable(provider))?;
        if secret.len() > 4096 || secret.chars().any(char::is_control) {
            return Err(unavailable(provider));
        }
        Ok(Client {
            client_id: id.into(),
            client_secret: secret.into(),
        })
    }

    fn providers(&self) -> Vec<ProviderStatus> {
        [("gmail", "Gmail"), ("outlook", "Outlook")]
            .into_iter()
            .map(|(provider, name)| ProviderStatus {
                provider,
                name,
                configured: self.client(provider).is_ok(),
            })
            .collect()
    }
}

fn settings() -> Settings<'static> {
    Settings {
        google_id: option_env!("YAS_MAIL_GOOGLE_CLIENT_ID"),
        google_secret: option_env!("YAS_MAIL_GOOGLE_CLIENT_SECRET"),
        microsoft_id: option_env!("YAS_MAIL_MICROSOFT_CLIENT_ID"),
    }
}

pub fn client(provider: &str) -> Result<Client> {
    settings().client(provider)
}

#[tauri::command]
pub fn oauth_providers() -> Vec<ProviderStatus> {
    settings().providers()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured() -> Settings<'static> {
        Settings {
            google_id: Some("123456-test.apps.googleusercontent.com"),
            google_secret: Some("test-desktop-metadata"),
            microsoft_id: Some("12345678-90ab-cdef-1234-567890abcdef"),
        }
    }

    #[test]
    fn empty_or_malformed_publisher_config_never_falls_back_to_user_setup() {
        for bad in [
            None,
            Some(""),
            Some("user@example.com"),
            Some("https://example.com"),
        ] {
            let config = Settings {
                google_id: bad,
                google_secret: None,
                microsoft_id: bad,
            };
            for provider in ["gmail", "outlook", "google", "onedrive"] {
                let error = config.client(provider).err().unwrap();
                assert!(error.contains("unavailable in this build"));
                assert!(!error.contains("client ID"));
                assert!(!error.contains("user@example.com"));
            }
            assert!(config.providers().iter().all(|status| !status.configured));
        }
    }

    #[test]
    fn configured_mail_and_drive_aliases_share_the_publisher_registration() {
        let config = configured();
        assert_eq!(
            config.client("gmail").unwrap().client_id,
            config.client("google").unwrap().client_id
        );
        assert_eq!(
            config.client("outlook").unwrap().client_id,
            config.client("onedrive").unwrap().client_id
        );
        assert_eq!(
            config.client("gmail").unwrap().client_secret,
            "test-desktop-metadata"
        );
        assert!(config.client("outlook").unwrap().client_secret.is_empty());
        assert!(config.client("dropbox").is_err());
    }

    #[test]
    fn provider_status_exposes_only_names_and_readiness_without_credentials() {
        let value = serde_json::to_value(configured().providers()).unwrap();
        assert_eq!(
            value,
            serde_json::json!([
                {"provider":"gmail", "name":"Gmail", "configured":true},
                {"provider":"outlook", "name":"Outlook", "configured":true}
            ])
        );
        let serialized = value.to_string();
        assert!(!serialized.contains("googleusercontent"));
        assert!(!serialized.contains("test-desktop-metadata"));
        assert!(!serialized.contains("client_id"));
    }

    #[test]
    fn providers_are_independent_and_desktop_secret_is_optional() {
        let config = Settings {
            google_id: configured().google_id,
            google_secret: None,
            microsoft_id: None,
        };
        assert!(config.client("google").unwrap().client_secret.is_empty());
        assert!(config.client("outlook").is_err());
        assert_eq!(
            config
                .providers()
                .iter()
                .map(|status| status.configured)
                .collect::<Vec<_>>(),
            [true, false]
        );
    }
}
