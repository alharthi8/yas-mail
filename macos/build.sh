#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
export PATH="$HOME/.cargo/bin:$PATH"
export MACOSX_DEPLOYMENT_TARGET=13.0
export CARGO_BUILD_TARGET=aarch64-apple-darwin
if [ "$(uname -m)" != arm64 ]; then echo 'This preview currently targets Apple Silicon only.' >&2; exit 1; fi
cargo test --release --locked --manifest-path src-tauri/Cargo.toml
node --test tests/*.test.mjs
node node_modules/@tauri-apps/cli/tauri.js build --target aarch64-apple-darwin --bundles app --ci -- --locked
target="${CARGO_TARGET_DIR:-$PWD/src-tauri/target}"
app="$target/aarch64-apple-darwin/release/bundle/macos/Yas Mail.app"
codesign --verify --deep --strict --verbose=2 "$app"
file "$app/Contents/MacOS/yas-mail"
version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist")"
mkdir -p dist
ditto -c -k --sequesterRsrc --keepParent "$app" "dist/Yas-Mail-$version-macOS-Apple-Silicon-preview.zip"
shasum -a 256 "dist/Yas-Mail-$version-macOS-Apple-Silicon-preview.zip"
