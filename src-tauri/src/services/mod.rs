//! Service management — dispatches between Docker and Native mode

pub mod jar_manifest;
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
    /// True for infrastructure rows (MySQL) — the UI shows these as
    /// read-only status with infra-specific controls, not JAR update actions.
    #[serde(default)]
    pub infra: bool,
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
    /// Whether this prerequisite can be auto-installed (Windows only, MySQL)
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
    // Must stay after Xenon: "gcp-puru-auth" also contains "puru-auth".
    PuruServiceDef {
        name: "Auth",
        image_patterns: &["puru-auth"],
        container_patterns: &["auth"],
        default_port: 8080,
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
        name: "DViewer",
        image_patterns: &["/dviewer:"],
        container_patterns: &["dviewer"],
        default_port: 3000,
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

/// Spring Boot services run actuator on a separate management port, app port
/// + 1000 (`management.server.port`: auth 9080, has 9082, …), the same
/// convention as native mode's SERVICE_MGMT_PORTS. The app port has no
/// /actuator, so probing it only made every service log a "No static resource"
/// error, and the 500 that came back was counted as healthy.
const MGMT_PORT_OFFSET: u16 = 1000;

/// One running container to probe.
struct ProbeTarget {
    container: String,
    /// Port the HTTP probe goes to (the management port for actuator); None
    /// when the management port isn't published (bridge mode).
    port: Option<u16>,
    endpoint: &'static str,
    /// App port: when the management port doesn't answer, a TCP connect here
    /// tells "running, health unknown" from "not answering at all".
    app_port: u16,
}

/// Result of one probe. `health: None` means the service answered but gave no
/// verdict; `detail` says why, for the services view.
struct ProbeOutcome {
    health: Option<HealthStatus>,
    ms: u64,
    detail: Option<String>,
}

/// Probe a service's HTTP health endpoint. None when nothing answers.
async fn probe_http_health(port: u16, endpoint: &str) -> Option<ProbeOutcome> {
    let url = format!("http://127.0.0.1:{}{}", port, endpoint);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .ok()?;

    let start = Instant::now();
    let resp = client.get(&url).send().await.ok()?;
    let ms = start.elapsed().as_millis() as u64;
    let code = resp.status();

    if !endpoint.contains("actuator/health") {
        // Web servers (hydrogen, dviewer, …): any answer but 503 means it serves.
        let health = if code.as_u16() == 503 { HealthStatus::Unhealthy } else { HealthStatus::Healthy };
        return Some(ProbeOutcome { health: Some(health), ms, detail: None });
    }

    // Actuator: its JSON status is the verdict (a 503 with {"status":"DOWN"} is real).
    let body = resp.text().await.unwrap_or_default();
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
        if let Some(status) = json.get("status").and_then(|v| v.as_str()) {
            return Some(if status == "UP" {
                ProbeOutcome { health: Some(HealthStatus::Healthy), ms, detail: None }
            } else {
                ProbeOutcome {
                    health: Some(HealthStatus::Unhealthy),
                    ms,
                    detail: Some(format!("Actuator on :{} reports {}. Check the service log.", port, status)),
                }
            });
        }
    }
    Some(match code.as_u16() {
        200..=299 => ProbeOutcome { health: Some(HealthStatus::Healthy), ms, detail: None },
        401 | 403 | 404 | 405 => ProbeOutcome {
            health: None,
            ms,
            detail: Some(format!("Actuator on :{} answers {}: health unknown.", port, code.as_u16())),
        },
        other => ProbeOutcome {
            health: Some(HealthStatus::Unhealthy),
            ms,
            detail: Some(format!("Actuator on :{} answered {}.", port, other)),
        },
    })
}

/// Probe one target; for actuator, fall back to a TCP check of the app port.
async fn probe_target(target: &ProbeTarget) -> Option<ProbeOutcome> {
    if let Some(port) = target.port {
        if let Some(outcome) = probe_http_health(port, target.endpoint).await {
            return Some(outcome);
        }
    }
    if !target.endpoint.contains("actuator/health") {
        return None;
    }
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], target.app_port));
    let start = Instant::now();
    let open = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::net::TcpStream::connect(addr),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false);
    if open {
        Some(ProbeOutcome {
            health: None,
            ms: start.elapsed().as_millis() as u64,
            detail: Some(match target.port {
                Some(port) => format!(
                    "Running on :{}, but its actuator on :{} doesn't answer: health unknown.",
                    target.app_port, port
                ),
                None => format!(
                    "Running on :{}; its actuator port isn't published: health unknown.",
                    target.app_port
                ),
            }),
        })
    } else {
        // Nothing listening yet: keep Docker's own status (starting, or crashed).
        None
    }
}

/// Run health probes for all running services concurrently, keyed by container name.
async fn probe_all_health(targets: &[ProbeTarget]) -> std::collections::HashMap<String, ProbeOutcome> {
    use futures_util::future::join_all;

    let results = join_all(targets.iter().map(|t| async move { (t.container.clone(), probe_target(t).await) })).await;
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
        DeploymentMode::Docker => {
            let mut services = get_docker_services().await?;
            // MySQL on the host (the usual Linux layout) has no container, so it
            // would otherwise be missing from status, the GUI and the watchdog.
            if !services.iter().any(|s| s.name == "MySQL") {
                services.insert(0, crate::infra::database_row(&config));
            }
            Ok(services)
        }
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
    update_native_service_progress(name, |_, _, _| {}).await
}

/// Like [`update_native_service`], but reports progress via `on(phase,
/// downloaded, total)` for the UI progress bar.
pub async fn update_native_service_progress<F>(
    name: &str,
    on: F,
) -> Result<crate::releases::JarPullResult, crate::error::NucleusError>
where
    F: Fn(&str, u64, u64) + Send + Sync,
{
    let config = crate::config::load_config()?;
    match config.deployment_mode {
        DeploymentMode::Native => native::update_service_progress(name, &config, on).await,
        DeploymentMode::Docker => Err(crate::error::NucleusError::Validation(
            "update_native_service is only available in native deployment mode".into(),
        )),
    }
}

/// Apply a previously-staged (downloaded) update for a native service:
/// stop → swap the staged JAR in → restart. Reports phase progress via `on`.
pub async fn apply_native_update_progress<F>(
    name: &str,
    on: F,
) -> Result<crate::releases::JarPullResult, crate::error::NucleusError>
where
    F: Fn(&str, u64, u64) + Send + Sync,
{
    let config = crate::config::load_config()?;
    match config.deployment_mode {
        DeploymentMode::Native => native::apply_update_progress(name, &config, on).await,
        DeploymentMode::Docker => Err(crate::error::NucleusError::Validation(
            "Staged updates are only available in native deployment mode".into(),
        )),
    }
}

/// Rollback a native service one step back in its manifest history (the
/// most-recent previous JAR). Only available in native mode.
pub async fn rollback_native_service(name: &str) -> Result<(), crate::error::NucleusError> {
    let config = crate::config::load_config()?;
    match config.deployment_mode {
        DeploymentMode::Native => native::rollback_service(name, &config).await,
        DeploymentMode::Docker => Err(crate::error::NucleusError::Validation(
            "rollback_native_service is only available in native deployment mode".into(),
        )),
    }
}

/// Rollback a native service to a specific historical JAR file (must appear
/// in `manifest.history`). Only available in native mode.
pub async fn rollback_native_service_to(
    name: &str,
    file: &str,
) -> Result<(), crate::error::NucleusError> {
    let config = crate::config::load_config()?;
    match config.deployment_mode {
        DeploymentMode::Native => native::rollback_service_to(name, file, &config).await,
        DeploymentMode::Docker => Err(crate::error::NucleusError::Validation(
            "rollback_native_service_to is only available in native deployment mode".into(),
        )),
    }
}

// ── Docker implementation ───────────────────────────────────────────────────

/// Wait for the Docker daemon to become reachable, then ensure every Puru
/// container that isn't running gets a `docker start`. Belt-and-suspenders on
/// top of the compose file's `restart: always`: covers containers that exited
/// zero (which `always` respects but `unless-stopped` doesn't restart), a
/// compose accidentally set to `restart: no`, or a container added while
/// dockerd was down. Called by the daemon on boot in Docker mode. Never errs
/// out — every step logs and continues so a partial failure still starts what
/// it can.
pub async fn docker_boot_start() {
    use bollard::container::ListContainersOptions;

    // 1. Wait for Docker daemon. Docker Desktop can take 30–60 s to come up
    //    after Windows boot, and the daemon may start before it — poll for up
    //    to ~3 minutes so we don't give up too early.
    let docker = {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
        #[allow(unused_assignments)]
        let mut last_err: Option<String> = None;
        loop {
            match Docker::connect_with_local_defaults() {
                Ok(d) => match d.ping().await {
                    Ok(_) => break d,
                    Err(e) => last_err = Some(e.to_string()),
                },
                Err(e) => last_err = Some(e.to_string()),
            }
            if std::time::Instant::now() >= deadline {
                tracing::error!(
                    "Daemon (docker): Docker daemon not reachable after 180s ({}). Aborting boot start — the watchdog will retry.",
                    last_err.unwrap_or_default()
                );
                return;
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    };
    tracing::info!("Daemon (docker): Docker daemon reachable");

    // 2. List all containers (running + stopped) and filter to Puru services.
    let containers = match docker
        .list_containers(Some(ListContainersOptions::<String> {
            all: true,
            ..Default::default()
        }))
        .await
    {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Daemon (docker): could not list containers: {}", e);
            return;
        }
    };

    let mut started = 0usize;
    let mut already = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for container in &containers {
        let image = container.image.as_deref().unwrap_or("");
        let raw_name = container
            .names
            .as_ref()
            .and_then(|n| n.first())
            .map(|n| n.trim_start_matches('/'))
            .unwrap_or("");

        // Only touch containers this deployment owns.
        if match_puru_service(image, raw_name).is_none() {
            continue;
        }

        // Respect operator intent — services explicitly stopped from the UI/CLI
        // must stay stopped, matching the watchdog's rule. `state == "running"`
        // = leave it. Anything else (`created`, `exited`, `dead`, `paused`,
        // `restarting`) → try to start.
        let state = container.state.as_deref().unwrap_or("");
        if state == "running" {
            already += 1;
            continue;
        }
        if is_manually_stopped(raw_name) {
            skipped += 1;
            continue;
        }

        match start_docker_service(raw_name).await {
            Ok(()) => {
                tracing::info!("Daemon (docker): started container {} (was '{}')", raw_name, state);
                started += 1;
            }
            Err(e) => {
                tracing::warn!("Daemon (docker): failed to start {}: {}", raw_name, e);
                failed += 1;
            }
        }
    }

    tracing::info!(
        "Daemon (docker): boot start complete — {} started, {} already running, {} skipped (manual stop), {} failed",
        started, already, skipped, failed
    );
}

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
    let mut probe_targets: Vec<ProbeTarget> = Vec::new();

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
        let default_mgmt = def.default_port.saturating_add(MGMT_PORT_OFFSET);
        // Host mode: the management port is on the host as is. Bridge mode: only
        // if it is published; otherwise the probe falls back to the app port.
        let mut mgmt_port: Option<u16> = None;
        let mut has_mappings = false;
        let ports = {
            let mut result = Vec::new();
            if let Some(ports) = &container.ports {
                for p in ports {
                    if let Some(public) = p.public_port {
                        has_mappings = true;
                        result.push(format!("{}:{}", public, p.private_port));
                        // Use the first public port that maps to the service's default port
                        if p.private_port == def.default_port {
                            host_port = public;
                        }
                        if p.private_port == default_mgmt {
                            mgmt_port = Some(public);
                        }
                    }
                }
            }
            if !has_mappings {
                mgmt_port = Some(default_mgmt);
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
        if state == "running" {
            if let Some(endpoint) = def.health_endpoint {
                let port = if endpoint.contains("actuator/health") { mgmt_port } else { Some(host_port) };
                probe_targets.push(ProbeTarget {
                    container: raw_name.to_string(),
                    port,
                    endpoint,
                    app_port: host_port,
                });
            }
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
            infra: false,
        });
    }

    // Run HTTP health probes concurrently for all running services
    if !probe_targets.is_empty() {
        let health_results = probe_all_health(&probe_targets).await;

        for service in &mut services {
            if let Some(outcome) = health_results.get(&service.container_name) {
                // HTTP probe result overrides Docker health status
                service.health = outcome.health.clone();
                service.health_response_ms = Some(outcome.ms);
                service.detail = outcome.detail.clone();
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

    for container in &all_containers {
        let image = container.image.as_deref().unwrap_or("");
        let raw_name = container
            .names
            .as_ref()
            .and_then(|n| n.first())
            .map(|n| n.trim_start_matches('/'))
            .unwrap_or("");

        if match_puru_service(image, raw_name).is_some() {
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

    // Real databases only — the old hard-coded list showed puru_* names that
    // didn't exist. Needs MySQL credentials in config; otherwise empty.
    let databases = list_puru_databases().await;

    let found = !puru_containers.is_empty();

    Ok(DetectionResult {
        found,
        containers: puru_containers,
        compose_path,
        databases,
    })
}

/// `puru%` schemas on the configured MySQL with their sizes. Empty when no
/// credentials are configured or the server can't be reached.
async fn list_puru_databases() -> Vec<DatabaseInfo> {
    use mysql_async::prelude::*;

    let Ok(cfg) = crate::config::load_config() else { return vec![] };
    if cfg.mysql_password.is_empty() {
        return vec![];
    }
    let opts = mysql_async::OptsBuilder::default()
        .ip_or_hostname(cfg.mysql_host.clone())
        .tcp_port(cfg.mysql_port)
        .user(Some(cfg.mysql_user.clone()))
        .pass(Some(cfg.mysql_password.clone()));
    let pool = mysql_async::Pool::new(opts);
    let rows: Vec<(String, u64)> = match pool.get_conn().await {
        Ok(mut conn) => conn
            .query(
                "SELECT s.schema_name, CAST(COALESCE(SUM(t.data_length + t.index_length), 0) / 1048576 AS UNSIGNED) \
                 FROM information_schema.schemata s \
                 LEFT JOIN information_schema.tables t ON t.table_schema = s.schema_name \
                 WHERE s.schema_name LIKE 'puru%' GROUP BY s.schema_name ORDER BY s.schema_name",
            )
            .await
            .unwrap_or_default(),
        Err(_) => vec![],
    };
    let _ = pool.disconnect().await;
    rows.into_iter()
        .map(|(name, size_mb)| DatabaseInfo { name, size_mb })
        .collect()
}

/// Check all prerequisites (Docker, Docker Compose, MySQL)
pub async fn check_prerequisites() -> Result<Vec<PrerequisiteStatus>, crate::error::NucleusError> {
    let (docker, compose, mysql) = tokio::join!(
        check_docker_prereq(),
        check_compose_prereq(),
        check_mysql_prereq(),
    );

    Ok(vec![docker, compose, mysql])
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


#[cfg(test)]
mod tests {
    use super::match_puru_service;

    const REG: &str = "asia-south2-docker.pkg.dev/puru-255206/puru1";

    fn matched(image: &str, container: &str) -> Option<&'static str> {
        match_puru_service(image, container).map(|d| d.name)
    }

    #[test]
    fn detects_auth_and_dviewer_from_compose_templates() {
        assert_eq!(matched(&format!("{REG}/puru-auth:1.2"), "auth"), Some("Auth"));
        assert_eq!(matched(&format!("{REG}/dviewer:1.2"), "dviewer"), Some("DViewer"));
    }

    #[test]
    fn legacy_gcp_puru_auth_image_stays_xenon() {
        assert_eq!(matched(&format!("{REG}/gcp-puru-auth:old"), "backend"), Some("Xenon (Backend)"));
    }

    #[test]
    fn falls_back_to_container_name() {
        assert_eq!(matched("some/custom-image:1", "auth"), Some("Auth"));
        assert_eq!(matched("some/custom-image:1", "dviewer"), Some("DViewer"));
        assert_eq!(matched("some/custom-image:1", "unrelated"), None);
    }
}
