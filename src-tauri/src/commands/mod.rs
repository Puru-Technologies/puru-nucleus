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
    ServiceManifest, ServiceUpdateInfo, StagedUpdate,
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

    // Update hospital_code and cloud-driven config
    let mut config = crate::config::load_config().map_err(|e| e.user_message())?;
    config.hospital_code = hospital_code.clone();

    // Pull deployment_mode from hospital document (set in oxygen admin)
    if let Some(mode_str) = doc.fields.get("deployment_mode")
        .and_then(|v| crate::firestore::convert::get_optional_string(v))
    {
        match mode_str.as_str() {
            "native" => config.deployment_mode = crate::config::DeploymentMode::Native,
            "docker" => config.deployment_mode = crate::config::DeploymentMode::Docker,
            _ => {}
        }
        tracing::info!("Deployment mode from cloud: {}", mode_str);
    }

    // Pull server_ip from hospital document
    if let Some(ip) = doc.fields.get("serverIp")
        .and_then(|v| crate::firestore::convert::get_optional_string(v))
    {
        if !ip.is_empty() {
            config.server_ip = ip;
        }
    }

    // Pull the data root (cloud `dataLocation`) so the config seed + Docker volume
    // mapping have the correct filesystem path from the moment of activation.
    if let Some(dl) = doc.fields.get("dataLocation")
        .and_then(|v| crate::firestore::convert::get_optional_string(v))
    {
        let dl = dl.trim().to_string();
        if !dl.is_empty() {
            config.puru_data_path = Some(dl);
        }
    }

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

    // Sync deployment_mode from cloud if set
    let mut config_changed = false;
    let mut config = crate::config::load_config().map_err(|e| e.user_message())?;
    if let Some(mode_str) = doc.fields.get("deployment_mode")
        .and_then(|v| crate::firestore::convert::get_optional_string(v))
    {
        let new_mode = match mode_str.as_str() {
            "native" => Some(crate::config::DeploymentMode::Native),
            "docker" => Some(crate::config::DeploymentMode::Docker),
            _ => None,
        };
        if let Some(m) = new_mode {
            if config.deployment_mode != m {
                config.deployment_mode = m;
                config_changed = true;
            }
        }
    }
    if let Some(ip) = doc.fields.get("serverIp")
        .and_then(|v| crate::firestore::convert::get_optional_string(v))
    {
        if !ip.is_empty() && config.server_ip != ip {
            config.server_ip = ip;
            config_changed = true;
        }
    }
    // Data root (where hospital files live) is defined in the cloud as
    // `dataLocation` and mirrored into nucleus config, so both the native path
    // and the Docker volume mapping derive from a single source of truth.
    if let Some(dl) = doc.fields.get("dataLocation")
        .and_then(|v| crate::firestore::convert::get_optional_string(v))
    {
        let dl = dl.trim().to_string();
        if !dl.is_empty() && config.puru_data_path.as_deref() != Some(dl.as_str()) {
            config.puru_data_path = Some(dl);
            config_changed = true;
        }
    }
    if config_changed {
        crate::config::save_config(&config).map_err(|e| e.user_message())?;
        tracing::info!("Config synced from cloud (deployment_mode / server_ip / dataLocation)");
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

/// Point-in-time recovery: restore backup + replay binlogs up to target timestamp
#[tauri::command]
pub async fn restore_pitr(
    backup_id: Option<String>,
    stop_datetime: String,
) -> Result<crate::backup::binlog::PitrResult, String> {
    crate::backup::binlog::restore_pitr(backup_id.as_deref(), &stop_datetime)
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

/// Mark the setup wizard as completed (so the UI can hide config screens by
/// default afterwards). Idempotent.
#[tauri::command]
pub async fn mark_setup_completed() -> Result<(), String> {
    let mut config = crate::config::load_config().map_err(|e| e.to_string())?;
    if !config.setup_completed {
        config.setup_completed = true;
        crate::config::save_config(&config).map_err(|e| e.to_string())?;
    }
    Ok(())
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
    /// The condition that raised this alert has cleared (set by the watchdog,
    /// not by a human — that's `acknowledged`).
    pub resolved: bool,
    pub resolved_at: Option<String>,
}

/// A cloud command's live state, for the GUI's command-activity banner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandActivity {
    pub id: String,
    pub command_type: String,
    /// pending | executing | completed | failed
    pub status: String,
    pub message: String,
    pub created_at: String,
}

/// Recent cloud commands (newest first) so the UI can surface an in-progress
/// banner. Returns an empty list when offline / unconfigured rather than erroring.
#[tauri::command]
pub async fn get_command_activity() -> Result<Vec<CommandActivity>, String> {
    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    if config.hospital_code.is_empty() {
        return Ok(Vec::new());
    }
    let client = match crate::firestore::FirestoreClient::new_from_config().await {
        Ok(c) => c,
        Err(_) => return Ok(Vec::new()),
    };
    let docs = match client.list_recent_commands(&config.hospital_code, 5).await {
        Ok(d) => d,
        Err(_) => return Ok(Vec::new()),
    };

    let sval = |d: &crate::firestore::types::FirestoreDocument, k: &str| -> String {
        d.fields
            .get(k)
            .and_then(|v| v.get("stringValue"))
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string()
    };
    let tsval = |d: &crate::firestore::types::FirestoreDocument, k: &str| -> String {
        d.fields
            .get(k)
            .and_then(|v| v.get("stringValue").or_else(|| v.get("timestampValue")))
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string()
    };

    Ok(docs
        .iter()
        .map(|d| {
            let err = sval(d, "error");
            let message = if err.is_empty() { sval(d, "result") } else { err };
            let status = {
                let s = sval(d, "status");
                if s.is_empty() { "pending".to_string() } else { s }
            };
            CommandActivity {
                id: d.name.rsplit('/').next().unwrap_or("").to_string(),
                command_type: sval(d, "type"),
                status,
                message,
                created_at: tsval(d, "created_at"),
            }
        })
        .collect())
}

/// True if a nucleus daemon is answering its health endpoint on `port`.
async fn daemon_health_ok(port: u16) -> bool {
    let url = format!("http://127.0.0.1:{}/api/health", port);
    match reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(700))
        .build()
    {
        Ok(c) => c
            .get(&url)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// Process pending cloud commands from the GUI when no daemon is running — so
/// commands (restart/stop/start service, backup) work whenever the app is open,
/// not only when the headless daemon is installed. If a daemon IS running it owns
/// processing and this is a no-op (prevents double-execution). Returns the count
/// processed. Mirrors the daemon's command_listener body.
#[tauri::command]
pub async fn process_pending_commands() -> Result<u32, String> {
    use crate::firestore::convert::{get_map_fields, get_optional_string, get_string};

    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    if config.hospital_code.is_empty() {
        return Ok(0);
    }
    // The daemon owns command processing when it's up — don't double-run.
    let daemon_port = config.daemon.as_ref().map(|d| d.port).unwrap_or(9090);
    if daemon_health_ok(daemon_port).await {
        return Ok(0);
    }

    let client = match crate::firestore::FirestoreClient::new_from_config().await {
        Ok(c) => c,
        Err(_) => return Ok(0),
    };
    let commands = match client.poll_pending_commands(&config.hospital_code).await {
        Ok(c) => c,
        Err(_) => return Ok(0),
    };

    let mut processed = 0u32;
    for doc in commands {
        let command_id = match doc.name.rsplit('/').next() {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => continue,
        };

        // TTL: fail commands older than 5 minutes.
        let created = doc.fields.get("created_at").and_then(|v| {
            get_optional_string(v)
                .or_else(|| v.get("timestampValue").and_then(|t| t.as_str()).map(|s| s.to_string()))
        });
        let expired = created
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| chrono::Utc::now() - dt.with_timezone(&chrono::Utc) > chrono::Duration::minutes(5))
            .unwrap_or(true);
        if expired {
            let _ = client
                .update_command_status(&config.hospital_code, &command_id, "failed", None,
                    Some("Command expired (older than 5 minutes)"))
                .await;
            continue;
        }

        let command_type = match doc.fields.get("type").and_then(|v| get_string(v).ok()) {
            Some(t) => t,
            None => {
                let _ = client
                    .update_command_status(&config.hospital_code, &command_id, "failed", None,
                        Some("Malformed command: missing 'type' field"))
                    .await;
                continue;
            }
        };
        let params = doc
            .fields
            .get("params")
            .and_then(|v| get_map_fields(v).ok())
            .cloned()
            .unwrap_or_default();

        if client
            .update_command_status(&config.hospital_code, &command_id, "executing", None, None)
            .await
            .is_err()
        {
            continue;
        }

        let result = crate::daemon::commands::execute(&command_type, &params).await;
        let status = if result.success { "completed" } else { "failed" };
        let (r, e) = if result.success {
            (Some(result.message.as_str()), None)
        } else {
            (None, Some(result.message.as_str()))
        };
        let _ = client
            .update_command_status(&config.hospital_code, &command_id, status, r, e)
            .await;
        processed += 1;
    }

    Ok(processed)
}

/// Push a status heartbeat to Firestore: mark this machine online with a fresh
/// last_seen (+ serverIp / deployment_mode) and a telemetry snapshot. Driven from
/// the GUI immediately on startup and every minute, so the cloud dashboard shows
/// the hospital online whenever the app is open — not only when the daemon runs.
/// Best-effort: never errors the UI when offline.
#[tauri::command]
pub async fn send_status_heartbeat() -> Result<(), String> {
    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    if config.hospital_code.is_empty() {
        return Ok(());
    }
    let client = match crate::firestore::FirestoreClient::new_from_config().await {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };
    // Status: online + last_seen + serverIp + deployment_mode.
    let _ = client.sync_config(&config.hospital_code, &config).await;
    // Telemetry snapshot (CPU / RAM / disk).
    if let Ok(snap) = crate::telemetry::collect_snapshot().await {
        let _ = client.push_telemetry(&config.hospital_code, &snap).await;
    }
    Ok(())
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
            // Alerts written before resolution tracking existed have no
            // `resolved` field — treat those as still open.
            let resolved = doc.fields.get("resolved")
                .and_then(|v| crate::firestore::convert::get_bool(v).ok())
                .unwrap_or(false);
            let resolved_at = doc.fields.get("resolved_at")
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
                resolved,
                resolved_at,
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

    // Probe the daemon API — this is the ONLY reliable liveness signal, because
    // the daemon and the GUI share the `puru-dc.exe` image, so a tasklist /
    // image-name check would count the running GUI as "the daemon". The health
    // endpoint only answers when the daemon process is actually up.
    let api_url = format!("http://127.0.0.1:{}/api/health", port);
    let running = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
    {
        Ok(client) => client
            .get(&api_url)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false),
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
        // Daemon liveness = the API answered. Do NOT OR in svc_status.running:
        // that is image-name based and matches the GUI's own puru-dc.exe.
        running,
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

// ── Daemon Log Reader ────────────────────────────────────────────────────────

/// Read the last N lines of the daemon log file
#[tauri::command]
pub async fn read_daemon_log(lines: Option<u32>) -> Result<String, String> {
    let log_path = crate::config::config_dir().join("daemon.log");
    if !log_path.exists() {
        return Ok("No daemon log found. Start the daemon service to generate logs.".to_string());
    }
    let bytes = tokio::fs::read(&log_path)
        .await
        .map_err(|e| e.to_string())?;
    let content = String::from_utf8_lossy(&bytes).into_owned();
    let max_lines = lines.unwrap_or(100) as usize;
    let all_lines: Vec<&str> = content.lines().collect();
    let start = if all_lines.len() > max_lines {
        all_lines.len() - max_lines
    } else {
        0
    };
    Ok(all_lines[start..].join("\n"))
}

// ── Log File Reader ──────────────────────────────────────────────────────────

/// Get all log sources — known directories + running Docker containers.
/// Containers are returned with `kind: "container"` and pseudo-path
/// `container:<container_name>`.
#[tauri::command]
pub async fn get_log_sources() -> Vec<crate::logs::LogSource> {
    crate::logs::enumerate_sources().await
}

/// List log files under a path (or aggregate all known directory sources
/// if no path given). Container sources have no listable files — they are
/// addressed directly via read_log_file with the `container:` pseudo-path.
#[tauri::command]
pub fn list_log_files(path: Option<String>) -> Result<Vec<crate::logs::LogFileInfo>, String> {
    if let Some(p) = path {
        crate::logs::list_log_files(&p).map_err(|e| e.to_string())
    } else {
        // Aggregate all known directory sources (container sources are
        // skipped — they have no files to list).
        let sources = crate::logs::get_known_log_paths();
        let mut all_files = Vec::new();
        for source in &sources {
            if let Ok(files) = crate::logs::list_log_files(&source.path) {
                all_files.extend(files);
            }
        }
        all_files.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
        Ok(all_files)
    }
}

/// Read a log file with optional tail or offset+limit pagination. Accepts
/// host paths AND `container:<name>` pseudo-paths.
#[tauri::command]
pub async fn read_log_file(
    path: String,
    tail: Option<usize>,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<crate::logs::LogFileContent, String> {
    crate::logs::read_log_file(&path, tail, offset, limit)
        .await
        .map_err(|e| e.to_string())
}

/// Start streaming new lines from a log source. Returns an opaque stream_id;
/// the frontend subscribes to "log-tail-{stream_id}" Tauri events for line
/// batches (payload: LogTailEvent). Call tail_log_stop with the same id to
/// terminate. Dispatches on path: `container:<name>` uses docker follow
/// stream, otherwise polls the host file every 1s.
#[tauri::command]
pub async fn tail_log_start(app: tauri::AppHandle, path: String) -> Result<String, String> {
    crate::logs::start_tail(app, path)
        .await
        .map_err(|e| e.to_string())
}

/// Stop a tail stream started by tail_log_start. No-op if stream_id is
/// unknown (already stopped or never existed).
#[tauri::command]
pub fn tail_log_stop(stream_id: String) {
    crate::logs::stop_tail(&stream_id);
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
    // Store encrypted (at-rest): the file holds AES ciphertext, not plaintext.
    crate::secret::write_encrypted(&dest.to_string_lossy(), content.as_bytes())
        .map_err(|e| format!("Failed to store credentials: {}", e))?;

    // Update config to point to the designated path
    let mut config = crate::config::load_config().map_err(|e| e.user_message())?;
    config.gcs_credentials_path = Some(dest.display().to_string());
    crate::config::save_config(&config).map_err(|e| e.user_message())?;

    tracing::info!("Credentials file imported (encrypted) from {}", source_path);
    Ok(())
}

/// Save credentials JSON content directly (e.g. pasted from a message)
/// and update config.gcs_credentials_path
#[tauri::command]
pub async fn save_credentials_content(content: String) -> Result<(), String> {
    validate_credentials_json(&content)?;

    let dest = credentials_path();
    // Store encrypted (at-rest): the file holds AES ciphertext, not plaintext.
    crate::secret::write_encrypted(&dest.to_string_lossy(), content.as_bytes())
        .map_err(|e| format!("Failed to store credentials: {}", e))?;

    // Update config to point to the designated path
    let mut config = crate::config::load_config().map_err(|e| e.user_message())?;
    config.gcs_credentials_path = Some(dest.display().to_string());
    crate::config::save_config(&config).map_err(|e| e.user_message())?;

    tracing::info!("Credentials saved (encrypted) from pasted content");
    Ok(())
}

/// Cloud Function that exchanges a one-time onboarding code for the hospital's
/// service-account key + email. Region is Firebase Gen-1 default (us-central1).
const ONBOARD_REDEEM_URL: &str =
    "https://us-central1-puru-255206.cloudfunctions.net/redeemOnboardingCode";

#[derive(serde::Serialize)]
struct RedeemRequest<'a> {
    code: &'a str,
    fingerprint: &'a str,
}

#[derive(serde::Deserialize)]
struct RedeemResponse {
    sa_key: String,
    hospital_email: String,
    hospital_code: String,
    hospital_name: String,
    /// "known" (re-activation of a registered machine) or "new_allowed".
    #[serde(default)]
    machine_status: String,
    /// Existing label when the machine is already registered.
    #[serde(default)]
    machine_name: String,
}

/// Machine currently occupying a slot on the hospital, returned by the cloud
/// function when the redeem is rejected with 403 machine_limit_reached. The
/// activation UI shows these so the operator knows which one to ask the admin
/// to reset from the oxygen panel.
#[derive(serde::Deserialize, serde::Serialize, Clone, Debug)]
pub struct ExistingMachine {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub activated_at: Option<String>,
    #[serde(default)]
    pub last_seen_at: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct RedeemErrorBody {
    #[serde(default)]
    error: String,
    #[serde(default)]
    existing_machines: Vec<ExistingMachine>,
}

/// Returned to the activation UI so the operator can confirm the hospital and
/// so the UI can skip the machine-name prompt for an already-registered machine.
#[derive(serde::Serialize)]
pub struct RedeemResult {
    pub hospital_email: String,
    pub hospital_name: String,
    pub hospital_code: String,
    pub machine_status: String,
    pub machine_name: String,
}

/// Structured error the activation UI matches on to render the "machine limit
/// reached" screen instead of a raw error string. Tag on `kind`, so the
/// frontend can dispatch — plain error strings from other paths keep working.
#[derive(serde::Serialize, Debug)]
#[serde(tag = "kind")]
pub enum RedeemError {
    #[serde(rename = "machine_limit_reached")]
    MachineLimitReached {
        existing_machines: Vec<ExistingMachine>,
    },
    #[serde(rename = "message")]
    Message { message: String },
}

impl RedeemError {
    fn message(msg: impl Into<String>) -> Self {
        RedeemError::Message {
            message: msg.into(),
        }
    }
}

impl std::fmt::Display for RedeemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Serialise as JSON so the frontend can JSON.parse a structured error.
        // Tauri surfaces command Err(String) verbatim to the JS layer.
        match serde_json::to_string(self) {
            Ok(s) => f.write_str(&s),
            Err(_) => f.write_str("{\"kind\":\"message\",\"message\":\"Activation failed.\"}"),
        }
    }
}

/// Redeem a one-time onboarding code: POST it (with this machine's hardware
/// fingerprint) to the cloud function, receive the SA key + hospital info,
/// store the key encrypted at rest, and return the hospital email/name for the
/// operator to confirm before activation proceeds.
#[tauri::command]
pub async fn redeem_onboarding_code(code: String) -> Result<RedeemResult, String> {
    let code = code.trim().to_uppercase();
    if code.is_empty() {
        return Err(RedeemError::message("Enter the onboarding code.").to_string());
    }

    let fingerprint = crate::licensing::fingerprint::get_cached_fingerprint().map_err(|e| {
        RedeemError::message(format!("Could not read hardware fingerprint: {}", e)).to_string()
    })?;

    let resp = reqwest::Client::new()
        .post(ONBOARD_REDEEM_URL)
        .json(&RedeemRequest {
            code: &code,
            fingerprint: &fingerprint,
        })
        .send()
        .await
        .map_err(|e| {
            RedeemError::message(format!("Could not reach the activation server: {}", e))
                .to_string()
        })?;

    let status = resp.status();
    if !status.is_success() {
        // For 403 machine_limit_reached, parse the body and return a structured
        // error the UI dispatches on. All other statuses collapse to a plain
        // message.
        if status.as_u16() == 403 {
            let body: RedeemErrorBody = resp.json().await.unwrap_or_default();
            if body.error == "machine_limit_reached" {
                return Err(RedeemError::MachineLimitReached {
                    existing_machines: body.existing_machines,
                }
                .to_string());
            }
        }
        let msg = match status.as_u16() {
            404 => "Invalid code — check it and try again.",
            410 => "This code has expired or was already used. Ask for a new one.",
            409 => "Cloud credentials aren't ready for this hospital yet.",
            403 => "This machine can't be activated for this hospital. Contact Puru customer care.",
            _ => "Activation failed. Please try again.",
        };
        return Err(RedeemError::message(msg).to_string());
    }

    let data: RedeemResponse = resp.json().await.map_err(|e| {
        RedeemError::message(format!("Unexpected response from activation server: {}", e))
            .to_string()
    })?;

    // Store the SA key encrypted at rest and point config at it.
    let dest = credentials_path();
    crate::secret::write_encrypted(&dest.to_string_lossy(), data.sa_key.as_bytes())
        .map_err(|e| RedeemError::message(format!("Failed to store credentials: {}", e)).to_string())?;
    let mut config = crate::config::load_config()
        .map_err(|e| RedeemError::message(e.user_message()).to_string())?;
    config.gcs_credentials_path = Some(dest.display().to_string());
    crate::config::save_config(&config)
        .map_err(|e| RedeemError::message(e.user_message()).to_string())?;

    tracing::info!(
        "Redeemed onboarding code for hospital {}",
        data.hospital_code
    );
    Ok(RedeemResult {
        hospital_email: data.hospital_email,
        hospital_name: data.hospital_name,
        hospital_code: data.hospital_code,
        machine_status: data.machine_status,
        machine_name: data.machine_name,
    })
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

// ── System Clock Check ────────────────────────────────────────────────────────

/// Result of comparing the local system clock to real-world time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemClockStatus {
    /// Whether we could reach a time source at all (false = offline; don't nag).
    pub checked: bool,
    /// True when the skew is within tolerance (clock is fine).
    pub ok: bool,
    /// Local clock minus real time, in seconds. Positive = clock is ahead.
    pub skew_seconds: i64,
    /// Skew magnitude allowed before we warn.
    pub tolerance_seconds: i64,
    /// Local time (RFC 3339, with the machine's UTC offset).
    pub local_time: String,
    /// Real time from the reference server (RFC 3339, UTC), if reachable.
    pub server_time: Option<String>,
    /// The machine's current UTC offset, e.g. "+05:30".
    pub timezone_offset: String,
    /// Which reference host answered, if any.
    pub source: Option<String>,
    /// Human-readable summary for the UI.
    pub message: String,
}

/// Compare the local system clock against real-world time, read from the HTTP
/// `Date` header of a Google endpoint — the same clock authority that signs GCP
/// tokens. A large skew makes token requests fail with `invalid_grant`, which
/// otherwise surfaces only as an opaque "Cloud authentication failed" error.
#[tauri::command]
pub async fn check_system_clock() -> Result<SystemClockStatus, String> {
    // Google rejects a JWT whose iat/exp fall outside a small window, so keep
    // the warning threshold tight. 90s absorbs request latency and the Date
    // header's 1-second resolution without missing a real, auth-breaking skew.
    const TOLERANCE_SECS: i64 = 90;

    let local_now = chrono::Local::now();
    let timezone_offset = local_now.offset().to_string();
    let local_time = local_now.to_rfc3339();

    // A response that could not be produced is not proof the clock is wrong —
    // when offline we report checked=false, ok=true so the UI stays quiet.
    let offline = |msg: String| SystemClockStatus {
        checked: false,
        ok: true,
        skew_seconds: 0,
        tolerance_seconds: TOLERANCE_SECS,
        local_time: local_time.clone(),
        server_time: None,
        timezone_offset: timezone_offset.clone(),
        source: None,
        message: msg,
    };

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => return Ok(offline(format!("Could not build HTTP client: {}", e))),
    };

    // Prefer the token endpoint itself (most relevant), then a plain fallback.
    let sources = [
        "https://oauth2.googleapis.com/token",
        "https://www.google.com/generate_204",
    ];

    let mut reference: Option<(chrono::DateTime<chrono::Utc>, String)> = None;
    for url in sources {
        if let Ok(resp) = client.get(url).send().await {
            if let Some(date) = resp.headers().get(reqwest::header::DATE) {
                if let Ok(s) = date.to_str() {
                    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(s) {
                        reference = Some((dt.with_timezone(&chrono::Utc), url.to_string()));
                        break;
                    }
                }
            }
        }
    }

    let (server_utc, source) = match reference {
        Some(v) => v,
        None => {
            return Ok(offline(
                "Could not reach a time source to verify the clock (offline?).".to_string(),
            ))
        }
    };

    let skew_seconds = chrono::Utc::now()
        .signed_duration_since(server_utc)
        .num_seconds();
    let ok = skew_seconds.abs() <= TOLERANCE_SECS;

    let message = if ok {
        "System clock is in sync with real time.".to_string()
    } else {
        let dir = if skew_seconds > 0 { "ahead of" } else { "behind" };
        let mag = skew_seconds.abs();
        let human = if mag >= 3600 {
            format!("{}h {}m", mag / 3600, (mag % 3600) / 60)
        } else if mag >= 60 {
            format!("{}m {}s", mag / 60, mag % 60)
        } else {
            format!("{}s", mag)
        };
        format!(
            "System clock is {} {} real time. Cloud sign-in (GCP) will fail until the clock is synced.",
            human, dir
        )
    };

    Ok(SystemClockStatus {
        checked: true,
        ok,
        skew_seconds,
        tolerance_seconds: TOLERANCE_SECS,
        local_time,
        server_time: Some(server_utc.to_rfc3339()),
        timezone_offset,
        source: Some(source),
        message,
    })
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

/// Docker images to pull from Artifact Registry.
/// Image names must match the GCS compose fragments at
/// `gs://puru-releases/templates/fragments/` (note `gcp-puru-*` for pacs/comm/realtime).
const PURU_REGISTRY: &str = "asia-south2-docker.pkg.dev/puru-255206/puru1";
const PURU_IMAGES: &[&str] = &[
    "asia-south2-docker.pkg.dev/puru-255206/puru1/puru-auth:latest",
    "asia-south2-docker.pkg.dev/puru-255206/puru1/puru-xenon:latest",
    "asia-south2-docker.pkg.dev/puru-255206/puru1/puru-has:latest",
    "asia-south2-docker.pkg.dev/puru-255206/puru1/gcp-puru-pacs:latest",
    "asia-south2-docker.pkg.dev/puru-255206/puru1/puru-argon:latest",
    "asia-south2-docker.pkg.dev/puru-255206/puru1/gcp-puru-comm:latest",
    "asia-south2-docker.pkg.dev/puru-255206/puru1/gcp-puru-realtime:latest",
    "asia-south2-docker.pkg.dev/puru-255206/puru1/puru-neon:latest",
    "asia-south2-docker.pkg.dev/puru-255206/puru1/puru-bridge:latest",
    "asia-south2-docker.pkg.dev/puru-255206/puru1/puru-integration:latest",
    "asia-south2-docker.pkg.dev/puru-255206/puru1/puru-hydrogen:production",
];

/// Step 1: Verify all prerequisites are installed
#[tauri::command]
pub async fn setup_check_prerequisites() -> Result<(), String> {
    tracing::info!("Setup: checking prerequisites");

    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    let is_native = config.deployment_mode == crate::config::DeploymentMode::Native;

    let prereqs = crate::services::check_prerequisites()
        .await
        .map_err(|e| e.user_message())?;

    let missing: Vec<&str> = prereqs
        .iter()
        .filter(|p| !p.installed)
        .filter(|p| {
            // In native mode, Docker and Docker Compose are not required
            if is_native {
                p.name != "Docker" && p.name != "Docker Compose"
            } else {
                true
            }
        })
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

/// Install missing prerequisites (MySQL, RabbitMQ + Erlang) on Windows.
/// Emits `install-progress` Tauri events for download/install progress.
#[tauri::command]
pub async fn install_prerequisites(
    app: tauri::AppHandle,
    software: Vec<String>,
) -> Result<Vec<crate::installer::InstallResult>, String> {
    // Installing Windows services and writing under Program Files require
    // administrator rights. Fail fast with a distinct signal so the UI triggers a
    // single UAC elevation up front, rather than running a half-install that dies
    // at the service-registration step and surfaces a confusing error afterwards.
    #[cfg(target_os = "windows")]
    if !is_elevated() {
        return Err("ELEVATION_REQUIRED".to_string());
    }

    tracing::info!("Installing prerequisites: {:?}", software);
    Ok(crate::installer::install_missing(&app, &software).await)
}

/// One artifact downloaded to the operator's OS Downloads folder by
/// [`download_prerequisites_to_downloads`].
#[derive(Debug, Clone, Serialize)]
pub struct ManualDownloadResult {
    pub software: String,
    pub file: String,
    pub path: String,
    pub size_mb: f64,
    pub success: bool,
    pub error: Option<String>,
}

/// Download infra installers (MySQL, Erlang, RabbitMQ, delayed-exchange plugin)
/// straight to the operator's OS Downloads folder so they can install them by
/// hand — useful when silent installers can't run (locked-down machines, no
/// admin rights, air-gapped copy-to-USB flows). No install, no elevation, no
/// side effects: just files on disk. Emits `manual-infra-download-progress`
/// events per artifact so the UI can show a bar.
#[tauri::command]
pub async fn download_prerequisites_to_downloads(
    app: tauri::AppHandle,
    software: Vec<String>,
) -> Result<Vec<ManualDownloadResult>, String> {
    use tauri::Emitter;

    // Resolve the OS Downloads folder up front — if we can't find it, refuse
    // rather than silently dumping files into some fallback path.
    let dest_dir = directories::UserDirs::new()
        .and_then(|u| u.download_dir().map(|p| p.to_path_buf()))
        .ok_or_else(|| "Could not locate the OS Downloads folder on this machine.".to_string())?;
    if !dest_dir.exists() {
        std::fs::create_dir_all(&dest_dir)
            .map_err(|e| format!("Could not create {}: {}", dest_dir.display(), e))?;
    }

    // Which components to fetch. Empty request → the full default infra set.
    let mut components: Vec<String> = if software.is_empty() {
        vec!["mysql".into(), "erlang".into(), "rabbitmq".into(), "rabbitmq-delayed-message-exchange".into()]
    } else {
        // Map friendly names ("MySQL", "RabbitMQ", …) to the manifest component
        // ids used by oxygen. RabbitMQ implicitly pulls Erlang + the delayed
        // exchange plugin — same rule the auto-installer uses.
        let mut out: Vec<String> = Vec::new();
        for s in &software {
            match s.to_ascii_lowercase().as_str() {
                "mysql" => out.push("mysql".into()),
                "rabbitmq" => {
                    out.push("erlang".into());
                    out.push("rabbitmq".into());
                    out.push("rabbitmq-delayed-message-exchange".into());
                }
                "erlang" => out.push("erlang".into()),
                other => out.push(other.to_string()),
            }
        }
        out
    };
    components.dedup();

    // Build a GCS client + fetch the manifest once, then resolve every artifact.
    let cred = crate::releases::get_credentials_path().map_err(|e| e.user_message())?;
    let client = crate::releases::create_gcs_client(&cred)
        .await
        .map_err(|e| e.user_message())?;
    let manifest = crate::releases::fetch_infra_manifest(&client)
        .await
        .map_err(|e| e.user_message())?;

    let mut results: Vec<ManualDownloadResult> = Vec::new();

    for component in &components {
        let artifact = match crate::releases::resolve_infra_artifact(&manifest, component) {
            Some(a) => a,
            None => {
                results.push(ManualDownloadResult {
                    software: component.clone(),
                    file: String::new(),
                    path: String::new(),
                    size_mb: 0.0,
                    success: false,
                    error: Some(format!(
                        "'{}' is not in the infra manifest for this platform ({})",
                        component,
                        crate::releases::infra_platform_key()
                    )),
                });
                continue;
            }
        };

        let dest = dest_dir.join(&artifact.file);
        let _ = app.emit(
            "manual-infra-download-progress",
            serde_json::json!({
                "software": component,
                "file": artifact.file,
                "stage": "start",
                "percent": 0u8,
                "bytes_downloaded": 0u64,
                "bytes_total": 0u64,
            }),
        );

        // Throttle progress emits so we don't flood the UI on fast links —
        // fire every ~2 % or on completion, whichever comes first.
        let last = std::sync::atomic::AtomicU8::new(0);
        let app_evt = app.clone();
        let comp_evt = component.clone();
        let file_evt = artifact.file.clone();
        let dl = crate::releases::download_infra_artifact(
            &client,
            &artifact,
            &dest,
            move |done, total| {
                let percent = if total > 0 {
                    ((done as f64 / total as f64) * 100.0) as u8
                } else {
                    0
                };
                let prev = last.load(std::sync::atomic::Ordering::Relaxed);
                if percent >= prev.saturating_add(2) || (total > 0 && done >= total) {
                    last.store(percent, std::sync::atomic::Ordering::Relaxed);
                    let _ = app_evt.emit(
                        "manual-infra-download-progress",
                        serde_json::json!({
                            "software": comp_evt,
                            "file": file_evt,
                            "stage": "downloading",
                            "percent": percent,
                            "bytes_downloaded": done,
                            "bytes_total": total,
                        }),
                    );
                }
            },
        )
        .await;

        match dl {
            Ok(()) => {
                let size_mb = std::fs::metadata(&dest)
                    .map(|m| m.len() as f64 / 1_048_576.0)
                    .unwrap_or(0.0);
                let path_str = dest.display().to_string();
                let _ = app.emit(
                    "manual-infra-download-progress",
                    serde_json::json!({
                        "software": component,
                        "file": artifact.file,
                        "stage": "completed",
                        "percent": 100u8,
                        "path": path_str,
                    }),
                );
                results.push(ManualDownloadResult {
                    software: component.clone(),
                    file: artifact.file,
                    path: path_str,
                    size_mb,
                    success: true,
                    error: None,
                });
            }
            Err(e) => {
                let msg = e.to_string();
                let _ = app.emit(
                    "manual-infra-download-progress",
                    serde_json::json!({
                        "software": component,
                        "file": artifact.file,
                        "stage": "failed",
                        "message": msg.clone(),
                    }),
                );
                results.push(ManualDownloadResult {
                    software: component.clone(),
                    file: artifact.file,
                    path: String::new(),
                    size_mb: 0.0,
                    success: false,
                    error: Some(msg),
                });
            }
        }
    }

    Ok(results)
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

    let mysql_bin = crate::services::resolve_mysql_bin()
        .await
        .ok_or_else(|| "MySQL client not found. Add mysql to PATH or install MySQL.".to_string())?;

    // Preflight: the server must actually be accepting connections. Without this,
    // a stopped MySQL surfaces as a raw "ERROR 2003 ... (10061)" driver message
    // two layers down instead of a clear, actionable one here.
    let reachable = {
        use std::net::ToSocketAddrs;
        (config.mysql_host.as_str(), config.mysql_port)
            .to_socket_addrs()
            .ok()
            .and_then(|mut a| a.next())
            .map(|addr| {
                std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(1200)).is_ok()
            })
            .unwrap_or(false)
    };
    if !reachable {
        return Err(format!(
            "MySQL is not running on {}:{}. Run the \"Install Prerequisites\" step (it initializes and starts MySQL), or start the MySQL service, then retry.",
            config.mysql_host, config.mysql_port
        ));
    }

    let output = crate::process::silent_cmd(&mysql_bin)
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

/// Step 3: Configure RabbitMQ user + permissions for Puru services.
///
/// We deliberately do **not** create an extra vhost — services use the default
/// `/` vhost. This grants user `puru`/`puru123` full permissions on `/`, tags it
/// `administrator`, and enables the management + delayed-message-exchange
/// plugins. On the host path we first repair the Erlang cookie so `rabbitmqctl`
/// can authenticate against the node.
///
/// Two transports, deliberately: `rabbitmqctl` needs a matching Erlang cookie,
/// which is the single most common thing to break on Windows. When it can't
/// reach the node we fall back to the Management API (cookie-free) and only fail
/// the step if *both* are unusable — a cookie problem alone should never block an
/// install whose goal is already achievable over HTTP.
#[tauri::command]
pub async fn setup_configure_rabbitmq() -> Result<(), String> {
    tracing::info!("Setup: configuring RabbitMQ (default vhost \"/\")");

    // Detect a containerised RabbitMQ with a read-only probe (no side effects).
    let docker_probe = crate::process::silent_cmd("docker")
        .args(["exec", "rabbitmq", "rabbitmqctl", "-q", "list_vhosts"])
        .output()
        .await;
    let use_docker = matches!(&docker_probe, Ok(out) if out.status.success());

    // Explains a host-path auth failure (Erlang cookie) precisely in the error.
    #[allow(unused_mut)]
    let mut cookie_note = String::new();

    // Resolve the rabbitmqctl entrypoint. For the host node we also prepare it
    // first (cookie → offline plugin fix → start) because those must happen
    // before the node accepts commands. `ctl` is the host binary path; for
    // Docker it is unused (verbs run via `docker exec`).
    let ctl: String = if use_docker {
        enable_rabbitmq_plugins_docker().await;
        "rabbitmqctl".to_string()
    } else {
        // Host-installed RabbitMQ. Order is critical:
        //  1. share the Erlang cookie (node runs as SYSTEM, rabbitmqctl as user),
        //  2. fix the enabled-plugins set OFFLINE — an enabled plugin whose .ez is
        //     missing aborts the node on boot, so management is enabled and
        //     delayed-exchange is enabled ONLY when its .ez is actually present
        //     (otherwise explicitly disabled to clear any stale entry),
        //  3. start the node.

        // Whether the node's own cookie file changed — decides below whether the
        // service has to be bounced for the repair to actually take effect.
        #[allow(unused_mut, unused_variables, unused_assignments)]
        let mut restart_for_cookie = false;

        #[cfg(target_os = "windows")]
        match ensure_erlang_cookie() {
            Ok(repair) => restart_for_cookie = repair.node_cookie_changed,
            Err(e) => {
                tracing::warn!("RabbitMQ: {}", e);
                cookie_note = format!(" (Erlang cookie: {})", e);
            }
        }

        let rabbitmqctl = find_rabbitmqctl().await.ok_or_else(|| {
            "Cannot reach RabbitMQ: rabbitmqctl was not found on PATH or in the default install directory. Is RabbitMQ installed?".to_string()
        })?;
        let plugins_bin = rabbitmq_plugins_from_ctl(&rabbitmqctl);

        // (2) Fix plugins offline, before the node starts.
        let _ = crate::process::silent_cmd(&plugins_bin)
            .args(["enable", "rabbitmq_management"])
            .output()
            .await;
        #[cfg(target_os = "windows")]
        {
            if rabbitmq_delayed_ez_present() {
                let _ = crate::process::silent_cmd(&plugins_bin)
                    .args(["enable", "rabbitmq_delayed_message_exchange"])
                    .output()
                    .await;
                tracing::info!("RabbitMQ: delayed_message_exchange .ez present — enabled");
            } else {
                // No .ez — make sure it is NOT enabled, or the node won't boot.
                let _ = crate::process::silent_cmd(&plugins_bin)
                    .args(["disable", "rabbitmq_delayed_message_exchange"])
                    .output()
                    .await;
                tracing::warn!(
                    "RabbitMQ: delayed_message_exchange .ez missing from the plugins dir — left disabled (upload the matching .ez to infra to enable it)"
                );
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = crate::process::silent_cmd(&plugins_bin)
                .args(["enable", "rabbitmq_delayed_message_exchange"])
                .output()
                .await;
        }

        // (3) Now the node can boot cleanly.
        #[cfg(target_os = "windows")]
        if let Err(e) = ensure_rabbitmq_running(restart_for_cookie).await {
            cookie_note = format!("{} ({})", cookie_note, e);
        }

        rabbitmqctl
    };

    // ── Create + converge the `puru` user on the default "/" vhost ──────────────
    // Idempotent by design: on a setup RE-RUN the user already exists, which is
    // NOT an error — instead of failing we reset its password to the expected
    // value and re-apply permissions, so a second run finishes cleanly.
    let (add_ok, add_diag) =
        rabbitmq_ctl(use_docker, &ctl, &["add_user", "puru", "puru123"]).await;
    let user_existed =
        !add_ok && (add_diag.contains("already_exists") || add_diag.contains("already exists"));
    if user_existed {
        // Existing user is fine — make sure the password still matches puru123.
        let _ = rabbitmq_ctl(use_docker, &ctl, &["change_password", "puru", "puru123"]).await;
        tracing::info!(
            "Setup: RabbitMQ user 'puru' already existed — password reset so the re-run stays idempotent"
        );
    }
    let _ = rabbitmq_ctl(
        use_docker,
        &ctl,
        &["set_permissions", "-p", "/", "puru", ".*", ".*", ".*"],
    )
    .await;
    let _ = rabbitmq_ctl(use_docker, &ctl, &["set_user_tags", "puru", "administrator"]).await;

    // ── Verify, and on failure explain *precisely* why ─────────────────────────
    let (auth_ok, auth_diag) =
        rabbitmq_ctl(use_docker, &ctl, &["authenticate_user", "puru", "puru123"]).await;
    if auth_ok {
        tracing::info!("Setup: RabbitMQ configured (vhost=\"/\", user=puru)");
        return Ok(());
    }

    // rabbitmqctl couldn't confirm it — but that is very often a *transport*
    // failure (Erlang cookie mismatch) rather than a broker failure: the node
    // itself is healthy and answering on HTTP. Converge and verify over the
    // Management API, which needs no cookie, before calling this step failed.
    // Configuring the broker is the goal; rabbitmqctl is just one way to do it.
    if let Err(e) = crate::infra::ensure_rabbitmq_user().await {
        tracing::warn!("RabbitMQ: management-API fallback unavailable: {}", e);
    }
    if crate::infra::verify_rabbitmq_app_user().await {
        tracing::info!(
            "Setup: RabbitMQ configured via the management API — rabbitmqctl was unreachable{}",
            if cookie_note.is_empty() {
                String::new()
            } else {
                format!(" —{}", cookie_note)
            }
        );
        return Ok(());
    }

    // Distinguish "node unreachable" from "user/password problem": `list_users`
    // fails with a connection/cookie error when the node is down, but succeeds
    // when it's up — so it tells the two cases apart.
    let (node_ok, node_diag) = rabbitmq_ctl(use_docker, &ctl, &["list_users"]).await;
    if !node_ok {
        let detail = if !node_diag.is_empty() { node_diag } else { add_diag };
        let detail = if detail.is_empty() { "(no output)".to_string() } else { detail };
        return Err(if use_docker {
            format!(
                "RabbitMQ configuration failed: the node inside the 'rabbitmq' container is not reachable \
                 (it may be stopped or still starting up).\n\nrabbitmqctl said: {}",
                detail
            )
        } else {
            format!(
                "RabbitMQ configuration failed: neither rabbitmqctl nor the management API on \
                 http://127.0.0.1:15672 could configure the broker. The RabbitMQ Windows service is \
                 most likely stopped; if it is running, the Erlang cookie doesn't match between the \
                 service (runs as SYSTEM) and your user account AND the management plugin is off, so \
                 there is no way left to reach the node.{}\n\nrabbitmqctl said: {}",
                cookie_note, detail
            )
        });
    }

    // Node is up, but 'puru' still can't authenticate with puru123.
    Err(format!(
        "RabbitMQ is running, but the 'puru' user could not be verified with the expected password. \
         It may already exist with a different password; setup tried to reset it but authentication still failed. \
         You can fix it manually with:  rabbitmqctl change_password puru puru123\n\n\
         add_user said: {}\nauthenticate_user said: {}",
        if add_diag.is_empty() { "(user created)".to_string() } else { add_diag },
        if auth_diag.is_empty() { "(no output)".to_string() } else { auth_diag },
    ))
}

/// Run a single `rabbitmqctl` verb against either the Docker container
/// (`docker exec rabbitmq rabbitmqctl …`) or the host node (`<ctl_path> …`).
/// Returns `(succeeded, diagnostic)`, where `diagnostic` is the trimmed stderr
/// (falling back to stdout) — used to build precise, user-facing error messages.
async fn rabbitmq_ctl(use_docker: bool, ctl_path: &str, verb: &[&str]) -> (bool, String) {
    let output = if use_docker {
        let mut args: Vec<&str> = vec!["exec", "rabbitmq", "rabbitmqctl"];
        args.extend_from_slice(verb);
        crate::process::silent_cmd("docker").args(&args).output().await
    } else {
        crate::process::silent_cmd(ctl_path).args(verb).output().await
    };
    match output {
        Ok(o) => {
            let mut diag = String::from_utf8_lossy(&o.stderr).trim().to_string();
            if diag.is_empty() {
                diag = String::from_utf8_lossy(&o.stdout).trim().to_string();
            }
            (o.status.success(), diag)
        }
        Err(e) => (false, format!("could not run rabbitmqctl ({})", e)),
    }
}

/// Enable RabbitMQ plugins inside the `rabbitmq` Docker container. Both are
/// best-effort: `rabbitmq_management` is bundled (idempotent), while
/// `rabbitmq_delayed_message_exchange` needs its `.ez` present in the plugins
/// dir first — warn (don't fail) if it isn't there yet.
async fn enable_rabbitmq_plugins_docker() {
    for plugin in ["rabbitmq_management", "rabbitmq_delayed_message_exchange"] {
        let out = crate::process::silent_cmd("docker")
            .args(["exec", "rabbitmq", "rabbitmq-plugins", "enable", plugin])
            .output()
            .await;
        match out {
            Ok(o) if o.status.success() => tracing::info!("RabbitMQ: enabled {}", plugin),
            Ok(o) => tracing::warn!(
                "RabbitMQ: could not enable {} — {}",
                plugin,
                String::from_utf8_lossy(&o.stderr).trim()
            ),
            Err(e) => tracing::warn!("RabbitMQ: could not enable {}: {}", plugin, e),
        }
    }
}

/// Outcome of a cookie repair — tells the caller whether the *node's* own cookie
/// file changed, because a node that is already running keeps the cookie it read
/// at boot and only adopts the new one after a service restart.
#[cfg(target_os = "windows")]
struct CookieRepair {
    node_cookie_changed: bool,
}

/// Repair the Erlang cookie on Windows: the RabbitMQ node runs as LocalSystem
/// while `rabbitmqctl` runs as the invoking user. Erlang authenticates the two by
/// a shared `.erlang.cookie`; a mismatch causes "TCP connection succeeded but
/// Erlang distribution failed".
///
/// Two rules make this actually converge:
///
/// 1. **The running node wins.** Its VM loaded a cookie at boot and cannot adopt
///    a different one without a restart, so when the broker is up we treat the
///    SYSTEM cookie as authoritative and align the user's copy to it — no
///    downtime. Only when the node is down are we free to pick (or generate) one.
/// 2. **Both homes must agree.** Writing one of the two leaves exactly the
///    mismatch this function exists to remove, so a partial repair is an error,
///    not a success.
#[cfg(target_os = "windows")]
fn ensure_erlang_cookie() -> Result<CookieRepair, String> {
    use std::path::PathBuf;

    let sys_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    // LocalSystem's home — where the RabbitMQ service (running as SYSTEM) reads
    // its cookie from.
    let systemprofile =
        PathBuf::from(&sys_root).join(r"System32\config\systemprofile\.erlang.cookie");
    // The invoking user's home — where rabbitmqctl reads its cookie from.
    let user = std::env::var("USERPROFILE")
        .ok()
        .map(|u| PathBuf::from(u).join(".erlang.cookie"));

    let sys_cookie = read_cookie(&systemprofile);
    let user_cookie = user.as_deref().and_then(read_cookie);

    let node_up = rabbitmq_node_up();
    let value: Vec<u8> = if node_up {
        // Rule 1 — never hand a live node a cookie it can't be holding.
        sys_cookie.clone().or_else(|| user_cookie.clone()).ok_or_else(|| {
            "the RabbitMQ node is running but no Erlang cookie could be read from either home — \
             re-run Nucleus as administrator so the SYSTEM cookie is readable"
                .to_string()
        })?
    } else {
        user_cookie
            .clone()
            .or_else(|| sys_cookie.clone())
            .unwrap_or_else(|| generate_erlang_cookie().into_bytes())
    };

    let node_cookie_changed = sys_cookie.as_deref() != Some(value.as_slice());

    let mut dests: Vec<PathBuf> = vec![systemprofile];
    if let Some(u) = user {
        dests.push(u);
    }

    let mut failures: Vec<String> = Vec::new();
    for dest in &dests {
        if read_cookie(dest).as_deref() == Some(value.as_slice()) {
            continue; // already correct — leave it alone
        }
        match write_cookie(dest, &value) {
            Ok(_) => tracing::info!("RabbitMQ: Erlang cookie aligned at {}", dest.display()),
            Err(e) => failures.push(format!("{} ({})", dest.display(), e)),
        }
    }

    // Rule 2 — anything less than "both homes hold the same value" is a failure.
    if !failures.is_empty() {
        return Err(format!(
            "could not align the Erlang cookie at: {} — re-run Nucleus as administrator if this persists",
            failures.join("; ")
        ));
    }
    Ok(CookieRepair { node_cookie_changed })
}

/// Read a cookie file, normalized. Erlang compares the cookie as exact file
/// contents, so a stray CRLF is the difference between matching and not — trim
/// on the way in and out and the comparison stays meaningful. `None` for absent
/// or empty files.
#[cfg(target_os = "windows")]
fn read_cookie(path: &std::path::Path) -> Option<Vec<u8>> {
    let raw = std::fs::read(path).ok()?;
    let trimmed = String::from_utf8_lossy(&raw).trim().as_bytes().to_vec();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Write a cookie file, clearing the ReadOnly attribute first.
///
/// RabbitMQ's own Windows installer marks `.erlang.cookie` ReadOnly, and on
/// Windows that *attribute* denies writes even when the ACL grants the owner
/// FullControl — so without this the repair fails with "Access is denied" on a
/// file the user owns, and no amount of elevation helps. The write is read back
/// because a silent partial write would reintroduce the very mismatch we are
/// here to remove.
#[cfg(target_os = "windows")]
fn write_cookie(dest: &std::path::Path, value: &[u8]) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(md) = std::fs::metadata(dest) {
        let mut perms = md.permissions();
        if perms.readonly() {
            perms.set_readonly(false);
            std::fs::set_permissions(dest, perms)
                .map_err(|e| format!("could not clear the ReadOnly attribute: {}", e))?;
        }
    }
    std::fs::write(dest, value).map_err(|e| e.to_string())?;
    match read_cookie(dest) {
        Some(v) if v == value => Ok(()),
        Some(_) => Err("written but read back with a different value".to_string()),
        None => Err("written but read back empty".to_string()),
    }
}

/// True when the broker is already answering AMQP — meaning its Erlang VM has
/// loaded a cookie and cannot adopt a new one without a service restart.
#[cfg(target_os = "windows")]
fn rabbitmq_node_up() -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], 5672)),
        std::time::Duration::from_millis(600),
    )
    .is_ok()
}

/// Generate a RabbitMQ-style Erlang cookie (uppercase alphanumeric, 20 chars).
#[cfg(target_os = "windows")]
fn generate_erlang_cookie() -> String {
    use rand::Rng;
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = rand::thread_rng();
    (0..20).map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char).collect()
}

/// Whether the `rabbitmq_delayed_message_exchange` .ez is actually present in a
/// RabbitMQ plugins dir. Enabling the plugin without the file makes the node
/// abort on boot, so we gate the enable on this.
#[cfg(target_os = "windows")]
fn rabbitmq_delayed_ez_present() -> bool {
    let base = std::path::Path::new(r"C:\Program Files\RabbitMQ Server");
    let Ok(entries) = std::fs::read_dir(base) else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("rabbitmq_server-") {
            continue;
        }
        if let Ok(files) = std::fs::read_dir(entry.path().join("plugins")) {
            for f in files.flatten() {
                let fname = f.file_name().to_string_lossy().to_lowercase();
                if fname.contains("delayed_message_exchange") && fname.ends_with(".ez") {
                    return true;
                }
            }
        }
    }
    false
}

/// Name of the registered RabbitMQ Windows service (via `sc query`), or None.
/// Version-agnostic — matches `RabbitMQ`, `RabbitMQ Server`, etc.
#[cfg(target_os = "windows")]
pub(crate) fn rabbitmq_service_name() -> Option<String> {
    let out = crate::process::silent_std_cmd("sc")
        .args(["query", "state=", "all"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("SERVICE_NAME:") {
            let name = rest.trim();
            if name.to_lowercase().contains("rabbitmq") {
                return Some(name.to_string());
            }
        }
    }
    None
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn rabbitmq_service_name() -> Option<String> {
    None
}

/// Ensure the MySQL Windows service is running, waiting until the server
/// accepts TCP on `mysql_host:mysql_port` (default 127.0.0.1:3306). Used both
/// by setup steps and by the daemon's boot-time native startup — Puru services
/// abort during init if MySQL isn't up. No-op if MySQL is already reachable.
#[cfg(target_os = "windows")]
pub(crate) async fn ensure_mysql_running() -> Result<(), String> {
    // Read the configured host/port so we probe the *right* endpoint (a fleet
    // machine might have MySQL on a non-default port).
    let cfg = crate::config::load_config().ok();
    let host = cfg
        .as_ref()
        .map(|c| c.mysql_host.clone())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let port = cfg.as_ref().map(|c| c.mysql_port).unwrap_or(3306);

    fn tcp_up(host: &str, port: u16) -> bool {
        use std::net::ToSocketAddrs;
        (host, port)
            .to_socket_addrs()
            .ok()
            .and_then(|mut a| a.next())
            .map(|addr| {
                std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(600))
                    .is_ok()
            })
            .unwrap_or(false)
    }

    if tcp_up(&host, port) {
        return Ok(());
    }

    // Discover the MySQL Windows service (name varies: MySQL, MySQL80, MySQL84…).
    let svc = crate::installer::mysql_service_name().unwrap_or_else(|| "MySQL".to_string());
    let _ = crate::process::silent_cmd("net")
        .args(["start", &svc])
        .output()
        .await;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        if tcp_up(&host, port) {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "MySQL did not accept connections on {}:{} within 60s (service '{}')",
                host, port, svc
            ));
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) async fn ensure_mysql_running() -> Result<(), String> {
    // On Unix, MySQL is under systemd/launchd and outside our control here.
    // The caller can treat a Docker/container MySQL as always up.
    Ok(())
}

/// Public wrapper so the daemon can call the Windows `ensure_rabbitmq_running`
/// (which is defined below for target_os = "windows" only) uniformly.
pub(crate) async fn ensure_rabbitmq_running_public() -> Result<(), String> {
    // No cookie was touched on this path, so a running node needs no restart.
    #[cfg(target_os = "windows")]
    return ensure_rabbitmq_running(false).await;

    #[cfg(not(target_os = "windows"))]
    Ok(())
}

/// Start the RabbitMQ Windows service if it isn't running and wait for the node
/// to accept AMQP on 5672. The service (SYSTEM) picks up the shared cookie written
/// by `ensure_erlang_cookie` on start, so this must run after it.
///
/// `restart_for_cookie` closes the gap that made the cookie repair inert: the
/// Erlang VM reads its cookie once, at boot. If we just rewrote the node's cookie
/// file while the node was up, the file is correct and the *node* is still using
/// the old value — so bounce the service rather than returning a success that
/// leaves the original mismatch in place.
#[cfg(target_os = "windows")]
async fn ensure_rabbitmq_running(restart_for_cookie: bool) -> Result<(), String> {
    // Discover the RabbitMQ service name (usually "RabbitMQ", but don't assume).
    let svc = rabbitmq_service_name().unwrap_or_else(|| "RabbitMQ".to_string());

    if rabbitmq_node_up() {
        if !restart_for_cookie {
            return Ok(());
        }
        tracing::info!(
            "RabbitMQ: node cookie changed while the broker was up — restarting service '{}' so it takes effect",
            svc
        );
        let _ = crate::process::silent_cmd("net")
            .args(["stop", &svc])
            .output()
            .await;
    }

    let _ = crate::process::silent_cmd("net")
        .args(["start", &svc])
        .output()
        .await;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        if rabbitmq_node_up() {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(
                "RabbitMQ service did not come up on 5672 within 60s — check the RabbitMQ service"
                    .to_string(),
            );
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Derive the `rabbitmq-plugins` binary from a resolved `rabbitmqctl` path —
/// they sit side by side (same PATH entry or the same `sbin` dir), so swapping
/// the name yields the plugins tool: `rabbitmqctl` → `rabbitmq-plugins`,
/// `…\sbin\rabbitmqctl.bat` → `…\sbin\rabbitmq-plugins.bat`.
fn rabbitmq_plugins_from_ctl(ctl: &str) -> String {
    ctl.replace("rabbitmqctl", "rabbitmq-plugins")
}

/// Find rabbitmqctl binary — checks PATH first, then Windows default install directory.
async fn find_rabbitmqctl() -> Option<String> {
    // 1. Check PATH
    if let Ok(output) = crate::process::silent_cmd("rabbitmqctl")
        .arg("version")
        .output()
        .await
    {
        if output.status.success() {
            return Some("rabbitmqctl".to_string());
        }
    }

    // 2. Windows: scan default install directory
    #[cfg(target_os = "windows")]
    {
        let rabbitmq_base = r"C:\Program Files\RabbitMQ Server";
        if let Ok(entries) = std::fs::read_dir(rabbitmq_base) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("rabbitmq_server-") {
                    let ctl = format!(r"{}\{}\sbin\rabbitmqctl.bat", rabbitmq_base, name);
                    if std::path::Path::new(&ctl).exists() {
                        return Some(ctl);
                    }
                }
            }
        }
    }

    None
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

    // Fetch service modules from Firestore to know which services to include
    let modules = if !config.hospital_code.is_empty() {
        match crate::firestore::FirestoreClient::new_from_config().await {
            Ok(client) => client.fetch_modules(&config.hospital_code).await
                .unwrap_or_else(|_| ServiceModules::all_enabled()),
            Err(_) => ServiceModules::all_enabled(),
        }
    } else {
        ServiceModules::default()
    };

    // Check which infra is host-installed (skip their Docker containers)
    let prereqs = crate::services::check_prerequisites().await.unwrap_or_default();
    let mysql_on_host = prereqs.iter().any(|p| p.name == "MySQL" && p.installed);
    let rmq_on_host = prereqs.iter().any(|p| p.name == "RabbitMQ" && p.installed);

    // Ensure compose fragments + env templates are present locally. Both calls
    // are idempotent (skip files that already exist on disk). Without this,
    // assemble_compose() would silently fall through to the inline fallback,
    // which carries a stale registry and image names.
    if let Err(e) = crate::compose_template::download_fragments().await {
        tracing::warn!("Setup: fragment download failed ({}); will try local fragments or inline fallback", e);
    }
    if let Err(e) = crate::compose_template::download_env_templates().await {
        tracing::warn!("Setup: env template download failed ({}); will rely on existing local env files", e);
    }

    // Try fragment-based assembly first (preferred), fallback to inline template.
    // Fragments contain {{PLACEHOLDERS}} that must be substituted before docker can
    // parse the file; the inline fallback has no placeholders so substitution is a no-op.
    let compose_content = match crate::compose_template::assemble_compose(&modules) {
        Ok(content) => {
            tracing::info!("Setup: assembled docker-compose.yml from fragments");
            let vars = crate::compose_template::build_variables_from_config(&config);
            crate::compose_template::substitute_variables(&content, &vars)
        }
        Err(_) => {
            tracing::info!("Setup: fragments not available, using inline template");
            generate_docker_compose(&config, &modules, mysql_on_host, rmq_on_host)
        }
    };

    std::fs::write(compose_path, &compose_content)
        .map_err(|e| format!("Failed to write docker-compose.yml: {}", e))?;

    tracing::info!("Setup: wrote docker-compose.yml to {}", config.docker_compose_path);
    Ok(())
}

/// Reset setup — delete the generated docker-compose.yml
#[tauri::command]
pub async fn setup_reset() -> Result<String, String> {
    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    let compose_path = std::path::Path::new(&config.docker_compose_path);

    let mut deleted = Vec::new();

    if compose_path.exists() {
        std::fs::remove_file(compose_path)
            .map_err(|e| format!("Failed to delete docker-compose.yml: {}", e))?;
        deleted.push(config.docker_compose_path.clone());
    }

    // Also delete env files alongside compose if they exist
    if let Some(parent) = compose_path.parent() {
        let env_dir = parent.join("env");
        if env_dir.exists() {
            let _ = std::fs::remove_dir_all(&env_dir);
            deleted.push(env_dir.display().to_string());
        }
    }

    if deleted.is_empty() {
        Ok("Nothing to reset — no generated files found.".to_string())
    } else {
        tracing::info!("Setup reset: deleted {:?}", deleted);
        Ok(format!("Deleted: {}", deleted.join(", ")))
    }
}

/// Generate a docker-compose.yml for the Puru stack (inline fallback).
/// Only includes services enabled in ServiceModules.
/// Skips MySQL/RabbitMQ containers if they're detected as host-installed.
fn generate_docker_compose(config: &NucleusConfig, modules: &ServiceModules, mysql_on_host: bool, rmq_on_host: bool) -> String {
    let mut compose = format!(
        r#"version: "3.8"

# Puru Labs Private Limited — Docker Compose
# Generated by puru-dc
# Hospital: {}

services:
"#,
        config.hospital_code,
    );

    // Only add MySQL container if NOT host-installed
    let mysql_is_docker = !mysql_on_host;
    if mysql_is_docker {
        compose.push_str(&format!(
            r#"  database:
    image: mysql:8.0
    container_name: purusql
    restart: unless-stopped
    environment:
      MYSQL_ROOT_PASSWORD: {pw}
    ports:
      - "{port}:3306"
    volumes:
      - mysql_data:/var/lib/mysql
    command: --character-set-server=utf8mb4 --collation-server=utf8mb4_unicode_ci

"#,
            pw = config.mysql_password,
            port = config.mysql_port,
        ));
    }

    // Only add RabbitMQ container if NOT host-installed
    let rmq_is_docker = !rmq_on_host;
    if rmq_is_docker {
        compose.push_str(
            r#"  rabbitmq:
    image: rabbitmq:3.12-management
    container_name: rabbitmq
    restart: unless-stopped
    ports:
      - "5672:5672"
      - "15672:15672"
    volumes:
      - rabbitmq_data:/var/lib/rabbitmq

"#,
        );
    }

    let mysql_host = if config.mysql_host.is_empty() { "127.0.0.1" } else { &config.mysql_host };
    let db_host = if mysql_is_docker { "database" } else { mysql_host };
    let rmq_host = if rmq_is_docker { "rabbitmq" } else { "localhost" };

    // Host data dir (cloud `dataLocation`) bind-mounted into each container at the
    // container-side data root `/data/puru`. This is why the config seed uses the
    // native path in native mode but `/data/puru` in Docker mode — same source.
    let host_data = config.puru_data_path.as_deref().unwrap_or("").replace('\\', "/");

    // Helper to generate a Spring Boot service block
    let svc = |name: &str, image: &str, container: &str, db: &str, port: u16, needs_rmq: bool| -> String {
        // Every backend microservice waits for auth to be created first — auth is
        // the token authority the rest authenticate against. Compose `depends_on`
        // orders container startup; `restart: unless-stopped` + Spring retries
        // cover the gap until auth is actually ready, mirroring the native gate.
        let depends = if modules.auth && container != "auth" {
            "    depends_on:\n      - auth\n"
        } else {
            ""
        };
        let mut s = format!(
            r#"  {container}:
    image: asia-south2-docker.pkg.dev/puru-255206/puru1/{image}:latest
    container_name: {container}
    restart: unless-stopped
{depends}    environment:
      SPRING_DATASOURCE_URL: jdbc:mysql://{db_host}:{db_port}/{db}?useSSL=false&allowPublicKeyRetrieval=true
      SPRING_DATASOURCE_USERNAME: {user}
      SPRING_DATASOURCE_PASSWORD: {pw}
"#,
            container = container,
            image = image,
            depends = depends,
            db_host = db_host,
            db_port = config.mysql_port,
            db = db,
            user = config.mysql_user,
            pw = config.mysql_password,
        );
        if needs_rmq {
            // Default vhost "/" — we deliberately do not create an extra vhost.
            s.push_str(&format!(
                r#"      SPRING_RABBITMQ_HOST: {rmq}
      SPRING_RABBITMQ_VIRTUAL_HOST: "/"
      SPRING_RABBITMQ_USERNAME: puru
      SPRING_RABBITMQ_PASSWORD: puru123
"#,
                rmq = rmq_host,
            ));
        }
        let volumes = if host_data.is_empty() {
            String::new()
        } else {
            format!("    volumes:\n      - \"{}:/data/puru\"\n", host_data)
        };
        s.push_str(&format!(
            r#"    ports:
      - "{port}:{port}"
    network_mode: host
{volumes}
"#,
            port = port,
            volumes = volumes,
        ));
        let _ = name; // used for clarity in the caller
        s
    };

    if modules.auth {
        compose.push_str(&svc("Auth", "puru-auth", "auth", "puru_auth", 8080, false));
    }
    if modules.xenon {
        compose.push_str(&svc("Xenon", "puru-xenon", "backend", "puru_im", 8081, true));
    }
    if modules.has {
        compose.push_str(&svc("HAS", "puru-has", "has", "puru_has", 8082, true));
    }
    if modules.pacs {
        compose.push_str(&svc("PACS", "gcp-puru-pacs", "pacs", "puru_dicom", 8083, false));
    }
    if modules.argon {
        compose.push_str(&svc("Argon", "puru-argon", "pathology", "puru_path", 8084, true));
    }
    if modules.comm {
        compose.push_str(&svc("Comm", "gcp-puru-comm", "comm_server", "puru_im", 8085, true));
    }
    if modules.realtime {
        compose.push_str(&svc("Realtime", "gcp-puru-realtime", "realtime", "puru_im", 8086, true));
    }
    if modules.neon {
        compose.push_str(&svc("Neon", "puru-neon", "medical", "puru_med", 8087, false));
    }
    if modules.mercury {
        compose.push_str(&svc("Mercury", "puru-mercury", "hrms", "puru_im", 8089, false));
    }
    if modules.counter {
        compose.push_str(&svc("Counter", "puru-counter", "counter", "puru_im", 8095, false));
    }
    if modules.bridge {
        compose.push_str(&svc("Bridge", "puru-bridge", "bridge", "puru_bridge", 8094, false));
    }
    if modules.integration {
        compose.push_str(&svc("Integration", "puru-integration", "integration", "puru_im", 8088, false));
    }

    if modules.hydrogen {
        compose.push_str(
            r#"  frontend:
    image: asia-south2-docker.pkg.dev/puru-255206/puru1/puru-hydrogen:production
    container_name: frontend
    restart: unless-stopped
    ports:
      - "80:80"

"#,
        );
    }

    // Volumes (only if Docker containers are used)
    let mut volumes = Vec::new();
    if mysql_is_docker { volumes.push("  mysql_data:"); }
    if rmq_is_docker { volumes.push("  rabbitmq_data:"); }
    if !volumes.is_empty() {
        compose.push_str("volumes:\n");
        for v in volumes {
            compose.push_str(v);
            compose.push('\n');
        }
    }

    compose
}

/// Step 5: Pull Docker images from Artifact Registry
#[tauri::command]
pub async fn setup_pull_images() -> Result<(), String> {
    tracing::info!("Setup: pulling Docker images");

    // Authenticate Docker with Artifact Registry using the service account key
    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    let registry_host = PURU_REGISTRY.split('/').next().unwrap_or("asia-south2-docker.pkg.dev");
    if let Some(ref cred_path) = config.gcs_credentials_path {
        let cred = std::path::Path::new(cred_path);
        if cred.exists() {
            tracing::info!("Authenticating Docker with {} using {}", registry_host, cred_path);
            let key_content = crate::secret::read_decrypted(cred_path)
                .map_err(|e| format!("Failed to read credentials: {}", e))?;

            let mut child = crate::process::silent_cmd("docker")
                .args(["login", "-u", "_json_key", "--password-stdin", registry_host])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| format!("Failed to run docker login: {}", e))?;

            if let Some(mut stdin) = child.stdin.take() {
                use tokio::io::AsyncWriteExt;
                stdin.write_all(key_content.as_bytes()).await
                    .map_err(|e| format!("Failed to write credentials to docker login: {}", e))?;
                drop(stdin);
            }

            let output = child.wait_with_output().await
                .map_err(|e| format!("docker login failed: {}", e))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::warn!("docker login warning: {}", stderr.trim());
            } else {
                tracing::info!("Docker authenticated with {}", registry_host);
            }
        } else {
            return Err(format!(
                "GCS credentials file not found at '{}'. Configure it in Settings first.",
                cred_path
            ));
        }
    } else {
        return Err(
            "GCS credentials path not configured. Set it in Settings before pulling images."
                .to_string(),
        );
    }

    for image in PURU_IMAGES {
        tracing::info!("Pulling image: {}", image);

        let output = crate::process::silent_cmd("docker")
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
    let output = crate::process::silent_cmd("docker")
        .args(["compose", "-f", compose_path, "up", "-d"])
        .output()
        .await;

    let result = match output {
        Ok(out) if out.status.success() => Ok(()),
        _ => {
            // Fallback to V1
            let output = crate::process::silent_cmd("docker-compose")
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

// ── Native Setup Steps ──────────────────────────────────────────────────────

/// Native Step 3: Generate env files for native JAR deployment
#[tauri::command]
pub async fn setup_generate_env_files() -> Result<(), String> {
    tracing::info!("Setup (native): generating environment files");

    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    let env_dir = config.env_dir();

    tokio::fs::create_dir_all(&env_dir)
        .await
        .map_err(|e| format!("Cannot create env directory {}: {}", env_dir.display(), e))?;

    // general.env
    let general = format!(
        "# Generated by puru-dc\n\
         HOSPITAL_CODE={}\n\
         SERVER_IP={}\n\
         PURU_DATA_PATH={}\n",
        config.hospital_code,
        config.server_ip,
        config.puru_data_path.as_deref().unwrap_or(""),
    );
    tokio::fs::write(env_dir.join("general.env"), &general)
        .await
        .map_err(|e| format!("Failed to write general.env: {}", e))?;

    // database.env
    let db_host = if config.mysql_host.is_empty() { "127.0.0.1" } else { &config.mysql_host };
    let database = format!(
        "# Generated by puru-dc\n\
         SPRING_DATASOURCE_URL=jdbc:mysql://{}:{{}}/{{}}?useSSL=false&allowPublicKeyRetrieval=true&serverTimezone=Asia/Kolkata\n\
         SPRING_DATASOURCE_USERNAME={}\n\
         SPRING_DATASOURCE_PASSWORD={}\n\
         MYSQL_HOST={}\n\
         MYSQL_PORT={}\n",
        db_host, config.mysql_user, config.mysql_password, db_host, config.mysql_port,
    );
    tokio::fs::write(env_dir.join("database.env"), &database)
        .await
        .map_err(|e| format!("Failed to write database.env: {}", e))?;

    // rabbitmq.env — default "/" vhost (we do not create an extra vhost)
    let rabbitmq = "# Generated by puru-dc\n\
         SPRING_RABBITMQ_HOST=127.0.0.1\n\
         SPRING_RABBITMQ_PORT=5672\n\
         SPRING_RABBITMQ_VIRTUAL_HOST=/\n\
         SPRING_RABBITMQ_USERNAME=puru\n\
         SPRING_RABBITMQ_PASSWORD=puru123\n";
    tokio::fs::write(env_dir.join("rabbitmq.env"), rabbitmq)
        .await
        .map_err(|e| format!("Failed to write rabbitmq.env: {}", e))?;

    tracing::info!("Setup (native): env files written to {}", env_dir.display());
    Ok(())
}

/// Native Step 4: Pull JARs + JRE from GCS (GUI command — emits progress events).
#[tauri::command]
pub async fn setup_pull_jars(app: tauri::AppHandle) -> Result<(), String> {
    setup_pull_jars_run(Some(app)).await
}

/// Core of the JAR-pull step, shared by the GUI command (which passes an
/// `AppHandle` to emit `jar-update-progress`) and the daemon route (which passes
/// `None`).
pub async fn setup_pull_jars_run(app: Option<tauri::AppHandle>) -> Result<(), String> {
    let config = crate::config::load_config().map_err(|e| e.user_message())?;

    // Ensure directories exist
    let jars_dir = config.jars_dir();
    let jres_dir = config.jres_dir();
    let logs_dir = config.native_logs_dir();
    let nginx_dir = config.nginx_html_dir();
    tracing::info!("Setup (native): directories — jars={}, jres={}, logs={}, nginx={}",
        jars_dir.display(), jres_dir.display(), logs_dir.display(), nginx_dir.display());
    for dir in [&jars_dir, &jres_dir, &logs_dir, &nginx_dir] {
        tokio::fs::create_dir_all(dir).await
            .map_err(|e| format!("Failed to create directory {}: {}", dir.display(), e))?;
    }

    // Fetch enabled modules from Firestore
    let modules = if !config.hospital_code.is_empty() {
        match crate::firestore::FirestoreClient::new_from_config().await {
            Ok(client) => {
                match client.fetch_modules(&config.hospital_code).await {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!("Failed to fetch modules from cloud: {}", e);
                        return Err(format!("Cannot fetch service modules from cloud: {}. Configure modules in Oxygen admin first.", e));
                    }
                }
            }
            Err(e) => {
                return Err(format!("Cannot connect to cloud: {}. Check GCS credentials.", e));
            }
        }
    } else {
        return Err("Hospital code not configured. Activate a license first.".into());
    };

    let services = modules.enabled_service_names();
    if services.is_empty() {
        return Err("No services enabled. Configure modules in Oxygen admin (Management → Hospital → edit).".into());
    }

    // Only pull services that aren't deployed yet — i.e. whatever got *added*
    // since the last setup. Already-installed services are left exactly as they
    // are: a setup re-run never overwrites a running JAR. For those we only
    // check whether a newer build exists and report it, so upgrades stay an
    // explicit choice (the Update action), not a side effect of re-running setup.
    let mut to_pull: Vec<String> = Vec::new();
    let mut existing: Vec<String> = Vec::new();
    for svc in &services {
        if crate::services::native::is_installed(&config, svc) {
            existing.push(svc.clone());
        } else {
            to_pull.push(svc.clone());
        }
    }

    if to_pull.is_empty() {
        tracing::info!(
            "Setup (native): no new services to pull ({} already installed)",
            existing.len()
        );
    } else {
        tracing::info!("Setup (native): pulling {} new service(s): {:?}", to_pull.len(), to_pull);

        use tauri::Emitter;
        let app_p = app.clone();
        let on = move |svc: &str, idx: u32, count: u32, d: u64, t: u64| {
            let Some(app_p) = app_p.as_ref() else { return };
            let pct = if t > 0 {
                (d as f64 / t as f64) * 100.0
            } else {
                0.0
            };
            let _ = app_p.emit(
                "jar-update-progress",
                JarUpdateProgress {
                    service: svc.to_string(),
                    phase: "downloading".into(),
                    message: format!("Downloading {} ({}/{})…", svc, idx, count),
                    downloaded: d,
                    total: t,
                    percent: pct,
                    index: idx,
                    count,
                },
            );
        };

        let result = crate::releases::pull_all_with_jres_progress(&to_pull, on)
            .await
            .map_err(|e| e.user_message())?;

        // Signal completion of the JAR download phase to the UI bar.
        if let Some(app) = app.as_ref() {
            let _ = app.emit(
                "jar-update-progress",
                JarUpdateProgress {
                    service: String::new(),
                    phase: "done".into(),
                    message: format!("Downloaded {} service(s)", to_pull.len()),
                    downloaded: 0,
                    total: 0,
                    percent: 100.0,
                    index: to_pull.len() as u32,
                    count: to_pull.len() as u32,
                },
            );
        }

        let succeeded: Vec<&str> = result.results.iter()
            .filter(|r| r.success)
            .map(|r| r.service.as_str())
            .collect();
        let failed: Vec<String> = result.results.iter()
            .filter(|r| !r.success)
            .map(|r| format!("{}: {}", r.service, r.message))
            .collect();

        // Hard failure only when nothing we tried to add could be pulled.
        if succeeded.is_empty() {
            return Err(format!("All new JARs failed to download:\n{}", failed.join("\n")));
        }
        if !failed.is_empty() {
            tracing::warn!("Some new JARs failed: {}", failed.join("; "));
        }
    }

    // Already-installed services: check (don't pull) whether a newer build is
    // available and leave the deployed JAR running untouched.
    if !existing.is_empty() {
        match crate::releases::check_jar_updates_available(&existing).await {
            Ok(checks) => {
                let updatable: Vec<&str> = checks.iter()
                    .filter(|c| c.update_available)
                    .map(|c| c.service.as_str())
                    .collect();
                if updatable.is_empty() {
                    tracing::info!(
                        "Setup (native): {} existing service(s) already up to date",
                        existing.len()
                    );
                } else {
                    tracing::info!(
                        "Setup (native): newer build available for {:?} — left as-is (use the Update action to upgrade)",
                        updatable
                    );
                }
            }
            Err(e) => tracing::warn!("Setup (native): update check skipped: {}", e.user_message()),
        }
    }

    // Verify files on disk
    let mut verified = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&jars_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".jar") && !name.ends_with(".bak") {
                if let Ok(meta) = entry.metadata() {
                    verified.push(format!("{} ({:.1} MB)", name, meta.len() as f64 / 1_048_576.0));
                }
            }
        }
    }

    if verified.is_empty() {
        return Err(format!(
            "Pull reported success but no JAR files found in {}. Check GCS credentials and network.",
            jars_dir.display()
        ));
    }

    tracing::info!("Setup (native): verified {} JARs in {}: {:?}",
        verified.len(), jars_dir.display(), verified);

    Ok(())
}

/// Per-service outcome of the native start/sync step. Surfaced to the UI so the
/// setup wizard can show each service individually.
#[derive(Debug, Clone, serde::Serialize)]
pub struct NativeServiceAction {
    pub service: String,
    /// "started" | "already-running" | "removed" | "not-installed" | "failed"
    pub action: String,
    pub success: bool,
    pub message: String,
}

/// Native Step 5: bring the installed JAR services in line with the cloud
/// config. Enabled services are started (or left running); services that have
/// been **de-selected** (a JAR on disk that's no longer enabled) are stopped and
/// removed. Emits a `native-service-progress` event per service so the UI can
/// show each one as it happens.
#[tauri::command]
pub async fn setup_start_native_services(
    app: tauri::AppHandle,
) -> Result<Vec<NativeServiceAction>, String> {
    sync_native_services(Some(&app)).await
}

/// Core of the native start/sync step, shared by the GUI command (which passes
/// an AppHandle to emit progress) and the daemon route (which passes None).
/// Start or reconcile a single native service. Returns the action taken plus
/// `(attempted, ok)` counters: `attempted` is true when we actually tried to
/// launch a stopped-but-enabled service, `ok` when that launch succeeded.
async fn sync_one_native_service(
    svc: &str,
    enabled: bool,
    config: &crate::config::NucleusConfig,
) -> (NativeServiceAction, bool, bool) {
    if enabled {
        // Enabled and being synced — supersede any prior manual stop.
        crate::services::clear_manually_stopped(svc);
        if crate::services::native::is_running(config, svc) {
            return (
                NativeServiceAction {
                    service: svc.to_string(),
                    action: "already-running".into(),
                    success: true,
                    message: "Already running".into(),
                },
                false,
                false,
            );
        }
        match crate::services::native::start_service(svc, config).await {
            Ok(_) => {
                tracing::info!("Setup (native): started {}", svc);
                (
                    NativeServiceAction {
                        service: svc.to_string(),
                        action: "started".into(),
                        success: true,
                        message: "Started".into(),
                    },
                    true,
                    true,
                )
            }
            Err(e) => {
                tracing::warn!(
                    "Setup (native): failed to start {}: {}",
                    svc,
                    e.user_message()
                );
                (
                    NativeServiceAction {
                        service: svc.to_string(),
                        action: "failed".into(),
                        success: false,
                        message: e.user_message(),
                    },
                    true,
                    false,
                )
            }
        }
    } else {
        // De-selected: stop and remove it (and drop any stop marker — it's gone).
        crate::services::clear_manually_stopped(svc);
        match crate::services::native::remove_service(config, svc).await {
            Ok(_) => {
                tracing::info!("Setup (native): removed de-selected service {}", svc);
                (
                    NativeServiceAction {
                        service: svc.to_string(),
                        action: "removed".into(),
                        success: true,
                        message: "Stopped and removed (no longer enabled)".into(),
                    },
                    false,
                    false,
                )
            }
            Err(e) => (
                NativeServiceAction {
                    service: svc.to_string(),
                    action: "failed".into(),
                    success: false,
                    message: format!("Remove failed: {}", e.user_message()),
                },
                false,
                false,
            ),
        }
    }
}

pub(crate) async fn sync_native_services(
    app: Option<&tauri::AppHandle>,
) -> Result<Vec<NativeServiceAction>, String> {
    use tauri::Emitter;

    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    let jars_dir = config.jars_dir();

    if !jars_dir.exists() {
        return Err(format!(
            "JARs directory does not exist: {}. Run 'Pull JARs' step first.",
            jars_dir.display()
        ));
    }

    // The enabled set is authoritative. If the cloud is unreachable we fall back
    // to None and never remove anything (start-what's-installed only) — tearing
    // a service down on a transient fetch failure would be dangerous.
    let enabled: Option<Vec<String>> = if config.hospital_code.is_empty() {
        None
    } else {
        match crate::firestore::FirestoreClient::new_from_config().await {
            Ok(client) => match client.fetch_modules(&config.hospital_code).await {
                Ok(m) => Some(m.enabled_service_names()),
                Err(e) => {
                    tracing::warn!("Setup (native): module fetch failed, not removing anything: {}", e);
                    None
                }
            },
            Err(e) => {
                tracing::warn!("Setup (native): cloud unreachable, not removing anything: {}", e);
                None
            }
        }
    };

    // Installed JAR services on disk (hydrogen is the static frontend, handled
    // separately by the managed nginx — not a JAR process).
    let mut jar_services: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&jars_dir)
        .map_err(|e| format!("Cannot read jars dir {}: {}", jars_dir.display(), e))?
    {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".jar") && !name.ends_with(".bak") && name != "puru-hydrogen.jar" {
            jar_services.push(name.trim_end_matches(".jar").to_string());
        }
    }

    if jar_services.is_empty() {
        return Err(format!(
            "No JAR files found in {}. Run 'Pull JARs' step first.",
            jars_dir.display()
        ));
    }

    let is_enabled = |svc: &str| -> bool {
        match &enabled {
            Some(set) => set.iter().any(|e| e == svc),
            None => true, // unknown → treat as enabled, never remove
        }
    };

    let mut actions: Vec<NativeServiceAction> = Vec::new();
    let mut start_attempts = 0usize;
    let mut start_ok = 0usize;

    let emit = |app: Option<&tauri::AppHandle>, action: &NativeServiceAction| {
        if let Some(app) = app {
            let _ = app.emit("native-service-progress", action.clone());
        }
    };

    // Startup order is not negotiable: puru-auth (the token authority the rest
    // authenticate against) and puru-hydrogen (the frontend) must come up before
    // any other microservice. We start that first tier, wait for auth to report
    // ready, then bring up everything else. puru-hydrogen is the static frontend
    // (managed nginx), not a JAR, so it isn't in `jar_services` — start it
    // explicitly here when enabled.
    macro_rules! record {
        ($action:expr, $attempted:expr, $ok:expr) => {{
            if $attempted {
                start_attempts += 1;
                if $ok {
                    start_ok += 1;
                }
            }
            emit(app, &$action);
            actions.push($action);
        }};
    }

    let auth_installed = jar_services.iter().any(|s| s == "puru-auth");
    let hydrogen_enabled = is_enabled("puru-hydrogen");

    // ── Tier 1: auth first, then hydrogen ──
    let mut auth_started = false;
    if auth_installed {
        let en = is_enabled("puru-auth");
        let (action, attempted, ok) = sync_one_native_service("puru-auth", en, &config).await;
        auth_started = en && (action.action == "started" || action.action == "already-running");
        record!(action, attempted, ok);
    }
    if hydrogen_enabled {
        let (action, attempted, ok) =
            sync_one_native_service("puru-hydrogen", true, &config).await;
        record!(action, attempted, ok);
    }

    // ── Gate: don't start the rest until auth is actually ready ──
    if auth_started {
        let ready = crate::services::native::wait_for_ready("puru-auth", 90).await;
        let action = NativeServiceAction {
            service: "puru-auth".into(),
            action: if ready { "ready".into() } else { "wait-timeout".into() },
            success: ready,
            message: if ready {
                "Auth is ready — starting remaining services".into()
            } else {
                "Auth not ready after 90s — starting remaining services anyway".into()
            },
        };
        emit(app, &action);
        actions.push(action);
    }

    // ── Tier 2: everything else (skip auth, already handled above) ──
    for svc in jar_services.iter().filter(|s| *s != "puru-auth").cloned().collect::<Vec<_>>() {
        let (action, attempted, ok) =
            sync_one_native_service(&svc, is_enabled(&svc), &config).await;
        record!(action, attempted, ok);
    }

    // Enabled services that have no JAR on disk yet — report so the operator
    // knows the Pull JARs step still needs to run for them.
    if let Some(set) = &enabled {
        for svc in set {
            if svc == "puru-hydrogen" || jar_services.iter().any(|j| j == svc) {
                continue;
            }
            let action = NativeServiceAction {
                service: svc.clone(),
                action: "not-installed".into(),
                success: false,
                message: "Enabled but no JAR — run the Pull JARs step".into(),
            };
            emit(app, &action);
            actions.push(action);
        }
    }

    // Only a hard failure if we tried to start services and every one failed.
    if start_attempts > 0 && start_ok == 0 {
        let reasons: Vec<String> = actions
            .iter()
            .filter(|a| a.action == "failed")
            .map(|a| format!("{}: {}", a.service, a.message))
            .collect();
        return Err(format!("Failed to start any services:\n{}", reasons.join("\n")));
    }

    // Give Spring Boot a moment to initialize before the health-check step.
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

    tracing::info!(
        "Setup (native): sync complete — {} action(s), {} started",
        actions.len(),
        start_ok
    );
    Ok(actions)
}

/// Manually seed fresh-install data: service databases (puru_config, ref_data,
/// charge categories, document master, bootstrap services), RabbitMQ queues, and
/// Jasper report templates. Idempotent — existing values are never overwritten.
///
/// Run this once after the services have booted for the first time (so their
/// tables exist). Passing all-false is treated as "seed everything", matching the
/// `puru seed` CLI with no flags.
#[tauri::command]
pub async fn seed_data(
    db: bool,
    queues: bool,
    templates: bool,
) -> Result<crate::seed::SeedReport, String> {
    let (db, queues, templates) = if !db && !queues && !templates {
        (true, true, true)
    } else {
        (db, queues, templates)
    };

    tracing::info!(
        "Seeding data (db={}, queues={}, templates={})",
        db,
        queues,
        templates
    );

    crate::seed::run_seed(db, queues, templates)
        .await
        .map_err(|e| e.user_message())
}

/// Manually seed master-data catalogues (e.g. radiology service rows). Runs
/// ONLY on explicit user request — never as part of first-run setup. Idempotent
/// on (name, s_class, type) so duplicate names across modalities all land.
#[tauri::command]
pub async fn seed_master_data(
    radiology: bool,
) -> Result<crate::seed::SeedReport, String> {
    tracing::info!("Seeding master data (radiology={})", radiology);
    crate::seed::run_master_data_seed(radiology)
        .await
        .map_err(|e| e.user_message())
}

/// Native setup step: seed RabbitMQ queues. Runs BEFORE services start — the
/// Spring services passively declare their queues at boot and crash if missing.
/// Strict: any queue-declare error fails the step so the operator sees it before
/// the services crash-loop.
#[tauri::command]
pub async fn setup_seed_queues() -> Result<(), String> {
    let report = crate::seed::run_seed(false, true, false)
        .await
        .map_err(|e| e.user_message())?;

    let errs: Vec<String> = report
        .sections
        .iter()
        .flat_map(|s| s.errors.iter().cloned())
        .collect();
    if !errs.is_empty() {
        return Err(format!("Queue seeding failed:\n{}", errs.join("\n")));
    }

    let created: u64 = report.sections.iter().map(|s| s.created).sum();
    let skipped: u64 = report.sections.iter().map(|s| s.skipped).sum();
    tracing::info!(
        "Setup (native): message queues seeded ({} created, {} already present)",
        created,
        skipped
    );
    Ok(())
}

/// Native setup step: initialize auth's roles, privileges, and the root user.
/// Calls auth's own unauthenticated `GET /init1` (the endpoint the front-end
/// "init" flow uses) — runs AFTER auth has started for the first time; the
/// function waits for auth to answer. Idempotent: skips when roles already
/// exist. Returns auth's summary string (includes the root login).
#[tauri::command]
pub async fn setup_init_auth() -> Result<(), String> {
    crate::seed::init_auth_roles()
        .await
        .map(|summary| tracing::info!("Setup (native): auth roles/root user — {}", summary))
        .map_err(|e| e.user_message())
}

/// Native setup step: seed database defaults + Jasper report templates. Runs
/// AFTER services boot (their tables must exist first). Idempotent and
/// supplementary — issues are logged but don't fail the whole setup.
#[tauri::command]
pub async fn setup_seed_database() -> Result<(), String> {
    match crate::seed::run_seed(true, false, true).await {
        Ok(report) => {
            for s in &report.sections {
                if s.errors.is_empty() {
                    tracing::info!(
                        "Setup (native): seed '{}' — {} created, {} skipped",
                        s.name, s.created, s.skipped
                    );
                } else {
                    tracing::warn!(
                        "Setup (native): seed '{}' had {} issue(s): {}",
                        s.name,
                        s.errors.len(),
                        s.errors.join("; ")
                    );
                }
            }
            Ok(())
        }
        Err(e) => {
            tracing::warn!(
                "Setup (native): database/template seed skipped: {}",
                e.user_message()
            );
            Ok(())
        }
    }
}

// ── Hospital template channel ─────────────────────────────────────────────────

/// Upload the finalized local templates to the hospital's GCS folder, making it
/// that hospital's source of truth. Publishes content to the cloud.
#[tauri::command]
pub async fn finalise_templates() -> Result<crate::templates::FinaliseReport, String> {
    crate::templates::with_config(|config| async move {
        crate::templates::finalise(&config).await
    })
    .await
    .map_err(|e| e.user_message())
}

/// Diff the hospital template folder against the local manifest. Returns the
/// new/changed files ("update available") without touching anything.
#[tauri::command]
pub async fn check_template_updates() -> Result<Vec<crate::templates::TemplateUpdate>, String> {
    crate::templates::with_config(|config| async move {
        crate::templates::check_updates(&config).await
    })
    .await
    .map_err(|e| e.user_message())
}

/// Force-overwrite local templates from the hospital folder. Call only after the
/// user has confirmed the overwrite.
#[tauri::command]
pub async fn apply_template_updates() -> Result<crate::templates::ApplyReport, String> {
    crate::templates::with_config(|config| async move {
        crate::templates::apply_updates(&config).await
    })
    .await
    .map_err(|e| e.user_message())
}

/// Step 7: Health check — verify all services are running
#[tauri::command]
pub async fn setup_health_check() -> Result<(), String> {
    tracing::info!("Setup: running health check");

    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    let is_native = config.deployment_mode == crate::config::DeploymentMode::Native;

    // Give services time to start up (Spring Boot takes a while)
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;

    let services = crate::services::get_services()
        .await
        .map_err(|e| e.user_message())?;

    if services.is_empty() {
        let hint = if is_native {
            "No services detected. Check that JARs were downloaded and started."
        } else {
            "No services detected. Ensure Docker is running and services started."
        };
        return Err(hint.to_string());
    }

    let running_count = services
        .iter()
        .filter(|s| s.status == crate::services::ServiceStatus::Running)
        .count();

    let total = services.len();

    if running_count == 0 {
        let logs_hint = if is_native {
            format!("No services are running. Check logs in {}", config.native_logs_dir().display())
        } else {
            "No services are running. Check Docker logs for errors.".to_string()
        };
        return Err(logs_hint);
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
        let mysql_bin = crate::services::resolve_mysql_bin()
            .await
            .unwrap_or_else(|| "mysql".to_string());
        let output = crate::process::silent_cmd(&mysql_bin)
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

/// Step 9: Register puru-dc as a system service (daemon)
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

            // Write the nginx HTTPS server block where it can actually take effect.
            let nginx_config = crate::tls::generate_nginx_https_config(&config.server_ip);
            #[cfg(unix)]
            {
                let nginx_path = std::path::Path::new("/etc/nginx/conf.d/puru-https.conf");
                if let Some(parent) = nginx_path.parent() {
                    tokio::fs::create_dir_all(parent).await.ok();
                }
                match tokio::fs::write(nginx_path, &nginx_config).await {
                    Ok(_) => tracing::info!("Setup: nginx HTTPS config written to {:?}", nginx_path),
                    Err(e) => tracing::warn!("Setup: could not write nginx config: {} (manual setup needed)", e),
                }
            }
            #[cfg(windows)]
            {
                // Native Windows serves via the managed nginx (C:\PuruNucleus\nginx),
                // not /etc/nginx. Drop the HTTPS block into its conf dir for the
                // operator; full activation (certs + a 443 server in puru.conf) is
                // still a manual step on native Windows — do NOT claim it's live.
                let dest = crate::webserver::conf_dir(&config).join("puru-https.conf");
                if let Some(parent) = dest.parent() {
                    tokio::fs::create_dir_all(parent).await.ok();
                }
                match tokio::fs::write(&dest, &nginx_config).await {
                    Ok(_) => tracing::warn!(
                        "Setup: HTTPS config written to {} — activate manually on native Windows (not auto-applied).",
                        dest.display()
                    ),
                    Err(e) => tracing::warn!("Setup: could not write HTTPS config: {}", e),
                }
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

            // Reload nginx so the new config takes effect. On Windows this is the
            // managed nginx (handled above / via webserver::start); the bare
            // `nginx -s reload` only applies to a system nginx on Unix.
            #[cfg(unix)]
            {
                let reload = crate::process::silent_cmd("nginx")
                    .arg("-s").arg("reload")
                    .output().await;
                match reload {
                    Ok(o) if o.status.success() => tracing::info!("Setup: nginx reloaded"),
                    _ => tracing::warn!("Setup: nginx reload failed (restart manually)"),
                }
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

/// Download and install nucleus update
#[tauri::command]
pub async fn download_and_install_nucleus_update(app: tauri::AppHandle) -> Result<String, String> {
    let cfg = crate::config::load_config().map_err(|e| e.to_string())?;

    // Download
    let result = crate::releases::download_nucleus_update(&cfg.release_channel)
        .await
        .map_err(|e| e.to_string())?;

    // Install
    let msg = crate::releases::install_nucleus_update(&result.file_path)
        .await
        .map_err(|e| e.to_string())?;

    // Exit the app after a short delay so the response can be sent back to frontend
    let app_clone = app.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        app_clone.exit(0);
    });

    Ok(msg)
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

/// Whether this process is running elevated (Administrator on Windows / root
/// elsewhere). Used by the GUI to offer a "Restart as Administrator" action.
#[tauri::command]
pub fn is_elevated() -> bool {
    #[cfg(target_os = "windows")]
    {
        // The process token's integrity level reflects elevation:
        // High Mandatory Level (S-1-16-12288) == elevated/admin;
        // Medium (S-1-16-8192) == normal. `whoami /groups` lists it.
        match crate::process::silent_std_cmd("whoami").args(["/groups"]).output() {
            Ok(o) => String::from_utf8_lossy(&o.stdout).contains("S-1-16-12288"),
            Err(_) => false,
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Feature is Windows-only; report elevated elsewhere so the button hides.
        true
    }
}

/// Relaunch the app elevated via UAC, then exit this (non-elevated) instance.
/// Returns an error (and stays running) if the UAC prompt is cancelled.
#[tauri::command]
pub async fn restart_as_admin(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        let exe = std::env::current_exe()
            .map_err(|e| format!("Cannot locate the executable: {}", e))?;
        // Single-quote the path for PowerShell, escaping embedded quotes.
        let exe_arg = format!("'{}'", exe.to_string_lossy().replace('\'', "''"));

        let status = crate::process::silent_cmd("powershell")
            .args([
                "-NoProfile",
                "-Command",
                // `--elevated-restart` tells the new instance that the lock it
                // is about to contend with belongs to *us*, and that we are on
                // our way out — so it waits for the handoff rather than
                // deferring to a GUI that is about to disappear.
                &format!(
                    "Start-Process -FilePath {} -ArgumentList '--elevated-restart' -Verb RunAs",
                    exe_arg
                ),
            ])
            .status()
            .await
            .map_err(|e| format!("Failed to relaunch: {}", e))?;

        if !status.success() {
            // PowerShell exits non-zero when the user declines the UAC prompt.
            return Err("Elevation was cancelled.".into());
        }

        // The elevated instance is launching — close this one.
        app.exit(0);
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
        Err("Restart as administrator is only available on Windows.".into())
    }
}

/// Progress event payload for JAR downloads/updates. Emitted as
/// `jar-update-progress` so the UI can render a real progress bar with a
/// phase label and byte counts. `percent` is a 0..100 overall estimate for the
/// current service; when `total` is 0 during a download the UI should show an
/// indeterminate bar.
#[derive(Clone, Serialize)]
pub struct JarUpdateProgress {
    pub service: String,
    pub phase: String, // "stopping" | "downloading" | "starting" | "done" | "error"
    pub message: String,
    pub downloaded: u64,
    pub total: u64,
    pub percent: f64,
    pub index: u32, // 1-based position within a batch
    pub count: u32, // batch size (1 for a single-service update)
}

/// Update a native service (stop → pull new JAR → start), emitting
/// `jar-update-progress` events throughout so the UI shows a proper progress bar.
#[tauri::command]
pub async fn update_native_service(
    app: tauri::AppHandle,
    service_name: String,
) -> Result<JarPullResult, String> {
    use tauri::Emitter;

    let emit = |phase: &str, message: String, downloaded: u64, total: u64, percent: f64| {
        let _ = app.emit(
            "jar-update-progress",
            JarUpdateProgress {
                service: service_name.clone(),
                phase: phase.to_string(),
                message,
                downloaded,
                total,
                percent,
                index: 1,
                count: 1,
            },
        );
    };

    // Map backend phases → a smooth overall percentage. The download is the long
    // part, so it owns most of the bar (8→90%).
    let on = |phase: &str, d: u64, t: u64| match phase {
        "stopping" => emit("stopping", "Stopping service…".into(), 0, 0, 5.0),
        "downloading" => {
            let pct = if t > 0 {
                8.0 + (d as f64 / t as f64) * 82.0
            } else {
                0.0
            };
            emit("downloading", "Downloading update…".into(), d, t, pct);
        }
        "starting" => emit("starting", "Starting service…".into(), 0, 0, 94.0),
        _ => {}
    };

    match crate::services::update_native_service_progress(&service_name, on).await {
        Ok(r) => {
            emit(
                "done",
                format!("Updated to build {} ({:.1} MB)", r.short_sha, r.size_mb),
                0,
                0,
                100.0,
            );
            Ok(r)
        }
        Err(e) => {
            let msg = e.to_string();
            emit("error", msg.clone(), 0, 0, 100.0);
            Err(msg)
        }
    }
}

/// Rollback a native service to its previous JAR (.bak)
#[tauri::command]
pub async fn rollback_native_service(service_name: String) -> Result<serde_json::Value, String> {
    crate::services::rollback_native_service(&service_name)
        .await
        .map(|_| serde_json::json!({"ok": true}))
        .map_err(|e| e.to_string())
}

// ── Split native update: identify → download (stage) → apply ─────────────────

/// Step 1 (identify): check whether a newer build exists for a single service.
#[tauri::command]
pub async fn check_service_update(service_name: String) -> Result<JarUpdateCheck, String> {
    let checks = crate::releases::check_jar_updates_available(&[service_name.clone()])
        .await
        .map_err(|e| e.to_string())?;
    checks
        .into_iter()
        .find(|c| c.service == service_name)
        .ok_or_else(|| format!("Could not check for updates for '{}'.", service_name))
}

/// Whether a downloaded-but-not-yet-applied update is staged for a service.
#[tauri::command]
pub async fn get_staged_update(service_name: String) -> Result<Option<StagedUpdate>, String> {
    Ok(crate::releases::staged_update_info(&service_name))
}

/// Step 2 (download): stage the latest JAR next to the live one WITHOUT touching
/// the running service. Emits `jar-update-progress` (phase `downloading` →
/// `staged`).
#[tauri::command]
pub async fn download_service_update(
    app: tauri::AppHandle,
    service_name: String,
) -> Result<StagedUpdate, String> {
    use tauri::Emitter;

    let emit = |phase: &str, message: String, downloaded: u64, total: u64, percent: f64| {
        let _ = app.emit(
            "jar-update-progress",
            JarUpdateProgress {
                service: service_name.clone(),
                phase: phase.to_string(),
                message,
                downloaded,
                total,
                percent,
                index: 1,
                count: 1,
            },
        );
    };

    let on = |d: u64, t: u64| {
        let pct = if t > 0 { (d as f64 / t as f64) * 100.0 } else { 0.0 };
        emit("downloading", "Downloading update…".into(), d, t, pct);
    };

    match crate::releases::stage_jar_progress(&service_name, on).await {
        Ok(s) => {
            emit(
                "staged",
                format!("Downloaded build {} ({:.1} MB) — ready to apply", s.short_sha, s.size_mb),
                0,
                0,
                100.0,
            );
            Ok(s)
        }
        Err(e) => {
            let msg = e.to_string();
            emit("error", msg.clone(), 0, 0, 100.0);
            Err(msg)
        }
    }
}

/// Step 3 (apply): stop → swap the staged JAR in → restart. Emits
/// `jar-update-progress` (phase `stopping` → `starting` → `done`).
#[tauri::command]
pub async fn apply_service_update(
    app: tauri::AppHandle,
    service_name: String,
) -> Result<JarPullResult, String> {
    use tauri::Emitter;

    let emit = |phase: &str, message: String, percent: f64| {
        let _ = app.emit(
            "jar-update-progress",
            JarUpdateProgress {
                service: service_name.clone(),
                phase: phase.to_string(),
                message,
                downloaded: 0,
                total: 0,
                percent,
                index: 1,
                count: 1,
            },
        );
    };

    let on = |phase: &str, _d: u64, _t: u64| match phase {
        "stopping" => emit("stopping", "Stopping service…".into(), 25.0),
        "starting" => emit("starting", "Starting service…".into(), 80.0),
        _ => {}
    };

    match crate::services::apply_native_update_progress(&service_name, on).await {
        Ok(r) => {
            emit("done", format!("Applied build {}", r.short_sha), 100.0);
            Ok(r)
        }
        Err(e) => {
            let msg = e.to_string();
            emit("error", msg.clone(), 100.0);
            Err(msg)
        }
    }
}

/// Discard a staged (downloaded, not-applied) update.
#[tauri::command]
pub async fn discard_service_update(service_name: String) -> Result<(), String> {
    crate::releases::discard_staged_jar(&service_name)
        .await
        .map_err(|e| e.to_string())
}

/// A snapshot of a service's JAR manifest — active + pending + history — for
/// the services page. Feeds the "restart to activate" warning banner, the
/// version dropdown for arbitrary-generation rollback, and diagnostics.
#[derive(Debug, Clone, Serialize)]
pub struct JarManifestView {
    pub service: String,
    pub active: Option<crate::services::jar_manifest::JarEntry>,
    pub pending: Option<crate::services::jar_manifest::JarEntry>,
    pub history: Vec<crate::services::jar_manifest::JarEntry>,
    pub updated_at: String,
}

/// Read the JAR manifest for a native-mode service. Returns the empty shape
/// (no active, no pending, no history) when the service has never been pulled.
#[tauri::command]
pub async fn get_jar_manifest(service_name: String) -> Result<JarManifestView, String> {
    let config = crate::config::load_config().map_err(|e| e.to_string())?;
    let manifest =
        crate::services::jar_manifest::JarManifest::read(&config.jars_dir(), &service_name);
    Ok(JarManifestView {
        service: manifest.service,
        active: manifest.active,
        pending: manifest.pending,
        history: manifest.history,
        updated_at: manifest.updated_at,
    })
}

/// A process still holding a service's active JAR — surfaced when apply or
/// GC couldn't free the file. Empty on non-Windows (Restart Manager API is
/// Windows-only).
#[derive(Debug, Clone, Serialize)]
pub struct LockingProcess {
    pub pid: u32,
    pub name: String,
}

/// List processes currently holding the service's active JAR open. Used by
/// the "stuck update" recovery panel — after an apply times out, the UI shows
/// this list so the operator knows what's wedged before deciding to force-kill.
#[tauri::command]
pub async fn list_locking_processes(service_name: String) -> Result<Vec<LockingProcess>, String> {
    let config = crate::config::load_config().map_err(|e| e.to_string())?;
    let jars_dir = config.jars_dir();
    let manifest =
        crate::services::jar_manifest::JarManifest::read(&jars_dir, &service_name);
    let Some(active) = manifest.active else {
        return Ok(Vec::new());
    };
    let path = jars_dir.join(&active.file);
    let pids = crate::file_lock::processes_locking(&path);
    let self_pid = std::process::id();
    let out = pids
        .into_iter()
        .filter(|p| *p != 0 && *p != self_pid)
        .map(|pid| LockingProcess {
            pid,
            name: process_name_for_pid(pid).unwrap_or_default(),
        })
        .collect();
    Ok(out)
}

fn process_name_for_pid(pid: u32) -> Option<String> {
    use sysinfo::{Pid, PidExt, ProcessExt, System, SystemExt};
    let mut sys = System::new();
    let spid = Pid::from_u32(pid);
    sys.refresh_process(spid);
    sys.process(spid).map(|p| p.name().to_string())
}

/// Force-free the active JAR (kill every holder via Windows Restart Manager),
/// then complete any pending promotion by stop → start. Used by the "Force
/// kill and restart" button in the stuck-update UI.
#[tauri::command]
pub async fn force_free_and_restart(service_name: String) -> Result<(), String> {
    let config = crate::config::load_config().map_err(|e| e.to_string())?;
    let jars_dir = config.jars_dir();
    let manifest =
        crate::services::jar_manifest::JarManifest::read(&jars_dir, &service_name);
    if let Some(active) = manifest.active {
        let path = jars_dir.join(&active.file);
        if path.exists() {
            let _killed = crate::file_lock::free_file(&path, 30).await?;
        }
    }
    // Cycle the service so start_service re-reads the manifest and applies
    // any pending promotion via recover_on_start.
    let _ = crate::services::native::stop_service(&service_name, &config).await;
    crate::services::native::start_service(&service_name, &config)
        .await
        .map_err(|e| e.to_string())
}

/// Rollback a native service to a specific historical JAR (by filename, from
/// `manifest.history`). Feeds the version dropdown for arbitrary-generation
/// rollback.
#[tauri::command]
pub async fn rollback_native_service_to(
    service_name: String,
    file: String,
) -> Result<(), String> {
    crate::services::rollback_native_service_to(&service_name, &file)
        .await
        .map_err(|e| e.to_string())
}

// ── Infra (MySQL / RabbitMQ) control + logs ──────────────────────────────────

/// Start / stop / restart the Windows service backing an infra component
/// (name = "MySQL" | "RabbitMQ"; action = "start" | "stop" | "restart").
#[tauri::command]
pub async fn control_infra_service(name: String, action: String) -> Result<String, String> {
    crate::infra::control(&name, &action).await
}

/// Tail the MySQL or RabbitMQ log (the crash reason for a boot failure).
#[tauri::command]
pub async fn get_infra_log(name: String, lines: Option<usize>) -> Result<String, String> {
    crate::infra::read_log(&name, lines.unwrap_or(200))
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
    std::fs::write(&probe, b"puru-dc write test")
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

// ── Process Explorer (Services tab panic-button) ─────────────────────────────

/// List Puru-relevant processes (java / nginx / mysqld / rabbitmq / beam.smp).
/// Used by the Services tab Port Tools panel to surface zombie JVMs holding
/// ports that `stop_service` couldn't free.
#[tauri::command]
pub async fn list_puru_processes() -> Result<Vec<crate::process_explorer::ProcessInfo>, String> {
    Ok(crate::process_explorer::list_processes().await)
}

/// Force-kill any process by PID (taskkill /F /T on Windows, SIGKILL on Unix).
/// Returns the PID actually killed on success.
#[tauri::command]
pub async fn kill_process_by_pid(pid: u32) -> Result<u32, String> {
    crate::process_explorer::kill_pid(pid)
        .await
        .map_err(|e| e.user_message())
}

// ── Performance (JVM memory plan) ────────────────────────────────────────────

/// The current memory plan for this box, with measured RSS folded in for every
/// service that's running so the screen can show plan against reality.
#[tauri::command]
pub async fn get_performance_plan() -> Result<crate::performance::MemoryPlan, String> {
    let config = crate::config::load_config().map_err(|e| e.user_message())?;
    let mut plan = crate::performance::plan(&config);

    // Match processes to services by the label the explorer already derives from
    // the PID file or the jar on the command line.
    let processes = crate::process_explorer::list_processes().await;
    for row in &mut plan.services {
        row.measured_rss_mb = processes
            .iter()
            .find(|p| p.label == row.service)
            .map(|p| p.mem_mb);
    }

    Ok(plan)
}

/// Persist an edited plan. Saving explicit per-service values turns auto-tuning
/// off — otherwise the next start would recompute over the operator's edits.
#[tauri::command]
pub async fn save_performance_config(
    performance: crate::performance::PerformanceConfig,
) -> Result<crate::performance::MemoryPlan, String> {
    let mut config = crate::config::load_config().map_err(|e| e.user_message())?;
    config.performance = performance;
    crate::config::save_config(&config).map_err(|e| e.user_message())?;
    Ok(crate::performance::plan(&config))
}
