//! Background task scheduler for daemon mode — backup scheduling, status reporting, and command listening

use chrono::{DateTime, Local, TimeZone, Utc};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

use crate::config::{BackupSchedule, DaemonConfig};
use crate::firestore::convert::{get_map_fields, get_optional_string, get_string};

/// Shared application state for the daemon
pub struct AppState {
    pub api_key: String,
    pub port: u16,
    pub started_at: Instant,
    pub started_at_utc: DateTime<Utc>,
}

/// Start all background tasks. Returns join handles for cleanup.
pub fn start_all(daemon_cfg: &DaemonConfig, telemetry_enabled: bool) -> Vec<JoinHandle<()>> {
    let mut handles = Vec::new();

    // Backup scheduler
    let schedule = daemon_cfg.backup_schedule.clone();
    handles.push(tokio::spawn(async move {
        backup_scheduler(schedule).await;
    }));

    // Status reporter (replaces telemetry reporter)
    if telemetry_enabled && daemon_cfg.telemetry_interval_minutes > 0 {
        let interval_mins = daemon_cfg.telemetry_interval_minutes;
        let port = daemon_cfg.port;
        handles.push(tokio::spawn(async move {
            status_reporter(interval_mins, port).await;
        }));
    }

    // Command listener (always enabled)
    handles.push(tokio::spawn(async move {
        command_listener().await;
    }));

    // Message poller (always enabled)
    handles.push(tokio::spawn(async move {
        message_poller().await;
    }));

    // Watchdog — monitors service health and system resources (always enabled)
    handles.push(tokio::spawn(async move {
        watchdog_loop().await;
    }));

    // LAN binlog shipping loop
    let ship_interval = daemon_cfg.backup_schedule.interval_hours.max(1);
    handles.push(tokio::spawn(async move {
        lan_binlog_shipping_loop(ship_interval).await;
    }));

    handles
}

/// Periodic backup task
async fn backup_scheduler(schedule: BackupSchedule) {
    if !schedule.enabled {
        tracing::info!("Backup scheduler disabled");
        return;
    }

    // Determine scheduling mode
    if let Some(ref time_str) = schedule.backup_time {
        // Time-based: run once daily at the specified time
        backup_scheduler_timed(time_str, &schedule).await;
    } else if schedule.interval_hours > 0 {
        // Interval-based: run every N hours (legacy)
        backup_scheduler_interval(&schedule).await;
    } else {
        tracing::info!("Backup scheduler: no schedule configured");
    }
}

/// Run backup at a fixed time each day (e.g. "02:00")
async fn backup_scheduler_timed(time_str: &str, schedule: &BackupSchedule) {
    let parts: Vec<&str> = time_str.split(':').collect();
    let (target_hour, target_minute) = match (parts.first(), parts.get(1)) {
        (Some(h), Some(m)) => {
            match (h.parse::<u32>(), m.parse::<u32>()) {
                (Ok(h), Ok(m)) if h < 24 && m < 60 => (h, m),
                _ => {
                    tracing::error!("Backup scheduler: invalid backup_time '{}', expected HH:MM", time_str);
                    return;
                }
            }
        }
        _ => {
            tracing::error!("Backup scheduler: invalid backup_time '{}', expected HH:MM", time_str);
            return;
        }
    };

    tracing::info!(
        "Backup scheduler started: daily at {:02}:{:02}, type={}",
        target_hour, target_minute, schedule.backup_type
    );

    loop {
        // Calculate sleep duration until next target time
        let now = chrono::Local::now();
        let today_target = now.date_naive()
            .and_hms_opt(target_hour, target_minute, 0)
            .unwrap();
        let today_target = chrono::Local.from_local_datetime(&today_target).unwrap();

        let next_run = if now >= today_target {
            // Already past today's time — schedule for tomorrow
            today_target + chrono::Duration::days(1)
        } else {
            today_target
        };

        let sleep_secs = (next_run - now).num_seconds().max(1) as u64;
        tracing::info!(
            "Backup scheduler: next backup at {} (in {}h {}m)",
            next_run.format("%Y-%m-%d %H:%M"),
            sleep_secs / 3600,
            (sleep_secs % 3600) / 60
        );

        tokio::time::sleep(Duration::from_secs(sleep_secs)).await;
        run_scheduled_backup(schedule).await;
    }
}

/// Run backup every N hours (interval-based)
async fn backup_scheduler_interval(schedule: &BackupSchedule) {
    let interval = Duration::from_secs(schedule.interval_hours as u64 * 3600);
    tracing::info!(
        "Backup scheduler started: every {}h, type={}",
        schedule.interval_hours, schedule.backup_type
    );

    let mut tick = tokio::time::interval(interval);
    tick.tick().await; // skip first immediate tick

    loop {
        tick.tick().await;
        run_scheduled_backup(schedule).await;
    }
}

/// Execute a single scheduled backup
async fn run_scheduled_backup(schedule: &BackupSchedule) {
    let license = match crate::licensing::load_license() {
        Ok(Some(l)) if l.is_valid() => l,
        Ok(Some(_)) => {
            tracing::warn!("Backup scheduler: license expired, skipping backup");
            return;
        }
        Ok(None) => {
            tracing::warn!("Backup scheduler: no license found, skipping backup");
            return;
        }
        Err(e) => {
            tracing::error!("Backup scheduler: failed to load license: {}", e);
            return;
        }
    };

    let backup_type = match schedule.backup_type.as_str() {
        "partial" => crate::backup::BackupType::Partial,
        _ => crate::backup::BackupType::Full,
    };

    tracing::info!("Backup scheduler: starting {:?} backup", backup_type);
    match crate::backup::start_backup(backup_type, &license).await {
        Ok(result) => {
            tracing::info!(
                "Backup scheduler: completed in {}s, size={}MB, id={}",
                result.duration_seconds, result.size_mb, result.backup_id
            );
        }
        Err(e) => {
            tracing::error!("Backup scheduler: backup failed: {}", e);
        }
    }
}

/// Enriched status reporter — pushes telemetry, services, backup summary, and daemon info.
async fn status_reporter(interval_minutes: u32, port: u16) {
    if interval_minutes == 0 {
        return;
    }

    let started_at = Instant::now();
    let interval = Duration::from_secs(interval_minutes as u64 * 60);
    tracing::info!(
        "Status reporter started: every {}min",
        interval_minutes
    );

    let mut tick = tokio::time::interval(interval);
    // Skip the first immediate tick
    tick.tick().await;

    loop {
        tick.tick().await;

        // Collect telemetry snapshot
        let snapshot = match crate::telemetry::collect_snapshot().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Status reporter: failed to collect snapshot: {}", e);
                continue;
            }
        };

        // Load config
        let config = match crate::config::load_config() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Status reporter: failed to load config: {}", e);
                continue;
            }
        };

        if config.hospital_code.is_empty() {
            tracing::warn!("Status reporter: hospital code not set, skipping");
            continue;
        }

        // Build DaemonInfo from backup history and config
        let backup_schedule = config
            .daemon
            .as_ref()
            .map(|d| &d.backup_schedule);
        let scheduled_backups_enabled = backup_schedule
            .map(|s| s.enabled)
            .unwrap_or(false);

        let backup_history = crate::backup::get_backup_history()
            .await
            .unwrap_or_default();
        let total_backups = backup_history.len();
        let last_backup = backup_history.first(); // sorted descending by date

        let daemon_info = crate::firestore::DaemonInfo {
            port,
            uptime_seconds: started_at.elapsed().as_secs(),
            scheduled_backups_enabled,
            last_backup_at: last_backup.map(|b| b.created_at.to_rfc3339()),
            last_backup_status: last_backup
                .map(|b| format!("{:?}", b.status).to_lowercase())
                .unwrap_or_else(|| "none".into()),
            total_backups,
        };

        // Get current services
        let services = match crate::services::get_services().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Status reporter: failed to get services: {}", e);
                Vec::new()
            }
        };

        // Create Firestore client and push
        let client = match crate::firestore::FirestoreClient::new_from_config().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Status reporter: Firestore unavailable: {}", e);
                continue;
            }
        };

        if let Err(e) = client
            .push_status(&config.hospital_code, &snapshot, &daemon_info, &services)
            .await
        {
            tracing::warn!("Status reporter: push failed: {}", e);
        } else {
            tracing::debug!("Status reporter: status pushed");
        }
    }
}

/// Command listener — polls Firestore for pending commands every 10 seconds.
async fn command_listener() {
    let interval = Duration::from_secs(10);
    tracing::info!("Command listener started: polling every 10s");

    let mut tick = tokio::time::interval(interval);
    // Skip the first immediate tick
    tick.tick().await;

    loop {
        tick.tick().await;

        // Load config
        let config = match crate::config::load_config() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Command listener: failed to load config: {}", e);
                continue;
            }
        };

        if config.hospital_code.is_empty() {
            continue; // Silently skip — no hospital code configured yet
        }

        let client = match crate::firestore::FirestoreClient::new_from_config().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Command listener: Firestore unavailable: {}", e);
                continue;
            }
        };

        // Poll for pending commands
        let commands = match client.poll_pending_commands(&config.hospital_code).await {
            Ok(docs) => docs,
            Err(e) => {
                tracing::warn!("Command listener: poll failed: {}", e);
                continue;
            }
        };

        for doc in commands {
            let command_id = match extract_document_id(&doc.name) {
                Some(id) => id,
                None => {
                    tracing::warn!("Command listener: could not extract ID from {}", doc.name);
                    continue;
                }
            };

            // TTL check: skip commands older than 5 minutes
            if is_command_expired(&doc) {
                tracing::warn!("Command listener: command {} expired, marking as failed", command_id);
                let _ = client
                    .update_command_status(
                        &config.hospital_code,
                        &command_id,
                        "failed",
                        None,
                        Some("Command expired (older than 5 minutes)"),
                    )
                    .await;
                continue;
            }

            // Extract command type
            let command_type = match doc.fields.get("type").and_then(|v| get_string(v).ok()) {
                Some(t) => t,
                None => {
                    let _ = client
                        .update_command_status(
                            &config.hospital_code,
                            &command_id,
                            "failed",
                            None,
                            Some("Malformed command: missing 'type' field"),
                        )
                        .await;
                    continue;
                }
            };

            // Extract params map (optional)
            let params = doc
                .fields
                .get("params")
                .and_then(|v| get_map_fields(v).ok())
                .cloned()
                .unwrap_or_default();

            // Mark as executing
            if let Err(e) = client
                .update_command_status(
                    &config.hospital_code,
                    &command_id,
                    "executing",
                    None,
                    None,
                )
                .await
            {
                tracing::warn!("Command listener: failed to mark {} as executing: {}", command_id, e);
                continue;
            }

            tracing::info!("Command listener: executing {} (type={})", command_id, command_type);

            // Execute the command
            let result = super::commands::execute(&command_type, &params).await;

            // Write result back
            let status = if result.success { "completed" } else { "failed" };
            let result_msg = if result.success { Some(result.message.as_str()) } else { None };
            let error_msg = if !result.success { Some(result.message.as_str()) } else { None };

            if let Err(e) = client
                .update_command_status(
                    &config.hospital_code,
                    &command_id,
                    status,
                    result_msg,
                    error_msg,
                )
                .await
            {
                tracing::warn!("Command listener: failed to write result for {}: {}", command_id, e);
            } else {
                tracing::info!(
                    "Command listener: {} {} — {}",
                    command_id,
                    status,
                    result.message
                );
            }
        }
    }
}

/// Message poller — polls Firestore inbox for new messages every 60 seconds.
async fn message_poller() {
    let interval = Duration::from_secs(60);
    tracing::info!("Message poller started: polling every 60s");

    let mut seen_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    let mut tick = tokio::time::interval(interval);
    // Skip the first immediate tick
    tick.tick().await;

    loop {
        tick.tick().await;

        // Load config
        let config = match crate::config::load_config() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Message poller: failed to load config: {}", e);
                continue;
            }
        };

        if config.hospital_code.is_empty() {
            continue; // Silently skip — no hospital code configured yet
        }

        let messages = match crate::messaging::service::get_messages(50).await {
            Ok(msgs) => msgs,
            Err(e) => {
                tracing::warn!("Message poller: failed to fetch messages: {}", e);
                continue;
            }
        };

        for msg in &messages {
            if !msg.read && !seen_ids.contains(&msg.id) {
                tracing::info!(
                    "Message poller: new message [{}] {:?} — {}",
                    msg.id,
                    msg.message_type,
                    msg.subject
                );
            }
            seen_ids.insert(msg.id.clone());
        }

        tracing::debug!(
            "Message poller: {} messages total, {} unread",
            messages.len(),
            messages.iter().filter(|m| !m.read).count()
        );
    }
}

// ── LAN binlog shipping ─────────────────────────────────────────────────────

/// Periodic LAN binlog shipping — ships binlog files to LAN network share
async fn lan_binlog_shipping_loop(interval_hours: u32) {
    let interval = Duration::from_secs(interval_hours as u64 * 3600);
    tracing::info!("LAN binlog shipping loop started: every {}h", interval_hours);

    let mut tick = tokio::time::interval(interval);
    // Skip the first immediate tick
    tick.tick().await;

    loop {
        tick.tick().await;

        // Re-read config each cycle to check if LAN binlog is enabled
        let cfg = match crate::config::load_config() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("LAN binlog shipping: failed to load config: {}", e);
                continue;
            }
        };

        if !cfg.lan.enabled || !cfg.lan.binlog_enabled || cfg.lan.path.is_empty() {
            tracing::debug!("LAN binlog shipping: disabled, skipping");
            continue;
        }

        // Check license
        let license = match crate::licensing::load_license() {
            Ok(Some(l)) if l.is_valid() => l,
            Ok(Some(_)) => {
                tracing::warn!("LAN binlog shipping: license expired, skipping");
                continue;
            }
            Ok(None) => {
                tracing::warn!("LAN binlog shipping: no license found, skipping");
                continue;
            }
            Err(e) => {
                tracing::error!("LAN binlog shipping: failed to load license: {}", e);
                continue;
            }
        };

        match crate::backup::binlog::ship_binlogs_to_lan(&license).await {
            Ok(result) => {
                if result.files_shipped > 0 {
                    tracing::info!(
                        "LAN binlog shipping: shipped {} files ({} bytes) in {}s",
                        result.files_shipped,
                        result.bytes_shipped,
                        result.duration_seconds
                    );
                }
            }
            Err(e) => {
                tracing::error!("LAN binlog shipping: failed: {}", e);
            }
        }
    }
}

// ── Watchdog ────────────────────────────────────────────────────────────────

/// Thresholds for system resource alerts
const DISK_CRITICAL_PERCENT: f64 = 90.0;
const DISK_WARNING_PERCENT: f64 = 80.0;
const RAM_WARNING_GB_FREE: f64 = 0.5;

/// Watchdog loop — monitors service health and system resources every 60 seconds.
///
/// Responsibilities:
/// 1. Detect stopped/unhealthy services and auto-restart them
/// 2. Monitor disk usage and generate alerts when thresholds are exceeded
/// 3. Monitor RAM usage and generate alerts when critically low
/// 4. Push alerts to Firestore for visibility in puru-oxygen
async fn watchdog_loop() {
    let interval = Duration::from_secs(60);
    tracing::info!("Watchdog started: checking every 60s");

    // Track which services we've already alerted about (to avoid alert spam)
    let mut alerted_services: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut disk_alerted = false;
    let mut ram_alerted = false;

    let mut tick = tokio::time::interval(interval);
    // Skip the first immediate tick
    tick.tick().await;

    loop {
        tick.tick().await;

        // Load config
        let config = match crate::config::load_config() {
            Ok(c) => c,
            Err(_) => continue,
        };

        if config.hospital_code.is_empty() {
            continue; // No hospital configured yet
        }

        // ── Service health check & auto-restart ─────────────────────────
        match crate::services::get_services().await {
            Ok(services) => {
                for svc in &services {
                    let is_down = svc.status == crate::services::ServiceStatus::Stopped
                        || svc.status == crate::services::ServiceStatus::Error;
                    let is_unhealthy = matches!(
                        svc.health,
                        Some(crate::services::HealthStatus::Unhealthy)
                    );

                    if is_down || is_unhealthy {
                        let reason = if is_down {
                            format!("{} is {:?}", svc.name, svc.status)
                        } else {
                            format!("{} is unhealthy", svc.name)
                        };

                        tracing::warn!("Watchdog: {} — attempting restart", reason);

                        // Attempt auto-restart
                        match crate::services::start_service(&svc.container_name).await {
                            Ok(()) => {
                                tracing::info!(
                                    "Watchdog: auto-restarted {}",
                                    svc.container_name
                                );

                                // Push alert only on first detection (not on every cycle)
                                if !alerted_services.contains(&svc.container_name) {
                                    push_watchdog_alert(
                                        &config.hospital_code,
                                        "warning",
                                        "service_restart",
                                        &format!("Auto-restarted: {}", svc.name),
                                        &format!(
                                            "Service {} was {} and was automatically restarted by the watchdog.",
                                            svc.name,
                                            if is_down { "stopped" } else { "unhealthy" }
                                        ),
                                    )
                                    .await;
                                    alerted_services.insert(svc.container_name.clone());
                                }
                            }
                            Err(e) => {
                                tracing::error!(
                                    "Watchdog: failed to restart {}: {}",
                                    svc.container_name,
                                    e
                                );

                                if !alerted_services.contains(&svc.container_name) {
                                    push_watchdog_alert(
                                        &config.hospital_code,
                                        "critical",
                                        "service_down",
                                        &format!("Service down: {}", svc.name),
                                        &format!(
                                            "Service {} is {} and auto-restart failed: {}",
                                            svc.name,
                                            if is_down { "stopped" } else { "unhealthy" },
                                            e
                                        ),
                                    )
                                    .await;
                                    alerted_services.insert(svc.container_name.clone());
                                }
                            }
                        }
                    } else if svc.status == crate::services::ServiceStatus::Running {
                        // Service recovered — clear its alert state
                        if alerted_services.remove(&svc.container_name) {
                            tracing::info!(
                                "Watchdog: {} recovered, clearing alert state",
                                svc.name
                            );
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Watchdog: failed to get services: {}", e);
            }
        }

        // ── System resource checks ──────────────────────────────────────
        if let Ok(snapshot) = crate::telemetry::collect_snapshot().await {
            // Disk usage check
            if snapshot.disk_percent >= DISK_CRITICAL_PERCENT {
                if !disk_alerted {
                    tracing::error!(
                        "Watchdog: CRITICAL disk usage {:.1}%",
                        snapshot.disk_percent
                    );
                    push_watchdog_alert(
                        &config.hospital_code,
                        "critical",
                        "disk_space",
                        "Critical: Disk space low",
                        &format!(
                            "Disk usage is at {:.1}%. Free up space immediately to prevent data loss.",
                            snapshot.disk_percent
                        ),
                    )
                    .await;
                    disk_alerted = true;
                }
            } else if snapshot.disk_percent >= DISK_WARNING_PERCENT {
                if !disk_alerted {
                    tracing::warn!(
                        "Watchdog: disk usage warning {:.1}%",
                        snapshot.disk_percent
                    );
                    push_watchdog_alert(
                        &config.hospital_code,
                        "warning",
                        "disk_space",
                        "Warning: Disk space getting low",
                        &format!(
                            "Disk usage is at {:.1}%. Consider freeing up space or expanding storage.",
                            snapshot.disk_percent
                        ),
                    )
                    .await;
                    disk_alerted = true;
                }
            } else if disk_alerted {
                // Disk usage back to normal
                disk_alerted = false;
                tracing::info!(
                    "Watchdog: disk usage back to normal ({:.1}%)",
                    snapshot.disk_percent
                );
            }

            // RAM check
            // Estimate total RAM from sysinfo (ram_gb is used memory)
            let total_ram_gb = {
                use sysinfo::{System, SystemExt};
                let sys = System::new_all();
                sys.total_memory() as f64 / 1024.0 / 1024.0 / 1024.0
            };
            let free_ram_gb = total_ram_gb - snapshot.ram_gb;

            if free_ram_gb < RAM_WARNING_GB_FREE && free_ram_gb >= 0.0 {
                if !ram_alerted {
                    tracing::warn!(
                        "Watchdog: low RAM — {:.2} GB free of {:.1} GB total",
                        free_ram_gb,
                        total_ram_gb
                    );
                    push_watchdog_alert(
                        &config.hospital_code,
                        "warning",
                        "memory",
                        "Warning: Low memory",
                        &format!(
                            "Only {:.2} GB RAM free out of {:.1} GB total. Services may become slow or crash.",
                            free_ram_gb, total_ram_gb
                        ),
                    )
                    .await;
                    ram_alerted = true;
                }
            } else if ram_alerted {
                ram_alerted = false;
                tracing::info!(
                    "Watchdog: RAM recovered ({:.2} GB free)",
                    free_ram_gb
                );
            }
        }
    }
}

/// Push an alert to Firestore (best-effort, never blocks the watchdog loop).
async fn push_watchdog_alert(
    hospital_code: &str,
    severity: &str,
    category: &str,
    title: &str,
    message: &str,
) {
    let client = match crate::firestore::FirestoreClient::new_from_config().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Watchdog alert: Firestore unavailable: {}", e);
            return;
        }
    };

    match client
        .push_alert(hospital_code, severity, category, title, message)
        .await
    {
        Ok(id) => {
            tracing::info!("Watchdog alert pushed: {} ({})", title, id);
        }
        Err(e) => {
            tracing::warn!("Watchdog alert: failed to push: {}", e);
        }
    }
}

/// Extract the document ID from a full Firestore document name.
/// e.g. "projects/puru-255206/databases/(default)/documents/hospital/ABC/commands/xyz123" -> "xyz123"
fn extract_document_id(name: &str) -> Option<String> {
    name.rsplit('/').next().map(|s| s.to_string())
}

/// Check if a command's `created_at` is older than 5 minutes.
fn is_command_expired(doc: &crate::firestore::types::FirestoreDocument) -> bool {
    let created_at = match doc.fields.get("created_at").and_then(|v| get_optional_string(v)) {
        Some(ts) => ts,
        None => {
            // If created_at is a timestampValue instead of stringValue
            match doc
                .fields
                .get("created_at")
                .and_then(|v| v.get("timestampValue"))
                .and_then(|v| v.as_str())
            {
                Some(ts) => ts.to_string(),
                None => return true, // No timestamp = treat as expired
            }
        }
    };

    match chrono::DateTime::parse_from_rfc3339(&created_at) {
        Ok(dt) => {
            let age = chrono::Utc::now() - dt.with_timezone(&chrono::Utc);
            age > chrono::Duration::minutes(5)
        }
        Err(_) => true, // Unparseable timestamp = treat as expired
    }
}
