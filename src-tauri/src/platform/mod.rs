//! Platform service management — install, uninstall, start, stop, and check
//! the puru-dc daemon as a system service.
//!
//! - **Linux**: systemd unit file
//! - **macOS**: launchd plist
//! - **Windows**: Windows Service via sc.exe

mod linux;
mod macos;
mod windows;

use serde::{Deserialize, Serialize};

/// Service installation/management result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceResult {
    pub success: bool,
    pub message: String,
}

/// Status of the puru-dc system service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub installed: bool,
    pub running: bool,
    pub enabled: bool,
    pub pid: Option<u32>,
    pub detail: String,
}

/// Get the `puru-dc` binary path, resolving the current executable.
fn get_exe_path() -> Result<String, String> {
    std::env::current_exe()
        .map(|p| p.display().to_string())
        .map_err(|e| format!("Cannot determine executable path: {}", e))
}

/// Ensure the per-user tray/GUI logon task exists so the tray reliably appears
/// after a reboot. Windows-only; no-op on other platforms. Called by the SYSTEM
/// daemon on boot (it has the rights a normal user lacks).
pub fn ensure_gui_logon_task() {
    #[cfg(windows)]
    windows::ensure_gui_logon_task();
}

/// True if the SYSTEM daemon boot task is currently registered with the
/// platform's service manager. Cheap probe used at GUI startup to decide
/// whether to reinstall a missing boot task.
pub fn boot_task_installed() -> bool {
    #[cfg(target_os = "windows")]
    return windows::boot_task_installed();

    #[cfg(not(target_os = "windows"))]
    return true;
}

/// Install puru-dc as a system service (platform-specific).
pub async fn install_service() -> Result<ServiceResult, String> {
    #[cfg(target_os = "linux")]
    return linux::install().await;

    #[cfg(target_os = "macos")]
    return macos::install().await;

    #[cfg(target_os = "windows")]
    return windows::install().await;

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    Err("Unsupported platform".to_string())
}

/// Uninstall the puru-dc system service.
pub async fn uninstall_service() -> Result<ServiceResult, String> {
    #[cfg(target_os = "linux")]
    return linux::uninstall().await;

    #[cfg(target_os = "macos")]
    return macos::uninstall().await;

    #[cfg(target_os = "windows")]
    return windows::uninstall().await;

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    Err("Unsupported platform".to_string())
}

/// Start the puru-dc system service.
pub async fn start_service() -> Result<ServiceResult, String> {
    #[cfg(target_os = "linux")]
    return linux::start().await;

    #[cfg(target_os = "macos")]
    return macos::start().await;

    #[cfg(target_os = "windows")]
    return windows::start().await;

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    Err("Unsupported platform".to_string())
}

/// Stop the puru-dc system service.
pub async fn stop_service() -> Result<ServiceResult, String> {
    #[cfg(target_os = "linux")]
    return linux::stop().await;

    #[cfg(target_os = "macos")]
    return macos::stop().await;

    #[cfg(target_os = "windows")]
    return windows::stop().await;

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    Err("Unsupported platform".to_string())
}

/// Query the service status.
pub async fn service_status() -> Result<ServiceStatus, String> {
    #[cfg(target_os = "linux")]
    return linux::status().await;

    #[cfg(target_os = "macos")]
    return macos::status().await;

    #[cfg(target_os = "windows")]
    return windows::status().await;

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    Err("Unsupported platform".to_string())
}

/// Return the platform name for display purposes.
pub fn platform_name() -> &'static str {
    #[cfg(target_os = "linux")]
    return "Linux (systemd)";

    #[cfg(target_os = "macos")]
    return "macOS (launchd)";

    #[cfg(target_os = "windows")]
    return "Windows (Service)";

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    return "Unknown";
}
