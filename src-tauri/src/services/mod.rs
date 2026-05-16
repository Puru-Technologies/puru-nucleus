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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ServiceStatus {
    Running,
    Stopped,
    Starting,
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
    let config = crate::config::load_config()?;
    match config.deployment_mode {
        DeploymentMode::Docker => start_docker_service(name).await,
        DeploymentMode::Native => native::start_service(name, &config).await,
    }
}

/// Stop a service — dispatches based on deployment mode.
pub async fn stop_service(name: &str) -> Result<(), crate::error::NucleusError> {
    let config = crate::config::load_config()?;
    match config.deployment_mode {
        DeploymentMode::Docker => stop_docker_service(name).await,
        DeploymentMode::Native => native::stop_service(name, &config).await,
    }
}

/// Restart a service — dispatches based on deployment mode.
pub async fn restart_service(name: &str) -> Result<(), crate::error::NucleusError> {
    let config = crate::config::load_config()?;
    match config.deployment_mode {
        DeploymentMode::Docker => restart_docker_service(name).await,
        DeploymentMode::Native => native::restart_service(name, &config).await,
    }
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
        required_version: Some("2.0.0".to_string()),
        installable: false,
    }
}

async fn check_mysql_prereq() -> PrerequisiteStatus {
    // Try host-installed MySQL
    if let Ok(output) = crate::process::silent_cmd("mysql")
        .arg("--version")
        .output()
        .await
    {
        if output.status.success() {
            let raw = String::from_utf8_lossy(&output.stdout);
            return PrerequisiteStatus {
                name: "MySQL".to_string(),
                installed: true,
                version: extract_version(&raw),
                required_version: None,
                installable: false,
            };
        }
    }

    // Fallback: detect MySQL Docker container
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

    PrerequisiteStatus {
        name: "MySQL".to_string(),
        installed: false,
        version: None,
        required_version: Some("8.0.0".to_string()),
        installable: true,
    }
}

async fn check_rabbitmq_prereq() -> PrerequisiteStatus {
    // Try host-installed RabbitMQ
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

    // Fallback: detect RabbitMQ Docker container
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
                        // Handle tags like "3.12.8-management"
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
