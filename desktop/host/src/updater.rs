use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::kv_store::KvStore;

const CHECK_TIMEOUT: Duration = Duration::from_mins(1);
const INSTALL_TIMEOUT: Duration = Duration::from_mins(5);
const AUTO_CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60; // 24 hours
const KV_LAST_CHECK_KEY: &str = "update:lastCheckTime";
const RESULT_FILENAME: &str = "update-check-result.json";

const UPDATE_ENDPOINT: &str = "https://updates.jstorrent.com/tauri";

const TARGET: &str = if cfg!(target_os = "macos") {
    "darwin"
} else if cfg!(target_os = "windows") {
    "windows"
} else {
    "linux"
};

const ARCH: &str = if cfg!(target_arch = "aarch64") {
    "aarch64"
} else {
    "x86_64"
};

/// Result read from the JSON file written by the headless Tauri updater.
#[derive(Debug, Deserialize, Default)]
pub struct UpdateCheckResult {
    pub available: bool,
    pub version: Option<String>,
    #[serde(rename = "currentVersion")]
    pub current_version: Option<String>,
    pub body: Option<String>,
    pub error: Option<String>,
}

/// Response from the Tauri update endpoint (HTTP 200).
#[derive(Deserialize)]
struct TauriUpdateResponse {
    version: String,
    notes: Option<String>,
}

/// Check for updates by hitting the Tauri update endpoint directly.
/// No Tauri app spawn needed — just an HTTP request.
pub async fn check_for_updates_http(current_version: &str) -> Result<UpdateCheckResult> {
    let url = format!("{UPDATE_ENDPOINT}/{TARGET}/{ARCH}/{current_version}");
    crate::log!("Checking for updates: {url}");

    let cfu_id = jstorrent_common::get_or_create_cfu_id().unwrap_or_default();
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .timeout(Duration::from_secs(15))
        .header("X-CFU-Id", &cfu_id)
        .header("X-Check-Reason", "host")
        .send()
        .await
        .context("Update check HTTP request failed")?;

    match resp.status().as_u16() {
        204 => Ok(UpdateCheckResult {
            available: false,
            version: None,
            current_version: Some(current_version.to_string()),
            body: None,
            error: None,
        }),
        200 => {
            let update: TauriUpdateResponse = resp
                .json()
                .await
                .context("Failed to parse update response")?;
            Ok(UpdateCheckResult {
                available: true,
                version: Some(update.version),
                current_version: Some(current_version.to_string()),
                body: update.notes,
                error: None,
            })
        }
        status => Ok(UpdateCheckResult {
            available: false,
            version: None,
            current_version: Some(current_version.to_string()),
            body: None,
            error: Some(format!("Update server returned HTTP {status}")),
        }),
    }
}

/// Run a headless update check by spawning the Tauri app with CLI flags.
/// Returns the parsed result from the JSON file it writes.
pub async fn run_update_check(auto_update: bool) -> Result<UpdateCheckResult> {
    let app_path = find_tauri_app_path()?;
    let flag = if auto_update {
        "--auto-update"
    } else {
        "--check-update"
    };
    let timeout = if auto_update {
        INSTALL_TIMEOUT
    } else {
        CHECK_TIMEOUT
    };

    crate::log!("Spawning Tauri updater: {} {}", app_path.display(), flag);

    let child = spawn_tauri_app(&app_path, flag)?;
    let exit_status = wait_with_timeout(child, timeout).await?;

    crate::log!("Tauri updater exited with status: {:?}", exit_status);

    // Read the result file
    read_result_file()
}

/// Check if enough time has passed since the last auto-check.
pub fn should_auto_check(kv: &KvStore) -> bool {
    let last_check: u64 = kv
        .get(KV_LAST_CHECK_KEY)
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    now.saturating_sub(last_check) >= AUTO_CHECK_INTERVAL_SECS
}

/// Record the current time as the last auto-check time.
pub fn record_check_time(kv: &KvStore) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let _ = kv.set(KV_LAST_CHECK_KEY, &now.to_string());
}

/// Find the Tauri app binary path.
///
/// On macOS: The Tauri app is installed as a .app bundle (typically /Applications/JSTorrent.app).
/// On Windows: Same directory as the native host binary (JSTorrent.exe).
/// On Linux: Same directory as the native host binary (`JSTorrent` or `jstorrent`).
pub(crate) fn find_tauri_app_path() -> Result<PathBuf> {
    let exe_path = std::env::current_exe()?;
    let exe_dir = exe_path
        .parent()
        .context("Failed to get executable directory")?;

    #[cfg(target_os = "macos")]
    {
        // On macOS, the native host lives in its own .app bundle under ~/Library/Application Support/JSTorrent/
        // but the Tauri app is at /Applications/JSTorrent.app or ~/Applications/JSTorrent.app.
        let candidates = [
            PathBuf::from("/Applications/JSTorrent.app"),
            dirs::home_dir()
                .map(|h| h.join("Applications/JSTorrent.app"))
                .unwrap_or_default(),
        ];

        for candidate in &candidates {
            if candidate.exists() {
                return Ok(candidate.clone());
            }
        }

        // Also check if we're a Tauri sidecar (host inside JSTorrent.app/Contents/MacOS/)
        let exe_dir_str = exe_dir.to_string_lossy();
        if exe_dir_str.contains("JSTorrent.app/Contents/MacOS") {
            // Walk up to the .app bundle
            if let Some(app_bundle) = exe_dir.parent().and_then(|p| p.parent()) {
                if app_bundle.extension().is_some_and(|e| e == "app") {
                    return Ok(app_bundle.to_path_buf());
                }
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let ext = if cfg!(windows) { ".exe" } else { "" };
        let mut candidates = vec![
            exe_dir.join(format!("JSTorrent{ext}")),
            exe_dir.join(format!("jstorrent{ext}")),
        ];

        // AppImage: the host binary lives at ~/.local/lib/jstorrent/ but the
        // AppImage is at ~/.local/bin/JSTorrent.AppImage (installed by install.sh)
        #[cfg(target_os = "linux")]
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join(".local/bin/JSTorrent.AppImage"));
        }

        for candidate in &candidates {
            if candidate.exists() {
                return Ok(candidate.clone());
            }
        }
    }

    anyhow::bail!("Tauri app not found. Host is at: {}", exe_path.display())
}

/// Spawn the Tauri app with the given flag.
fn spawn_tauri_app(app_path: &std::path::Path, flag: &str) -> Result<tokio::process::Child> {
    #[cfg(target_os = "macos")]
    {
        // Use `open -a` on macOS to respect Gatekeeper and launch the .app bundle properly.
        // -g: open in background (don't steal focus from Chrome)
        let child = tokio::process::Command::new("open")
            .arg("-g")
            .arg("-a")
            .arg(app_path)
            .arg("-W") // Wait for the app to exit
            .arg("--args")
            .arg(flag)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .with_context(|| {
                format!("Failed to spawn Tauri app via open: {}", app_path.display())
            })?;
        Ok(child)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let child = tokio::process::Command::new(app_path)
            .arg(flag)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .with_context(|| format!("Failed to spawn Tauri app: {}", app_path.display()))?;
        Ok(child)
    }
}

/// Wait for a child process with a timeout. Kills the child on timeout.
async fn wait_with_timeout(
    mut child: tokio::process::Child,
    timeout: Duration,
) -> Result<std::process::ExitStatus> {
    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => Ok(status),
        Ok(Err(e)) => Err(anyhow::anyhow!("Failed to wait for Tauri updater: {e}")),
        Err(_) => {
            crate::log!("Tauri updater timed out after {:?}, killing", timeout);
            let _ = child.kill().await;
            anyhow::bail!("Tauri updater timed out after {timeout:?}")
        }
    }
}

/// Launch the Tauri desktop app with --force-desktop and optional --profile.
/// Fire-and-forget: spawns the process detached and returns immediately.
pub(crate) fn launch_desktop_app(profile_id: Option<&str>) -> Result<()> {
    let app_path = find_tauri_app_path()?;
    crate::log!(
        "Launching Tauri desktop app: {} (profile: {:?})",
        app_path.display(),
        profile_id
    );

    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("open");
        cmd.arg("-a")
            .arg(&app_path)
            .arg("--args")
            .arg("--force-desktop");
        if let Some(pid) = profile_id {
            cmd.arg("--profile").arg(pid);
        }
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .with_context(|| {
                format!(
                    "Failed to launch Tauri desktop app via open: {}",
                    app_path.display()
                )
            })?;
    }

    #[cfg(not(target_os = "macos"))]
    {
        let mut cmd = std::process::Command::new(&app_path);
        cmd.arg("--force-desktop");
        if let Some(pid) = profile_id {
            cmd.arg("--profile").arg(pid);
        }
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .with_context(|| {
                format!("Failed to launch Tauri desktop app: {}", app_path.display())
            })?;
    }

    Ok(())
}

/// Read the result file written by the headless Tauri updater.
fn read_result_file() -> Result<UpdateCheckResult> {
    let config_dir = jstorrent_common::get_config_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;
    let path = config_dir.join("jstorrent-native").join(RESULT_FILENAME);

    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read update result file: {}", path.display()))?;

    serde_json::from_str(&contents)
        .with_context(|| format!("Failed to parse update result file: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kv_store::KvStore;

    #[test]
    fn should_auto_check_empty_kv_returns_true() {
        let kv = KvStore::open_in_memory().unwrap();
        assert!(should_auto_check(&kv), "empty KV should trigger check");
    }

    #[test]
    fn should_auto_check_after_record_returns_false() {
        let kv = KvStore::open_in_memory().unwrap();
        record_check_time(&kv);
        assert!(
            !should_auto_check(&kv),
            "immediately after recording should NOT trigger check"
        );
    }

    #[test]
    fn should_auto_check_stale_timestamp_returns_true() {
        let kv = KvStore::open_in_memory().unwrap();
        // Set timestamp to 25 hours ago
        let stale = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - (25 * 60 * 60);
        kv.set(KV_LAST_CHECK_KEY, &stale.to_string()).unwrap();
        assert!(
            should_auto_check(&kv),
            "25h-old timestamp should trigger check"
        );
    }

    #[test]
    fn should_auto_check_recent_timestamp_returns_false() {
        let kv = KvStore::open_in_memory().unwrap();
        // Set timestamp to 23 hours ago
        let recent = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - (23 * 60 * 60);
        kv.set(KV_LAST_CHECK_KEY, &recent.to_string()).unwrap();
        assert!(
            !should_auto_check(&kv),
            "23h-old timestamp should NOT trigger check"
        );
    }

    #[test]
    fn should_auto_check_exactly_24h_returns_true() {
        let kv = KvStore::open_in_memory().unwrap();
        let boundary = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - AUTO_CHECK_INTERVAL_SECS;
        kv.set(KV_LAST_CHECK_KEY, &boundary.to_string()).unwrap();
        assert!(
            should_auto_check(&kv),
            "exactly 24h-old timestamp should trigger check (>= comparison)"
        );
    }

    #[test]
    fn should_auto_check_corrupt_value_returns_true() {
        let kv = KvStore::open_in_memory().unwrap();
        kv.set(KV_LAST_CHECK_KEY, "not-a-number").unwrap();
        assert!(
            should_auto_check(&kv),
            "corrupt value should fall back to 0 and trigger check"
        );
    }

    #[test]
    fn record_check_time_writes_recent_timestamp() {
        let kv = KvStore::open_in_memory().unwrap();
        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        record_check_time(&kv);
        let after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let stored: u64 = kv.get(KV_LAST_CHECK_KEY).unwrap().unwrap().parse().unwrap();
        assert!(
            stored >= before && stored <= after,
            "recorded timestamp {stored} should be between {before} and {after}"
        );
    }

    #[test]
    fn record_then_check_then_stale_roundtrip() {
        let kv = KvStore::open_in_memory().unwrap();

        // Fresh KV → should check
        assert!(should_auto_check(&kv));

        // Record → should NOT check
        record_check_time(&kv);
        assert!(!should_auto_check(&kv));

        // Manually backdate → should check again
        let old = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - (48 * 60 * 60);
        kv.set(KV_LAST_CHECK_KEY, &old.to_string()).unwrap();
        assert!(should_auto_check(&kv));
    }

    #[test]
    fn update_check_result_deserialize() {
        let json =
            r#"{"available":true,"version":"1.2.0","currentVersion":"1.1.0","body":"notes"}"#;
        let result: UpdateCheckResult = serde_json::from_str(json).unwrap();
        assert!(result.available);
        assert_eq!(result.version.as_deref(), Some("1.2.0"));
        assert_eq!(result.current_version.as_deref(), Some("1.1.0"));
        assert_eq!(result.body.as_deref(), Some("notes"));
        assert!(result.error.is_none());
    }

    #[test]
    fn update_check_result_no_update() {
        let json = r#"{"available":false}"#;
        let result: UpdateCheckResult = serde_json::from_str(json).unwrap();
        assert!(!result.available);
        assert!(result.version.is_none());
    }
}
