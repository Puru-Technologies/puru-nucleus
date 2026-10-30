//! Audit trail of configuration changes.
//!
//! Every change Nucleus makes to its own config, to a hospital env file, or to
//! the hospital's Firebase document is appended to `<config_dir>/audit.log`,
//! one line per change. The file is append-only: Nucleus never rewrites or
//! trims it. Before an env file is edited, the whole file is copied to
//! `<config_dir>/audit/` so the old version can always be put back.
//!
//! Lines are also copied to `hospital/{code}/audit` in Firestore. The status
//! reporter ships new lines with each telemetry push; `audit.cloud-offset`
//! remembers how far the file has been sent.
//!
//! Line format (tab-separated):
//! `2026-10-31T02:10:44+05:30  sync  env/general.env  puru.server.ip  "a" → "b"  by=puru(sudo)  backup=audit/…`

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::config;

/// One change, as written to a line of audit.log.
#[derive(Debug, Clone, PartialEq)]
pub struct AuditEntry {
    pub at: String,
    /// What made the change: `connect`, `sync`, `setup`, …
    pub source: String,
    /// What was changed: `nucleus.toml`, an env file path, `firebase`.
    pub target: String,
    pub key: String,
    pub old: String,
    pub new: String,
    pub by: String,
    /// Copy of the file taken before the change, if one was made.
    pub backup: String,
}

fn log_path() -> PathBuf {
    config::config_dir().join("audit.log")
}

fn offset_path() -> PathBuf {
    config::config_dir().join("audit.cloud-offset")
}

/// Who made the change: the sudo user when run with sudo, else the login user.
fn who() -> String {
    if let Ok(u) = std::env::var("SUDO_USER") {
        if !u.is_empty() {
            return format!("{}(sudo)", u);
        }
    }
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| "nucleus".into())
}

/// Machine name from the licence, for the cloud copy.
pub fn machine_name() -> String {
    crate::licensing::load_license()
        .ok()
        .flatten()
        .and_then(|l| l.machine_name)
        .unwrap_or_default()
}

/// Passwords and keys are never written in clear.
fn is_secret(key: &str) -> bool {
    let k = key.to_lowercase();
    ["password", "api_key", "secret", "token"].iter().any(|s| k.contains(s))
}

fn mask(key: &str, value: &str) -> String {
    if is_secret(key) && !value.is_empty() {
        "********".into()
    } else {
        value.to_string()
    }
}

/// Append one change to audit.log. Never fails the caller: an audit write
/// error is logged and the change goes ahead.
pub fn record(source: &str, target: &str, key: &str, old: &str, new: &str, backup: Option<&Path>) {
    let entry = AuditEntry {
        at: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
        source: source.into(),
        target: target.into(),
        key: key.into(),
        old: mask(key, old),
        new: mask(key, new),
        by: who(),
        backup: backup.map(|p| p.display().to_string()).unwrap_or_default(),
    };
    if let Err(e) = append(&entry) {
        tracing::warn!("Audit: could not write audit.log: {}", e);
    }
}

fn append(entry: &AuditEntry) -> std::io::Result<()> {
    let path = log_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(f, "{}", format_line(entry))
}

fn format_line(e: &AuditEntry) -> String {
    format!(
        "{}\t{}\t{}\t{}\t{:?} → {:?}\tby={}\tbackup={}",
        e.at, e.source, e.target, e.key, e.old, e.new, e.by, e.backup
    )
}

/// Inverse of `{:?}` for the quoted values in a line.
fn unquote(s: &str) -> String {
    let inner = s.trim().strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(s);
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

fn parse_line(line: &str) -> Option<AuditEntry> {
    let cols: Vec<&str> = line.split('\t').collect();
    if cols.len() < 7 {
        return None;
    }
    let (old, new) = cols[4].split_once(" → ")?;
    Some(AuditEntry {
        at: cols[0].into(),
        source: cols[1].into(),
        target: cols[2].into(),
        key: cols[3].into(),
        old: unquote(old),
        new: unquote(new),
        by: cols[5].strip_prefix("by=").unwrap_or(cols[5]).into(),
        backup: cols[6].strip_prefix("backup=").unwrap_or(cols[6]).into(),
    })
}

/// Copy a file into `<config_dir>/audit/` before it is edited. Returns the copy.
pub fn backup_file(path: &Path) -> std::io::Result<PathBuf> {
    let dir = config::config_dir().join("audit");
    std::fs::create_dir_all(&dir)?;
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let stamp = chrono::Local::now().format("%Y-%m-%dT%H%M%S");
    let mut dest = dir.join(format!("{}-{}", stamp, name));
    let mut n = 2;
    while dest.exists() {
        dest = dir.join(format!("{}-{}-{}", stamp, n, name));
        n += 1;
    }
    std::fs::copy(path, &dest)?;
    Ok(dest)
}

/// Send audit.log lines not yet in Firestore. Stops at the first failure and
/// keeps the rest for the next call, so nothing is lost or sent twice.
pub async fn ship_pending() {
    let config = match config::load_config() {
        Ok(c) if !c.hospital_code.is_empty() => c,
        _ => return,
    };
    let content = match std::fs::read(log_path()) {
        Ok(c) => c,
        Err(_) => return,
    };
    let start: usize = std::fs::read_to_string(offset_path())
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|&o: &usize| o <= content.len())
        .unwrap_or(0);
    if start == content.len() {
        return;
    }
    let client = match crate::firestore::FirestoreClient::new_from_config().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Audit: Firestore unavailable, will retry: {}", e);
            return;
        }
    };

    let mut offset = start;
    let mut sent = 0;
    // Only whole lines (ending in '\n'); a half-written last line waits.
    while let Some(nl) = content[offset..].iter().position(|&b| b == b'\n') {
        let line = String::from_utf8_lossy(&content[offset..offset + nl]).to_string();
        if let Some(entry) = parse_line(&line) {
            if let Err(e) = client.push_audit_entry(&config.hospital_code, &entry).await {
                tracing::warn!("Audit: cloud copy failed, will retry: {}", e);
                break;
            }
            sent += 1;
        }
        offset += nl + 1;
    }
    if offset != start {
        if let Err(e) = std::fs::write(offset_path(), offset.to_string()) {
            tracing::warn!("Audit: could not save cloud offset: {}", e);
        }
    }
    if sent > 0 {
        tracing::info!("Audit: {} entr{} copied to the cloud", sent, if sent == 1 { "y" } else { "ies" });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_round_trips() {
        let e = AuditEntry {
            at: "2026-10-31T02:10:44+05:30".into(),
            source: "sync".into(),
            target: "/home/puru/docker/env/general.env".into(),
            key: "puru.server.ip".into(),
            old: "192.168.0.111".into(),
            new: "say \"hi\"".into(),
            by: "puru(sudo)".into(),
            backup: "/etc/puru-nucleus/audit/x-general.env".into(),
        };
        assert_eq!(parse_line(&format_line(&e)), Some(e));
    }

    #[test]
    fn secrets_are_masked() {
        assert_eq!(mask("mysql_password", "abc"), "********");
        assert_eq!(mask("server_ip", "1.2.3.4"), "1.2.3.4");
        assert_eq!(mask("mysql_password", ""), "");
    }
}
