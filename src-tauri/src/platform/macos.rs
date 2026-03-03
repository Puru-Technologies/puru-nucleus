//! macOS launchd service management

use super::{ServiceResult, ServiceStatus, get_exe_path};

const LABEL: &str = "com.puru.nucleus";
const PLIST_PATH: &str = "/Library/LaunchDaemons/com.puru.nucleus.plist";
const LOG_DIR: &str = "/usr/local/var/log/puru-nucleus";

fn plist_content(exe_path: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe_path}</string>
        <string>daemon</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{LOG_DIR}/puru-nucleus.log</string>
    <key>StandardErrorPath</key>
    <string>{LOG_DIR}/puru-nucleus.err</string>
    <key>WorkingDirectory</key>
    <string>/usr/local/etc/puru-nucleus</string>
</dict>
</plist>"#
    )
}

/// Install the launchd plist and load it.
pub async fn install() -> Result<ServiceResult, String> {
    let exe_path = get_exe_path()?;

    // Ensure log directory exists
    std::fs::create_dir_all(LOG_DIR)
        .map_err(|e| format!("Failed to create log directory {}: {}", LOG_DIR, e))?;

    let content = plist_content(&exe_path);
    std::fs::write(PLIST_PATH, content)
        .map_err(|e| format!("Failed to write plist (run as root?): {}", e))?;

    // Load the daemon
    let output = tokio::process::Command::new("launchctl")
        .args(["load", PLIST_PATH])
        .output()
        .await
        .map_err(|e| format!("launchctl load failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.contains("already loaded") && !stderr.contains("service already loaded") {
            return Err(format!("launchctl load failed: {}", stderr.trim()));
        }
    }

    Ok(ServiceResult {
        success: true,
        message: format!("Launch daemon installed at {} and loaded.", PLIST_PATH),
    })
}

/// Unload and remove the launchd plist.
pub async fn uninstall() -> Result<ServiceResult, String> {
    // Unload if loaded (ignore errors)
    let _ = tokio::process::Command::new("launchctl")
        .args(["unload", PLIST_PATH])
        .output()
        .await;

    if std::path::Path::new(PLIST_PATH).exists() {
        std::fs::remove_file(PLIST_PATH)
            .map_err(|e| format!("Failed to remove plist: {}", e))?;
    }

    Ok(ServiceResult {
        success: true,
        message: "Launch daemon uninstalled.".to_string(),
    })
}

/// Start the launch daemon.
pub async fn start() -> Result<ServiceResult, String> {
    let output = tokio::process::Command::new("launchctl")
        .args(["start", LABEL])
        .output()
        .await
        .map_err(|e| format!("launchctl start failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("launchctl start failed: {}", stderr.trim()));
    }

    Ok(ServiceResult {
        success: true,
        message: format!("{} started.", LABEL),
    })
}

/// Stop the launch daemon.
pub async fn stop() -> Result<ServiceResult, String> {
    let output = tokio::process::Command::new("launchctl")
        .args(["stop", LABEL])
        .output()
        .await
        .map_err(|e| format!("launchctl stop failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("launchctl stop failed: {}", stderr.trim()));
    }

    Ok(ServiceResult {
        success: true,
        message: format!("{} stopped.", LABEL),
    })
}

/// Query launchd service status via `launchctl list`.
pub async fn status() -> Result<ServiceStatus, String> {
    let installed = std::path::Path::new(PLIST_PATH).exists();

    if !installed {
        return Ok(ServiceStatus {
            installed: false,
            running: false,
            enabled: false,
            pid: None,
            detail: "Service not installed.".to_string(),
        });
    }

    // `launchctl list <label>` returns: PID\tLastExitStatus\tLabel
    let output = tokio::process::Command::new("launchctl")
        .args(["list", LABEL])
        .output()
        .await
        .map_err(|e| format!("launchctl list failed: {}", e))?;

    if !output.status.success() {
        // Not loaded means installed but not running
        return Ok(ServiceStatus {
            installed: true,
            running: false,
            enabled: false,
            pid: None,
            detail: "Installed but not loaded.".to_string(),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Parse: first column is PID (or `-` if not running)
    let pid = stdout
        .lines()
        .last()
        .and_then(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            parts.first().and_then(|p| p.trim().parse::<u32>().ok())
        });

    let running = pid.is_some();

    Ok(ServiceStatus {
        installed: true,
        running,
        enabled: true, // If loaded in launchd it's enabled
        pid,
        detail: stdout.trim().to_string(),
    })
}
