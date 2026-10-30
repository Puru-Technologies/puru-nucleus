//! A site that already runs Puru in Docker: what the server itself says
//! (compose file, env files, volumes, scheduled jobs) and how that compares
//! with the hospital's Firebase document.
//!
//! The server is the source of truth. Connect copies its values into
//! nucleus.toml and up to Firebase; `puru-nucleus sync` lets the operator
//! decide property by property, and changes an env file only after showing
//! the change and getting a yes. Every change is written to the audit log.

use std::path::{Path, PathBuf};

use crate::audit;
use crate::config::NucleusConfig;

// ── What the server says ─────────────────────────────────────────────────────

pub struct ServerFacts {
    pub compose_path: String,
    pub env_files: Vec<PathBuf>,
    pub general_env: Option<PathBuf>,
    /// Host folders the containers write logs to (from the compose volumes).
    pub log_dirs: Vec<String>,
    /// Host data root, e.g. /home/puru/data (from the compose volumes).
    pub data_root: Option<String>,
    pub db_user: Option<String>,
    pub db_password: Option<String>,
}

pub fn read_server(compose_path: &str) -> ServerFacts {
    let compose = Path::new(compose_path);
    let env_dir = compose.parent().map(|d| d.join("env")).filter(|d| d.is_dir());
    let mut env_files: Vec<PathBuf> = env_dir
        .as_ref()
        .and_then(|d| std::fs::read_dir(d).ok())
        .map(|it| it.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect())
        .unwrap_or_default();
    env_files.sort();
    let general_env = env_dir.as_ref().map(|d| d.join("general.env")).filter(|p| p.is_file());

    let host_paths = std::fs::read_to_string(compose)
        .map(|t| volume_host_paths(&t))
        .unwrap_or_default();
    let mut log_dirs: Vec<String> = host_paths
        .iter()
        .filter(|p| p.to_lowercase().contains("log"))
        .cloned()
        .collect();
    log_dirs.sort();
    log_dirs.dedup();

    let db = crate::detection::detect_environment(compose_path)
        .ok()
        .and_then(|d| d.database);

    ServerFacts {
        compose_path: compose_path.to_string(),
        env_files,
        general_env,
        log_dirs,
        data_root: data_root_of(&host_paths),
        db_user: db.as_ref().map(|d| d.username.clone()).filter(|s| !s.is_empty()),
        db_password: db.map(|d| d.password).filter(|s| !s.is_empty()),
    }
}

/// Host side of every bind mount in a compose file: short syntax
/// (`- /home/puru/data/has:/data`) and long syntax (`source: /home/puru/logs`).
fn volume_host_paths(compose: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in compose.lines() {
        let t = line.trim();
        let (entry, short) = if let Some(rest) = t.strip_prefix("- ") {
            (rest, true)
        } else if let Some(rest) = t.strip_prefix("source:") {
            (rest, false)
        } else {
            continue;
        };
        let entry = entry.trim().trim_matches(|c: char| c == '"' || c == '\'');
        let bytes = entry.as_bytes();
        let host = if bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            // Windows: D:\puru\data:/data
            match entry[2..].find(':') {
                Some(i) => &entry[..i + 2],
                None => entry,
            }
        } else if entry.starts_with('/') {
            entry.split(':').next().unwrap_or("")
        } else {
            continue;
        };
        // A short-syntax volume always has a container side after the colon.
        if short && !entry[host.len()..].starts_with(':') {
            continue;
        }
        let host = host.trim_end_matches(&['/', '\\'][..]);
        if !host.is_empty() {
            out.push(host.to_string());
        }
    }
    out
}

/// `/home/puru/data/has` → `/home/puru/data`: the path up to its `data` folder.
fn data_root_of(paths: &[String]) -> Option<String> {
    paths.iter().find_map(|p| {
        let norm = p.replace('\\', "/");
        let parts: Vec<&str> = norm.split('/').collect();
        let i = parts.iter().position(|c| c.eq_ignore_ascii_case("data"))?;
        Some(parts[..=i].join("/")).filter(|r| !r.is_empty())
    })
}

/// `key=value` from an env file; empty and `{{PLACEHOLDER}}` values don't count.
pub fn env_get(path: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().find_map(|line| {
        let (k, v) = env_line(line)?;
        (k == key).then(|| v.to_string())
    })
    .filter(|v| !v.is_empty() && !v.contains("{{"))
}

/// Key and unquoted value of a `key=value` line; None for comments and blanks.
fn env_line(line: &str) -> Option<(&str, &str)> {
    let t = line.trim();
    if t.is_empty() || t.starts_with('#') {
        return None;
    }
    let (k, v) = t.split_once('=')?;
    let v = v.trim();
    let v = v.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(v);
    Some((k.trim(), v))
}

// ── Properties compared with Firebase ────────────────────────────────────────

/// The nucleus.toml field a property feeds, if any.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Store {
    ServerIp,
    DataPath,
    None,
}

pub struct Property {
    pub label: &'static str,
    pub firebase_field: &'static str,
    /// general.env key that holds the server's value.
    pub env_key: Option<&'static str>,
    pub store: Store,
}

pub const PROPERTIES: &[Property] = &[
    Property { label: "Server IP", firebase_field: "serverIp", env_key: Some("puru.server.ip"), store: Store::ServerIp },
    Property { label: "Data folder", firebase_field: "dataLocation", env_key: None, store: Store::DataPath },
    Property { label: "Deployment", firebase_field: "deployment_mode", env_key: None, store: Store::None },
    Property { label: "Hospital name", firebase_field: "name", env_key: Some("puru.hospital.name.en"), store: Store::None },
    Property { label: "Registration no.", firebase_field: "registrationNo", env_key: Some("puru.hospital.reg.no"), store: Store::None },
    Property { label: "Logo URL", firebase_field: "logoUrl", env_key: Some("hospital.logo.url"), store: Store::None },
];

impl Property {
    /// Whether a Firebase value can be applied on the server: it needs an env
    /// key or a nucleus.toml field to land in. Deployment is a fact of the
    /// server (it runs Docker), not a setting.
    pub fn can_take_firebase(&self) -> bool {
        self.env_key.is_some() || self.store != Store::None
    }
}

pub struct Row {
    pub prop: &'static Property,
    pub server: String,
    pub firebase: String,
}

impl Row {
    pub fn differs(&self) -> bool {
        !self.server.is_empty() && self.server != self.firebase
    }
}

fn server_value(p: &Property, facts: &ServerFacts) -> String {
    if let Some(key) = p.env_key {
        return facts.general_env.as_deref().and_then(|f| env_get(f, key)).unwrap_or_default();
    }
    match p.store {
        Store::DataPath => facts.data_root.clone().unwrap_or_default(),
        _ if p.firebase_field == "deployment_mode" => "docker".into(),
        _ => String::new(),
    }
}

pub fn compare(facts: &ServerFacts, doc: &crate::firestore::types::FirestoreDocument) -> Vec<Row> {
    PROPERTIES
        .iter()
        .map(|p| Row {
            prop: p,
            server: server_value(p, facts),
            firebase: doc
                .fields
                .get(p.firebase_field)
                .and_then(crate::firestore::convert::get_optional_string)
                .unwrap_or_default(),
        })
        .collect()
}

/// Write one server value to the hospital's Firebase document, with an audit line.
pub async fn push_to_firebase(
    client: &crate::firestore::FirestoreClient,
    code: &str,
    row: &Row,
    source: &str,
) -> Result<(), String> {
    client
        .set_hospital_string_field(code, row.prop.firebase_field, &row.server)
        .await
        .map_err(|e| e.user_message())?;
    audit::record(source, "firebase", row.prop.firebase_field, &row.firebase, &row.server, None);
    Ok(())
}

/// Every server value that Firebase has wrong, written to Firebase.
/// Returns how many were written.
pub async fn push_all_to_firebase(code: &str, facts: &ServerFacts, source: &str) -> Result<usize, String> {
    let client = crate::firestore::FirestoreClient::new_from_config()
        .await
        .map_err(|e| e.user_message())?;
    let doc = client.get_hospital(code).await.map_err(|e| e.user_message())?;
    let mut written = 0;
    for row in compare(facts, &doc).iter().filter(|r| r.differs()) {
        push_to_firebase(&client, code, row, source).await?;
        written += 1;
    }
    Ok(written)
}

// ── nucleus.toml from the server ─────────────────────────────────────────────

/// One nucleus.toml field changed in memory, audited once the config is saved.
pub struct Change {
    pub key: &'static str,
    pub old: String,
    pub new: String,
}

/// Write the audit lines for config changes that have been saved.
pub fn audit_config_changes(source: &str, changes: &[Change]) {
    for c in changes {
        audit::record(source, "nucleus.toml", c.key, &c.old, &c.new, None);
    }
}

fn set_tracked(changes: &mut Vec<Change>, key: &'static str, slot: &mut String, new: &str) {
    if !new.is_empty() && slot != new {
        changes.push(Change { key, old: slot.clone(), new: new.to_string() });
        *slot = new.to_string();
    }
}

fn set_tracked_opt(changes: &mut Vec<Change>, key: &'static str, slot: &mut Option<String>, new: &str) {
    let mut current = slot.clone().unwrap_or_default();
    set_tracked(changes, key, &mut current, new);
    if !current.is_empty() {
        *slot = Some(current);
    }
}

/// Take the server's values into nucleus.toml (overriding whatever activation
/// copied from Firebase). Returns the changes for the audit log; the caller
/// saves the config and then audits them.
pub fn apply_server_to_config(config: &mut NucleusConfig, facts: &ServerFacts) -> Vec<Change> {
    let mut changes = Vec::new();
    set_tracked(&mut changes, "docker_compose_path", &mut config.docker_compose_path, &facts.compose_path);
    if let Some(ip) = facts.general_env.as_deref().and_then(|f| env_get(f, "puru.server.ip")) {
        set_tracked(&mut changes, "server_ip", &mut config.server_ip, &ip);
    }
    if let Some(root) = &facts.data_root {
        set_tracked_opt(&mut changes, "puru_data_path", &mut config.puru_data_path, root);
    }
    if let Some(user) = &facts.db_user {
        set_tracked(&mut changes, "mysql_user", &mut config.mysql_user, user);
    }
    if let Some(pw) = &facts.db_password {
        set_tracked(&mut changes, "mysql_password", &mut config.mysql_password, pw);
    }
    if config.docker_log_dirs != facts.log_dirs {
        changes.push(Change {
            key: "docker_log_dirs",
            old: config.docker_log_dirs.join(", "),
            new: facts.log_dirs.join(", "),
        });
        config.docker_log_dirs = facts.log_dirs.clone();
    }
    changes
}

/// Put a Firebase value into nucleus.toml for a property that has a field there.
pub fn apply_to_config(config: &mut NucleusConfig, store: Store, value: &str) -> Vec<Change> {
    let mut changes = Vec::new();
    match store {
        Store::ServerIp => set_tracked(&mut changes, "server_ip", &mut config.server_ip, value),
        Store::DataPath => set_tracked_opt(&mut changes, "puru_data_path", &mut config.puru_data_path, value),
        Store::None => {}
    }
    changes
}

// ── Env file edits ───────────────────────────────────────────────────────────

/// The line as it is now (None if the key is missing) and as it would become.
pub fn env_change_preview(path: &Path, key: &str, value: &str) -> (Option<String>, String) {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let old = text
        .lines()
        .find(|l| env_line(l).map(|(k, _)| k == key).unwrap_or(false))
        .map(String::from);
    let quoted = old
        .as_deref()
        .and_then(|l| l.split_once('='))
        .map(|(_, v)| v.trim().starts_with('"'))
        .unwrap_or(false);
    let new = if quoted { format!("{}=\"{}\"", key, value) } else { format!("{}={}", key, value) };
    (old, new)
}

/// Set `key` in an env file: copy the file to the audit folder, replace the
/// key's line (or add it at the end), and write the audit line.
pub fn set_env_value(path: &Path, key: &str, value: &str, source: &str) -> std::io::Result<()> {
    let text = std::fs::read_to_string(path)?;
    let (old_line, new_line) = env_change_preview(path, key, value);
    let old_value = old_line.as_deref().and_then(env_line).map(|(_, v)| v.to_string()).unwrap_or_default();

    let backup = audit::backup_file(path)?;
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    match lines.iter().position(|l| env_line(l).map(|(k, _)| k == key).unwrap_or(false)) {
        Some(i) => lines[i] = new_line,
        None => lines.push(new_line),
    }
    let mut out = lines.join("\n");
    if text.ends_with('\n') || text.is_empty() {
        out.push('\n');
    }
    // Write in place so the file keeps its owner and permissions.
    std::fs::write(path, out)?;
    audit::record(source, &path.display().to_string(), key, &old_value, value, Some(&backup));
    Ok(())
}

// ── Scheduled backup jobs already on the server ─────────────────────────────

/// Nucleus's backup time when nothing else is near it.
const DEFAULT_BACKUP_MINUTE: u32 = 2 * 60;
/// Minutes kept between another backup job and Nucleus's.
const CLEARANCE_MINUTES: u32 = 30;

#[derive(Debug, Clone, PartialEq)]
pub struct BackupJob {
    pub hour: u32,
    pub minute: u32,
    /// Where it was found (cron file or systemd timer).
    pub source: String,
}

impl BackupJob {
    fn minutes(&self) -> u32 {
        self.hour * 60 + self.minute
    }
}

fn looks_like_backup(text: &str) -> bool {
    let t = text.to_lowercase();
    t.contains("backup") || t.contains("mysqldump") || t.contains("dump")
}

/// Fixed daily times of a cron line that runs a backup. Lines with ranges or
/// `*` in the minute/hour fields aren't daily jobs at a time and are skipped.
fn cron_backup_times(line: &str) -> Vec<(u32, u32)> {
    let t = line.trim();
    if t.is_empty() || t.starts_with('#') || !looks_like_backup(t) {
        return vec![];
    }
    let fields: Vec<&str> = t.split_whitespace().collect();
    if fields.len() < 6 {
        return vec![];
    }
    let nums = |f: &str, max: u32| -> Option<Vec<u32>> {
        f.split(',').map(|x| x.parse::<u32>().ok().filter(|&n| n < max)).collect()
    };
    match (nums(fields[0], 60), nums(fields[1], 24)) {
        (Some(mins), Some(hours)) => hours
            .iter()
            .flat_map(|&h| mins.iter().map(move |&m| (h, m)))
            .collect(),
        _ => vec![],
    }
}

/// Backup jobs in cron (system, cron.d and user crontabs) and systemd timers.
/// Read only.
pub fn existing_backup_jobs() -> Vec<BackupJob> {
    let mut files = vec![PathBuf::from("/etc/crontab")];
    for dir in ["/etc/cron.d", "/var/spool/cron/crontabs", "/var/spool/cron"] {
        if let Ok(rd) = std::fs::read_dir(dir) {
            files.extend(rd.flatten().map(|e| e.path()).filter(|p| p.is_file()));
        }
    }
    let mut jobs = Vec::new();
    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else { continue };
        for line in text.lines() {
            for (hour, minute) in cron_backup_times(line) {
                jobs.push(BackupJob { hour, minute, source: file.display().to_string() });
            }
        }
    }
    jobs.extend(systemd_backup_timers());
    jobs
}

fn systemd_backup_timers() -> Vec<BackupJob> {
    let Ok(out) = std::process::Command::new("systemctl")
        .args(["list-timers", "--all", "--no-legend", "--no-pager"])
        .output()
    else {
        return vec![];
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| looks_like_backup(l))
        .filter_map(|l| {
            let unit = l.split_whitespace().find(|w| w.ends_with(".timer")).unwrap_or("timer");
            // NEXT column: "Sat 2026-10-31 02:15:00 IST"
            let hms = l.split_whitespace().find(|w| w.len() == 8 && w.as_bytes()[2] == b':')?;
            let hour = hms[0..2].parse().ok()?;
            let minute = hms[3..5].parse().ok()?;
            Some(BackupJob { hour, minute, source: format!("systemd {}", unit) })
        })
        .collect()
}

fn circular_gap(a: u32, b: u32) -> u32 {
    let d = a.abs_diff(b) % 1440;
    d.min(1440 - d)
}

/// Nucleus's daily backup time ("HH:MM"): 02:00, moved 30 minutes past any
/// backup job that would start within 30 minutes of it. Also returns the job
/// it moved for, if any.
pub fn pick_backup_time(jobs: &[BackupJob]) -> (String, Option<BackupJob>) {
    let mut t = DEFAULT_BACKUP_MINUTE;
    let mut moved_for = None;
    for _ in 0..jobs.len() + 1 {
        match jobs.iter().find(|j| circular_gap(j.minutes(), t) < CLEARANCE_MINUTES) {
            Some(j) => {
                t = (j.minutes() + CLEARANCE_MINUTES) % 1440;
                moved_for = Some(j.clone());
            }
            None => break,
        }
    }
    (format!("{:02}:{:02}", t / 60, t % 60), moved_for)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volumes_short_and_long_syntax() {
        let compose = r#"
services:
  has:
    image: gcr.io/puru-255206/has
    ports:
      - "8082:8082"
    environment:
      - PURU_TOKEN_ENDPOINT=http://127.0.0.1:9090/api/gcp/token
    volumes:
      - /home/puru/data/has:/data
      - "/home/puru/logs/has:/logs:rw"
      - type: bind
        source: /home/puru/logs/pacs/
        target: /logs
"#;
        assert_eq!(
            volume_host_paths(compose),
            vec!["/home/puru/data/has", "/home/puru/logs/has", "/home/puru/logs/pacs"]
        );
        assert_eq!(data_root_of(&volume_host_paths(compose)).as_deref(), Some("/home/puru/data"));
    }

    #[test]
    fn cron_lines() {
        assert_eq!(cron_backup_times("15 2 * * * root /opt/puru/backup.sh"), vec![(2, 15)]);
        assert_eq!(cron_backup_times("15 2 * * * /opt/puru/backup.sh"), vec![(2, 15)]);
        assert!(cron_backup_times("*/5 * * * * root /opt/backup.sh").is_empty());
        assert!(cron_backup_times("# 15 2 * * * backup").is_empty());
        assert!(cron_backup_times("15 2 * * * root /usr/bin/logrotate").is_empty());
    }

    #[test]
    fn backup_time_moves_past_existing_job() {
        // BTCT: old job at 02:15 → Nucleus at 02:45.
        let jobs = vec![BackupJob { hour: 2, minute: 15, source: "cron".into() }];
        assert_eq!(pick_backup_time(&jobs).0, "02:45");
        assert_eq!(pick_backup_time(&[]).0, "02:00");
        let far = vec![BackupJob { hour: 20, minute: 0, source: "cron".into() }];
        assert_eq!(pick_backup_time(&far), ("02:00".into(), None));
    }

    #[test]
    fn env_edit_keeps_other_lines() {
        let dir = std::env::temp_dir().join("puru-existing-site-test");
        let _ = std::fs::create_dir_all(&dir);
        let f = dir.join("general.env");
        std::fs::write(&f, "# hospital\npuru.server.ip=192.168.0.10\nhospital.short.code=BTCT\n").unwrap();
        let (old, new) = env_change_preview(&f, "puru.server.ip", "192.168.0.111");
        assert_eq!(old.as_deref(), Some("puru.server.ip=192.168.0.10"));
        assert_eq!(new, "puru.server.ip=192.168.0.111");
        let (old, _) = env_change_preview(&f, "puru.hospital.reg.no", "R1");
        assert_eq!(old, None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
