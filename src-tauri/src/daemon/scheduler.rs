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
            deployment_mode: format!("{:?}", config.deployment_mode).to_lowercase(),
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
/// RAM the OS can still hand out before we call it low. Compared against
/// *available* memory, so reclaimable page cache doesn't read as "in use".
const RAM_WARNING_GB_AVAILABLE: f64 = 0.5;

/// An alert we raised and have not yet resolved.
///
/// Held so the watchdog can (a) avoid re-alerting every 60s, and (b) close the
/// original Firestore doc — and say so — once the condition clears.
#[derive(Debug, Clone)]
struct ActiveAlert {
    /// Severity we raised it at. Tracked so an escalation (warning → critical)
    /// raises a fresh alert instead of being swallowed as a duplicate.
    severity: &'static str,
    /// Category, mirrored into the hospital doc's rollup.
    category: &'static str,
    /// Firestore doc id. `None` when the push failed (offline); the condition
    /// is still tracked so the recovery notice fires once we're back.
    id: Option<String>,
}

/// Counts of open watchdog alerts, mirrored onto the hospital document for the
/// puru-oxygen dashboard. Compared between ticks so we only write on change
/// instead of once a minute forever.
#[derive(Debug, Clone, Default, PartialEq)]
struct AlertRollup {
    critical: i64,
    warning: i64,
    /// Sorted + deduped categories, so the dashboard can say *what* is wrong.
    categories: Vec<String>,
}

impl AlertRollup {
    fn from_active<'a>(active: impl Iterator<Item = &'a ActiveAlert>) -> Self {
        let mut rollup = AlertRollup::default();
        for alert in active {
            match alert.severity {
                "critical" => rollup.critical += 1,
                _ => rollup.warning += 1,
            }
            rollup.categories.push(alert.category.to_string());
        }
        rollup.categories.sort();
        rollup.categories.dedup();
        rollup
    }
}

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

    // Alerts we've raised and not yet resolved — keyed so we can both suppress
    // duplicates and close the right Firestore doc on recovery.
    let mut alerted_services: std::collections::HashMap<String, ActiveAlert> =
        std::collections::HashMap::new();
    let mut disk_alert: Option<ActiveAlert> = None;
    let mut ram_alert: Option<ActiveAlert> = None;
    let mut boot_task_alert: Option<ActiveAlert> = None;
    // Last rollup mirrored to the hospital doc — so we write on change only.
    let mut last_rollup: Option<AlertRollup> = None;

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

        // ── Lightweight liveness heartbeat ──────────────────────────────
        // Refresh nucleus.last_seen every 60s so the cloud dashboard's
        // online/offline status stays current between the 15-min status pushes.
        if config.telemetry_enabled {
            match crate::firestore::FirestoreClient::new_from_config().await {
                Ok(client) => {
                    if let Err(e) = client.push_heartbeat(&config.hospital_code).await {
                        tracing::debug!("Heartbeat push failed: {}", e);
                    }
                }
                Err(e) => tracing::debug!("Heartbeat: Firestore unavailable: {}", e),
            }
        }

        // ── Boot task still there? ──────────────────────────────────────
        // We are running, so the task that starts us was registered at some
        // point. If it has since vanished, something external deleted it —
        // in practice Defender quarantining puru-dc for Persistence.A!ml. The
        // damage is invisible until the next reboot, when nothing starts at
        // all, so this is the one chance to report it while we can still
        // reach Firestore. Alert once; re-arm if it comes back.
        if !crate::platform::boot_task_installed() {
            if boot_task_alert.is_none() {
                let detail = crate::platform::defender::recent_detection()
                    .await
                    .unwrap_or_else(|| {
                        "The PuruDC boot task is no longer registered. The daemon will \
                         not start after the next reboot. Re-run `puru service install` \
                         from an elevated prompt."
                            .to_string()
                    });
                let id = push_watchdog_alert(
                    &config.hospital_code,
                    "critical",
                    "boot_task_missing",
                    "Daemon boot task disappeared",
                    &detail,
                )
                .await;
                tracing::error!("Watchdog: boot task missing — {}", detail);
                boot_task_alert = Some(ActiveAlert {
                    severity: "critical",
                    category: "boot_task_missing",
                    id,
                });
            }
        } else if let Some(active) = boot_task_alert.take() {
            tracing::info!("Watchdog: boot task is registered again");
            clear_watchdog_alert(
                &config.hospital_code,
                active,
                Some((
                    "boot_task_missing",
                    "Resolved: daemon boot task is back",
                    "The PuruDC boot task is registered again — the daemon will start \
                     after the next reboot.",
                )),
            )
            .await;
        }

        // ── Service health check & auto-restart ─────────────────────────
        match crate::services::get_services().await {
            Ok(services) => {
                for svc in &services {
                    // Enabled-but-not-installed is a deployment gap, not a crash —
                    // there is no process/container to (re)start. Alert once so an
                    // operator can re-run setup, but never spin on it.
                    if svc.status == crate::services::ServiceStatus::NotInstalled {
                        if !alerted_services.contains_key(&svc.name) {
                            let id = push_watchdog_alert(
                                &config.hospital_code,
                                "warning",
                                "service_not_installed",
                                &format!("Service enabled but not installed: {}", svc.name),
                                &format!(
                                    "{} is enabled for this hospital but its build is not installed. \
                                     Re-run setup (Pull JARs) to deploy it.",
                                    svc.name
                                ),
                            )
                            .await;
                            alerted_services.insert(
                                svc.name.clone(),
                                ActiveAlert {
                                    severity: "warning",
                                    category: "service_not_installed",
                                    id,
                                },
                            );
                        }
                        continue;
                    }

                    // Identifier used to (re)start this service — Docker needs the
                    // container name, native needs the service name. Also the key
                    // for the manual-stop marker.
                    let svc_id = match config.deployment_mode {
                        crate::config::DeploymentMode::Docker => svc.container_name.clone(),
                        crate::config::DeploymentMode::Native => svc.name.clone(),
                    };

                    // Respect an operator's intentional stop — never auto-restart
                    // something that was stopped on purpose from the UI/CLI/API.
                    if svc.status == crate::services::ServiceStatus::Stopped
                        && crate::services::is_manually_stopped(&svc_id)
                    {
                        continue;
                    }

                    // Truly down (no live process/container) → start aggressively.
                    let is_down = svc.status == crate::services::ServiceStatus::Stopped;
                    // Alive but failing — a post-grace Error (native: never became
                    // ready), or a running service whose actuator reports DOWN
                    // (docker). Restart once. `Starting` is deliberately excluded
                    // so we never interrupt a service that's still booting.
                    let is_unhealthy = svc.status == crate::services::ServiceStatus::Error
                        || (svc.status == crate::services::ServiceStatus::Running
                            && matches!(
                                svc.health,
                                Some(crate::services::HealthStatus::Unhealthy)
                            ));

                    if is_down || is_unhealthy {
                        // A persistently-unhealthy (but running) service gets one
                        // restart attempt, not one per cycle — restarting it every
                        // 60s won't fix an unhealthy dependency.
                        if !is_down && alerted_services.contains_key(&svc.name) {
                            continue;
                        }

                        let reason = if is_down {
                            format!("{} is {:?}", svc.name, svc.status)
                        } else {
                            format!("{} is unhealthy", svc.name)
                        };

                        tracing::warn!("Watchdog: {} — attempting restart", reason);

                        if svc_id.is_empty() {
                            tracing::warn!(
                                "Watchdog: no restart identifier for {} — skipping",
                                svc.name
                            );
                            continue;
                        }

                        // Down → start it (no process yet). Alive-but-failing →
                        // restart it (stop+start) to actually cycle the process.
                        let attempt = if is_down {
                            crate::services::start_service(&svc_id).await
                        } else {
                            crate::services::restart_service(&svc_id).await
                        };
                        match attempt {
                            Ok(()) => {
                                tracing::info!(
                                    "Watchdog: auto-restarted {}",
                                    svc.name
                                );

                                // Push alert only on first detection (not on every cycle)
                                if !alerted_services.contains_key(&svc.name) {
                                    let id = push_watchdog_alert(
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
                                    alerted_services.insert(
                                        svc.name.clone(),
                                        ActiveAlert {
                                            severity: "warning",
                                            category: "service_restart",
                                            id,
                                        },
                                    );
                                }
                            }
                            Err(e) => {
                                tracing::error!(
                                    "Watchdog: failed to restart {}: {}",
                                    svc.name,
                                    e
                                );

                                if !alerted_services.contains_key(&svc.name) {
                                    let id = push_watchdog_alert(
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
                                    alerted_services.insert(
                                        svc.name.clone(),
                                        ActiveAlert {
                                            severity: "critical",
                                            category: "service_down",
                                            id,
                                        },
                                    );
                                }
                            }
                        }
                    } else if svc.status == crate::services::ServiceStatus::Running {
                        // Service recovered — close its alert and say so.
                        if let Some(active) = alerted_services.remove(&svc.name) {
                            tracing::info!(
                                "Watchdog: {} recovered, clearing alert state",
                                svc.name
                            );
                            clear_watchdog_alert(
                                &config.hospital_code,
                                active,
                                Some((
                                    "service_recovered",
                                    &format!("Resolved: {} is running again", svc.name),
                                    &format!(
                                        "Service {} is back up and reporting healthy.",
                                        svc.name
                                    ),
                                )),
                            )
                            .await;
                        }
                        // It's running, so any stale manual-stop marker no longer
                        // applies (it was started by some path that didn't clear it).
                        crate::services::clear_manually_stopped(&svc_id);
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Watchdog: failed to get services: {}", e);
            }
        }

        // ── Hard disk space ─────────────────────────────────────────────
        // Reported per-drive, not as an average across every mount: one drive
        // at 95% is the thing an operator has to act on, and averaging it with
        // a mostly-empty second drive hides it. Titles say "hard disk" so they
        // can't be misread as the RAM alert below.
        if let Some(disk) = crate::telemetry::fullest_disk() {
            let level = if disk.used_percent >= DISK_CRITICAL_PERCENT {
                Some("critical")
            } else if disk.used_percent >= DISK_WARNING_PERCENT {
                Some("warning")
            } else {
                None
            };

            let already_at_this_level =
                disk_alert.as_ref().map(|a| a.severity) == level;

            match level {
                Some(severity) if !already_at_this_level => {
                    // First alert, or an escalation/de-escalation between
                    // warning and critical — close the old doc without a
                    // "resolved" notice, since nothing is actually fixed.
                    if let Some(previous) = disk_alert.take() {
                        clear_watchdog_alert(&config.hospital_code, previous, None).await;
                    }

                    let (title, advice) = if severity == "critical" {
                        (
                            "Critical: hard disk almost full",
                            "Free up space immediately — backups and database writes will \
                             start failing.",
                        )
                    } else {
                        (
                            "Warning: hard disk space getting low",
                            "Consider freeing up space or expanding storage.",
                        )
                    };
                    let message = format!(
                        "Hard disk {} is {:.1}% full — {:.1} GB free of {:.1} GB. {}",
                        disk.mount, disk.used_percent, disk.free_gb, disk.total_gb, advice
                    );

                    if severity == "critical" {
                        tracing::error!("Watchdog: {}", message);
                    } else {
                        tracing::warn!("Watchdog: {}", message);
                    }

                    let id = push_watchdog_alert(
                        &config.hospital_code,
                        severity,
                        "disk_space",
                        title,
                        &message,
                    )
                    .await;
                    disk_alert = Some(ActiveAlert {
                        severity,
                        category: "disk_space",
                        id,
                    });
                }
                Some(_) => {} // already alerted at this level — stay quiet
                None => {
                    if let Some(previous) = disk_alert.take() {
                        tracing::info!(
                            "Watchdog: disk usage back to normal ({:.1}% on {})",
                            disk.used_percent,
                            disk.mount
                        );
                        clear_watchdog_alert(
                            &config.hospital_code,
                            previous,
                            Some((
                                "disk_space",
                                "Resolved: hard disk space recovered",
                                &format!(
                                    "Hard disk {} is back to {:.1}% full — {:.1} GB free of \
                                     {:.1} GB.",
                                    disk.mount, disk.used_percent, disk.free_gb, disk.total_gb
                                ),
                            )),
                        )
                        .await;
                    }
                }
            }
        }

        // ── RAM (physical memory) ───────────────────────────────────────
        // "Memory" on its own reads as disk space to plenty of people, so every
        // string here says RAM explicitly. Measured against *available* memory,
        // which counts reclaimable cache — `total - used` under-reports free RAM
        // badly on Linux and produced false alerts.
        let ram = crate::telemetry::ram_usage();
        if ram.total_gb > 0.0 && ram.available_gb < RAM_WARNING_GB_AVAILABLE {
            if ram_alert.is_none() {
                let message = format!(
                    "Only {:.2} GB of RAM is available out of {:.1} GB installed \
                     ({:.0}% in use). This is system memory, not hard disk space — \
                     services may slow down or be killed by the OS.",
                    ram.available_gb, ram.total_gb, ram.used_percent
                );
                tracing::warn!("Watchdog: {}", message);
                let id = push_watchdog_alert(
                    &config.hospital_code,
                    "warning",
                    "memory",
                    "Warning: low RAM (system memory)",
                    &message,
                )
                .await;
                ram_alert = Some(ActiveAlert {
                    severity: "warning",
                    category: "memory",
                    id,
                });
            }
        } else if let Some(previous) = ram_alert.take() {
            tracing::info!(
                "Watchdog: RAM recovered ({:.2} GB available)",
                ram.available_gb
            );
            clear_watchdog_alert(
                &config.hospital_code,
                previous,
                Some((
                    "memory",
                    "Resolved: RAM is available again",
                    &format!(
                        "{:.2} GB of RAM is available again out of {:.1} GB installed.",
                        ram.available_gb, ram.total_gb
                    ),
                )),
            )
            .await;
        }

        // ── Mirror open alerts onto the hospital doc for puru-oxygen ────
        let rollup = AlertRollup::from_active(
            alerted_services
                .values()
                .chain(disk_alert.iter())
                .chain(ram_alert.iter())
                .chain(boot_task_alert.iter()),
        );
        if last_rollup.as_ref() != Some(&rollup) {
            match crate::firestore::FirestoreClient::new_from_config().await {
                Ok(client) => {
                    match client
                        .push_alert_summary(
                            &config.hospital_code,
                            rollup.critical,
                            rollup.warning,
                            &rollup.categories,
                        )
                        .await
                    {
                        Ok(()) => last_rollup = Some(rollup),
                        Err(e) => {
                            // Leave last_rollup alone so the next tick retries.
                            tracing::warn!("Watchdog: alert summary push failed: {}", e);
                        }
                    }
                }
                Err(e) => tracing::debug!("Watchdog: Firestore unavailable for summary: {}", e),
            }
        }
    }
}

/// Push an alert to Firestore (best-effort, never blocks the watchdog loop).
///
/// Returns the new document's id so the caller can close it out when the
/// condition clears; `None` if the push didn't land (offline, no credentials).
async fn push_watchdog_alert(
    hospital_code: &str,
    severity: &str,
    category: &str,
    title: &str,
    message: &str,
) -> Option<String> {
    let client = match crate::firestore::FirestoreClient::new_from_config().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Watchdog alert: Firestore unavailable: {}", e);
            return None;
        }
    };

    match client
        .push_alert(hospital_code, severity, category, title, message)
        .await
    {
        Ok(id) => {
            tracing::info!("Watchdog alert pushed: {} ({})", title, id);
            Some(id)
        }
        Err(e) => {
            tracing::warn!("Watchdog alert: failed to push: {}", e);
            None
        }
    }
}

/// Close out an alert whose condition has cleared.
///
/// Marks the original document resolved so puru-oxygen can drop it from the
/// "currently broken" list, and — when `recovery` is given — pushes a short
/// info alert so whoever saw the problem also sees the fix. Pass `None` for
/// `recovery` when the alert is being replaced rather than genuinely fixed
/// (e.g. a disk warning escalating to critical).
async fn clear_watchdog_alert(
    hospital_code: &str,
    active: ActiveAlert,
    recovery: Option<(&str, &str, &str)>, // category, title, message
) {
    if let Some(alert_id) = active.id {
        match crate::firestore::FirestoreClient::new_from_config().await {
            Ok(client) => {
                if let Err(e) = client.resolve_alert(hospital_code, &alert_id).await {
                    tracing::warn!("Watchdog: failed to resolve alert {}: {}", alert_id, e);
                }
            }
            Err(e) => {
                tracing::warn!("Watchdog: Firestore unavailable to resolve alert: {}", e);
            }
        }
    }

    if let Some((category, title, message)) = recovery {
        let _ = push_watchdog_alert(hospital_code, "info", category, title, message).await;
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
