//! Windows Service integration via the `windows-service` crate.
//!
//! When launched by the Windows Service Control Manager (SCM), the process must
//! register a service entry point and report status changes. This module handles
//! that handshake so `puru-nucleus daemon` works both as a standalone process
//! (interactive) and as a proper Windows Service.

use std::ffi::OsString;
use std::sync::mpsc;
use std::time::Duration;
use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
};

const SERVICE_NAME: &str = "PuruNucleus";
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

/// Attempt to register with the Windows SCM and run as a service.
/// Returns Ok(()) if successfully ran as a service (process exits after).
/// Returns Err(()) if we're not being launched by SCM (caller should run interactively).
pub fn run_as_service() -> Result<(), ()> {
    // service_dispatcher::start will fail immediately if not launched by SCM
    service_dispatcher::start(SERVICE_NAME, ffi_service_main).map_err(|_| ())
}

// Generate the FFI-compatible service entry point
define_windows_service!(ffi_service_main, service_main);

/// The actual service main function called by the SCM dispatcher.
fn service_main(_arguments: Vec<OsString>) {
    if let Err(e) = run_service() {
        // Log to file since we have no console
        let log_dir = crate::config::config_dir();
        let _ = std::fs::create_dir_all(&log_dir);
        let log_path = log_dir.join("service-error.log");
        let _ = std::fs::write(
            &log_path,
            format!("[{}] Service error: {:?}\n", chrono::Local::now(), e),
        );
    }
}

fn run_service() -> Result<(), Box<dyn std::error::Error>> {
    // Create a channel to receive stop events
    let (shutdown_tx, shutdown_rx) = mpsc::channel();

    // Register the service control handler
    let status_handle = service_control_handler::register(SERVICE_NAME, move |control| {
        match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = shutdown_tx.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    })?;

    // Report: starting
    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::StartPending,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::from_secs(10),
        process_id: None,
    })?;

    // Initialize logging
    crate::init_daemon_logging();
    tracing::info!("PuruNucleus Windows Service starting");

    // Create tokio runtime and start the daemon
    let rt = tokio::runtime::Runtime::new()?;

    // Start the daemon in a background task
    let daemon_handle = rt.spawn(async {
        crate::daemon::run_daemon().await;
    });

    // Report: running
    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    tracing::info!("PuruNucleus Windows Service is running");

    // Wait for stop signal from SCM
    let _ = shutdown_rx.recv();
    tracing::info!("PuruNucleus Windows Service received stop signal");

    // Abort the daemon task and shut down the runtime
    daemon_handle.abort();
    rt.shutdown_timeout(Duration::from_secs(10));

    // Report: stopped
    status_handle.set_service_status(ServiceStatus {
        service_type: SERVICE_TYPE,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    tracing::info!("PuruNucleus Windows Service stopped");
    Ok(())
}
