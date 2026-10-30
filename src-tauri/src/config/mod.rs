//! Configuration management

use crate::performance::PerformanceConfig;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Deployment mode — Docker containers vs native JAR processes
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DeploymentMode {
    Docker,
    Native,
}

impl Default for DeploymentMode {
    fn default() -> Self {
        Self::Docker
    }
}

/// Main nucleus configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NucleusConfig {
    pub hospital_code: String,
    pub server_ip: String,
    pub docker_compose_path: String,
    pub gcs_credentials_path: Option<String>,
    pub backup_enabled: bool,
    pub telemetry_enabled: bool,
    pub mysql_host: String,
    pub mysql_port: u16,
    pub mysql_user: String,
    pub mysql_password: String,
    pub auto_update_enabled: bool,
    pub release_channel: String,
    pub deployment_mode: DeploymentMode,
    pub jars_dir: Option<String>,
    pub jres_dir: Option<String>,
    pub native_logs_dir: Option<String>,
    pub dviewer_dir: Option<String>,
    /// Native-mode: how many previous JAR versions to retain per service (in
    /// addition to `active` and `pending`) for rollback. Older files are
    /// deleted by the manifest GC after a successful update. `0` still keeps
    /// the currently-active JAR; only history is trimmed.
    #[serde(default = "default_jar_history_keep")]
    pub jar_history_keep: usize,
    pub puru_data_path: Option<String>,
    /// Docker mode: host folders the containers write their logs to, read
    /// from the compose volumes when an existing setup is connected.
    pub docker_log_dirs: Vec<String>,
    pub daemon: Option<DaemonConfig>,
    pub lan: LanConfig,
    /// Set true when the setup wizard has completed at least once. Gates whether
    /// the configuration screens are shown by default in the UI.
    pub setup_completed: bool,
    /// When true, the UI hides all configuration screens (Settings, Compose,
    /// Setup, Master Data, Shell). Flipped by ops in nucleus.toml at handover.
    pub production_mode: bool,
    /// JVM memory budget for native-mode services. Defaults to auto-tuning from
    /// the box's RAM; see `crate::performance`.
    pub performance: PerformanceConfig,
}

/// Daemon mode configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DaemonConfig {
    pub port: u16,
    pub api_key: String,
    pub backup_schedule: BackupSchedule,
    pub telemetry_interval_minutes: u32,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            port: 9090,
            api_key: String::new(),
            backup_schedule: BackupSchedule::default(),
            telemetry_interval_minutes: 15,
        }
    }
}

/// Backup schedule configuration for daemon mode
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupSchedule {
    pub enabled: bool,
    pub interval_hours: u32,
    pub backup_type: String,
    /// Fixed time to run daily backup (e.g. "02:00" for 2 AM). 24-hour format.
    /// If set, overrides interval_hours — runs once daily at this time.
    #[serde(default)]
    pub backup_time: Option<String>,
}

impl Default for BackupSchedule {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_hours: 24,
            backup_type: "full".to_string(),
            backup_time: Some("02:00".to_string()),
        }
    }
}

/// LAN backup configuration — copies backups/binlogs to a network share (NFS/SMB mount)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LanConfig {
    pub enabled: bool,
    pub path: String,
    pub binlog_enabled: bool,
}

impl Default for LanConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: String::new(),
            binlog_enabled: false,
        }
    }
}

impl Default for NucleusConfig {
    fn default() -> Self {
        Self {
            hospital_code: String::new(),
            server_ip: String::new(),
            docker_compose_path: default_docker_compose_path(),
            gcs_credentials_path: None,
            backup_enabled: true,
            telemetry_enabled: true,
            mysql_host: "127.0.0.1".to_string(),
            mysql_port: 3306,
            mysql_user: "root".to_string(),
            mysql_password: String::new(),
            auto_update_enabled: true,
            release_channel: "stable".to_string(),
            deployment_mode: DeploymentMode::default(),
            jars_dir: None,
            jres_dir: None,
            native_logs_dir: None,
            dviewer_dir: None,
            jar_history_keep: default_jar_history_keep(),
            puru_data_path: None,
            docker_log_dirs: Vec::new(),
            daemon: None,
            lan: LanConfig::default(),
            setup_completed: false,
            production_mode: false,
            performance: PerformanceConfig::default(),
        }
    }
}

impl NucleusConfig {
    /// Resolved JARs directory
    pub fn jars_dir(&self) -> PathBuf {
        self.jars_dir
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                #[cfg(target_os = "windows")]
                { PathBuf::from(r"C:\PuruNucleus\jars") }
                #[cfg(not(target_os = "windows"))]
                { PathBuf::from("/opt/puru/jars") }
            })
    }

    /// Resolved JREs directory
    pub fn jres_dir(&self) -> PathBuf {
        self.jres_dir
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                #[cfg(target_os = "windows")]
                { PathBuf::from(r"C:\PuruNucleus\jres") }
                #[cfg(not(target_os = "windows"))]
                { PathBuf::from("/opt/puru/jres") }
            })
    }

    /// Resolved native logs directory
    pub fn native_logs_dir(&self) -> PathBuf {
        self.native_logs_dir
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                #[cfg(target_os = "windows")]
                { PathBuf::from(r"C:\PuruNucleus\logs") }
                #[cfg(not(target_os = "windows"))]
                { PathBuf::from("/opt/puru/logs") }
            })
    }

    /// Resolved env files directory
    pub fn env_dir(&self) -> PathBuf {
        #[cfg(target_os = "windows")]
        { PathBuf::from(r"C:\PuruNucleus\env") }
        #[cfg(not(target_os = "windows"))]
        { PathBuf::from("/opt/puru/env") }
    }

    /// Resolved nginx html directory
    pub fn nginx_html_dir(&self) -> PathBuf {
        #[cfg(target_os = "windows")]
        { PathBuf::from(r"C:\PuruNucleus\nginx\html") }
        #[cfg(not(target_os = "windows"))]
        { PathBuf::from("/opt/puru/nginx/html") }
    }

    /// Resolved dviewer static bundle directory (OHIF-Viewer Puru fork,
    /// served by the bundled nginx on port 3000 in native mode).
    pub fn dviewer_dir(&self) -> PathBuf {
        self.dviewer_dir
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                #[cfg(target_os = "windows")]
                { PathBuf::from(r"C:\PuruNucleus\dviewer") }
                #[cfg(not(target_os = "windows"))]
                { PathBuf::from("/opt/puru/dviewer") }
            })
    }
}

fn default_jar_history_keep() -> usize {
    3
}

/// Get default config directory path
pub fn config_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        PathBuf::from("C:\\PuruNucleus")
    }

    #[cfg(target_os = "linux")]
    {
        with_legacy_fallback("/etc/puru-nucleus", "/etc/puru-dc")
    }

    #[cfg(target_os = "macos")]
    {
        with_legacy_fallback("/usr/local/etc/puru-nucleus", "/usr/local/etc/puru-dc")
    }
}

/// Prefer the current config dir; fall back to the pre-rename ("puru-dc") dir
/// only while it is the one that exists, so an upgraded box keeps its config
/// and license until `migrate_legacy_config_dir` moves it.
#[cfg(unix)]
fn with_legacy_fallback(current: &str, legacy: &str) -> PathBuf {
    let current = PathBuf::from(current);
    let legacy = PathBuf::from(legacy);
    if !current.exists() && legacy.exists() {
        legacy
    } else {
        current
    }
}

/// Move the pre-rename config dir (`/etc/puru-dc`) to `/etc/puru-nucleus` once.
/// Needs root; without it the fallback in `config_dir` keeps using the old
/// dir. No-op on Windows, whose dir (`C:\PuruNucleus`) never changed.
pub fn migrate_legacy_config_dir() {
    #[cfg(target_os = "linux")]
    let (current, legacy) = ("/etc/puru-nucleus", "/etc/puru-dc");
    #[cfg(target_os = "macos")]
    let (current, legacy) = ("/usr/local/etc/puru-nucleus", "/usr/local/etc/puru-dc");
    #[cfg(unix)]
    {
        let (current, legacy) = (std::path::Path::new(current), std::path::Path::new(legacy));
        if legacy.exists() && !current.exists() {
            match std::fs::rename(legacy, current) {
                Ok(()) => tracing::info!("Moved config dir {} → {}", legacy.display(), current.display()),
                Err(e) => tracing::debug!("Config dir not moved ({}); using {}", e, legacy.display()),
            }
        }
    }
}

/// Resolve `{home}/puru/` using the real OS home directory.
pub fn home_puru_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|b| b.home_dir().join("puru"))
        .unwrap_or_else(|| PathBuf::from("puru"))
}

/// Get default docker compose path
fn default_docker_compose_path() -> String {
    #[cfg(target_os = "windows")]
    {
        "C:\\PuruDocker\\docker-compose.yml".to_string()
    }

    // The compose fragments mount /home/puru/data, so Linux sites live under
    // /home/puru. Using $HOME instead put the compose file in /root/puru when
    // nucleus runs as root, which it must on Linux.
    #[cfg(target_os = "linux")]
    {
        "/home/puru/docker/docker-compose.yml".to_string()
    }

    #[cfg(target_os = "macos")]
    {
        home_puru_dir()
            .join("docker")
            .join("docker-compose.yml")
            .to_string_lossy()
            .to_string()
    }
}

/// Give the daemon an API key if it has none. With an empty key the daemon
/// skips auth entirely and its REST API (service control, shell, config with
/// the MySQL password) is open to the whole LAN on 0.0.0.0. Returns true when a
/// key was generated, so the caller knows to save the config.
pub fn ensure_daemon_api_key(config: &mut NucleusConfig) -> bool {
    use rand::Rng;
    const CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";

    let daemon = config.daemon.get_or_insert_with(DaemonConfig::default);
    if !daemon.api_key.is_empty() {
        return false;
    }
    let mut rng = rand::thread_rng();
    daemon.api_key = (0..32).map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char).collect();
    true
}

/// Load configuration from file
pub fn load_config() -> Result<NucleusConfig, crate::error::NucleusError> {
    let config_path = config_dir().join("nucleus.toml");

    if !config_path.exists() {
        return Ok(NucleusConfig::default());
    }

    let content = std::fs::read_to_string(&config_path)?;
    let config: NucleusConfig = toml::from_str(&content).map_err(|e| {
        crate::error::NucleusError::InvalidConfig(format!("Failed to parse config: {}", e))
    })?;

    Ok(config)
}

/// Save configuration to file
pub fn save_config(config: &NucleusConfig) -> Result<(), crate::error::NucleusError> {
    let config_path = config_dir().join("nucleus.toml");

    // Ensure directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = toml::to_string_pretty(config).map_err(|e| {
        crate::error::NucleusError::InvalidConfig(format!("Failed to serialize config: {}", e))
    })?;

    std::fs::write(&config_path, content)?;

    Ok(())
}

/// Sync status for config files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSyncStatus {
    pub last_synced: Option<chrono::DateTime<chrono::Utc>>,
    pub pending_count: usize,
}

impl Default for ConfigSyncStatus {
    fn default() -> Self {
        Self {
            last_synced: None,
            pending_count: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_daemon_config() {
        let cfg = DaemonConfig::default();
        assert_eq!(cfg.port, 9090);
        assert!(cfg.api_key.is_empty());
        assert!(cfg.backup_schedule.enabled);
        assert_eq!(cfg.backup_schedule.interval_hours, 24);
        assert_eq!(cfg.backup_schedule.backup_type, "full");
        assert_eq!(cfg.telemetry_interval_minutes, 15);
    }

    #[test]
    fn test_config_without_daemon_section() {
        let toml_str = r#"
hospital_code = "TEST"
server_ip = "192.168.1.1"
"#;
        let config: NucleusConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.hospital_code, "TEST");
        assert!(config.daemon.is_none());
    }

    #[test]
    fn test_config_with_daemon_section() {
        let toml_str = r#"
hospital_code = "TEST"
server_ip = "192.168.1.1"

[daemon]
port = 8080
api_key = "secret123"
telemetry_interval_minutes = 30

[daemon.backup_schedule]
enabled = false
interval_hours = 12
backup_type = "partial"
"#;
        let config: NucleusConfig = toml::from_str(toml_str).unwrap();
        let daemon = config.daemon.unwrap();
        assert_eq!(daemon.port, 8080);
        assert_eq!(daemon.api_key, "secret123");
        assert_eq!(daemon.telemetry_interval_minutes, 30);
        assert!(!daemon.backup_schedule.enabled);
        assert_eq!(daemon.backup_schedule.interval_hours, 12);
        assert_eq!(daemon.backup_schedule.backup_type, "partial");
    }

    #[test]
    fn test_config_with_lan_section() {
        let toml_str = r#"
hospital_code = "TEST"

[lan]
enabled = true
path = "/mnt/nas/backups"
binlog_enabled = true
"#;
        let config: NucleusConfig = toml::from_str(toml_str).unwrap();
        assert!(config.lan.enabled);
        assert_eq!(config.lan.path, "/mnt/nas/backups");
        assert!(config.lan.binlog_enabled);
    }

    #[test]
    fn test_config_without_lan_section() {
        let toml_str = r#"
hospital_code = "TEST"
"#;
        let config: NucleusConfig = toml::from_str(toml_str).unwrap();
        assert!(!config.lan.enabled);
        assert!(config.lan.path.is_empty());
        assert!(!config.lan.binlog_enabled);
    }

    #[test]
    fn test_config_with_partial_daemon_section() {
        let toml_str = r#"
hospital_code = "TEST"

[daemon]
port = 9091
"#;
        let config: NucleusConfig = toml::from_str(toml_str).unwrap();
        let daemon = config.daemon.unwrap();
        assert_eq!(daemon.port, 9091);
        assert!(daemon.api_key.is_empty());
        assert!(daemon.backup_schedule.enabled);
        assert_eq!(daemon.backup_schedule.interval_hours, 24);
    }
}
