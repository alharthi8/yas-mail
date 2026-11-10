//! Update endpoints and signature keys are publisher configuration, never message content.
use crate::{err, App, Result};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tauri::{Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

fn settings() -> Result<(url::Url, String)> {
    let endpoint=option_env!("YAS_MAIL_UPDATE_MANIFEST_URL").unwrap_or("").trim();
    let key=option_env!("YAS_MAIL_UPDATE_PUBLIC_KEY").unwrap_or("").trim();
    let url=url::Url::parse(endpoint).map_err(|_|"Updates are not configured for this build.")?;
    if url.scheme()!="https"||url.host_str().is_none()||!url.username().is_empty()||url.password().is_some()||key.is_empty(){return Err("Updates are not configured for this build.".into());}
    Ok((url,key.into()))
}
#[tauri::command]
pub fn release_status()->Value{
    json!({"version":env!("CARGO_PKG_VERSION"),"preview":true,"debugBuild":cfg!(debug_assertions),"updateConfigured":settings().is_ok(),"publisherSignIn":crate::publisher_oauth::oauth_providers(),"updateSignatureRequired":true})
}
fn registered(window:&tauri::WebviewWindow,state:&App)->Result<()> {
    if !state.windows.lock().map_err(err)?.seeds.contains_key(window.label()){return Err("Window is no longer registered.".into());}Ok(())
}
#[tauri::command]
pub async fn check_for_updates(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<App>>)->Result<Value>{
    registered(&window,&state)?;let(endpoint,key)=settings()?;
    let updater=window.app_handle().updater_builder().endpoints(vec![endpoint]).map_err(err)?.pubkey(key).timeout(Duration::from_secs(30)).build().map_err(err)?;
    match updater.check().await.map_err(err)?{Some(update)=>Ok(json!({"available":true,"version":update.version,"notes":update.body,"currentVersion":env!("CARGO_PKG_VERSION")})),None=>Ok(json!({"available":false,"currentVersion":env!("CARGO_PKG_VERSION")}))}
}
#[tauri::command]
pub async fn install_update(window:tauri::WebviewWindow,state:tauri::State<'_,Arc<App>>,version:String)->Result<()> {
    registered(&window,&state)?;let(endpoint,key)=settings()?;
    let updater=window.app_handle().updater_builder().endpoints(vec![endpoint]).map_err(err)?.pubkey(key).timeout(Duration::from_secs(30)).build().map_err(err)?;
    let update=updater.check().await.map_err(err)?.ok_or("No update is available.")?;
    if update.version!=version{return Err("The available update changed. Check for updates again.".into());}
    // The official updater verifies the publisher signature before installation.
    let progress=window.clone();let finished=window.clone();
    update.download_and_install(move|bytes,total|{let _=progress.emit("yasmail:update-progress",json!({"bytes":bytes,"total":total}));},move||{let _=finished.emit("yasmail:update-downloaded",json!({}));}).await.map_err(err)
}
