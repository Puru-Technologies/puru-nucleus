//! Host filesystem + Docker container log reader.
//!
//! Provides discovery, listing, reading, and live tailing of log sources:
//! Spring Boot logs, MySQL error logs, Nucleus daemon logs, and Docker
//! container stdout/stderr. Container sources use the pseudo-path
//! `container:<name>` (e.g. `container:auth`) so the rest of the API can
//! address them uniformly.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use tauri::Emitter;

const CONTAINER_PREFIX: &str = "container:";

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogSource {
    pub name: String,
    pub path: String,
    pub source_type: String,
    /// "directory" (a path containing log files) or "container" (a docker
    /// container whose stdout/stderr is the log). Default "directory" so
    /// existing JSON consumers stay valid.
    #[serde(default = "default_kind")]
    pub kind: String,
}

fn default_kind() -> String {
    "directory".to_string()
}

/// Event payload pushed to the frontend for each batch of new log lines.
#[derive(Debug, Clone, Serialize)]
pub struct LogTailEvent {
    pub stream_id: String,
    pub lines: Vec<String>,
    /// Set on the final event of a stream (file deleted, container exited,
    /// or stop_tail called). Frontend should unsubscribe.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogFileInfo {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub modified_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogFileContent {
    pub path: String,
    pub content: String,
    pub total_lines: usize,
    pub offset: usize,
    pub lines_returned: usize,
}

// ── Security ─────────────────────────────────────────────────────────────────

/// Sensitive filenames that must never be read.
const BLOCKED_FILENAMES: &[&str] = &[
    "nucleus.toml",
    "gcs-credentials.json",
    ".env",
    "shadow",
    "passwd",
    "id_rsa",
    "id_ed25519",
    "authorized_keys",
];

/// Maximum file size (50 MB) when no tail/pagination is specified.
const MAX_UNPAGEDSIZE: u64 = 50 * 1024 * 1024;

/// Maximum recursive scan depth for listing log files.
const MAX_SCAN_DEPTH: usize = 3;

/// Validate that the given path is under an allowed base directory
/// and does not reference sensitive files.
fn validate_path(path: &Path) -> Result<PathBuf, crate::error::NucleusError> {
    // Canonicalize to resolve symlinks and ..
    let canonical = std::fs::canonicalize(path).map_err(|e| {
        crate::error::NucleusError::Validation(format!(
            "Cannot resolve path '{}': {}",
            path.display(),
            e
        ))
    })?;

    // Block sensitive filenames
    if let Some(filename) = canonical.file_name().and_then(|f| f.to_str()) {
        let lower = filename.to_lowercase();
        for blocked in BLOCKED_FILENAMES {
            if lower == *blocked {
                return Err(crate::error::NucleusError::PermissionDenied(format!(
                    "Access to '{}' is not allowed",
                    filename
                )));
            }
        }
    }

    // Build list of allowed base directories
    let allowed = get_allowed_base_dirs();

    for base in &allowed {
        if let Ok(base_canon) = std::fs::canonicalize(base) {
            if canonical.starts_with(&base_canon) {
                return Ok(canonical);
            }
        }
    }

    Err(crate::error::NucleusError::PermissionDenied(format!(
        "Path '{}' is outside allowed log directories",
        canonical.display()
    )))
}

/// Get the list of directories we allow reading logs from.
fn get_allowed_base_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // Nucleus config dir
    dirs.push(crate::config::config_dir());

    // Platform-specific log directories
    #[cfg(target_os = "linux")]
    {
        dirs.push(PathBuf::from("/var/log"));
    }

    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/var/log"));
        dirs.push(PathBuf::from("/usr/local/var/log"));
    }

    #[cfg(target_os = "windows")]
    {
        dirs.push(PathBuf::from("C:\\ProgramData"));
        dirs.push(PathBuf::from("C:\\PuruNucleus\\logs"));
    }

    // Docker compose parent directory (from config)
    if let Ok(cfg) = crate::config::load_config() {
        if !cfg.docker_compose_path.is_empty() {
            if let Some(parent) = Path::new(&cfg.docker_compose_path).parent() {
                dirs.push(parent.to_path_buf());
            }
        }
    }

    dirs
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Return a list of known log source directories for the current platform.
pub fn get_known_log_paths() -> Vec<LogSource> {
    let mut sources = Vec::new();

    // Nucleus logs
    let config_dir = crate::config::config_dir();
    sources.push(LogSource {
        name: "Nucleus".into(),
        path: config_dir.display().to_string(),
        source_type: "nucleus".into(),
    });

    // Platform-specific
    #[cfg(target_os = "linux")]
    {
        sources.push(LogSource {
            name: "System Logs".into(),
            path: "/var/log".into(),
            source_type: "system".into(),
        });
    }

    #[cfg(target_os = "macos")]
    {
        sources.push(LogSource {
            name: "System Logs".into(),
            path: "/var/log".into(),
            source_type: "system".into(),
        });
        sources.push(LogSource {
            name: "Homebrew Logs".into(),
            path: "/usr/local/var/log".into(),
            source_type: "system".into(),
        });
    }

    // Docker compose directory
    if let Ok(cfg) = crate::config::load_config() {
        if !cfg.docker_compose_path.is_empty() {
            if let Some(parent) = Path::new(&cfg.docker_compose_path).parent() {
                sources.push(LogSource {
                    name: "Docker Compose".into(),
                    path: parent.display().to_string(),
                    source_type: "docker".into(),
                    kind: "directory".into(),
                });
            }
        }
    }

    // Tag existing directory sources (they were created before the field
    // existed; serde default would have made them "directory" on the wire
    // anyway, but be explicit)
    for s in &mut sources {
        if s.kind.is_empty() {
            s.kind = "directory".into();
        }
    }

    sources
}

/// Enumerate all log sources — directory sources from get_known_log_paths,
/// plus one entry per running Puru container (Docker mode only). Containers
/// are returned with `kind: "container"` and a pseudo-path
/// `container:<container_name>` that read_log_file / start_tail accept.
///
/// Failures to query Docker are swallowed — the directory sources are
/// always returned so the UI degrades gracefully when Docker is down.
pub async fn enumerate_sources() -> Vec<LogSource> {
    let mut sources = get_known_log_paths();

    if let Ok(services) = crate::services::get_services().await {
        for svc in services {
            // Only surface services that have a container — skip not-installed
            if svc.container_name.is_empty() {
                continue;
            }
            sources.push(LogSource {
                name: svc.name.clone(),
                path: format!("{}{}", CONTAINER_PREFIX, svc.container_name),
                source_type: "container".into(),
                kind: "container".into(),
            });
        }
    }

    sources
}

/// List log files under a base path, recursively up to MAX_SCAN_DEPTH.
/// Finds files matching: *.log, *.err, *.out, *.gz
/// Sorted by modification time (newest first).
pub fn list_log_files(base_path: &str) -> Result<Vec<LogFileInfo>, crate::error::NucleusError> {
    let base = validate_path(Path::new(base_path))?;

    let mut files = Vec::new();
    scan_dir(&base, 0, &mut files)?;

    // Sort by modified_at descending (newest first)
    files.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));

    Ok(files)
}

fn scan_dir(
    dir: &Path,
    depth: usize,
    files: &mut Vec<LogFileInfo>,
) -> Result<(), crate::error::NucleusError> {
    if depth > MAX_SCAN_DEPTH {
        return Ok(());
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(()), // Skip unreadable directories
    };

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
            scan_dir(&path, depth + 1, files)?;
            continue;
        }

        if !is_log_file(&path) {
            continue;
        }

        // Skip sensitive files silently
        if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
            let lower = filename.to_lowercase();
            if BLOCKED_FILENAMES.iter().any(|b| lower == *b) {
                continue;
            }
        }

        if let Ok(meta) = entry.metadata() {
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .map(|d| {
                            chrono::DateTime::from_timestamp(d.as_secs() as i64, 0)
                                .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                                .unwrap_or_default()
                        })
                })
                .unwrap_or_default();

            files.push(LogFileInfo {
                name: path
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default(),
                path: path.display().to_string(),
                size_bytes: meta.len(),
                modified_at: modified,
            });
        }
    }

    Ok(())
}

fn is_log_file(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => matches!(ext.to_lowercase().as_str(), "log" | "err" | "out" | "gz"),
        None => false,
    }
}

/// Read a log source. Dispatches on path:
/// - `container:<name>`: fetches stdout/stderr from the named Docker
///   container (Docker mode only). Honors `tail` (defaults to 1000 if not
///   set, since container logs have no random-access offset). `offset` /
///   `limit` are applied client-side after fetching.
/// - any other path: validated against the allowed-base-dirs whitelist and
///   read from disk (existing behavior).
pub async fn read_log_file(
    path: &str,
    tail: Option<usize>,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<LogFileContent, crate::error::NucleusError> {
    if let Some(container) = path.strip_prefix(CONTAINER_PREFIX) {
        return read_container_log(container, tail, offset, limit).await;
    }
    read_host_file(path, tail, offset, limit)
}

/// Read a container's stdout/stderr (Docker mode). Returns a LogFileContent
/// shaped the same as host files so the frontend can render uniformly.
async fn read_container_log(
    container: &str,
    tail: Option<usize>,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<LogFileContent, crate::error::NucleusError> {
    // Container logs have no random-access offset — Docker only supports
    // "last N lines". So `tail` is the primary control; `offset`/`limit`
    // narrow the returned window client-side after fetch.
    let fetch_tail = tail.unwrap_or(1000) as u64;
    let raw = crate::services::get_container_logs(container, fetch_tail, None, None).await?;

    let all_lines: Vec<&str> = raw.lines().collect();
    let total_lines = all_lines.len();

    let (selected, actual_offset) = if let Some(off) = offset {
        let lim = limit.unwrap_or(500);
        let start = off.min(total_lines);
        let end = (start + lim).min(total_lines);
        (&all_lines[start..end], start)
    } else {
        (&all_lines[..], 0)
    };

    Ok(LogFileContent {
        path: format!("{}{}", CONTAINER_PREFIX, container),
        content: selected.join("\n"),
        total_lines,
        offset: actual_offset,
        lines_returned: selected.len(),
    })
}

/// Read a host filesystem log file with optional tail or offset+limit
/// pagination. (Original `read_log_file` body, now private + invoked via
/// the async dispatcher above.)
fn read_host_file(
    path: &str,
    tail: Option<usize>,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<LogFileContent, crate::error::NucleusError> {
    let canonical = validate_path(Path::new(path))?;

    let meta = std::fs::metadata(&canonical)?;

    // Size guard when no pagination
    if tail.unwrap_or(0) == 0 && offset.is_none() && meta.len() > MAX_UNPAGEDSIZE {
        return Err(crate::error::NucleusError::Validation(format!(
            "File is {} MB — use tail or pagination to read large files",
            meta.len() / (1024 * 1024)
        )));
    }

    let content = std::fs::read_to_string(&canonical).map_err(|e| {
        crate::error::NucleusError::Io(e)
    })?;

    let all_lines: Vec<&str> = content.lines().collect();
    let total_lines = all_lines.len();

    let (selected_lines, actual_offset) = if let Some(t) = tail {
        if t > 0 {
            let start = total_lines.saturating_sub(t);
            (&all_lines[start..], start)
        } else {
            (&all_lines[..], 0)
        }
    } else if let Some(off) = offset {
        let lim = limit.unwrap_or(500);
        let start = off.min(total_lines);
        let end = (start + lim).min(total_lines);
        (&all_lines[start..end], start)
    } else {
        (&all_lines[..], 0)
    };

    let lines_returned = selected_lines.len();
    let result_content = selected_lines.join("\n");

    Ok(LogFileContent {
        path: canonical.display().to_string(),
        content: result_content,
        total_lines,
        offset: actual_offset,
        lines_returned,
    })
}

// ── Live tail streaming ─────────────────────────────────────────────────────
//
// Two backends share the same surface (start_tail / stop_tail + Tauri events
// named "log-tail-{stream_id}"):
//
//   • Host files: poll the file every TAIL_POLL_INTERVAL, track byte
//     offset, read appended bytes, split into lines, emit. Survives log
//     rotation (size shrunk → reset offset to 0 and re-read from start).
//   • Containers: bollard's logs(follow=true) async stream — bytes arrive
//     as the container writes them; split into lines per chunk.
//
// Streams are tracked in STREAMS by an opaque atomic-counter ID. stop_tail
// aborts the JoinHandle, which cancels the task at the next await point.

const TAIL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1000);
const TAIL_INITIAL_LINES: usize = 200;

static STREAMS: OnceLock<Mutex<HashMap<String, tokio::task::JoinHandle<()>>>> = OnceLock::new();
static STREAM_COUNTER: AtomicU64 = AtomicU64::new(1);

fn streams() -> &'static Mutex<HashMap<String, tokio::task::JoinHandle<()>>> {
    STREAMS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_stream_id() -> String {
    format!("s{}", STREAM_COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn event_name(stream_id: &str) -> String {
    format!("log-tail-{}", stream_id)
}

fn emit_lines(app: &tauri::AppHandle, stream_id: &str, lines: Vec<String>) {
    if lines.is_empty() {
        return;
    }
    let _ = app.emit(
        &event_name(stream_id),
        LogTailEvent {
            stream_id: stream_id.to_string(),
            lines,
            ended: None,
        },
    );
}

fn emit_end(app: &tauri::AppHandle, stream_id: &str, reason: &str) {
    let _ = app.emit(
        &event_name(stream_id),
        LogTailEvent {
            stream_id: stream_id.to_string(),
            lines: Vec::new(),
            ended: Some(reason.to_string()),
        },
    );
}

/// Start tailing a log source. Dispatches on path (`container:<name>` →
/// docker stream, otherwise → host file poll). Returns an opaque stream_id
/// the frontend uses to (a) subscribe to "log-tail-{stream_id}" events and
/// (b) call stop_tail when done. Initial back-fill of the last
/// TAIL_INITIAL_LINES is emitted as the first event, then new lines
/// stream as they appear.
pub async fn start_tail(
    app: tauri::AppHandle,
    path: String,
) -> Result<String, crate::error::NucleusError> {
    let stream_id = next_stream_id();

    if let Some(container) = path.strip_prefix(CONTAINER_PREFIX) {
        let handle = spawn_container_tail(app, stream_id.clone(), container.to_string()).await?;
        streams().lock().unwrap().insert(stream_id.clone(), handle);
    } else {
        // Validate before spawning so callers get an immediate error on
        // bad/forbidden paths rather than a silent no-op task.
        let canonical = validate_path(Path::new(&path))?;
        let handle = spawn_file_tail(app, stream_id.clone(), canonical);
        streams().lock().unwrap().insert(stream_id.clone(), handle);
    }

    Ok(stream_id)
}

/// Stop a running tail stream. Safe to call with an unknown id (no-op).
pub fn stop_tail(stream_id: &str) {
    if let Some(handle) = streams().lock().unwrap().remove(stream_id) {
        handle.abort();
    }
}

fn spawn_file_tail(
    app: tauri::AppHandle,
    stream_id: String,
    path: PathBuf,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};

        // Initial back-fill: read the file end and emit last N lines so the
        // viewer isn't empty before the next poll tick.
        let mut byte_offset: u64 = match tokio::fs::metadata(&path).await {
            Ok(m) => m.len(),
            Err(e) => {
                emit_end(&app, &stream_id, &format!("open failed: {}", e));
                return;
            }
        };

        if let Ok(content) = tokio::fs::read_to_string(&path).await {
            let lines: Vec<&str> = content.lines().collect();
            let start = lines.len().saturating_sub(TAIL_INITIAL_LINES);
            let initial: Vec<String> = lines[start..].iter().map(|s| s.to_string()).collect();
            emit_lines(&app, &stream_id, initial);
        }

        let mut ticker = tokio::time::interval(TAIL_POLL_INTERVAL);
        ticker.tick().await; // consume the immediate first tick

        // Partial-line carry-over: if a poll lands mid-line, hold the
        // tail bytes until the next newline arrives.
        let mut carry = String::new();

        loop {
            ticker.tick().await;

            let meta = match tokio::fs::metadata(&path).await {
                Ok(m) => m,
                Err(_) => {
                    emit_end(&app, &stream_id, "file removed");
                    return;
                }
            };

            let size = meta.len();
            if size < byte_offset {
                // Log rotation / truncation — reset.
                byte_offset = 0;
                carry.clear();
            }
            if size == byte_offset {
                continue;
            }

            let mut file = match tokio::fs::OpenOptions::new().read(true).open(&path).await {
                Ok(f) => f,
                Err(_) => continue,
            };
            if file.seek(SeekFrom::Start(byte_offset)).await.is_err() {
                continue;
            }

            let mut buf = Vec::with_capacity((size - byte_offset) as usize);
            if file.read_to_end(&mut buf).await.is_err() {
                continue;
            }
            byte_offset = size;

            let chunk = String::from_utf8_lossy(&buf);
            let combined = format!("{}{}", carry, chunk);

            let mut lines: Vec<String> =
                combined.split('\n').map(|s| s.to_string()).collect();
            // Last element is empty (trailing newline) or partial.
            let last = lines.pop().unwrap_or_default();
            if combined.ends_with('\n') {
                carry.clear();
            } else {
                carry = last;
            }

            emit_lines(&app, &stream_id, lines);
        }
    })
}

async fn spawn_container_tail(
    app: tauri::AppHandle,
    stream_id: String,
    container: String,
) -> Result<tokio::task::JoinHandle<()>, crate::error::NucleusError> {
    use bollard::container::LogsOptions;
    use futures_util::StreamExt;

    // Connect once on the calling task so a connection failure surfaces as a
    // start_tail error rather than a silent dead task.
    let docker = bollard::Docker::connect_with_local_defaults()
        .map_err(|e| crate::error::NucleusError::Docker(format!("docker connect: {}", e)))?;

    let handle = tokio::spawn(async move {
        let mut stream = docker.logs(
            &container,
            Some(LogsOptions::<String> {
                stdout: true,
                stderr: true,
                follow: true,
                tail: TAIL_INITIAL_LINES.to_string(),
                timestamps: true,
                ..Default::default()
            }),
        );

        let mut carry = String::new();
        while let Some(chunk) = stream.next().await {
            let text = match chunk {
                Ok(out) => out.to_string(),
                Err(e) => {
                    emit_end(&app, &stream_id, &format!("docker error: {}", e));
                    return;
                }
            };

            let combined = format!("{}{}", carry, text);
            let mut lines: Vec<String> = combined.split('\n').map(|s| s.to_string()).collect();
            let last = lines.pop().unwrap_or_default();
            if combined.ends_with('\n') {
                carry.clear();
            } else {
                carry = last;
            }
            emit_lines(&app, &stream_id, lines);
        }

        // Stream ended naturally (container exited or detached).
        emit_end(&app, &stream_id, "container stream ended");
    });

    Ok(handle)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_known_log_paths_returns_non_empty() {
        let paths = get_known_log_paths();
        assert!(!paths.is_empty(), "Should return at least one log source");
        // Should always have Nucleus source
        assert!(
            paths.iter().any(|s| s.source_type == "nucleus"),
            "Should include nucleus source"
        );
    }

    #[test]
    fn test_validate_path_rejects_sensitive_files() {
        // Create a temp file named .env to test
        let tmp = std::env::temp_dir().join(".env");
        std::fs::write(&tmp, "SECRET=value").ok();

        let result = validate_path(&tmp);
        // Should be rejected either because it's blocked or outside allowed dirs
        assert!(result.is_err());

        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn test_validate_path_rejects_outside_allowed_dirs() {
        // /tmp is not in allowed dirs
        let tmp = std::env::temp_dir().join("test_nucleus_outside.log");
        std::fs::write(&tmp, "test content").ok();

        let result = validate_path(&tmp);
        assert!(result.is_err());

        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn test_validate_path_rejects_traversal() {
        // Attempt path traversal
        let result = validate_path(Path::new("/var/log/../etc/shadow"));
        assert!(result.is_err());
    }

    #[test]
    fn test_is_log_file() {
        assert!(is_log_file(Path::new("app.log")));
        assert!(is_log_file(Path::new("error.err")));
        assert!(is_log_file(Path::new("output.out")));
        assert!(is_log_file(Path::new("archive.gz")));
        assert!(!is_log_file(Path::new("config.toml")));
        assert!(!is_log_file(Path::new("script.sh")));
    }
}
