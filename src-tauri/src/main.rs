//! puru-dc - Control center for Puru hospital deployments
//!
//! A Tauri desktop application that manages Docker-based hospital
//! software deployments, backups, and telemetry.
//!
//! Three operating modes:
//! - **GUI** (default): `puru` or `puru-dc` — launches Tauri window
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
mod emergency;
mod messaging;
mod network;
mod file_lock;
mod logs;
mod performance;
mod platform;
mod process;
mod process_explorer;
mod remote_shell;
mod secret;
mod seed;
mod templates;
mod webserver;
mod tls;
mod installer;
mod infra;

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
            tracing::info!("Starting puru-dc in daemon mode");
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
            tracing::info!("Starting puru-dc");
            run_gui(cli_args.minimized, cli_args.elevated_restart);
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

/// Tray icon canvas size (matches the embedded brand mark, 32×32).
const TRAY_SIZE: u32 = 32;

/// The Puru Labs brand mark (design-system app icon), embedded at compile time
/// and used as the tray icon base.
const BRAND_MARK_PNG: &[u8] = include_bytes!("../icons/32x32.png");

/// Decode the embedded brand mark into a 32×32 RGBA buffer. On any decode
/// failure it returns a transparent buffer so the tray still builds.
fn brand_mark_rgba() -> Vec<u8> {
    let needed = (TRAY_SIZE * TRAY_SIZE * 4) as usize;
    let mut out = vec![0u8; needed];
    if let Ok(mut reader) = png::Decoder::new(BRAND_MARK_PNG).read_info() {
        let mut buf = vec![0u8; reader.output_buffer_size()];
        if reader.next_frame(&mut buf).is_ok() {
            let n = buf.len().min(needed);
            out[..n].copy_from_slice(&buf[..n]);
        }
    }
    out
}

/// Build the tray icon: the Puru Labs brand mark with a small status badge
/// composited in the bottom-right corner (green = daemon up, red = down,
/// grey = unknown until the first poll). Keeps the brand visible while still
/// giving an at-a-glance health signal.
fn dot_icon(r: u8, g: u8, b: u8) -> tauri::image::Image<'static> {
    let mut rgba = brand_mark_rgba();

    // Badge geometry (bottom-right corner). A white ring separates the coloured
    // dot from whatever brand pixels sit behind it.
    let cx = TRAY_SIZE as f32 - 8.0;
    let cy = TRAY_SIZE as f32 - 8.0;
    let fill_r = 6.0_f32;
    let ring_r = 7.75_f32;

    for y in 0..TRAY_SIZE {
        for x in 0..TRAY_SIZE {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            let d2 = dx * dx + dy * dy;
            let idx = ((y * TRAY_SIZE + x) * 4) as usize;
            if d2 <= fill_r * fill_r {
                rgba[idx] = r;
                rgba[idx + 1] = g;
                rgba[idx + 2] = b;
                rgba[idx + 3] = 255;
            } else if d2 <= ring_r * ring_r {
                rgba[idx] = 0xff;
                rgba[idx + 1] = 0xff;
                rgba[idx + 2] = 0xff;
                rgba[idx + 3] = 255;
            }
        }
    }

    tauri::image::Image::new_owned(rgba, TRAY_SIZE, TRAY_SIZE)
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
        .tooltip("Puru DC — checking…")
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
                        format!("Puru DC — daemon up · v{}", ver),
                    )
                }
                _ => (
                    dot_icon(0xf4, 0x43, 0x36),
                    "Puru DC — daemon DOWN".to_string(),
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

/// Register (idempotently) a per-user login autostart so the GUI — and thus the
/// tray health indicator — comes back after a reboot, started minimized to the
/// tray. Runs in the operator's user context; the daemon's boot task (SYSTEM)
/// is separate and headless.
///
/// Also self-heals two things on GUI startup so a fresh MSI install "just works"
/// after reboot:
///   1. **GUI logon task** — normally created by the SYSTEM daemon; created here
///      too so the tray reappears at next login even if the daemon has never run.
///   2. **Daemon boot task** — reinstalled if the setup wizard has completed but
///      the task is missing (setup wiped, MSI reinstalled, admin unregistered
///      it, …). Requires elevation; silently skipped otherwise (the operator
///      will see the daemon-down badge in the tray and can re-run setup).
#[cfg(target_os = "windows")]
fn ensure_login_autostart() {
    // Remove any legacy HKCU Run entry from earlier releases so we don't
    // double-launch the tray (the current autostart is a scheduled task).
    let _ = crate::process::silent_std_cmd("reg")
        .args([
            "delete",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
            "/v",
            "PuruDC",
            "/f",
        ])
        .output();

    // Always try to plant the per-user tray/GUI logon task — cheap, idempotent,
    // and the whole reason the tray comes back after login. If the daemon has
    // been running as SYSTEM it will already exist; this covers the "daemon
    // never ran" case so operators are never left without a tray.
    crate::platform::ensure_gui_logon_task();

    // (Re)write the emergency stop-all script alongside the config, so an
    // operator can kill every Puru JVM by hand if puru-dc itself is broken.
    // The daemon also does this on boot; the GUI covers the "daemon never
    // ran" case (first launch, or daemon crashed on startup).
    if let Ok(cfg) = crate::config::load_config() {
        crate::emergency::ensure_emergency_stop_script(&cfg);
    }

    // Self-heal the daemon boot task if setup has completed but the task is
    // gone. Only attempt when elevated — otherwise `schtasks /Create` for a
    // SYSTEM principal is denied, which would just produce noisy failures.
    let setup_done = crate::config::load_config()
        .map(|c| c.setup_completed)
        .unwrap_or(false);
    if setup_done
        && !crate::platform::boot_task_installed()
        && crate::commands::is_elevated()
    {
        tracing::info!("Daemon boot task missing after setup — reinstalling");
        // `platform::install_service` is async; spin a short-lived runtime
        // rather than blocking Tauri's main setup callback. Failure is logged
        // and shrugged off; the operator can re-run Setup manually.
        std::thread::spawn(|| {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(r) => r,
                Err(_) => return,
            };
            match rt.block_on(crate::platform::install_service()) {
                Ok(r) => tracing::info!("Boot task self-heal: {}", r.message),
                Err(e) => tracing::warn!("Boot task self-heal failed: {}", e),
            }
        });
    }
}
#[cfg(not(target_os = "windows"))]
fn ensure_login_autostart() {}

/// Holds the GUI single-instance mutex for the life of the process. Windows
/// releases the handle on process teardown regardless, so even a hard kill frees
/// the lock — the explicit release just makes a clean exit deterministic.
#[cfg(target_os = "windows")]
struct GuiInstanceLock(windows_sys::Win32::Foundation::HANDLE);

#[cfg(target_os = "windows")]
impl Drop for GuiInstanceLock {
    fn drop(&mut self) {
        // Null means we never actually got a mutex (CreateMutexW failed and we
        // chose to start anyway) — closing a null handle raises under a debugger.
        if self.0.is_null() {
            return;
        }
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.0);
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

/// Claim the "one GUI at a time" lock, or `None` if another GUI genuinely holds it.
///
/// Why a mutex and not just the single-instance plugin: the plugin finds the
/// first instance through a hidden window and `SendMessage`, and **UIPI blocks
/// window messages from a lower to a higher integrity process**. So after
/// `restart_as_admin` relaunches us elevated, the new instance cannot see the
/// old medium-integrity one and both keep running. Kernel objects aren't subject
/// to UIPI, so a named mutex sees across the boundary the plugin can't.
///
/// `Local\` (session) namespace, not `Global\`: the broken case is two instances
/// in the *same* session at different integrity levels, which `Local\` covers,
/// while `Global\` needs `SeCreateGlobalPrivilege` — which standard users don't
/// hold, so it would fail exactly where it's needed.
///
/// On contention we wait **only during an elevation handoff**, which `handoff`
/// marks (it is `--elevated-restart`, set solely by `restart_as_admin`).
/// `restart_as_admin` spawns the elevated copy and only then exits the old one,
/// so those two overlap by design; a fail-fast lock would make the elevated
/// instance give up mid-handoff and the app would appear to just close.
///
/// Every other launch must *not* wait. An operator double-clicking the tray
/// app's shortcut while it is already running is the common case, and blocking
/// there for 15s before giving up reads as "the app didn't start" — the second
/// process has to reach the Tauri builder promptly so the single-instance
/// plugin can raise the window that already exists.
#[cfg(target_os = "windows")]
fn acquire_gui_instance_lock(handoff: bool) -> Option<GuiInstanceLock> {
    use windows_sys::Win32::Foundation::{
        GetLastError, CloseHandle, ERROR_ALREADY_EXISTS, WAIT_ABANDONED, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};

    /// How long to let an in-flight elevation handoff finish before calling it a
    /// duplicate.
    const HANDOFF_WAIT_MS: u32 = 15_000;

    // UTF-16, NUL-terminated.
    let name: Vec<u16> = "Local\\PuruDC-GUI\0".encode_utf16().collect();

    unsafe {
        let handle = CreateMutexW(std::ptr::null(), 1 /* take ownership */, name.as_ptr());
        if handle.is_null() {
            // Can't create the lock at all — don't punish the user by refusing to
            // start; fall back to the plugin's guard alone.
            tracing::warn!("GUI instance lock: CreateMutexW failed ({}), continuing", GetLastError());
            return Some(GuiInstanceLock(handle));
        }

        if GetLastError() != ERROR_ALREADY_EXISTS {
            return Some(GuiInstanceLock(handle)); // uncontended — we own it
        }

        // Someone else holds it. Wait for them to go away (elevation handoff), or
        // conclude this really is a second GUI.
        let wait_ms = if handoff { HANDOFF_WAIT_MS } else { 0 };
        tracing::info!("GUI instance lock held — waiting up to {}ms for handoff", wait_ms);
        match WaitForSingleObject(handle, wait_ms) {
            // WAIT_ABANDONED is the *normal* outcome here: the previous owner was
            // terminated rather than releasing cleanly, which is exactly what
            // `app.exit(0)` looks like. It still transfers ownership to us.
            WAIT_OBJECT_0 | WAIT_ABANDONED => Some(GuiInstanceLock(handle)),
            _ => {
                CloseHandle(handle);
                None
            }
        }
    }
}

fn run_gui(minimized: bool, elevated_restart: bool) {
    // One GUI per session. Bound to a named local so it lives until the process
    // exits — `let _ = ...` would drop it immediately and release the lock.
    //
    // Losing the race is not fatal: we deliberately fall through to the builder
    // without a lock so the single-instance plugin runs, raises the window that
    // already exists, and exits this process itself. Returning here instead
    // would make a second launch look like a silent no-op. The plugin exits on
    // `ERROR_ALREADY_EXISTS` whether or not its `SendMessage` lands, so a
    // duplicate GUI still cannot survive even across an integrity boundary.
    #[cfg(target_os = "windows")]
    let _instance_lock = match acquire_gui_instance_lock(elevated_restart) {
        Some(lock) => Some(lock),
        None => {
            tracing::warn!("Another puru-dc GUI holds the instance lock — deferring to it");
            None
        }
    };
    #[cfg(not(target_os = "windows"))]
    let _ = elevated_restart;

    tauri::Builder::default()
        // Single-instance: if the operator (or an autostart chain) launches
        // puru-dc a second time, hand its args to the already-running instance
        // and exit this one. The `_argv` / `_cwd` callback fires *inside the
        // first* process, so we take that as our cue to reveal the existing
        // window instead of quietly ignoring it.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tracing::info!("Second instance blocked — focusing the running window");
            show_main(app);
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            setup_tray(app.handle().clone())?;

            // Make sure we relaunch to the tray on the next login.
            ensure_login_autostart();

            // The window is created hidden (visible:false in tauri.conf). Show it
            // for a normal launch; keep it in the tray when autostarted.
            if !minimized {
                show_main(app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // System
            commands::get_system_info,
            // Process explorer (Services tab port tools)
            commands::list_puru_processes,
            commands::kill_process_by_pid,
            // Performance (JVM memory plan)
            commands::get_performance_plan,
            commands::save_performance_config,
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
            commands::mark_setup_completed,
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
            // Status heartbeat (online + telemetry, GUI-driven)
            commands::send_status_heartbeat,
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
            commands::redeem_onboarding_code,
            // Environment checks
            commands::check_system_clock,
            // Logging
            commands::log_error,
            // Setup
            commands::install_prerequisites,
            commands::download_prerequisites_to_downloads,
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
            commands::check_service_update,
            commands::get_staged_update,
            commands::download_service_update,
            commands::apply_service_update,
            commands::discard_service_update,
            commands::get_jar_manifest,
            commands::list_locking_processes,
            commands::force_free_and_restart,
            commands::rollback_native_service_to,
            commands::control_infra_service,
            commands::get_infra_log,
            // Native Setup Steps
            commands::setup_generate_env_files,
            commands::setup_pull_jars,
            commands::setup_start_native_services,
            commands::setup_seed_queues,
            commands::setup_init_auth,
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
