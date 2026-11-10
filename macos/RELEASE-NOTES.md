# Yas Mail 0.3.2 — experimental macOS preview

Reuses the macOS port built and tested in the project's “Plan macOS Yas apps” chat, including native Keychain support and platform-specific local AI handling. Adds a separate signed GitHub macOS update channel and whole-app-bundle replacement, using the app's pinned public verification key. Windows source and its published update channel remain unchanged.

Apple Silicon (M1 or newer). Built with a macOS 13 deployment target; older macOS releases and Intel Macs have not been validated. Rust/Tauri with Apple's WKWebView and native Keychain credential storage. No comparative RAM/CPU percentage has been measured for this port.

This updater-enabled build passes 102 Rust and 52 JavaScript tests. Release automation additionally requires an actual Tauri/frontend launch handshake, a synthetic Keychain save/read/delete test, signature verification, and a signed full-bundle replacement/tamper-rejection test before publication. Real provider account sign-in, mail send/receive, attachment dialogs, detached windows, notifications, and optional local AI still need interactive testing. Provider OAuth publisher setup remains pending. Manual setup depends on your provider's protocol and authentication requirements.

## Install and update

Extract the preview ZIP and move Yas Mail.app to a writable Applications folder, such as ~/Applications. Do not run from inside the ZIP or a disk image. The preview has an ad-hoc development signature. It is **not Apple Developer ID signed or notarized**. macOS Gatekeeper may block opening it. See Apple's official guidance at https://support.apple.com/en-us/102445; do not disable Gatekeeper.

Choose Settings → Check for app updates. Startup checks use the same channel. Updates require approval, verify the repository's pinned Ed25519 signature and the extracted bundle signature, and replace the complete .app bundle. Fully quit and reopen to use the update. Accounts and mail data remain in their separate user-data directory. This macOS preview channel does not install Windows assets or change the Windows latest-release channel.

The expanded macOS source includes work in progress on encrypted caching, HTML mail, groupware, and local AI. Do not treat automated unit tests as verification of all live-provider behavior. Report issues without passwords, tokens, or private mail.
