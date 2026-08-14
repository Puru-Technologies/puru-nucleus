//! Service management — dispatches between Docker and Native mode

pub mod native;

use bollard::container::{
    ListContainersOptions, RestartContainerOptions, StartContainerOptions, StopContainerOptions,
};
use bollard::Docker;
use serde::{Deserialize, Serialize};
use std::time::Instant;

use crate::config::DeploymentMode;

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub name: String,
    pub container_name: String,
    pub image: String,
    pub status: ServiceStatus,
    pub health: Option<HealthStatus>,
    pub ports: Vec<String>,
    pub uptime: Option<String>,
    pub health_response_ms: Option<u64>,
    /// Human-readable reason for a non-healthy state (e.g. "actuator not
    /// reachable on :9082 — is the management endpoint enabled?"). Shown in the
    /// UI so an operator sees *why* a service isn't green.
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ServiceStatus {
    Running,
    Stopped,
    Starting,
    /// Enabled for this hospital but its build is not present on disk yet —
    /// distinct from a crash. The watchdog must NOT try to restart these.
    NotInstalled,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Healthy,
    Unhealthy,
    Starting,
}

/// Detect existing Puru setup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectionResult {
    pub found: bool,
    pub containers: Vec<ContainerInfo>,
    pub compose_path: Option<String>,
    pub databases: Vec<DatabaseInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerInfo {
    pub name: String,
    pub image: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseInfo {
    pub name: String,
    pub size_mb: u64,
}

/// Check prerequisites
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrerequisiteStatus {
    pub name: String,
    pub installed: bool,
    pub version: Option<String>,
    pub required_version: Option<String>,
    /// Whether this prerequisite can be auto-installed (Windows only, MySQL/RabbitMQ)
    #[serde(default)]
    pub installable: bool,
}

// ── Service Registry ─────────────────────────────────────────────────────────

struct PuruServiceDef {
    name: &'static str,
    image_patterns: &'static [&'static str],
    container_patterns: &'static [&'static str],
    default_port: u16,
    health_endpoint: Option<&'static str>,
}

const PURU_SERVICE_REGISTRY: &[PuruServiceDef] = &[
    PuruServiceDef {
        name: "Xenon (Backend)",
        image_patterns: &["puru-xenon", "gcp-puru-auth"],
        container_patterns: &["backend", "puruboot", "im"],
        default_port: 8081,
        health_endpoint: Some("/actuator/health"),
    },
    PuruServiceDef {
        name: "HAS",
        image_patterns: &["puru-has"],
        container_patterns: &["has"],
        default_port: 8082,
        health_endpoint: Some("/actuator/health"),
    },
    PuruServiceDef {
        name: "PACS",
        image_patterns: &["puru-pacs", "gcp-puru-pacs"],
        container_patterns: &["pacs", "purupacs"],
        default_port: 8083,
        health_endpoint: Some("/actuator/health"),
    },
    PuruServiceDef {
        name: "Argon (Pathology)",
        image_patterns: &["puru-argon"],
        container_patterns: &["pathology"],
        default_port: 8084,
        health_endpoint: Some("/actuator/health"),
    },
    PuruServiceDef {
        name: "Comm",
        image_patterns: &["puru-comm", "gcp-puru-comm"],
        container_patterns: &["comm_server", "purucomm"],
        default_port: 8085,
        health_endpoint: Some("/actuator/health"),
    },
    PuruServiceDef {
        name: "Realtime",
        image_patterns: &["puru-realtime", "gcp-puru-realtime"],
        container_patterns: &["realtime", "pururt"],
        default_port: 8086,
        health_endpoint: Some("/actuator/health"),
    },
    PuruServiceDef {
        name: "Neon (Medical)",
        image_patterns: &["puru-neon"],
        container_patterns: &["medical"],
        default_port: 8087,
        health_endpoint: Some("/actuator/health"),
    },
    PuruServiceDef {
        name: "Bridge",
        image_patterns: &["puru-bridge"],
        container_patterns: &["bridge"],
        default_port: 8094,
        health_endpoint: Some("/actuator/health"),
    },
    PuruServiceDef {
        name: "Integration",
        image_patterns: &["puru-integration"],
        container_patterns: &["integration"],
        default_port: 8088,
        health_endpoint: Some("/actuator/health"),
    },
    PuruServiceDef {
        name: "Radon",
        image_patterns: &["puru-radon"],
        container_patterns: &["gh"],
        default_port: 8096,
        health_endpoint: Some("/actuator/health"),
    },
    PuruServiceDef {
        name: "Mercury (HRMS)",
        image_patterns: &["puru-mercury"],
        container_patterns: &["mercury", "hrms"],
        default_port: 8089,
        health_endpoint: Some("/actuator/health"),
    },
    PuruServiceDef {
        name: "Counter",
        image_patterns: &["puru-counter"],
        container_patterns: &["counter"],
        default_port: 8095,
        health_endpoint: Some("/actuator/health"),
    },
    PuruServiceDef {
        name: "Hydrogen (Frontend)",
        image_patterns: &["puru-hydrogen"],
        container_patterns: &["frontend", "ui", "puruui"],
        default_port: 80,
        health_endpoint: Some("/"),
    },
    PuruServiceDef {
        name: "MySQL",
        image_patterns: &["mysql:"],
        container_patterns: &["database", "purusql"],
        default_port: 3306,
        health_endpoint: None, // MySQL uses TCP probe, not HTTP
    },
    PuruServiceDef {
        name: "RabbitMQ",
        image_patterns: &["rabbitmq:"],
        container_patterns: &["rabbitmq", "pururmq"],
        default_port: 5672,
        health_endpoint: None, // RabbitMQ management on 15672, but not always available
    },
    PuruServiceDef {
        name: "Metabase",
        image_patterns: &["metabase/metabase"],
        container_patterns: &["metabase"],
        default_port: 3000,
        health_endpoint: Some("/api/health"),
    },
    PuruServiceDef {
        name: "File Server",
        image_patterns: &["nginx:"],
        container_patterns: &["file-server"],
        default_port: 81,
        health_endpoint: Some("/"),
    },
];

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Connect to Docker daemon and verify it's alive
async fn connect_docker() -> Result<Docker, crate::error::NucleusError> {
    let docker = Docker::connect_with_local_defaults()
        .map_err(|e| crate::error::NucleusError::Docker(e.to_string()))?;
    docker
        .ping()
        .await
        .map_err(|_| crate::error::NucleusError::DockerNotRunning)?;
    Ok(docker)
}

/// Match a container against the Puru service registry.
/// Tries image name first (most reliable across hospitals), then falls back to container name.
fn match_puru_service(image: &str, container_name: &str) -> Option<&'static PuruServiceDef> {
    let image_lower = image.to_lowercase();
    let name_lower = container_name.to_lowercase();

    // Image match first — most reliable
    for def in PURU_SERVICE_REGISTRY {
        for pattern in def.image_patterns {
            if image_lower.contains(pattern) {
                return Some(def);
            }
        }
    }

    // Container name fallback — exact match to avoid false positives
    for def in PURU_SERVICE_REGISTRY {
        for pattern in def.container_patterns {
            if name_lower == *pattern {
                return Some(def);
            }
        }
    }

    None
}

/// Extract uptime from Docker status string (e.g. "Up 2 days (healthy)" → "2 days")
fn extract_uptime(status: &str, state: &str) -> Option<String> {
    if state != "running" {
        return None;
    }
    if let Some(idx) = status.find("Up ") {
        let rest = &status[idx + 3..];
        let uptime = rest.split(" (").next().unwrap_or(rest);
        Some(uptime.trim().to_string())
    } else {
        None
    }
}

/// Map Docker container state to ServiceStatus enum
fn map_container_status(state: &str) -> ServiceStatus {
    match state {
        "running" => ServiceStatus::Running,
        "created" | "restarting" => ServiceStatus::Starting,
        "exited" | "dead" | "removing" => ServiceStatus::Stopped,
        _ => ServiceStatus::Error,
    }
}

/// Extract health status from Docker status string
fn map_health_status(status: &str, state: &str) -> Option<HealthStatus> {
    if state != "running" {
        return None;
    }
    if status.contains("(healthy)") {
        Some(HealthStatus::Healthy)
    } else if status.contains("(unhealthy)") {
        Some(HealthStatus::Unhealthy)
    } else if status.contains("(health: starting)") {
        Some(HealthStatus::Starting)
    } else {
        None // no health check configured
    }
}

/// Extract a version number from command output
fn extract_version(output: &str) -> Option<String> {
    for word in output.split_whitespace() {
        let clean = word
            .trim_start_matches('v')
            .trim_start_matches('V')
            .trim_end_matches(',');
        if clean
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
            && clean.contains('.')
        {
            let version = clean.split('-').next().unwrap_or(clean);
            return Some(version.to_string());
        }
    }
    None
}

/// Search common paths for docker-compose.yml
async fn find_compose_path() -> Option<String> {
    let mut candidates: Vec<std::path::PathBuf> = vec![];

    // Linux common paths
    candidates.push("/opt/puru/docker-compose.yml".into());
    candidates.push("/opt/puru/docker-compose.yaml".into());
    candidates.push("/home/puru/docker/docker-compose.yml".into());
    candidates.push("/home/puru/docker/docker-compose.yaml".into());
    candidates.push("/home/puru/docker-compose.yml".into());

    // Windows common paths
    candidates.push("C:\\puru\\docker-compose.yml".into());
    candidates.push("C:\\puru\\docker\\docker-compose.yml".into());
    candidates.push("C:\\Users\\puru\\docker\\docker-compose.yml".into());
    candidates.push("C:\\Users\\puru\\docker-compose.yml".into());

    if let Some(base_dirs) = directories::BaseDirs::new() {
        let home = base_dirs.home_dir();
        let puru_dir = home.join("puru");
        candidates.push(puru_dir.join("docker").join("docker-compose.yml"));
        candidates.push(puru_dir.join("docker").join("docker-compose.yaml"));
        candidates.push(puru_dir.join("docker-compose.yml"));
        candidates.push(puru_dir.join("docker-compose.yaml"));
        candidates.push(home.join("docker").join("docker-compose.yml"));
        candidates.push(home.join("docker").join("docker-compose.yaml"));
        candidates.push(home.join("docker-compose.yml"));
        candidates.push(home.join("docker-compose.yaml"));
    }

    // Also check config — user may have already set the path
    if let Ok(config) = crate::config::load_config() {
        if !config.docker_compose_path.is_empty() {
            let p = std::path::PathBuf::from(&config.docker_compose_path);
            if !candidates.contains(&p) {
                candidates.insert(0, p); // check configured path first
            }
        }
    }

    for path in &candidates {
        if tokio::fs::metadata(path).await.is_ok() {
            return Some(path.to_string_lossy().to_string());
        }
    }

    None
}

// ── HTTP Health Probes ──────────────────────────────────────────────────────

/// Probe a service's HTTP health endpoint.
/// Returns (HealthStatus, response_time_ms) or None if no endpoint configured.
async fn probe_http_health(
    port: u16,
    endpoint: &str,
) -> Option<(HealthStatus, u64)> {
    let url = format!("http://127.0.0.1:{}{}", port, endpoint);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .ok()?;

    let start = Instant::now();
    match client.get(&url).send().await {
        Ok(resp) => {
            let elapsed_ms = start.elapsed().as_millis() as u64;
            let status = resp.status();

            if status.is_success() {
                // For Spring Boot /actuator/health, check the body for "UP"
                if endpoint.contains("actuator/health") {
                    if let Ok(body) = resp.text().await {
                        if body.contains("\"UP\"") || body.contains("\"status\":\"UP\"") {
                            return Some((HealthStatus::Healthy, elapsed_ms));
                        }
                        // Service responded but reported DOWN
                        return Some((HealthStatus::Unhealthy, elapsed_ms));
                    }
                }
                Some((HealthStatus::Healthy, elapsed_ms))
            } else if status.as_u16() == 503 {
                Some((HealthStatus::Unhealthy, elapsed_ms))
            } else {
                // Any HTTP response means the service is at least responding
                Some((HealthStatus::Healthy, elapsed_ms))
            }
        }
        Err(_) => None, // Connection refused or timeout
    }
}

/// Run HTTP health probes for all running services concurrently.
/// Returns a map of container_name → (HealthStatus, response_ms).
async fn probe_all_health(
    services: &[(String, u16, Option<&'static str>)],
) -> std::collections::HashMap<String, (HealthStatus, u64)> {
    use futures_util::future::join_all;

    let futures: Vec<_> = services
        .iter()
        .filter_map(|(name, port, endpoint)| {
            let endpoint = (*endpoint)?;
            let name = name.clone();
            let port = *port;
            Some(async move {
                let result = probe_http_health(port, endpoint).await;
                (name, result)
            })
        })
        .collect();

    let results = join_all(futures).await;

    results
        .into_iter()
        .filter_map(|(name, result)| result.map(|r| (name, r)))
        .collect()
}

// ── Public API (mode dispatch) ───────────────────────────────────────────────

/// Get list of Puru services — dispatches based on deployment mode.
pub async fn get_services() -> Result<Vec<ServiceInfo>, crate::error::NucleusError> {
    let config = crate::config::load_config()?;
    match config.deployment_mode {
        DeploymentMode::Docker => get_docker_services().await,
        DeploymentMode::Native => native::get_services(&config).await,
    }
}

/// Start a service — dispatches based on deployment mode.
pub async fn start_service(name: &str) -> Result<(), crate::error::NucleusError> {
    // An explicit start clears any "manually stopped" marker.
    clear_manually_stopped(name);
    let config = crate::config::load_config()?;
    match config.deployment_mode {
        DeploymentMode::Docker => start_docker_service(name).await,
        DeploymentMode::Native => native::start_service(name, &config).await,
    }
}

/// Stop a service — dispatches based on deployment mode.
pub async fn stop_service(name: &str) -> Result<(), crate::error::NucleusError> {
    let config = crate::config::load_config()?;
    let result = match config.deployment_mode {
        DeploymentMode::Docker => stop_docker_service(name).await,
        DeploymentMode::Native => native::stop_service(name, &config).await,
    };
    // Record the operator's intent so the watchdog doesn't auto-restart it.
    if result.is_ok() {
        mark_manually_stopped(name);
    }
    result
}

/// Restart a service — dispatches based on deployment mode.
pub async fn restart_service(name: &str) -> Result<(), crate::error::NucleusError> {
    // A restart means the operator wants it running — clear any stop marker.
    clear_manually_stopped(name);
    let config = crate::config::load_config()?;
    match config.deployment_mode {
        DeploymentMode::Docker => restart_docker_service(name).await,
        DeploymentMode::Native => native::restart_service(name, &config).await,
    }
}

// ── Manual-stop marker ────────────────────────────────────────────────────────
// Services an operator intentionally stopped (from the UI/CLI/API). The daemon
// watchdog consults this set and will NOT auto-restart anything in it, so a
// manual stop actually stays stopped. The marker is keyed by the same
// identifier `stop_service`/`start_service` receive (native service name, or
// Docker container name). Persisted so it survives daemon/app restarts.

fn manual_stop_path() -> std::path::PathBuf {
    crate::config::config_dir().join("stopped-services.json")
}

/// Load the set of intentionally-stopped service identifiers.
pub fn load_manually_stopped() -> std::collections::HashSet<String> {
    match std::fs::read_to_string(manual_stop_path()) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => std::collections::HashSet::new(),
    }
}

fn save_manually_stopped(set: &std::collections::HashSet<String>) {
    let path = manual_stop_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string(set) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                tracing::warn!("Failed to persist manual-stop set: {}", e);
            }
        }
        Err(e) => tracing::warn!("Failed to serialize manual-stop set: {}", e),
    }
}

/// Mark a service as intentionally stopped.
pub fn mark_manually_stopped(name: &str) {
    let mut set = load_manually_stopped();
    if set.insert(name.to_string()) {
        save_manually_stopped(&set);
    }
}

/// Clear a service's manual-stop marker (called when it's (re)started).
pub fn clear_manually_stopped(name: &str) {
    let mut set = load_manually_stopped();
    if set.remove(name) {
        save_manually_stopped(&set);
    }
}

/// Whether a service was intentionally stopped by an operator.
pub fn is_manually_stopped(name: &str) -> bool {
    load_manually_stopped().contains(name)
}

/// Get logs — dispatches based on deployment mode.
pub async fn get_container_logs(
    container_name: &str,
    tail: u64,
    since: Option<i64>,
    until: Option<i64>,
) -> Result<String, crate::error::NucleusError> {
    let config = crate::config::load_config()?;
    match config.deployment_mode {
        DeploymentMode::Docker => get_docker_container_logs(container_name, tail, since, until).await,
        DeploymentMode::Native => native::get_logs(container_name, tail, &config).await,
    }
}

/// Update a native service (stop → pull new JAR → start).
/// Only available in native mode.
pub async fn update_native_service(name: &str) -> Result<crate::releases::JarPullResult, crate::error::NucleusError> {
    let config = crate::config::load_config()?;
    match config.deployment_mode {
        DeploymentMode::Native => native::update_service(name, &config).await,
        DeploymentMode::Docker => Err(crate::error::NucleusError::Validation(
            "update_native_service is only available in native deployment mode".into(),
        )),
    }
}

/// Rollback a native service to its previous JAR (.bak).
/// Only available in native mode.
pub async fn rollback_native_service(name: &str) -> Result<(), crate::error::NucleusError> {
    let config = crate::config::load_config()?;
    match config.deployment_mode {
        DeploymentMode::Native => native::rollback_service(name, &config).await,
        DeploymentMode::Docker => Err(crate::error::NucleusError::Validation(
            "rollback_native_service is only available in native deployment mode".into(),
        )),
    }
}

// ── Docker implementation ───────────────────────────────────────────────────

/// Get list of Puru services from Docker
async fn get_docker_services() -> Result<Vec<ServiceInfo>, crate::error::NucleusError> {
    // Gracefully handle Docker unavailable — return empty list, not an error
    let docker = match connect_docker().await {
        Ok(d) => d,
        Err(_) => return Ok(vec![]),
    };

    let containers = docker
        .list_containers(Some(ListContainersOptions::<String> {
            all: true,
            ..Default::default()
        }))
        .await
        .map_err(|e| crate::error::NucleusError::Docker(e.to_string()))?;

    let mut services: Vec<ServiceInfo> = Vec::new();
    // Collect running services for HTTP health probes
    let mut probe_targets: Vec<(String, u16, Option<&'static str>)> = Vec::new();

    for container in &containers {
        let image = container.image.as_deref().unwrap_or("");
        let raw_name = container
            .names
            .as_ref()
            .and_then(|n| n.first())
            .map(|n| n.trim_start_matches('/'))
            .unwrap_or("");

        let Some(def) = match_puru_service(image, raw_name) else {
            continue;
        };

        let state = container.state.as_deref().unwrap_or("");
        let status_str = container.status.as_deref().unwrap_or("");

        // Extract the host-facing port for health probes.
        // In bridge mode Docker maps public_port → private_port.
        // In host mode there are no port mappings — fall back to default_port.
        let mut host_port: u16 = def.default_port;
        let ports = {
            let mut result = Vec::new();
            if let Some(ports) = &container.ports {
                for p in ports {
                    if let Some(public) = p.public_port {
                        result.push(format!("{}:{}", public, p.private_port));
                        // Use the first public port that maps to the service's default port
                        if p.private_port == def.default_port {
                            host_port = public;
                        }
                    }
                }
            }
            result.sort();
            result.dedup();
            if result.is_empty() {
                result.push(format!(
                    "{}:{}",
                    def.default_port, def.default_port
                ));
            }
            result
        };

        // Queue running services with HTTP health endpoints for probing
        // Uses the actual host-facing port from Docker, not the hardcoded default
        if state == "running" && def.health_endpoint.is_some() {
            probe_targets.push((
                raw_name.to_string(),
                host_port,
                def.health_endpoint,
            ));
        }

        services.push(ServiceInfo {
            name: def.name.to_string(),
            container_name: raw_name.to_string(),
            image: image.to_string(),
            status: map_container_status(state),
            health: map_health_status(status_str, state),
            ports,
            uptime: extract_uptime(status_str, state),
            health_response_ms: None,
            detail: None,
        });
    }

    // Run HTTP health probes concurrently for all running services
    if !probe_targets.is_empty() {
        let health_results = probe_all_health(&probe_targets).await;

        for service in &mut services {
            if let Some((health, ms)) = health_results.get(&service.container_name) {
                // HTTP probe result overrides Docker health status
                service.health = Some(health.clone());
                service.health_response_ms = Some(*ms);
            }
        }
    }

    services.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(services)
}

/// Start a Docker container by name
async fn start_docker_service(name: &str) -> Result<(), crate::error::NucleusError> {
    let docker = connect_docker().await?;
    tracing::info!("Starting container: {}", name);
    docker
        .start_container(name, None::<StartContainerOptions<String>>)
        .await
        .map_err(|e| crate::error::NucleusError::Docker(e.to_string()))
}

/// Stop a Docker container by name (30s graceful timeout for Spring Boot services)
async fn stop_docker_service(name: &str) -> Result<(), crate::error::NucleusError> {
    let docker = connect_docker().await?;
    tracing::info!("Stopping container: {}", name);
    docker
        .stop_container(name, Some(StopContainerOptions { t: 30 }))
        .await
        .map_err(|e| crate::error::NucleusError::Docker(e.to_string()))
}

/// Restart a Docker container by name (30s graceful timeout)
async fn restart_docker_service(name: &str) -> Result<(), crate::error::NucleusError> {
    let docker = connect_docker().await?;
    tracing::info!("Restarting container: {}", name);
    docker
        .restart_container(name, Some(RestartContainerOptions { t: 30 }))
        .await
        .map_err(|e| crate::error::NucleusError::Docker(e.to_string()))
}

/// Get logs from a Docker container
async fn get_docker_container_logs(
    container_name: &str,
    tail: u64,
    since: Option<i64>,
    until: Option<i64>,
) -> Result<String, crate::error::NucleusError> {
    use bollard::container::LogsOptions;
    use futures_util::StreamExt;

    let docker = connect_docker().await?;

    let mut stream = docker.logs(
        container_name,
        Some(LogsOptions::<String> {
            stdout: true,
            stderr: true,
            tail: tail.to_string(),
            timestamps: true,
            since: since.unwrap_or(0),
            until: until.unwrap_or(0),
            ..Default::default()
        }),
    );

    let mut output = String::new();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(log) => output.push_str(&log.to_string()),
            Err(e) => {
                return Err(crate::error::NucleusError::Docker(format!(
                    "Failed to read logs: {}",
                    e
                )));
            }
        }
    }

    Ok(output)
}

/// Detect existing Puru setup on the host
pub async fn detect_existing_setup() -> Result<DetectionResult, crate::error::NucleusError> {
    // Gracefully handle Docker unavailable — return found: false, not an error
    let docker = match Docker::connect_with_local_defaults() {
        Ok(d) => d,
        Err(_) => {
            return Ok(DetectionResult {
                found: false,
                containers: vec![],
                compose_path: None,
                databases: vec![],
            });
        }
    };

    if docker.ping().await.is_err() {
        return Ok(DetectionResult {
            found: false,
            containers: vec![],
            compose_path: None,
            databases: vec![],
        });
    }

    let all_containers = docker
        .list_containers(Some(ListContainersOptions::<String> {
            all: true,
            ..Default::default()
        }))
        .await
        .map_err(|e| crate::error::NucleusError::Docker(e.to_string()))?;

    let mut puru_containers = Vec::new();
    let mut has_mysql = false;

    for container in &all_containers {
        let image = container.image.as_deref().unwrap_or("");
        let raw_name = container
            .names
            .as_ref()
            .and_then(|n| n.first())
            .map(|n| n.trim_start_matches('/'))
            .unwrap_or("");

        if match_puru_service(image, raw_name).is_some() {
            if image.contains("mysql:") {
                has_mysql = true;
            }
            puru_containers.push(ContainerInfo {
                name: raw_name.to_string(),
                image: image.to_string(),
                status: container
                    .status
                    .as_deref()
                    .unwrap_or("unknown")
                    .to_string(),
            });
        }
    }

    let compose_path = find_compose_path().await;

    // Infer known Puru databases if MySQL container is present
    let databases = if has_mysql {
        [
            "puru_im",
            "puru_has",
            "puru_med",
            "puru_dicom",
            "puru_path",
            "puru_bridge",
            "puru_auth",
            "puru_gh",
        ]
        .iter()
        .map(|name| DatabaseInfo {
            name: name.to_string(),
            size_mb: 0, // would need MySQL connection to get actual size
        })
        .collect()
    } else {
        vec![]
    };

    let found = !puru_containers.is_empty();

    Ok(DetectionResult {
        found,
        containers: puru_containers,
        compose_path,
        databases,
    })
}

/// Check all prerequisites (Docker, Docker Compose, MySQL, RabbitMQ)
pub async fn check_prerequisites() -> Result<Vec<PrerequisiteStatus>, crate::error::NucleusError> {
    let (docker, compose, mysql, rabbitmq) = tokio::join!(
        check_docker_prereq(),
        check_compose_prereq(),
        check_mysql_prereq(),
        check_rabbitmq_prereq(),
    );

    Ok(vec![docker, compose, mysql, rabbitmq])
}

async fn check_docker_prereq() -> PrerequisiteStatus {
    let docker = match Docker::connect_with_local_defaults() {
        Ok(d) => d,
        Err(_) => {
            return PrerequisiteStatus {
                name: "Docker".to_string(),
                installed: false,
                version: None,
                required_version: None,
                installable: false,
            };
        }
    };

    match docker.version().await {
        Ok(v) => PrerequisiteStatus {
            name: "Docker".to_string(),
            installed: true,
            version: v.version,
            required_version: None,
            installable: false,
        },
        Err(_) => PrerequisiteStatus {
            name: "Docker".to_string(),
            installed: false,
            version: None,
            required_version: None,
            installable: false,
        },
    }
}

async fn check_compose_prereq() -> PrerequisiteStatus {
    // Try V2 plugin first: docker compose version --short
    if let Ok(output) = crate::process::silent_cmd("docker")
        .args(["compose", "version", "--short"])
        .output()
        .await
    {
        if output.status.success() {
            let raw = String::from_utf8_lossy(&output.stdout);
            let version = extract_version(&raw).or_else(|| Some(raw.trim().to_string()));
            return PrerequisiteStatus {
                name: "Docker Compose".to_string(),
                installed: true,
                version,
                required_version: None,
                installable: false,
            };
        }
    }

    // Fallback to V1 standalone: docker-compose --version
    if let Ok(output) = crate::process::silent_cmd("docker-compose")
        .arg("--version")
        .output()
        .await
    {
        if output.status.success() {
            let raw = String::from_utf8_lossy(&output.stdout);
            return PrerequisiteStatus {
                name: "Docker Compose".to_string(),
                installed: true,
                version: extract_version(&raw),
                required_version: None,
                installable: false,
            };
        }
    }

    PrerequisiteStatus {
        name: "Docker Compose".to_string(),
        installed: false,
        version: None,
        required_version: None,
        installable: false,
    }
}

async fn check_mysql_prereq() -> PrerequisiteStatus {
    // Locate a host MySQL client and its version (PATH, then the standard Windows
    // install dirs). None if no host binary is present. The *client* existing does
    // NOT mean the server is usable — that's decided by a reachability probe below.
    let mut found_version: Option<String> = None;

    // 1. mysql on PATH
    if let Ok(output) = crate::process::silent_cmd("mysql")
        .arg("--version")
        .output()
        .await
    {
        if output.status.success() {
            found_version = extract_version(&String::from_utf8_lossy(&output.stdout));
        }
    }

    // 2. Windows: hardcoded fast-paths + a version-agnostic scan of the MySQL dir.
    #[cfg(target_os = "windows")]
    if found_version.is_none() {
        let mut candidates: Vec<String> = vec![
            r"C:\Program Files\MySQL\MySQL Server 8.0\bin\mysql.exe".to_string(),
            r"C:\Program Files\MySQL\MySQL Server 8.4\bin\mysql.exe".to_string(),
            r"C:\Program Files\MySQL\MySQL Server 9.0\bin\mysql.exe".to_string(),
        ];
        if let Ok(entries) = std::fs::read_dir(r"C:\Program Files\MySQL") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("MySQL Server") {
                    candidates.push(format!(r"C:\Program Files\MySQL\{}\bin\mysql.exe", name));
                }
            }
        }
        for path in candidates {
            if std::path::Path::new(&path).exists() {
                if let Ok(output) = crate::process::silent_cmd(&path)
                    .arg("--version")
                    .output()
                    .await
                {
                    if output.status.success() {
                        found_version = extract_version(&String::from_utf8_lossy(&output.stdout));
                        break;
                    }
                }
            }
        }
    }

    // A host MySQL is present. Its readiness depends on the *server* running, not
    // just the client binary — probe 3306. Reachable → installed. Not reachable →
    // "present but not running", reported as installable so the install/setup step
    // will initialize + start it (rather than passing the prereq gate and then
    // failing at the database-creation step).
    if found_version.is_some() {
        let reachable = std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], 3306)),
            std::time::Duration::from_millis(800),
        )
        .is_ok();
        return PrerequisiteStatus {
            name: "MySQL".to_string(),
            installed: reachable,
            version: found_version,
            required_version: None,
            installable: !reachable,
        };
    }

    // 3. Fallback: a MySQL Docker container (its own deployment — container
    // presence counts as installed).
    if let Ok(docker) = Docker::connect_with_local_defaults() {
        if let Ok(containers) = docker
            .list_containers(Some(ListContainersOptions::<String> {
                all: true,
                ..Default::default()
            }))
            .await
        {
            for container in &containers {
                let image = container.image.as_deref().unwrap_or("");
                if image.contains("mysql:") {
                    let version = image
                        .rsplit(':')
                        .next()
                        .filter(|v| {
                            v.chars()
                                .next()
                                .map(|c| c.is_ascii_digit())
                                .unwrap_or(false)
                        })
                        .map(|v| v.to_string());
                    return PrerequisiteStatus {
                        name: "MySQL".to_string(),
                        installed: true,
                        version,
                        required_version: None,
                        installable: false,
                    };
                }
            }
        }
    }

    // 4. Not found at all.
    PrerequisiteStatus {
        name: "MySQL".to_string(),
        installed: false,
        version: None,
        required_version: None,
        installable: true,
    }
}

/// Resolve the full path to the `mysql` binary.
/// Checks PATH first, then common Windows install locations.
pub(crate) async fn resolve_mysql_bin() -> Option<String> {
    resolve_mysql_tool("mysql").await
}

/// Resolve a MySQL client tool (`mysql`, `mysqldump`, `mysqlbinlog`, …) to a
/// runnable path. Tries PATH first, then the standard Windows MySQL install
/// dirs — those `bin\` folders are NOT on PATH by default, which is why a bare
/// `mysqldump` lookup fails even when MySQL Server is installed.
pub(crate) async fn resolve_mysql_tool(tool: &str) -> Option<String> {
    // 1. Try the tool on PATH
    if let Ok(output) = crate::process::silent_cmd(tool)
        .arg("--version")
        .output()
        .await
    {
        if output.status.success() {
            return Some(tool.to_string());
        }
    }

    // 2. Windows: check common install paths
    #[cfg(target_os = "windows")]
    {
        let exe = format!("{}.exe", tool);
        let search_paths = [
            format!(r"C:\Program Files\MySQL\MySQL Server 8.0\bin\{}", exe),
            format!(r"C:\Program Files\MySQL\MySQL Server 8.4\bin\{}", exe),
            format!(r"C:\Program Files\MySQL\MySQL Server 9.0\bin\{}", exe),
        ];
        for path in &search_paths {
            if std::path::Path::new(path).exists() {
                return Some(path.clone());
            }
        }
        // Scan for any MySQL Server directory
        if let Ok(entries) = std::fs::read_dir(r"C:\Program Files\MySQL") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("MySQL Server") {
                    let bin = format!(r"C:\Program Files\MySQL\{}\bin\{}", name, exe);
                    if std::path::Path::new(&bin).exists() {
                        return Some(bin);
                    }
                }
            }
        }
    }

    None
}

async fn check_rabbitmq_prereq() -> PrerequisiteStatus {
    // 1. Try rabbitmqctl on PATH
    if let Ok(output) = crate::process::silent_cmd("rabbitmqctl")
        .arg("version")
        .output()
        .await
    {
        if output.status.success() {
            let raw = String::from_utf8_lossy(&output.stdout);
            return PrerequisiteStatus {
                name: "RabbitMQ".to_string(),
                installed: true,
                version: extract_version(&raw),
                required_version: None,
                installable: false,
            };
        }
    }

    // 2. Windows: check common install path (rabbitmqctl is often not on PATH)
    #[cfg(target_os = "windows")]
    {
        let rabbitmq_base = r"C:\Program Files\RabbitMQ Server";
        if let Ok(entries) = std::fs::read_dir(rabbitmq_base) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("rabbitmq_server-") {
                    let ctl = format!(r"{}\{}\sbin\rabbitmqctl.bat", rabbitmq_base, name);
                    if let Ok(output) = crate::process::silent_cmd(&ctl)
                        .arg("version")
                        .output()
                        .await
                    {
                        if output.status.success() {
                            let raw = String::from_utf8_lossy(&output.stdout);
                            return PrerequisiteStatus {
                                name: "RabbitMQ".to_string(),
                                installed: true,
                                version: extract_version(&raw),
                                required_version: None,
                                installable: false,
                            };
                        }
                    }
                    // Binaries present but rabbitmqctl couldn't reach the node —
                    // present but not running. Report installable so setup starts +
                    // configures it, rather than passing the gate and failing later.
                    let version = name.strip_prefix("rabbitmq_server-").map(|v| v.to_string());
                    return PrerequisiteStatus {
                        name: "RabbitMQ".to_string(),
                        installed: false,
                        version,
                        required_version: None,
                        installable: true,
                    };
                }
            }
        }
    }

    // 3. Check management API (works regardless of PATH — just needs RabbitMQ running)
    if let Ok(resp) = reqwest::Client::new()
        .get("http://localhost:15672/api/overview")
        .basic_auth("guest", Some("guest"))
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
    {
        if resp.status().is_success() {
            // Try to extract version from the API response
            let version = resp
                .json::<serde_json::Value>()
                .await
                .ok()
                .and_then(|v| v.get("rabbitmq_version")?.as_str().map(|s| s.to_string()));
            return PrerequisiteStatus {
                name: "RabbitMQ".to_string(),
                installed: true,
                version,
                required_version: None,
                installable: false,
            };
        }
    }

    // 4. Fallback: detect RabbitMQ Docker container
    if let Ok(docker) = Docker::connect_with_local_defaults() {
        if let Ok(containers) = docker
            .list_containers(Some(ListContainersOptions::<String> {
                all: true,
                ..Default::default()
            }))
            .await
        {
            for container in &containers {
                let image = container.image.as_deref().unwrap_or("");
                if image.contains("rabbitmq:") {
                    let version = image.rsplit(':').next().and_then(|tag| {
                        let v = tag.split('-').next().unwrap_or(tag);
                        if v.chars()
                            .next()
                            .map(|c| c.is_ascii_digit())
                            .unwrap_or(false)
                        {
                            Some(v.to_string())
                        } else {
                            None
                        }
                    });
                    return PrerequisiteStatus {
                        name: "RabbitMQ".to_string(),
                        installed: true,
                        version,
                        required_version: None,
                        installable: false,
                    };
                }
            }
        }
    }

    PrerequisiteStatus {
        name: "RabbitMQ".to_string(),
        installed: false,
        version: None,
        required_version: None,
        installable: true,
    }
}
