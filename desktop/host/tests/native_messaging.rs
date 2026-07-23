//! Integration test for the native messaging protocol between
//! the Tauri backend (or any host) and jstorrent-host (system-bridge).
//!
//! Prerequisites: both binaries must be built:
//!   `cargo build -p jstorrent-host -p jstorrent-io-daemon`
//!
//! Run:
//!   `cargo test -p jstorrent-host --test native_messaging`

use std::io::{Read, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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

/// Wait for child to exit with a timeout. Returns true if exited.
fn wait_with_timeout(child: &mut std::process::Child, timeout: Duration) -> bool {
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

#[test]
#[allow(non_snake_case)]
fn conformance__handshake__daemon_info_is_returned__impl__rust() {
    let host_bin = env!("CARGO_BIN_EXE_jstorrent-host");

    // Check io-daemon exists (sibling binary, different package)
    let host_dir = std::path::Path::new(host_bin).parent().unwrap();
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

    // Use a temp config dir so the test doesn't read real ~/.config/jstorrent-native/
    // (avoids false failures from mutual exclusion with a running Tauri app)
    let config_dir = tempfile::tempdir().expect("failed to create temp config dir");

    let mut child = Command::new(host_bin)
        .env("JSTORRENT_CONFIG_DIR", config_dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn jstorrent-host");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();

    // 1. Send Handshake
    let handshake = serde_json::json!({
        "id": "test-handshake",
        "op": "handshake",
        "extensionId": "tauri-integration-test",
        "installId": "test-install-id",
    });
    write_native_message(&mut stdin, &handshake);

    // 2. Read response
    let response = read_native_message(&mut stdout);

    assert_eq!(response["id"], "test-handshake", "response id must match");
    if response["ok"] != true {
        // Drain stderr for diagnostics before asserting
        drop(stdin);
        let _ = child.kill();
        let mut stderr_buf = Vec::new();
        let _ = stderr.read_to_end(&mut stderr_buf);
        let stderr_str = String::from_utf8_lossy(&stderr_buf);
        let daemon_meta = std::fs::metadata(&daemon_bin).map_or_else(
            |e| format!("metadata error: {e}"),
            |m| {
                #[cfg(unix)]
                let perms = format!("{:o}", m.permissions().mode());
                #[cfg(not(unix))]
                let perms = format!("{:?}", m.permissions());
                format!("size={}, permissions={perms}", m.len())
            },
        );
        panic!(
            "handshake must succeed: {response}\n\
             host_bin: {host_bin}\n\
             daemon_bin: {}\n\
             daemon_bin exists: {}\n\
             daemon_bin metadata: {daemon_meta}\n\
             stderr:\n{stderr_str}",
            daemon_bin.display(),
            daemon_bin.exists(),
        );
    }
    assert_eq!(
        response["type"], "DaemonInfo",
        "response type must be DaemonInfo"
    );

    let payload = &response["payload"];
    let profile_id = payload["profileId"]
        .as_str()
        .expect("profileId must be present");
    assert!(!profile_id.is_empty(), "profileId must not be empty");
    let port = payload["port"].as_u64().expect("port must be a number");
    assert!(port > 0, "port must be > 0");
    let token = payload["token"].as_str().expect("token must be a string");
    assert!(!token.is_empty(), "token must not be empty");
    let version = payload["version"]
        .as_str()
        .expect("version must be a string");
    assert!(!version.is_empty(), "version must not be empty");
    let protocol_version = payload["protocolVersion"]
        .as_u64()
        .expect("protocolVersion must be a number");
    assert_eq!(protocol_version, 1, "protocolVersion must be 1");
    let behavior_version = payload["behaviorVersion"]
        .as_u64()
        .expect("behaviorVersion must be a number");
    assert_eq!(behavior_version, 1, "behaviorVersion must be 1");
    assert!(
        payload["roots"].is_array(),
        "roots must be present as an array"
    );

    eprintln!("Handshake OK: port={port}, version={version}, profileId={profile_id}");

    // 3. Send a second request to validate framing (deleteDownloadRoot with nonexistent key)
    let delete_req = serde_json::json!({
        "id": "test-delete",
        "op": "deleteDownloadRoot",
        "key": "nonexistent-key",
    });
    write_native_message(&mut stdin, &delete_req);

    let delete_response = read_native_message(&mut stdout);
    assert_eq!(
        delete_response["id"], "test-delete",
        "response id must match"
    );
    // Response may be ok or error depending on implementation; we just validate framing works
    eprintln!("Delete response: ok={}", delete_response["ok"]);

    // 4. Close stdin -> system-bridge should exit cleanly (EOF shutdown)
    drop(stdin);

    if !wait_with_timeout(&mut child, Duration::from_secs(10)) {
        child.kill().ok();
        panic!("system-bridge did not exit within 10 seconds after stdin close");
    }

    eprintln!("system-bridge exited cleanly after stdin EOF");
}

#[test]
#[allow(non_snake_case)]
fn conformance__handshake__capabilities_are_reported__impl__rust() {
    let host_bin = env!("CARGO_BIN_EXE_jstorrent-host");
    let config_dir = tempfile::tempdir().expect("failed to create temp config dir");

    let mut child = Command::new(host_bin)
        .env("JSTORRENT_CONFIG_DIR", config_dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn jstorrent-host");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    let handshake = serde_json::json!({
        "id": "test-capabilities",
        "op": "handshake",
        "extensionId": "tauri-integration-test",
    });
    write_native_message(&mut stdin, &handshake);

    let response = read_native_message(&mut stdout);
    assert_eq!(response["ok"], true, "handshake must succeed: {response}");
    assert_eq!(response["type"], "DaemonInfo");

    let capabilities = response["payload"]["capabilities"]
        .as_object()
        .expect("DaemonInfo.capabilities must be present");
    assert_eq!(
        capabilities
            .get("roots_manageable")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        capabilities
            .get("lan_share_urls")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        capabilities
            .get("free_space")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        capabilities
            .get("write_atomic")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );

    drop(stdin);
    if !wait_with_timeout(&mut child, Duration::from_secs(10)) {
        child.kill().ok();
        panic!("system-bridge did not exit within 10 seconds after stdin close");
    }
}

#[test]
#[allow(non_snake_case)]
fn conformance__handshake__contract_versions_are_reported__impl__rust() {
    let host_bin = env!("CARGO_BIN_EXE_jstorrent-host");
    let config_dir = tempfile::tempdir().expect("failed to create temp config dir");

    let mut child = Command::new(host_bin)
        .env("JSTORRENT_CONFIG_DIR", config_dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn jstorrent-host");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    let handshake = serde_json::json!({
        "id": "test-versions",
        "op": "handshake",
        "extensionId": "tauri-integration-test",
    });
    write_native_message(&mut stdin, &handshake);

    let response = read_native_message(&mut stdout);
    assert_eq!(response["ok"], true, "handshake must succeed: {response}");
    assert_eq!(response["type"], "DaemonInfo");
    assert_eq!(response["payload"]["protocolVersion"].as_u64(), Some(1));
    assert_eq!(response["payload"]["behaviorVersion"].as_u64(), Some(1));

    drop(stdin);
    if !wait_with_timeout(&mut child, Duration::from_secs(10)) {
        child.kill().ok();
        panic!("system-bridge did not exit within 10 seconds after stdin close");
    }
}

#[test]
#[allow(non_snake_case)]
fn conformance__handshake__roots_are_included_in_daemon_info__impl__rust() {
    let host_bin = env!("CARGO_BIN_EXE_jstorrent-host");
    let config_dir = tempfile::tempdir().expect("failed to create temp config dir");

    let mut child = Command::new(host_bin)
        .env("JSTORRENT_CONFIG_DIR", config_dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn jstorrent-host");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();

    let handshake = serde_json::json!({
        "id": "test-roots",
        "op": "handshake",
        "extensionId": "tauri-integration-test",
    });
    write_native_message(&mut stdin, &handshake);

    let response = read_native_message(&mut stdout);
    assert_eq!(response["ok"], true, "handshake must succeed: {response}");
    assert_eq!(response["type"], "DaemonInfo");
    assert!(
        response["payload"]["roots"].is_array(),
        "DaemonInfo.roots must be present as an array"
    );

    drop(stdin);
    if !wait_with_timeout(&mut child, Duration::from_secs(10)) {
        child.kill().ok();
        panic!("system-bridge did not exit within 10 seconds after stdin close");
    }
}
