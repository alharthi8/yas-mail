use crate::{err, App, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tauri::{
    Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalRect, PhysicalSize, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};

struct InitialWindowFit {
    inner_size: PhysicalSize<u32>,
    outer_position: PhysicalPosition<i32>,
    minimum: Option<LogicalSize<f64>>,
}

fn initial_window_fit(
    work: &PhysicalRect<i32, u32>,
    scale: f64,
    inner: PhysicalSize<u32>,
    outer: PhysicalSize<u32>,
    position: PhysicalPosition<i32>,
) -> Option<InitialWindowFit> {
    if !scale.is_finite() || scale <= 0.0 || work.size.width == 0 || work.size.height == 0 {
        return None;
    }
    // Work-area coordinates and frame measurements are physical pixels. Keep a
    // 12-DIP gap from taskbars and screen edges, including on scaled monitors.
    let margin = (12.0 * scale).ceil() as u32;
    let margin_x = margin.min(work.size.width.saturating_sub(1) / 2);
    let margin_y = margin.min(work.size.height.saturating_sub(1) / 2);
    let frame_width = outer.width.saturating_sub(inner.width);
    let frame_height = outer.height.saturating_sub(inner.height);
    let max_width = work
        .size
        .width
        .saturating_sub(2 * margin_x)
        .saturating_sub(frame_width)
        .max(1);
    let max_height = work
        .size
        .height
        .saturating_sub(2 * margin_y)
        .saturating_sub(frame_height)
        .max(1);
    let inner_size = PhysicalSize::new(inner.width.min(max_width), inner.height.min(max_height));
    let min_width = (max_width as f64 / scale).min(1000.0);
    let min_height = (max_height as f64 / scale).min(650.0);
    // Preserve the normal minimum unless this display cannot accommodate it.
    let minimum =
        (min_width < 1000.0 || min_height < 650.0).then(|| LogicalSize::new(min_width, min_height));
    let clamp_axis = |position: i32, origin: i32, span: u32, gap: u32, window: u32| {
        let first = i64::from(origin) + i64::from(gap);
        let last =
            (i64::from(origin) + i64::from(span) - i64::from(gap) - i64::from(window)).max(first);
        i64::from(position)
            .clamp(first, last)
            .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
    };
    Some(InitialWindowFit {
        inner_size,
        outer_position: PhysicalPosition::new(
            clamp_axis(
                position.x,
                work.position.x,
                work.size.width,
                margin_x,
                inner_size.width.saturating_add(frame_width),
            ),
            clamp_axis(
                position.y,
                work.position.y,
                work.size.height,
                margin_y,
                inner_size.height.saturating_add(frame_height),
            ),
        ),
        minimum,
    })
}

/// Fit only newly created windows. Never constrain a user's later resize or a
/// receiving window when an account tab moves between existing windows.
pub fn fit_initial_window_to_work_area(window: &WebviewWindow) -> tauri::Result<()> {
    let monitor = match window.current_monitor()? {
        Some(monitor) => Some(monitor),
        None => window.primary_monitor()?,
    };
    let Some(monitor) = monitor else {
        return Ok(());
    };
    let inner = window.inner_size()?;
    let position = window.outer_position()?;
    let Some(fit) = initial_window_fit(
        monitor.work_area(),
        monitor.scale_factor(),
        inner,
        window.outer_size()?,
        position,
    ) else {
        return Ok(());
    };
    if let Some(minimum) = fit.minimum {
        window.set_min_size(Some(minimum))?;
    }
    if fit.inner_size != inner {
        window.set_size(fit.inner_size)?;
    }
    if fit.outer_position != position {
        window.set_position(fit.outer_position)?;
    }
    Ok(())
}

#[derive(Clone, Default)]
pub struct WindowSeed {
    pub initial_account: Option<String>,
    pub view: Value,
    pub handoff_id: Option<String>,
    pub drop_x: Option<f64>,
}
pub struct WindowRegistry {
    pub owners: HashMap<String, String>,
    pub seeds: HashMap<String, WindowSeed>,
    pub blocked: HashSet<String>,
    pub previews: HashMap<String, String>,
    pub preview_epochs: HashMap<String, u64>,
}
impl WindowRegistry {
    pub fn new(accounts: impl IntoIterator<Item = String>) -> Self {
        Self {
            owners: accounts.into_iter().map(|id| (id, "main".into())).collect(),
            seeds: HashMap::from([("main".into(), WindowSeed::default())]),
            blocked: HashSet::new(),
            previews: HashMap::new(),
            preview_epochs: HashMap::new(),
        }
    }
    pub fn check(&self, label: &str, account: &str) -> Result<()> {
        if !self.seeds.contains_key(label) {
            return Err("This Yas Mail window is no longer registered".into());
        }
        match self.owners.get(account) {
            Some(owner) if owner == label => Ok(()),
            Some(_) => Err("This account is open in another Yas Mail window".into()),
            None => Err("Account not found".into()),
        }
    }
    pub fn transfer(&mut self, account: &str, from: &str, to: &str) -> Result<()> {
        self.check(from, account)?;
        if !self.seeds.contains_key(to) {
            return Err("Destination window not found".into());
        }
        self.owners.insert(account.into(), to.into());
        if from != to
            && self
                .seeds
                .get(from)
                .is_some_and(|seed| seed.initial_account.as_deref() == Some(account))
        {
            self.seeds.insert(from.into(), WindowSeed::default());
        }
        Ok(())
    }
    pub fn should_close_empty_child(&self, label: &str) -> bool {
        label != "main"
            && self.seeds.contains_key(label)
            && !self.owners.values().any(|owner| owner == label)
    }
    pub fn abort_creation(&mut self, account: &str, created: &str, preferred: &str) {
        self.seeds.remove(created);
        if self
            .owners
            .get(account)
            .is_some_and(|owner| owner == created)
        {
            let target = if self.seeds.contains_key(preferred) {
                preferred.to_string()
            } else if self.seeds.contains_key("main") {
                String::from("main")
            } else {
                let mut labels = self.seeds.keys().cloned().collect::<Vec<_>>();
                labels.sort();
                labels.into_iter().next().unwrap_or_else(|| "main".into())
            };
            self.owners.insert(account.into(), target);
        }
    }
    pub fn closed(&mut self, label: &str, remaining: &[String]) {
        self.seeds.remove(label);
        self.blocked.remove(label);
        self.preview_epochs.remove(label);
        self.previews
            .retain(|source, target| source != label && target != label);
        let target = if remaining.iter().any(|label| label == "main") {
            "main".into()
        } else {
            remaining.first().cloned().unwrap_or_else(|| "main".into())
        };
        for owner in self.owners.values_mut() {
            if owner == label {
                *owner = target.clone();
            }
        }
    }
}
pub fn check_owner(state: &App, label: &str, account: &str) -> Result<()> {
    state.windows.lock().map_err(err)?.check(label, account)
}
pub fn changed(app: &tauri::AppHandle) {
    let _ = app.emit("yasmail:accounts-changed", json!({}));
}

#[derive(Deserialize)]
pub struct DropPosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleasePoint {
    pub client_x: f64,
    pub client_y: f64,
}

struct DropTarget {
    window: WebviewWindow,
    drop_x: f64,
}

fn tab_strip_contains(x: f64, y: f64, left: f64, top: f64, width: f64, scale: f64) -> bool {
    if ![x, y, left, top, width, scale]
        .iter()
        .all(|value| value.is_finite())
        || scale <= 0.0
    {
        return false;
    }
    x >= left && x < left + (width - 132.0 * scale).max(0.0) && y >= top && y < top + 46.0 * scale
}

fn physical_release(
    origin_x: f64,
    origin_y: f64,
    scale: f64,
    release: &ReleasePoint,
) -> Result<(f64, f64)> {
    if ![
        origin_x,
        origin_y,
        scale,
        release.client_x,
        release.client_y,
    ]
    .iter()
    .all(|v| v.is_finite())
        || scale <= 0.0
        || release.client_x.abs() > 1_000_000.0
        || release.client_y.abs() > 1_000_000.0
    {
        return Err("Invalid tab release position".into());
    }
    Ok((
        origin_x + release.client_x * scale,
        origin_y + release.client_y * scale,
    ))
}

#[cfg(windows)]
fn top_window_at_physical_point(x: f64, y: f64) -> Result<windows_sys::Win32::Foundation::HWND> {
    use windows_sys::Win32::UI::{
        HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2},
        WindowsAndMessaging::{GetAncestor, WindowFromPoint, GA_ROOT},
    };
    // Async commands may run on worker threads whose DPI context differs from
    // the UI thread. Use physical coordinates for this native hit test, then
    // restore the same thread immediately; no await or early return intervenes.
    let previous =
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    if previous.is_null() {
        return Err(format!(
            "Could not use physical screen coordinates for tab transfer: {}",
            std::io::Error::last_os_error()
        ));
    }
    let under_cursor = unsafe {
        GetAncestor(
            WindowFromPoint(windows_sys::Win32::Foundation::POINT {
                x: x.round() as i32,
                y: y.round() as i32,
            }),
            GA_ROOT,
        )
    };
    let restored = unsafe { SetThreadDpiAwarenessContext(previous) };
    if restored.is_null() {
        return Err(format!(
            "Could not restore the display context after tab transfer hit testing: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(under_cursor)
}

fn find_drop_target(
    source: &WebviewWindow,
    state: &App,
    release: &ReleasePoint,
) -> Result<Option<DropTarget>> {
    if !release.client_x.is_finite()
        || !release.client_y.is_finite()
        || release.client_x.abs() > 1_000_000.0
        || release.client_y.abs() > 1_000_000.0
    {
        return Err("Invalid tab release position".into());
    }
    let (labels, blocked) = {
        let registry = state.windows.lock().map_err(err)?;
        if !registry.seeds.contains_key(source.label()) {
            return Err("Source window closed".into());
        }
        (
            registry.seeds.keys().cloned().collect::<HashSet<_>>(),
            registry.blocked.clone(),
        )
    };
    let origin = source.inner_position().map_err(err)?;
    let source_scale = source.scale_factor().map_err(err)?;
    let (x, y) = physical_release(origin.x as f64, origin.y as f64, source_scale, release)?;
    #[cfg(windows)]
    let under_cursor = top_window_at_physical_point(x, y)?;
    for (label, target) in source.app_handle().webview_windows() {
        if label == source.label()
            || !labels.contains(&label)
            || blocked.contains(&label)
            || !target.is_visible().unwrap_or(false)
            || target.is_minimized().unwrap_or(true)
        {
            continue;
        }
        #[cfg(windows)]
        if target.hwnd().map_err(err)?.0 != under_cursor {
            continue;
        }
        let position = target.inner_position().map_err(err)?;
        let size = target.inner_size().map_err(err)?;
        let scale = target.scale_factor().map_err(err)?;
        if tab_strip_contains(
            x,
            y,
            position.x as f64,
            position.y as f64,
            size.width as f64,
            scale,
        ) {
            return Ok(Some(DropTarget {
                window: target,
                drop_x: (x - position.x as f64) / scale,
            }));
        }
    }
    Ok(None)
}

fn clear_preview(app: &tauri::AppHandle, state: &App, source: &str) -> Result<()> {
    let target = {
        let mut registry = state.windows.lock().map_err(err)?;
        *registry.preview_epochs.entry(source.into()).or_default() += 1;
        registry.previews.remove(source)
    };
    if let Some(target) = target {
        let _ = app.emit_to(
            target,
            "yasmail:tab-drop-preview",
            json!({"active":false,"source":source}),
        );
    }
    Ok(())
}

#[tauri::command]
pub async fn set_tab_drop_blocked(
    window: WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    blocked: bool,
) -> Result<()> {
    {
        let mut registry = state.windows.lock().map_err(err)?;
        if !registry.seeds.contains_key(window.label()) {
            return Err("Window closed".into());
        }
        if blocked {
            registry.blocked.insert(window.label().into());
        } else {
            registry.blocked.remove(window.label());
        }
    }
    if !blocked {
        clear_preview(window.app_handle(), &state, window.label())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn probe_tab_drop(
    window: WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    release: ReleasePoint,
) -> Result<Value> {
    let epoch = {
        let registry = state.windows.lock().map_err(err)?;
        if !registry.blocked.contains(window.label()) {
            return Ok(Value::Null);
        }
        *registry.preview_epochs.get(window.label()).unwrap_or(&0)
    };
    let target = find_drop_target(&window, &state, &release)?;
    let next = target
        .as_ref()
        .map(|target| target.window.label().to_string());
    let previous = {
        let mut registry = state.windows.lock().map_err(err)?;
        if !registry.blocked.contains(window.label())
            || *registry.preview_epochs.get(window.label()).unwrap_or(&0) != epoch
        {
            return Ok(Value::Null);
        }
        let previous = registry.previews.remove(window.label());
        if let Some(label) = &next {
            registry
                .previews
                .insert(window.label().into(), label.clone());
        }
        previous
    };
    if let Some(previous) = previous {
        if Some(&previous) != next.as_ref() {
            let _ = window.app_handle().emit_to(
                previous,
                "yasmail:tab-drop-preview",
                json!({"active":false,"source":window.label()}),
            );
        }
    }
    if let Some(target) = target {
        let _ = window.app_handle().emit_to(
            target.window.label(),
            "yasmail:tab-drop-preview",
            json!({"active":true,"dropX":target.drop_x,"source":window.label()}),
        );
        // Cancellation may race emission after native geometry queries. Send a
        // final clear if the source ended its drag after the publish check.
        let still_current = {
            let registry = state.windows.lock().map_err(err)?;
            registry.blocked.contains(window.label())
                && *registry.preview_epochs.get(window.label()).unwrap_or(&0) == epoch
        };
        if !still_current {
            let _ = window.app_handle().emit_to(
                target.window.label(),
                "yasmail:tab-drop-preview",
                json!({"active":false,"source":window.label()}),
            );
            return Ok(Value::Null);
        }
        Ok(
            json!({"label":target.window.label(),"title":target.window.title().unwrap_or_else(|_|"Yas Mail".into()),"dropX":target.drop_x}),
        )
    } else {
        Ok(Value::Null)
    }
}

#[tauri::command]
pub async fn clear_tab_drop(
    window: WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
) -> Result<()> {
    clear_preview(window.app_handle(), &state, window.label())
}

#[tauri::command]
pub async fn window_context(
    window: WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
) -> Result<Value> {
    let total = state.accounts.lock().await.len();
    let registry = state.windows.lock().map_err(err)?;
    let seed = registry
        .seeds
        .get(window.label())
        .ok_or("Window is no longer registered")?;
    Ok(
        json!({"label":window.label(),"isDetached":window.label()!="main","totalAccounts":total,"initialAccount":seed.initial_account,"view":seed.view,"handoffId":seed.handoff_id,"dropX":seed.drop_x}),
    )
}

#[tauri::command]
pub async fn detach_account(
    window: WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
    view: Value,
    position: Option<DropPosition>,
) -> Result<Value> {
    // WebView2 window creation must run in an async command on Windows.
    let _creation = state.window_creation.lock().await;
    detach_inner(&window, &state, account, view, position).await
}

async fn detach_inner(
    window: &WebviewWindow,
    state: &App,
    account: String,
    view: Value,
    position: Option<DropPosition>,
) -> Result<Value> {
    check_owner(&state, window.label(), &account)?;
    let saved = state.account(&account).await?;
    let app = window.app_handle().clone();
    let label = format!("account-{:016x}", rand::random::<u64>());
    let source = window.label().to_string();
    let handoff_id = format!("{:016x}", rand::random::<u64>());
    {
        let mut registry = state.windows.lock().map_err(err)?;
        registry.seeds.insert(
            label.clone(),
            WindowSeed {
                initial_account: Some(account.clone()),
                view,
                handoff_id: Some(handoff_id.clone()),
                drop_x: None,
            },
        );
        registry.transfer(&account, &source, &label)?;
    }
    let mut builder = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html".into()))
        .title(format!("{} — Yas Mail", saved.email))
        .inner_size(1180.0, 820.0)
        .min_inner_size(1000.0, 650.0)
        .decorations(false)
        .resizable(true)
        .shadow(true)
        .visible(false);
    if let Some(p) = position {
        if p.x.is_finite() && p.y.is_finite() && p.x.abs() < 100_000.0 && p.y.abs() < 100_000.0 {
            builder = builder.position(p.x, p.y);
        } else {
            builder = builder.center();
        }
    } else {
        builder = builder.center();
    }
    let child = match builder.build() {
        Ok(child) => child,
        Err(error) => {
            {
                let mut registry = state.windows.lock().map_err(err)?;
                registry.abort_creation(&account, &label, &source);
            }
            changed(&app);
            return Err(err(error));
        }
    };
    if let Err(error) = fit_initial_window_to_work_area(&child).and_then(|()| child.show()) {
        {
            let mut registry = state.windows.lock().map_err(err)?;
            registry.abort_creation(&account, &label, &source);
        }
        let _ = child.destroy();
        changed(&app);
        return Err(err(error));
    }
    let _ = child.set_focus();
    changed(&app);
    // Creation and showing the new window can dispatch native window events.
    // Read ownership again only after success, so a failed detachment keeps
    // its source window and an account reassigned in the meantime stays open.
    let close_source = state
        .windows
        .lock()
        .map_err(err)?
        .should_close_empty_child(&source);
    if close_source {
        let _ = window.close();
    }
    Ok(json!({"label":label,"account":account,"attached":false,"handoffId":handoff_id}))
}

#[tauri::command]
pub async fn move_account(
    window: WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
    view: Value,
    release: ReleasePoint,
    position: Option<DropPosition>,
) -> Result<Value> {
    let _creation = state.window_creation.lock().await;
    check_owner(&state, window.label(), &account)?;
    let target = find_drop_target(&window, &state, &release)?;
    clear_preview(window.app_handle(), &state, window.label())?;
    let Some(target) = target else {
        return detach_inner(&window, &state, account, view, position).await;
    };
    let destination = target.window.label().to_string();
    let handoff_id = format!("{:016x}", rand::random::<u64>());
    {
        let mut registry = state.windows.lock().map_err(err)?;
        // Recheck immediately before ownership changes; modal state may have
        // changed while native window geometry was being queried.
        if registry.blocked.contains(&destination) {
            return Err("Destination window is busy. Close its dialog before moving a tab.".into());
        }
        registry.transfer(&account, window.label(), &destination)?;
        registry.seeds.insert(
            destination.clone(),
            WindowSeed {
                initial_account: Some(account.clone()),
                view,
                handoff_id: Some(handoff_id.clone()),
                drop_x: Some(target.drop_x),
            },
        );
    }
    changed(window.app_handle());
    let _ = target.window.show();
    let _ = target.window.set_focus();
    let close_source = state
        .windows
        .lock()
        .map_err(err)?
        .should_close_empty_child(window.label());
    if close_source {
        let _ = window.close();
    }
    Ok(json!({"label":destination,"account":account,"attached":true,"handoffId":handoff_id}))
}

#[tauri::command]
pub async fn return_account(
    window: WebviewWindow,
    state: tauri::State<'_, Arc<App>>,
    account: String,
) -> Result<Value> {
    let _creation = state.window_creation.lock().await;
    check_owner(&state, window.label(), &account)?;
    let app = window.app_handle().clone();
    let target = if window.label() != "main" {
        if app.get_webview_window("main").is_none() {
            state
                .windows
                .lock()
                .map_err(err)?
                .seeds
                .insert("main".into(), WindowSeed::default());
            let created =
                WebviewWindowBuilder::new(&app, "main", WebviewUrl::App("index.html".into()))
                    .title("Yas Mail")
                    .inner_size(1380.0, 900.0)
                    .min_inner_size(1000.0, 650.0)
                    .decorations(false)
                    .resizable(true)
                    .shadow(true)
                    .visible(false)
                    .build();
            match created {
                Ok(destination) => {
                    if let Err(error) = fit_initial_window_to_work_area(&destination) {
                        state.windows.lock().map_err(err)?.seeds.remove("main");
                        let _ = destination.destroy();
                        return Err(err(error));
                    }
                }
                Err(error) => {
                    state.windows.lock().map_err(err)?.seeds.remove("main");
                    return Err(err(error));
                }
            }
        }
        "main".to_string()
    } else {
        return Err("This account is already in the main window".into());
    };
    {
        let mut registry = state.windows.lock().map_err(err)?;
        registry.transfer(&account, window.label(), &target)?;
        registry.seeds.insert(
            target.clone(),
            WindowSeed {
                initial_account: Some(account.clone()),
                view: Value::Null,
                handoff_id: Some(format!("{:016x}", rand::random::<u64>())),
                drop_x: None,
            },
        );
    }
    changed(&app);
    if let Some(destination) = app.get_webview_window(&target) {
        let _ = destination.show();
        let _ = destination.set_focus();
    }
    let empty = !state
        .windows
        .lock()
        .map_err(err)?
        .owners
        .values()
        .any(|owner| owner == window.label());
    if empty {
        let _ = window.close();
    }
    Ok(json!({"label":target,"account":account}))
}

pub fn handle_destroyed(app: &tauri::AppHandle, label: &str) {
    let mut remaining = app
        .webview_windows()
        .keys()
        .filter(|other| other.as_str() != label)
        .cloned()
        .collect::<Vec<_>>();
    remaining.sort();
    if let Some(state) = app.try_state::<Arc<App>>() {
        let _ = clear_preview(app, &state, label);
        if let Ok(mut registry) = state.windows.lock() {
            registry.closed(label, &remaining);
        }
        changed(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn initial_window_fits_225_percent_screen_above_taskbar_with_frame_and_margin() {
        let work = PhysicalRect {
            position: PhysicalPosition::new(0, 0),
            size: PhysicalSize::new(2880, 1692),
        };
        let fit = initial_window_fit(
            &work,
            2.25,
            PhysicalSize::new(3105, 2025),
            PhysicalSize::new(3123, 2043),
            PhysicalPosition::new(-150, -200),
        )
        .unwrap();
        assert_eq!(fit.inner_size, PhysicalSize::new(2808, 1620));
        assert_eq!(fit.outer_position, PhysicalPosition::new(27, 27));
        assert!(fit.minimum.is_none());
        assert_eq!(
            fit.outer_position.y + fit.inner_size.height as i32 + 18,
            1665
        );
        assert!(1665 < work.size.height as i32);
    }
    #[test]
    fn initial_window_supports_negative_monitor_origin_and_reduces_unfit_minimum() {
        let work = PhysicalRect {
            position: PhysicalPosition::new(-1920, -120),
            size: PhysicalSize::new(1280, 800),
        };
        let fit = initial_window_fit(
            &work,
            1.5,
            PhysicalSize::new(1770, 1230),
            PhysicalSize::new(1786, 1270),
            PhysicalPosition::new(0, 100),
        )
        .unwrap();
        assert_eq!(fit.inner_size, PhysicalSize::new(1228, 724));
        assert_eq!(fit.outer_position, PhysicalPosition::new(-1902, -102));
        let minimum = fit.minimum.unwrap();
        assert_eq!(minimum.width, 1228.0 / 1.5);
        assert_eq!(minimum.height, 724.0 / 1.5);
        assert_eq!(
            fit.outer_position.x + fit.inner_size.width as i32 + 16,
            -658
        );
        assert_eq!(
            fit.outer_position.y + fit.inner_size.height as i32 + 40,
            662
        );
    }
    #[test]
    fn initial_window_preserves_size_position_and_minimum_when_already_fitting() {
        let inner = PhysicalSize::new(1180, 820);
        let position = PhysicalPosition::new(300, 100);
        let fit = initial_window_fit(
            &PhysicalRect {
                position: PhysicalPosition::new(0, 0),
                size: PhysicalSize::new(1920, 1040),
            },
            1.0,
            inner,
            PhysicalSize::new(1196, 858),
            position,
        )
        .unwrap();
        assert_eq!(fit.inner_size, inner);
        assert_eq!(fit.outer_position, position);
        assert!(fit.minimum.is_none());
    }
    #[test]
    fn tab_target_respects_scale_and_native_button_area() {
        assert!(tab_strip_contains(450.0, 220.0, 300.0, 200.0, 1500.0, 1.5));
        assert!(!tab_strip_contains(
            1700.0, 220.0, 300.0, 200.0, 1500.0, 1.5
        ));
        assert!(!tab_strip_contains(450.0, 270.0, 300.0, 200.0, 1500.0, 1.5));
        assert!(!tab_strip_contains(250.0, 220.0, 300.0, 200.0, 1500.0, 1.5));
        assert!(tab_strip_contains(
            -1800.0, -10.0, -1920.0, -40.0, 1200.0, 1.0
        ));
    }
    #[test]
    fn release_uses_native_source_origin_and_scale() {
        assert_eq!(
            physical_release(
                -1920.0,
                100.0,
                1.5,
                &ReleasePoint {
                    client_x: 1200.0,
                    client_y: -40.0
                }
            )
            .unwrap(),
            (-120.0, 40.0)
        );
        assert!(physical_release(
            0.0,
            0.0,
            0.0,
            &ReleasePoint {
                client_x: 1.0,
                client_y: 2.0
            }
        )
        .is_err());
        assert!(physical_release(
            0.0,
            0.0,
            1.0,
            &ReleasePoint {
                client_x: f64::NAN,
                client_y: 2.0
            }
        )
        .is_err());
    }
    #[test]
    fn transfer_clears_stale_source_view_seed() {
        let mut r = WindowRegistry::new(["a".into()]);
        r.seeds.insert(
            "main".into(),
            WindowSeed {
                initial_account: Some("a".into()),
                view: json!({"folder":"sent"}),
                handoff_id: Some("old".into()),
                drop_x: Some(50.0),
            },
        );
        r.seeds.insert("account-new".into(), WindowSeed::default());
        r.transfer("a", "main", "account-new").unwrap();
        assert!(r.seeds["main"].initial_account.is_none());
        assert!(r.seeds["main"].view.is_null());
        assert!(r.check("account-new", "a").is_ok());
    }
    #[test]
    fn closed_window_clears_drop_block_and_preview_routes() {
        let mut r = WindowRegistry::new(["a".into()]);
        r.blocked.insert("account-old".into());
        r.previews.insert("main".into(), "account-old".into());
        r.previews.insert("account-old".into(), "main".into());
        r.closed("account-old", &["main".into()]);
        assert!(!r.blocked.contains("account-old"));
        assert!(r.previews.is_empty());
    }
    #[test]
    fn owner_scope_is_authoritative() {
        let registry = WindowRegistry::new(["a".into()]);
        assert!(registry.check("main", "a").is_ok());
        assert!(registry.check("account-spoof", "a").is_err());
        assert!(registry.check("main", "missing").is_err());
    }
    #[test]
    fn transfer_preserves_single_ownership() {
        let mut r = WindowRegistry::new(["a".into(), "b".into()]);
        r.seeds.insert("account-one".into(), WindowSeed::default());
        r.transfer("a", "main", "account-one").unwrap();
        assert!(r.check("main", "a").is_err());
        assert!(r.check("account-one", "a").is_ok());
        assert!(r.check("main", "b").is_ok());
        assert!(r.transfer("a", "main", "account-one").is_err());
    }
    #[test]
    fn empty_child_closes_only_after_its_last_account_moves() {
        let mut r = WindowRegistry::new(["a".into(), "b".into()]);
        r.seeds
            .insert("account-source".into(), WindowSeed::default());
        r.seeds.insert("account-new".into(), WindowSeed::default());
        r.transfer("a", "main", "account-source").unwrap();
        r.transfer("b", "main", "account-source").unwrap();
        assert!(!r.should_close_empty_child("main"));
        r.transfer("a", "account-source", "account-new").unwrap();
        assert!(!r.should_close_empty_child("account-source"));
        r.transfer("b", "account-source", "account-new").unwrap();
        assert!(r.should_close_empty_child("account-source"));
        // If native events have returned an account while a new window was
        // being shown, the final ownership check keeps its source alive.
        r.transfer("b", "account-new", "account-source").unwrap();
        assert!(!r.should_close_empty_child("account-source"));
        r.seeds.remove("account-source");
        assert!(!r.should_close_empty_child("account-source"));
    }
    #[test]
    fn close_returns_accounts_to_main() {
        let mut r = WindowRegistry::new(["a".into()]);
        r.seeds.insert("account-one".into(), WindowSeed::default());
        r.transfer("a", "main", "account-one").unwrap();
        r.closed("account-one", &["main".into()]);
        assert_eq!(r.owners["a"], "main");
        assert!(!r.seeds.contains_key("account-one"));
    }
    #[test]
    fn closing_main_transfers_to_remaining_window() {
        let mut r = WindowRegistry::new(["a".into(), "b".into()]);
        r.seeds.insert("account-one".into(), WindowSeed::default());
        r.transfer("b", "main", "account-one").unwrap();
        r.closed("main", &["account-one".into()]);
        assert_eq!(r.owners["a"], "account-one");
        assert_eq!(r.owners["b"], "account-one");
    }
    #[test]
    fn failed_creation_rolls_back_account_and_seed() {
        let mut r = WindowRegistry::new(["a".into()]);
        r.seeds.insert("account-one".into(), WindowSeed::default());
        r.transfer("a", "main", "account-one").unwrap();
        r.abort_creation("a", "account-one", "main");
        assert!(r.check("main", "a").is_ok());
        assert_eq!(r.owners.len(), 1);
    }
    #[test]
    fn rollback_handles_a_source_closed_during_creation() {
        let mut r = WindowRegistry::new(["a".into()]);
        r.seeds.insert("account-one".into(), WindowSeed::default());
        r.seeds
            .insert("account-other".into(), WindowSeed::default());
        r.transfer("a", "main", "account-one").unwrap();
        r.seeds.remove("main");
        r.abort_creation("a", "account-one", "main");
        assert!(r.check("account-other", "a").is_ok());
    }
    #[test]
    fn rollback_does_not_override_destroyed_event_reassignment() {
        let mut r = WindowRegistry::new(["a".into()]);
        r.seeds.insert("account-one".into(), WindowSeed::default());
        r.transfer("a", "main", "account-one").unwrap();
        r.closed("account-one", &["main".into()]);
        r.abort_creation("a", "account-one", "already-closed-source");
        assert!(r.check("main", "a").is_ok());
    }
}
