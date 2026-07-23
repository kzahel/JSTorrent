use crate::protocol::Event;
use crate::state::State as AppState;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::sync::Arc;
use tokio::net::TcpListener;
use uuid::Uuid;

/// Info carried by main.rs to write into rpc-info.json.
/// Named `RpcWriteInfo` to avoid collision with `jstorrent_common::RpcInfo`.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RpcWriteInfo {
    pub version: String,
    pub pid: u32,
    pub port: u16,
    pub token: String,
    pub started: u64,
    pub last_used: u64,
    pub browser: BrowserInfo,
    /// None = don't update roots, Some(vec) = set roots to vec (even if empty)
    pub download_roots: Option<Vec<DownloadRoot>>,
    pub profile_id: String,
    pub display_name: String,
    pub created: u64,
    pub client_type: Option<String>,
    pub client_version: Option<String>,
    pub launcher: Option<String>,
    /// Client types to accumulate (appended, not overwritten)
    #[serde(default)]
    pub client_types_used: Vec<String>,
}

#[derive(Deserialize)]
pub struct TokenQuery {
    token: String,
}

#[derive(Deserialize)]
pub struct AddMagnetRequest {
    magnet: String,
}

#[derive(Deserialize)]
pub struct AddTorrentRequest {
    file_name: String,
    contents_base64: String,
}

#[derive(Serialize)]
pub struct HealthResponse {
    status: String,
    pid: u32,
    version: String,
}

#[derive(Serialize)]
pub struct StatusResponse {
    status: String,
    message: String,
}

pub async fn start_server(state: Arc<AppState>) -> anyhow::Result<(u16, String)> {
    let token = Uuid::new_v4().to_string();
    let token_clone = token.clone();

    let app = Router::new()
        .route("/health", get(health_handler))
        .route("/add-magnet", post(add_magnet_handler))
        .route("/add-torrent", post(add_torrent_handler))
        .with_state((state, token_clone));

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            crate::log!("RPC server error: {e}");
        }
    });

    Ok((port, token))
}

async fn health_handler(
    State((_, server_token)): State<(Arc<AppState>, String)>,
    Query(query): Query<TokenQuery>,
) -> Result<Json<HealthResponse>, StatusCode> {
    if query.token != server_token {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(Json(HealthResponse {
        status: "ok".to_string(),
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }))
}

async fn add_magnet_handler(
    State((state, server_token)): State<(Arc<AppState>, String)>,
    Query(query): Query<TokenQuery>,
    Json(payload): Json<AddMagnetRequest>,
) -> Result<Json<StatusResponse>, StatusCode> {
    if query.token != server_token {
        crate::log!("Refused add-magnet request: Invalid token");
        return Err(StatusCode::FORBIDDEN);
    }

    crate::log!("Received add-magnet request: {}", payload.magnet);

    if let Some(sender) = &state.event_sender {
        let event = Event::MagnetAdded {
            link: payload.magnet.clone(),
        };
        let _ = sender.send(event).await;
    }

    crate::log!("Magnet link queued successfully");

    Ok(Json(StatusResponse {
        status: "queued".to_string(),
        message: "Magnet link queued".to_string(),
    }))
}

async fn add_torrent_handler(
    State((state, server_token)): State<(Arc<AppState>, String)>,
    Query(query): Query<TokenQuery>,
    Json(payload): Json<AddTorrentRequest>,
) -> Result<Json<StatusResponse>, StatusCode> {
    if query.token != server_token {
        crate::log!("Refused add-torrent request: Invalid token");
        return Err(StatusCode::FORBIDDEN);
    }

    // Chrome native messaging limits messages to 1MB. Base64 adds ~33% overhead,
    // plus JSON wrapper. Reject files that would exceed this limit.
    const MAX_BASE64_SIZE: usize = 900_000; // ~675KB original, conservative margin
    if payload.contents_base64.len() > MAX_BASE64_SIZE {
        crate::log!(
            "Torrent file too large: {} bytes base64 (limit: {})",
            payload.contents_base64.len(),
            MAX_BASE64_SIZE
        );
        return Err(StatusCode::PAYLOAD_TOO_LARGE);
    }

    crate::log!(
        "Received add-torrent request: {} ({} bytes)",
        payload.file_name,
        payload.contents_base64.len()
    );

    if let Some(sender) = &state.event_sender {
        let event = Event::TorrentAdded {
            name: payload.file_name,
            infohash: String::new(), // Extension will calculate this
            contents_base64: payload.contents_base64,
        };

        let _ = sender.send(event).await;
    }

    crate::log!("Torrent file queued successfully");

    Ok(Json(StatusResponse {
        status: "queued".to_string(),
        message: "Torrent file queued".to_string(),
    }))
}

pub use jstorrent_common::{get_config_dir, BrowserInfo, DownloadRoot, ProfileEntry, RpcInfo};

/// Read the rpc-info.json file, returning empty profiles if missing or corrupt.
pub fn read_discovery_file() -> RpcInfo {
    let Some(config_dir) = get_config_dir() else {
        return RpcInfo {
            version: 1,
            add_token: None,
            profiles: Vec::new(),
        };
    };
    let rpc_file = config_dir.join("jstorrent-native").join("rpc-info.json");
    if rpc_file.exists() {
        std::fs::File::open(&rpc_file)
            .ok()
            .and_then(|f| serde_json::from_reader(f).ok())
            .unwrap_or(RpcInfo {
                version: 1,
                add_token: None,
                profiles: Vec::new(),
            })
    } else {
        RpcInfo {
            version: 1,
            add_token: None,
            profiles: Vec::new(),
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
pub fn write_discovery_file(info: RpcWriteInfo) -> anyhow::Result<Vec<DownloadRoot>> {
    let config_dir =
        get_config_dir().ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?;
    let app_dir = config_dir.join("jstorrent-native");

    if !app_dir.exists() {
        fs::create_dir_all(&app_dir)?;
    }

    let rpc_file = app_dir.join("rpc-info.json");

    let mut rpc_info = if rpc_file.exists() {
        let file = fs::File::open(&rpc_file)?;
        serde_json::from_reader(file).unwrap_or_else(|_| RpcInfo {
            version: 1,
            add_token: None,
            profiles: Vec::new(),
        })
    } else {
        RpcInfo {
            version: 1,
            add_token: None,
            profiles: Vec::new(),
        }
    };

    // Ensure add_token is generated if missing
    if rpc_info.add_token.is_none() {
        rpc_info.add_token = Some(Uuid::new_v4().to_string());
    }

    // Find existing entry by profile_id
    let found_idx = rpc_info
        .profiles
        .iter()
        .position(|p| p.profile_id == info.profile_id);

    let active_roots;

    if let Some(idx) = found_idx {
        // Update existing entry
        let mut entry = rpc_info.profiles[idx].clone();
        entry.pid = info.pid;
        entry.port = info.port;
        entry.token.clone_from(&info.token);
        entry.started = info.started;
        entry.last_used = info.last_used;
        // Update browser info, but preserve existing binary if new one doesn't exist on disk
        let new_binary = &info.browser.binary;
        if !new_binary.is_empty() && std::path::Path::new(new_binary).exists() {
            entry.browser.clone_from(&info.browser);
        } else {
            entry.browser.name.clone_from(&info.browser.name);
            entry
                .browser
                .extension_id
                .clone_from(&info.browser.extension_id);
        }
        entry.extension_id.clone_from(&info.browser.extension_id);

        // Update client metadata if provided
        if info.client_type.is_some() {
            entry.client_type.clone_from(&info.client_type);
        }
        if info.client_version.is_some() {
            entry.client_version.clone_from(&info.client_version);
        }

        // Update launcher if provided
        if info.launcher.is_some() {
            entry.launcher.clone_from(&info.launcher);
        }

        // Accumulate client_types_used (append new types, no duplicates)
        for ct in &info.client_types_used {
            if !entry.client_types_used.contains(ct) {
                entry.client_types_used.push(ct.clone());
            }
        }

        // desktop_ever_used is sticky — once true, stays true
        // (only set externally via mark_desktop_activated, never cleared)

        // Only update roots if explicitly provided (Some)
        // None means "don't update" - preserves existing roots on startup
        if let Some(roots) = &info.download_roots {
            entry.download_roots.clone_from(roots);
        }

        active_roots = entry.download_roots.clone();
        rpc_info.profiles[idx] = entry;
    } else {
        // New entry
        let new_entry = ProfileEntry {
            extension_id: info.browser.extension_id.clone(),
            profile_id: info.profile_id.clone(),
            display_name: info.display_name.clone(),
            created: info.created,
            client_type: info.client_type.clone(),
            client_version: info.client_version.clone(),
            pid: info.pid,
            port: info.port,
            token: info.token.clone(),
            started: info.started,
            last_used: info.last_used,
            browser: info.browser.clone(),
            download_roots: info.download_roots.clone().unwrap_or_default(),
            launcher: info.launcher.clone(),
            desktop_ever_used: false,
            client_types_used: info.client_types_used.clone(),
        };
        active_roots = new_entry.download_roots.clone();
        rpc_info.profiles.push(new_entry);
    }

    // Atomic write
    let temp_file = tempfile::NamedTempFile::new_in(&app_dir)?;
    serde_json::to_writer(&temp_file, &rpc_info)?;
    temp_file.as_file().sync_all()?;
    temp_file.persist(&rpc_file).map_err(|e| e.error)?;

    Ok(active_roots)
}

/// Check if a profile's daemon is alive by hitting its health endpoint.
pub async fn check_profile_liveness(port: u16, token: &str) -> bool {
    let url = format!("http://127.0.0.1:{port}/health?token={token}");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(100))
        .build();
    let Ok(client) = client else { return false };
    match client.get(&url).send().await {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// Rename a profile's display name in rpc-info.json.
/// Targeted read-modify-write that only touches `display_name` for one profile entry.
pub fn rename_profile(profile_id: &str, display_name: &str) -> anyhow::Result<()> {
    let config_dir =
        get_config_dir().ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?;
    let app_dir = config_dir.join("jstorrent-native");
    let rpc_file = app_dir.join("rpc-info.json");

    let mut rpc_info: RpcInfo = if rpc_file.exists() {
        let file = fs::File::open(&rpc_file)?;
        serde_json::from_reader(file).unwrap_or_else(|_| RpcInfo {
            version: 1,
            add_token: None,
            profiles: Vec::new(),
        })
    } else {
        return Err(anyhow::anyhow!("Profile not found: {profile_id}"));
    };

    let entry = rpc_info
        .profiles
        .iter_mut()
        .find(|p| p.profile_id == profile_id)
        .ok_or_else(|| anyhow::anyhow!("Profile not found: {profile_id}"))?;
    entry.display_name = display_name.to_string();

    // Atomic write via tempfile + rename
    let temp_file = tempfile::NamedTempFile::new_in(&app_dir)?;
    serde_json::to_writer(&temp_file, &rpc_info)?;
    temp_file.as_file().sync_all()?;
    temp_file.persist(&rpc_file).map_err(|e| e.error)?;

    Ok(())
}

/// Delete a profile from rpc-info.json and remove its data directory.
pub fn delete_profile(profile_id: &str) -> anyhow::Result<()> {
    let config_dir =
        get_config_dir().ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?;
    let app_dir = config_dir.join("jstorrent-native");
    let rpc_file = app_dir.join("rpc-info.json");

    let mut rpc_info: RpcInfo = if rpc_file.exists() {
        let file = fs::File::open(&rpc_file)?;
        serde_json::from_reader(file).unwrap_or_else(|_| RpcInfo {
            version: 1,
            add_token: None,
            profiles: Vec::new(),
        })
    } else {
        return Err(anyhow::anyhow!("Profile not found: {profile_id}"));
    };

    let len_before = rpc_info.profiles.len();
    rpc_info.profiles.retain(|p| p.profile_id != profile_id);
    if rpc_info.profiles.len() == len_before {
        return Err(anyhow::anyhow!("Profile not found: {profile_id}"));
    }

    // Atomic write via tempfile + rename
    let temp_file = tempfile::NamedTempFile::new_in(&app_dir)?;
    serde_json::to_writer(&temp_file, &rpc_info)?;
    temp_file.as_file().sync_all()?;
    temp_file.persist(&rpc_file).map_err(|e| e.error)?;

    // Remove per-profile data directory if it exists
    let profile_dir = app_dir.join("profiles").join(profile_id);
    if profile_dir.is_dir() {
        let _ = fs::remove_dir_all(&profile_dir);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use tempfile::TempDir;

    fn make_test_root(key: &str, path: &str) -> DownloadRoot {
        DownloadRoot {
            key: key.to_string(),
            path: path.to_string(),
            display_name: format!("Test Root {key}"),
            removable: true,
            last_stat_ok: true,
            last_checked: 0,
            disk_id: String::new(),
        }
    }

    fn make_rpc_info(pid: u32, profile_id: &str, roots: Option<Vec<DownloadRoot>>) -> RpcWriteInfo {
        RpcWriteInfo {
            version: "0.1.0".to_string(),
            pid,
            port: 12345,
            token: "test-token".to_string(),
            started: 1000,
            last_used: 1000,
            browser: BrowserInfo {
                name: "Chrome".to_string(),
                binary: "/bin/sh".to_string(),
                extension_id: Some("test-ext-id".to_string()),
            },
            download_roots: roots,
            profile_id: profile_id.to_string(),
            display_name: format!("Profile {profile_id}"),
            created: 1000,
            client_type: None,
            client_version: None,
            launcher: None,
            client_types_used: Vec::new(),
        }
    }

    /// Test: Startup with existing roots preserves them when writing with None
    #[test]
    #[serial]
    fn test_startup_preserves_existing_roots() {
        let temp_dir = TempDir::new().unwrap();
        std::env::set_var("JSTORRENT_CONFIG_DIR", temp_dir.path());

        let app_dir = temp_dir.path().join("jstorrent-native");
        std::fs::create_dir_all(&app_dir).unwrap();

        let profile_id = "test-profile-123";
        let test_root = make_test_root("root-key-1", "/home/user/Downloads");

        // Step 1: Create entry with roots
        let info1 = make_rpc_info(1000, profile_id, Some(vec![test_root.clone()]));
        let roots1 = write_discovery_file(info1).unwrap();
        assert_eq!(roots1.len(), 1);
        assert_eq!(roots1[0].key, "root-key-1");

        // Step 2: Update same profile with None roots — preserves existing
        let info2 = make_rpc_info(2000, profile_id, None);
        let roots2 = write_discovery_file(info2).unwrap();
        assert_eq!(
            roots2.len(),
            1,
            "Roots should be preserved after update with None"
        );
        assert_eq!(roots2[0].key, "root-key-1");

        std::env::remove_var("JSTORRENT_CONFIG_DIR");
    }

    /// Test: Some([]) wipes roots
    #[test]
    #[serial]
    fn test_some_empty_wipes_roots_regression() {
        let temp_dir = TempDir::new().unwrap();
        std::env::set_var("JSTORRENT_CONFIG_DIR", temp_dir.path());

        let app_dir = temp_dir.path().join("jstorrent-native");
        std::fs::create_dir_all(&app_dir).unwrap();

        let profile_id = "test-profile-regression";
        let test_root = make_test_root("root-key-1", "/home/user/Downloads");

        let info1 = make_rpc_info(1000, profile_id, Some(vec![test_root]));
        let roots1 = write_discovery_file(info1).unwrap();
        assert_eq!(roots1.len(), 1);

        // Some([]) wipes roots explicitly
        let info2 = make_rpc_info(2000, profile_id, Some(vec![]));
        let roots2 = write_discovery_file(info2).unwrap();
        assert_eq!(roots2.len(), 0, "Some([]) wipes roots");

        std::env::remove_var("JSTORRENT_CONFIG_DIR");
    }

    /// Test: Removing a root actually removes it
    #[test]
    #[serial]
    fn test_removing_root_actually_removes_it() {
        let temp_dir = TempDir::new().unwrap();
        std::env::set_var("JSTORRENT_CONFIG_DIR", temp_dir.path());

        let app_dir = temp_dir.path().join("jstorrent-native");
        std::fs::create_dir_all(&app_dir).unwrap();

        let profile_id = "test-profile-456";
        let test_root = make_test_root("root-to-remove", "/home/user/Videos");

        let info1 = make_rpc_info(1000, profile_id, Some(vec![test_root]));
        let roots1 = write_discovery_file(info1).unwrap();
        assert_eq!(roots1.len(), 1);

        let info2 = make_rpc_info(1000, profile_id, Some(vec![]));
        let roots2 = write_discovery_file(info2).unwrap();
        assert_eq!(roots2.len(), 0, "Root should be removed");

        let info3 = make_rpc_info(1000, profile_id, None);
        let roots3 = write_discovery_file(info3).unwrap();
        assert_eq!(
            roots3.len(),
            0,
            "Root should still be gone after preserve-mode read"
        );

        std::env::remove_var("JSTORRENT_CONFIG_DIR");
    }

    /// Test: Adding a root works
    #[test]
    #[serial]
    fn test_adding_root() {
        let temp_dir = TempDir::new().unwrap();
        std::env::set_var("JSTORRENT_CONFIG_DIR", temp_dir.path());

        let app_dir = temp_dir.path().join("jstorrent-native");
        std::fs::create_dir_all(&app_dir).unwrap();

        let profile_id = "test-profile-789";

        let info1 = make_rpc_info(1000, profile_id, Some(vec![]));
        let roots1 = write_discovery_file(info1).unwrap();
        assert_eq!(roots1.len(), 0);

        let new_root = make_test_root("new-root", "/home/user/Music");
        let info2 = make_rpc_info(1000, profile_id, Some(vec![new_root.clone()]));
        let roots2 = write_discovery_file(info2).unwrap();
        assert_eq!(roots2.len(), 1);
        assert_eq!(roots2[0].key, "new-root");

        let info3 = make_rpc_info(1000, profile_id, None);
        let roots3 = write_discovery_file(info3).unwrap();
        assert_eq!(roots3.len(), 1, "Added root should persist");

        std::env::remove_var("JSTORRENT_CONFIG_DIR");
    }

    /// Test: Writing with same `profile_id` updates same entry
    #[test]
    #[serial]
    fn test_profile_match_by_id() {
        let temp_dir = TempDir::new().unwrap();
        std::env::set_var("JSTORRENT_CONFIG_DIR", temp_dir.path());

        let app_dir = temp_dir.path().join("jstorrent-native");
        std::fs::create_dir_all(&app_dir).unwrap();

        let profile_id = "match-test";

        let info1 = make_rpc_info(1000, profile_id, Some(vec![]));
        write_discovery_file(info1).unwrap();

        // Rewrite with same profile_id, different PID
        let info2 = make_rpc_info(2000, profile_id, None);
        write_discovery_file(info2).unwrap();

        // Should still be one entry
        let rpc_info = read_discovery_file();
        assert_eq!(rpc_info.profiles.len(), 1);
        assert_eq!(rpc_info.profiles[0].pid, 2000);

        std::env::remove_var("JSTORRENT_CONFIG_DIR");
    }

    /// Test: Writing with different `profile_id` creates new entry
    #[test]
    #[serial]
    fn test_profile_create_new() {
        let temp_dir = TempDir::new().unwrap();
        std::env::set_var("JSTORRENT_CONFIG_DIR", temp_dir.path());

        let app_dir = temp_dir.path().join("jstorrent-native");
        std::fs::create_dir_all(&app_dir).unwrap();

        let info1 = make_rpc_info(1000, "profile-x", Some(vec![]));
        write_discovery_file(info1).unwrap();

        let info2 = make_rpc_info(2000, "profile-y", Some(vec![]));
        write_discovery_file(info2).unwrap();

        let rpc_info = read_discovery_file();
        assert_eq!(rpc_info.profiles.len(), 2);

        std::env::remove_var("JSTORRENT_CONFIG_DIR");
    }

    /// Test: `display_name` and created survive updates
    #[test]
    #[serial]
    fn test_profile_metadata_preserved() {
        let temp_dir = TempDir::new().unwrap();
        std::env::set_var("JSTORRENT_CONFIG_DIR", temp_dir.path());

        let app_dir = temp_dir.path().join("jstorrent-native");
        std::fs::create_dir_all(&app_dir).unwrap();

        let mut info1 = make_rpc_info(1000, "meta-test", Some(vec![]));
        info1.display_name = "My Profile".to_string();
        info1.created = 12345;
        write_discovery_file(info1).unwrap();

        // Update with different PID — display_name and created are on the entry,
        // not overwritten because we only update specific fields
        let info2 = make_rpc_info(2000, "meta-test", None);
        write_discovery_file(info2).unwrap();

        let rpc_info = read_discovery_file();
        assert_eq!(rpc_info.profiles.len(), 1);
        assert_eq!(rpc_info.profiles[0].display_name, "My Profile");
        assert_eq!(rpc_info.profiles[0].created, 12345);

        std::env::remove_var("JSTORRENT_CONFIG_DIR");
    }
}
