//! REST API handler functions for daemon mode.
//! Each handler is a thin wrapper calling shared module functions.

use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::scheduler::AppState;

// ── Request / Response types ─────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct BackupRequest {
    #[serde(rename = "type")]
    pub backup_type: String,
}

#[derive(Deserialize)]
pub struct UpdateRequest {
    pub tag: String,
}

#[derive(Deserialize)]
pub struct ExecRequest {
    pub command: String,
}

#[derive(Deserialize)]
pub struct ScheduleInput {
    pub enabled: Option<bool>,
    pub interval_hours: Option<u32>,
    pub backup_type: Option<String>,
}

#[derive(Deserialize)]
pub struct LogsQuery {
    pub tail: Option<u64>,
    pub since: Option<i64>,
    pub until: Option<i64>,
}

#[derive(Deserialize)]
pub struct RestoreRequest {
    pub backup_id: Option<String>,
    #[serde(default)]
    pub latest: bool,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub uptime_seconds: u64,
    pub docker_connected: bool,
}

// ── Error helper ─────────────────────────────────────────────────────────────

fn internal_err(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

/// Map a unit-returning command result to `{"ok": true}` or a 500.
fn ok_json(r: Result<(), String>) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    r.map(|_| Json(serde_json::json!({ "ok": true })))
        .map_err(internal_err)
}

/// Map any Serialize-returning command result to JSON or a 500, without needing
/// the concrete type at the handler site.
fn to_value_json<T: Serialize>(
    r: Result<T, String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    r.and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string()))
        .map(Json)
        .map_err(internal_err)
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// GET /api/health — public, no auth required
pub async fn health(
    state: axum::extract::State<Arc<AppState>>,
) -> Json<HealthResponse> {
    let docker_connected = match bollard::Docker::connect_with_local_defaults() {
        Ok(docker) => docker.ping().await.is_ok(),
        Err(_) => false,
    };

    Json(HealthResponse {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: state.started_at.elapsed().as_secs(),
        docker_connected,
    })
}

/// GET /api/services
pub async fn list_services() -> Result<Json<Vec<crate::services::ServiceInfo>>, (StatusCode, String)> {
    crate::services::get_services()
        .await
        .map(Json)
        .map_err(|e| internal_err(e.user_message()))
}

/// POST /api/services/:name/start
pub async fn start_service(
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    crate::services::start_service(&name)
        .await
        .map(|_| Json(serde_json::json!({"ok": true})))
        .map_err(|e| internal_err(e.user_message()))
}

/// POST /api/services/:name/stop
pub async fn stop_service(
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    crate::services::stop_service(&name)
        .await
        .map(|_| Json(serde_json::json!({"ok": true})))
        .map_err(|e| internal_err(e.user_message()))
}

/// POST /api/services/:name/restart
pub async fn restart_service(
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    crate::services::restart_service(&name)
        .await
        .map(|_| Json(serde_json::json!({"ok": true})))
        .map_err(|e| internal_err(e.user_message()))
}

/// GET /api/services/:name/logs?tail=N&since=X&until=Y
pub async fn service_logs(
    Path(name): Path<String>,
    Query(params): Query<LogsQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let tail = params.tail.unwrap_or(200);
    crate::services::get_container_logs(&name, tail, params.since, params.until)
        .await
        .map(|logs| Json(serde_json::json!({"logs": logs})))
        .map_err(|e| internal_err(e.user_message()))
}

/// GET /api/system — telemetry snapshot
pub async fn system_info() -> Result<Json<crate::telemetry::TelemetrySnapshot>, (StatusCode, String)> {
    crate::telemetry::collect_snapshot()
        .await
        .map(Json)
        .map_err(|e| internal_err(e.user_message()))
}

/// GET /api/config
pub async fn get_config() -> Result<Json<crate::config::NucleusConfig>, (StatusCode, String)> {
    crate::config::load_config()
        .map(Json)
        .map_err(|e| internal_err(e.user_message()))
}

/// POST /api/backup
pub async fn trigger_backup(
    Json(body): Json<BackupRequest>,
) -> Result<Json<crate::backup::BackupResult>, (StatusCode, String)> {
    let license = crate::licensing::load_license()
        .map_err(|e| internal_err(e.user_message()))?
        .ok_or_else(|| {
            (
                StatusCode::PRECONDITION_FAILED,
                "No license found. Activate a license first.".to_string(),
            )
        })?;

    let backup_type = match body.backup_type.as_str() {
        "partial" => crate::backup::BackupType::Partial,
        _ => crate::backup::BackupType::Full,
    };

    crate::backup::start_backup(backup_type, &license)
        .await
        .map(Json)
        .map_err(|e| internal_err(e.user_message()))
}

/// GET /api/backup/list
pub async fn backup_list() -> Result<Json<Vec<crate::backup::BackupRecord>>, (StatusCode, String)> {
    crate::backup::get_backup_history()
        .await
        .map(Json)
        .map_err(|e| internal_err(e.user_message()))
}

/// GET /api/backup/schedule
pub async fn get_backup_schedule() -> Result<Json<crate::config::BackupSchedule>, (StatusCode, String)> {
    let config = crate::config::load_config().map_err(|e| internal_err(e.user_message()))?;
    let schedule = config
        .daemon
        .map(|d| d.backup_schedule)
        .unwrap_or_default();
    Ok(Json(schedule))
}

/// PUT /api/backup/schedule
pub async fn update_backup_schedule(
    Json(input): Json<ScheduleInput>,
) -> Result<Json<crate::config::BackupSchedule>, (StatusCode, String)> {
    let mut config = crate::config::load_config().map_err(|e| internal_err(e.user_message()))?;
    let mut daemon = config.daemon.unwrap_or_default();

    if let Some(enabled) = input.enabled {
        daemon.backup_schedule.enabled = enabled;
    }
    if let Some(hours) = input.interval_hours {
        daemon.backup_schedule.interval_hours = hours;
    }
    if let Some(bt) = input.backup_type {
        daemon.backup_schedule.backup_type = bt;
    }

    let schedule = daemon.backup_schedule.clone();
    config.daemon = Some(daemon);
    crate::config::save_config(&config).map_err(|e| internal_err(e.user_message()))?;

    Ok(Json(schedule))
}

/// GET /api/updates/check
pub async fn check_updates() -> Result<Json<Vec<crate::releases::ServiceUpdateInfo>>, (StatusCode, String)> {
    crate::releases::check_service_updates()
        .await
        .map(Json)
        .map_err(|e| internal_err(e.user_message()))
}

/// POST /api/updates/:service
pub async fn update_service(
    Path(service): Path<String>,
    Json(body): Json<UpdateRequest>,
) -> Result<Json<crate::docker_update::DockerUpdateResult>, (StatusCode, String)> {
    crate::docker_update::update_service(&service, &body.tag)
        .await
        .map(Json)
        .map_err(|e| internal_err(e.user_message()))
}

/// POST /api/rollback/:service
pub async fn rollback_service(
    Path(service): Path<String>,
) -> Result<Json<crate::docker_update::DockerUpdateResult>, (StatusCode, String)> {
    crate::docker_update::rollback_service(&service)
        .await
        .map(Json)
        .map_err(|e| internal_err(e.user_message()))
}

/// POST /api/exec
pub async fn exec_command(
    Json(body): Json<ExecRequest>,
) -> Result<Json<crate::remote_shell::ShellResult>, (StatusCode, String)> {
    crate::remote_shell::execute_command(&body.command)
        .await
        .map(Json)
        .map_err(|e| internal_err(e.user_message()))
}

/// GET /api/exec/history
pub async fn exec_history() -> Result<Json<Vec<crate::remote_shell::ShellAuditEntry>>, (StatusCode, String)> {
    crate::remote_shell::get_audit_log()
        .await
        .map(Json)
        .map_err(|e| internal_err(e.user_message()))
}

/// GET /api/exec/allowed
pub async fn exec_allowed() -> Json<Vec<String>> {
    Json(crate::remote_shell::get_allowed_commands())
}

/// GET /api/license
pub async fn get_license() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    match crate::licensing::load_license() {
        Ok(Some(license)) => Ok(Json(serde_json::to_value(license).unwrap())),
        Ok(None) => Err((StatusCode::NOT_FOUND, "No license found".to_string())),
        Err(e) => Err(internal_err(e.user_message())),
    }
}

/// GET /api/alerts — list all alerts for this hospital
pub async fn list_alerts() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let config = crate::config::load_config().map_err(|e| internal_err(e.user_message()))?;
    if config.hospital_code.is_empty() {
        return Err((
            StatusCode::PRECONDITION_FAILED,
            "Hospital code not configured".to_string(),
        ));
    }

    let client = crate::firestore::FirestoreClient::new_from_config()
        .await
        .map_err(|e| internal_err(e.user_message()))?;

    let alerts = client
        .list_alerts(&config.hospital_code)
        .await
        .map_err(|e| internal_err(e.user_message()))?;

    // Convert Firestore documents to a simpler JSON structure
    let result: Vec<serde_json::Value> = alerts
        .into_iter()
        .map(|doc| {
            let id = doc.name.rsplit('/').next().unwrap_or("").to_string();
            let mut obj = serde_json::Map::new();
            obj.insert("id".into(), serde_json::json!(id));
            // Include raw fields for flexibility
            obj.insert("fields".into(), serde_json::json!(doc.fields));
            serde_json::Value::Object(obj)
        })
        .collect();

    Ok(Json(serde_json::json!(result)))
}

/// POST /api/alerts/:id/ack — acknowledge an alert
pub async fn acknowledge_alert(
    Path(alert_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let config = crate::config::load_config().map_err(|e| internal_err(e.user_message()))?;
    if config.hospital_code.is_empty() {
        return Err((
            StatusCode::PRECONDITION_FAILED,
            "Hospital code not configured".to_string(),
        ));
    }

    let client = crate::firestore::FirestoreClient::new_from_config()
        .await
        .map_err(|e| internal_err(e.user_message()))?;

    client
        .acknowledge_alert(&config.hospital_code, &alert_id)
        .await
        .map_err(|e| internal_err(e.user_message()))?;

    Ok(Json(serde_json::json!({"ok": true})))
}

/// POST /api/restore — trigger a restore from backup
pub async fn trigger_restore(
    Json(body): Json<RestoreRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let backup_id = if body.latest {
        // Find the latest completed backup
        let records = crate::backup::get_backup_history()
            .await
            .map_err(|e| internal_err(e.user_message()))?;
        records
            .iter()
            .find(|r| r.status == crate::backup::BackupStatus::Completed)
            .map(|r| r.id.clone())
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    "No completed backups found".to_string(),
                )
            })?
    } else if let Some(id) = body.backup_id {
        id
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            "Specify backup_id or set latest: true".to_string(),
        ));
    };

    crate::backup::restore_backup(&backup_id)
        .await
        .map_err(|e| internal_err(e.user_message()))?;

    Ok(Json(serde_json::json!({"ok": true, "backup_id": backup_id})))
}

#[derive(Deserialize)]
pub struct PitrRequest {
    pub backup_id: Option<String>,
    pub stop_datetime: String,
}

/// POST /api/restore/pitr — point-in-time recovery
pub async fn trigger_pitr(
    Json(body): Json<PitrRequest>,
) -> Result<Json<crate::backup::binlog::PitrResult>, (StatusCode, String)> {
    crate::backup::binlog::restore_pitr(body.backup_id.as_deref(), &body.stop_datetime)
        .await
        .map_err(|e| internal_err(e.user_message()))
        .map(Json)
}

// ── Log File Reader ──────────────────────────────────────────────────────────

/// GET /api/logs/sources — list known log source directories AND running
/// containers (containers tagged kind="container", pseudo-path
/// "container:<name>").
pub async fn log_sources() -> Json<Vec<crate::logs::LogSource>> {
    Json(crate::logs::enumerate_sources().await)
}

#[derive(Deserialize)]
pub struct LogFilesQuery {
    pub path: Option<String>,
}

/// GET /api/logs/files?path=... — list log files
pub async fn log_files(
    Query(params): Query<LogFilesQuery>,
) -> Result<Json<Vec<crate::logs::LogFileInfo>>, (StatusCode, String)> {
    if let Some(ref p) = params.path {
        crate::logs::list_log_files(p)
            .map(Json)
            .map_err(|e| internal_err(e.user_message()))
    } else {
        // Aggregate all sources
        let sources = crate::logs::get_known_log_paths();
        let mut all = Vec::new();
        for source in &sources {
            if let Ok(files) = crate::logs::list_log_files(&source.path) {
                all.extend(files);
            }
        }
        all.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
        Ok(Json(all))
    }
}

#[derive(Deserialize)]
pub struct LogFileReadQuery {
    pub path: String,
    pub tail: Option<usize>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

/// GET /api/logs/file?path=...&tail=100 — read a log file. Accepts host
/// paths under the allowed-base-dirs whitelist and `container:<name>`
/// pseudo-paths for Docker container stdout/stderr.
pub async fn log_file_read(
    Query(params): Query<LogFileReadQuery>,
) -> Result<Json<crate::logs::LogFileContent>, (StatusCode, String)> {
    crate::logs::read_log_file(&params.path, params.tail, params.offset, params.limit)
        .await
        .map(Json)
        .map_err(|e| internal_err(e.user_message()))
}

/// POST /api/pull — pull hospital settings from Firestore
pub async fn pull_settings() -> Result<Json<crate::commands::PullSettingsResult>, (StatusCode, String)> {
    let config = crate::config::load_config().map_err(|e| internal_err(e.user_message()))?;
    if config.hospital_code.is_empty() {
        return Err((
            StatusCode::PRECONDITION_FAILED,
            "Hospital code not configured".to_string(),
        ));
    }

    let client = crate::firestore::FirestoreClient::new_from_config()
        .await
        .map_err(|e| internal_err(e.user_message()))?;

    let doc = client
        .get_hospital(&config.hospital_code)
        .await
        .map_err(|e| internal_err(e.user_message()))?;

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
        true
    } else {
        current_fp
            .as_ref()
            .map(|fp| machines.iter().any(|(m_fp, _)| m_fp == fp))
            .unwrap_or(false)
    };

    let current_license = crate::licensing::load_license()
        .map_err(|e| internal_err(e.user_message()))?;

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

    // Preserve local machine fields
    if let Some(ref current) = current_license {
        pulled_license.machine_fingerprint = current.machine_fingerprint.clone();
        pulled_license.machine_name = current.machine_name.clone();
    }

    if license_changed {
        crate::licensing::save_license(&pulled_license)
            .map_err(|e| internal_err(e.user_message()))?;
    }

    Ok(Json(crate::commands::PullSettingsResult {
        hospital_info,
        license: pulled_license,
        license_changed,
        machine_registered,
    }))
}

/// GET /api/network — quick connectivity check
pub async fn network_check() -> Result<Json<crate::network::NetworkStatus>, (StatusCode, String)> {
    crate::network::check_network()
        .await
        .map(Json)
        .map_err(|e| internal_err(e.user_message()))
}

/// POST /api/network/speedtest — full speed test
pub async fn network_speed_test() -> Result<Json<crate::network::SpeedTestResult>, (StatusCode, String)> {
    crate::network::run_speed_test()
        .await
        .map(Json)
        .map_err(|e| internal_err(e.user_message()))
}

// ── Seed ─────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SeedRequest {
    #[serde(default)]
    pub db: Option<bool>,
    #[serde(default)]
    pub queues: Option<bool>,
    #[serde(default)]
    pub templates: Option<bool>,
}

/// POST /api/seed — seed databases, RabbitMQ queues, and report templates.
/// Idempotent: existing values are never overwritten. Omitting all flags
/// (or an empty body) seeds everything.
pub async fn run_seed(
    body: Option<Json<SeedRequest>>,
) -> Result<Json<crate::seed::SeedReport>, (StatusCode, String)> {
    let req = body.map(|Json(r)| r).unwrap_or(SeedRequest {
        db: None,
        queues: None,
        templates: None,
    });
    let all = req.db.is_none() && req.queues.is_none() && req.templates.is_none();
    let do_db = req.db.unwrap_or(all);
    let do_queues = req.queues.unwrap_or(all);
    let do_templates = req.templates.unwrap_or(all);

    crate::seed::run_seed(do_db, do_queues, do_templates)
        .await
        .map(Json)
        .map_err(|e| internal_err(e.user_message()))
}

#[derive(Deserialize)]
pub struct SeedMasterDataRequest {
    #[serde(default)]
    pub radiology: Option<bool>,
}

/// POST /api/seed/master-data — seed master-data catalogues (e.g. radiology
/// services). User-triggered; never runs on first-install. Omitting the flag
/// (or an empty body) seeds every available catalogue.
pub async fn run_seed_master_data(
    body: Option<Json<SeedMasterDataRequest>>,
) -> Result<Json<crate::seed::SeedReport>, (StatusCode, String)> {
    let req = body.map(|Json(r)| r).unwrap_or(SeedMasterDataRequest {
        radiology: None,
    });
    let all = req.radiology.is_none();
    let do_radiology = req.radiology.unwrap_or(all);

    crate::seed::run_master_data_seed(do_radiology)
        .await
        .map(Json)
        .map_err(|e| internal_err(e.user_message()))
}

// ── Remote setup / detection / templates / config ─────────────────────────────
// These call the same #[tauri::command] functions the local GUI uses, so the
// remote setup wizard runs identical logic on the daemon host. setup_install_daemon
// is deliberately NOT exposed — the daemon is already installed where it runs.

pub async fn setup_check_prerequisites() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ok_json(crate::commands::setup_check_prerequisites().await)
}
pub async fn setup_create_databases() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ok_json(crate::commands::setup_create_databases().await)
}
pub async fn setup_configure_rabbitmq() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ok_json(crate::commands::setup_configure_rabbitmq().await)
}
pub async fn setup_generate_config() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ok_json(crate::commands::setup_generate_config().await)
}
pub async fn setup_pull_images() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ok_json(crate::commands::setup_pull_images().await)
}
pub async fn setup_start_services() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ok_json(crate::commands::setup_start_services().await)
}
pub async fn setup_generate_env_files() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ok_json(crate::commands::setup_generate_env_files().await)
}
pub async fn setup_pull_jars() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ok_json(crate::commands::setup_pull_jars().await)
}
pub async fn setup_start_native_services() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    to_value_json(crate::commands::sync_native_services(None).await)
}
pub async fn setup_seed_queues() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ok_json(crate::commands::setup_seed_queues().await)
}
pub async fn setup_seed_database() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ok_json(crate::commands::setup_seed_database().await)
}
pub async fn setup_health_check() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ok_json(crate::commands::setup_health_check().await)
}
pub async fn setup_configure_backups() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ok_json(crate::commands::setup_configure_backups().await)
}
pub async fn setup_tls() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ok_json(crate::commands::setup_tls().await)
}
pub async fn setup_reset() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    to_value_json(crate::commands::setup_reset().await)
}

pub async fn detect_existing_setup() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    to_value_json(crate::commands::detect_existing_setup().await)
}
pub async fn check_prerequisites() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    to_value_json(crate::commands::check_prerequisites().await)
}
pub async fn get_deployment_mode() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    to_value_json(crate::commands::get_deployment_mode().await)
}
pub async fn get_service_modules() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    to_value_json(crate::commands::get_service_modules().await)
}
pub async fn save_config(
    Json(config): Json<crate::config::NucleusConfig>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    ok_json(crate::commands::save_config(config).await)
}

pub async fn finalise_templates() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    to_value_json(crate::commands::finalise_templates().await)
}
pub async fn check_template_updates() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    to_value_json(crate::commands::check_template_updates().await)
}
pub async fn apply_template_updates() -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    to_value_json(crate::commands::apply_template_updates().await)
}

// ── Native JAR Deployment ────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct PullJarsRequest {
    #[serde(default)]
    pub services: Option<Vec<String>>,
}

/// POST /api/jars/pull — pull latest JARs from GCS
pub async fn pull_jars(
    Json(body): Json<PullJarsRequest>,
) -> Result<Json<crate::releases::PullAllResult>, (StatusCode, String)> {
    let services = body
        .services
        .unwrap_or_else(crate::releases::all_updatable_services);

    crate::releases::pull_all_with_jres(&services)
        .await
        .map(Json)
        .map_err(|e| internal_err(e.user_message()))
}

/// GET /api/jars/updates — check for available JAR updates
pub async fn check_jar_updates() -> Result<Json<Vec<crate::releases::JarUpdateCheck>>, (StatusCode, String)> {
    let services = crate::releases::all_updatable_services();

    crate::releases::check_jar_updates_available(&services)
        .await
        .map(Json)
        .map_err(|e| internal_err(e.user_message()))
}

/// POST /api/jars/update/:service — update a native service (stop → pull → start)
pub async fn update_native_service(
    Path(service): Path<String>,
) -> Result<Json<crate::releases::JarPullResult>, (StatusCode, String)> {
    crate::services::update_native_service(&service)
        .await
        .map(Json)
        .map_err(|e| internal_err(e.user_message()))
}

/// POST /api/jars/rollback/:service — rollback a native service to previous JAR
pub async fn rollback_native_service(
    Path(service): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    crate::services::rollback_native_service(&service)
        .await
        .map(|_| Json(serde_json::json!({"ok": true})))
        .map_err(|e| internal_err(e.user_message()))
}

// ── LAN Binlog ──────────────────────────────────────────────────────────────

/// POST /api/lan/binlog/ship — ship binlog files to LAN
pub async fn ship_binlogs_lan() -> Result<Json<crate::backup::binlog::BinlogShipResult>, (StatusCode, String)> {
    let license = crate::licensing::load_license()
        .map_err(|e| internal_err(e.user_message()))?
        .ok_or_else(|| {
            (
                StatusCode::PRECONDITION_FAILED,
                "No license found. Activate a license first.".to_string(),
            )
        })?;

    crate::backup::binlog::ship_binlogs_to_lan(&license)
        .await
        .map(Json)
        .map_err(|e| internal_err(e.user_message()))
}
