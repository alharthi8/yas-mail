#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [ "$(uname -s)" != Darwin ]; then echo "Run this script on your MacBook."; exit 1; fi
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
if ! xcode-select -p >/dev/null 2>&1; then echo "Install Apple Command Line Tools: xcode-select --install; then rerun."; exit 1; fi
if ! command -v cargo >/dev/null || ! command -v rustup >/dev/null; then echo "Install Rust from https://rustup.rs, reopen Terminal, then rerun."; exit 1; fi
if ! cargo tauri --version >/dev/null 2>&1; then echo "Install the packaging tool: cargo install tauri-cli --version '^2' --locked; then rerun."; exit 1; fi
case "$(uname -m)" in
 arm64) target=aarch64-apple-darwin ;;
 x86_64) target=x86_64-apple-darwin ;;
 *) echo "Unsupported architecture"; exit 1 ;;
esac
if [ "${1:-}" = "--universal" ]; then
 rustup target add aarch64-apple-darwin x86_64-apple-darwin
 target=universal-apple-darwin
else
 rustup target add "$target"
fi
export CARGO_TARGET_DIR="$PWD/src-tauri/target"
export MACOSX_DEPLOYMENT_TARGET=13.0
mkdir -p build-logs
log="build-logs/macos-$(date +%Y%m%d-%H%M%S).log"
# Build only an app for local testing; DMG creation can require an interactive desktop.
# '-' is an ad-hoc development signature, not an Apple Developer distribution identity.
{
 cargo test --manifest-path src-tauri/Cargo.toml --locked
 cargo tauri build --target "$target" --bundles app --ci -- --locked
} 2>&1 | tee "$log"
echo "Built: src-tauri/target/$target/release/bundle/macos/Yas Mail.app"
echo "Log: $log"
echo "To launch: open \"src-tauri/target/$target/release/bundle/macos/Yas Mail.app\""
