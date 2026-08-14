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
mod process;
mod process_explorer;
mod remote_shell;
mod seed;
mod templates;
mod webserver;
mod tls;
mod installer;

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
pub(crate) fn init_daemon_logging() {
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

/// Build a 32×32 RGBA round "dot" icon in the given colour on a transparent
/// background — used as the tray health indicator (green = up, red = down).
fn dot_icon(r: u8, g: u8, b: u8) -> tauri::image::Image<'static> {
    const SIZE: u32 = 32;
    let radius = 13.0_f32;
    let center = SIZE as f32 / 2.0;
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 + 0.5 - center;
            let dy = y as f32 + 0.5 - center;
            if dx * dx + dy * dy <= radius * radius {
                rgba.extend_from_slice(&[r, g, b, 255]);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    tauri::image::Image::new_owned(rgba, SIZE, SIZE)
}

/// Reveal and focus the main dashboard window (used by the tray click/menu).
fn show_main(app: &tauri::AppHandle) {
    use tauri::Manager;
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Install a system-tray health indicator that polls the daemon's
/// `/api/health` and recolours the icon (green = up, red = down).
fn setup_tray(app: tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let status_i = MenuItem::with_id(&app, "status", "Daemon: checking…", false, None::<&str>)?;
    let open_i = MenuItem::with_id(&app, "open", "Open Dashboard", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(&app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(&app, &[&status_i, &open_i, &quit_i])?;

    let tray = TrayIconBuilder::with_id("health")
        .icon(dot_icon(0x9e, 0x9e, 0x9e)) // grey = unknown until first poll
        .tooltip("Puru Nucleus — checking…")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        })
        .build(&app)?;

    // Poll the daemon health endpoint forever; the running task also keeps the
    // tray + status menu-item handles alive for the life of the app.
    let port = crate::config::load_config()
        .ok()
        .and_then(|c| c.daemon)
        .map(|d| d.port)
        .unwrap_or(9090);

    tauri::async_runtime::spawn(async move {
        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{}/api/health", port);
        loop {
            let (icon, tip) = match client
                .get(&url)
                .timeout(std::time::Duration::from_secs(3))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    let body: serde_json::Value = resp.json().await.unwrap_or_default();
                    let ver = body.get("version").and_then(|v| v.as_str()).unwrap_or("?");
                    (
                        dot_icon(0x4c, 0xaf, 0x50),
                        format!("Puru Nucleus — daemon up · v{}", ver),
                    )
                }
                _ => (
                    dot_icon(0xf4, 0x43, 0x36),
                    "Puru Nucleus — daemon DOWN".to_string(),
                ),
            };
            let _ = tray.set_icon(Some(icon));
            let _ = tray.set_tooltip(Some(tip.as_str()));
            let _ = status_i.set_text(tip.as_str());
            tokio::time::sleep(std::time::Duration::from_secs(8)).await;
        }
    });

    Ok(())
}

fn run_gui() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            setup_tray(app.handle().clone())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // System
            commands::get_system_info,
            // Process explorer (Services tab port tools)
            commands::list_puru_processes,
            commands::kill_process_by_pid,
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
            commands::restore_pitr,
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
            // Cloud command activity (UI banner) + GUI-side processing
            commands::get_command_activity,
            commands::process_pending_commands,
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
            commands::tail_log_start,
            commands::tail_log_stop,
            commands::read_daemon_log,
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
            // Environment checks
            commands::check_system_clock,
            // Logging
            commands::log_error,
            // Setup
            commands::install_prerequisites,
            commands::setup_check_prerequisites,
            commands::setup_create_databases,
            commands::setup_configure_rabbitmq,
            commands::setup_generate_config,
            commands::setup_reset,
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
            commands::update_native_service,
            commands::rollback_native_service,
            // Native Setup Steps
            commands::setup_generate_env_files,
            commands::setup_pull_jars,
            commands::setup_start_native_services,
            commands::setup_seed_queues,
            commands::setup_seed_database,
            commands::is_elevated,
            commands::restart_as_admin,
            commands::seed_data,
            commands::seed_master_data,
            commands::finalise_templates,
            commands::check_template_updates,
            commands::apply_template_updates,
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
        .on_window_event(|window, event| {
            // Close-to-tray: hide the window instead of quitting so the tray
            // health indicator keeps running in the background.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
