# Yas Mail macOS preview

An isolated macOS port of Yas Mail with Rust/Tauri, Apple Keychain support, native file dialogs, account tabs, platform-aware local AI discovery, and a signed whole-app-bundle update channel. Its source baseline comes from the project's existing macOS conversation. The Windows application and latest-release update channel remain in the repository root and are not changed by this folder.

Build on Apple Silicon using the Mac build tools described in MACOS-START-HERE.md. Run `cargo test --manifest-path src-tauri/Cargo.toml --locked` and the JavaScript tests before packaging. Bundle version: 0.3.2. A Mac with a signed-in desktop and unlocked login Keychain is required to verify launch, native controls, credentials, and real account behavior.

In this port, Settings → Check for app updates and the startup check use the pinned repository public key. The updater selects only macos-vVERSION releases and an exact Apple Silicon asset name. It checks the signed ZIP and extracted app-bundle signature before replacing the installed .app. The active session stays open; fully quit and reopen after applying an update. Windows releases are not selected by this updater.

Apple Developer ID signing/notarization is not currently configured. A local ad-hoc signature is for development and is not a notarization ticket. Experimental builds must carry this limitation. Intel has not been tested. Public provider OAuth activation remains separate from the macOS port.
