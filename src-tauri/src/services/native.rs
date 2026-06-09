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
    ("puru-auth", 8095),
    ("puru-mercury", 8096),
    ("puru-integration", 8088),
];

/// Health endpoint for Spring Boot services
const HEALTH_ENDPOINT: &str = "/actuator/health";

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
        // Use tasklist to check if PID exists — no extra dependencies needed
        std::process::Command::new("tasklist")
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
fn load_env_files(config: &NucleusConfig, _service: &str) -> std::collections::HashMap<String, String> {
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

    vars
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Start a service as a java -jar process.
pub async fn start_service(name: &str, config: &NucleusConfig) -> Result<(), NucleusError> {
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

    // Open log file (append mode)
    let log_file_path = log_path(config, name);
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file_path)?;
    let stderr_file = log_file.try_clone()?;

    // Load env vars
    let env_vars = load_env_files(config, name);

    // Spawn process
    let child = crate::process::silent_cmd(&java_bin.to_string_lossy())
        .args(["-jar", &jar.to_string_lossy()])
        .envs(env_vars)
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
        // On Windows, use taskkill /PID which sends WM_CLOSE first (graceful)
        let _ = crate::process::silent_cmd("taskkill")
            .args(["/PID", &pid.to_string()])
            .output()
            .await;

        // Wait up to 30 seconds for graceful shutdown
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(30);
        loop {
            if !is_process_alive(pid) {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                tracing::warn!("{} did not stop within 30s, force killing", name);
                // Force kill with /F
                let _ = crate::process::silent_cmd("taskkill")
                    .args(["/F", "/PID", &pid.to_string()])
                    .output()
                    .await;
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
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
        let jar_exists = jar_path(config, svc_name).exists();

        let status = if alive {
            ServiceStatus::Running
        } else if jar_exists {
            ServiceStatus::Stopped
        } else {
            ServiceStatus::Error // Enabled but not installed
        };

        let port = service_port(svc_name);

        if alive {
            if let Some(p) = port {
                probe_targets.push((svc_name.clone(), p));
            }
        }

        let image = if let Ok(Some(meta)) = releases::read_local_jar_meta(svc_name) {
            format!("{}@{}", meta.jar_file, meta.short_sha)
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
        });
    }

    // Run HTTP health probes concurrently for running services
    if !probe_targets.is_empty() {
        let health_results = probe_native_health(&probe_targets).await;
        for svc in &mut services {
            if let Some((health, ms)) = health_results.get(&svc.name) {
                svc.health = Some(health.clone());
                svc.health_response_ms = Some(*ms);
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

// ── Health probing ──────────────────────────────────────────────────────────

async fn probe_native_health(
    targets: &[(String, u16)],
) -> std::collections::HashMap<String, (HealthStatus, u64)> {
    use futures_util::future::join_all;
    use std::time::Instant;

    let futures: Vec<_> = targets
        .iter()
        .map(|(name, port)| {
            let name = name.clone();
            let port = *port;
            async move {
                let url = format!("http://127.0.0.1:{}{}", port, HEALTH_ENDPOINT);
                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(5))
                    .build()
                    .ok();

                let client = match client {
                    Some(c) => c,
                    None => return (name, None),
                };

                let start = Instant::now();
                match client.get(&url).send().await {
                    Ok(resp) => {
                        let elapsed_ms = start.elapsed().as_millis() as u64;
                        if resp.status().is_success() {
                            if let Ok(body) = resp.text().await {
                                // Parse JSON properly: look for {"status":"UP"}
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                                    if json.get("status").and_then(|v| v.as_str()) == Some("UP") {
                                        return (name, Some((HealthStatus::Healthy, elapsed_ms)));
                                    }
                                    return (name, Some((HealthStatus::Unhealthy, elapsed_ms)));
                                }
                                // Fallback for non-JSON 200 responses
                                return (name, Some((HealthStatus::Healthy, elapsed_ms)));
                            }
                        }
                        (name, Some((HealthStatus::Unhealthy, elapsed_ms)))
                    }
                    Err(_) => (name, None),
                }
            }
        })
        .collect();

    let results = join_all(futures).await;
    results
        .into_iter()
        .filter_map(|(name, result)| result.map(|r| (name, r)))
        .collect()
}
