use crate::AppState;
use axum::{
    extract::{DefaultBodyLimit, Path, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::io::{ErrorKind, SeekFrom};
use std::path::{Component, Path as StdPath, PathBuf};
use std::sync::Arc;
use tokio::fs::{self, File};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

// 64MB limit for piece writes (must match MAX_PIECE_SIZE in engine)
pub const MAX_BODY_SIZE: usize = 64 * 1024 * 1024;

#[allow(deprecated)]
pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        // New header-based endpoints (preferred) - use base64-encoded path in headers
        .route("/write/:root_key", post(write_file_v2))
        .route("/read/:root_key", get(read_file_v2))
        // DEPRECATED: Legacy path-based endpoints - path in URL breaks on # and ? characters
        // These are no longer used by the TypeScript engine as of 2024-12
        .route(
            "/files/*path",
            get(read_file_deprecated).post(write_file_deprecated),
        )
        .route("/files/ensure_dir", post(ensure_dir))
        .route("/ops/stat", get(stat_file))
        .route("/ops/exists", get(exists_file))
        .route("/ops/list", get(list_dir))
        .route("/ops/delete", post(delete_file))
        .route("/ops/batch_delete", post(batch_delete))
        .route("/ops/truncate", post(truncate_file))
        .route("/ops/list_tree", get(list_tree_dir))
        .route("/ops/verify_chunks", post(verify_chunks))
        .route("/ops/free_space", get(free_space))
        .route("/ops/write_atomic/:root_key", post(write_atomic))
        .layer(DefaultBodyLimit::max(MAX_BODY_SIZE))
}

// ============================================================================
// DEPRECATED: Legacy path-based endpoints
// These use the file path in the URL which breaks on # and ? characters.
// Use /read/:root_key and /write/:root_key with X-Path-Base64 header instead.
// ============================================================================

#[derive(Deserialize)]
struct ReadParams {
    offset: Option<u64>,
    length: Option<u64>,
    root_key: String,
}

#[deprecated(
    since = "0.1.0",
    note = "Use read_file_v2 with X-Path-Base64 header instead"
)]
async fn read_file_deprecated(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
    axum::extract::Query(params): axum::extract::Query<ReadParams>,
) -> Result<Vec<u8>, (StatusCode, String)> {
    tracing::warn!("DEPRECATED: /files/* endpoint called for read. Use /read/:root_key with X-Path-Base64 header instead.");

    let full_path = validate_path(&state, &params.root_key, &path)?;

    let mut file = File::open(&full_path)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    if let Some(offset) = params.offset {
        file.seek(SeekFrom::Start(offset))
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    let mut buffer = Vec::new();
    if let Some(len) = params.length {
        buffer.resize(len as usize, 0);
        file.read_exact(&mut buffer)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    } else {
        file.read_to_end(&mut buffer)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    Ok(buffer)
}

#[derive(Deserialize)]
struct WriteParams {
    offset: Option<u64>,
    root_key: String,
}

#[deprecated(
    since = "0.1.0",
    note = "Use write_file_v2 with X-Path-Base64 header instead"
)]
async fn write_file_deprecated(
    State(state): State<Arc<AppState>>,
    Path(path): Path<String>,
    axum::extract::Query(params): axum::extract::Query<WriteParams>,
    body: axum::body::Bytes,
) -> Result<(), (StatusCode, String)> {
    tracing::warn!("DEPRECATED: /files/* endpoint called for write. Use /write/:root_key with X-Path-Base64 header instead.");

    let full_path = validate_path(&state, &params.root_key, &path)?;

    // Ensure parent directory exists
    if let Some(parent) = full_path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&full_path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(offset) = params.offset {
        file.seek(SeekFrom::Start(offset))
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    file.write_all(&body)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(())
}

// ============================================================================
// Current API: Header-based endpoints
// ============================================================================

/// Helper to extract path from X-Path-Base64 header
fn extract_path_from_header(headers: &HeaderMap) -> Result<String, (StatusCode, String)> {
    let path_b64 = headers
        .get("X-Path-Base64")
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Missing X-Path-Base64 header".into(),
        ))?
        .to_str()
        .map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid X-Path-Base64 header".into(),
            )
        })?;

    let path_bytes = BASE64.decode(path_b64).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid base64 in X-Path-Base64".into(),
        )
    })?;

    String::from_utf8(path_bytes)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid UTF-8 in path".into()))
}

/// Helper to extract optional u64 from header
fn extract_u64_header(
    headers: &HeaderMap,
    name: &str,
) -> Result<Option<u64>, (StatusCode, String)> {
    match headers.get(name) {
        Some(value) => {
            let s = value
                .to_str()
                .map_err(|_| (StatusCode::BAD_REQUEST, format!("Invalid {name} header")))?;
            let n = s
                .parse()
                .map_err(|_| (StatusCode::BAD_REQUEST, format!("Invalid {name} value")))?;
            Ok(Some(n))
        }
        None => Ok(None),
    }
}

/// New write endpoint with base64 path in header and optional hash verification.
/// POST /`write/{root_key`}
/// Headers:
///   X-Path-Base64: <base64 encoded path>
///   X-Offset: <optional offset>
///   X-Expected-SHA1: <optional hex SHA1 hash for verification>
/// Body: raw bytes
/// Returns: 200 OK, 409 Conflict (hash mismatch), 507 Insufficient (disk full)
async fn write_file_v2(
    State(state): State<Arc<AppState>>,
    Path(root_key): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<(), (StatusCode, String)> {
    let path = extract_path_from_header(&headers)?;
    let offset = extract_u64_header(&headers, "X-Offset")?.unwrap_or(0);

    let full_path = validate_path(&state, &root_key, &path)?;

    // Hash verification FIRST (before any file operations)
    if let Some(expected_hex) = headers.get("X-Expected-SHA1") {
        let expected_hex = expected_hex.to_str().map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid X-Expected-SHA1 header".into(),
            )
        })?;

        let mut hasher = Sha1::new();
        hasher.update(&body);
        let actual = hex::encode(hasher.finalize());

        if actual != expected_hex {
            return Err((
                StatusCode::CONFLICT,
                format!("Hash mismatch: expected {expected_hex}, got {actual}"),
            ));
        }
    }

    // Ensure parent directory exists
    if let Some(parent) = full_path.parent() {
        fs::create_dir_all(parent).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::StorageFull {
                (StatusCode::INSUFFICIENT_STORAGE, e.to_string())
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
            }
        })?;
    }

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&full_path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if offset > 0 {
        file.seek(SeekFrom::Start(offset))
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    file.write_all(&body).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::StorageFull {
            (StatusCode::INSUFFICIENT_STORAGE, e.to_string())
        } else {
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    })?;

    Ok(())
}

/// New read endpoint with base64 path in header.
/// GET /`read/{root_key`}
/// Headers:
///   X-Path-Base64: <base64 encoded path>
///   X-Offset: <optional offset>
///   X-Length: <optional length>
/// Returns: raw bytes
async fn read_file_v2(
    State(state): State<Arc<AppState>>,
    Path(root_key): Path<String>,
    headers: HeaderMap,
) -> Result<Vec<u8>, (StatusCode, String)> {
    let path = extract_path_from_header(&headers)?;
    let offset = extract_u64_header(&headers, "X-Offset")?;
    let length = extract_u64_header(&headers, "X-Length")?;

    let full_path = validate_path(&state, &root_key, &path)?;

    let mut file = File::open(&full_path)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    if let Some(off) = offset {
        file.seek(SeekFrom::Start(off))
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    let mut buffer = Vec::new();
    if let Some(len) = length {
        buffer.resize(len as usize, 0);
        file.read_exact(&mut buffer).await.map_err(|e| {
            let status = if e.kind() == ErrorKind::UnexpectedEof {
                StatusCode::RANGE_NOT_SATISFIABLE
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, e.to_string())
        })?;
    } else {
        file.read_to_end(&mut buffer)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    Ok(buffer)
}

#[derive(Deserialize)]
struct EnsureDirParams {
    path: String,
    root_key: String,
}

async fn ensure_dir(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<EnsureDirParams>,
) -> Result<(), (StatusCode, String)> {
    let full_path = validate_path(&state, &payload.root_key, &payload.path)?;

    fs::create_dir_all(full_path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(())
}

#[derive(Deserialize)]
struct StatParams {
    path: String,
    root_key: String,
}

#[derive(Serialize)]
struct FileStat {
    size: u64,
    mtime: u64, // milliseconds since epoch
    is_directory: bool,
    is_file: bool,
}

async fn stat_file(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<StatParams>,
) -> Result<Json<FileStat>, (StatusCode, String)> {
    let full_path = validate_path(&state, &params.root_key, &params.path)?;

    let metadata = fs::metadata(&full_path).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            (StatusCode::NOT_FOUND, e.to_string())
        } else {
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    })?;

    let mtime = metadata
        .modified()
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    Ok(Json(FileStat {
        size: metadata.len(),
        mtime,
        is_directory: metadata.is_dir(),
        is_file: metadata.is_file(),
    }))
}

#[derive(Deserialize)]
struct ExistsParams {
    path: String,
    root_key: String,
}

#[derive(Serialize)]
struct ExistsResult {
    exists: bool,
}

async fn exists_file(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<ExistsParams>,
) -> Result<Json<ExistsResult>, (StatusCode, String)> {
    let full_path = validate_path(&state, &params.root_key, &params.path)?;

    let exists = fs::metadata(&full_path).await.is_ok();

    Ok(Json(ExistsResult { exists }))
}

#[derive(Deserialize)]
struct ListParams {
    path: String,
    root_key: String,
}

async fn list_dir(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<ListParams>,
) -> Result<Json<Vec<String>>, (StatusCode, String)> {
    let full_path = validate_path(&state, &params.root_key, &params.path)?;

    let mut entries = fs::read_dir(&full_path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut filenames = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        if let Ok(name) = entry.file_name().into_string() {
            filenames.push(name);
        }
    }

    Ok(Json(filenames))
}

#[derive(Deserialize)]
struct ListTreeParams {
    path: String,
    root_key: String,
}

#[derive(Serialize)]
struct FileTreeEntry {
    path: String,
    size: u64,
}

async fn list_tree_dir(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<ListTreeParams>,
) -> Result<Json<Vec<FileTreeEntry>>, (StatusCode, String)> {
    let base = validate_path(&state, &params.root_key, &params.path)?;

    let mut entries = Vec::new();
    let mut stack = Vec::new();

    if base.is_dir() {
        stack.push(base.clone());
    }

    while let Some(dir) = stack.pop() {
        let mut read_dir = fs::read_dir(&dir)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let path = entry.path();
            let Ok(metadata) = fs::metadata(&path).await else {
                continue;
            };
            if metadata.is_file() {
                if let Ok(rel) = path.strip_prefix(&base) {
                    let rel_str = rel.to_string_lossy().replace('\\', "/");
                    entries.push(FileTreeEntry {
                        path: rel_str,
                        size: metadata.len(),
                    });
                }
            } else if metadata.is_dir() {
                stack.push(path);
            }
        }
    }

    Ok(Json(entries))
}

#[derive(Deserialize)]
struct DeleteParams {
    path: String,
    root_key: String,
}

async fn delete_file(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<DeleteParams>,
) -> Result<(), (StatusCode, String)> {
    let full_path = validate_path(&state, &payload.root_key, &payload.path)?;

    if full_path.is_dir() {
        fs::remove_dir_all(full_path)
            .await
            .map_err(|e| map_delete_error(&e))?;
    } else {
        fs::remove_file(full_path)
            .await
            .map_err(|e| map_delete_error(&e))?;
    }

    Ok(())
}

fn map_delete_error(error: &std::io::Error) -> (StatusCode, String) {
    if error.kind() == ErrorKind::NotFound || error.raw_os_error() == Some(2) {
        (StatusCode::NOT_FOUND, error.to_string())
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
    }
}

#[derive(Deserialize)]
struct BatchDeleteParams {
    root_key: String,
    directory: String,
    entries: Vec<String>,
}

async fn batch_delete(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<BatchDeleteParams>,
) -> Result<Json<Vec<String>>, (StatusCode, String)> {
    let dir_path = validate_path(&state, &payload.root_key, &payload.directory)?;
    let mut failed: Vec<String> = Vec::new();

    for entry in &payload.entries {
        if !is_single_path_entry(entry) {
            failed.push(entry.clone());
            continue;
        }

        let entry_path = dir_path.join(entry);
        let result = if entry_path.is_dir() {
            fs::remove_dir(&entry_path).await
        } else {
            fs::remove_file(&entry_path).await
        };

        match result {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {
                // Missing entries silently ignored per spec
            }
            Err(_) => {
                failed.push(entry.clone());
            }
        }
    }

    Ok(Json(failed))
}

fn is_single_path_entry(entry: &str) -> bool {
    if entry.is_empty() {
        return false;
    }

    let mut components = StdPath::new(entry).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

#[derive(Deserialize)]
struct TruncateParams {
    path: String,
    root_key: String,
    length: u64,
}

async fn truncate_file(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<TruncateParams>,
) -> Result<(), (StatusCode, String)> {
    let full_path = validate_path(&state, &payload.root_key, &payload.path)?;

    let file = fs::OpenOptions::new()
        .write(true)
        .open(&full_path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    file.set_len(payload.length)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(())
}

/// Atomically write a file (write to temp, then rename).
/// POST /`ops/write_atomic/{root_key`}
/// Headers:
///   X-Path-Base64: <base64 encoded path>
/// Body: raw bytes
async fn write_atomic(
    State(state): State<Arc<AppState>>,
    Path(root_key): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<(), (StatusCode, String)> {
    let path = extract_path_from_header(&headers)?;
    let full_path = validate_path(&state, &root_key, &path)?;

    // Ensure parent directory exists
    if let Some(parent) = full_path.parent() {
        fs::create_dir_all(parent).await.map_err(|e| {
            if e.kind() == ErrorKind::StorageFull {
                (StatusCode::INSUFFICIENT_STORAGE, e.to_string())
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
            }
        })?;
    }

    // Write to a temp file, then rename atomically
    let tmp_path = full_path.with_extension(format!(
        "{}.tmp",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    // Write data to temp file
    fs::write(&tmp_path, &body).await.map_err(|e| {
        if e.kind() == ErrorKind::StorageFull {
            (StatusCode::INSUFFICIENT_STORAGE, e.to_string())
        } else {
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        }
    })?;

    // Atomic rename
    if let Err(e) = fs::rename(&tmp_path, &full_path).await {
        // Clean up temp file on rename failure
        let _ = fs::remove_file(&tmp_path).await;
        return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
    }

    Ok(())
}

pub fn validate_path(
    state: &AppState,
    root_key: &str,
    path: &str,
) -> Result<PathBuf, (StatusCode, String)> {
    // Find root by key
    let roots = state.download_roots.read().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Lock poisoned".to_string(),
        )
    })?;
    let root = roots
        .iter()
        .find(|r| r.key == root_key)
        .ok_or_else(|| (StatusCode::FORBIDDEN, "Invalid root key".to_string()))?;

    let root_path = PathBuf::from(&root.path);
    let safe_root = resolve_with_existing_prefix(&root_path)?;
    let components = normalized_relative_components(path)?;

    let mut current = safe_root.clone();
    for component in components {
        let candidate = current.join(&component);
        if candidate.exists() {
            let resolved = candidate.canonicalize().map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to resolve path: {e}"),
                )
            })?;
            if !resolved.starts_with(&safe_root) {
                return Err((StatusCode::BAD_REQUEST, "Invalid path".to_string()));
            }
            current = resolved;
        } else {
            current.push(component);
        }
    }

    if !current.starts_with(&safe_root) {
        return Err((StatusCode::BAD_REQUEST, "Invalid path".to_string()));
    }

    Ok(current)
}

fn normalized_relative_components(
    path: &str,
) -> Result<Vec<std::ffi::OsString>, (StatusCode, String)> {
    let mut components = Vec::new();

    for component in StdPath::new(path).components() {
        match component {
            Component::Normal(part) => components.push(part.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err((StatusCode::BAD_REQUEST, "Invalid path".to_string()))
            }
        }
    }

    Ok(components)
}

fn resolve_with_existing_prefix(path: &StdPath) -> Result<PathBuf, (StatusCode, String)> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to resolve current directory: {e}"),
                )
            })?
            .join(path)
    };

    let mut current = absolute.clone();
    let mut missing_components = Vec::new();

    while !current.exists() {
        let name = current
            .file_name()
            .ok_or_else(|| (StatusCode::BAD_REQUEST, "Invalid root path".to_string()))?;
        missing_components.push(name.to_os_string());
        current = current
            .parent()
            .ok_or_else(|| (StatusCode::BAD_REQUEST, "Invalid root path".to_string()))?
            .to_path_buf();
    }

    let mut resolved = current.canonicalize().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to resolve root path: {e}"),
        )
    })?;

    for component in missing_components.iter().rev() {
        resolved.push(component);
    }

    Ok(resolved)
}

#[derive(Deserialize)]
struct FreeSpaceParams {
    root_key: String,
}

#[derive(Serialize)]
struct FreeSpaceResponse {
    free_space: u64,
}

async fn free_space(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<FreeSpaceParams>,
) -> Result<Json<FreeSpaceResponse>, (StatusCode, String)> {
    let root_path = validate_path(&state, &params.root_key, "")?;

    let free = get_available_space(&root_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("statvfs failed: {e}"),
        )
    })?;

    Ok(Json(FreeSpaceResponse { free_space: free }))
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn get_available_space(path: &std::path::Path) -> Result<u64, String> {
    use std::ffi::CString;
    let c_path = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|e| format!("invalid path: {e}"))?;
    unsafe {
        let mut stat: libc::statvfs = std::mem::zeroed();
        if libc::statvfs(c_path.as_ptr(), &raw mut stat) != 0 {
            return Err(format!("statvfs: {}", std::io::Error::last_os_error()));
        }
        // Types vary by platform (u32 on some, u64 on others)
        #[allow(clippy::unnecessary_cast, clippy::cast_lossless)]
        Ok(stat.f_bavail as u64 * stat.f_frsize as u64)
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn get_available_space(path: &std::path::Path) -> Result<u64, String> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut free_bytes: u64 = 0;
    let result = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free_bytes,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if result == 0 {
        return Err("GetDiskFreeSpaceExW failed".to_string());
    }
    Ok(free_bytes)
}

#[derive(Deserialize)]
struct VerifyChunksRequest {
    root_key: String,
    files: Vec<VerifyChunkFile>,
    chunk_size: u64,
    hashes: String, // base64-encoded concatenated 20-byte SHA1 hashes
    start_chunk: u64,
    chunk_count: u64,
}

#[derive(Deserialize)]
struct VerifyChunkFile {
    path: String,
    length: u64,
}

const VERIFY_MATCH: u8 = 0;
const VERIFY_MISMATCH: u8 = 1;
const VERIFY_IO_ERROR: u8 = 2;

/// Core verify-chunks logic: reads resolved files as a concatenated byte stream,
/// hashes each chunk with SHA1, and compares against expected hashes.
///
/// `resolved_files`: Vec of (path, length) — already validated.
/// `hashes_bytes`: raw concatenated 20-byte SHA1 hashes for each chunk.
/// Returns one byte per chunk: 0=MATCH, 1=MISMATCH, `2=IO_ERROR`.
async fn verify_chunks_core(
    resolved_files: &[(PathBuf, u64)],
    chunk_size: u64,
    hashes_bytes: &[u8],
    start_chunk: u64,
    chunk_count: usize,
) -> Vec<u8> {
    let total_length: u64 = resolved_files.iter().map(|(_, len)| len).sum();

    // Cumulative end offsets for each file in the concatenated stream
    let mut file_ends: Vec<u64> = Vec::with_capacity(resolved_files.len());
    {
        let mut cum = 0u64;
        for (_, len) in resolved_files {
            cum += len;
            file_ends.push(cum);
        }
    }

    let mut results = Vec::with_capacity(chunk_count);

    // Sequential read state
    let mut cur_file_idx: usize = 0;
    let mut cur_file: Option<File> = None;
    let mut cur_file_read_pos: u64 = 0;

    // Advance to the starting file
    let mut stream_pos = start_chunk * chunk_size;
    while cur_file_idx < resolved_files.len() && stream_pos >= file_ends[cur_file_idx] {
        cur_file_idx += 1;
    }

    let buf_size = std::cmp::min(chunk_size as usize, 256 * 1024);
    let mut read_buf = vec![0u8; buf_size];

    for chunk_i in 0..chunk_count {
        let chunk_len = std::cmp::min(chunk_size, total_length.saturating_sub(stream_pos));

        if chunk_len == 0 {
            results.push(VERIFY_IO_ERROR);
            stream_pos += chunk_size;
            continue;
        }

        let mut hasher = Sha1::new();
        let mut bytes_hashed: u64 = 0;
        let mut io_error = false;

        while bytes_hashed < chunk_len && !io_error {
            if cur_file_idx >= resolved_files.len() {
                io_error = true;
                break;
            }

            // Open file if needed
            if cur_file.is_none() {
                if let Ok(mut f) = File::open(&resolved_files[cur_file_idx].0).await {
                    let file_start = if cur_file_idx > 0 {
                        file_ends[cur_file_idx - 1]
                    } else {
                        0
                    };
                    let pos_in_file = stream_pos + bytes_hashed - file_start;
                    if pos_in_file > 0 && f.seek(SeekFrom::Start(pos_in_file)).await.is_err() {
                        io_error = true;
                        break;
                    }
                    cur_file_read_pos = pos_in_file;
                    cur_file = Some(f);
                } else {
                    io_error = true;
                    break;
                }
            }

            let file = cur_file.as_mut().unwrap();
            let file_remaining = resolved_files[cur_file_idx].1 - cur_file_read_pos;
            let chunk_remaining = chunk_len - bytes_hashed;
            let to_read = std::cmp::min(
                std::cmp::min(file_remaining, chunk_remaining) as usize,
                read_buf.len(),
            );

            if to_read == 0 {
                // Exhausted this file, move to next
                cur_file = None;
                cur_file_idx += 1;
                continue;
            }

            match file.read_exact(&mut read_buf[..to_read]).await {
                Ok(_) => {
                    hasher.update(&read_buf[..to_read]);
                    bytes_hashed += to_read as u64;
                    cur_file_read_pos += to_read as u64;

                    if cur_file_read_pos >= resolved_files[cur_file_idx].1 {
                        cur_file = None;
                        cur_file_idx += 1;
                    }
                }
                Err(_) => {
                    io_error = true;
                }
            }
        }

        if io_error {
            results.push(VERIFY_IO_ERROR);
            // Reset file state for next chunk
            cur_file = None;
            stream_pos += chunk_size;
            cur_file_idx = 0;
            while cur_file_idx < resolved_files.len() && stream_pos >= file_ends[cur_file_idx] {
                cur_file_idx += 1;
            }
        } else {
            let actual_hash = hasher.finalize();
            let expected_hash = &hashes_bytes[chunk_i * 20..(chunk_i + 1) * 20];
            results.push(if actual_hash.as_slice() == expected_hash {
                VERIFY_MATCH
            } else {
                VERIFY_MISMATCH
            });
            stream_pos += chunk_size;
        }
    }

    results
}

/// Verify chunks by reading files as a concatenated byte stream, hashing each
/// chunk with SHA1, and comparing against expected hashes.
///
/// POST /`ops/verify_chunks`
/// Body: JSON `VerifyChunksRequest`
/// Response: `application/octet-stream` — one byte per chunk (0=match, 1=mismatch, `2=io_error`)
async fn verify_chunks(
    State(state): State<Arc<AppState>>,
    Json(req): Json<VerifyChunksRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let hashes_bytes = BASE64.decode(&req.hashes).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid base64 hashes: {e}"),
        )
    })?;

    let chunk_count = req.chunk_count as usize;
    if hashes_bytes.len() != chunk_count * 20 {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Hash length mismatch: {} bytes for {} chunks",
                hashes_bytes.len(),
                chunk_count,
            ),
        ));
    }

    // Resolve all file paths upfront
    let mut resolved_files: Vec<(PathBuf, u64)> = Vec::with_capacity(req.files.len());
    for f in &req.files {
        let path = validate_path(&state, &req.root_key, &f.path)?;
        resolved_files.push((path, f.length));
    }

    let results = verify_chunks_core(
        &resolved_files,
        req.chunk_size,
        &hashes_bytes,
        req.start_chunk,
        chunk_count,
    )
    .await;

    Ok((
        [(header::CONTENT_TYPE, "application/octet-stream")],
        results,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        control_stream::ControlStreamSessionRegistry, media::HttpStreamSessionRegistry, DaemonStats,
    };
    use jstorrent_common::DownloadRoot;
    use sha1::{Digest, Sha1};
    use std::path::PathBuf;
    use std::sync::{Arc, RwLock};
    use tokio::sync::Mutex;

    fn make_test_state(root_path: &std::path::Path) -> Arc<AppState> {
        Arc::new(AppState {
            token: Arc::new(RwLock::new("secret".to_string())),
            profile_id: "test".to_string(),
            extension_id: Arc::new(RwLock::new(None)),
            download_roots: Arc::new(RwLock::new(vec![DownloadRoot {
                key: "root-a".to_string(),
                path: root_path.to_string_lossy().to_string(),
                display_name: "Root A".to_string(),
                removable: false,
                last_stat_ok: true,
                last_checked: 0,
                disk_id: String::new(),
            }])),
            stats: Arc::new(DaemonStats::new()),
            http_streams: Arc::new(HttpStreamSessionRegistry::default()),
            media_server: Arc::new(Mutex::new(crate::media::MediaServerState::default())),
            control_stream_sessions: Arc::new(ControlStreamSessionRegistry::default()),
            http_stream_bridge: None,
        })
    }

    /// Test helper: compute SHA1 hash the same way as `write_file_v2`
    fn compute_sha1_hex(data: &[u8]) -> String {
        let mut hasher = Sha1::new();
        hasher.update(data);
        hex::encode(hasher.finalize())
    }

    /// Test helper: compute raw SHA1 hash bytes
    fn sha1_bytes(data: &[u8]) -> Vec<u8> {
        let mut hasher = Sha1::new();
        hasher.update(data);
        hasher.finalize().to_vec()
    }

    /// Test helper: concatenate multiple SHA1 hashes
    fn concat_hashes(hashes: &[Vec<u8>]) -> Vec<u8> {
        let mut result = Vec::with_capacity(hashes.len() * 20);
        for h in hashes {
            result.extend_from_slice(h);
        }
        result
    }

    #[test]
    fn test_sha1_hash_computation() {
        // Known test vector: SHA1("hello") = aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d
        let hash = compute_sha1_hex(b"hello");
        assert_eq!(hash, "aaf4c61ddcc5e8a2dabede0f3b482cd9aea9434d");
    }

    #[test]
    fn test_sha1_empty_data() {
        // SHA1("") = da39a3ee5e6b4b0d3255bfef95601890afd80709
        let hash = compute_sha1_hex(b"");
        assert_eq!(hash, "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn test_sha1_binary_data() {
        // Test with binary data (16KB of 0xAB bytes)
        let data = vec![0xABu8; 16384];
        let hash = compute_sha1_hex(&data);
        // Just verify it produces a 40-char hex string
        assert_eq!(hash.len(), 40);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_hash_comparison_case_sensitive() {
        // The Rust implementation uses lowercase hex and exact comparison
        let hash = compute_sha1_hex(b"test");
        assert_eq!(hash, hash.to_lowercase());
        // Verify uppercase would NOT match (this is intentional behavior)
        assert_ne!(hash, hash.to_uppercase());
    }

    #[test]
    fn test_validate_path_rejects_parent_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let state = make_test_state(dir.path());

        let result = validate_path(&state, "root-a", "../escape.bin");
        assert!(matches!(result, Err((StatusCode::BAD_REQUEST, _))));
    }

    #[test]
    fn test_validate_path_rejects_symlink_escape() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("safe")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), dir.path().join("safe").join("escape")).unwrap();

        let state = make_test_state(dir.path());
        let result = validate_path(&state, "root-a", "safe/escape/file.bin");

        #[cfg(unix)]
        assert!(matches!(result, Err((StatusCode::BAD_REQUEST, _))));
    }

    #[test]
    fn test_is_single_path_entry_allows_literal_double_dots() {
        assert!(is_single_path_entry("temp....abc"));
        assert!(!is_single_path_entry("../escape"));
        assert!(!is_single_path_entry("nested/file"));
    }

    #[tokio::test]
    async fn test_verify_chunks_single_file_match() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("file.bin");
        let data = vec![1u8, 2, 3, 4, 5];
        tokio::fs::write(&file_path, &data).await.unwrap();

        let hashes = concat_hashes(&[sha1_bytes(&data)]);
        let files = vec![(file_path, 5u64)];

        let results = verify_chunks_core(&files, 5, &hashes, 0, 1).await;
        assert_eq!(results, vec![VERIFY_MATCH]);
    }

    #[tokio::test]
    async fn test_verify_chunks_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("file.bin");
        tokio::fs::write(&file_path, &[0u8, 0, 0, 0, 0])
            .await
            .unwrap();

        // Hash for different data
        let hashes = concat_hashes(&[sha1_bytes(&[1, 2, 3, 4, 5])]);
        let files = vec![(file_path, 5u64)];

        let results = verify_chunks_core(&files, 5, &hashes, 0, 1).await;
        assert_eq!(results, vec![VERIFY_MISMATCH]);
    }

    #[tokio::test]
    async fn test_verify_chunks_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.bin");

        let hashes = concat_hashes(&[sha1_bytes(&[0; 5])]);
        let files = vec![(missing, 5u64)];

        let results = verify_chunks_core(&files, 5, &hashes, 0, 1).await;
        assert_eq!(results, vec![VERIFY_IO_ERROR]);
    }

    #[tokio::test]
    async fn test_verify_chunks_multiple_chunks_last_shorter() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("file.bin");
        // 10 bytes, chunk_size=4 → chunks: [4, 4, 2]
        let data: Vec<u8> = (1..=10).collect();
        tokio::fs::write(&file_path, &data).await.unwrap();

        let chunk0 = &data[0..4];
        let chunk1 = &data[4..8];
        let chunk2 = &data[8..10];
        let hashes = concat_hashes(&[sha1_bytes(chunk0), sha1_bytes(chunk1), sha1_bytes(chunk2)]);
        let files = vec![(file_path, 10u64)];

        let results = verify_chunks_core(&files, 4, &hashes, 0, 3).await;
        assert_eq!(results, vec![VERIFY_MATCH, VERIFY_MATCH, VERIFY_MATCH]);
    }

    #[tokio::test]
    async fn test_verify_chunks_spanning_two_files() {
        let dir = tempfile::tempdir().unwrap();
        let f1_path = dir.path().join("f1.bin");
        let f2_path = dir.path().join("f2.bin");
        // f1: [1,2,3], f2: [4,5,6,7,8] → concat [1..8], chunk_size=4
        // chunk0: [1,2,3,4] spans both files, chunk1: [5,6,7,8]
        tokio::fs::write(&f1_path, &[1u8, 2, 3]).await.unwrap();
        tokio::fs::write(&f2_path, &[4u8, 5, 6, 7, 8])
            .await
            .unwrap();

        let hashes = concat_hashes(&[sha1_bytes(&[1, 2, 3, 4]), sha1_bytes(&[5, 6, 7, 8])]);
        let files = vec![(f1_path, 3u64), (f2_path, 5u64)];

        let results = verify_chunks_core(&files, 4, &hashes, 0, 2).await;
        assert_eq!(results, vec![VERIFY_MATCH, VERIFY_MATCH]);
    }

    #[tokio::test]
    async fn test_verify_chunks_start_chunk_offset() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("file.bin");
        let data: Vec<u8> = (0..12).collect();
        tokio::fs::write(&file_path, &data).await.unwrap();

        // 3 chunks of 4 bytes, only verify chunk 1
        let chunk1 = &data[4..8];
        let hashes = concat_hashes(&[sha1_bytes(chunk1)]);
        let files = vec![(file_path, 12u64)];

        let results = verify_chunks_core(&files, 4, &hashes, 1, 1).await;
        assert_eq!(results, vec![VERIFY_MATCH]);
    }

    #[tokio::test]
    async fn test_verify_chunks_corrupted_middle_file() {
        let dir = tempfile::tempdir().unwrap();
        let f1 = dir.path().join("f1.bin");
        let f2 = dir.path().join("f2.bin");
        let f3 = dir.path().join("f3.bin");

        let d1 = [1u8, 2, 3, 4];
        let d2_correct = [5u8, 6, 7, 8];
        let d3 = [9u8, 10, 11, 12];

        tokio::fs::write(&f1, &d1).await.unwrap();
        tokio::fs::write(&f2, &[0u8, 0, 0, 0]).await.unwrap(); // corrupted
        tokio::fs::write(&f3, &d3).await.unwrap();

        let hashes = concat_hashes(&[sha1_bytes(&d1), sha1_bytes(&d2_correct), sha1_bytes(&d3)]);
        let files = vec![(f1, 4u64), (f2, 4u64), (f3, 4u64)];

        let results = verify_chunks_core(&files, 4, &hashes, 0, 3).await;
        assert_eq!(results, vec![VERIFY_MATCH, VERIFY_MISMATCH, VERIFY_MATCH]);
    }

    #[tokio::test]
    async fn test_verify_chunks_missing_middle_file() {
        let dir = tempfile::tempdir().unwrap();
        let f1 = dir.path().join("f1.bin");
        let f2_missing = dir.path().join("f2.bin");
        let f3 = dir.path().join("f3.bin");

        let d1 = [1u8, 2, 3, 4];
        let d3 = [9u8, 10, 11, 12];

        tokio::fs::write(&f1, &d1).await.unwrap();
        // f2 not created — missing
        tokio::fs::write(&f3, &d3).await.unwrap();

        let hashes = concat_hashes(&[sha1_bytes(&d1), sha1_bytes(&[5, 6, 7, 8]), sha1_bytes(&d3)]);
        let files: Vec<(PathBuf, u64)> = vec![(f1, 4), (f2_missing, 4), (f3, 4)];

        let results = verify_chunks_core(&files, 4, &hashes, 0, 3).await;
        assert_eq!(results, vec![VERIFY_MATCH, VERIFY_IO_ERROR, VERIFY_MATCH]);
    }

    #[tokio::test]
    async fn test_delete_file_missing_returns_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let state = make_test_state(dir.path());

        let result = delete_file(
            State(state),
            Json(DeleteParams {
                root_key: "root-a".to_string(),
                path: "missing-file.bin".to_string(),
            }),
        )
        .await;

        assert!(matches!(result, Err((StatusCode::NOT_FOUND, _))));
    }
}
