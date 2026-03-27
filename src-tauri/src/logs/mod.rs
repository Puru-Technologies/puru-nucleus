//! Host filesystem log file reader.
//!
//! Provides discovery, listing, and reading of log files from the host:
//! Spring Boot logs, MySQL error logs, Nucleus daemon logs, etc.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogSource {
    pub name: String,
    pub path: String,
    pub source_type: String,
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
                });
            }
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

/// Read a log file with optional tail or offset+limit pagination.
///
/// - If `tail > 0`: return the last `tail` lines.
/// - If `offset` and `limit` are set: return lines starting at `offset` with `limit` count.
/// - Otherwise: return the entire file (with a 50MB size guard).
pub fn read_log_file(
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
