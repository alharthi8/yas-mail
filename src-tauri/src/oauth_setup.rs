//! Checks public OAuth application identifiers before starting browser sign-in.

pub fn validate_client_id(provider: &str, client_id: &str) -> Result<(), String> {
    let client_id = client_id.trim();
    let microsoft = matches!(provider, "outlook" | "onedrive");
    let google = matches!(provider, "gmail" | "google");
    if !microsoft && !google && !matches!(provider, "dropbox" | "zoho") {
        return Err("Choose a supported account provider".into());
    }
    if client_id.contains('@') {
        return Err(if microsoft {
            "An email address is not a Microsoft application ID. Copy the Application (client) ID from Microsoft Entra → App registrations. It has the form xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx."
        } else if google {
            "An email address is not a Google application ID. Use the client ID from a Google Cloud OAuth Desktop app; it ends in .apps.googleusercontent.com."
        } else {
            "An email address is not an application client ID. Use the client ID or app key from your provider's application registration."
        }.into());
    }
    if client_id.is_empty()
        || client_id.len() > 4096
        || client_id
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
        || client_id.contains("://")
    {
        return Err("Enter the application client ID from your provider's app registration, not an email address or website URL".into());
    }
    if microsoft {
        let bytes = client_id.as_bytes();
        let valid = bytes.len() == 36
            && bytes.iter().enumerate().all(|(index, byte)| {
                if matches!(index, 8 | 13 | 18 | 23) {
                    *byte == b'-'
                } else {
                    byte.is_ascii_hexdigit()
                }
            });
        if !valid {
            return Err("Use the Microsoft Application (client) ID from Entra → App registrations, in the form xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx. Do not use an email address, tenant ID, or client secret.".into());
        }
    }
    if google {
        let normalized = client_id.to_ascii_lowercase();
        let valid = normalized
            .strip_suffix(".apps.googleusercontent.com")
            .is_some_and(|prefix| {
                prefix
                    .as_bytes()
                    .first()
                    .is_some_and(|byte| byte.is_ascii_alphanumeric())
                    && prefix
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            });
        if !valid {
            return Err("Use the Google Cloud OAuth client ID for a Desktop app. It ends in .apps.googleusercontent.com; do not use your email address or client secret.".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_client_id;

    #[test]
    fn accepts_microsoft_guid_shape_without_restricting_uuid_version() {
        for provider in ["outlook", "onedrive"] {
            for client_id in [
                "12345678-90ab-cdef-1234-567890abcdef",
                "ABCDEF12-3456-7890-ABCD-EF1234567890",
                "  12345678-90ab-cdef-1234-567890abcdef  ",
            ] {
                assert!(validate_client_id(provider, client_id).is_ok());
            }
        }
    }

    #[test]
    fn rejects_microsoft_email_and_malformed_ids_before_authorization() {
        for provider in ["outlook", "onedrive"] {
            let message = validate_client_id(provider, "person@outlook.com").unwrap_err();
            assert!(message.contains("email address"));
            assert!(message.contains("Application (client) ID"));
            for client_id in [
                "",
                "1234567890abcdef1234567890abcdef",
                "12345678_90ab-cdef-1234-567890abcdef",
                "12345678-90ab-cdef-1234-567890abcdeg",
                "{12345678-90ab-cdef-1234-567890abcdef}",
                "12345678-90ab-cdef-1234-567890abcdef extra",
                "https://login.microsoftonline.com/",
                "１２３４５６７８-90ab-cdef-1234-567890abcdef",
            ] {
                assert!(validate_client_id(provider, client_id).is_err());
            }
        }
    }

    #[test]
    fn accepts_google_desktop_client_ids_and_rejects_email_or_wrong_suffix() {
        for provider in ["gmail", "google"] {
            assert!(validate_client_id(
                provider,
                " 1234567890-example.apps.googleusercontent.com "
            )
            .is_ok());
            assert!(
                validate_client_id(provider, "1234567890-EXAMPLE.APPS.GOOGLEUSERCONTENT.COM")
                    .is_ok()
            );
            for client_id in [
                "person@gmail.com",
                ".apps.googleusercontent.com",
                "123-example.apps.googleusercontent.com.example.org",
                "https://123-example.apps.googleusercontent.com",
                "123 example.apps.googleusercontent.com",
                "123\nexample.apps.googleusercontent.com",
                "-123-example.apps.googleusercontent.com",
                "_123-example.apps.googleusercontent.com",
            ] {
                assert!(validate_client_id(provider, client_id).is_err());
            }
        }
    }

    #[test]
    fn preserves_other_provider_ids_with_basic_input_checks() {
        assert!(validate_client_id("dropbox", "example-app-key").is_ok());
        assert!(validate_client_id("zoho", "1000.example-client").is_ok());
        for provider in ["dropbox", "zoho"] {
            for client_id in [
                "",
                "person@example.com",
                "https://example.com/app",
                "key\nvalue",
            ] {
                assert!(validate_client_id(provider, client_id).is_err());
            }
            assert!(validate_client_id(provider, &"x".repeat(4097)).is_err());
        }
        assert!(validate_client_id("unknown", "example").is_err());
    }
}
