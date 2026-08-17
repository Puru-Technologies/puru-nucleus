//! Windows daemon management via Task Scheduler (schtasks.exe).
//!
//! Uses a scheduled task instead of a Windows Service because Windows Services
//! require SCM integration (StartServiceCtrlDispatcher) which adds FFI dependencies
//! that can fail on some Windows versions. Task Scheduler provides the same
//! functionality (auto-start, restart on failure, background execution) without
//! the SCM requirement.

use super::{ServiceResult, ServiceStatus, get_exe_path};

const TASK_NAME: &str = "PuruDC";
const DISPLAY_NAME: &str = "Puru DC";

/// XML-escape a string for embedding in the task definition.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Register puru-dc as a scheduled task that runs the daemon "always":
/// at every boot (no login required), as LocalSystem with highest privileges,
/// auto-restarting on failure, with NO execution time limit (a plain
/// `schtasks /SC ONSTART` task inherits the default 72h limit and would be killed)
/// and unaffected by battery state.
pub async fn install() -> Result<ServiceResult, String> {
    let exe_path = get_exe_path()?;

    // Full task definition (Task Scheduler 1.2 schema). S-1-5-18 = LocalSystem.
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Puru DC background daemon (heartbeat, commands, backups).</Description>
  </RegistrationInfo>
  <Triggers>
    <BootTrigger><Enabled>true</Enabled></BootTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>S-1-5-18</UserId>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>999</Count>
    </RestartOnFailure>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>daemon</Arguments>
    </Exec>
  </Actions>
</Task>"#,
        exe = xml_escape(&exe_path),
    );

    let xml_path = std::env::temp_dir().join("puru-dc-daemon.xml");
    // `schtasks /Create /XML` requires the file to be UTF-16LE (with BOM);
    // a UTF-8 file fails with "unable to switch the encoding". Encode explicitly.
    let mut utf16: Vec<u8> = vec![0xFF, 0xFE]; // UTF-16LE BOM
    for unit in xml.encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(&xml_path, &utf16)
        .map_err(|e| format!("Could not write task definition: {}", e))?;

    let output = crate::process::silent_cmd("schtasks")
        .args([
            "/Create",
            "/TN",
            TASK_NAME,
            "/XML",
            &xml_path.to_string_lossy(),
            "/F",
        ])
        .output()
        .await
        .map_err(|e| format!("schtasks create failed: {}", e))?;
    let _ = std::fs::remove_file(&xml_path);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("Task registration failed: {}{}", stdout.trim(), stderr.trim()));
    }

    // Start it now (in addition to the boot trigger).
    let _ = crate::process::silent_cmd("schtasks")
        .args(["/Run", "/TN", TASK_NAME])
        .output()
        .await;

    Ok(ServiceResult {
        success: true,
        message: format!(
            "{} installed — starts at boot as SYSTEM and auto-restarts on failure.",
            DISPLAY_NAME
        ),
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
/// process. The GUI and the daemon share the same `puru-dc.exe` image, so we
/// must NEVER kill by image name / window — we target the process whose command
/// line contains the `daemon` argument. (The previous `WINDOWTITLE eq *` filter
/// matched the windowed GUI and missed the headless daemon — exactly backwards.)
pub async fn stop() -> Result<ServiceResult, String> {
    // End the scheduled task instance (terminates the task's process, targeted).
    let _ = crate::process::silent_cmd("schtasks")
        .args(["/End", "/TN", TASK_NAME])
        .output()
        .await;

    // Kill any puru-dc *daemon* process specifically, matched by command
    // line — the GUI's command line does not contain "daemon", so it is safe.
    let _ = crate::process::silent_cmd("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_Process | Where-Object { $_.Name -eq 'puru-dc.exe' -and $_.CommandLine -like '*daemon*' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }",
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

    // Check if the DAEMON process is actually running. The daemon and the GUI
    // share the puru-dc.exe image, so match by command line ("daemon") — a
    // plain image-name check would count a running GUI as the daemon.
    let ps_output = crate::process::silent_cmd("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_Process | Where-Object { $_.Name -eq 'puru-dc.exe' -and $_.CommandLine -like '*daemon*' } | Select-Object -First 1 -ExpandProperty ProcessId",
        ])
        .output()
        .await
        .ok();

    let mut running = false;
    let mut pid = None;

    if let Some(ps) = ps_output {
        let out = String::from_utf8_lossy(&ps.stdout);
        if let Ok(p) = out.trim().parse::<u32>() {
            running = true;
            pid = Some(p);
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
