//! E2E integration tests for the profile system.
//!
//! These tests spawn real `jstorrent-host` + `jstorrent-io-daemon` processes
//! and exercise profile scenarios via the native messaging protocol.
//!
//! Prerequisites: both binaries must be built:
//!   `cargo build -p jstorrent-host -p jstorrent-io-daemon`
//!
//! Run:
//!   `cargo test -p jstorrent-host --test profile_scenarios`

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_id() -> String {
    format!(
        "test-req-{}",
        REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

// ---------------------------------------------------------------------------
// HostProcess wrapper
// ---------------------------------------------------------------------------

struct HostProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: ChildStdout,
    #[allow(dead_code)]
    stderr: ChildStderr,
}

impl Drop for HostProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ---------------------------------------------------------------------------
// Native messaging helpers (same protocol as native_messaging.rs)
// ---------------------------------------------------------------------------

fn write_native_message(stdin: &mut impl Write, msg: &serde_json::Value) {
    let json = serde_json::to_vec(msg).unwrap();
    let len = (json.len() as u32).to_le_bytes();
    stdin.write_all(&len).unwrap();
    stdin.write_all(&json).unwrap();
    stdin.flush().unwrap();
}

fn read_native_message(stdout: &mut impl Read) -> serde_json::Value {
    let mut len_buf = [0u8; 4];
    stdout.read_exact(&mut len_buf).unwrap();
    let len = u32::from_le_bytes(len_buf) as usize;
    assert!(len < 10 * 1024 * 1024, "Message too large: {len} bytes");
    let mut buf = vec![0u8; len];
    stdout.read_exact(&mut buf).unwrap();
    serde_json::from_slice(&buf).unwrap()
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait().unwrap().is_some() {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

// ---------------------------------------------------------------------------
// Spawn / shutdown helpers
// ---------------------------------------------------------------------------

fn assert_daemon_binary_exists() {
    let host_bin = env!("CARGO_BIN_EXE_jstorrent-host");
    let host_dir = Path::new(host_bin).parent().unwrap();
    let daemon_name = if cfg!(windows) {
        "jstorrent-io-daemon.exe"
    } else {
        "jstorrent-io-daemon"
    };
    let daemon_bin = host_dir.join(daemon_name);
    assert!(
        daemon_bin.exists(),
        "jstorrent-io-daemon not found at {}. Build it first:\n  cargo build -p jstorrent-io-daemon",
        daemon_bin.display()
    );
}

fn spawn_host(config_dir: &Path) -> HostProcess {
    let host_bin = env!("CARGO_BIN_EXE_jstorrent-host");
    let mut child = Command::new(host_bin)
        .env("JSTORRENT_CONFIG_DIR", config_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn jstorrent-host");

    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    HostProcess {
        child,
        stdin: Some(stdin),
        stdout,
        stderr,
    }
}

fn shutdown_host(mut host: HostProcess) {
    // Close stdin → triggers EOF → host exits
    host.stdin.take();
    if !wait_with_timeout(&mut host.child, Duration::from_secs(10)) {
        host.child.kill().ok();
        panic!("host did not exit within 10 seconds after stdin close");
    }
}

// ---------------------------------------------------------------------------
// Operation helpers
// ---------------------------------------------------------------------------

fn handshake(
    host: &mut HostProcess,
    extension_id: &str,
    profile_id: Option<&str>,
) -> serde_json::Value {
    handshake_with_client_type(host, extension_id, profile_id, None)
}

fn handshake_with_client_type(
    host: &mut HostProcess,
    extension_id: &str,
    profile_id: Option<&str>,
    client_type: Option<&str>,
) -> serde_json::Value {
    let mut msg = serde_json::json!({
        "id": next_id(),
        "op": "handshake",
        "extensionId": extension_id,
    });
    if let Some(pid) = profile_id {
        msg["profileId"] = serde_json::Value::String(pid.to_string());
    }
    if let Some(ct) = client_type {
        msg["clientType"] = serde_json::Value::String(ct.to_string());
    }
    write_native_message(host.stdin.as_mut().unwrap(), &msg);
    read_native_message(&mut host.stdout)
}

fn read_torrent_file(host: &mut HostProcess, path: &str) -> serde_json::Value {
    let msg = serde_json::json!({
        "id": next_id(),
        "op": "readTorrentFile",
        "path": path,
    });
    write_native_message(host.stdin.as_mut().unwrap(), &msg);
    read_native_message(&mut host.stdout)
}

fn register_download_root(host: &mut HostProcess, path: &str) -> serde_json::Value {
    let msg = serde_json::json!({
        "id": next_id(),
        "op": "registerDownloadRoot",
        "path": path,
    });
    write_native_message(host.stdin.as_mut().unwrap(), &msg);
    read_native_message(&mut host.stdout)
}

fn delete_download_root(host: &mut HostProcess, key: &str) -> serde_json::Value {
    let msg = serde_json::json!({
        "id": next_id(),
        "op": "deleteDownloadRoot",
        "key": key,
    });
    write_native_message(host.stdin.as_mut().unwrap(), &msg);
    read_native_message(&mut host.stdout)
}

fn takeover(host: &mut HostProcess, extension_id: &str, profile_id: &str) -> serde_json::Value {
    let msg = serde_json::json!({
        "id": next_id(),
        "op": "takeOver",
        "extensionId": extension_id,
        "profileId": profile_id,
    });
    write_native_message(host.stdin.as_mut().unwrap(), &msg);
    read_native_message(&mut host.stdout)
}

fn kv_set(host: &mut HostProcess, key: &str, value: &str) -> serde_json::Value {
    let msg = serde_json::json!({
        "id": next_id(),
        "op": "kvSet",
        "key": key,
        "value": value,
    });
    write_native_message(host.stdin.as_mut().unwrap(), &msg);
    read_native_message(&mut host.stdout)
}

fn kv_get(host: &mut HostProcess, key: &str) -> serde_json::Value {
    let msg = serde_json::json!({
        "id": next_id(),
        "op": "kvGet",
        "key": key,
    });
    write_native_message(host.stdin.as_mut().unwrap(), &msg);
    read_native_message(&mut host.stdout)
}

// ---------------------------------------------------------------------------
// Assertion / utility helpers
// ---------------------------------------------------------------------------

/// Validate a `DaemonInfo` response and return (`profile_id`, `daemon_port`, `daemon_token`).
fn assert_daemon_info(response: &serde_json::Value) -> (String, u16, String) {
    assert_eq!(response["ok"], true, "response must be ok: {response}");
    assert_eq!(
        response["type"], "DaemonInfo",
        "response type must be DaemonInfo: {response}"
    );

    let payload = &response["payload"];
    let profile_id = payload["profileId"]
        .as_str()
        .expect("profileId must be present")
        .to_string();
    assert!(!profile_id.is_empty(), "profileId must not be empty");

    let port = payload["port"].as_u64().expect("port must be a number") as u16;
    assert!(port > 0, "port must be > 0");

    let token = payload["token"]
        .as_str()
        .expect("token must be a string")
        .to_string();
    assert!(!token.is_empty(), "token must not be empty");

    (profile_id, port, token)
}

/// Hit the daemon's /health endpoint (no auth required).
fn check_daemon_health(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{port}/health");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    match client.get(&url).send() {
        Ok(resp) => resp.status().is_success(),
        Err(_) => false,
    }
}

/// Read rpc-info.json from the test config directory.
fn read_rpc_info(config_dir: &Path) -> serde_json::Value {
    let rpc_file = config_dir.join("jstorrent-native").join("rpc-info.json");
    let contents = std::fs::read_to_string(&rpc_file)
        .unwrap_or_else(|e| panic!("Failed to read {}: {e}", rpc_file.display()));
    serde_json::from_str(&contents).unwrap()
}

// ===========================================================================
// Tests
// ===========================================================================

/// `profile_id`: None → creates a new profile with a valid UUID.
#[test]
fn test_fresh_profile_creation() {
    assert_daemon_binary_exists();
    let config_dir = tempfile::tempdir().unwrap();

    let mut host = spawn_host(config_dir.path());
    let response = handshake(&mut host, "ext-fresh-test", None);
    let (profile_id, daemon_port, _) = assert_daemon_info(&response);

    // Profile ID should be UUID format (36 chars with dashes)
    assert_eq!(profile_id.len(), 36, "profileId should be UUID format");
    assert!(profile_id.contains('-'), "profileId should be a UUID");

    // Daemon should be running
    assert!(
        check_daemon_health(daemon_port),
        "daemon should respond to health check"
    );

    // Verify rpc-info.json has the profile
    let rpc_info = read_rpc_info(config_dir.path());
    let profiles = rpc_info["profiles"]
        .as_array()
        .expect("profiles should be array");
    let profile = profiles
        .iter()
        .find(|p| p["profile_id"].as_str() == Some(profile_id.as_str()))
        .expect("should find profile in rpc-info.json");
    assert!(profile["pid"].as_u64().unwrap() > 0);

    shutdown_host(host);
}

/// Passing an explicit `profile_id` from a previous session → reuses the same profile.
#[test]
fn test_profile_reuse_by_profile_id() {
    assert_daemon_binary_exists();
    let config_dir = tempfile::tempdir().unwrap();

    // Host A: create profile
    let mut host_a = spawn_host(config_dir.path());
    let response_a = handshake(&mut host_a, "ext-reuse-test", None);
    let (profile_id_a, _, _) = assert_daemon_info(&response_a);
    shutdown_host(host_a);

    std::thread::sleep(Duration::from_millis(500));

    // Host B: pass A's profile_id explicitly → should reuse it
    let mut host_b = spawn_host(config_dir.path());
    let response_b = handshake(&mut host_b, "ext-reuse-test", Some(&profile_id_a));
    let (profile_id_b, _, _) = assert_daemon_info(&response_b);

    assert_eq!(
        profile_id_a, profile_id_b,
        "explicit profile_id should reuse the same profile"
    );

    shutdown_host(host_b);
}

/// Two hosts with no `profile_id` → each gets a separate new profile,
/// even with the same `extension_id`.
#[test]
fn test_no_profile_id_always_creates_new() {
    assert_daemon_binary_exists();
    let config_dir = tempfile::tempdir().unwrap();

    let mut host_a = spawn_host(config_dir.path());
    let response_a = handshake(&mut host_a, "ext-same", None);
    let (profile_id_a, _, _) = assert_daemon_info(&response_a);
    shutdown_host(host_a);

    std::thread::sleep(Duration::from_millis(500));

    let mut host_b = spawn_host(config_dir.path());
    let response_b = handshake(&mut host_b, "ext-same", None);
    let (profile_id_b, _, _) = assert_daemon_info(&response_b);

    assert_ne!(
        profile_id_a, profile_id_b,
        "two handshakes with profile_id: None should create different profiles"
    );

    shutdown_host(host_b);
}

/// Host A active with profile. Host B sends same `profile_id` → `ProfileInUse`.
#[test]
#[allow(non_snake_case)]
fn conformance__handshake__profile_in_use_is_reported__impl__rust() {
    assert_daemon_binary_exists();
    let config_dir = tempfile::tempdir().unwrap();

    // Host A: create and hold profile
    let mut host_a = spawn_host(config_dir.path());
    let response_a = handshake(&mut host_a, "ext-in-use-a", None);
    let (profile_id_a, _, _) = assert_daemon_info(&response_a);

    // Verify host A's RPC health endpoint works
    let rpc_info = read_rpc_info(config_dir.path());
    let host_a_entry = rpc_info["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["profile_id"].as_str().unwrap() == profile_id_a)
        .expect("should find host A's profile entry");
    let host_a_rpc_port = host_a_entry["port"].as_u64().unwrap() as u16;
    let host_a_rpc_token = host_a_entry["token"].as_str().unwrap();

    let health_url = format!("http://127.0.0.1:{host_a_rpc_port}/health?token={host_a_rpc_token}");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    let health_resp = client
        .get(&health_url)
        .send()
        .expect("health check should succeed");
    assert!(
        health_resp.status().is_success(),
        "host A RPC health check should succeed"
    );

    // Host B: explicitly request A's profile → ProfileInUse
    let mut host_b = spawn_host(config_dir.path());
    let response_b = handshake(&mut host_b, "ext-in-use-b", Some(&profile_id_a));

    assert_eq!(response_b["ok"], false, "should be error: {response_b}");
    assert_eq!(
        response_b["error"].as_str().unwrap(),
        "profile_in_use",
        "error should be profile_in_use: {response_b}"
    );
    assert_eq!(
        response_b["type"], "ProfileInUse",
        "type should be ProfileInUse: {response_b}"
    );

    let payload_b = &response_b["payload"];
    assert_eq!(
        payload_b["profileId"].as_str().unwrap(),
        profile_id_a,
        "ProfileInUse should reference A's profileId"
    );
    assert!(
        payload_b["pid"].as_u64().unwrap() > 0,
        "ProfileInUse should have incumbent PID"
    );

    shutdown_host(host_b);
    shutdown_host(host_a);
}

/// Host A crashes (stale entry). Host B passes A's `profile_id` → takes over.
#[test]
fn test_stale_process_takeover() {
    assert_daemon_binary_exists();
    let config_dir = tempfile::tempdir().unwrap();

    // Host A: create profile
    let mut host_a = spawn_host(config_dir.path());
    let response_a = handshake(&mut host_a, "ext-stale-test", None);
    let (profile_id_a, _, _) = assert_daemon_info(&response_a);

    // Kill host A (simulates crash — leaves stale entry in rpc-info.json)
    host_a.child.kill().ok();
    let _ = host_a.child.wait();

    // Wait for daemon to notice parent death
    std::thread::sleep(Duration::from_secs(2));

    // Host B: pass A's profile_id → stale PID is dead → takes over
    let mut host_b = spawn_host(config_dir.path());
    let response_b = handshake(&mut host_b, "ext-stale-test", Some(&profile_id_a));
    let (profile_id_b, daemon_port_b, _) = assert_daemon_info(&response_b);

    assert_eq!(
        profile_id_a, profile_id_b,
        "should reuse same profile after stale process takeover"
    );
    assert!(
        check_daemon_health(daemon_port_b),
        "new daemon should be healthy"
    );

    shutdown_host(host_b);
}

/// Host B sends `TakeOver` with A's `profile_id` → kills A, takes profile.
#[test]
#[allow(non_snake_case)]
fn conformance__profiles__takeover_reuses_profile__impl__rust() {
    assert_daemon_binary_exists();
    let config_dir = tempfile::tempdir().unwrap();

    // Host A: create and hold profile
    let mut host_a = spawn_host(config_dir.path());
    let response_a = handshake(&mut host_a, "ext-takeover-a", None);
    let (profile_id_a, _, _) = assert_daemon_info(&response_a);

    // Host B: TakeOver with A's profile_id → kills A, then handshakes
    let mut host_b = spawn_host(config_dir.path());
    let response_b = takeover(&mut host_b, "ext-takeover-b", &profile_id_a);
    let (profile_id_b, daemon_port_b, _) = assert_daemon_info(&response_b);

    assert_eq!(
        profile_id_a, profile_id_b,
        "should get same profile after takeover"
    );

    // Verify host A was killed
    std::thread::sleep(Duration::from_millis(200));
    let exit_status = host_a.child.try_wait().unwrap();
    assert!(
        exit_status.is_some(),
        "host A should have been killed by takeover"
    );

    assert!(
        check_daemon_health(daemon_port_b),
        "host B's daemon should be healthy"
    );

    shutdown_host(host_b);
}

#[test]
#[allow(non_snake_case)]
fn conformance__roots__register_download_root_returns_root_added__impl__rust() {
    assert_daemon_binary_exists();
    let config_dir = tempfile::tempdir().unwrap();

    let mut host = spawn_host(config_dir.path());
    let response = handshake(&mut host, "ext-root-add", None);
    let (profile_id, _, _) = assert_daemon_info(&response);

    let root_dir = tempfile::tempdir().unwrap();
    let canonical_root_path = root_dir.path().canonicalize().unwrap();
    let add_response = register_download_root(&mut host, root_dir.path().to_str().unwrap());

    assert_eq!(
        add_response["ok"], true,
        "registerDownloadRoot should succeed: {add_response}"
    );
    assert_eq!(add_response["type"], "RootAdded");

    let root = &add_response["payload"]["root"];
    assert_eq!(
        root["path"].as_str().unwrap(),
        canonical_root_path.to_str().unwrap()
    );
    let root_key = root["key"]
        .as_str()
        .expect("RootAdded must include root key");
    assert!(!root_key.is_empty(), "root key must not be empty");

    let rpc_info = read_rpc_info(config_dir.path());
    let profile = rpc_info["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["profile_id"].as_str() == Some(&profile_id))
        .expect("should find profile");
    let roots = profile["download_roots"].as_array().unwrap();
    assert!(roots
        .iter()
        .any(|entry| entry["key"].as_str() == Some(root_key)));

    shutdown_host(host);
}

#[test]
#[allow(non_snake_case)]
fn conformance__roots__delete_download_root_returns_root_removed__impl__rust() {
    assert_daemon_binary_exists();
    let config_dir = tempfile::tempdir().unwrap();

    let mut host = spawn_host(config_dir.path());
    let response = handshake(&mut host, "ext-root-remove", None);
    let (profile_id, _, _) = assert_daemon_info(&response);

    let root_dir = tempfile::tempdir().unwrap();
    let add_response = register_download_root(&mut host, root_dir.path().to_str().unwrap());
    let root_key = add_response["payload"]["root"]["key"]
        .as_str()
        .expect("RootAdded must include root key")
        .to_string();

    let remove_response = delete_download_root(&mut host, &root_key);
    assert_eq!(
        remove_response["ok"], true,
        "deleteDownloadRoot should succeed: {remove_response}"
    );
    assert_eq!(remove_response["type"], "RootRemoved");
    assert_eq!(
        remove_response["payload"]["key"].as_str().unwrap(),
        root_key
    );

    let rpc_info = read_rpc_info(config_dir.path());
    let profile = rpc_info["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["profile_id"].as_str() == Some(&profile_id))
        .expect("should find profile");
    let roots = profile["download_roots"].as_array().unwrap();
    assert!(!roots
        .iter()
        .any(|entry| entry["key"].as_str() == Some(root_key.as_str())));

    shutdown_host(host);
}

/// Two hosts both send `profile_id`: None → two different profiles, two daemons.
#[test]
fn test_multiple_independent_profiles() {
    assert_daemon_binary_exists();
    let config_dir = tempfile::tempdir().unwrap();

    let mut host_a = spawn_host(config_dir.path());
    let response_a = handshake(&mut host_a, "ext-multi-a", None);
    let (profile_id_a, daemon_port_a, _) = assert_daemon_info(&response_a);

    let mut host_b = spawn_host(config_dir.path());
    let response_b = handshake(&mut host_b, "ext-multi-b", None);
    let (profile_id_b, daemon_port_b, _) = assert_daemon_info(&response_b);

    assert_ne!(profile_id_a, profile_id_b, "should get different profiles");
    assert_ne!(
        daemon_port_a, daemon_port_b,
        "daemons should be on different ports"
    );

    assert!(
        check_daemon_health(daemon_port_a),
        "daemon A should be healthy"
    );
    assert!(
        check_daemon_health(daemon_port_b),
        "daemon B should be healthy"
    );

    // Verify rpc-info.json has both
    let rpc_info = read_rpc_info(config_dir.path());
    let profiles = rpc_info["profiles"].as_array().unwrap();
    assert!(profiles
        .iter()
        .any(|p| p["profile_id"].as_str() == Some(profile_id_a.as_str())));
    assert!(profiles
        .iter()
        .any(|p| p["profile_id"].as_str() == Some(profile_id_b.as_str())));

    shutdown_host(host_a);
    shutdown_host(host_b);
}

/// Explicit bad `profile_id` → auto-creates a new profile (self-recovery).
#[test]
#[allow(non_snake_case)]
fn conformance__handshake__invalid_profile_id_creates_new_profile__impl__rust() {
    assert_daemon_binary_exists();
    let config_dir = tempfile::tempdir().unwrap();

    let mut host = spawn_host(config_dir.path());

    let response = handshake(
        &mut host,
        "ext-invalid-test",
        Some("nonexistent-uuid-12345"),
    );
    assert_eq!(
        response["ok"], true,
        "invalid profileId should auto-create new profile: {response}"
    );
    // The returned profileId should NOT be the stale one
    let new_profile_id = response["payload"]["profileId"].as_str().unwrap();
    assert_ne!(
        new_profile_id, "nonexistent-uuid-12345",
        "should have created a new profile, not reused the invalid one"
    );

    shutdown_host(host);
}

/// KV data is isolated per profile and persists across host restarts.
#[test]
#[allow(non_snake_case)]
fn conformance__profiles__kv_isolated_per_profile__impl__rust() {
    assert_daemon_binary_exists();
    let config_dir = tempfile::tempdir().unwrap();

    // Host A: create profile, set KV value
    let mut host_a = spawn_host(config_dir.path());
    let response_a = handshake(&mut host_a, "ext-kv-a", None);
    let (profile_id_a, _, _) = assert_daemon_info(&response_a);

    let set_resp = kv_set(&mut host_a, "setting", "hello");
    assert_eq!(set_resp["ok"], true, "KvSet should succeed: {set_resp}");

    let get_resp = kv_get(&mut host_a, "setting");
    assert_eq!(get_resp["ok"], true, "KvGet should succeed: {get_resp}");
    assert_eq!(get_resp["payload"]["value"].as_str().unwrap(), "hello");

    shutdown_host(host_a);
    std::thread::sleep(Duration::from_millis(500));

    // Host B: different profile (None) → should NOT see A's KV data
    let mut host_b = spawn_host(config_dir.path());
    let response_b = handshake(&mut host_b, "ext-kv-b", None);
    let (profile_id_b, _, _) = assert_daemon_info(&response_b);
    assert_ne!(profile_id_a, profile_id_b, "should be different profiles");

    let get_resp_b = kv_get(&mut host_b, "setting");
    assert_eq!(get_resp_b["ok"], true, "KvGet should succeed on host B");
    assert!(
        get_resp_b["payload"]["value"].is_null(),
        "different profile should not see A's KV data: {get_resp_b}"
    );

    shutdown_host(host_b);
    std::thread::sleep(Duration::from_millis(500));

    // Host C: reconnect to A's profile by profile_id → should see persisted KV data
    let mut host_c = spawn_host(config_dir.path());
    let response_c = handshake(&mut host_c, "ext-kv-c", Some(&profile_id_a));
    let (profile_id_c, _, _) = assert_daemon_info(&response_c);
    assert_eq!(profile_id_a, profile_id_c, "should reuse A's profile");

    let get_resp_c = kv_get(&mut host_c, "setting");
    assert_eq!(get_resp_c["ok"], true, "KvGet should succeed on host C");
    assert_eq!(
        get_resp_c["payload"]["value"].as_str().unwrap(),
        "hello",
        "should see A's persisted KV data"
    );

    shutdown_host(host_c);
}

// ===========================================================================
// Magnet/Torrent Routing — Phase 1 Tests
// ===========================================================================

/// Fresh config dir → handshake → rpc-info.json has non-empty `add_token`.
/// Restart host → same `add_token` persisted.
#[test]
fn test_add_token_generated() {
    assert_daemon_binary_exists();
    let config_dir = tempfile::tempdir().unwrap();

    // Host A: create profile → should generate add_token
    let mut host_a = spawn_host(config_dir.path());
    let response_a = handshake(&mut host_a, "ext-token-test", None);
    assert_daemon_info(&response_a);

    let rpc_info = read_rpc_info(config_dir.path());
    let add_token = rpc_info["add_token"]
        .as_str()
        .expect("add_token should be present");
    assert!(
        !add_token.is_empty(),
        "add_token should be a non-empty string"
    );

    shutdown_host(host_a);
    std::thread::sleep(Duration::from_millis(500));

    // Host B: reconnect → same add_token persisted
    let mut host_b = spawn_host(config_dir.path());
    let response_b = handshake(&mut host_b, "ext-token-test-2", None);
    assert_daemon_info(&response_b);

    let rpc_info2 = read_rpc_info(config_dir.path());
    let add_token2 = rpc_info2["add_token"]
        .as_str()
        .expect("add_token should still be present");
    assert_eq!(
        add_token, add_token2,
        "add_token should be stable across restarts"
    );

    shutdown_host(host_b);
}

/// Two hosts with different profiles in same config dir → both see same `add_token`.
#[test]
fn test_add_token_stable_across_profiles() {
    assert_daemon_binary_exists();
    let config_dir = tempfile::tempdir().unwrap();

    let mut host_a = spawn_host(config_dir.path());
    let response_a = handshake(&mut host_a, "ext-token-a", None);
    assert_daemon_info(&response_a);

    let rpc_info1 = read_rpc_info(config_dir.path());
    let token1 = rpc_info1["add_token"]
        .as_str()
        .expect("add_token from host A")
        .to_string();

    shutdown_host(host_a);
    std::thread::sleep(Duration::from_millis(500));

    let mut host_b = spawn_host(config_dir.path());
    let response_b = handshake(&mut host_b, "ext-token-b", None);
    assert_daemon_info(&response_b);

    let rpc_info2 = read_rpc_info(config_dir.path());
    let token2 = rpc_info2["add_token"]
        .as_str()
        .expect("add_token from host B");

    assert_eq!(
        token1, token2,
        "add_token should be the same across different profiles"
    );

    shutdown_host(host_b);
}

/// Handshake with `client_type`: "extension" → profile records the extension.
/// Reconnect with `client_type`: "tauri" → both types, without duplicates.
#[test]
fn test_client_types_used_accumulated() {
    assert_daemon_binary_exists();
    let config_dir = tempfile::tempdir().unwrap();

    // Host A: handshake with client_type = "extension"
    let mut host_a = spawn_host(config_dir.path());
    let response_a =
        handshake_with_client_type(&mut host_a, "ext-ctu-test", None, Some("extension"));
    let (profile_id, _, _) = assert_daemon_info(&response_a);

    let rpc_info = read_rpc_info(config_dir.path());
    let profile = rpc_info["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["profile_id"].as_str() == Some(&profile_id))
        .expect("should find profile");
    let ctu: Vec<&str> = profile["client_types_used"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        ctu,
        vec!["extension"],
        "should have extension after first handshake"
    );

    shutdown_host(host_a);
    std::thread::sleep(Duration::from_millis(500));

    // Host B: reconnect same profile with client_type = "tauri"
    let mut host_b = spawn_host(config_dir.path());
    let response_b = handshake_with_client_type(
        &mut host_b,
        "ext-ctu-test",
        Some(&profile_id),
        Some("tauri"),
    );
    assert_daemon_info(&response_b);

    let rpc_info2 = read_rpc_info(config_dir.path());
    let profile2 = rpc_info2["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["profile_id"].as_str() == Some(&profile_id))
        .expect("should find profile");
    let ctu2: Vec<&str> = profile2["client_types_used"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        ctu2,
        vec!["extension", "tauri"],
        "should accumulate both client types"
    );

    shutdown_host(host_b);
    std::thread::sleep(Duration::from_millis(500));

    // Host C: reconnect same profile with client_type = "extension" again → no duplicate
    let mut host_c = spawn_host(config_dir.path());
    let response_c = handshake_with_client_type(
        &mut host_c,
        "ext-ctu-test",
        Some(&profile_id),
        Some("extension"),
    );
    assert_daemon_info(&response_c);

    let rpc_info3 = read_rpc_info(config_dir.path());
    let profile3 = rpc_info3["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["profile_id"].as_str() == Some(&profile_id))
        .expect("should find profile");
    let ctu3: Vec<&str> = profile3["client_types_used"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        ctu3,
        vec!["extension", "tauri"],
        "should not duplicate 'extension'"
    );

    shutdown_host(host_c);
}

/// Manually set `desktop_ever_used`: true in rpc-info.json for a profile →
/// handshake with that `profile_id` → field still true after handshake updates.
#[test]
fn test_desktop_ever_used_preserved() {
    assert_daemon_binary_exists();
    let config_dir = tempfile::tempdir().unwrap();

    // Host A: create profile
    let mut host_a = spawn_host(config_dir.path());
    let response_a = handshake(&mut host_a, "ext-deu-test", None);
    let (profile_id, _, _) = assert_daemon_info(&response_a);

    // Verify desktop_ever_used starts as false
    let rpc_info = read_rpc_info(config_dir.path());
    let profile = rpc_info["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["profile_id"].as_str() == Some(&profile_id))
        .expect("should find profile");
    assert!(
        !profile["desktop_ever_used"].as_bool().unwrap_or(false),
        "desktop_ever_used should start as false"
    );

    shutdown_host(host_a);

    // Manually set desktop_ever_used: true in rpc-info.json
    let rpc_file = config_dir
        .path()
        .join("jstorrent-native")
        .join("rpc-info.json");
    let mut rpc_data: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&rpc_file).unwrap()).unwrap();
    for p in rpc_data["profiles"].as_array_mut().unwrap() {
        if p["profile_id"].as_str() == Some(&profile_id) {
            p["desktop_ever_used"] = serde_json::Value::Bool(true);
        }
    }
    std::fs::write(&rpc_file, serde_json::to_string(&rpc_data).unwrap()).unwrap();

    std::thread::sleep(Duration::from_millis(500));

    // Host B: reconnect to same profile → desktop_ever_used should still be true
    let mut host_b = spawn_host(config_dir.path());
    let response_b = handshake(&mut host_b, "ext-deu-test", Some(&profile_id));
    assert_daemon_info(&response_b);

    let rpc_info2 = read_rpc_info(config_dir.path());
    let profile2 = rpc_info2["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["profile_id"].as_str() == Some(&profile_id))
        .expect("should find profile after reconnect");
    assert!(
        profile2["desktop_ever_used"].as_bool().unwrap_or(false),
        "desktop_ever_used should be preserved across handshakes"
    );

    shutdown_host(host_b);
}

/// Write a test .torrent file → send `ReadTorrentFile` → get back `TorrentFileContents`.
/// Also test rejection for non-.torrent paths.
#[test]
#[allow(non_snake_case)]
fn conformance__torrent__read_torrent_file_returns_contents__impl__rust() {
    assert_daemon_binary_exists();
    let config_dir = tempfile::tempdir().unwrap();

    let mut host = spawn_host(config_dir.path());
    let response = handshake(&mut host, "ext-rtf-test", None);
    assert_daemon_info(&response);

    // Create a test .torrent file
    let torrent_dir = tempfile::tempdir().unwrap();
    let torrent_path = torrent_dir.path().join("test.torrent");
    let torrent_contents = b"d8:announce35:http://tracker.example.com/announcee";
    std::fs::write(&torrent_path, torrent_contents).unwrap();

    // Read it via the protocol
    let resp = read_torrent_file(&mut host, torrent_path.to_str().unwrap());
    assert_eq!(resp["ok"], true, "ReadTorrentFile should succeed: {resp}");
    assert_eq!(resp["type"], "TorrentFileContents");

    let payload = &resp["payload"];
    assert_eq!(payload["name"].as_str().unwrap(), "test.torrent");

    // Verify base64 roundtrips
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(payload["contentsBase64"].as_str().unwrap())
        .expect("should be valid base64");
    assert_eq!(decoded, torrent_contents);

    // Test rejection of non-.torrent path
    let txt_path = torrent_dir.path().join("readme.txt");
    std::fs::write(&txt_path, "hello").unwrap();
    let resp2 = read_torrent_file(&mut host, txt_path.to_str().unwrap());
    assert_eq!(
        resp2["ok"], false,
        "non-.torrent path should be rejected: {resp2}"
    );
    let error = resp2["error"].as_str().unwrap();
    assert!(
        error.contains(".torrent"),
        "error should mention .torrent: {error}"
    );

    shutdown_host(host);
}

/// Send `ReadTorrentFile` for a nonexistent path → error response.
#[test]
fn test_read_torrent_file_not_found() {
    assert_daemon_binary_exists();
    let config_dir = tempfile::tempdir().unwrap();

    let mut host = spawn_host(config_dir.path());
    let response = handshake(&mut host, "ext-rtf-404", None);
    assert_daemon_info(&response);

    let resp = read_torrent_file(&mut host, "/tmp/nonexistent-12345.torrent");
    assert_eq!(resp["ok"], false, "nonexistent file should fail: {resp}");
    let error = resp["error"].as_str().unwrap();
    assert!(
        error.contains("Failed to read") || error.contains("No such file"),
        "error should mention read failure: {error}"
    );

    shutdown_host(host);
}
