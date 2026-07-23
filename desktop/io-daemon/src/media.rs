use crate::{files::validate_path, AppState};
use async_stream::stream;
use async_trait::async_trait;
use axum::{
    body::{Body, Bytes},
    extract::{Path, State},
    http::{
        header::{
            ACCEPT_RANGES, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, PRAGMA,
        },
        HeaderMap, Method, StatusCode,
    },
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io,
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt, SeekFrom},
    net::TcpListener,
};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

const DEFAULT_STREAM_IDLE_TIMEOUT_MS: u64 = 12 * 60 * 60 * 1000;
const MEDIA_STREAM_CHUNK_SIZE: usize = 256 * 1024;

#[derive(Default)]
pub struct MediaServerState {
    pub port: Option<u16>,
}

#[derive(Clone)]
pub struct RegisteredHttpStream {
    pub token: String,
    pub owner_id: Option<String>,
    pub torrent_id: String,
    pub file_index: u32,
    pub root_key: String,
    pub path: String,
    #[allow(dead_code)]
    pub file_size: u64,
    pub mime_type: Option<String>,
    #[allow(dead_code)]
    pub created_at_ms: u64,
    pub last_accessed_at_ms: u64,
}

#[derive(Default)]
pub struct HttpStreamSessionRegistry {
    sessions: RwLock<HashMap<String, RegisteredHttpStream>>,
}

impl HttpStreamSessionRegistry {
    #[allow(clippy::too_many_arguments)]
    pub fn register(
        &self,
        token: String,
        owner_id: Option<String>,
        torrent_id: String,
        file_index: u32,
        root_key: String,
        path: String,
        file_size: u64,
        mime_type: Option<String>,
    ) -> RegisteredHttpStream {
        let now = now_ms();
        let session = RegisteredHttpStream {
            token: token.clone(),
            owner_id,
            torrent_id,
            file_index,
            root_key,
            path,
            file_size,
            mime_type,
            created_at_ms: now,
            last_accessed_at_ms: now,
        };
        let mut sessions = self
            .sessions
            .write()
            .expect("http stream registry poisoned");
        sessions.retain(|_, existing| !is_expired(existing, now));
        sessions.insert(token, session.clone());
        session
    }

    pub fn get_and_touch(&self, token: &str) -> Option<RegisteredHttpStream> {
        let now = now_ms();
        let mut sessions = self
            .sessions
            .write()
            .expect("http stream registry poisoned");
        match sessions.get_mut(token) {
            Some(session) if !is_expired(session, now) => {
                session.last_accessed_at_ms = now;
                Some(session.clone())
            }
            Some(_) => {
                sessions.remove(token);
                None
            }
            None => None,
        }
    }

    pub fn revoke(&self, token: &str) -> bool {
        let mut sessions = self
            .sessions
            .write()
            .expect("http stream registry poisoned");
        sessions.remove(token).is_some()
    }

    pub fn revoke_owned_by(&self, owner_id: &str) -> usize {
        let mut sessions = self
            .sessions
            .write()
            .expect("http stream registry poisoned");
        let before = sessions.len();
        sessions.retain(|_, session| session.owner_id.as_deref() != Some(owner_id));
        before.saturating_sub(sessions.len())
    }

    pub fn revoke_torrent(&self, torrent_id: &str) -> usize {
        let mut sessions = self
            .sessions
            .write()
            .expect("http stream registry poisoned");
        let before = sessions.len();
        sessions.retain(|_, session| session.torrent_id != torrent_id);
        before.saturating_sub(sessions.len())
    }

    pub fn peek(&self, token: &str) -> Option<RegisteredHttpStream> {
        let sessions = self.sessions.read().expect("http stream registry poisoned");
        sessions.get(token).cloned()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TorrentHttpStreamStatus {
    FileSkipped,
    StreamSessionMismatch,
    StreamSessionNotFound,
    TorrentErrored,
    TorrentInactive,
    TorrentRemoved,
    TorrentStopped,
}

#[derive(Debug)]
pub struct TorrentHttpStreamError {
    pub status: TorrentHttpStreamStatus,
    pub message: String,
}

impl TorrentHttpStreamError {
    pub(crate) fn new(status: TorrentHttpStreamStatus, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for TorrentHttpStreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for TorrentHttpStreamError {}

#[async_trait]
pub trait TorrentHttpStreamBridge: Send + Sync {
    async fn open_stream_session(
        &self,
        session_id: &str,
        stream_token: &str,
        torrent_id: &str,
        file_index: u32,
    ) -> Result<(), TorrentHttpStreamError>;

    async fn wait_for_range(
        &self,
        session_id: &str,
        stream_token: &str,
        torrent_id: &str,
        file_index: u32,
        offset: u64,
        length: usize,
    ) -> Result<(), TorrentHttpStreamError>;

    fn close_stream_session(&self, session_id: &str, reason: &str);
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegisterStreamRequest {
    pub(crate) stream_token: String,
    pub(crate) torrent_id: String,
    pub(crate) file_index: u32,
    pub(crate) root_key: String,
    pub(crate) path: String,
    pub(crate) file_size: u64,
    pub(crate) mime_type: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegisterStreamResponse {
    pub(crate) ok: bool,
    pub(crate) media_port: u16,
}

#[derive(Clone, Copy)]
struct HttpByteRange {
    start: u64,
    end_inclusive: u64,
    total_size: u64,
    partial: bool,
}

impl HttpByteRange {
    fn content_length(self) -> u64 {
        if self.total_size == 0 || self.end_inclusive < self.start {
            0
        } else {
            self.end_inclusive - self.start + 1
        }
    }

    fn content_range_header(self) -> String {
        format!(
            "bytes {}-{}/{}",
            self.start, self.end_inclusive, self.total_size
        )
    }
}

pub fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/stream/register", post(register_http_stream))
}

async fn register_http_stream(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RegisterStreamRequest>,
) -> Result<Json<RegisterStreamResponse>, (StatusCode, String)> {
    let response = register_http_stream_with_owner(state, payload, None).await?;
    Ok(Json(response))
}

pub(crate) async fn register_http_stream_with_owner(
    state: Arc<AppState>,
    payload: RegisterStreamRequest,
    owner_id: Option<String>,
) -> Result<RegisterStreamResponse, (StatusCode, String)> {
    if payload.stream_token.trim().is_empty() || payload.stream_token.len() > 256 {
        return Err((StatusCode::BAD_REQUEST, "Invalid streamToken".to_string()));
    }
    if payload.torrent_id.trim().is_empty() || payload.torrent_id.len() > 256 {
        return Err((StatusCode::BAD_REQUEST, "Invalid torrentId".to_string()));
    }
    if payload.root_key.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Invalid rootKey".to_string()));
    }
    if payload.path.trim().is_empty() || payload.path.contains("..") {
        return Err((StatusCode::BAD_REQUEST, "Invalid path".to_string()));
    }

    let full_path = validate_path(&state, &payload.root_key, &payload.path)?;
    let metadata = tokio::fs::metadata(&full_path)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if !metadata.is_file() {
        return Err((StatusCode::BAD_REQUEST, "Path is not a file".to_string()));
    }

    state.http_streams.register(
        payload.stream_token,
        owner_id,
        payload.torrent_id,
        payload.file_index,
        payload.root_key,
        payload.path,
        payload.file_size,
        payload.mime_type,
    );

    let media_port = ensure_media_server_started(state).await?;
    Ok(RegisterStreamResponse {
        ok: true,
        media_port,
    })
}

async fn ensure_media_server_started(state: Arc<AppState>) -> Result<u16, (StatusCode, String)> {
    let mut media_state = state.media_server.lock().await;
    if let Some(port) = media_state.port {
        return Ok(port);
    }

    let listener = TcpListener::bind("0.0.0.0:0")
        .await
        .map_err(internal_error)?;
    let port = listener.local_addr().map_err(internal_error)?.port();

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/stream/:token", get(stream_file))
        .with_state(state.clone());

    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app).await {
            tracing::error!("media server error: {}", error);
        }
    });

    tracing::info!("media server started on 0.0.0.0:{}", port);
    media_state.port = Some(port);
    Ok(port)
}

async fn stream_file(
    State(state): State<Arc<AppState>>,
    Path(token): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    if token.trim().is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }

    let Some(stream) = state.http_streams.get_and_touch(&token) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let full_path = match validate_path(&state, &stream.root_key, &stream.path) {
        Ok(path) => path,
        Err((status, message)) => {
            state.http_streams.revoke(&token);
            return (status, message).into_response();
        }
    };

    let metadata = match tokio::fs::metadata(&full_path).await {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) | Err(_) => {
            state.http_streams.revoke(&token);
            return StatusCode::NOT_FOUND.into_response();
        }
    };

    let total_size = metadata.len();
    let Some(range) = resolve_http_byte_range(
        headers
            .get(axum::http::header::RANGE)
            .and_then(|value| value.to_str().ok()),
        total_size,
    ) else {
        return range_not_satisfiable(total_size);
    };

    let mut response = Response::builder()
        .status(if range.partial {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header(
            CONTENT_TYPE,
            stream
                .mime_type
                .unwrap_or_else(|| "application/octet-stream".into()),
        )
        .header(ACCEPT_RANGES, "bytes")
        .header(CACHE_CONTROL, "private, no-store")
        .header(PRAGMA, "no-cache")
        .header(CONTENT_LENGTH, range.content_length().to_string());

    if range.partial {
        response = response.header(CONTENT_RANGE, range.content_range_header());
    }

    if method == Method::HEAD {
        return response
            .body(Body::empty())
            .unwrap_or_else(internal_response_error);
    }

    if range.content_length() == 0 {
        return response
            .body(Body::empty())
            .unwrap_or_else(internal_response_error);
    }

    if let Some(bridge) = state
        .http_stream_bridge
        .clone()
        .filter(|_| stream.owner_id.is_some())
    {
        let session_id = format!("rust-stream-{}", Uuid::new_v4());
        if let Err(error) = bridge
            .open_stream_session(&session_id, &token, &stream.torrent_id, stream.file_index)
            .await
        {
            return handle_stream_error(&state, &token, &error);
        }

        let mut file = match File::open(&full_path).await {
            Ok(file) => file,
            Err(error) => {
                bridge.close_stream_session(&session_id, "request-complete");
                return internal_error_response(error);
            }
        };

        if let Err(error) = file.seek(SeekFrom::Start(range.start)).await {
            bridge.close_stream_session(&session_id, "request-complete");
            return internal_error_response(error);
        }

        let first_chunk_len = first_chunk_len(range);
        if let Err(error) = bridge
            .wait_for_range(
                &session_id,
                &token,
                &stream.torrent_id,
                stream.file_index,
                range.start,
                first_chunk_len,
            )
            .await
        {
            bridge.close_stream_session(&session_id, "request-complete");
            return handle_stream_error(&state, &token, &error);
        }

        let mut first_chunk = vec![0u8; first_chunk_len];
        if let Err(error) = file.read_exact(&mut first_chunk).await {
            bridge.close_stream_session(&session_id, "request-complete");
            return internal_error_response(error);
        }

        let state_for_stream = StreamBodyState {
            bridge: bridge.clone(),
            session_id: session_id.clone(),
            close_reason: "request-complete",
        };
        let remaining_bytes = range.content_length() as usize - first_chunk_len;
        let remaining_stream = stream! {
            let _stream_state = state_for_stream;
            yield Ok::<Bytes, io::Error>(Bytes::from(first_chunk));

            let mut next_offset = range.start + first_chunk_len as u64;
            let mut bytes_left = remaining_bytes;
            while bytes_left > 0 {
                let chunk_len = bytes_left.min(MEDIA_STREAM_CHUNK_SIZE);
                bridge
                    .wait_for_range(
                        &session_id,
                        &token,
                        &stream.torrent_id,
                        stream.file_index,
                        next_offset,
                        chunk_len,
                    )
                    .await
                    .map_err(io::Error::other)?;

                let mut chunk = vec![0u8; chunk_len];
                file.read_exact(&mut chunk).await?;
                next_offset += chunk_len as u64;
                bytes_left -= chunk_len;
                yield Ok::<Bytes, io::Error>(Bytes::from(chunk));
            }
        };

        return response
            .body(Body::from_stream(remaining_stream))
            .unwrap_or_else(internal_response_error);
    }

    let mut file = match File::open(&full_path).await {
        Ok(file) => file,
        Err(error) => return internal_error_response(error),
    };

    if let Err(error) = file.seek(SeekFrom::Start(range.start)).await {
        return internal_error_response(error);
    }

    let stream_body = ReaderStream::new(file.take(range.content_length()));
    response
        .body(Body::from_stream(stream_body))
        .unwrap_or_else(internal_response_error)
}

fn range_not_satisfiable(total_size: u64) -> Response {
    Response::builder()
        .status(StatusCode::RANGE_NOT_SATISFIABLE)
        .header(ACCEPT_RANGES, "bytes")
        .header(CONTENT_RANGE, format!("bytes */{total_size}"))
        .header(CONTENT_LENGTH, "0")
        .body(Body::empty())
        .unwrap_or_else(internal_response_error)
}

#[derive(Clone)]
struct StreamBodyState {
    bridge: Arc<dyn TorrentHttpStreamBridge>,
    session_id: String,
    close_reason: &'static str,
}

impl Drop for StreamBodyState {
    fn drop(&mut self) {
        self.bridge
            .close_stream_session(&self.session_id, self.close_reason);
    }
}

fn first_chunk_len(range: HttpByteRange) -> usize {
    range.content_length().min(MEDIA_STREAM_CHUNK_SIZE as u64) as usize
}

fn handle_stream_error(
    state: &AppState,
    stream_token: &str,
    error: &TorrentHttpStreamError,
) -> Response {
    match error.status {
        TorrentHttpStreamStatus::TorrentStopped => {
            text_response(StatusCode::CONFLICT, "Torrent is stopped")
        }
        TorrentHttpStreamStatus::TorrentInactive => {
            text_response(StatusCode::CONFLICT, "Torrent is not active")
        }
        TorrentHttpStreamStatus::TorrentErrored => {
            text_response(StatusCode::CONFLICT, "Torrent is in an error state")
        }
        TorrentHttpStreamStatus::FileSkipped => {
            text_response(StatusCode::CONFLICT, "File is skipped")
        }
        TorrentHttpStreamStatus::TorrentRemoved
        | TorrentHttpStreamStatus::StreamSessionMismatch
        | TorrentHttpStreamStatus::StreamSessionNotFound => {
            state.http_streams.revoke(stream_token);
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

fn text_response(status: StatusCode, text: &str) -> Response {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(CONTENT_LENGTH, text.len().to_string())
        .body(Body::from(text.to_owned()))
        .unwrap_or_else(internal_response_error)
}

fn resolve_http_byte_range(range_header: Option<&str>, total_size: u64) -> Option<HttpByteRange> {
    if let Some(range_header) = range_header {
        if !range_header.starts_with("bytes=") || total_size == 0 {
            return None;
        }

        let spec = range_header.trim_start_matches("bytes=").trim();
        if spec.is_empty() || spec.contains(',') {
            return None;
        }

        let (start_part, end_part) = spec.split_once('-')?;
        let start_part = start_part.trim();
        let end_part = end_part.trim();

        if start_part.is_empty() {
            let suffix_length = end_part.parse::<u64>().ok()?;
            if suffix_length == 0 {
                return None;
            }
            let start = total_size.saturating_sub(suffix_length);
            return Some(HttpByteRange {
                start,
                end_inclusive: total_size.saturating_sub(1),
                total_size,
                partial: true,
            });
        }

        let start = start_part.parse::<u64>().ok()?;
        if start >= total_size {
            return None;
        }

        let end_inclusive = if end_part.is_empty() {
            total_size.saturating_sub(1)
        } else {
            end_part
                .parse::<u64>()
                .ok()?
                .min(total_size.saturating_sub(1))
        };
        if end_inclusive < start {
            return None;
        }

        return Some(HttpByteRange {
            start,
            end_inclusive,
            total_size,
            partial: true,
        });
    }

    Some(HttpByteRange {
        start: 0,
        end_inclusive: total_size.saturating_sub(1),
        total_size,
        partial: false,
    })
}

fn is_expired(session: &RegisteredHttpStream, now_ms: u64) -> bool {
    now_ms.saturating_sub(session.last_accessed_at_ms) > DEFAULT_STREAM_IDLE_TIMEOUT_MS
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn internal_error<E: std::fmt::Display>(error: E) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}

fn internal_error_response<E: std::fmt::Display>(error: E) -> Response {
    internal_error(error).into_response()
}

fn internal_response_error(error: axum::http::Error) -> Response {
    internal_error_response(error)
}

#[cfg(test)]
mod tests {
    use super::{
        resolve_http_byte_range, stream_file, HttpByteRange, HttpStreamSessionRegistry,
        MediaServerState, RegisteredHttpStream, TorrentHttpStreamBridge, TorrentHttpStreamError,
        TorrentHttpStreamStatus,
    };
    use crate::{control_stream::ControlStreamSessionRegistry, AppState, DaemonStats};
    use async_trait::async_trait;
    use axum::body::to_bytes;
    use axum::body::Bytes;
    use axum::extract::{Path, State};
    use axum::http::{header::RANGE, HeaderMap, Method, StatusCode};
    use axum::response::Response;
    use jstorrent_common::DownloadRoot;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;
    use tokio::sync::Notify;

    #[test]
    fn resolves_full_range_without_header() {
        assert_eq!(
            range_tuple(resolve_http_byte_range(None, 100)),
            Some((0, 99, false))
        );
    }

    #[test]
    fn resolves_explicit_partial_range() {
        assert_eq!(
            range_tuple(resolve_http_byte_range(Some("bytes=10-19"), 100)),
            Some((10, 19, true))
        );
    }

    #[test]
    fn resolves_suffix_range() {
        assert_eq!(
            range_tuple(resolve_http_byte_range(Some("bytes=-10"), 100)),
            Some((90, 99, true))
        );
    }

    #[test]
    fn rejects_invalid_or_multi_ranges() {
        assert!(resolve_http_byte_range(Some("bytes=10-5"), 100).is_none());
        assert!(resolve_http_byte_range(Some("bytes=1-2,4-5"), 100).is_none());
        assert!(resolve_http_byte_range(Some("items=0-9"), 100).is_none());
    }

    fn range_tuple(range: Option<HttpByteRange>) -> Option<(u64, u64, bool)> {
        range.map(|value| (value.start, value.end_inclusive, value.partial))
    }

    #[tokio::test]
    async fn head_request_does_not_open_stream_session() {
        let fixture = TestFixture::new(None).await;
        fixture.register("head-token", "torrent-a", 0, "fixture.bin", 32);

        let response = stream_file(
            State(fixture.state.clone()),
            Path("head-token".to_string()),
            Method::HEAD,
            headers_with_range("bytes=0-15"),
        )
        .await;

        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok()),
            Some("bytes 0-15/64")
        );
        assert_eq!(fixture.body_bytes(response).await, Bytes::new());
    }

    #[tokio::test]
    async fn bridge_backed_request_blocks_then_returns_partial_content() {
        let bridge = Arc::new(FakeBridge::blocking());
        let fixture = TestFixture::new(Some(bridge.clone())).await;
        fixture.register("stream-token", "torrent-a", 0, "fixture.bin", 64);

        let task = tokio::spawn(stream_file(
            State(fixture.state.clone()),
            Path("stream-token".to_string()),
            Method::GET,
            headers_with_range("bytes=8-23"),
        ));

        bridge.wait_started.notified().await;
        assert!(!task.is_finished());

        bridge.release_first_wait();
        let response = task.await.expect("stream task should complete");

        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        let bytes: Bytes = fixture.body_bytes(response).await;
        assert_eq!(bytes.as_ref(), &fixture.contents[8..24]);
        assert_eq!(bridge.open_count.load(Ordering::Relaxed), 1);
        assert_eq!(
            bridge.closed_reasons.lock().unwrap().as_slice(),
            ["request-complete"]
        );
    }

    #[tokio::test]
    async fn stopped_torrent_returns_conflict() {
        let bridge = Arc::new(FakeBridge::with_error(
            TorrentHttpStreamStatus::TorrentStopped,
        ));
        let fixture = TestFixture::new(Some(bridge.clone())).await;
        fixture.register("stopped-token", "torrent-stop", 0, "fixture.bin", 32);

        let response = stream_file(
            State(fixture.state.clone()),
            Path("stopped-token".to_string()),
            Method::GET,
            headers_with_range("bytes=0-15"),
        )
        .await;

        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            fixture.body_bytes(response).await,
            Bytes::from_static(b"Torrent is stopped")
        );
        assert_eq!(
            bridge.closed_reasons.lock().unwrap().as_slice(),
            ["request-complete"]
        );
    }

    #[tokio::test]
    async fn removed_torrent_revokes_stream_token() {
        let bridge = Arc::new(FakeBridge::with_error(
            TorrentHttpStreamStatus::TorrentRemoved,
        ));
        let fixture = TestFixture::new(Some(bridge)).await;
        fixture.register("removed-token", "torrent-remove", 0, "fixture.bin", 32);

        let response = stream_file(
            State(fixture.state.clone()),
            Path("removed-token".to_string()),
            Method::GET,
            headers_with_range("bytes=0-15"),
        )
        .await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(fixture
            .state
            .http_streams
            .get_and_touch("removed-token")
            .is_none());
    }

    struct TestFixture {
        _temp_dir: TempDir,
        state: Arc<AppState>,
        contents: Vec<u8>,
    }

    impl TestFixture {
        async fn new(bridge: Option<Arc<dyn TorrentHttpStreamBridge>>) -> Self {
            let temp_dir = TempDir::new().expect("temp dir");
            let root_path = temp_dir.path().to_path_buf();
            let contents: Vec<u8> = (0..64).map(|i| i as u8).collect();
            tokio::fs::write(root_path.join("fixture.bin"), &contents)
                .await
                .expect("write fixture");

            let state = Arc::new(AppState {
                token: Arc::new(std::sync::RwLock::new("secret".to_string())),
                profile_id: "test".to_string(),
                extension_id: Arc::new(std::sync::RwLock::new(None)),
                download_roots: Arc::new(std::sync::RwLock::new(vec![DownloadRoot {
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
                media_server: Arc::new(tokio::sync::Mutex::new(MediaServerState::default())),
                control_stream_sessions: Arc::new(ControlStreamSessionRegistry::default()),
                http_stream_bridge: bridge,
            });

            Self {
                _temp_dir: temp_dir,
                state,
                contents,
            }
        }

        fn register(
            &self,
            token: &str,
            torrent_id: &str,
            file_index: u32,
            path: &str,
            file_size: u64,
        ) -> RegisteredHttpStream {
            self.state.http_streams.register(
                token.to_string(),
                Some("test-owner".to_string()),
                torrent_id.to_string(),
                file_index,
                "root-a".to_string(),
                path.to_string(),
                file_size,
                Some("application/octet-stream".to_string()),
            )
        }

        async fn body_bytes(&self, response: Response) -> Bytes {
            to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body bytes")
        }
    }

    fn headers_with_range(range: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(RANGE, range.parse().expect("valid range header"));
        headers
    }

    struct FakeBridge {
        behavior: Mutex<BridgeBehavior>,
        open_count: AtomicUsize,
        wait_started: Arc<Notify>,
        closed_reasons: Mutex<Vec<String>>,
    }

    enum BridgeBehavior {
        Blocking { gate: Arc<Notify>, released: bool },
        Error(TorrentHttpStreamStatus),
    }

    impl FakeBridge {
        fn blocking() -> Self {
            Self {
                behavior: Mutex::new(BridgeBehavior::Blocking {
                    gate: Arc::new(Notify::new()),
                    released: false,
                }),
                open_count: AtomicUsize::new(0),
                wait_started: Arc::new(Notify::new()),
                closed_reasons: Mutex::new(Vec::new()),
            }
        }

        fn with_error(status: TorrentHttpStreamStatus) -> Self {
            Self {
                behavior: Mutex::new(BridgeBehavior::Error(status)),
                open_count: AtomicUsize::new(0),
                wait_started: Arc::new(Notify::new()),
                closed_reasons: Mutex::new(Vec::new()),
            }
        }

        fn release_first_wait(&self) {
            let gate = {
                let mut behavior = self.behavior.lock().unwrap();
                match &mut *behavior {
                    BridgeBehavior::Blocking { gate, released } => {
                        *released = true;
                        Some(gate.clone())
                    }
                    BridgeBehavior::Error(_) => None,
                }
            };
            if let Some(gate) = gate {
                gate.notify_waiters();
            }
        }
    }

    #[async_trait]
    impl TorrentHttpStreamBridge for FakeBridge {
        async fn open_stream_session(
            &self,
            _session_id: &str,
            _stream_token: &str,
            _torrent_id: &str,
            _file_index: u32,
        ) -> Result<(), TorrentHttpStreamError> {
            self.open_count.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }

        async fn wait_for_range(
            &self,
            _session_id: &str,
            _stream_token: &str,
            _torrent_id: &str,
            _file_index: u32,
            _offset: u64,
            _length: usize,
        ) -> Result<(), TorrentHttpStreamError> {
            self.wait_started.notify_waiters();
            let wait_gate = {
                let behavior = self.behavior.lock().unwrap();
                match &*behavior {
                    BridgeBehavior::Blocking { gate, released } if !released => Some(gate.clone()),
                    BridgeBehavior::Error(status) => {
                        return Err(TorrentHttpStreamError::new(*status, status_label(*status)));
                    }
                    BridgeBehavior::Blocking { .. } => None,
                }
            };

            if let Some(gate) = wait_gate {
                gate.notified().await;
            }
            Ok(())
        }

        fn close_stream_session(&self, _session_id: &str, reason: &str) {
            self.closed_reasons.lock().unwrap().push(reason.to_string());
        }
    }

    fn status_label(status: TorrentHttpStreamStatus) -> &'static str {
        match status {
            TorrentHttpStreamStatus::FileSkipped => "FileSkipped",
            TorrentHttpStreamStatus::StreamSessionMismatch => "StreamSessionMismatch",
            TorrentHttpStreamStatus::StreamSessionNotFound => "StreamSessionNotFound",
            TorrentHttpStreamStatus::TorrentErrored => "TorrentErrored",
            TorrentHttpStreamStatus::TorrentInactive => "TorrentInactive",
            TorrentHttpStreamStatus::TorrentRemoved => "TorrentRemoved",
            TorrentHttpStreamStatus::TorrentStopped => "TorrentStopped",
        }
    }
}
