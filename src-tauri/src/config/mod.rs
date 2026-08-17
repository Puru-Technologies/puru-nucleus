//! Configuration management

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
    pub puru_data_path: Option<String>,
    pub daemon: Option<DaemonConfig>,
    pub lan: LanConfig,
    /// Set true when the setup wizard has completed at least once. Gates whether
    /// the configuration screens are shown by default in the UI.
    pub setup_completed: bool,
    /// When true, the UI hides all configuration screens (Settings, Compose,
    /// Setup, Master Data, Shell). Flipped by ops in nucleus.toml at handover.
    pub production_mode: bool,
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
            puru_data_path: None,
            daemon: None,
            lan: LanConfig::default(),
            setup_completed: false,
            production_mode: false,
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

/// Get default config directory path
pub fn config_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        PathBuf::from("C:\\PuruNucleus")
    }

    #[cfg(target_os = "linux")]
    {
        PathBuf::from("/etc/puru-dc")
    }

    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/usr/local/etc/puru-dc")
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

    #[cfg(unix)]
    {
        home_puru_dir()
            .join("docker")
            .join("docker-compose.yml")
            .to_string_lossy()
            .to_string()
    }
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
