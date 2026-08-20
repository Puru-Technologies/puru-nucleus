//! Daemon mode — headless REST API server for remote management

pub mod auth;
pub mod commands;
pub mod routes;
pub mod scheduler;

use axum::{
    middleware,
    routing::{get, post, put},
    Router,
};
use std::sync::Arc;
use std::time::Instant;
use tower_http::cors::CorsLayer;

use scheduler::AppState;

/// Run puru-dc in daemon mode (headless REST API server).
pub async fn run_daemon() {
    // 1. Load config
    let config = crate::config::load_config().unwrap_or_default();
    let daemon_cfg = config.daemon.clone().unwrap_or_default();
    let port = daemon_cfg.port;

    tracing::info!("Daemon mode: port={}, auth={}", port, !daemon_cfg.api_key.is_empty());

    // Single-instance guard #1 — OS-level lock. On Windows this is a *global*
    // named mutex (`Global\PuruDCDaemon`) that survives across session/user
    // boundaries, so a per-user `puru daemon` and the SYSTEM scheduled task
    // can't both run. On Unix it's an flock on a lock file. Held for the whole
    // process lifetime via the returned guard.
    let _instance_lock = match single_instance::acquire() {
        Ok(g) => g,
        Err(e) => {
            tracing::error!(
                "Another puru-dc daemon is already running ({}). Exiting without starting background tasks.",
                e
            );
            return;
        }
    };

    // Single-instance guard #2 — bind the port BEFORE launching background
    // tasks. Redundant with the OS lock but catches the case where a stale
    // process still holds the port (e.g. crashed before releasing the mutex).
    let addr = format!("0.0.0.0:{}", port);
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(
                "Daemon port {} already in use ({}). Another puru-dc daemon is running — exiting without starting background tasks.",
                port, e
            );
            return;
        }
    };

    // Ensure the tray/GUI logon task exists (the daemon runs as SYSTEM and can
    // create it; a normal user can't). This is what makes the tray reliably
    // appear after a reboot instead of the daemon running "without a trace."
    crate::platform::ensure_gui_logon_task();

    // Plant / refresh the operator's emergency stop-all script. Cheap; lets
    // an operator kill every Puru JVM by hand if puru-dc itself is broken.
    crate::emergency::ensure_emergency_stop_script(&config);

    // 2. Build shared state
    let state = Arc::new(AppState {
        api_key: daemon_cfg.api_key.clone(),
        port,
        started_at: Instant::now(),
        started_at_utc: chrono::Utc::now(),
    });

    // 3. Start background tasks (safe now — we own the port)
    let _handles = scheduler::start_all(&daemon_cfg, config.telemetry_enabled);

    // 3a. Proactive boot pass — bring services up NOW instead of waiting for
    // the watchdog's first 60-second tick. Spawned so the HTTP server can
    // start serving immediately (a full boot start can take minutes:
    // infra warm-up, then per-service startup).
    //
    // - **Docker mode**: wait for the Docker daemon, then `docker start` any
    //   Puru container that isn't running. Docker's `restart: always` handles
    //   the common case; this is a safety net for containers that exited
    //   cleanly or a compose set to `restart: no`.
    // - **Native mode**: ensure MySQL + RabbitMQ, then `sync_native_services`
    //   (auth first, then everything else).
    if config.deployment_mode == crate::config::DeploymentMode::Docker {
        tokio::spawn(async {
            tracing::info!("Daemon (docker): running proactive boot start");
            crate::services::docker_boot_start().await;
        });
    } else if config.deployment_mode == crate::config::DeploymentMode::Native {
        tokio::spawn(async move {
            tracing::info!("Daemon (native): running proactive boot start");
            match crate::commands::ensure_mysql_running().await {
                Ok(_) => tracing::info!("Daemon (native): MySQL reachable"),
                Err(e) => {
                    tracing::error!(
                        "Daemon (native): MySQL not reachable ({}). Aborting boot start — the watchdog will retry once MySQL is up.",
                        e
                    );
                    return;
                }
            }
            match crate::commands::ensure_rabbitmq_running_public().await {
                Ok(_) => tracing::info!("Daemon (native): RabbitMQ reachable"),
                Err(e) => {
                    tracing::error!(
                        "Daemon (native): RabbitMQ not reachable ({}). Aborting boot start — the watchdog will retry once RabbitMQ is up.",
                        e
                    );
                    return;
                }
            }
            match crate::commands::sync_native_services(None).await {
                Ok(actions) => {
                    let started = actions.iter().filter(|a| a.action == "started").count();
                    let already = actions.iter().filter(|a| a.action == "already-running").count();
                    let failed = actions.iter().filter(|a| a.action == "failed").count();
                    tracing::info!(
                        "Daemon (native): boot start complete — {} started, {} already running, {} failed",
                        started, already, failed
                    );
                }
                Err(e) => tracing::error!("Daemon (native): boot start failed: {}", e),
            }
        });
    }

    // 4. Build router
    let app = Router::new()
        // Health (public)
        .route("/api/health", get(routes::health))
        // Services
        .route("/api/services", get(routes::list_services))
        .route("/api/services/:name/start", post(routes::start_service))
        .route("/api/services/:name/stop", post(routes::stop_service))
        .route("/api/services/:name/restart", post(routes::restart_service))
        .route("/api/services/:name/logs", get(routes::service_logs))
        // System
        .route("/api/system", get(routes::system_info))
        // Config
        .route("/api/config", get(routes::get_config))
        .route("/api/performance", get(routes::get_performance))
        .route("/api/performance", put(routes::update_performance))
        // Backup
        .route("/api/backup", post(routes::trigger_backup))
        .route("/api/backup/list", get(routes::backup_list))
        .route("/api/backup/schedule", get(routes::get_backup_schedule))
        .route("/api/backup/schedule", put(routes::update_backup_schedule))
        // Updates
        .route("/api/updates/check", get(routes::check_updates))
        .route("/api/updates/:service", post(routes::update_service))
        .route("/api/rollback/:service", post(routes::rollback_service))
        // Remote shell
        .route("/api/exec", post(routes::exec_command))
        .route("/api/exec/history", get(routes::exec_history))
        .route("/api/exec/allowed", get(routes::exec_allowed))
        // License
        .route("/api/license", get(routes::get_license))
        // Alerts
        .route("/api/alerts", get(routes::list_alerts))
        .route("/api/alerts/:id/ack", post(routes::acknowledge_alert))
        // Restore
        .route("/api/restore", post(routes::trigger_restore))
        .route("/api/restore/pitr", post(routes::trigger_pitr))
        // Log file reader
        .route("/api/logs/sources", get(routes::log_sources))
        .route("/api/logs/files", get(routes::log_files))
        .route("/api/logs/file", get(routes::log_file_read))
        // Pull settings
        .route("/api/pull", post(routes::pull_settings))
        // Network
        .route("/api/network", get(routes::network_check))
        .route("/api/network/speedtest", post(routes::network_speed_test))
        // Seed (fresh-install data)
        .route("/api/seed", post(routes::run_seed))
        // Master-data seed (user-triggered, never auto-runs)
        .route("/api/seed/master-data", post(routes::run_seed_master_data))
        // Remote setup pipeline (mirrors the GUI setup_* commands)
        .route("/api/setup/prerequisites/check", post(routes::setup_check_prerequisites))
        .route("/api/setup/databases", post(routes::setup_create_databases))
        .route("/api/setup/rabbitmq", post(routes::setup_configure_rabbitmq))
        .route("/api/setup/compose", post(routes::setup_generate_config))
        .route("/api/setup/images", post(routes::setup_pull_images))
        .route("/api/setup/services/start", post(routes::setup_start_services))
        .route("/api/setup/env-files", post(routes::setup_generate_env_files))
        .route("/api/setup/jars", post(routes::setup_pull_jars))
        .route("/api/setup/native-services/start", post(routes::setup_start_native_services))
        .route("/api/setup/seed-queues", post(routes::setup_seed_queues))
        .route("/api/setup/init-auth", post(routes::setup_init_auth))
        .route("/api/setup/seed-database", post(routes::setup_seed_database))
        .route("/api/setup/health-check", post(routes::setup_health_check))
        .route("/api/setup/backups", post(routes::setup_configure_backups))
        .route("/api/setup/tls", post(routes::setup_tls))
        .route("/api/setup/reset", post(routes::setup_reset))
        // Detection / prerequisites / deployment mode / modules
        .route("/api/detect", get(routes::detect_existing_setup))
        .route("/api/prerequisites", get(routes::check_prerequisites))
        .route("/api/deployment-mode", get(routes::get_deployment_mode))
        .route("/api/modules", get(routes::get_service_modules))
        // Config write
        .route("/api/config", put(routes::save_config))
        // Hospital templates
        .route("/api/templates/finalise", post(routes::finalise_templates))
        .route("/api/templates/updates", get(routes::check_template_updates))
        .route("/api/templates/apply", post(routes::apply_template_updates))
        // Native JAR deployment
        .route("/api/jars/pull", post(routes::pull_jars))
        .route("/api/jars/updates", get(routes::check_jar_updates))
        .route("/api/jars/update/:service", post(routes::update_native_service))
        .route("/api/jars/rollback/:service", post(routes::rollback_native_service))
        // LAN binlog
        .route("/api/lan/binlog/ship", post(routes::ship_binlogs_lan))
        // GCP token broker for local service JVMs (loopback-only, enforced in handler)
        .route("/api/gcp/token", post(routes::gcp_token))
        // Middleware
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_api_key,
        ))
        .layer(CorsLayer::permissive())
        .with_state(state);

    // 5. Serve on the listener bound earlier (before scheduler start-up).
    tracing::info!("Daemon listening on http://{}", addr);

    // On Windows, spawned service processes (java -jar with redirected stdio)
    // inherit every inheritable handle — including this listening socket.
    // An inherited listener keeps the port bound after the daemon dies, so a
    // restarted daemon can't bind until all child services are killed too.
    #[cfg(windows)]
    disable_listener_inheritance(&listener);

    if let Err(e) = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    {
        tracing::error!("Daemon server error: {}", e);
    }

    tracing::info!("Daemon shut down gracefully");
}

/// Strip the inherit flag from the listening socket so child processes
/// (java services spawned with redirected stdio) cannot inherit it.
#[cfg(windows)]
fn disable_listener_inheritance(listener: &tokio::net::TcpListener) {
    use std::os::windows::io::AsRawSocket;

    const HANDLE_FLAG_INHERIT: u32 = 0x0000_0001;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetHandleInformation(handle: isize, mask: u32, flags: u32) -> i32;
    }

    let handle = listener.as_raw_socket() as isize;
    let ok = unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
    if ok == 0 {
        tracing::warn!("Could not clear inherit flag on the listener socket");
    }
}

/// Wait for Ctrl-C or SIGTERM for graceful shutdown
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("Received Ctrl-C, shutting down..."),
        _ = terminate => tracing::info!("Received SIGTERM, shutting down..."),
    }
}

/// OS-level single-instance lock for the daemon. Held for the process lifetime
/// via an RAII guard — dropping the guard releases the lock.
///
/// - **Windows**: a `Global\`-prefixed named mutex, so it's shared across
///   session/user boundaries (the SYSTEM scheduled task and a per-user
///   `puru daemon` share the same mutex namespace).
/// - **Unix**: an exclusive flock on `<config_dir>/daemon.lock`.
mod single_instance {
    /// RAII guard — holds the underlying OS resource until dropped.
    pub struct Guard {
        #[cfg(windows)]
        _handle: WindowsMutexGuard,
        #[cfg(unix)]
        _fd: UnixFlockGuard,
    }

    #[cfg(windows)]
    pub fn acquire() -> Result<Guard, String> {
        Ok(Guard { _handle: WindowsMutexGuard::acquire()? })
    }

    #[cfg(unix)]
    pub fn acquire() -> Result<Guard, String> {
        Ok(Guard { _fd: UnixFlockGuard::acquire()? })
    }

    // ── Windows named mutex ─────────────────────────────────────────────────
    #[cfg(windows)]
    pub struct WindowsMutexGuard {
        handle: windows_sys::Win32::Foundation::HANDLE,
    }

    #[cfg(windows)]
    impl WindowsMutexGuard {
        fn acquire() -> Result<Self, String> {
            use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
            use windows_sys::Win32::System::Threading::CreateMutexW;

            // Try `Global\` first (visible across sessions + SYSTEM). If the
            // caller lacks SeCreateGlobalPrivilege — a non-elevated user — we
            // fall back to a Local mutex (session-scoped). Local still catches
            // the common case of the same user launching the daemon twice; the
            // port bind check catches cross-session conflicts we can't see.
            for name_str in ["Global\\PuruDCDaemon\0", "Local\\PuruDCDaemon\0"] {
                let name: Vec<u16> = name_str.encode_utf16().collect();
                let handle = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
                if handle.is_null() {
                    // Access denied on Global — try Local. If Local also fails,
                    // we surface the error.
                    continue;
                }
                let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
                if already {
                    unsafe { CloseHandle(handle) };
                    return Err(format!("{} mutex already held", name_str.trim_end_matches('\0')));
                }
                return Ok(WindowsMutexGuard { handle });
            }
            Err("CreateMutexW failed for both Global and Local namespaces".to_string())
        }
    }

    #[cfg(windows)]
    impl Drop for WindowsMutexGuard {
        fn drop(&mut self) {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::Threading::ReleaseMutex;
            unsafe {
                let _ = ReleaseMutex(self.handle);
                let _ = CloseHandle(self.handle);
            }
        }
    }

    // ── Unix flock ──────────────────────────────────────────────────────────
    #[cfg(unix)]
    pub struct UnixFlockGuard {
        fd: std::os::unix::io::RawFd,
    }

    #[cfg(unix)]
    impl UnixFlockGuard {
        fn acquire() -> Result<Self, String> {
            use std::os::unix::io::AsRawFd;

            let dir = crate::config::config_dir();
            let _ = std::fs::create_dir_all(&dir);
            let lock_path = dir.join("daemon.lock");

            let file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .open(&lock_path)
                .map_err(|e| format!("open {}: {}", lock_path.display(), e))?;
            let fd = file.as_raw_fd();

            // Non-blocking exclusive lock. Held for the process lifetime; the
            // kernel releases it automatically on exit even if we crash before
            // the Drop runs, which makes stale locks a non-issue.
            let rc = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
            if rc != 0 {
                let err = std::io::Error::last_os_error();
                return Err(format!("flock {}: {}", lock_path.display(), err));
            }

            // Leak the file so its fd stays open (and the lock stays held) for
            // the rest of the process; we track the raw fd in the guard.
            std::mem::forget(file);
            Ok(UnixFlockGuard { fd })
        }
    }

    #[cfg(unix)]
    impl Drop for UnixFlockGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = libc::flock(self.fd, libc::LOCK_UN);
                libc::close(self.fd);
            }
        }
    }
}
