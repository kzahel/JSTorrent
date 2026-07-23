use jstorrent_common::{get_config_dir, RpcInfo};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{ChildStdin, ChildStdout};
use std::sync::{Arc, Mutex};
#[cfg(target_os = "macos")]
use tauri::menu::MenuItemKind;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, SubmenuBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_window_state::WindowExt as WindowStateExt;
use tokio::sync::oneshot;

mod headless_updater;
mod native_host;

const TARGET_TRIPLE: &str = env!("TARGET_TRIPLE");

/// Strip the `\\?\` extended-length path prefix that Windows APIs like
/// `canonicalize()` and `current_exe()` produce. Chrome's native messaging
/// launcher doesn't understand this prefix, so we need plain paths.
#[cfg(windows)]
pub(crate) fn strip_win_prefix(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        p
    }
}

/// Show a fatal error to the user. On Windows (where the GUI subsystem hides
/// stderr), this displays a native message box so the error is actually visible.
fn fatal_error(message: &str) -> ! {
    eprintln!("{message}");
    // Write crash log next to the executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let _ = std::fs::write(dir.join("crash.log"), message);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        extern "system" {
            fn MessageBoxW(
                hwnd: *mut std::ffi::c_void,
                text: *const u16,
                caption: *const u16,
                utype: u32,
            ) -> i32;
        }
        let wide_msg: Vec<u16> = std::ffi::OsStr::new(message)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let wide_title: Vec<u16> = std::ffi::OsStr::new("JSTorrent")
            .encode_wide()
            .chain(Some(0))
            .collect();
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                wide_msg.as_ptr(),
                wide_title.as_ptr(),
                0x10, // MB_ICONERROR
            );
        }
    }
    std::process::exit(1);
}

/// CLI launch args parsed at startup for --force-desktop / --profile.
struct LaunchArgs {
    force_desktop: bool,
    profile_id: Option<String>,
}

struct HostBridge {
    stdin: Mutex<ChildStdin>,
    pending: Mutex<HashMap<String, oneshot::Sender<serde_json::Value>>>,
}

impl HostBridge {
    /// Write a length-prefixed JSON message to native host stdin.
    fn send(&self, message: &serde_json::Value) -> Result<(), String> {
        let json = serde_json::to_vec(message).map_err(|e| e.to_string())?;
        let len = (json.len() as u32).to_le_bytes();
        let mut stdin = self.stdin.lock().map_err(|e| e.to_string())?;
        stdin.write_all(&len).map_err(|e| e.to_string())?;
        stdin.write_all(&json).map_err(|e| e.to_string())?;
        stdin.flush().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Send a request and wait for the matching response.
    async fn request(&self, mut message: serde_json::Value) -> Result<serde_json::Value, String> {
        let id = uuid::Uuid::new_v4().to_string();
        message
            .as_object_mut()
            .ok_or("message must be a JSON object")?
            .insert("id".into(), serde_json::Value::String(id.clone()));

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().map_err(|e| e.to_string())?;
            pending.insert(id.clone(), tx);
        }

        if let Err(e) = self.send(&message) {
            let mut pending = self.pending.lock().map_err(|e| e.to_string())?;
            pending.remove(&id);
            return Err(e);
        }

        rx.await.map_err(|_| "Response channel closed".to_string())
    }
}

/// Pending deep link events that arrived before the frontend was ready.
struct DeepLinkState {
    pending: Mutex<Vec<serde_json::Value>>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct Settings {
    #[serde(default)]
    autostart: bool,
    #[serde(default = "default_true")]
    run_in_background: bool,
    /// Show tray icon in macOS menu bar. Ignored on other platforms.
    #[serde(default = "default_true")]
    show_in_menu_bar: bool,
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            autostart: false,
            run_in_background: true,
            show_in_menu_bar: true,
        }
    }
}

/// On macOS, check menu items appear in both the app menu and the tray menu.
/// This holds all instances so we can keep their checked state in sync.
#[cfg(target_os = "macos")]
struct CheckItemSync(HashMap<String, Vec<CheckMenuItem<tauri::Wry>>>);

#[cfg(target_os = "macos")]
fn sync_check_items(app: &tauri::AppHandle, id: &str, checked: bool) {
    let state = app.state::<CheckItemSync>();
    if let Some(items) = state.0.get(id) {
        for item in items {
            let _ = item.set_checked(checked);
        }
    }
}

fn load_settings(app: &tauri::AppHandle) -> Settings {
    let data_dir = app.path().app_data_dir().expect("no app data directory");
    let path = data_dir.join("settings.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_settings(app: &tauri::AppHandle, settings: &Settings) {
    let data_dir = app.path().app_data_dir().expect("no app data directory");
    std::fs::create_dir_all(&data_dir).ok();
    let path = data_dir.join("settings.json");
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        std::fs::write(&path, json).ok();
    }
}

/// Create a host-event JSON value from a deep link URL string or file path.
/// Returns None if the input isn't a recognized deep link type.
/// Extract a magnet URI from a `jstorrent://` deep link query string.
/// e.g. `jstorrent://launch?magnet=magnet%3A%3Fxt%3D...` -> `magnet:?xt=...`
fn extract_magnet_from_jstorrent_url(url_str: &str) -> Option<String> {
    let query = url_str.split('?').nth(1)?;
    for param in query.split('&') {
        if let Some(value) = param.strip_prefix("magnet=") {
            let decoded = urlencoding::decode(value).ok()?;
            if decoded.starts_with("magnet:") {
                return Some(decoded.into_owned());
            }
        }
    }
    None
}

fn deep_link_event(url_str: &str) -> Option<serde_json::Value> {
    if url_str.starts_with("magnet:") {
        Some(serde_json::json!({
            "event": "MagnetAdded",
            "payload": { "link": url_str }
        }))
    } else if url_str.to_lowercase().ends_with(".torrent") {
        // Accept both file:// URLs and raw file paths (Windows passes raw paths
        // via command-line args when opening associated .torrent files).
        torrent_file_event(url_str)
    } else {
        None
    }
}

/// Convert a `file://` URL or raw path to a filesystem path string.
/// Strips the `file://` prefix and URL-decodes percent-encoded characters
/// (e.g. `%20` → space). Raw paths (no `file://` prefix) are returned as-is.
fn file_url_to_path(url_or_path: &str) -> String {
    match url_or_path.strip_prefix("file://") {
        Some(encoded) => urlencoding::decode(encoded)
            .unwrap_or(std::borrow::Cow::Borrowed(encoded))
            .into_owned(),
        None => url_or_path.to_string(),
    }
}

/// Read a .torrent file from a file:// URL or raw path and create a `TorrentAdded` event.
fn torrent_file_event(file_url: &str) -> Option<serde_json::Value> {
    use base64::Engine;

    // Accept both file:// URLs and raw file paths (Windows file associations).
    let path_str = file_url_to_path(file_url);
    let path = std::path::Path::new(&path_str);

    let contents = std::fs::read(path).ok()?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&contents);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();

    Some(serde_json::json!({
        "event": "TorrentAdded",
        "payload": {
            "name": name,
            "contentsBase64": encoded
        }
    }))
}

/// Set the window icon to a high-resolution PNG on Windows.
/// Tauri v2 + tao's `set_icon()` only sends `WM_SETICON` with `ICON_SMALL`,
/// which updates the title bar but NOT the Windows taskbar. We additionally
/// set `ICON_BIG` via the Win32 API so the taskbar icon renders correctly.
/// See https://github.com/tauri-apps/tauri/issues/14596
#[cfg(windows)]
fn set_window_icon(window: &tauri::WebviewWindow) {
    let png_bytes = include_bytes!("../icons/icon.png");
    // ICON_SMALL (title bar) — via Tauri API
    if let Ok(icon) = tauri::image::Image::from_bytes(png_bytes) {
        let _ = window.set_icon(icon);
    }
    // ICON_BIG (taskbar) — via Win32 API directly
    if let Ok(image) = tauri::image::Image::from_bytes(png_bytes) {
        set_icon_big(window, &image);
    }
}

/// Send `WM_SETICON` with `ICON_BIG` to set the Windows taskbar icon.
/// Uses `CreateIconIndirect` with a 32-bit BGRA DIB section for proper
/// alpha transparency support.
#[cfg(windows)]
fn set_icon_big(window: &tauri::WebviewWindow, image: &tauri::image::Image<'_>) {
    extern "system" {
        fn CreateBitmap(
            width: i32,
            height: i32,
            planes: u32,
            bit_count: u32,
            bits: *const u8,
        ) -> isize;
        fn CreateDIBSection(
            hdc: isize,
            pbmi: *const BitmapInfoHeader,
            usage: u32,
            ppv_bits: *mut *mut u8,
            section: isize,
            offset: u32,
        ) -> isize;
        fn CreateIconIndirect(piconinfo: *mut IconInfo) -> isize;
        fn DeleteObject(obj: isize) -> i32;
        fn SendMessageW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize;
    }

    #[repr(C)]
    struct BitmapInfoHeader {
        size: u32,
        width: i32,
        height: i32,
        planes: u16,
        bit_count: u16,
        compression: u32,
        size_image: u32,
        x_pels_per_meter: i32,
        y_pels_per_meter: i32,
        clr_used: u32,
        clr_important: u32,
    }

    #[repr(C)]
    struct IconInfo {
        f_icon: i32,
        x_hotspot: u32,
        y_hotspot: u32,
        hbm_mask: isize,
        hbm_color: isize,
    }

    const WM_SETICON: u32 = 0x0080;
    const ICON_BIG: usize = 1;

    let Ok(hwnd) = window.hwnd() else { return };
    let rgba = image.rgba();
    let width = image.width() as i32;
    let height = image.height() as i32;

    unsafe {
        // Monochrome AND mask (all zeros = fully opaque; alpha channel handles transparency)
        let and_row_bytes = ((width + 31) / 32 * 4) as usize;
        let and_mask = vec![0u8; and_row_bytes * height as usize];
        let h_and = CreateBitmap(width, height, 1, 1, and_mask.as_ptr());
        if h_and == 0 {
            return;
        }

        // 32-bit top-down BGRA DIB section
        let bmi = BitmapInfoHeader {
            size: std::mem::size_of::<BitmapInfoHeader>() as u32,
            width,
            height: -height, // negative = top-down
            planes: 1,
            bit_count: 32,
            compression: 0,
            size_image: 0,
            x_pels_per_meter: 0,
            y_pels_per_meter: 0,
            clr_used: 0,
            clr_important: 0,
        };

        let mut bits_ptr: *mut u8 = std::ptr::null_mut();
        let h_color = CreateDIBSection(0, &bmi, 0, &mut bits_ptr, 0, 0);
        if h_color == 0 || bits_ptr.is_null() {
            DeleteObject(h_and);
            return;
        }

        // Copy RGBA → premultiplied BGRA (Windows expects premultiplied alpha for 32-bit icons)
        let pixel_count = (width * height) as usize;
        let dst = std::slice::from_raw_parts_mut(bits_ptr, pixel_count * 4);
        for i in 0..pixel_count {
            let a = rgba[i * 4 + 3] as u32;
            dst[i * 4] = ((rgba[i * 4 + 2] as u32 * a) / 255) as u8; // B
            dst[i * 4 + 1] = ((rgba[i * 4 + 1] as u32 * a) / 255) as u8; // G
            dst[i * 4 + 2] = ((rgba[i * 4] as u32 * a) / 255) as u8; // R
            dst[i * 4 + 3] = a as u8; // A
        }

        let mut info = IconInfo {
            f_icon: 1, // TRUE = icon (not cursor)
            x_hotspot: 0,
            y_hotspot: 0,
            hbm_mask: h_and,
            hbm_color: h_color,
        };
        let hicon = CreateIconIndirect(&mut info);

        // Clean up bitmaps — Windows copies them internally
        DeleteObject(h_and);
        DeleteObject(h_color);

        if hicon != 0 {
            // HWND is repr(transparent) around isize — safe to transmute_copy
            let hwnd_raw: isize = std::mem::transmute_copy(&hwnd);
            SendMessageW(hwnd_raw, WM_SETICON, ICON_BIG, hicon);
            // Intentionally leak the HICON — it must remain valid for the window's lifetime.
        }
    }
}

/// Set the window icon on Linux so the desktop environment uses it for
/// the taskbar instead of falling back to the generic gear/application icon.
/// macOS uses the .icns from the bundle, so this is a no-op there.
#[cfg(target_os = "linux")]
fn set_window_icon(window: &tauri::WebviewWindow) {
    let png_bytes = include_bytes!("../icons/icon.png");
    if let Ok(icon) = tauri::image::Image::from_bytes(png_bytes) {
        let _ = window.set_icon(icon);
    }
}

#[cfg(target_os = "macos")]
fn set_window_icon(_window: &tauri::WebviewWindow) {}

/// Show, unminimize, and focus the main window.
/// If the window was destroyed (`run_in_background=false`), recreate it.
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        set_window_icon(&window);
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    } else if let Ok(window) =
        tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("index.html".into()))
            .title("JSTorrent")
            .inner_size(1024.0, 700.0)
            .build()
    {
        set_window_icon(&window);
        let _ = window.restore_state(tauri_plugin_window_state::StateFlags::all());
    }
}

/// Read rpc-info.json from the standard config location.
fn read_rpc_info() -> RpcInfo {
    let Some(config_dir) = get_config_dir() else {
        return RpcInfo {
            version: 1,
            add_token: None,
            profiles: Vec::new(),
        };
    };
    let rpc_file = config_dir.join("jstorrent-native").join("rpc-info.json");
    std::fs::read_to_string(&rpc_file)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(RpcInfo {
            version: 1,
            add_token: None,
            profiles: Vec::new(),
        })
}

/// Get the launch URL, checking env file override first.
fn get_launch_url() -> String {
    jstorrent_common::read_env_value("LAUNCH_URL")
        .unwrap_or_else(|| "https://jstorrent.com/launch".to_string())
}

/// Determine whether a deep link should be routed to the browser extension.
/// Auto-routing: should we route to extension based on profile history?
fn should_route_to_extension(rpc_info: &RpcInfo) -> bool {
    auto_route_decision(rpc_info)
}

/// Auto-mode routing heuristic. Returns true = route to extension.
fn auto_route_decision(rpc_info: &RpcInfo) -> bool {
    let profiles = &rpc_info.profiles;

    if profiles.is_empty() {
        return false;
    }

    let any_desktop = profiles
        .iter()
        .any(|p| p.desktop_ever_used || p.client_types_used.iter().any(|ct| ct == "tauri"));
    let any_extension = profiles
        .iter()
        .any(|p| p.client_types_used.iter().any(|ct| ct == "extension"));

    if any_extension && !any_desktop {
        return true;
    }

    if any_desktop && !any_extension {
        return false;
    }

    // Both have evidence — most recently used profile wins
    if any_desktop && any_extension {
        let mut active: Vec<_> = profiles
            .iter()
            .filter(|p| !p.download_roots.is_empty())
            .collect();
        if active.is_empty() {
            active = profiles.iter().collect();
        }
        active.sort_by_key(|profile| std::cmp::Reverse(profile.last_used));
        if let Some(most_recent) = active.first() {
            if let Some(ct) = &most_recent.client_type {
                return ct == "extension";
            }
        }
    }

    false
}

/// What to do when the app starts up (after deep link processing).
#[derive(Debug, PartialEq)]
enum StartupAction {
    /// Show the desktop window.
    ShowDesktop,
    /// Open the extension via launch URL (don't show desktop window).
    OpenExtension,
    /// Deep links were already routed to extension; do nothing.
    AlreadyRouted,
}

/// Pure function: decide the startup action based on whether deep links were
/// already routed and the routing heuristic for bare launches.
fn determine_startup_action(
    startup_routed_to_extension: bool,
    rpc_info: &RpcInfo,
) -> StartupAction {
    if startup_routed_to_extension {
        StartupAction::AlreadyRouted
    } else if should_route_to_extension(rpc_info) {
        StartupAction::OpenExtension
    } else {
        StartupAction::ShowDesktop
    }
}

enum RouteResult {
    Desktop,
    Extension,
    NotRecognized,
}

/// Route a magnet link to the browser extension via launch URL.
fn route_magnet_to_extension(app: &tauri::AppHandle, magnet: &str, add_token: Option<&str>) {
    let base = get_launch_url();
    let encoded = urlencoding::encode(magnet);
    let url = match add_token {
        Some(token) => format!("{base}#magnet={encoded}&token={token}"),
        None => format!("{base}#magnet={encoded}"),
    };
    eprintln!("[deep-link] route_magnet_to_extension: opening URL: {url}");
    let _ = app.opener().open_url(&url, None::<&str>);
}

/// Route a .torrent file to the browser extension via launch URL.
fn route_torrent_to_extension(app: &tauri::AppHandle, path: &str, add_token: Option<&str>) {
    let base = get_launch_url();
    let encoded = urlencoding::encode(path);
    let url = match add_token {
        Some(token) => format!("{base}#torrent={encoded}&token={token}"),
        None => format!("{base}#torrent={encoded}"),
    };
    let _ = app.opener().open_url(&url, None::<&str>);
}

/// Handle a deep link URL with routing logic.
/// Decides whether to route to desktop or extension, then dispatches.
fn handle_deep_link_routed(app: &tauri::AppHandle, url_str: &str) -> RouteResult {
    let is_magnet = url_str.starts_with("magnet:");
    let is_torrent = url_str.to_lowercase().ends_with(".torrent");
    if !is_magnet && !is_torrent {
        return RouteResult::NotRecognized;
    }

    let window_visible = app
        .get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);

    let rpc_info = read_rpc_info();

    // If desktop window is visible, always handle locally
    let route_to_ext = if window_visible {
        false
    } else {
        should_route_to_extension(&rpc_info)
    };

    if route_to_ext {
        let add_token = rpc_info.add_token.as_deref();
        if is_magnet {
            route_magnet_to_extension(app, url_str, add_token);
        } else {
            let path = file_url_to_path(url_str);
            route_torrent_to_extension(app, &path, add_token);
        }
        RouteResult::Extension
    } else {
        if let Some(event) = deep_link_event(url_str) {
            let _ = app.emit("host-event", &event);
        }
        show_main_window(app);
        RouteResult::Desktop
    }
}

/// Resolve sidecar binary path following Tauri's naming convention.
/// Checks multiple locations to handle different installer layouts:
/// - With/without `binaries/` subdirectory
/// - With/without target triple suffix
/// - In both `resource_dir` and exe directory
fn resolve_sidecar(app: &tauri::AppHandle, name: &str) -> Result<PathBuf, String> {
    let ext = if cfg!(windows) { ".exe" } else { "" };
    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf));

    // Extract just the filename (e.g. "jstorrent-host" from "binaries/jstorrent-host")
    let basename = std::path::Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(name);

    let mut candidates = Vec::new();
    for dir in [Some(&resource_dir), exe_dir.as_ref()]
        .into_iter()
        .flatten()
    {
        // Standard Tauri: {dir}/{name}-{triple}{ext} (e.g. binaries/jstorrent-host-x86_64-...)
        candidates.push(dir.join(format!("{name}-{TARGET_TRIPLE}{ext}")));
        // Without triple: {dir}/{name}{ext}
        candidates.push(dir.join(format!("{name}{ext}")));
        // Flat with triple: {dir}/{basename}-{triple}{ext}
        candidates.push(dir.join(format!("{basename}-{TARGET_TRIPLE}{ext}")));
        // Flat without triple: {dir}/{basename}{ext}
        candidates.push(dir.join(format!("{basename}{ext}")));
    }

    for candidate in &candidates {
        if candidate.exists() {
            #[cfg(windows)]
            return Ok(strip_win_prefix(candidate.clone()));
            #[cfg(not(windows))]
            return Ok(candidate.clone());
        }
    }

    Err(format!(
        "Sidecar not found. Searched:\n{}",
        candidates
            .iter()
            .map(|c| format!("  {}", c.display()))
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

/// Read native messaging frames from native host stdout and dispatch them.
fn run_stdout_reader(stdout: &mut ChildStdout, bridge: &HostBridge, app_handle: &tauri::AppHandle) {
    let mut len_buf = [0u8; 4];

    loop {
        if stdout.read_exact(&mut len_buf).is_err() {
            eprintln!("native host: stdout closed");
            break;
        }
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut buf = vec![0u8; len];
        if stdout.read_exact(&mut buf).is_err() {
            eprintln!("native host: read error");
            break;
        }

        let msg: serde_json::Value = match serde_json::from_slice(&buf) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("native host: invalid JSON: {e}");
                continue;
            }
        };

        // Dispatch: response (has "id") vs event (has "event")
        if let Some(id) = msg.get("id").and_then(|v| v.as_str()) {
            if let Ok(mut pending) = bridge.pending.lock() {
                if let Some(tx) = pending.remove(id) {
                    let _ = tx.send(msg);
                }
            }
        } else if msg.get("event").is_some() {
            let _ = app_handle.emit("host-event", &msg);
        }
    }

    // Clean up pending requests on disconnect
    if let Ok(mut pending) = bridge.pending.lock() {
        pending.clear();
    }
}

#[tauri::command]
fn js_log(msg: &str) {
    eprintln!("[webview] {msg}");
}

#[tauri::command]
async fn host_handshake(
    state: tauri::State<'_, Arc<HostBridge>>,
    launch_args: tauri::State<'_, LaunchArgs>,
    profile_id: Option<String>,
) -> Result<serde_json::Value, String> {
    // CLI --profile overrides frontend-provided profileId
    let effective_profile = launch_args.profile_id.clone().or(profile_id);

    let mut msg = serde_json::json!({
        "op": "handshake",
        "extensionId": "tauri-desktop",
        "clientType": "tauri",
        "clientVersion": env!("CARGO_PKG_VERSION"),
    });
    if let Some(pid) = &effective_profile {
        msg["profileId"] = serde_json::Value::String(pid.clone());
    }
    let response = state.request(msg).await?;

    // If --force-desktop and profile is in use, auto-send takeOver
    if launch_args.force_desktop {
        let is_in_use = response
            .get("error")
            .and_then(|e| e.as_str())
            .is_some_and(|e| e == "profile_in_use");
        if is_in_use {
            eprintln!("--force-desktop: profile in use, sending takeOver");
            let mut takeover_msg = serde_json::json!({
                "op": "takeOver",
                "extensionId": "tauri-desktop",
                "clientType": "tauri",
                "clientVersion": env!("CARGO_PKG_VERSION"),
            });
            if let Some(pid) = &effective_profile {
                takeover_msg["profileId"] = serde_json::Value::String(pid.clone());
            }
            return state.request(takeover_msg).await;
        }
    }

    Ok(response)
}

#[tauri::command]
async fn host_message(
    state: tauri::State<'_, Arc<HostBridge>>,
    message: serde_json::Value,
) -> Result<serde_json::Value, String> {
    state.request(message).await
}

#[tauri::command]
async fn pick_download_folder(
    app: tauri::AppHandle,
    window: tauri::Window,
    bridge: tauri::State<'_, Arc<HostBridge>>,
    start_dir: Option<String>,
) -> Result<serde_json::Value, String> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = oneshot::channel();

    let mut builder = app
        .dialog()
        .file()
        .set_parent(&window)
        .set_title("Select Download Directory");

    if let Some(ref dir) = start_dir {
        builder = builder.set_directory(dir);
    }

    builder.pick_folder(move |path| {
        let _ = tx.send(path);
    });

    let path = rx.await.map_err(|_| "Dialog channel closed".to_string())?;
    let Some(path) = path else {
        return Err("User cancelled".to_string());
    };

    let path_str = path
        .into_path()
        .map_err(|e| format!("Invalid path: {e}"))?
        .to_string_lossy()
        .to_string();

    bridge
        .request(serde_json::json!({
            "op": "registerDownloadRoot",
            "path": path_str,
        }))
        .await
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn restart_app(app: tauri::AppHandle) {
    app.restart();
}

/// Mark the current desktop profile as having been used for torrents.
/// Called by the frontend when the user adds their first torrent via the desktop UI.
#[tauri::command]
fn mark_desktop_activated() -> Result<(), String> {
    let config_dir = get_config_dir().ok_or("No config directory")?;
    let app_dir = config_dir.join("jstorrent-native");
    let rpc_file = app_dir.join("rpc-info.json");

    let mut rpc_info: RpcInfo = std::fs::read_to_string(&rpc_file)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .ok_or("Could not read rpc-info.json")?;

    let mut changed = false;
    for profile in &mut rpc_info.profiles {
        if profile.client_type.as_deref() == Some("tauri") && !profile.desktop_ever_used {
            profile.desktop_ever_used = true;
            changed = true;
        }
    }

    if !changed {
        return Ok(());
    }

    let temp = tempfile::NamedTempFile::new_in(&app_dir).map_err(|e| e.to_string())?;
    serde_json::to_writer(&temp, &rpc_info).map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(&rpc_file)
        .map_err(|e| format!("Failed to persist: {}", e.error))?;

    Ok(())
}

/// Return and clear any deep link events that arrived before the frontend was ready.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn get_pending_deep_links(state: tauri::State<'_, DeepLinkState>) -> Vec<serde_json::Value> {
    let mut pending = state
        .pending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    pending.drain(..).collect()
}

fn format_bytes(bytes: f64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    if bytes >= GB {
        format!("{:.1} GB", bytes / GB)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes / MB)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes / KB)
    } else {
        format!("{bytes:.0} B")
    }
}

fn handle_menu_event(app: &tauri::AppHandle, event_id: &str) {
    match event_id {
        "show" => {
            show_main_window(app);
        }
        "open-extension" => {
            let _ = app
                .opener()
                .open_url("https://jstorrent.com/launch", None::<&str>);
        }
        "check-updates" => {
            show_main_window(app);
            let _ = app.emit("check-for-updates", ());
        }
        "autostart" => {
            let state = app.state::<Mutex<Settings>>();
            let mut s = state.lock().unwrap();
            s.autostart = !s.autostart;
            let checked = s.autostart;
            if checked {
                let _ = app.autolaunch().enable();
            } else {
                let _ = app.autolaunch().disable();
            }
            save_settings(app, &s);
            drop(s);
            #[cfg(target_os = "macos")]
            sync_check_items(app, "autostart", checked);
        }
        "run-in-background" => {
            let state = app.state::<Mutex<Settings>>();
            let mut s = state.lock().unwrap();
            s.run_in_background = !s.run_in_background;
            #[cfg(target_os = "macos")]
            let checked = s.run_in_background;
            save_settings(app, &s);
            drop(s);
            #[cfg(target_os = "macos")]
            sync_check_items(app, "run-in-background", checked);
        }
        "show-in-menu-bar" => {
            let state = app.state::<Mutex<Settings>>();
            let mut s = state.lock().unwrap();
            s.show_in_menu_bar = !s.show_in_menu_bar;
            let visible = s.show_in_menu_bar;
            save_settings(app, &s);
            drop(s);
            if let Some(tray) = app.tray_by_id("tray") {
                let _ = tray.set_visible(visible);
            }
            #[cfg(target_os = "macos")]
            sync_check_items(app, "show-in-menu-bar", visible);
        }
        "quit" => {
            app.exit(0);
        }
        _ => {
            eprintln!("handle_menu_event: unhandled event: {event_id}");
        }
    }
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn update_tray_stats(app: tauri::AppHandle, stats: serde_json::Value) {
    let Some(tray) = app.tray_by_id("tray") else {
        return;
    };

    let download_speed = stats
        .get("downloadSpeed")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let upload_speed = stats
        .get("uploadSpeed")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let active_count = stats
        .get("activeCount")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let error_count = stats
        .get("errorCount")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);

    let active = if download_speed > 0.0 || upload_speed > 0.0 || active_count > 0 {
        let mut lines = vec![format!(
            "\u{2193} {}/s  \u{2191} {}/s",
            format_bytes(download_speed),
            format_bytes(upload_speed)
        )];
        if active_count > 0 {
            lines.push(format!("{active_count} active"));
        }
        if error_count > 0 {
            lines.push(format!("{error_count} error"));
        }
        Some(lines.join("\n"))
    } else {
        None
    };

    let tooltip = match &active {
        Some(detail) => format!("JSTorrent\n{detail}"),
        None => "JSTorrent".to_string(),
    };
    let _ = tray.set_tooltip(Some(&tooltip));

    // On macOS, show speed in menu bar next to icon
    #[cfg(target_os = "macos")]
    if download_speed > 0.0 || upload_speed > 0.0 {
        let _ = tray.set_title(Some(&format!(
            "\u{2193} {}/s",
            format_bytes(download_speed)
        )));
    } else {
        let _ = tray.set_title(Some(""));
    }
}

#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
fn show_notification(app: tauri::AppHandle, title: String, body: String) {
    std::thread::spawn(move || {
        show_notification_native(&app, &title, &body);
    });
}

#[cfg(target_os = "macos")]
fn show_notification_native(app: &tauri::AppHandle, title: &str, body: &str) {
    let bundle_id = &app.config().identifier;
    let _ = mac_notification_sys::set_application(bundle_id);

    let response = mac_notification_sys::Notification::new()
        .title(title)
        .message(body)
        .wait_for_click(true)
        .send();

    if let Ok(mac_notification_sys::NotificationResponse::Click) = response {
        show_main_window(app);
    }
}

#[cfg(target_os = "linux")]
fn show_notification_native(app: &tauri::AppHandle, title: &str, body: &str) {
    let result = notify_rust::Notification::new()
        .summary(title)
        .body(body)
        .action("default", "Open")
        .show();

    if let Ok(handle) = result {
        let app = app.clone();
        handle.wait_for_action(move |action| {
            if action == "default" || action == "__closed" {
                // "__closed" means user clicked the notification body on some DEs
            }
            if action == "default" {
                show_main_window(&app);
            }
        });
    }
}

#[cfg(target_os = "windows")]
fn show_notification_native(app: &tauri::AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    let _ = app.notification().builder().title(title).body(body).show();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Generate context once (the macro embeds static symbols, so it can't be called twice)
    let context = tauri::generate_context!();

    // Check for headless updater mode before building the full app
    let args: Vec<String> = std::env::args().collect();
    let check_update = args.iter().any(|a| a == "--check-update");
    let auto_update = args.iter().any(|a| a == "--auto-update");
    if check_update || auto_update {
        headless_updater::run(auto_update, context);
        return;
    }

    // Parse --force-desktop and --profile <id>
    let force_desktop = args.iter().any(|a| a == "--force-desktop");
    let cli_profile_id = args
        .iter()
        .position(|a| a == "--profile")
        .and_then(|i| args.get(i + 1).cloned());

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // Second instance launched (e.g., magnet link clicked on Windows/Linux).
            // Route deep link URLs through the routing logic.
            let mut any_deep_link = false;
            for arg in &args {
                match handle_deep_link_routed(app, arg) {
                    RouteResult::Extension | RouteResult::Desktop => {
                        any_deep_link = true;
                    }
                    RouteResult::NotRecognized => {}
                }
            }
            if !any_deep_link {
                show_main_window(app);
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_nosleep::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .invoke_handler(tauri::generate_handler![
            js_log,
            host_handshake,
            host_message,
            pick_download_folder,
            get_pending_deep_links,
            update_tray_stats,
            show_notification,
            restart_app,
            mark_desktop_activated,
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.app_handle().state::<Mutex<Settings>>();
                if state.lock().unwrap().run_in_background {
                    // Hide window but keep webview alive (downloads continue)
                    let _ = window.hide();
                    api.prevent_close();
                } else {
                    // "Run in Background" is off — user intends to fully quit
                    window.app_handle().exit(0);
                }
            }
        })
        .setup(move |app| {
            // Auto-updater with check-for-update ID header
            #[cfg(desktop)]
            {
                let mut builder = tauri_plugin_updater::Builder::new();
                if let Some(cfu_id) = jstorrent_common::get_or_create_cfu_id() {
                    builder = builder.header("X-CFU-Id", &cfu_id)?;
                }
                app.handle().plugin(builder.build())?;
            }

            // Store CLI launch args
            app.manage(LaunchArgs {
                force_desktop,
                profile_id: cli_profile_id,
            });

            // Settings
            let settings = load_settings(app.handle());
            app.manage(Mutex::new(settings.clone()));

            // Helper: build the settings submenu items for a menu.
            // Each menu needs its own item instances (macOS NSMenuItem can only
            // have one parent), so we create fresh items per menu.
            let build_settings_menu = |app: &tauri::App,
                                       settings: &Settings|
             -> Result<
                tauri::menu::Submenu<tauri::Wry>,
                Box<dyn std::error::Error>,
            > {
                let autostart_i = CheckMenuItem::with_id(
                    app,
                    "autostart",
                    "Start at Login",
                    true,
                    settings.autostart,
                    None::<&str>,
                )?;
                let background_i = CheckMenuItem::with_id(
                    app,
                    "run-in-background",
                    "Run in Background",
                    true,
                    settings.run_in_background,
                    None::<&str>,
                )?;
                let builder = SubmenuBuilder::new(app, "Settings")
                    .item(&autostart_i)
                    .item(&background_i);
                #[cfg(target_os = "macos")]
                let builder = {
                    let show_in_menu_bar_i = CheckMenuItem::with_id(
                        app,
                        "show-in-menu-bar",
                        "Show Icon in Menu Bar",
                        true,
                        settings.show_in_menu_bar,
                        None::<&str>,
                    )?;
                    builder.item(&show_in_menu_bar_i)
                };
                Ok(builder.build()?)
            };

            // macOS native app menu bar (built first, before tray, so items
            // don't get stolen from the tray menu by AppKit reparenting)
            #[cfg(target_os = "macos")]
            {
                let app_settings_menu = build_settings_menu(app, &settings)?;
                let app_submenu = SubmenuBuilder::new(app, "JSTorrent")
                    .about(Some(tauri::menu::AboutMetadata {
                        name: Some("JSTorrent".to_string()),
                        version: Some(app.config().version.clone().unwrap_or_default()),
                        website: Some("https://jstorrent.com".to_string()),
                        ..Default::default()
                    }))
                    .separator()
                    .items(&[
                        &MenuItem::with_id(
                            app,
                            "check-updates",
                            "Check for Updates",
                            true,
                            None::<&str>,
                        )?,
                        &MenuItem::with_id(
                            app,
                            "open-extension",
                            "Open Extension",
                            true,
                            None::<&str>,
                        )?,
                    ])
                    .separator()
                    .item(&app_settings_menu)
                    .separator()
                    .hide()
                    .hide_others()
                    .show_all()
                    .separator()
                    .quit()
                    .build()?;

                let app_menu = Menu::with_items(app, &[&app_submenu])?;
                app.set_menu(app_menu)?;
            }

            // System tray (separate item instances — no sharing with app menu)
            let tray_settings_menu = build_settings_menu(app, &settings)?;
            let tray_menu = {
                let show_i = MenuItem::with_id(app, "show", "Show App", true, None::<&str>)?;
                let open_ext_i =
                    MenuItem::with_id(app, "open-extension", "Open Extension", true, None::<&str>)?;
                let update_i = MenuItem::with_id(
                    app,
                    "check-updates",
                    "Check for Updates",
                    true,
                    None::<&str>,
                )?;
                let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
                let sep1 = PredefinedMenuItem::separator(app)?;
                let sep2 = PredefinedMenuItem::separator(app)?;

                Menu::with_items(
                    app,
                    &[
                        &show_i,
                        &open_ext_i,
                        &update_i,
                        &sep1,
                        &tray_settings_menu,
                        &sep2,
                        &quit_i,
                    ],
                )?
            };

            // On macOS, collect all CheckMenuItems from both the app menu
            // and tray menu so we can keep their checked state in sync.
            #[cfg(target_os = "macos")]
            {
                let mut sync_map: HashMap<String, Vec<CheckMenuItem<tauri::Wry>>> = HashMap::new();
                fn collect_checks(
                    items: Vec<MenuItemKind<tauri::Wry>>,
                    map: &mut HashMap<String, Vec<CheckMenuItem<tauri::Wry>>>,
                ) {
                    for item in items {
                        match item {
                            MenuItemKind::Check(c) => {
                                map.entry(c.id().as_ref().to_string()).or_default().push(c);
                            }
                            MenuItemKind::Submenu(sub) => {
                                collect_checks(sub.items().unwrap_or_default(), map);
                            }
                            _ => {}
                        }
                    }
                }
                if let Some(app_menu) = app.menu() {
                    collect_checks(app_menu.items().unwrap_or_default(), &mut sync_map);
                }
                collect_checks(tray_menu.items().unwrap_or_default(), &mut sync_map);
                app.manage(CheckItemSync(sync_map));
            }

            // Register a single global menu handler for both app-menu and
            // tray-menu events.  Previously each menu had its own handler,
            // which caused tray items to fire twice on macOS (once from
            // app.on_menu_event, once from the tray's on_menu_event).
            app.on_menu_event(move |app, event| {
                handle_menu_event(app, event.id.as_ref());
            });

            // Load icon from PNG at runtime instead of using
            // app.default_window_icon() — Tauri's codegen only reads the
            // first entry from ICO files, producing a broken icon on Windows.
            // See https://github.com/tauri-apps/tauri/issues/14596
            let tray_icon = tauri::image::Image::from_bytes(include_bytes!("../icons/icon.png"))
                .expect("failed to load tray icon PNG");

            TrayIconBuilder::with_id("tray")
                .tooltip("JSTorrent")
                .icon(tray_icon)
                .menu(&tray_menu)
                .show_menu_on_left_click(cfg!(target_os = "macos"))
                .on_tray_icon_event(|tray, event| {
                    if !cfg!(target_os = "macos") {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            show_main_window(tray.app_handle());
                        }
                    }
                })
                .build(app)?;

            // Hide tray icon if user disabled "Show Icon in Menu Bar" (macOS only)
            #[cfg(target_os = "macos")]
            if !settings.show_in_menu_bar {
                if let Some(tray) = app.tray_by_id("tray") {
                    let _ = tray.set_visible(false);
                }
            }

            // Deep links
            let deep_link_state = DeepLinkState {
                pending: Mutex::new(Vec::new()),
            };

            // Track whether startup deep links were routed to extension
            // (used to decide whether to show the window at end of setup).
            let mut startup_routed_to_extension = false;
            // jstorrent:// deep links force desktop mode (launched from launch page fallback)
            let mut force_desktop_from_deep_link = false;

            // Collect any URLs that launched the app (startup deep links).
            // Route to extension or queue as pending for the frontend.
            let deep_link_result = app.deep_link().get_current();
            eprintln!("[deep-link] get_current() = {deep_link_result:?}");
            if let Ok(Some(urls)) = deep_link_result {
                let rpc_info = read_rpc_info();
                eprintln!("[deep-link] Processing {} startup URL(s), route_to_ext={}", urls.len(), should_route_to_extension(&rpc_info));

                for url in urls {
                    let url_str: &str = url.as_ref();
                    eprintln!("[deep-link] URL: {url_str}");

                    // jstorrent:// links always force desktop (launch page fallback).
                    // They may carry a magnet in the query string.
                    if url_str.starts_with("jstorrent:") {
                        force_desktop_from_deep_link = true;
                        if let Some(magnet) = extract_magnet_from_jstorrent_url(url_str) {
                            if let Ok(mut pending) = deep_link_state.pending.lock() {
                                if let Some(event) = deep_link_event(&magnet) {
                                    pending.push(event);
                                }
                            }
                        }
                        continue;
                    }

                    let is_magnet = url_str.starts_with("magnet:");
                    let is_torrent = url_str.to_lowercase().ends_with(".torrent");

                    if !is_magnet && !is_torrent {
                        continue;
                    }

                    if should_route_to_extension(&rpc_info) {
                        let add_token = rpc_info.add_token.as_deref();
                        if is_magnet {
                            route_magnet_to_extension(app.handle(), url_str, add_token);
                        } else {
                            let path = file_url_to_path(url_str);
                            route_torrent_to_extension(app.handle(), &path, add_token);
                        }
                        startup_routed_to_extension = true;
                    } else if let Ok(mut pending) = deep_link_state.pending.lock() {
                        if let Some(event) = deep_link_event(url_str) {
                            pending.push(event);
                        }
                    }
                }
            }

            app.manage(deep_link_state);

            // --- Early exit: route to extension and quit ---
            // If the routing heuristic says extension, open the launch URL and
            // exit immediately. No sidecar, no tray, no event loop needed —
            // the extension launches its own native host.
            // Skip this entirely when --force-desktop is set (launched from extension).
            let launch_args = app.state::<LaunchArgs>();
            let skip_extension_routing = launch_args.force_desktop || force_desktop_from_deep_link;
            let startup_action = if skip_extension_routing {
                StartupAction::ShowDesktop
            } else {
                determine_startup_action(startup_routed_to_extension, &read_rpc_info())
            };
            eprintln!("[deep-link] startup_action={startup_action:?}, skip_extension_routing={skip_extension_routing}, startup_routed_to_extension={startup_routed_to_extension}");
            if !matches!(startup_action, StartupAction::ShowDesktop) {
                // Register native messaging manifests so the extension can find
                // the native host binary (important on first install).
                native_host::register_native_messaging_hosts(app.handle()).ok();

                if matches!(startup_action, StartupAction::OpenExtension) {
                    let url = get_launch_url();
                    eprintln!("[deep-link] OpenExtension: opening bare launch URL: {url}");
                    let _ = app.opener().open_url(&url, None::<&str>);
                }
                // AlreadyRouted: deep links were sent to extension above.

                eprintln!("Routed to extension, exiting Tauri app");
                std::process::exit(0);
            }

            // --- Desktop path: full app setup ---

            // Handle deep links received while the app is already running.
            // On macOS, the OS routes URLs to the running process via this handler.
            // On Windows/Linux, the single-instance plugin (registered above) forwards
            // the second instance's args to this instance and exits the duplicate.
            let deep_link_handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                let urls = event.urls();
                eprintln!("[deep-link] on_open_url: {} URL(s)", urls.len());
                let mut any_deep_link = false;
                for url in urls {
                    let url_str: &str = url.as_ref();
                    eprintln!("[deep-link] on_open_url URL: {url_str}");
                    // jstorrent:// links always show desktop (launch page fallback)
                    if url_str.starts_with("jstorrent:") {
                        if let Some(magnet) = extract_magnet_from_jstorrent_url(url_str) {
                            if let Some(event) = deep_link_event(&magnet) {
                                let _ = deep_link_handle.emit("host-event", &event);
                            }
                        }
                        show_main_window(&deep_link_handle);
                        any_deep_link = true;
                        continue;
                    }
                    match handle_deep_link_routed(&deep_link_handle, url_str) {
                        RouteResult::Extension | RouteResult::Desktop => {
                            any_deep_link = true;
                        }
                        RouteResult::NotRecognized => {}
                    }
                }
                if !any_deep_link {
                    show_main_window(&deep_link_handle);
                }
            });

            // Register URL scheme handlers at runtime (Windows/Linux only).
            // macOS uses Info.plist entries generated from tauri.conf.json at build time.
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            app.deep_link().register_all()?;

            // Register native messaging host manifest for all detected browsers
            if let Err(e) = native_host::register_native_messaging_hosts(app.handle()) {
                eprintln!("native-host: registration failed: {e}");
            }

            // Spawn native host sidecar
            let host_path = resolve_sidecar(app.handle(), "binaries/jstorrent-host")?;
            eprintln!("Spawning native host: {}", host_path.display());

            let mut cmd = std::process::Command::new(&host_path);
            cmd.arg("--launcher")
                .arg("tauri")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::inherit());
            // Prevent a visible console window for the sidecar on Windows
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            }
            let mut child = cmd
                .spawn()
                .map_err(|e| format!("Failed to spawn native host: {e}"))?;

            let stdin = child.stdin.take().expect("stdin not captured");
            let mut stdout = child.stdout.take().expect("stdout not captured");

            let bridge = Arc::new(HostBridge {
                stdin: Mutex::new(stdin),
                pending: Mutex::new(HashMap::new()),
            });

            app.manage(bridge.clone());

            // Background stdout reader on a dedicated OS thread.
            // When stdout closes (sidecar died, e.g. killed by extension TakeOver),
            // exit the Tauri app so it doesn't linger as a headless window.
            let app_handle = app.handle().clone();
            std::thread::spawn(move || {
                let _child = child; // Keep child handle alive
                run_stdout_reader(&mut stdout, &bridge, &app_handle);
                eprintln!("native host: sidecar exited, shutting down Tauri app");
                app_handle.exit(0);
            });

            show_main_window(app.handle());

            Ok(())
        })
        .build(context)
        .unwrap_or_else(|e| fatal_error(&format!("Failed to start JSTorrent: {e}")));

    // Keep app alive when all windows are hidden (user closes window -> hide, not exit).
    // Explicit quit via tray menu calls app.exit(0), which sets code = Some(0).
    app.run(|app_handle, event| {
        #[cfg(not(target_os = "macos"))]
        let _ = app_handle;

        match event {
            tauri::RunEvent::ExitRequested { api, code, .. }
                // Keep app alive for tray when windows close.
                // Only app.exit(0) from Quit menu (code=Some(0)) actually exits.
                if code.is_none() => {
                    api.prevent_exit();
                }
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => {
                show_main_window(app_handle);
            }
            _ => {}
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use jstorrent_common::{BrowserInfo, DownloadRoot, ProfileEntry};

    fn make_profile(
        client_type: Option<&str>,
        desktop_ever_used: bool,
        client_types_used: &[&str],
        last_used: u64,
        has_roots: bool,
    ) -> ProfileEntry {
        ProfileEntry {
            extension_id: None,
            profile_id: format!("p-{last_used}"),
            display_name: String::new(),
            created: 1000,
            client_type: client_type.map(String::from),
            client_version: None,
            pid: 0,
            port: 0,
            token: String::new(),
            started: 1000,
            last_used,
            browser: BrowserInfo {
                name: String::new(),
                binary: String::new(),
                extension_id: None,
            },
            download_roots: if has_roots {
                vec![DownloadRoot {
                    key: "k".into(),
                    path: "/tmp".into(),
                    display_name: "Test".into(),
                    removable: false,
                    last_stat_ok: true,
                    last_checked: 0,
                    disk_id: String::new(),
                }]
            } else {
                vec![]
            },
            launcher: None,
            desktop_ever_used,
            client_types_used: client_types_used.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    fn rpc(profiles: Vec<ProfileEntry>) -> RpcInfo {
        RpcInfo {
            version: 1,
            add_token: Some("test-token".into()),
            profiles,
        }
    }

    #[test]
    fn test_routing_fresh_install() {
        let r = rpc(vec![]);
        assert!(!should_route_to_extension(&r));
    }

    #[test]
    fn test_routing_extension_only() {
        let r = rpc(vec![make_profile(
            Some("extension"),
            false,
            &["extension"],
            2000,
            true,
        )]);
        assert!(should_route_to_extension(&r));
    }

    #[test]
    fn test_routing_desktop_used() {
        let r = rpc(vec![make_profile(
            Some("tauri"),
            true,
            &["tauri"],
            2000,
            true,
        )]);
        assert!(!should_route_to_extension(&r));
    }

    #[test]
    fn test_routing_most_recent_extension_wins() {
        let r = rpc(vec![
            make_profile(Some("tauri"), true, &["tauri"], 1000, true),
            make_profile(Some("extension"), false, &["extension"], 2000, true),
        ]);
        assert!(should_route_to_extension(&r));
    }

    #[test]
    fn test_routing_most_recent_desktop_wins() {
        let r = rpc(vec![
            make_profile(Some("extension"), false, &["extension"], 1000, true),
            make_profile(Some("tauri"), true, &["tauri"], 2000, true),
        ]);
        assert!(!should_route_to_extension(&r));
    }

    #[test]
    fn test_routing_desktop_handshake_without_activation() {
        // Scenario: extension used first, then desktop opened via "Open in Desktop".
        // desktop_ever_used is still false (user hasn't added a torrent yet),
        // but client_types_used includes "tauri" from the handshake.
        // The most-recent heuristic should still pick desktop.
        let r = rpc(vec![make_profile(
            Some("tauri"),
            false,
            &["extension", "tauri"],
            2000,
            true,
        )]);
        assert!(!should_route_to_extension(&r));
    }

    #[test]
    fn test_routing_extension_no_roots() {
        let r = rpc(vec![make_profile(
            Some("extension"),
            false,
            &["extension"],
            2000,
            false,
        )]);
        assert!(should_route_to_extension(&r));
    }

    // -- Startup action scenarios (bare app launch, no deep link) --

    #[test]
    fn test_startup_fresh_install_shows_desktop() {
        let r = rpc(vec![]);
        assert_eq!(
            determine_startup_action(false, &r),
            StartupAction::ShowDesktop
        );
    }

    #[test]
    fn test_startup_extension_only_user_opens_extension() {
        let r = rpc(vec![make_profile(
            Some("extension"),
            false,
            &["extension"],
            2000,
            true,
        )]);
        assert_eq!(
            determine_startup_action(false, &r),
            StartupAction::OpenExtension
        );
    }

    #[test]
    fn test_startup_desktop_user_shows_desktop() {
        let r = rpc(vec![make_profile(
            Some("tauri"),
            true,
            &["tauri"],
            2000,
            true,
        )]);
        assert_eq!(
            determine_startup_action(false, &r),
            StartupAction::ShowDesktop
        );
    }

    #[test]
    fn test_startup_deep_links_already_routed() {
        let r = rpc(vec![]);
        assert_eq!(
            determine_startup_action(true, &r),
            StartupAction::AlreadyRouted
        );
    }

    // -- Settings serde --

    #[test]
    fn test_settings_serde_backward_compat() {
        // Old settings with magnet_handler field should still deserialize
        let json =
            r#"{"autostart": false, "run_in_background": true, "magnet_handler": "desktop"}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert!(!s.autostart);
        assert!(s.run_in_background);
    }

    #[test]
    fn test_settings_serde_roundtrip() {
        let s = Settings::default();
        let json = serde_json::to_string(&s).unwrap();
        let parsed: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.autostart, s.autostart);
        assert_eq!(parsed.run_in_background, s.run_in_background);
    }

    // -- jstorrent:// URL parsing --

    #[test]
    fn test_extract_magnet_from_jstorrent_url() {
        assert_eq!(
            extract_magnet_from_jstorrent_url("jstorrent://launch?magnet=magnet%3A%3Fxt%3Durn"),
            Some("magnet:?xt=urn".to_string())
        );
        assert_eq!(
            extract_magnet_from_jstorrent_url("jstorrent://launch"),
            None
        );
        assert_eq!(
            extract_magnet_from_jstorrent_url("jstorrent://launch?other=value"),
            None
        );
    }

    // -- Auto-updater config validation --
    // These tests catch accidental breakage of the updater config in tauri.conf.json.
    // If any of these fail, auto-updates would be silently broken for all users.

    fn load_tauri_conf() -> serde_json::Value {
        serde_json::from_str(include_str!("../tauri.conf.json"))
            .expect("tauri.conf.json must be valid JSON")
    }

    #[test]
    fn test_updater_config_endpoint_valid() {
        let conf = load_tauri_conf();
        let endpoint = conf["plugins"]["updater"]["endpoints"][0]
            .as_str()
            .expect("updater endpoint must be a string");

        assert!(
            endpoint.starts_with("https://"),
            "updater endpoint must use HTTPS: {endpoint}"
        );
        assert!(
            endpoint.contains("{{target}}"),
            "updater endpoint missing {{{{target}}}} placeholder: {endpoint}"
        );
        assert!(
            endpoint.contains("{{arch}}"),
            "updater endpoint missing {{{{arch}}}} placeholder: {endpoint}"
        );
        assert!(
            endpoint.contains("{{current_version}}"),
            "updater endpoint missing {{{{current_version}}}} placeholder: {endpoint}"
        );
    }

    #[test]
    fn test_updater_config_pubkey_valid() {
        use base64::Engine;

        let conf = load_tauri_conf();
        let pubkey = conf["plugins"]["updater"]["pubkey"]
            .as_str()
            .expect("updater pubkey must be a string");

        let decoded = base64::engine::general_purpose::STANDARD
            .decode(pubkey)
            .expect("updater pubkey must be valid base64");

        // A minisign public key (with untrusted comment + key line) is ~74 bytes.
        assert!(
            decoded.len() > 32,
            "updater pubkey suspiciously short ({} bytes) — may be corrupted",
            decoded.len()
        );
    }

    #[test]
    fn test_updater_artifacts_enabled() {
        let conf = load_tauri_conf();
        let value = &conf["bundle"]["createUpdaterArtifacts"];
        // Can be bool `true` or string `"v2"` depending on Tauri version
        let enabled = value.as_bool().unwrap_or(false)
            || value.as_str().is_some_and(|s| s == "true" || s == "v2");
        assert!(
            enabled,
            "bundle.createUpdaterArtifacts must be true — updater artifacts won't be generated in CI"
        );
    }
}
