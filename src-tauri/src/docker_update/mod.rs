//! Docker update management — live container updates with rollback and history

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Instant;

use crate::config;
use crate::error::NucleusError;
use crate::services::ServiceStatus;

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerUpdateResult {
    pub service: String,
    pub previous_image: String,
    pub new_image: String,
    pub success: bool,
    pub rolled_back: bool,
    pub message: String,
    pub duration_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateRecord {
    pub service: String,
    pub previous_image: String,
    pub new_image: String,
    pub success: bool,
    pub rolled_back: bool,
    pub timestamp: DateTime<Utc>,
    pub duration_seconds: u64,
    pub message: String,
}

// ── Constants ────────────────────────────────────────────────────────────────

/// Maps service image names to their compose service names.
/// Derived from the generated docker-compose.yml in commands/mod.rs.
const SERVICE_TO_COMPOSE: &[(&str, &str)] = &[
    ("puru-xenon", "backend"),
    ("puru-has", "has"),
    ("puru-pacs", "pacs"),
    ("puru-argon", "pathology"),
    ("puru-comm", "comm_server"),
    ("puru-realtime", "realtime"),
    ("puru-neon", "medical"),
    ("puru-bridge", "bridge"),
    ("puru-hydrogen", "frontend"),
];

const IMAGE_REGISTRY: &str = "gcr.io/puru-255206";

// ── TOML History persistence ─────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct UpdateHistory {
    records: Vec<UpdateRecord>,
}

fn history_path() -> PathBuf {
    config::config_dir().join("update_history.toml")
}

fn load_history() -> Result<Vec<UpdateRecord>, NucleusError> {
    let path = history_path();
    if !path.exists() {
        return Ok(vec![]);
    }

    let content = std::fs::read_to_string(&path)?;
    let history: UpdateHistory = toml::from_str(&content).map_err(|e| {
        NucleusError::InvalidConfig(format!("Failed to parse update history: {}", e))
    })?;

    Ok(history.records)
}

fn save_history(records: &[UpdateRecord]) -> Result<(), NucleusError> {
    let history = UpdateHistory {
        records: records.to_vec(),
    };

    let content = toml::to_string_pretty(&history).map_err(|e| {
        NucleusError::InvalidConfig(format!("Failed to serialize update history: {}", e))
    })?;

    let path = history_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, content)?;

    Ok(())
}

fn add_record(record: UpdateRecord) -> Result<(), NucleusError> {
    let mut records = load_history()?;
    records.push(record);
    save_history(&records)
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Look up the compose service name for a given image/service name.
pub fn service_to_compose_name(service_name: &str) -> Option<&'static str> {
    SERVICE_TO_COMPOSE
        .iter()
        .find(|(svc, _)| *svc == service_name)
        .map(|(_, compose)| *compose)
}

/// Replace the image tag for a specific service in a docker-compose.yml.
/// Finds `image: gcr.io/puru-255206/{service_name}:...` and replaces the tag.
pub fn replace_image_tag(compose_content: &str, service_name: &str, new_tag: &str) -> String {
    let prefix = format!("image: {}/{}", IMAGE_REGISTRY, service_name);

    compose_content
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with(&prefix) {
                let indent = &line[..line.len() - trimmed.len()];
                format!(
                    "{}image: {}/{}:{}",
                    indent, IMAGE_REGISTRY, service_name, new_tag
                )
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Extract the current tag from a compose file for a given service.
pub fn extract_current_tag(compose_content: &str, service_name: &str) -> Option<String> {
    let prefix = format!("{}/{}", IMAGE_REGISTRY, service_name);

    for line in compose_content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("image: ") {
            if rest.starts_with(&prefix) {
                return match rest.rsplit_once(':') {
                    Some((_, tag)) if !tag.is_empty() => Some(tag.to_string()),
                    _ => Some("latest".to_string()),
                };
            }
        }
    }

    None
}

/// Poll `get_services()` until the container is Running or timeout expires.
async fn wait_for_healthy(compose_name: &str, timeout_secs: u64) -> Result<bool, NucleusError> {
    let start = Instant::now();
    let timeout = std::time::Duration::from_secs(timeout_secs);

    loop {
        if start.elapsed() >= timeout {
            return Ok(false);
        }

        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        let services = crate::services::get_services().await?;
        let found = services.iter().find(|s| s.container_name == compose_name);

        if let Some(svc) = found {
            if svc.status == ServiceStatus::Running {
                return Ok(true);
            }
        }
    }
}

/// Run a docker compose command with V2/V1 fallback.
async fn run_compose_command(compose_path: &str, args: &[&str]) -> Result<(), NucleusError> {
    // Try docker compose (V2) first
    let mut v2_args = vec!["compose", "-f", compose_path];
    v2_args.extend_from_slice(args);

    let output = crate::process::silent_cmd("docker")
        .args(&v2_args)
        .output()
        .await;

    match output {
        Ok(out) if out.status.success() => return Ok(()),
        _ => {}
    }

    // Fallback to docker-compose (V1)
    let mut v1_args = vec!["-f", compose_path];
    v1_args.extend_from_slice(args);

    let output = crate::process::silent_cmd("docker-compose")
        .args(&v1_args)
        .output()
        .await
        .map_err(|e| NucleusError::Docker(format!("Failed to run docker compose: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NucleusError::Docker(format!(
            "docker compose failed: {}",
            stderr.trim()
        )));
    }

    Ok(())
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Update a single service to a new Docker image tag.
///
/// Flow:
/// 1. Validate service name
/// 2. Read compose file, extract current tag
/// 3. Backup compose to .bak
/// 4. `docker pull` new image
/// 5. Stop service via compose
/// 6. Replace image tag in compose file
/// 7. Start service via compose
/// 8. Wait for healthy (60s timeout)
/// 9. If unhealthy → restore .bak, restart old → rolled_back=true
/// 10. Record history, return result
pub async fn update_service(
    service_name: &str,
    new_tag: &str,
) -> Result<DockerUpdateResult, NucleusError> {
    let start = Instant::now();

    // 1) Validate service name
    let compose_name = service_to_compose_name(service_name).ok_or_else(|| {
        NucleusError::Validation(format!(
            "Unknown service '{}'. Valid services: {}",
            service_name,
            SERVICE_TO_COMPOSE
                .iter()
                .map(|(s, _)| *s)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;

    // 2) Load config, read compose file
    let cfg = config::load_config()?;
    let compose_path = &cfg.docker_compose_path;

    if !std::path::Path::new(compose_path).exists() {
        return Err(NucleusError::NotFound(format!(
            "docker-compose.yml at {}",
            compose_path
        )));
    }

    let compose_content = std::fs::read_to_string(compose_path)?;

    // 3) Extract current tag
    let current_tag = extract_current_tag(&compose_content, service_name)
        .unwrap_or_else(|| "latest".to_string());

    let previous_image = format!("{}/{}:{}", IMAGE_REGISTRY, service_name, current_tag);
    let new_image = format!("{}/{}:{}", IMAGE_REGISTRY, service_name, new_tag);

    tracing::info!(
        "Updating {} from {} to {}",
        service_name,
        current_tag,
        new_tag
    );

    // 4) Backup compose to .bak
    let bak_path = format!("{}.bak", compose_path);
    std::fs::copy(compose_path, &bak_path)?;

    // 5) Docker pull the new image
    let pull_output = crate::process::silent_cmd("docker")
        .args(["pull", &new_image])
        .output()
        .await
        .map_err(|e| NucleusError::Docker(format!("Failed to pull {}: {}", new_image, e)))?;

    if !pull_output.status.success() {
        let stderr = String::from_utf8_lossy(&pull_output.stderr);
        // Clean up backup since we didn't modify compose
        let _ = std::fs::remove_file(&bak_path);
        return Err(NucleusError::Docker(format!(
            "Failed to pull {}: {}",
            new_image,
            stderr.trim()
        )));
    }

    // 6) Stop the service
    run_compose_command(compose_path, &["stop", compose_name]).await?;

    // 7) Replace image tag in compose file and write
    let updated_content = replace_image_tag(&compose_content, service_name, new_tag);
    std::fs::write(compose_path, &updated_content)?;

    // 8) Start the service
    run_compose_command(compose_path, &["up", "-d", compose_name]).await?;

    // 9) Wait for healthy
    let healthy = wait_for_healthy(compose_name, 60).await.unwrap_or(false);

    let duration = start.elapsed().as_secs();

    if !healthy {
        // 10) Rollback: restore .bak, restart old
        tracing::warn!(
            "Service {} unhealthy after update, rolling back",
            service_name
        );
        std::fs::copy(&bak_path, compose_path)?;
        let _ = run_compose_command(compose_path, &["up", "-d", compose_name]).await;

        let result = DockerUpdateResult {
            service: service_name.to_string(),
            previous_image: previous_image.clone(),
            new_image: new_image.clone(),
            success: false,
            rolled_back: true,
            message: format!(
                "Service {} did not become healthy within 60s. Rolled back to {}.",
                service_name, current_tag
            ),
            duration_seconds: duration,
        };

        let record = UpdateRecord {
            service: result.service.clone(),
            previous_image: result.previous_image.clone(),
            new_image: result.new_image.clone(),
            success: false,
            rolled_back: true,
            timestamp: Utc::now(),
            duration_seconds: duration,
            message: result.message.clone(),
        };
        let _ = add_record(record);

        // Clean up backup
        let _ = std::fs::remove_file(&bak_path);

        return Ok(result);
    }

    // Success
    let _ = std::fs::remove_file(&bak_path);

    let result = DockerUpdateResult {
        service: service_name.to_string(),
        previous_image: previous_image.clone(),
        new_image: new_image.clone(),
        success: true,
        rolled_back: false,
        message: format!(
            "Successfully updated {} from {} to {}",
            service_name, current_tag, new_tag
        ),
        duration_seconds: duration,
    };

    let record = UpdateRecord {
        service: result.service.clone(),
        previous_image: result.previous_image.clone(),
        new_image: result.new_image.clone(),
        success: true,
        rolled_back: false,
        timestamp: Utc::now(),
        duration_seconds: duration,
        message: result.message.clone(),
    };
    let _ = add_record(record);

    tracing::info!("Update complete: {}", result.message);
    Ok(result)
}

/// Rollback a service to its previous version by finding the most recent
/// successful non-rollback update and reverting to its previous tag.
pub async fn rollback_service(
    service_name: &str,
) -> Result<DockerUpdateResult, NucleusError> {
    // Validate service name
    let _ = service_to_compose_name(service_name).ok_or_else(|| {
        NucleusError::Validation(format!("Unknown service '{}'", service_name))
    })?;

    let history = load_history()?;

    // Find the most recent successful, non-rollback update for this service
    let last_update = history
        .iter()
        .rev()
        .find(|r| r.service == service_name && r.success && !r.rolled_back);

    let last_update = last_update.ok_or_else(|| {
        NucleusError::NotFound(format!(
            "No previous successful update found for '{}'. Cannot rollback.",
            service_name
        ))
    })?;

    // Extract the previous tag from the previous image
    let previous_tag = crate::releases::extract_docker_version(&last_update.previous_image);
    if previous_tag == "unknown" {
        return Err(NucleusError::Validation(format!(
            "Cannot determine previous version for '{}'. Previous image: {}",
            service_name, last_update.previous_image
        )));
    }

    tracing::info!(
        "Rolling back {} to previous tag: {}",
        service_name,
        previous_tag
    );

    update_service(service_name, &previous_tag).await
}

/// Get the update history sorted by timestamp descending.
pub async fn get_update_history() -> Result<Vec<UpdateRecord>, NucleusError> {
    let mut records = load_history()?;
    records.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    Ok(records)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_COMPOSE: &str = r#"version: "3.8"

services:
  backend:
    image: gcr.io/puru-255206/puru-xenon:2.3.5
    container_name: backend
    restart: unless-stopped
    ports:
      - "8081:8081"

  has:
    image: gcr.io/puru-255206/puru-has:1.0.0
    container_name: has
    restart: unless-stopped
    ports:
      - "8082:8082"

  frontend:
    image: gcr.io/puru-255206/puru-hydrogen:latest
    container_name: frontend
    restart: unless-stopped
    ports:
      - "80:80"
"#;

    #[test]
    fn test_replace_image_tag() {
        let result = replace_image_tag(SAMPLE_COMPOSE, "puru-xenon", "3.0.0");

        // Correct service updated
        assert!(result.contains("gcr.io/puru-255206/puru-xenon:3.0.0"));
        // Other services untouched
        assert!(result.contains("gcr.io/puru-255206/puru-has:1.0.0"));
        assert!(result.contains("gcr.io/puru-255206/puru-hydrogen:latest"));
    }

    #[test]
    fn test_replace_image_tag_latest() {
        let result = replace_image_tag(SAMPLE_COMPOSE, "puru-hydrogen", "2.0.0");

        assert!(result.contains("gcr.io/puru-255206/puru-hydrogen:2.0.0"));
        // Others unchanged
        assert!(result.contains("gcr.io/puru-255206/puru-xenon:2.3.5"));
    }

    #[test]
    fn test_extract_current_tag() {
        assert_eq!(
            extract_current_tag(SAMPLE_COMPOSE, "puru-xenon"),
            Some("2.3.5".to_string())
        );
        assert_eq!(
            extract_current_tag(SAMPLE_COMPOSE, "puru-has"),
            Some("1.0.0".to_string())
        );
        assert_eq!(
            extract_current_tag(SAMPLE_COMPOSE, "puru-hydrogen"),
            Some("latest".to_string())
        );
        // Missing service
        assert_eq!(extract_current_tag(SAMPLE_COMPOSE, "puru-nonexistent"), None);
    }

    #[test]
    fn test_service_to_compose_name() {
        assert_eq!(service_to_compose_name("puru-xenon"), Some("backend"));
        assert_eq!(service_to_compose_name("puru-has"), Some("has"));
        assert_eq!(service_to_compose_name("puru-argon"), Some("pathology"));
        assert_eq!(service_to_compose_name("puru-comm"), Some("comm_server"));
        assert_eq!(service_to_compose_name("puru-hydrogen"), Some("frontend"));
        assert_eq!(service_to_compose_name("puru-unknown"), None);
    }
}
