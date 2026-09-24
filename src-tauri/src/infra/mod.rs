//! Infrastructure status + control for the prerequisite that backs the native
//! services: MySQL (Database). Surfaced as a read-only status row in the
//! Services screen — with a diagnosis when down — plus start/stop/restart of
//! the underlying Windows service and a log tail, so an operator can see
//! *why* (e.g. a boot crash) and act.

use std::time::Duration;

use crate::config::NucleusConfig;
use crate::services::{ServiceInfo, ServiceStatus};

const FILE_SERVER_PORT: u16 = 81;

/// TCP-reachable on localhost within a short timeout.
fn port_open(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_millis(600),
    )
    .is_ok()
}

/// Windows service state from `sc query`: "RUNNING" | "STOPPED" |
/// "START_PENDING" | "UNKNOWN" (also "UNKNOWN" when sc can't be run).
#[cfg(target_os = "windows")]
fn service_state(svc: &str) -> String {
    match crate::process::silent_std_cmd("sc").args(["query", svc]).output() {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout).to_uppercase();
            if s.contains("RUNNING") {
                "RUNNING".into()
            } else if s.contains("START_PENDING") {
                "START_PENDING".into()
            } else if s.contains("STOP_PENDING") {
                "STOP_PENDING".into()
            } else if s.contains("STOPPED") {
                "STOPPED".into()
            } else {
                "UNKNOWN".into()
            }
        }
        Err(_) => "UNKNOWN".into(),
    }
}
#[cfg(not(target_os = "windows"))]
fn service_state(_svc: &str) -> String {
    "UNKNOWN".into()
}

/// Build the infra rows (Database, and the File Server when a data tree is
/// being served) for the Services list.
pub async fn infra_rows(config: &NucleusConfig) -> Vec<ServiceInfo> {
    let mut rows = vec![mysql_row(config)];
    if let Some(fs) = file_server_row(config) {
        rows.push(fs);
    }
    rows
}

/// The static File Server (nginx serving the `puru_data` tree on :81). It shares
/// the managed nginx process with the web app, so its status is simply "is :81
/// answering", and control routes to the web server. Shown when a data path is
/// configured or the port is already up.
fn file_server_row(config: &NucleusConfig) -> Option<ServiceInfo> {
    let configured = config
        .puru_data_path
        .as_deref()
        .map(|p| !p.is_empty())
        .unwrap_or(false);
    let up = port_open(FILE_SERVER_PORT);
    if !configured && !up {
        return None;
    }
    let (status, detail) = if up {
        (
            ServiceStatus::Running,
            Some("Serving the data tree on port 81 (via the managed web server).".to_string()),
        )
    } else {
        (
            ServiceStatus::Stopped,
            Some(
                "Not answering on port 81 — start the web server (it shares the same nginx as the web app)."
                    .to_string(),
            ),
        )
    };
    Some(ServiceInfo {
        name: "File Server".into(),
        container_name: String::new(),
        image: String::new(),
        status,
        health: None,
        ports: vec![FILE_SERVER_PORT.to_string()],
        uptime: None,
        health_response_ms: None,
        detail,
        infra: true,
    })
}

fn mysql_row(config: &NucleusConfig) -> ServiceInfo {
    let port = if config.mysql_port == 0 { 3306 } else { config.mysql_port };
    let up = port_open(port);
    let (status, detail) = if up {
        (ServiceStatus::Running, None)
    } else {
        let detail = match crate::installer::mysql_service_name() {
            Some(name) => match service_state(&name).as_str() {
                "STOPPED" => format!("Database service '{}' is stopped.", name),
                "START_PENDING" => format!("Database service '{}' is starting…", name),
                _ => format!("Database service '{}' is not answering on port {}.", name, port),
            },
            None => "Database is not installed (no Windows service found).".to_string(),
        };
        (ServiceStatus::Stopped, Some(detail))
    };
    ServiceInfo {
        name: "Database".into(),
        container_name: String::new(),
        image: String::new(),
        status,
        health: None,
        ports: vec![format!("{}:{}", port, port)],
        uptime: None,
        health_response_ms: None,
        detail,
        infra: true,
    }
}

// ── Control (start / stop / restart) ────────────────────────────────────────

fn infra_service_name(display: &str) -> Option<String> {
    match display {
        "Database" => crate::installer::mysql_service_name(),
        _ => None,
    }
}

/// Start / stop / restart the Windows service backing an infra component.
/// Managing a Windows service needs admin, so this runs `sc` elevated (one UAC
/// prompt). Returns a short status message.
pub async fn control(display: &str, action: &str) -> Result<String, String> {
    // The File Server is the managed nginx serving :81 — not a Windows service.
    // Route its control to the web server (this also affects the web app on :80,
    // since they are one nginx process).
    if display == "File Server" {
        let config = crate::config::load_config().map_err(|e| e.to_string())?;
        match action {
            "start" => crate::webserver::start(&config).await.map_err(|e| e.to_string())?,
            "stop" => crate::webserver::stop(&config).await.map_err(|e| e.to_string())?,
            "restart" => {
                let _ = crate::webserver::stop(&config).await;
                crate::webserver::start(&config).await.map_err(|e| e.to_string())?;
            }
            other => return Err(format!("Unknown action '{}'.", other)),
        }
        return Ok(format!(
            "Web server {} requested (File Server on port 81).",
            action
        ));
    }

    let svc = infra_service_name(display)
        .ok_or_else(|| format!("{} is not installed (no Windows service found).", display))?;

    #[cfg(target_os = "windows")]
    {
        let ps = match action {
            "start" => format!("Start-Service -Name '{}'", svc),
            "stop" => format!("Stop-Service -Name '{}' -Force", svc),
            "restart" => format!("Restart-Service -Name '{}' -Force", svc),
            other => return Err(format!("Unknown action '{}'.", other)),
        };
        let status = crate::process::silent_cmd("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("Start-Process powershell -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '-NoProfile','-Command',\"{}\"", ps),
            ])
            .status()
            .await
            .map_err(|e| format!("Failed to run service control: {}", e))?;
        if !status.success() {
            return Err(format!("{} {} was cancelled or failed (needs administrator).", display, action));
        }
        Ok(format!("{} {} requested for service '{}'.", display, action, svc))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = svc;
        Err("Infra service control is only implemented on Windows.".into())
    }
}

// ── Log tail ────────────────────────────────────────────────────────────────

/// Return the last `lines` of the infra component's log (lossy-decoded).
pub fn read_log(display: &str, lines: usize) -> Result<String, String> {
    let path = match display {
        "Database" => mysql_error_log()
            .ok_or_else(|| "Database error log location could not be determined.".to_string())?,
        "File Server" => {
            let config = crate::config::load_config().map_err(|e| e.to_string())?;
            crate::webserver::error_log_path(&config)
                .ok_or_else(|| "No web server (nginx) log found yet.".to_string())?
        }
        _ => return Err(format!("Unknown infra component '{}'.", display)),
    };
    let bytes = std::fs::read(&path).map_err(|e| format!("Read {}: {}", path.display(), e))?;
    let text = String::from_utf8_lossy(&bytes);
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);
    Ok(format!("[{}]\n{}", path.display(), all[start..].join("\n")))
}

/// Best-effort MySQL error-log location (common ProgramData / install data dirs).
#[cfg(target_os = "windows")]
fn mysql_error_log() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    let host = std::env::var("COMPUTERNAME").unwrap_or_default();
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(pd) = std::env::var("ProgramData") {
        // ProgramData\MySQL\MySQL Server X.Y\Data\{host}.err
        if let Ok(entries) = std::fs::read_dir(format!(r"{}\MySQL", pd)) {
            for e in entries.flatten() {
                let data = e.path().join("Data");
                if data.is_dir() {
                    candidates.push(data.join(format!("{}.err", host)));
                    // also any *.err in that data dir
                    if let Ok(files) = std::fs::read_dir(&data) {
                        for f in files.flatten() {
                            let p = f.path();
                            if p.extension().map(|x| x == "err").unwrap_or(false) {
                                candidates.push(p);
                            }
                        }
                    }
                }
            }
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}
#[cfg(not(target_os = "windows"))]
fn mysql_error_log() -> Option<std::path::PathBuf> {
    None
}
