#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
[ "$(uname -s)" = Darwin ] || { echo "Run on macOS."; exit 1; }
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
cargo test --manifest-path src-tauri/Cargo.toml --locked
# This opt-in test creates and deletes a synthetic Keychain item. macOS may ask for access.
cargo test --manifest-path src-tauri/Cargo.toml --locked macos_keychain_round_trip -- --ignored --exact
if command -v node >/dev/null; then node --test tests/*.test.mjs; fi
