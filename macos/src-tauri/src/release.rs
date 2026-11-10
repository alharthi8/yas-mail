//! One signed macOS update channel; never derive release settings from email content.
use crate::{err, App, Result};
use serde_json::{json, Value};
use std::sync::Arc;

#[tauri::command]
pub fn release_status() -> Value {
    json!({"version":env!("CARGO_PKG_VERSION"),"preview":true,"debugBuild":cfg!(debug_assertions),"updateConfigured":true,"publisherSignIn":crate::publisher_oauth::oauth_providers(),"updateSignatureRequired":true,"message":"Signed Apple Silicon previews from GitHub. Approve an update in the native dialog, then fully quit and reopen. This build is not Apple notarized."})
}

#[tauri::command]
pub fn check_for_updates(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<App>>)->Result<Value>{
    if !state.windows.lock().map_err(err)?.seeds.contains_key(window.label()) {
        return Err("Window is no longer registered.".into());
    }
    crate::ota::check(true);
    Ok(json!({"message":"Checking the signed macOS channel. Use the native update dialog when a release is available."}))
}
