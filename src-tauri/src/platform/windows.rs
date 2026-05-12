//! Windows Service management via sc.exe

use super::{ServiceResult, ServiceStatus, get_exe_path};

const SERVICE_NAME: &str = "PuruNucleus";
const DISPLAY_NAME: &str = "Puru Nucleus";

/// Register as a Windows service via sc.exe create.
pub async fn install() -> Result<ServiceResult, String> {
    let exe_path = get_exe_path()?;
    let bin_path = format!("\"{}\" daemon", exe_path);

    let output = tokio::process::Command::new("sc.exe")
        .args([
            "create",
            SERVICE_NAME,
            &format!("binPath={}", bin_path),
            "start=auto",
            &format!("DisplayName={}", DISPLAY_NAME),
        ])
        .output()
        .await
        .map_err(|e| format!("sc.exe create failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("{}{}", stdout, stderr);
        if !combined.contains("already exists") {
            if combined.contains("Access is denied") || combined.contains("FAILED 5") {
                return Err(
                    "Administrator privileges required to install the service. \
                     Right-click Puru Nucleus and select 'Run as Administrator', then try again."
                        .to_string(),
                );
            }
            return Err(format!("Service registration failed: {}", combined.trim()));
        }
    }

    // Set description
    let _ = tokio::process::Command::new("sc.exe")
        .args([
            "description",
            SERVICE_NAME,
            "Puru Nucleus - Hospital Deployment Manager daemon",
        ])
        .output()
        .await;

    // Configure recovery: restart on failure
    let _ = tokio::process::Command::new("sc.exe")
        .args([
            "failure",
            SERVICE_NAME,
            "reset=86400",
            "actions=restart/10000/restart/30000/restart/60000",
        ])
        .output()
        .await;

    Ok(ServiceResult {
        success: true,
        message: format!(
            "Windows service '{}' registered. Run `sc.exe start {}` to start.",
            DISPLAY_NAME, SERVICE_NAME
        ),
    })
}

/// Remove the Windows service.
pub async fn uninstall() -> Result<ServiceResult, String> {
    // Stop first (ignore errors)
    let _ = tokio::process::Command::new("sc.exe")
        .args(["stop", SERVICE_NAME])
        .output()
        .await;

    // Brief pause to let it stop
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    let output = tokio::process::Command::new("sc.exe")
        .args(["delete", SERVICE_NAME])
        .output()
        .await
        .map_err(|e| format!("sc.exe delete failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("{}{}", stdout, stderr);
        if !combined.contains("not exist") {
            return Err(format!("Service deletion failed: {}", combined.trim()));
        }
    }

    Ok(ServiceResult {
        success: true,
        message: format!("Windows service '{}' removed.", SERVICE_NAME),
    })
}

/// Start the Windows service.
pub async fn start() -> Result<ServiceResult, String> {
    let output = tokio::process::Command::new("sc.exe")
        .args(["start", SERVICE_NAME])
        .output()
        .await
        .map_err(|e| format!("sc.exe start failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("{}{}", stdout.trim(), stderr.trim());
        if combined.contains("Access is denied") || combined.contains("FAILED 5") {
            return Err("Administrator privileges required. Run as Administrator and try again.".to_string());
        }
        return Err(format!("Service start failed: {}", combined));
    }

    Ok(ServiceResult {
        success: true,
        message: format!("{} started.", SERVICE_NAME),
    })
}

/// Stop the Windows service.
pub async fn stop() -> Result<ServiceResult, String> {
    let output = tokio::process::Command::new("sc.exe")
        .args(["stop", SERVICE_NAME])
        .output()
        .await
        .map_err(|e| format!("sc.exe stop failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("{}{}", stdout.trim(), stderr.trim());
        if combined.contains("Access is denied") || combined.contains("FAILED 5") {
            return Err("Administrator privileges required. Run as Administrator and try again.".to_string());
        }
        return Err(format!("Service stop failed: {}", combined));
    }

    Ok(ServiceResult {
        success: true,
        message: format!("{} stopped.", SERVICE_NAME),
    })
}

/// Query Windows service status via `sc.exe query`.
pub async fn status() -> Result<ServiceStatus, String> {
    let output = tokio::process::Command::new("sc.exe")
        .args(["query", SERVICE_NAME])
        .output()
        .await
        .map_err(|e| format!("sc.exe query failed: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    if !output.status.success() || stdout.contains("does not exist") {
        return Ok(ServiceStatus {
            installed: false,
            running: false,
            enabled: false,
            pid: None,
            detail: "Service not installed.".to_string(),
        });
    }

    // Parse STATE line: e.g., "STATE : 4  RUNNING"
    let running = stdout.contains("RUNNING");
    let stopped = stdout.contains("STOPPED");

    // Parse PID line: e.g., "PID : 1234"
    let pid = stdout
        .lines()
        .find(|l| l.contains("PID"))
        .and_then(|l| {
            l.split(':')
                .nth(1)
                .and_then(|v| v.trim().parse::<u32>().ok())
                .filter(|&p| p > 0)
        });

    // Check if auto-start
    let qc_output = tokio::process::Command::new("sc.exe")
        .args(["qc", SERVICE_NAME])
        .output()
        .await
        .ok();
    let enabled = qc_output
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("AUTO_START"))
        .unwrap_or(false);

    let state_str = if running {
        "RUNNING"
    } else if stopped {
        "STOPPED"
    } else {
        "UNKNOWN"
    };

    Ok(ServiceStatus {
        installed: true,
        running,
        enabled,
        pid,
        detail: format!("state={}, auto_start={}", state_str, enabled),
    })
}
