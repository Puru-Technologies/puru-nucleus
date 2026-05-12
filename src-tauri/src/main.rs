//! puru-nucleus - Control center for Puru hospital deployments
//!
//! A Tauri desktop application that manages Docker-based hospital
//! software deployments, backups, and telemetry.
//!
//! Three operating modes:
//! - **GUI** (default): `puru` or `puru-nucleus` — launches Tauri window
//! - **CLI**: `puru status`, `puru backup`, etc. — terminal commands
//! - **Daemon**: `puru daemon` — headless background service

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backup;
mod cli;
mod cli_runner;
mod commands;
mod compose_template;
mod config;
mod daemon;
mod detection;
mod error;
mod firestore;
mod licensing;
mod services;
mod releases;
mod telemetry;
mod docker_update;
mod messaging;
mod network;
mod logs;
mod platform;
mod remote_shell;
mod tls;

use clap::Parser;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn main() {
    // Parse CLI args first
    let cli_args = cli::Cli::parse();

    // Route based on subcommand
    match cli_args.command {
        Some(cli::Commands::Daemon) => {
            // Daemon mode — headless background service, log to file
            init_daemon_logging();
            tracing::info!("Starting puru-nucleus in daemon mode");
            let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
            rt.block_on(daemon::run_daemon());
        }
        Some(command) => {
            // CLI mode — run command and exit
            init_logging();
            let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
            rt.block_on(cli_runner::run(command));
        }
        None => {
            // GUI mode — launch Tauri window (default when no subcommand)
            init_logging();
            tracing::info!("Starting puru-nucleus");
            run_gui();
        }
    }
}

fn init_logging() {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive("puru_nucleus=info".parse().unwrap()))
        .init();
}

/// Initialize logging for daemon mode — writes to a log file so errors are visible
/// even when running as a Windows Service (no console).
fn init_daemon_logging() {
    let log_dir = crate::config::config_dir();
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("daemon.log");

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path);

    match file {
        Ok(file) => {
            tracing_subscriber::registry()
                .with(fmt::layer().with_writer(std::sync::Mutex::new(file)))
                .with(EnvFilter::from_default_env().add_directive("puru_nucleus=info".parse().unwrap()))
                .init();
        }
        Err(_) => {
            // Fallback to stderr if log file can't be opened
            init_logging();
        }
    }
}

fn run_gui() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            // System
            commands::get_system_info,
            // Services
            commands::get_services,
            commands::start_service,
            commands::stop_service,
            commands::restart_service,
            // License
            commands::get_license,
            commands::get_machine_fingerprint,
            commands::reset_activation,
            commands::activate_license,
            // Pull settings
            commands::pull_settings,
            // Backup
            commands::start_backup,
            commands::get_backup_history,
            commands::restore_backup,
            // Config
            commands::get_config,
            commands::save_config,
            commands::sync_config_to_cloud,
            commands::get_sync_status,
            // Detection
            commands::detect_existing_setup,
            commands::check_prerequisites,
            commands::detect_environment,
            commands::apply_detected_environment,
            // Alerts
            commands::get_alerts,
            commands::acknowledge_alert,
            // Daemon
            commands::get_daemon_status,
            commands::install_daemon_service,
            commands::uninstall_daemon_service,
            commands::start_daemon,
            commands::stop_daemon,
            commands::restart_daemon,
            // Logs
            commands::get_container_logs,
            commands::get_log_sources,
            commands::list_log_files,
            commands::read_log_file,
            // Telemetry
            commands::get_telemetry_snapshot,
            // Releases
            commands::check_nucleus_update,
            commands::check_service_updates,
            commands::download_nucleus_update,
            commands::download_and_install_nucleus_update,
            commands::download_service_jar,
            commands::list_service_versions,
            // Credentials
            commands::check_credentials_file,
            commands::import_credentials_file,
            commands::save_credentials_content,
            // Logging
            commands::log_error,
            // Setup
            commands::setup_check_prerequisites,
            commands::setup_create_databases,
            commands::setup_configure_rabbitmq,
            commands::setup_generate_config,
            commands::setup_pull_images,
            commands::setup_start_services,
            commands::setup_health_check,
            commands::setup_configure_backups,
            commands::setup_install_daemon,
            commands::setup_tls,
            // Native JAR Deployment
            commands::pull_jars,
            commands::pull_single_jar,
            commands::check_jar_updates,
            commands::get_deployment_mode,
            // Docker Updates
            commands::update_docker_service,
            commands::rollback_docker_service,
            commands::get_update_history,
            // Remote Shell
            commands::execute_shell_command,
            commands::get_shell_audit_log,
            commands::get_allowed_shell_commands,
            // LAN Backup
            commands::validate_lan_path,
            commands::ship_binlogs_lan,
            commands::get_binlog_status,
            // Network
            commands::check_network,
            commands::run_speed_test,
            // Messaging
            commands::get_messages,
            commands::get_unread_count,
            commands::mark_message_read,
            commands::download_attachment,
            commands::apply_config_file,
            commands::install_certificate,
            // Compose Template
            commands::download_compose_template,
            commands::get_compose_content,
            commands::substitute_compose_variables,
            commands::save_compose_content,
            commands::upload_compose_to_cloud,
            commands::get_service_modules,
            commands::assemble_compose_file,
            // Env File Templates
            commands::download_env_templates,
            commands::get_env_files,
            commands::save_env_file,
            commands::upload_env_files_to_cloud,
            // TLS
            commands::get_tls_status,
            commands::generate_client_setup_script,
            commands::generate_nginx_https_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
