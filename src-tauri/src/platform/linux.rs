//! Linux systemd service management

use super::{ServiceResult, ServiceStatus, get_exe_path};

const SERVICE_NAME: &str = "puru-nucleus";
const UNIT_PATH: &str = "/etc/systemd/system/puru-nucleus.service";

fn unit_file_content(exe_path: &str) -> String {
    format!(
        r#"[Unit]
Description=Puru Nucleus - Hospital Deployment Manager
After=network.target docker.service
Requires=docker.service

[Service]
Type=simple
ExecStart={exe_path} daemon
Restart=on-failure
RestartSec=10
Environment=RUST_LOG=puru_nucleus=info
WorkingDirectory=/etc/puru-nucleus

[Install]
WantedBy=multi-user.target
"#
    )
}

/// Install the systemd service unit and enable it.
pub async fn install() -> Result<ServiceResult, String> {
    let exe_path = get_exe_path()?;
    let content = unit_file_content(&exe_path);

    std::fs::write(UNIT_PATH, content)
        .map_err(|e| format!("Failed to write unit file (run as root?): {}", e))?;

    run_systemctl(&["daemon-reload"]).await?;
    run_systemctl(&["enable", SERVICE_NAME]).await?;

    Ok(ServiceResult {
        success: true,
        message: format!(
            "Systemd service installed at {} and enabled. Run `sudo systemctl start {}` to start.",
            UNIT_PATH, SERVICE_NAME
        ),
    })
}

/// Disable and remove the systemd service.
pub async fn uninstall() -> Result<ServiceResult, String> {
    // Stop if running (ignore errors)
    let _ = run_systemctl(&["stop", SERVICE_NAME]).await;
    let _ = run_systemctl(&["disable", SERVICE_NAME]).await;

    if std::path::Path::new(UNIT_PATH).exists() {
        std::fs::remove_file(UNIT_PATH)
            .map_err(|e| format!("Failed to remove unit file: {}", e))?;
    }

    run_systemctl(&["daemon-reload"]).await?;

    Ok(ServiceResult {
        success: true,
        message: "Systemd service uninstalled.".to_string(),
    })
}

/// Start the systemd service.
pub async fn start() -> Result<ServiceResult, String> {
    run_systemctl(&["start", SERVICE_NAME]).await?;
    Ok(ServiceResult {
        success: true,
        message: format!("{} started.", SERVICE_NAME),
    })
}

/// Stop the systemd service.
pub async fn stop() -> Result<ServiceResult, String> {
    run_systemctl(&["stop", SERVICE_NAME]).await?;
    Ok(ServiceResult {
        success: true,
        message: format!("{} stopped.", SERVICE_NAME),
    })
}

/// Query service status via `systemctl is-active` and `systemctl is-enabled`.
pub async fn status() -> Result<ServiceStatus, String> {
    let installed = std::path::Path::new(UNIT_PATH).exists();

    if !installed {
        return Ok(ServiceStatus {
            installed: false,
            running: false,
            enabled: false,
            pid: None,
            detail: "Service not installed.".to_string(),
        });
    }

    let active_output = crate::process::silent_cmd("systemctl")
        .args(["is-active", SERVICE_NAME])
        .output()
        .await
        .map_err(|e| format!("systemctl is-active failed: {}", e))?;
    let active_str = String::from_utf8_lossy(&active_output.stdout).trim().to_string();
    let running = active_str == "active";

    let enabled_output = crate::process::silent_cmd("systemctl")
        .args(["is-enabled", SERVICE_NAME])
        .output()
        .await
        .map_err(|e| format!("systemctl is-enabled failed: {}", e))?;
    let enabled_str = String::from_utf8_lossy(&enabled_output.stdout).trim().to_string();
    let enabled = enabled_str == "enabled";

    // Try to get PID from systemctl show
    let pid = if running {
        let show_output = crate::process::silent_cmd("systemctl")
            .args(["show", SERVICE_NAME, "--property=MainPID", "--value"])
            .output()
            .await
            .ok();
        show_output.and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse::<u32>()
                .ok()
                .filter(|&p| p > 0)
        })
    } else {
        None
    };

    let detail = format!("active={}, enabled={}", active_str, enabled_str);

    Ok(ServiceStatus {
        installed,
        running,
        enabled,
        pid,
        detail,
    })
}

/// Helper: run systemctl with args, returning error on non-zero exit.
async fn run_systemctl(args: &[&str]) -> Result<(), String> {
    let output = crate::process::silent_cmd("systemctl")
        .args(args)
        .output()
        .await
        .map_err(|e| format!("Failed to run systemctl {}: {}", args.join(" "), e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("systemctl {} failed: {}", args.join(" "), stderr.trim()));
    }

    Ok(())
}
