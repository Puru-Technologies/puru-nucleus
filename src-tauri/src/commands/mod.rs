//! Tauri command handlers

use crate::backup::{BackupRecord, BackupResult, BackupType};
use crate::compose_template::{
    ComposeUploadResult, EnvDownloadResult, EnvFileEntry, EnvUploadResult,
    FragmentDownloadResult, ServiceModules, TemplateVariables,
};
use crate::config::{ConfigSyncStatus, NucleusConfig};
use crate::docker_update::{DockerUpdateResult, UpdateRecord};
use crate::licensing::License;
use crate::messaging::types::{
    DownloadResult as MsgDownloadResult, FileActionResult, HospitalMessage,
};
use crate::releases::{
    DownloadResult, JarPullResult, JarUpdateCheck, NucleusUpdateInfo, PullAllResult,
    ServiceManifest, ServiceUpdateInfo,
};
use crate::remote_shell::{ShellAuditEntry, ShellResult};
use crate::services::{DetectionResult, PrerequisiteStatus, ServiceInfo};
use crate::telemetry::TelemetrySnapshot;
use serde::{Deserialize, Serialize};

/// System information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfo {
    pub os: String,
    pub arch: String,
    pub hostname: String,
    pub cpu_cores: usize,
    pub total_ram_gb: f64,
    pub disk_free_gb: f64,
}

/// Get system information
#[tauri::command]
pub async fn get_system_info() -> Result<SystemInfo, String> {
    use sysinfo::{System, SystemExt, DiskExt};

    let mut sys = System::new_all();
    sys.refresh_all();

    let disk_free_gb = sys
        .disks()
        .iter()
        .map(|d| d.available_space())
        .sum::<u64>() as f64
        / 1024.0
        / 1024.0
        / 1024.0;

    Ok(SystemInfo {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        hostname: sys.host_name().unwrap_or_else(|| "unknown".to_string()),
        cpu_cores: sys.cpus().len(),
        total_ram_gb: sys.total_memory() as f64 / 1024.0 / 1024.0 / 1024.0,
        disk_free_gb,
    })
}

/// Get services
#[tauri::command]
pub async fn get_services() -> Result<Vec<ServiceInfo>, String> {
    crate::services::get_services()
        .await
        .map_err(|e| e.to_string())
}

/// Start a service
#[tauri::command]
pub async fn start_service(name: String) -> Result<(), String> {
    crate::services::start_service(&name)
        .await
        .map_err(|e| e.to_string())
}

/// Stop a service
#[tauri::command]
pub async fn stop_service(name: String) -> Result<(), String> {
    crate::services::stop_service(&name)
        .await
        .map_err(|e| e.to_string())
}

/// Restart a service
#[tauri::command]
pub async fn restart_service(name: String) -> Result<(), String> {
    crate::services::restart_service(&name)
        .await
        .map_err(|e| e.to_string())
}

/// Get license (returns null if not yet activated)
#[tauri::command]
pub async fn get_license() -> Result<Option<License>, String> {
    crate::licensing::load_license().map_err(|e| e.to_string())
}

/// Reset activation — audit log to Firestore, delete license, clear config.
#[tauri::command]
pub async fn reset_activation() -> Result<(), String> {
    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    let hospital_code = config.hospital_code.clone();
    let fingerprint = crate::licensing::fingerprint::get_cached_fingerprint()
        .unwrap_or_default();

    // Log deactivation to Firestore (best-effort)
    if !hospital_code.is_empty() {
        match crate::firestore::FirestoreClient::new_from_config().await {
            Ok(client) => {
                if let Err(e) = client.log_deactivation(&hospital_code, &fingerprint).await {
                    tracing::warn!("Firestore deactivation log failed (non-fatal): {:?}", e);
                } else {
                    tracing::info!("Deactivation logged to Firestore for {}", hospital_code);
                }
            }
            Err(_) => {
                tracing::warn!("Could not connect to Firestore for deactivation audit (non-fatal)");
            }
        }
    }

    // Delete license file
    let license_path = crate::config::config_dir().join("license.toml");
    if license_path.exists() {
        std::fs::remove_file(&license_path)
            .map_err(|e| format!("Failed to delete license: {}", e))?;
    }

    // Clear hospital_code from config
    let mut config = crate::config::load_config().map_err(|e| e.user_message())?;
    config.hospital_code = String::new();
    crate::config::save_config(&config).map_err(|e| e.user_message())?;

    tracing::info!("Activation reset — license removed, hospital_code cleared");
    Ok(())
}

/// Get the hardware fingerprint for this machine.
#[tauri::command]
pub async fn get_machine_fingerprint() -> Result<String, String> {
    crate::licensing::fingerprint::get_cached_fingerprint().map_err(|e| e.to_string())
}

/// Activate license with email — queries Firestore, extracts license, saves locally
#[tauri::command]
pub async fn activate_license(email: String, machine_name: String) -> Result<(), String> {
    if email.trim().is_empty() {
        return Err("Email is required".to_string());
    }
    if machine_name.trim().is_empty() {
        return Err("Machine name is required".to_string());
    }

    tracing::info!("License activation requested for: {}", email);

    // Generate hardware fingerprint
    let fingerprint = crate::licensing::fingerprint::generate_fingerprint()
        .map_err(|e| format!("Failed to generate hardware fingerprint: {}", e))?;

    let client = crate::firestore::FirestoreClient::new_from_config()
        .await
        .map_err(|e| {
            tracing::error!("Firestore client init failed: {:?}", e);
            e.user_message()
        })?;

    // Query hospital by email
    let doc = client
        .find_hospital_by_email(email.trim())
        .await
        .map_err(|e| {
            tracing::error!("Hospital query failed: {:?}", e);
            format!("{} ({})", e.user_message(), e)
        })?;

    // Extract hospital_code from document name (last path segment)
    let hospital_code = doc
        .name
        .split('/')
        .last()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Invalid hospital document path".to_string())?
        .to_string();

    // Extract hospital_name
    let hospital_name = doc
        .fields
        .get("name")
        .and_then(|v| crate::firestore::convert::get_optional_string(v))
        .unwrap_or_else(|| hospital_code.clone());

    // Build license using shared helper
    let mut license =
        crate::licensing::extract_license_from_firestore(&doc.fields, &hospital_name);

    // Validate license is not expired
    crate::licensing::validate_license(&license)?;

    // Set machine fingerprint and name
    license.machine_fingerprint = Some(fingerprint.clone());
    license.machine_name = Some(machine_name.trim().to_string());

    // Register machine in Firestore
    if let Err(e) = client
        .register_machine(&hospital_code, &fingerprint, machine_name.trim())
        .await
    {
        tracing::warn!("Failed to register machine in cloud (non-fatal): {:?}", e);
    }

    // Save license locally
    crate::licensing::save_license(&license).map_err(|e| e.user_message())?;

    // Update hospital_code in config
    let mut config = crate::config::load_config().map_err(|e| e.user_message())?;
    config.hospital_code = hospital_code.clone();
    crate::config::save_config(&config).map_err(|e| e.user_message())?;

    tracing::info!(
        "License activated for hospital: {} ({}) — machine: {}",
        license.hospital_name,
        hospital_code,
        fingerprint
    );
    Ok(())
}

/// Result of pulling hospital settings from cloud.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullSettingsResult {
    pub hospital_info: crate::licensing::HospitalInfo,
    pub license: License,
    pub license_changed: bool,
    pub machine_registered: bool,
}

/// Pull hospital settings (info + license) from Firestore.
#[tauri::command]
pub async fn pull_settings() -> Result<PullSettingsResult, String> {
    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    if config.hospital_code.is_empty() {
        return Err("Hospital code not configured. Activate a license first.".to_string());
    }

    let client = crate::firestore::FirestoreClient::new_from_config()
        .await
        .map_err(|e| e.user_message())?;

    let doc = client
        .get_hospital(&config.hospital_code)
        .await
        .map_err(|e| e.user_message())?;

    let hospital_name = doc
        .fields
        .get("name")
        .and_then(|v| crate::firestore::convert::get_optional_string(v))
        .unwrap_or_else(|| config.hospital_code.clone());

    let hospital_info = crate::licensing::extract_hospital_info(&doc.fields);
    let mut pulled_license =
        crate::licensing::extract_license_from_firestore(&doc.fields, &hospital_name);

    // Check machine registration
    let machines = crate::licensing::extract_registered_machines(&doc.fields);
    let current_fp = crate::licensing::fingerprint::get_cached_fingerprint().ok();
    let machine_registered = if machines.is_empty() {
        // No nucleus_machines array = no enforcement yet
        true
    } else {
        current_fp
            .as_ref()
            .map(|fp| machines.iter().any(|(m_fp, _)| m_fp == fp))
            .unwrap_or(false)
    };

    // Compare pulled license with current local license
    let current_license = crate::licensing::load_license().map_err(|e| e.user_message())?;
    let license_changed = match &current_license {
        Some(current) => {
            current.valid_till != pulled_license.valid_till
                || current.features.binlog_shipping != pulled_license.features.binlog_shipping
                || current.features.point_in_time_recovery
                    != pulled_license.features.point_in_time_recovery
                || current.features.priority_support != pulled_license.features.priority_support
                || current.limits.backup_retention_days
                    != pulled_license.limits.backup_retention_days
                || current.limits.max_storage_gb != pulled_license.limits.max_storage_gb
        }
        None => true,
    };

    // Preserve local machine_fingerprint and machine_name when saving pulled license
    if let Some(ref current) = current_license {
        pulled_license.machine_fingerprint = current.machine_fingerprint.clone();
        pulled_license.machine_name = current.machine_name.clone();
    }

    // If license changed, save the new license
    if license_changed {
        crate::licensing::save_license(&pulled_license).map_err(|e| e.user_message())?;
        tracing::info!(
            "License updated from cloud for hospital: {}",
            config.hospital_code
        );
    }

    Ok(PullSettingsResult {
        hospital_info,
        license: pulled_license,
        license_changed,
        machine_registered,
    })
}

/// Start backup
#[tauri::command]
pub async fn start_backup(r#type: String) -> Result<BackupResult, String> {
    let backup_type = r#type;
    let license = crate::licensing::load_license()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "No license activated. Activate a license before running backups.".to_string())?;

    let bt = match backup_type.as_str() {
        "full" => BackupType::Full,
        "partial" => BackupType::Partial,
        _ => return Err("Invalid backup type".to_string()),
    };

    crate::backup::start_backup(bt, &license)
        .await
        .map_err(|e| e.to_string())
}

/// Get backup history
#[tauri::command]
pub async fn get_backup_history() -> Result<Vec<BackupRecord>, String> {
    crate::backup::get_backup_history()
        .await
        .map_err(|e| e.to_string())
}

/// Restore backup
#[tauri::command]
pub async fn restore_backup(backup_id: String) -> Result<(), String> {
    crate::backup::restore_backup(&backup_id)
        .await
        .map_err(|e| e.to_string())
}

/// Get configuration
#[tauri::command]
pub async fn get_config() -> Result<NucleusConfig, String> {
    crate::config::load_config().map_err(|e| e.to_string())
}

/// Save configuration
#[tauri::command]
pub async fn save_config(config: NucleusConfig) -> Result<(), String> {
    crate::config::save_config(&config).map_err(|e| e.to_string())
}

/// Sync config to cloud
#[tauri::command]
pub async fn sync_config_to_cloud() -> Result<(), String> {
    tracing::info!("Config sync requested");

    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    if config.hospital_code.is_empty() {
        return Err("No hospital activated. Activate a license first.".to_string());
    }

    let client = crate::firestore::FirestoreClient::new_from_config()
        .await
        .map_err(|e| e.user_message())?;

    client
        .sync_config(&config.hospital_code, &config)
        .await
        .map_err(|e| e.user_message())?;

    tracing::info!("Config synced to cloud for hospital: {}", config.hospital_code);
    Ok(())
}

/// Get sync status
#[tauri::command]
pub async fn get_sync_status() -> Result<ConfigSyncStatus, String> {
    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    if config.hospital_code.is_empty() || config.gcs_credentials_path.is_none() {
        return Ok(ConfigSyncStatus::default());
    }

    let client = match crate::firestore::FirestoreClient::new_from_config().await {
        Ok(c) => c,
        Err(_) => return Ok(ConfigSyncStatus::default()),
    };

    client
        .get_sync_status(&config.hospital_code)
        .await
        .map_err(|e| e.user_message())
}

/// Detect existing setup
#[tauri::command]
pub async fn detect_existing_setup() -> Result<DetectionResult, String> {
    crate::services::detect_existing_setup()
        .await
        .map_err(|e| e.to_string())
}

/// Check prerequisites
#[tauri::command]
pub async fn check_prerequisites() -> Result<Vec<PrerequisiteStatus>, String> {
    crate::services::check_prerequisites()
        .await
        .map_err(|e| e.to_string())
}

/// Detect environment from env files alongside docker-compose.yml.
/// Parses general.env, database.env, rabbitmq.env to extract hospital info.
#[tauri::command]
pub async fn detect_environment(
    compose_path: Option<String>,
) -> Result<crate::detection::EnvDetectionResult, String> {
    let path = match compose_path {
        Some(p) if !p.is_empty() => p,
        _ => {
            // Use configured path or try to auto-detect
            let config = crate::config::load_config().unwrap_or_default();
            if !config.docker_compose_path.is_empty() {
                config.docker_compose_path
            } else {
                return Err("No docker-compose.yml path configured. Set it in Settings.".into());
            }
        }
    };

    crate::detection::detect_environment(&path).map_err(|e| e.to_string())
}

/// Apply detected environment to the current config.
/// Overwrites config with detected values; returns mismatches so UI can warn.
#[tauri::command]
pub async fn apply_detected_environment(
    compose_path: Option<String>,
) -> Result<crate::detection::ApplyResult, String> {
    let path = match compose_path {
        Some(p) if !p.is_empty() => p,
        _ => {
            let config = crate::config::load_config().unwrap_or_default();
            config.docker_compose_path
        }
    };

    if path.is_empty() {
        return Err("No docker-compose.yml path configured.".into());
    }

    let detection = crate::detection::detect_environment(&path).map_err(|e| e.to_string())?;

    let mut config = crate::config::load_config().map_err(|e| e.user_message())?;
    let result = crate::detection::apply_detected_config(&mut config, &detection);

    if !result.mismatches.is_empty() {
        for m in &result.mismatches {
            tracing::warn!(
                "Config mismatch: {} is '{}' but env file has '{}' — not overwritten, fix manually in Settings",
                m.field, m.config_value, m.detected_value
            );
        }
    }

    crate::config::save_config(&config).map_err(|e| e.user_message())?;

    tracing::info!("Applied detected environment to config ({} fields, {} mismatches)",
        result.applied.len(), result.mismatches.len());
    Ok(result)
}

/// Alert structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alert {
    pub id: String,
    pub severity: String,
    pub category: String,
    pub title: String,
    pub message: String,
    pub acknowledged: bool,
    pub created_at: String,
    pub acknowledged_at: Option<String>,
}

/// Get alerts from Firestore
#[tauri::command]
pub async fn get_alerts() -> Result<Vec<Alert>, String> {
    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    if config.hospital_code.is_empty() {
        return Ok(vec![]);
    }

    let client = match crate::firestore::FirestoreClient::new_from_config().await {
        Ok(c) => c,
        Err(_) => return Ok(vec![]),
    };

    let docs = client
        .list_alerts(&config.hospital_code)
        .await
        .map_err(|e| e.user_message())?;

    let alerts = docs
        .into_iter()
        .filter_map(|doc| {
            let id = doc.name.split('/').last()?.to_string();
            let severity = doc.fields.get("severity")
                .and_then(crate::firestore::convert::get_optional_string)
                .unwrap_or_else(|| "info".into());
            let category = doc.fields.get("category")
                .and_then(crate::firestore::convert::get_optional_string)
                .unwrap_or_else(|| "general".into());
            let title = doc.fields.get("title")
                .and_then(crate::firestore::convert::get_optional_string)
                .unwrap_or_default();
            let message = doc.fields.get("message")
                .and_then(crate::firestore::convert::get_optional_string)
                .unwrap_or_default();
            let acknowledged = doc.fields.get("acknowledged")
                .and_then(|v| crate::firestore::convert::get_bool(v).ok())
                .unwrap_or(false);
            let created_at = doc.fields.get("created_at")
                .and_then(crate::firestore::convert::get_optional_timestamp)
                .unwrap_or_default();
            let acknowledged_at = doc.fields.get("acknowledged_at")
                .and_then(crate::firestore::convert::get_optional_timestamp);

            Some(Alert {
                id,
                severity,
                category,
                title,
                message,
                acknowledged,
                created_at,
                acknowledged_at,
            })
        })
        .collect();

    Ok(alerts)
}

/// Acknowledge alert in Firestore
#[tauri::command]
pub async fn acknowledge_alert(alert_id: String) -> Result<(), String> {
    tracing::info!("Acknowledging alert: {}", alert_id);

    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    if config.hospital_code.is_empty() {
        return Err("No hospital activated. Activate a license first.".to_string());
    }

    let client = crate::firestore::FirestoreClient::new_from_config()
        .await
        .map_err(|e| e.user_message())?;

    client
        .acknowledge_alert(&config.hospital_code, &alert_id)
        .await
        .map_err(|e| e.user_message())?;

    tracing::info!("Alert {} acknowledged", alert_id);
    Ok(())
}

/// Daemon status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonStatus {
    pub running: bool,
    pub docker_connected: bool,
    pub api_port: u16,
    pub api_url: Option<String>,
    pub backup_schedule_enabled: bool,
    pub backup_interval_hours: u32,
    pub telemetry_interval_minutes: u32,
    pub service_installed: bool,
    pub service_enabled: bool,
    pub service_pid: Option<u32>,
    pub service_detail: String,
    pub platform: String,
}

/// Get daemon status — checks Docker, service status, probes daemon API
#[tauri::command]
pub async fn get_daemon_status() -> Result<DaemonStatus, String> {
    let docker_connected = match bollard::Docker::connect_with_local_defaults() {
        Ok(docker) => docker.ping().await.is_ok(),
        Err(_) => false,
    };

    let config = crate::config::load_config().unwrap_or_default();
    let daemon_cfg = config.daemon.unwrap_or_default();
    let port = daemon_cfg.port;

    // Probe the daemon API to see if it's actually running
    let api_url = format!("http://127.0.0.1:{}/api/health", port);
    let running = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(client) => client.get(&api_url).send().await.is_ok(),
        Err(_) => false,
    };

    // Query system service status
    let svc_status = crate::platform::service_status().await.unwrap_or(
        crate::platform::ServiceStatus {
            installed: false,
            running: false,
            enabled: false,
            pid: None,
            detail: "Could not query service status".to_string(),
        }
    );

    Ok(DaemonStatus {
        running: running || svc_status.running,
        docker_connected,
        api_port: port,
        api_url: if running { Some(api_url) } else { None },
        backup_schedule_enabled: daemon_cfg.backup_schedule.enabled,
        backup_interval_hours: daemon_cfg.backup_schedule.interval_hours,
        telemetry_interval_minutes: daemon_cfg.telemetry_interval_minutes,
        service_installed: svc_status.installed,
        service_enabled: svc_status.enabled,
        service_pid: svc_status.pid,
        service_detail: svc_status.detail,
        platform: crate::platform::platform_name().to_string(),
    })
}

/// Install daemon as system service
#[tauri::command]
pub async fn install_daemon_service() -> Result<String, String> {
    tracing::info!("Installing daemon service");
    let result = crate::platform::install_service().await?;
    tracing::info!("Install result: {}", result.message);
    Ok(result.message)
}

/// Uninstall daemon system service
#[tauri::command]
pub async fn uninstall_daemon_service() -> Result<String, String> {
    tracing::info!("Uninstalling daemon service");
    let result = crate::platform::uninstall_service().await?;
    Ok(result.message)
}

/// Start daemon service
#[tauri::command]
pub async fn start_daemon() -> Result<String, String> {
    tracing::info!("Starting daemon service");
    let result = crate::platform::start_service().await?;
    Ok(result.message)
}

/// Stop daemon service
#[tauri::command]
pub async fn stop_daemon() -> Result<String, String> {
    tracing::info!("Stopping daemon service");
    let result = crate::platform::stop_service().await?;
    Ok(result.message)
}

/// Restart daemon service (stop + start)
#[tauri::command]
pub async fn restart_daemon() -> Result<String, String> {
    tracing::info!("Restarting daemon service");
    let _ = crate::platform::stop_service().await;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let result = crate::platform::start_service().await?;
    Ok(result.message)
}

/// Log error from frontend
#[tauri::command]
pub async fn log_error(command: String, message: String, timestamp: i64) -> Result<(), String> {
    tracing::error!(
        command = %command,
        timestamp = %timestamp,
        "Frontend error: {}",
        message
    );
    Ok(())
}

/// Get container logs
#[tauri::command]
pub async fn get_container_logs(
    container_name: String,
    tail: Option<u64>,
    since: Option<i64>,
    until: Option<i64>,
) -> Result<String, String> {
    crate::services::get_container_logs(&container_name, tail.unwrap_or(200), since, until)
        .await
        .map_err(|e| e.to_string())
}

// ── Log File Reader ──────────────────────────────────────────────────────────

/// Get known log source directories
#[tauri::command]
pub fn get_log_sources() -> Vec<crate::logs::LogSource> {
    crate::logs::get_known_log_paths()
}

/// List log files under a path (or aggregate all known sources if no path given)
#[tauri::command]
pub fn list_log_files(path: Option<String>) -> Result<Vec<crate::logs::LogFileInfo>, String> {
    if let Some(p) = path {
        crate::logs::list_log_files(&p).map_err(|e| e.to_string())
    } else {
        // Aggregate all known sources
        let sources = crate::logs::get_known_log_paths();
        let mut all_files = Vec::new();
        for source in &sources {
            if let Ok(files) = crate::logs::list_log_files(&source.path) {
                all_files.extend(files);
            }
        }
        // Sort by modified_at descending
        all_files.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
        Ok(all_files)
    }
}

/// Read a log file with optional tail or offset+limit pagination
#[tauri::command]
pub fn read_log_file(
    path: String,
    tail: Option<usize>,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<crate::logs::LogFileContent, String> {
    crate::logs::read_log_file(&path, tail, offset, limit).map_err(|e| e.to_string())
}

/// Get a telemetry snapshot (current system + service metrics)
#[tauri::command]
pub async fn get_telemetry_snapshot() -> Result<TelemetrySnapshot, String> {
    crate::telemetry::collect_snapshot()
        .await
        .map_err(|e| e.to_string())
}

// ── Credentials File Management ──────────────────────────────────────────────

/// Status of the GCS credentials file at the designated path
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialsStatus {
    pub exists: bool,
    pub path: String,
}

/// Designated path for the GCS credentials file (inside config_dir)
fn credentials_path() -> std::path::PathBuf {
    crate::config::config_dir().join("gcs-credentials.json")
}

/// Check if the credentials file exists at the designated path
#[tauri::command]
pub async fn check_credentials_file() -> Result<CredentialsStatus, String> {
    let path = credentials_path();
    Ok(CredentialsStatus {
        exists: path.exists(),
        path: path.display().to_string(),
    })
}

/// Import a credentials file from a source path — copies it to the designated location
/// and updates config.gcs_credentials_path
#[tauri::command]
pub async fn import_credentials_file(source_path: String) -> Result<(), String> {
    let source = std::path::Path::new(&source_path);
    if !source.exists() {
        return Err(format!("File not found: {}", source_path));
    }

    // Validate it's valid JSON with expected fields
    let content = std::fs::read_to_string(source)
        .map_err(|e| format!("Cannot read file: {}", e))?;
    validate_credentials_json(&content)?;

    let dest = credentials_path();
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create config directory: {}", e))?;
    }

    std::fs::copy(source, &dest)
        .map_err(|e| format!("Failed to copy credentials file: {}", e))?;

    // Update config to point to the designated path
    let mut config = crate::config::load_config().map_err(|e| e.user_message())?;
    config.gcs_credentials_path = Some(dest.display().to_string());
    crate::config::save_config(&config).map_err(|e| e.user_message())?;

    tracing::info!("Credentials file imported from {}", source_path);
    Ok(())
}

/// Save credentials JSON content directly (e.g. pasted from a message)
/// and update config.gcs_credentials_path
#[tauri::command]
pub async fn save_credentials_content(content: String) -> Result<(), String> {
    validate_credentials_json(&content)?;

    let dest = credentials_path();
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create config directory: {}", e))?;
    }

    std::fs::write(&dest, &content)
        .map_err(|e| format!("Failed to write credentials file: {}", e))?;

    // Update config to point to the designated path
    let mut config = crate::config::load_config().map_err(|e| e.user_message())?;
    config.gcs_credentials_path = Some(dest.display().to_string());
    crate::config::save_config(&config).map_err(|e| e.user_message())?;

    tracing::info!("Credentials file saved from pasted content");
    Ok(())
}

/// Validate that JSON content looks like a GCP service account key
fn validate_credentials_json(content: &str) -> Result<(), String> {
    let json: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| format!("Invalid JSON: {}", e))?;

    let obj = json.as_object().ok_or("Credentials must be a JSON object")?;

    if !obj.contains_key("type") || !obj.contains_key("project_id") {
        return Err(
            "This doesn't look like a GCP service account key. Expected fields: type, project_id"
                .to_string(),
        );
    }

    let cred_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if cred_type != "service_account" {
        return Err(format!(
            "Expected type \"service_account\", got \"{}\". Make sure this is a GCP service account key.",
            cred_type
        ));
    }

    Ok(())
}

// ── Setup Wizard Commands ────────────────────────────────────────────────────

/// Known Puru databases to create during setup
const PURU_DATABASES: &[&str] = &[
    "puru_im",
    "puru_has",
    "puru_med",
    "puru_dicom",
    "puru_path",
    "puru_bridge",
    "puru_auth",
    "puru_gh",
];

/// Docker images to pull from GCR
const PURU_IMAGES: &[&str] = &[
    "gcr.io/puru-255206/puru-xenon:latest",
    "gcr.io/puru-255206/puru-has:latest",
    "gcr.io/puru-255206/puru-pacs:latest",
    "gcr.io/puru-255206/puru-argon:latest",
    "gcr.io/puru-255206/puru-comm:latest",
    "gcr.io/puru-255206/puru-realtime:latest",
    "gcr.io/puru-255206/puru-neon:latest",
    "gcr.io/puru-255206/puru-bridge:latest",
    "gcr.io/puru-255206/puru-hydrogen:latest",
];

/// Step 1: Verify all prerequisites are installed
#[tauri::command]
pub async fn setup_check_prerequisites() -> Result<(), String> {
    tracing::info!("Setup: checking prerequisites");

    let prereqs = crate::services::check_prerequisites()
        .await
        .map_err(|e| e.user_message())?;

    let missing: Vec<&str> = prereqs
        .iter()
        .filter(|p| !p.installed)
        .map(|p| p.name.as_str())
        .collect();

    if !missing.is_empty() {
        return Err(format!(
            "Missing prerequisites: {}. Install them before proceeding.",
            missing.join(", ")
        ));
    }

    tracing::info!("Setup: all prerequisites met");
    Ok(())
}

/// Step 2: Create MySQL databases for all Puru services
#[tauri::command]
pub async fn setup_create_databases() -> Result<(), String> {
    tracing::info!("Setup: creating MySQL databases");

    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    if config.mysql_password.is_empty() {
        return Err("MySQL password not configured. Set it in Settings first.".to_string());
    }

    // Build SQL to create all databases with utf8mb4 charset
    let sql: String = PURU_DATABASES
        .iter()
        .map(|db| {
            format!(
                "CREATE DATABASE IF NOT EXISTS `{}` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;",
                db
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let output = tokio::process::Command::new("mysql")
        .env("MYSQL_PWD", &config.mysql_password)
        .args([
            &format!("-h{}", config.mysql_host),
            &format!("-P{}", config.mysql_port),
            &format!("-u{}", config.mysql_user),
            "-e",
            &sql,
        ])
        .output()
        .await
        .map_err(|e| format!("Failed to run mysql: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Database creation failed: {}", stderr.trim()));
    }

    tracing::info!("Setup: created {} databases", PURU_DATABASES.len());
    Ok(())
}

/// Step 3: Configure RabbitMQ vhost and user for Puru services
#[tauri::command]
pub async fn setup_configure_rabbitmq() -> Result<(), String> {
    tracing::info!("Setup: configuring RabbitMQ");

    // Try Docker exec first (most common setup — RabbitMQ runs in container)
    let docker_exec = tokio::process::Command::new("docker")
        .args([
            "exec", "rabbitmq", "rabbitmqctl", "add_vhost", "puru",
        ])
        .output()
        .await;

    let use_docker = match &docker_exec {
        Ok(out) => out.status.success() || String::from_utf8_lossy(&out.stderr).contains("already exists"),
        Err(_) => false,
    };

    if use_docker {
        // Set up user and permissions via docker exec
        let commands: &[&[&str]] = &[
            &["exec", "rabbitmq", "rabbitmqctl", "add_user", "puru", "puru123"],
            &["exec", "rabbitmq", "rabbitmqctl", "set_permissions", "-p", "puru", "puru", ".*", ".*", ".*"],
            &["exec", "rabbitmq", "rabbitmqctl", "set_user_tags", "puru", "administrator"],
        ];

        for args in commands {
            let output = tokio::process::Command::new("docker")
                .args(*args)
                .output()
                .await
                .map_err(|e| format!("Failed to configure RabbitMQ: {}", e))?;

            // Ignore "already exists" errors — idempotent setup
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if !stderr.contains("already exists") && !stderr.contains("already_exists") {
                    tracing::warn!("RabbitMQ command warning: {}", stderr.trim());
                }
            }
        }
    } else {
        // Fallback: try host-installed rabbitmqctl
        let vhost_output = tokio::process::Command::new("rabbitmqctl")
            .args(["add_vhost", "puru"])
            .output()
            .await
            .map_err(|e| format!("Cannot reach RabbitMQ. Is it running? ({})", e))?;

        if !vhost_output.status.success() {
            let stderr = String::from_utf8_lossy(&vhost_output.stderr);
            if !stderr.contains("already exists") && !stderr.contains("already_exists") {
                return Err(format!("RabbitMQ vhost creation failed: {}", stderr.trim()));
            }
        }

        let user_commands: &[&[&str]] = &[
            &["add_user", "puru", "puru123"],
            &["set_permissions", "-p", "puru", "puru", ".*", ".*", ".*"],
            &["set_user_tags", "puru", "administrator"],
        ];

        for args in user_commands {
            let output = tokio::process::Command::new("rabbitmqctl")
                .args(*args)
                .output()
                .await
                .map_err(|e| format!("Failed to configure RabbitMQ: {}", e))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if !stderr.contains("already exists") && !stderr.contains("already_exists") {
                    tracing::warn!("RabbitMQ command warning: {}", stderr.trim());
                }
            }
        }
    }

    tracing::info!("Setup: RabbitMQ configured (vhost=puru, user=puru)");
    Ok(())
}

/// Step 4: Generate docker-compose.yml and save config
#[tauri::command]
pub async fn setup_generate_config() -> Result<(), String> {
    tracing::info!("Setup: generating configuration files");

    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    let compose_path = std::path::Path::new(&config.docker_compose_path);

    // Ensure parent directory exists
    if let Some(parent) = compose_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create directory {}: {}", parent.display(), e))?;
    }

    // Generate docker-compose.yml with all Puru services
    let compose_content = generate_docker_compose(&config);
    std::fs::write(compose_path, &compose_content)
        .map_err(|e| format!("Failed to write docker-compose.yml: {}", e))?;

    tracing::info!("Setup: wrote docker-compose.yml to {}", config.docker_compose_path);
    Ok(())
}

/// Generate a docker-compose.yml for the Puru stack
fn generate_docker_compose(config: &NucleusConfig) -> String {
    format!(
        r#"version: "3.8"

# Puru Technologies — Docker Compose
# Generated by puru-nucleus
# Hospital: {hospital_code}

services:
  database:
    image: mysql:8.0
    container_name: purusql
    restart: unless-stopped
    environment:
      MYSQL_ROOT_PASSWORD: {mysql_password}
    ports:
      - "{mysql_port}:3306"
    volumes:
      - mysql_data:/var/lib/mysql
    command: --character-set-server=utf8mb4 --collation-server=utf8mb4_unicode_ci

  rabbitmq:
    image: rabbitmq:3.12-management
    container_name: rabbitmq
    restart: unless-stopped
    ports:
      - "5672:5672"
      - "15672:15672"
    volumes:
      - rabbitmq_data:/var/lib/rabbitmq

  backend:
    image: gcr.io/puru-255206/puru-xenon:latest
    container_name: backend
    restart: unless-stopped
    depends_on:
      - database
      - rabbitmq
    environment:
      SPRING_DATASOURCE_URL: jdbc:mysql://database:3306/puru_im?useSSL=false&allowPublicKeyRetrieval=true
      SPRING_DATASOURCE_USERNAME: {mysql_user}
      SPRING_DATASOURCE_PASSWORD: {mysql_password}
      SPRING_RABBITMQ_HOST: rabbitmq
      SPRING_RABBITMQ_VIRTUAL_HOST: puru
      SPRING_RABBITMQ_USERNAME: puru
      SPRING_RABBITMQ_PASSWORD: puru123
    ports:
      - "8081:8081"
    network_mode: host

  has:
    image: gcr.io/puru-255206/puru-has:latest
    container_name: has
    restart: unless-stopped
    depends_on:
      - database
      - rabbitmq
    environment:
      SPRING_DATASOURCE_URL: jdbc:mysql://database:3306/puru_has?useSSL=false&allowPublicKeyRetrieval=true
      SPRING_DATASOURCE_USERNAME: {mysql_user}
      SPRING_DATASOURCE_PASSWORD: {mysql_password}
      SPRING_RABBITMQ_HOST: rabbitmq
      SPRING_RABBITMQ_VIRTUAL_HOST: puru
      SPRING_RABBITMQ_USERNAME: puru
      SPRING_RABBITMQ_PASSWORD: puru123
    ports:
      - "8082:8082"
    network_mode: host

  pacs:
    image: gcr.io/puru-255206/puru-pacs:latest
    container_name: pacs
    restart: unless-stopped
    depends_on:
      - database
    environment:
      SPRING_DATASOURCE_URL: jdbc:mysql://database:3306/puru_dicom?useSSL=false&allowPublicKeyRetrieval=true
      SPRING_DATASOURCE_USERNAME: {mysql_user}
      SPRING_DATASOURCE_PASSWORD: {mysql_password}
    ports:
      - "8083:8083"
    network_mode: host

  pathology:
    image: gcr.io/puru-255206/puru-argon:latest
    container_name: pathology
    restart: unless-stopped
    depends_on:
      - database
      - rabbitmq
    environment:
      SPRING_DATASOURCE_URL: jdbc:mysql://database:3306/puru_path?useSSL=false&allowPublicKeyRetrieval=true
      SPRING_DATASOURCE_USERNAME: {mysql_user}
      SPRING_DATASOURCE_PASSWORD: {mysql_password}
      SPRING_RABBITMQ_HOST: rabbitmq
      SPRING_RABBITMQ_VIRTUAL_HOST: puru
      SPRING_RABBITMQ_USERNAME: puru
      SPRING_RABBITMQ_PASSWORD: puru123
    ports:
      - "8084:8084"
    network_mode: host

  comm_server:
    image: gcr.io/puru-255206/puru-comm:latest
    container_name: comm_server
    restart: unless-stopped
    depends_on:
      - database
      - rabbitmq
    environment:
      SPRING_DATASOURCE_URL: jdbc:mysql://database:3306/puru_im?useSSL=false&allowPublicKeyRetrieval=true
      SPRING_DATASOURCE_USERNAME: {mysql_user}
      SPRING_DATASOURCE_PASSWORD: {mysql_password}
      SPRING_RABBITMQ_HOST: rabbitmq
      SPRING_RABBITMQ_VIRTUAL_HOST: puru
      SPRING_RABBITMQ_USERNAME: puru
      SPRING_RABBITMQ_PASSWORD: puru123
    ports:
      - "8085:8085"
    network_mode: host

  realtime:
    image: gcr.io/puru-255206/puru-realtime:latest
    container_name: realtime
    restart: unless-stopped
    depends_on:
      - database
      - rabbitmq
    environment:
      SPRING_DATASOURCE_URL: jdbc:mysql://database:3306/puru_im?useSSL=false&allowPublicKeyRetrieval=true
      SPRING_DATASOURCE_USERNAME: {mysql_user}
      SPRING_DATASOURCE_PASSWORD: {mysql_password}
      SPRING_RABBITMQ_HOST: rabbitmq
      SPRING_RABBITMQ_VIRTUAL_HOST: puru
      SPRING_RABBITMQ_USERNAME: puru
      SPRING_RABBITMQ_PASSWORD: puru123
    ports:
      - "8086:8086"
    network_mode: host

  medical:
    image: gcr.io/puru-255206/puru-neon:latest
    container_name: medical
    restart: unless-stopped
    depends_on:
      - database
    environment:
      SPRING_DATASOURCE_URL: jdbc:mysql://database:3306/puru_med?useSSL=false&allowPublicKeyRetrieval=true
      SPRING_DATASOURCE_USERNAME: {mysql_user}
      SPRING_DATASOURCE_PASSWORD: {mysql_password}
    ports:
      - "8087:8087"
    network_mode: host

  bridge:
    image: gcr.io/puru-255206/puru-bridge:latest
    container_name: bridge
    restart: unless-stopped
    depends_on:
      - database
    environment:
      SPRING_DATASOURCE_URL: jdbc:mysql://database:3306/puru_bridge?useSSL=false&allowPublicKeyRetrieval=true
      SPRING_DATASOURCE_USERNAME: {mysql_user}
      SPRING_DATASOURCE_PASSWORD: {mysql_password}
    ports:
      - "8094:8094"
    network_mode: host

  frontend:
    image: gcr.io/puru-255206/puru-hydrogen:latest
    container_name: frontend
    restart: unless-stopped
    ports:
      - "80:80"

volumes:
  mysql_data:
  rabbitmq_data:
"#,
        hospital_code = config.hospital_code,
        mysql_password = config.mysql_password,
        mysql_port = config.mysql_port,
        mysql_user = config.mysql_user,
    )
}

/// Step 5: Pull Docker images from GCR
#[tauri::command]
pub async fn setup_pull_images() -> Result<(), String> {
    tracing::info!("Setup: pulling Docker images");

    for image in PURU_IMAGES {
        tracing::info!("Pulling image: {}", image);

        let output = tokio::process::Command::new("docker")
            .args(["pull", image])
            .output()
            .await
            .map_err(|e| format!("Failed to pull {}: {}", image, e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Failed to pull {}: {}", image, stderr.trim()));
        }
    }

    tracing::info!("Setup: pulled {} images", PURU_IMAGES.len());
    Ok(())
}

/// Step 6: Start all services via docker compose
#[tauri::command]
pub async fn setup_start_services() -> Result<(), String> {
    tracing::info!("Setup: starting services");

    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    let compose_path = &config.docker_compose_path;

    if !std::path::Path::new(compose_path).exists() {
        return Err(format!(
            "docker-compose.yml not found at {}. Run 'Generate configuration' first.",
            compose_path
        ));
    }

    // Try docker compose (V2) first, fallback to docker-compose (V1)
    let output = tokio::process::Command::new("docker")
        .args(["compose", "-f", compose_path, "up", "-d"])
        .output()
        .await;

    let result = match output {
        Ok(out) if out.status.success() => Ok(()),
        _ => {
            // Fallback to V1
            let output = tokio::process::Command::new("docker-compose")
                .args(["-f", compose_path, "up", "-d"])
                .output()
                .await
                .map_err(|e| format!("Failed to start services: {}", e))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(format!("Failed to start services: {}", stderr.trim()))
            } else {
                Ok(())
            }
        }
    };

    result?;

    // Wait a moment for containers to initialize
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    tracing::info!("Setup: services started");
    Ok(())
}

/// Step 7: Health check — verify all services are running
#[tauri::command]
pub async fn setup_health_check() -> Result<(), String> {
    tracing::info!("Setup: running health check");

    // Give services time to start up (Spring Boot takes a while)
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    let services = crate::services::get_services()
        .await
        .map_err(|e| e.user_message())?;

    if services.is_empty() {
        return Err("No Puru services detected. Ensure Docker is running and services started.".to_string());
    }

    let running_count = services
        .iter()
        .filter(|s| s.status == crate::services::ServiceStatus::Running)
        .count();

    let total = services.len();

    if running_count == 0 {
        return Err("No services are running. Check Docker logs for errors.".to_string());
    }

    if running_count < total {
        let stopped: Vec<&str> = services
            .iter()
            .filter(|s| s.status != crate::services::ServiceStatus::Running)
            .map(|s| s.name.as_str())
            .collect();
        tracing::warn!(
            "Health check: {}/{} services running. Stopped: {}",
            running_count,
            total,
            stopped.join(", ")
        );
        // Non-fatal — some services may still be starting
    }

    tracing::info!("Setup: health check passed ({}/{} running)", running_count, total);
    Ok(())
}

/// Step 8: Configure backups — ensure backup directory exists and config is set
#[tauri::command]
pub async fn setup_configure_backups() -> Result<(), String> {
    tracing::info!("Setup: configuring backups");

    let mut config = crate::config::load_config().map_err(|e| e.user_message())?;

    // Ensure backup is enabled
    config.backup_enabled = true;

    // Ensure the backups directory exists
    let backups_dir = crate::config::config_dir().join("backups");
    std::fs::create_dir_all(&backups_dir)
        .map_err(|e| format!("Failed to create backups directory: {}", e))?;

    crate::config::save_config(&config).map_err(|e| e.user_message())?;

    // Verify MySQL is accessible by running a simple query
    if !config.mysql_password.is_empty() {
        let output = tokio::process::Command::new("mysql")
            .env("MYSQL_PWD", &config.mysql_password)
            .args([
                &format!("-h{}", config.mysql_host),
                &format!("-P{}", config.mysql_port),
                &format!("-u{}", config.mysql_user),
                "-e",
                "SELECT 1",
            ])
            .output()
            .await;

        match output {
            Ok(out) if out.status.success() => {
                tracing::info!("Setup: MySQL connectivity verified for backups");
            }
            _ => {
                tracing::warn!("Setup: MySQL not reachable — backups will fail until database settings are corrected");
            }
        }
    }

    tracing::info!("Setup: backups configured");
    Ok(())
}

/// Step 9: Register puru-nucleus as a system service (daemon)
#[tauri::command]
pub async fn setup_install_daemon() -> Result<(), String> {
    tracing::info!("Setup: installing daemon service ({})", crate::platform::platform_name());

    let result = crate::platform::install_service().await?;
    tracing::info!("Setup: {}", result.message);

    Ok(())
}

/// Step 10: Configure HTTPS — generate CA + cert via mkcert, write nginx config
#[tauri::command]
pub async fn setup_tls() -> Result<(), String> {
    tracing::info!("Setup: configuring TLS/HTTPS");

    let config = crate::config::load_config().map_err(|e| e.user_message())?;

    if config.server_ip.is_empty() {
        return Err("Server IP not configured. Set it in the infrastructure step.".into());
    }

    // Check if mkcert is available — if not, skip gracefully (TLS is optional)
    match crate::tls::setup_tls(&config).await {
        Ok(result) => {
            tracing::info!("Setup: TLS configured — {}", result.message);

            // Write nginx HTTPS config
            let nginx_config = crate::tls::generate_nginx_https_config(&config.server_ip);
            let nginx_path = std::path::Path::new("/etc/nginx/conf.d/puru-https.conf");
            if let Some(parent) = nginx_path.parent() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
            match tokio::fs::write(nginx_path, &nginx_config).await {
                Ok(_) => tracing::info!("Setup: nginx HTTPS config written to {:?}", nginx_path),
                Err(e) => tracing::warn!("Setup: could not write nginx config: {} (manual setup needed)", e),
            }

            // Generate and save client setup script for download via file explorer
            let script = crate::tls::generate_client_setup_script(&config.server_ip, 81, 443);
            let compose_path = std::path::PathBuf::from(&config.docker_compose_path);
            let data_root = compose_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("/home/puru"))
                .join("puru-data");
            let script_path = data_root.join("uploaded-document").join("puru-secure-setup.bat");
            match tokio::fs::write(&script_path, &script).await {
                Ok(_) => tracing::info!("Setup: client setup script saved to {:?}", script_path),
                Err(e) => tracing::warn!("Setup: could not write client script: {}", e),
            }

            // Try to reload nginx
            let reload = tokio::process::Command::new("nginx")
                .arg("-s").arg("reload")
                .output().await;
            match reload {
                Ok(o) if o.status.success() => tracing::info!("Setup: nginx reloaded"),
                _ => tracing::warn!("Setup: nginx reload failed (restart manually)"),
            }

            Ok(())
        }
        Err(e) => {
            // TLS setup failed — log but don't block the overall setup
            let msg = format!("TLS setup skipped: {}. Voice calls and screen sharing will require manual Chrome flag setup on client machines.", e);
            tracing::warn!("Setup: {}", msg);
            // We return Ok to not block the wizard — TLS is optional
            Ok(())
        }
    }
}

// ── Release management ───────────────────────────────────────────────────────

/// Check if a nucleus update is available
#[tauri::command]
pub async fn check_nucleus_update() -> Result<NucleusUpdateInfo, String> {
    let cfg = crate::config::load_config().map_err(|e| e.to_string())?;
    crate::releases::check_nucleus_update(&cfg.release_channel)
        .await
        .map_err(|e| e.to_string())
}

/// Check for updates across all running services
#[tauri::command]
pub async fn check_service_updates() -> Result<Vec<ServiceUpdateInfo>, String> {
    crate::releases::check_service_updates()
        .await
        .map_err(|e| e.to_string())
}

/// Download the nucleus installer for the current platform
#[tauri::command]
pub async fn download_nucleus_update() -> Result<DownloadResult, String> {
    let cfg = crate::config::load_config().map_err(|e| e.to_string())?;
    crate::releases::download_nucleus_update(&cfg.release_channel)
        .await
        .map_err(|e| e.to_string())
}

/// Download a specific service JAR version
#[tauri::command]
pub async fn download_service_jar(
    service_name: String,
    version: String,
) -> Result<DownloadResult, String> {
    crate::releases::download_service_jar(&service_name, &version)
        .await
        .map_err(|e| e.to_string())
}

/// List available versions for a service
#[tauri::command]
pub async fn list_service_versions(
    service_name: String,
) -> Result<ServiceManifest, String> {
    crate::releases::list_service_versions(&service_name)
        .await
        .map_err(|e| e.to_string())
}

// ── Native JAR deployment ────────────────────────────────────────────────────

/// Pull latest JARs from GCS for specified services
#[tauri::command]
pub async fn pull_jars(services: Vec<String>) -> Result<PullAllResult, String> {
    crate::releases::pull_all_with_jres(&services)
        .await
        .map_err(|e| e.to_string())
}

/// Pull latest JAR for a single service
#[tauri::command]
pub async fn pull_single_jar(service_name: String) -> Result<JarPullResult, String> {
    crate::releases::pull_jar(&service_name)
        .await
        .map_err(|e| e.to_string())
}

/// Check which services have newer JARs available
#[tauri::command]
pub async fn check_jar_updates(services: Vec<String>) -> Result<Vec<JarUpdateCheck>, String> {
    crate::releases::check_jar_updates_available(&services)
        .await
        .map_err(|e| e.to_string())
}

/// Get current deployment mode
#[tauri::command]
pub async fn get_deployment_mode() -> Result<String, String> {
    let cfg = crate::config::load_config().map_err(|e| e.to_string())?;
    Ok(format!("{:?}", cfg.deployment_mode))
}

// ── Docker update management ─────────────────────────────────────────────────

/// Update a Docker service to a new image tag
#[tauri::command]
pub async fn update_docker_service(
    service_name: String,
    new_tag: String,
) -> Result<DockerUpdateResult, String> {
    crate::docker_update::update_service(&service_name, &new_tag)
        .await
        .map_err(|e| e.to_string())
}

/// Rollback a Docker service to its previous version
#[tauri::command]
pub async fn rollback_docker_service(
    service_name: String,
) -> Result<DockerUpdateResult, String> {
    crate::docker_update::rollback_service(&service_name)
        .await
        .map_err(|e| e.to_string())
}

/// Get Docker update history
#[tauri::command]
pub async fn get_update_history() -> Result<Vec<UpdateRecord>, String> {
    crate::docker_update::get_update_history()
        .await
        .map_err(|e| e.to_string())
}

// ── Remote shell ─────────────────────────────────────────────────────────────

/// Execute an allowlisted shell command
#[tauri::command]
pub async fn execute_shell_command(
    command: String,
) -> Result<ShellResult, String> {
    crate::remote_shell::execute_command(&command)
        .await
        .map_err(|e| e.to_string())
}

/// Get the shell audit log
#[tauri::command]
pub async fn get_shell_audit_log() -> Result<Vec<ShellAuditEntry>, String> {
    crate::remote_shell::get_audit_log()
        .await
        .map_err(|e| e.to_string())
}

/// Get the list of allowed shell commands
#[tauri::command]
pub async fn get_allowed_shell_commands() -> Result<Vec<String>, String> {
    Ok(crate::remote_shell::get_allowed_commands())
}

// ── Messaging ────────────────────────────────────────────────────────────────

/// Get messages from the hospital inbox
#[tauri::command]
pub async fn get_messages() -> Result<Vec<HospitalMessage>, String> {
    crate::messaging::service::get_messages(100)
        .await
        .map_err(|e| e.user_message())
}

/// Get unread message count
#[tauri::command]
pub async fn get_unread_count() -> Result<u32, String> {
    crate::messaging::service::get_unread_count()
        .await
        .map_err(|e| e.user_message())
}

/// Mark a message as read
#[tauri::command]
pub async fn mark_message_read(message_id: String) -> Result<(), String> {
    crate::messaging::service::mark_message_read(&message_id)
        .await
        .map_err(|e| e.user_message())
}

/// Download a message attachment from GCS
#[tauri::command]
pub async fn download_attachment(
    message_id: String,
    attachment_index: u32,
    file_url: String,
) -> Result<MsgDownloadResult, String> {
    crate::messaging::files::download_attachment(&message_id, attachment_index, &file_url)
        .await
        .map_err(|e| e.user_message())
}

/// Apply a downloaded config file
#[tauri::command]
pub async fn apply_config_file(file_path: String) -> Result<FileActionResult, String> {
    crate::messaging::file_actions::apply_config_file(&file_path)
        .await
        .map_err(|e| e.user_message())
}

/// Install a downloaded certificate file
#[tauri::command]
pub async fn install_certificate(file_path: String) -> Result<FileActionResult, String> {
    crate::messaging::file_actions::install_certificate(&file_path)
        .await
        .map_err(|e| e.user_message())
}

// ── LAN Backup ──────────────────────────────────────────────────────────────

/// Validate a LAN backup path — checks it exists, is a directory, and is writable
#[tauri::command]
pub async fn validate_lan_path(path: String) -> Result<(), String> {
    let p = std::path::Path::new(&path);

    if !p.exists() {
        return Err("Path does not exist".into());
    }
    if !p.is_dir() {
        return Err("Path is not a directory".into());
    }

    // Write + delete probe file to verify write access
    let probe = p.join(".puru-probe");
    std::fs::write(&probe, b"puru-nucleus write test")
        .map_err(|e| format!("Path is not writable: {}", e))?;
    let _ = std::fs::remove_file(&probe);

    Ok(())
}

/// Ship binlog files to LAN network share
#[tauri::command]
pub async fn ship_binlogs_lan() -> Result<crate::backup::binlog::BinlogShipResult, String> {
    let license = crate::licensing::load_license()
        .map_err(|e| e.user_message())?
        .ok_or_else(|| "No license found. Activate a license first.".to_string())?;

    crate::backup::binlog::ship_binlogs_to_lan(&license)
        .await
        .map_err(|e| e.user_message())
}

/// Get binlog shipping status
#[tauri::command]
pub async fn get_binlog_status() -> Result<crate::backup::binlog::BinlogStatus, String> {
    crate::backup::binlog::get_binlog_status()
        .await
        .map_err(|e| e.user_message())
}

// ── Network ─────────────────────────────────────────────────────────────────

/// Quick connectivity + latency check
#[tauri::command]
pub async fn check_network() -> Result<crate::network::NetworkStatus, String> {
    crate::network::check_network()
        .await
        .map_err(|e| e.user_message())
}

/// Full speed test (download + upload)
#[tauri::command]
pub async fn run_speed_test() -> Result<crate::network::SpeedTestResult, String> {
    crate::network::run_speed_test()
        .await
        .map_err(|e| e.user_message())
}

// ── Compose Template ─────────────────────────────────────────────────────────

/// Download compose fragment files from GCS (skips files that already exist)
#[tauri::command]
pub async fn download_compose_template() -> Result<FragmentDownloadResult, String> {
    crate::compose_template::download_fragments()
        .await
        .map_err(|e| e.user_message())
}

/// Read the local compose file content for editing
#[tauri::command]
pub async fn get_compose_content() -> Result<String, String> {
    crate::compose_template::read_compose_file()
        .await
        .map_err(|e| e.user_message())
}

/// Substitute template variables into compose content
#[tauri::command]
pub async fn substitute_compose_variables(
    content: String,
    variables: TemplateVariables,
) -> Result<String, String> {
    Ok(crate::compose_template::substitute_variables(&content, &variables))
}

/// Save compose content to disk
#[tauri::command]
pub async fn save_compose_content(content: String) -> Result<String, String> {
    crate::compose_template::save_compose_file(&content)
        .await
        .map_err(|e| e.user_message())
}

/// Upload finalized compose file to GCS
#[tauri::command]
pub async fn upload_compose_to_cloud() -> Result<ComposeUploadResult, String> {
    let cfg = crate::config::load_config().map_err(|e| e.user_message())?;
    if cfg.hospital_code.is_empty() {
        return Err("Hospital code not configured. Set it in Settings first.".into());
    }
    crate::compose_template::upload_compose_to_gcs(&cfg.hospital_code)
        .await
        .map_err(|e| e.user_message())
}

/// Fetch service modules config from Firestore (hospital/{code}.modules)
#[tauri::command]
pub async fn get_service_modules() -> Result<ServiceModules, String> {
    let cfg = crate::config::load_config().map_err(|e| e.user_message())?;
    if cfg.hospital_code.is_empty() {
        return Err("Hospital code not configured. Set it in Settings first.".into());
    }
    let client = crate::firestore::FirestoreClient::new_from_config()
        .await
        .map_err(|e| e.user_message())?;
    client
        .fetch_modules(&cfg.hospital_code)
        .await
        .map_err(|e| e.user_message())
}

/// Assemble docker-compose.yml from fragment files based on enabled modules
#[tauri::command]
pub async fn assemble_compose_file(
    modules: ServiceModules,
) -> Result<String, String> {
    crate::compose_template::assemble_compose(&modules)
        .map_err(|e| e.user_message())
}

// ── Env File Templates ───────────────────────────────────────────────────────

/// Download env file templates from GCS (skips files that already exist)
#[tauri::command]
pub async fn download_env_templates() -> Result<EnvDownloadResult, String> {
    crate::compose_template::download_env_templates()
        .await
        .map_err(|e| e.user_message())
}

/// Read all local env files
#[tauri::command]
pub async fn get_env_files() -> Result<Vec<EnvFileEntry>, String> {
    crate::compose_template::read_env_files()
        .await
        .map_err(|e| e.user_message())
}

/// Save a single env file by name
#[tauri::command]
pub async fn save_env_file(name: String, content: String) -> Result<String, String> {
    crate::compose_template::save_env_file(&name, &content)
        .await
        .map_err(|e| e.user_message())
}

/// Upload all env files to GCS
#[tauri::command]
pub async fn upload_env_files_to_cloud() -> Result<EnvUploadResult, String> {
    let cfg = crate::config::load_config().map_err(|e| e.user_message())?;
    if cfg.hospital_code.is_empty() {
        return Err("Hospital code not configured. Set it in Settings first.".into());
    }
    crate::compose_template::upload_env_files_to_gcs(&cfg.hospital_code)
        .await
        .map_err(|e| e.user_message())
}

// ============================================================
// TLS / Certificate Management
// ============================================================

/// Get TLS status
#[tauri::command]
pub async fn get_tls_status() -> Result<crate::tls::TlsStatus, String> {
    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    Ok(crate::tls::get_tls_status(&config).await)
}

/// Generate client setup script content
#[tauri::command]
pub async fn generate_client_setup_script() -> Result<String, String> {
    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    if config.server_ip.is_empty() {
        return Err("server_ip not configured".into());
    }
    Ok(crate::tls::generate_client_setup_script(&config.server_ip, 81, 443))
}

/// Generate nginx HTTPS config
#[tauri::command]
pub async fn generate_nginx_https_config() -> Result<String, String> {
    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    if config.server_ip.is_empty() {
        return Err("server_ip not configured".into());
    }
    Ok(crate::tls::generate_nginx_https_config(&config.server_ip))
}
