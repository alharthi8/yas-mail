# Build Yas Mail for macOS

This folder contains the Apple Silicon preview source. The repository root contains the separate Windows version. Read RELEASE-NOTES.md for release limitations and install/update instructions.

On an Apple Silicon Mac, install Rust, Node.js 24, and Xcode Command Line Tools. Then run from this folder:

```sh
npm ci
bash build.sh
```

The script runs the Rust and JavaScript tests, uses the pinned Tauri CLI, and packages a locally signed app ZIP in dist/. No mail accounts or credentials are included. The bundle has an ad-hoc development signature and is not Apple notarized. The macOS 13 deployment target is not a claim that every older OS version has been tested.

The GitHub macos-release workflow additionally requires a native frontend launch handshake, synthetic Keychain round-trip, and signed bundle install/tamper-rejection test. A tag matching macos-vVERSION publishes a prerelease after those checks pass. macOS releases never become the repository's Windows latest release.

Real account send/receive, detached-window behavior, notifications, file dialogs, and optional local AI require further interactive testing. Google/Microsoft publisher OAuth setup remains separate. Do not put passwords, tokens, private mail, signing keys, or models in source archives or bug reports.
