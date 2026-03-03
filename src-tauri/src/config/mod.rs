//! Configuration management

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    pub daemon: Option<DaemonConfig>,
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
}

impl Default for BackupSchedule {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_hours: 24,
            backup_type: "full".to_string(),
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
            daemon: None,
        }
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
        PathBuf::from("/etc/puru-nucleus")
    }

    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/usr/local/etc/puru-nucleus")
    }
}

/// Get default docker compose path
fn default_docker_compose_path() -> String {
    #[cfg(target_os = "windows")]
    {
        "C:\\PuruDocker\\docker-compose.yml".to_string()
    }

    #[cfg(target_os = "linux")]
    {
        "/home/puru/docker/docker-compose.yml".to_string()
    }

    #[cfg(target_os = "macos")]
    {
        "/Users/puru/docker/docker-compose.yml".to_string()
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
