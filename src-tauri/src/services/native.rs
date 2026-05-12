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
    #[cfg(not(unix))]
    {
        // Use sysinfo to check process existence on Windows
        use sysinfo::{Pid, System};
        let mut sys = System::new();
        sys.refresh_process(Pid::from(pid as usize));
        sys.process(Pid::from(pid as usize)).is_some()
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
    let java_bin = jre_dir.join("bin").join("java");

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
    let child = tokio::process::Command::new(&java_bin)
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

    // Send SIGTERM
    #[cfg(unix)]
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
            #[cfg(unix)]
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
            // Wait a moment for SIGKILL to take effect
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            break;
        }
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
    let all_services = releases::all_updatable_services();
    let mut services = Vec::new();

    // Collect probe targets for concurrent health checks
    let mut probe_targets: Vec<(String, u16)> = Vec::new();

    for svc_name in &all_services {
        let pid = read_pid(config, svc_name);
        let alive = pid.map(is_process_alive).unwrap_or(false);
        let jar_exists = jar_path(config, svc_name).exists();

        let status = if alive {
            ServiceStatus::Running
        } else if jar_exists {
            ServiceStatus::Stopped
        } else {
            ServiceStatus::Error // JAR not installed
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
