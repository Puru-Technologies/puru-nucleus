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

/// Run puru-nucleus in daemon mode (headless REST API server).
pub async fn run_daemon() {
    // 1. Load config
    let config = crate::config::load_config().unwrap_or_default();
    let daemon_cfg = config.daemon.clone().unwrap_or_default();
    let port = daemon_cfg.port;

    tracing::info!("Daemon mode: port={}, auth={}", port, !daemon_cfg.api_key.is_empty());

    // 2. Build shared state
    let state = Arc::new(AppState {
        api_key: daemon_cfg.api_key.clone(),
        port,
        started_at: Instant::now(),
        started_at_utc: chrono::Utc::now(),
    });

    // 3. Start background tasks
    let _handles = scheduler::start_all(&daemon_cfg, config.telemetry_enabled);

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
        // Log file reader
        .route("/api/logs/sources", get(routes::log_sources))
        .route("/api/logs/files", get(routes::log_files))
        .route("/api/logs/file", get(routes::log_file_read))
        // Pull settings
        .route("/api/pull", post(routes::pull_settings))
        // Network
        .route("/api/network", get(routes::network_check))
        .route("/api/network/speedtest", post(routes::network_speed_test))
        // LAN binlog
        .route("/api/lan/binlog/ship", post(routes::ship_binlogs_lan))
        // Middleware
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_api_key,
        ))
        .layer(CorsLayer::permissive())
        .with_state(state);

    // 5. Bind and serve
    let addr = format!("0.0.0.0:{}", port);
    tracing::info!("Daemon listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect(&format!("Failed to bind to port {}. Is it already in use?", port));

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Daemon server error");

    tracing::info!("Daemon shut down gracefully");
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
