#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/../.."
umask 077
key="$(mktemp)"
trap 'rm -f "$key"' EXIT
python3 - "$key" <<'PY'
import os,base64,sys
key=base64.b64decode(os.environ['YAS_UPDATE_SIGNING_KEY'],validate=True)
assert len(key)==64
open(sys.argv[1],'wb').write(key)
PY
version="$(python3 -c 'import tomllib;print(tomllib.load(open("macos/src-tauri/Cargo.toml","rb"))["package"]["version"])')"
preview="macos/dist/Yas-Mail-$version-macOS-Apple-Silicon-preview.zip"
asset="macos/dist/yas-mail-ota-$version-aarch64-apple-darwin.zip"
cp "$preview" "$asset"
zipsign sign zip "$asset" "$key"
zipsign verify zip "$asset" release.pub
"macos/src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Yas Mail.app/Contents/MacOS/yas-mail" --yas-verify-update "$asset"
cp "$asset" "$preview"
export YAS_SIGNED_UPDATE_FIXTURE="$PWD/$asset"
cargo test --release --locked --manifest-path "macos/src-tauri/Cargo.toml" signed_bundle_install_and_tamper_rejection -- --ignored --nocapture
(cd macos/dist && shasum -a 256 *.zip > SHA256SUMS.txt)
