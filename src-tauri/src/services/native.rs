//! Native JAR process management — runs services as `java -jar` child processes
//!
//! Used when `deployment_mode = "native"` in nucleus.toml.
//! Each service runs as a child process with stdout/stderr redirected to log files.
//! PID files track running processes.

use crate::config::NucleusConfig;
use crate::error::NucleusError;
use crate::releases;
use crate::services::{HealthStatus, ServiceInfo, ServiceStatus};
use std::path::PathBuf;

// ── Constants ────────────────────────────────────────────────────────────────

/// Service port mapping for native mode (same ports as Docker)
const SERVICE_PORTS: &[(&str, u16)] = &[
    ("puru-xenon", 8081),
    ("puru-has", 8082),
    ("puru-pacs", 8083),
    ("puru-argon", 8084),
    ("puru-comm", 8085),
    ("puru-realtime", 8086),
    ("puru-neon", 8087),
    ("puru-bridge", 8094),
    ("puru-auth", 8080),
    ("puru-mercury", 8089),
    ("puru-integration", 8088),
];

/// Separate management/actuator port for services that run actuator on a
/// dedicated port (Spring Boot `management.server.port`) instead of the app
/// port. Health/readiness is probed here. Convention: mgmt port = app port +
/// 1000 (8081 → 9081, 8082 → 9082, …). Services NOT listed are assumed to
/// expose actuator on their app port (see SERVICE_PORTS) and fall back to it —
/// if absent there, the probe reports a "actuator not reachable" error.
const SERVICE_MGMT_PORTS: &[(&str, u16)] = &[
    ("puru-auth", 9080),
    ("puru-xenon", 9081),
    ("puru-has", 9082),
    ("puru-pacs", 9083),
    ("puru-argon", 9084),
    ("puru-comm", 9085),
    ("puru-realtime", 9086),
    ("puru-neon", 9087),
    ("puru-integration", 9088),
    ("puru-mercury", 9089),
    ("puru-bridge", 9094),
    ("puru-counter", 9095),
];

/// MySQL database used by each service (same mapping as the Docker compose generator)
const SERVICE_DBS: &[(&str, &str)] = &[
    ("puru-auth", "puru_auth"),
    ("puru-xenon", "puru_im"),
    ("puru-has", "puru_has"),
    ("puru-pacs", "puru_dicom"),
    ("puru-argon", "puru_path"),
    ("puru-comm", "puru_im"),
    ("puru-realtime", "puru_im"),
    ("puru-neon", "puru_med"),
    ("puru-mercury", "puru_im"),
    ("puru-counter", "puru_im"),
    ("puru-bridge", "puru_bridge"),
    ("puru-integration", "puru_im"),
];

/// Aggregate health endpoint for Spring Boot services (liveness-ish).
const HEALTH_ENDPOINT: &str = "/actuator/health";

/// Readiness probe — UP only once the app has *fully* started and is accepting
/// traffic. A crash-looping or never-fully-started service never reaches this,
/// which is exactly what distinguishes "the JVM is alive" from "the app works".
const READINESS_ENDPOINT: &str = "/actuator/health/readiness";

/// An alive process that hasn't reported ready within this many seconds is
/// treated as failing (stuck / crash-looping), not merely "starting".
const STARTUP_GRACE_SECS: u64 = 150;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn pid_path(config: &NucleusConfig, service: &str) -> PathBuf {
    config.native_logs_dir().join(format!("{}.pid", service))
}

fn log_path(config: &NucleusConfig, service: &str) -> PathBuf {
    config.native_logs_dir().join(format!("{}.log", service))
}

fn jar_path(config: &NucleusConfig, service: &str) -> PathBuf {
    config.jars_dir().join(format!("{}.jar", service))
}

fn service_port(service: &str) -> Option<u16> {
    SERVICE_PORTS
        .iter()
        .find(|(s, _)| *s == service)
        .map(|(_, p)| *p)
}

/// Port to probe actuator on: the dedicated management port if the service has
/// one, otherwise the app port.
fn health_probe_port(service: &str) -> Option<u16> {
    SERVICE_MGMT_PORTS
        .iter()
        .find(|(s, _)| *s == service)
        .map(|(_, p)| *p)
        .or_else(|| service_port(service))
}

fn db_for_service(service: &str) -> Option<&'static str> {
    SERVICE_DBS
        .iter()
        .find(|(s, _)| *s == service)
        .map(|(_, d)| *d)
}

/// Whether a service's build is present on disk. For JAR services that means
/// the `.jar` exists; for puru-hydrogen it means the nginx html root is laid
/// down. Used by the setup re-run to decide what needs pulling.
pub(crate) fn is_installed(config: &NucleusConfig, service: &str) -> bool {
    if service == "puru-hydrogen" {
        config.nginx_html_dir().join("index.html").exists()
    } else {
        jar_path(config, service).exists()
    }
}

/// Whether a JAR service's process is currently alive (by its PID file).
pub(crate) fn is_running(config: &NucleusConfig, service: &str) -> bool {
    read_pid(config, service)
        .map(is_process_alive)
        .unwrap_or(false)
}

/// Poll a TCP port on 127.0.0.1 until nothing answers (= port is free for
/// `bind`), giving up after `max_secs`. Used to gate `start_service` so we
/// don't spawn a JVM that's just going to die with "Port X already in use"
/// because the OS hasn't released the listening socket from a just-killed
/// instance yet. Returns true if the port is free, false on timeout.
async fn wait_for_port_free(port: u16, max_secs: u64) -> bool {
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(max_secs);
    loop {
        let busy = std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            std::time::Duration::from_millis(200),
        )
        .is_ok();
        if !busy {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }
}

/// Read PID from pid file, returns None if file doesn't exist or is invalid
fn read_pid(config: &NucleusConfig, service: &str) -> Option<u32> {
    let path = pid_path(config, service);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
}

/// Check if a process with the given PID is alive
fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // kill(pid, 0) checks if process exists without sending a signal
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(windows)]
    {
        // Use tasklist to check if PID exists — no extra dependencies needed.
        // silent_std_cmd suppresses the console window the watchdog would
        // otherwise flash for every service on every health cycle.
        crate::process::silent_std_cmd("tasklist")
            .args(["/FI", &format!("PID eq {}", pid), "/NH"])
            .output()
            .map(|o| {
                let out = String::from_utf8_lossy(&o.stdout);
                // tasklist output contains the PID if the process exists,
                // otherwise it says "no tasks" or "INFO: No tasks"
                out.contains(&pid.to_string()) && !out.contains("No tasks")
            })
            .unwrap_or(false)
    }
}

/// Load environment variables from env files
fn load_env_files(config: &NucleusConfig, service: &str) -> std::collections::HashMap<String, String> {
    let env_dir = config.env_dir();
    let mut vars = std::collections::HashMap::new();

    // Load general.env, database.env, etc. in order
    let env_files = ["general.env", "database.env", "rabbitmq.env"];
    for filename in &env_files {
        let path = env_dir.join(filename);
        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((key, value)) = line.split_once('=') {
                    vars.insert(key.trim().to_string(), value.trim().to_string());
                }
            }
        }
    }

    // database.env carries a shared URL template — `jdbc:mysql://host:{}/{}`.
    // Fill in the port and the per-service database here; each service has its
    // own schema, so a literal shared URL can never be correct for all of them.
    if let Some(url) = vars.get("SPRING_DATASOURCE_URL").cloned() {
        if url.contains("{}") {
            match db_for_service(service) {
                Some(db) => {
                    let port = vars
                        .get("MYSQL_PORT")
                        .cloned()
                        .unwrap_or_else(|| "3306".to_string());
                    let fixed = url.replacen("{}", &port, 1).replacen("{}", db, 1);
                    vars.insert("SPRING_DATASOURCE_URL".to_string(), fixed);
                }
                None => {
                    // Unknown service — a malformed URL is worse than none at all
                    vars.remove("SPRING_DATASOURCE_URL");
                }
            }
        }
    }

    vars
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Start a service as a java -jar process.
pub async fn start_service(name: &str, config: &NucleusConfig) -> Result<(), NucleusError> {
    // puru-hydrogen is the Angular UI: not a JAR, but static files served by a
    // managed nginx. Starting it means provisioning + (re)starting nginx.
    if name == "puru-hydrogen" {
        return crate::webserver::start(config).await;
    }

    // Check if already running
    if let Some(pid) = read_pid(config, name) {
        if is_process_alive(pid) {
            return Err(NucleusError::Validation(format!(
                "Service {} is already running (PID {})",
                name, pid
            )));
        }
        // Clean up stale PID file from a dead process
        let _ = std::fs::remove_file(pid_path(config, name));
    }

    // Resolve java binary path
    let java_version = releases::java_version_for_service(name).ok_or_else(|| {
        NucleusError::Validation(format!("Unknown service: {}", name))
    })?;

    let jre_dir = config.jres_dir().join(format!("temurin-{}", java_version));
    let java_bin = if cfg!(windows) {
        jre_dir.join("bin").join("java.exe")
    } else {
        jre_dir.join("bin").join("java")
    };

    if !java_bin.exists() {
        return Err(NucleusError::NotFound(format!(
            "JRE {} not found. Run `puru pull-jars` first.",
            java_version
        )));
    }

    // Check JAR exists
    let jar = jar_path(config, name);
    if !jar.exists() {
        return Err(NucleusError::NotFound(format!(
            "JAR for {} not found at {}. Run `puru pull-jars` first.",
            name,
            jar.display()
        )));
    }

    // Ensure logs directory exists
    let logs_dir = config.native_logs_dir();
    tokio::fs::create_dir_all(&logs_dir).await?;

    // Make sure the app port has actually been released by the previous
    // instance (or any other holder). Without this, a quick restart on
    // Windows often races the kernel: we taskkill /F the old JVM, spawn the
    // new one within ~1s, and the bind fails with "Port X already in use"
    // before the OS has released the listening socket. 15s is enough to
    // cover the normal release window AND to surface a clear error if a
    // rogue process is holding the port.
    if let Some(port) = service_port(name) {
        if !wait_for_port_free(port, 15).await {
            return Err(NucleusError::Validation(format!(
                "Port {} is still in use after 15s — kill the holder and retry",
                port
            )));
        }
    }

    // Open log file (append mode)
    let log_file_path = log_path(config, name);
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file_path)?;
    let stderr_file = log_file.try_clone()?;

    // Load env vars
    let env_vars = load_env_files(config, name);

    // Spawn process from the config dir — services resolve relative paths
    // (e.g. credential files) against their working directory
    let child = crate::process::silent_cmd(&java_bin.to_string_lossy())
        .args(["-jar", &jar.to_string_lossy()])
        .envs(env_vars)
        .current_dir(crate::config::config_dir())
        .stdout(log_file)
        .stderr(stderr_file)
        .spawn()
        .map_err(|e| NucleusError::Internal(format!("Failed to start {}: {}", name, e)))?;

    // Write PID file
    if let Some(pid) = child.id() {
        let pid_file = pid_path(config, name);
        std::fs::write(&pid_file, pid.to_string())?;
        tracing::info!("Started {} (PID {})", name, pid);
    }

    Ok(())
}

/// Stop a running service. SIGTERM → 30s wait → SIGKILL.
pub async fn stop_service(name: &str, config: &NucleusConfig) -> Result<(), NucleusError> {
    if name == "puru-hydrogen" {
        return crate::webserver::stop(config).await;
    }

    let pid = read_pid(config, name).ok_or_else(|| {
        NucleusError::NotFound(format!("No PID file for {}. Is it running?", name))
    })?;

    if !is_process_alive(pid) {
        // Clean up stale PID file
        let _ = std::fs::remove_file(pid_path(config, name));
        return Ok(());
    }

    tracing::info!("Stopping {} (PID {})...", name, pid);

    #[cfg(unix)]
    {
        // Send SIGTERM for graceful shutdown
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }

        // Wait up to 30 seconds for graceful shutdown
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(30);
        loop {
            if !is_process_alive(pid) {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                tracing::warn!("{} did not stop within 30s, sending SIGKILL", name);
                unsafe {
                    libc::kill(pid as i32, libc::SIGKILL);
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
    }

    #[cfg(windows)]
    {
        // Headless `java.exe` (spawned with CREATE_NO_WINDOW) has no console
        // window, so `taskkill` without `/F` would dispatch WM_CLOSE to a
        // nonexistent receiver — a no-op that just wastes the grace window.
        // Force-kill the JVM and its child processes directly. `start_service`
        // polls the app port before spawning, so any kernel-held socket from
        // the freshly killed JVM is waited out there, not here.
        let _ = crate::process::silent_cmd("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .output()
            .await;
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    // Remove PID file
    let _ = std::fs::remove_file(pid_path(config, name));
    tracing::info!("Stopped {}", name);
    Ok(())
}

/// Restart = stop + start.
pub async fn restart_service(name: &str, config: &NucleusConfig) -> Result<(), NucleusError> {
    // Stop if running (ignore errors — might not be running)
    let _ = stop_service(name, config).await;
    // Brief pause between stop and start
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    start_service(name, config).await
}

/// List all services with status (running/stopped), PID, port, health.
pub async fn get_services(config: &NucleusConfig) -> Result<Vec<ServiceInfo>, NucleusError> {
    // Fetch enabled modules from Firestore — only show configured services
    let enabled_services = if !config.hospital_code.is_empty() {
        match crate::firestore::FirestoreClient::new_from_config().await {
            Ok(client) => match client.fetch_modules(&config.hospital_code).await {
                Ok(modules) => modules.enabled_service_names(),
                Err(_) => releases::all_updatable_services(),
            },
            Err(_) => releases::all_updatable_services(),
        }
    } else {
        releases::all_updatable_services()
    };

    let mut services = Vec::new();

    // Collect probe targets for concurrent health checks
    let mut probe_targets: Vec<(String, u16)> = Vec::new();

    for svc_name in &enabled_services {
        let pid = read_pid(config, svc_name);
        let alive = pid.map(is_process_alive).unwrap_or(false);

        // puru-hydrogen is static files served by an external web server,
        // not a JAR process — judge it by deployment + whether port 80 answers
        let status = if svc_name == "puru-hydrogen" {
            let deployed = config.nginx_html_dir().join("index.html").exists();
            let served = std::net::TcpStream::connect_timeout(
                &std::net::SocketAddr::from(([127, 0, 0, 1], 80)),
                std::time::Duration::from_millis(500),
            )
            .is_ok();
            if deployed && served {
                ServiceStatus::Running
            } else if deployed {
                ServiceStatus::Stopped // deployed but no web server on :80
            } else {
                ServiceStatus::NotInstalled // enabled but not deployed
            }
        } else if alive {
            ServiceStatus::Running
        } else if jar_path(config, svc_name).exists() {
            ServiceStatus::Stopped
        } else {
            ServiceStatus::NotInstalled // enabled but JAR not pulled yet
        };

        let port = service_port(svc_name);

        // Health/readiness is probed on the management port if the service uses
        // a dedicated one, else the app port.
        if alive {
            if let Some(p) = health_probe_port(svc_name) {
                probe_targets.push((svc_name.clone(), p));
            }
        }

        let image = if let Ok(Some(meta)) = releases::read_local_jar_meta(svc_name) {
            let artifact = if meta.jar_file.is_empty() {
                "angular"
            } else {
                meta.jar_file.as_str()
            };
            format!("{}@{}", artifact, meta.short_sha)
        } else if svc_name == "puru-hydrogen" {
            "angular".to_string()
        } else {
            "not installed".to_string()
        };

        let port_str = port.map(|p| format!("{}:{}", p, p)).unwrap_or_default();

        services.push(ServiceInfo {
            name: svc_name.clone(),
            container_name: pid.map(|p| format!("PID:{}", p)).unwrap_or_default(),
            image,
            status,
            health: None,
            ports: vec![port_str],
            uptime: None, // Could be computed from PID start time
            health_response_ms: None,
            detail: None,
        });
    }

    // Run HTTP health/readiness probes concurrently, then derive the *displayed*
    // status from the verdict — not from PID liveness alone. A live JVM that
    // isn't ready (crash-looping, failing to start beans, dependency down) must
    // not read as a green "Running".
    if !probe_targets.is_empty() {
        let probe_results = probe_native_health(&probe_targets).await;
        for svc in &mut services {
            // Only services we actually probed (alive + known port). Leave
            // hydrogen and port-less services on their PID-based status.
            let Some(outcome) = probe_results.get(&svc.name) else { continue };
            if svc.status != ServiceStatus::Running {
                continue;
            }

            svc.health_response_ms = if outcome.response_ms > 0 {
                Some(outcome.response_ms)
            } else {
                None
            };

            // Still inside the startup grace window? (compute before mutating svc)
            let booting = read_pid(config, &svc.name)
                .and_then(process_uptime_secs)
                .map_or(false, |u| u < STARTUP_GRACE_SECS);
            let probe_port = health_probe_port(&svc.name).unwrap_or(0);

            match outcome.up {
                // Ready/UP — genuinely healthy.
                Some(true) => {
                    svc.health = Some(HealthStatus::Healthy);
                    svc.status = ServiceStatus::Running;
                    svc.detail = None;
                }
                // Answered but DOWN / not-ready — degraded. Within the startup
                // grace we call it Starting; past it, it's a real failure.
                Some(false) => {
                    svc.health = Some(HealthStatus::Unhealthy);
                    if booting {
                        svc.status = ServiceStatus::Starting;
                        svc.detail = Some("Started, not ready yet".into());
                    } else {
                        svc.status = ServiceStatus::Error;
                        svc.detail = Some(format!(
                            "Not ready — actuator on :{} reports DOWN. Check the service log.",
                            probe_port
                        ));
                    }
                }
                // No readable verdict — the actuator gave us nothing usable. This
                // means actuator is disabled/secured/absent (reachable) or the
                // management port isn't listening (unreachable). Surface it as an
                // error after the grace window instead of a misleading "Running".
                None => {
                    svc.health = None;
                    if booting {
                        svc.status = ServiceStatus::Starting;
                        svc.detail = Some("Starting — actuator not responding yet".into());
                    } else if outcome.reachable {
                        svc.status = ServiceStatus::Error;
                        svc.detail = Some(format!(
                            "Actuator not readable on :{} — enable and publicly expose \
                             /actuator/health/readiness (management endpoint appears disabled or secured).",
                            probe_port
                        ));
                    } else {
                        svc.status = ServiceStatus::Error;
                        svc.detail = Some(format!(
                            "Actuator unreachable on :{} — is the management endpoint enabled, \
                             or its port mapped in Nucleus?",
                            probe_port
                        ));
                    }
                }
            }
        }
    }

    Ok(services)
}

/// Read log file tail for a service.
pub async fn get_logs(
    name: &str,
    tail: u64,
    config: &NucleusConfig,
) -> Result<String, NucleusError> {
    let path = log_path(config, name);

    if !path.exists() {
        return Ok(format!("No log file found for {}", name));
    }

    // Read the file and return last N lines
    let content = tokio::fs::read_to_string(&path).await?;
    let lines: Vec<&str> = content.lines().collect();
    let start = if lines.len() > tail as usize {
        lines.len() - tail as usize
    } else {
        0
    };

    Ok(lines[start..].join("\n"))
}

/// Update a service: stop → pull new JAR → start.
/// Returns the pull result with new SHA info.
pub async fn update_service(name: &str, config: &NucleusConfig) -> Result<releases::JarPullResult, NucleusError> {
    tracing::info!("Updating {} ...", name);

    // Stop if running (ignore errors — might already be stopped)
    let was_running = read_pid(config, name)
        .map(is_process_alive)
        .unwrap_or(false);

    if was_running {
        stop_service(name, config).await?;
        // Brief pause to ensure port is released
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }

    // Pull new JAR (old one is backed up as .bak automatically)
    let pull_result = releases::pull_jar(name).await?;

    // Restart if it was running before
    if was_running {
        start_service(name, config).await?;
    }

    tracing::info!("Updated {} to build {}", name, pull_result.short_sha);
    Ok(pull_result)
}

/// Rollback a service to the previous JAR (.bak file).
pub async fn rollback_service(name: &str, config: &NucleusConfig) -> Result<(), NucleusError> {
    let jars_dir = config.jars_dir();
    let jar = jars_dir.join(format!("{}.jar", name));
    let bak = jars_dir.join(format!("{}.jar.bak", name));

    if !bak.exists() {
        return Err(NucleusError::NotFound(format!(
            "No backup JAR found for {}. Cannot rollback.",
            name
        )));
    }

    let was_running = read_pid(config, name)
        .map(is_process_alive)
        .unwrap_or(false);

    if was_running {
        stop_service(name, config).await?;
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }

    // Swap: current → .bad, .bak → current
    let bad = jars_dir.join(format!("{}.jar.bad", name));
    if jar.exists() {
        std::fs::rename(&jar, &bad)?;
    }
    std::fs::rename(&bak, &jar)?;
    // Remove the bad version
    let _ = std::fs::remove_file(&bad);

    if was_running {
        start_service(name, config).await?;
    }

    tracing::info!("Rolled back {} to previous JAR", name);
    Ok(())
}

/// Tear a service down: stop the process (if running) and delete its on-disk
/// artifacts (jar, backup jar, build meta, pid file). The .log file is kept for
/// audit. Used when a service has been de-selected in the cloud config.
pub(crate) async fn remove_service(config: &NucleusConfig, name: &str) -> Result<(), NucleusError> {
    // Stop first — ignore "not running" errors, we just want it down.
    let _ = stop_service(name, config).await;

    let jars = config.jars_dir();
    let targets = [
        jars.join(format!("{}.jar", name)),
        jars.join(format!("{}.jar.bak", name)),
        jars.join(format!("{}.jar.bad", name)),
        jars.join(format!("{}.meta.json", name)),
        pid_path(config, name),
    ];
    for f in targets {
        if f.exists() {
            if let Err(e) = std::fs::remove_file(&f) {
                tracing::warn!("Failed to remove {}: {}", f.display(), e);
            }
        }
    }

    tracing::info!("Removed native service {}", name);
    Ok(())
}

// ── Health probing ──────────────────────────────────────────────────────────

/// Outcome of probing a service's actuator endpoints.
struct ProbeOutcome {
    /// The port answered at all (the web server is up).
    reachable: bool,
    /// Verdict from the actuator status: `Some(true)` = UP/ready,
    /// `Some(false)` = answered but DOWN/not-ready, `None` = answered but the
    /// status couldn't be read (endpoint secured/absent) or no answer at all.
    up: Option<bool>,
    response_ms: u64,
}

/// Process uptime in seconds (epoch now − process start time), or None if the
/// process can't be found. Used to tell "still booting" from "stuck".
fn process_uptime_secs(pid: u32) -> Option<u64> {
    use sysinfo::{Pid, PidExt, ProcessExt, System, SystemExt};
    let mut sys = System::new();
    let spid = Pid::from_u32(pid);
    sys.refresh_process(spid);
    let proc_ = sys.process(spid)?;
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    Some(now.saturating_sub(proc_.start_time()))
}

/// GET a URL and interpret the actuator response into (reachable, up?, ms).
async fn probe_url(client: &reqwest::Client, url: &str) -> (bool, Option<bool>, u64) {
    use std::time::Instant;
    let start = Instant::now();
    match client.get(url).send().await {
        Ok(resp) => {
            let ms = start.elapsed().as_millis() as u64;
            let code = resp.status();
            let body = resp.text().await.unwrap_or_default();
            // Actuator JSON status is authoritative (a 503 with {"status":"DOWN"}
            // is a real verdict, as is {"status":"UP"}).
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(s) = json.get("status").and_then(|v| v.as_str()) {
                    return (true, Some(s == "UP"), ms);
                }
            }
            if code.is_success() {
                (true, Some(true), ms)
            } else if matches!(code.as_u16(), 401 | 403 | 404 | 405) {
                // Answered, but the endpoint is secured/absent — can't read a verdict.
                (true, None, ms)
            } else {
                (true, Some(false), ms)
            }
        }
        Err(_) => (false, None, 0),
    }
}

async fn probe_native_health(
    targets: &[(String, u16)],
) -> std::collections::HashMap<String, ProbeOutcome> {
    use futures_util::future::join_all;

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return std::collections::HashMap::new(),
    };

    let futures: Vec<_> = targets
        .iter()
        .map(|(name, port)| {
            let name = name.clone();
            let port = *port;
            let client = client.clone();
            async move {
                // Prefer readiness — it only reports UP once fully started.
                let ready_url = format!("http://127.0.0.1:{}{}", port, READINESS_ENDPOINT);
                let (r1, up1, ms1) = probe_url(&client, &ready_url).await;
                let outcome = if up1.is_some() {
                    ProbeOutcome { reachable: r1, up: up1, response_ms: ms1 }
                } else {
                    // Readiness missing/secured/unreachable → fall back to the
                    // aggregate health endpoint so services that haven't adopted
                    // readiness probes yet behave as before (no regression).
                    let health_url = format!("http://127.0.0.1:{}{}", port, HEALTH_ENDPOINT);
                    let (r2, up2, ms2) = probe_url(&client, &health_url).await;
                    ProbeOutcome {
                        reachable: r1 || r2,
                        up: up2,
                        response_ms: if r2 { ms2 } else { ms1 },
                    }
                };
                (name, outcome)
            }
        })
        .collect();

    join_all(futures).await.into_iter().collect()
}
