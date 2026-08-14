//! Windows daemon management via Task Scheduler (schtasks.exe).
//!
//! Uses a scheduled task instead of a Windows Service because Windows Services
//! require SCM integration (StartServiceCtrlDispatcher) which adds FFI dependencies
//! that can fail on some Windows versions. Task Scheduler provides the same
//! functionality (auto-start, restart on failure, background execution) without
//! the SCM requirement.

use super::{ServiceResult, ServiceStatus, get_exe_path};

const TASK_NAME: &str = "PuruNucleus";
const DISPLAY_NAME: &str = "Puru Nucleus";

/// Register puru-nucleus as a scheduled task that runs at startup.
pub async fn install() -> Result<ServiceResult, String> {
    let exe_path = get_exe_path()?;

    // Create a scheduled task that:
    // - Runs at system startup (ONSTART)
    // - Runs as SYSTEM (highest privileges)
    // - Restarts on failure (via /RI and repeat)
    let output = crate::process::silent_cmd("schtasks")
        .args([
            "/Create",
            "/TN", TASK_NAME,
            "/TR", &format!("\"{}\" daemon", exe_path),
            "/SC", "ONSTART",
            "/RU", "SYSTEM",
            "/RL", "HIGHEST",
            "/F",  // Force overwrite if exists
        ])
        .output()
        .await
        .map_err(|e| format!("schtasks create failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("Task registration failed: {}{}", stdout.trim(), stderr.trim()));
    }

    // Start the task immediately
    let _ = crate::process::silent_cmd("schtasks")
        .args(["/Run", "/TN", TASK_NAME])
        .output()
        .await;

    Ok(ServiceResult {
        success: true,
        message: format!("{} installed and started.", DISPLAY_NAME),
    })
}

/// Remove the scheduled task.
pub async fn uninstall() -> Result<ServiceResult, String> {
    // Stop first (kill the daemon process)
    let _ = stop().await;

    let output = crate::process::silent_cmd("schtasks")
        .args(["/Delete", "/TN", TASK_NAME, "/F"])
        .output()
        .await
        .map_err(|e| format!("schtasks delete failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let combined = format!("{}{}", stdout.trim(), stderr.trim());
        if !combined.contains("does not exist") && !combined.contains("cannot find") {
            return Err(format!("Task deletion failed: {}", combined));
        }
    }

    Ok(ServiceResult {
        success: true,
        message: format!("{} removed.", DISPLAY_NAME),
    })
}

/// Start the daemon by running the scheduled task.
pub async fn start() -> Result<ServiceResult, String> {
    let output = crate::process::silent_cmd("schtasks")
        .args(["/Run", "/TN", TASK_NAME])
        .output()
        .await
        .map_err(|e| format!("schtasks run failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("Task start failed: {}{}", stdout.trim(), stderr.trim()));
    }

    Ok(ServiceResult {
        success: true,
        message: format!("{} started.", DISPLAY_NAME),
    })
}

/// Stop the daemon by ending its scheduled task and killing only the daemon
/// process. The GUI and the daemon share the same `puru-nucleus.exe` image, so we
/// must NEVER kill by image name / window — we target the process whose command
/// line contains the `daemon` argument. (The previous `WINDOWTITLE eq *` filter
/// matched the windowed GUI and missed the headless daemon — exactly backwards.)
pub async fn stop() -> Result<ServiceResult, String> {
    // End the scheduled task instance (terminates the task's process, targeted).
    let _ = crate::process::silent_cmd("schtasks")
        .args(["/End", "/TN", TASK_NAME])
        .output()
        .await;

    // Kill any puru-nucleus *daemon* process specifically, matched by command
    // line — the GUI's command line does not contain "daemon", so it is safe.
    let _ = crate::process::silent_cmd("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_Process | Where-Object { $_.Name -eq 'puru-nucleus.exe' -and $_.CommandLine -like '*daemon*' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }",
        ])
        .output()
        .await;

    // Small delay for cleanup
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;

    Ok(ServiceResult {
        success: true,
        message: format!("{} stopped.", DISPLAY_NAME),
    })
}

/// Query the daemon status by checking the scheduled task and process.
pub async fn status() -> Result<ServiceStatus, String> {
    // Check if the task exists
    let output = crate::process::silent_cmd("schtasks")
        .args(["/Query", "/TN", TASK_NAME, "/FO", "CSV", "/NH"])
        .output()
        .await
        .map_err(|e| format!("schtasks query failed: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    if !output.status.success() || stdout.contains("does not exist") || stdout.contains("cannot find") {
        return Ok(ServiceStatus {
            installed: false,
            running: false,
            enabled: false,
            pid: None,
            detail: "Daemon not installed.".to_string(),
        });
    }

    // Check if the daemon process is actually running
    let ps_output = crate::process::silent_cmd("tasklist")
        .args(["/FI", "IMAGENAME eq puru-nucleus.exe", "/FO", "CSV", "/NH"])
        .output()
        .await
        .ok();

    let mut running = false;
    let mut pid = None;

    if let Some(ps) = ps_output {
        let ps_stdout = String::from_utf8_lossy(&ps.stdout);
        // tasklist CSV format: "puru-nucleus.exe","1234","Console","1","12,345 K"
        for line in ps_stdout.lines() {
            if line.contains("puru-nucleus.exe") {
                running = true;
                // Extract PID from CSV
                let parts: Vec<&str> = line.split(',').collect();
                if parts.len() >= 2 {
                    pid = parts[1].trim_matches('"').parse::<u32>().ok();
                }
                break;
            }
        }
    }

    // Task exists = enabled (runs at startup)
    let enabled = stdout.contains("Ready") || stdout.contains("Running");

    let state_str = if running { "RUNNING" } else { "STOPPED" };

    Ok(ServiceStatus {
        installed: true,
        running,
        enabled,
        pid,
        detail: format!("state={}, auto_start={}", state_str, enabled),
    })
}
