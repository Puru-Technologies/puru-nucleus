//! Infrastructure status + control for the two prerequisites that back the
//! native services: MySQL (Database) and RabbitMQ (Message Broker). These are
//! surfaced as read-only status rows in the Services screen — with a diagnosis
//! when down — plus start/stop/restart of the underlying Windows service and a
//! log tail, so an operator can see *why* (e.g. a RabbitMQ boot crash) and act.

use std::time::Duration;

use crate::config::NucleusConfig;
use crate::services::{ServiceInfo, ServiceStatus};

const RMQ_AMQP: u16 = 5672;
const RMQ_MGMT: u16 = 15672;
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

const RMQ_API: &str = "http://127.0.0.1:15672/api";
const RMQ_ADMIN_USER: &str = "guest"; // localhost-only admin, present even on a virgin node
const RMQ_ADMIN_PASS: &str = "guest";
const RMQ_APP_USER: &str = "puru";
const RMQ_APP_PASS: &str = "puru123";

/// GET a RabbitMQ Management API path (as the local admin). `None` when the
/// management plugin/API isn't reachable.
async fn rmq_api_get(path: &str) -> Option<serde_json::Value> {
    let resp = reqwest::Client::new()
        .get(format!("{}/{}", RMQ_API, path))
        .basic_auth(RMQ_ADMIN_USER, Some(RMQ_ADMIN_PASS))
        .timeout(Duration::from_secs(4))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<serde_json::Value>().await.ok()
}

/// Build the infra rows (Database, Message Broker, and the File Server when a
/// data tree is being served) for the Services list.
pub async fn infra_rows(config: &NucleusConfig) -> Vec<ServiceInfo> {
    let mut rows = vec![mysql_row(config), rabbitmq_row().await];
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

/// RabbitMQ row via the Management API (real health + user presence). Falls back
/// to TCP + Windows-service + log diagnosis when the management API is off.
async fn rabbitmq_row() -> ServiceInfo {
    let row = |status: ServiceStatus, detail: Option<String>| ServiceInfo {
        name: "Message Broker".into(),
        container_name: String::new(),
        image: String::new(),
        status,
        health: None,
        ports: vec!["5672".into(), "15672".into()],
        uptime: None,
        health_response_ms: None,
        detail,
        infra: true,
    };

    match rmq_api_get("nodes").await {
        Some(nodes) => {
            let node = nodes.as_array().and_then(|a| a.first());
            let running = node.and_then(|n| n["running"].as_bool()).unwrap_or(false);
            if !running {
                return row(ServiceStatus::Stopped, Some("Broker node is not running.".into()));
            }
            let mem_alarm = node.and_then(|n| n["mem_alarm"].as_bool()).unwrap_or(false);
            let disk_alarm = node.and_then(|n| n["disk_free_alarm"].as_bool()).unwrap_or(false);
            let alive = rmq_api_get("aliveness-test/%2f")
                .await
                .and_then(|v| v["status"].as_str().map(|s| s == "ok"))
                .unwrap_or(false);
            // Presence of the app user — a broker reset (Khepri virgin node) drops
            // it, which is what aborts every service on boot.
            let puru_missing = rmq_api_get(&format!("users/{}", RMQ_APP_USER)).await.is_none();

            let mut warns: Vec<String> = Vec::new();
            if puru_missing {
                warns.push(format!(
                    "app user '{}' is MISSING — services can't log in (broker was reset). Restart a service to self-heal, or it re-creates on next start.",
                    RMQ_APP_USER
                ));
            }
            if mem_alarm {
                warns.push("memory alarm active — publishers are blocked".into());
            }
            if disk_alarm {
                warns.push("disk free-space alarm active — publishers are blocked".into());
            }
            if !alive {
                warns.push("aliveness check failing on '/'".into());
            }
            let detail = if warns.is_empty() { None } else { Some(warns.join("\n")) };
            // Broker is up but unusable (user gone / alarm / not alive) → Error.
            let status = if puru_missing || mem_alarm || disk_alarm || !alive {
                ServiceStatus::Error
            } else {
                ServiceStatus::Running
            };
            row(status, detail)
        }
        // Management API unreachable — TCP + Windows-service + log fallback.
        None => rabbitmq_row_fallback(),
    }
}

/// (Re)create the `puru` app user with full permissions on `/` via the Management
/// API — cookie-free, using the local `guest` admin. Best-effort self-heal for
/// the Khepri virgin-node reset that silently drops the user. No-op / Err when
/// the management API is off.
pub async fn ensure_rabbitmq_user() -> Result<(), String> {
    let client = reqwest::Client::new();
    // Probe the API first so we don't spam errors when the plugin is off.
    if rmq_api_get("overview").await.is_none() {
        return Err("Message broker management API not reachable (plugin off?).".into());
    }
    let user_body = serde_json::json!({ "password": RMQ_APP_PASS, "tags": "administrator" });
    let put = |path: String, body: serde_json::Value| {
        let c = client.clone();
        async move {
            c.put(format!("{}/{}", RMQ_API, path))
                .basic_auth(RMQ_ADMIN_USER, Some(RMQ_ADMIN_PASS))
                .json(&body)
                .timeout(Duration::from_secs(5))
                .send()
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| {
                    if r.status().is_success() {
                        Ok(())
                    } else {
                        Err(format!("HTTP {}", r.status()))
                    }
                })
        }
    };
    put(format!("users/{}", RMQ_APP_USER), user_body).await?;
    put(
        format!("permissions/%2f/{}", RMQ_APP_USER),
        serde_json::json!({ "configure": ".*", "write": ".*", "read": ".*" }),
    )
    .await?;
    tracing::info!("RabbitMQ: ensured app user '{}' via management API", RMQ_APP_USER);
    Ok(())
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

/// TCP + Windows-service + node-log diagnosis, used when the Management API is
/// unreachable (management plugin off).
fn rabbitmq_row_fallback() -> ServiceInfo {
    let up = port_open(RMQ_AMQP);
    let (status, detail) = if up {
        let d = if port_open(RMQ_MGMT) {
            None
        } else {
            Some("Broker up, but the management UI (15672) isn't reachable — enable the management plugin if you need it.".to_string())
        };
        (ServiceStatus::Running, d)
    } else {
        let svc = rabbitmq_service_name();
        let base = match svc.as_deref() {
            None => "Message broker is not installed (no Windows service found).".to_string(),
            Some(name) => match service_state(name).as_str() {
                "STOPPED" => format!("Message broker service '{}' is stopped — start it.", name),
                "START_PENDING" => format!("Message broker service '{}' is starting…", name),
                _ => format!(
                    "Message broker service '{}' is running but the node isn't accepting connections on {} — it likely crashed on boot (Erlang cookie mismatch, or a missing/enabled .ez plugin).",
                    name, RMQ_AMQP
                ),
            },
        };
        let detail = match rabbitmq_last_error() {
            Some(err) => format!("{}\nLast log error: {}", base, err),
            None => base,
        };
        (ServiceStatus::Stopped, Some(detail))
    };
    ServiceInfo {
        name: "Message Broker".into(),
        container_name: String::new(),
        image: String::new(),
        status,
        health: None,
        ports: vec!["5672".into(), "15672".into()],
        uptime: None,
        health_response_ms: None,
        detail,
        infra: true,
    }
}

fn rabbitmq_service_name() -> Option<String> {
    crate::commands::rabbitmq_service_name()
}

/// Candidate RabbitMQ log directories (SYSTEM service profile, user profile,
/// and an explicit RABBITMQ_LOG_BASE override).
#[cfg(target_os = "windows")]
fn rabbitmq_log_dirs() -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(base) = std::env::var("RABBITMQ_LOG_BASE") {
        v.push(base);
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        v.push(format!(r"{}\RabbitMQ\log", appdata));
    }
    v.push(r"C:\Windows\System32\config\systemprofile\AppData\Roaming\RabbitMQ\log".to_string());
    v
}
#[cfg(not(target_os = "windows"))]
fn rabbitmq_log_dirs() -> Vec<String> {
    vec!["/var/log/rabbitmq".to_string()]
}

/// Newest `rabbit@*.log` file across the candidate dirs.
fn newest_rabbitmq_log() -> Option<std::path::PathBuf> {
    let mut newest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
    for dir in rabbitmq_log_dirs() {
        let p = std::path::Path::new(&dir);
        if !p.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(p) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            let is_log = path
                .file_name()
                .map(|n| {
                    let n = n.to_string_lossy();
                    n.starts_with("rabbit") && n.ends_with(".log")
                })
                .unwrap_or(false);
            if !is_log {
                continue;
            }
            if let Ok(m) = std::fs::metadata(&path).and_then(|m| m.modified()) {
                if newest.as_ref().map(|(t, _)| m > *t).unwrap_or(true) {
                    newest = Some((m, path));
                }
            }
        }
    }
    newest.map(|(_, p)| p)
}

/// Last error/crash-ish line from the newest RabbitMQ node log — the deepest
/// "clue" for a boot failure, surfaced inline in the row detail.
fn rabbitmq_last_error() -> Option<String> {
    let log = newest_rabbitmq_log()?;
    let bytes = std::fs::read(&log).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let hit = text.lines().rev().find(|l| {
        let low = l.to_lowercase();
        low.contains("error")
            || low.contains("crash")
            || low.contains("abort")
            || low.contains("failed")
            || low.contains("boot failed")
    })?;
    Some(hit.trim().chars().take(280).collect())
}

// ── Control (start / stop / restart) ────────────────────────────────────────

fn infra_service_name(display: &str) -> Option<String> {
    match display {
        "Database" => crate::installer::mysql_service_name(),
        "Message Broker" => rabbitmq_service_name(),
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
        "Message Broker" => newest_rabbitmq_log()
            .ok_or_else(|| "No message broker log found (checked service + user profiles).".to_string())?,
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
