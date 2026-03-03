//! Release management — check for updates, download installers/JARs from GCS

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::path::PathBuf;

use crate::config;
use crate::error::NucleusError;

// ── Constants ────────────────────────────────────────────────────────────────

const RELEASES_BUCKET: &str = "puru-releases";

const UPDATABLE_SERVICES: &[&str] = &[
    "puru-xenon",
    "puru-has",
    "puru-pacs",
    "puru-neon",
    "puru-argon",
    "puru-comm",
    "puru-realtime",
    "puru-bridge",
];

// ── Nucleus manifest types ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NucleusManifest {
    pub version: String,
    pub release_date: String,
    pub release_notes: String,
    pub min_supported_version: String,
    pub platforms: NucleusPlatforms,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NucleusPlatforms {
    pub windows: Option<PlatformArchs>,
    pub linux: Option<PlatformArchs>,
    pub macos: Option<PlatformArchs>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformArchs {
    pub x64: Option<PlatformArtifacts>,
    pub amd64: Option<PlatformArtifacts>,
    pub universal: Option<PlatformArtifacts>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformArtifacts {
    pub msi: Option<ArtifactInfo>,
    pub deb: Option<ArtifactInfo>,
    pub dmg: Option<ArtifactInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactInfo {
    pub file: String,
    pub sha256: String,
    pub size_mb: f64,
}

// ── Service manifest types ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceManifest {
    pub service: String,
    pub latest_version: String,
    pub versions: Vec<ServiceVersionInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceVersionInfo {
    pub version: String,
    pub release_date: String,
    pub jar: String,
    pub sha256: String,
    pub size_mb: f64,
    pub docker_tag: String,
    pub min_java: String,
    pub changelog: String,
}

// ── Command response types ───────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NucleusUpdateInfo {
    pub update_available: bool,
    pub current_version: String,
    pub latest_version: String,
    pub release_date: String,
    pub release_notes: String,
    pub download_size_mb: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceUpdateInfo {
    pub service: String,
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub release_date: String,
    pub changelog: String,
    pub size_mb: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadResult {
    pub success: bool,
    pub file_path: String,
    pub size_mb: f64,
}

// ── Version helpers ──────────────────────────────────────────────────────────

/// Compare two semver-style version strings numerically.
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect()
    };

    let va = parse(a);
    let vb = parse(b);
    let max_len = va.len().max(vb.len());

    for i in 0..max_len {
        let pa = va.get(i).copied().unwrap_or(0);
        let pb = vb.get(i).copied().unwrap_or(0);
        match pa.cmp(&pb) {
            Ordering::Equal => continue,
            other => return other,
        }
    }

    Ordering::Equal
}

/// Returns true if `latest` is strictly newer than `current`.
pub fn is_newer(latest: &str, current: &str) -> bool {
    compare_versions(latest, current) == Ordering::Greater
}

/// Extract version from a Docker image tag. E.g. `gcr.io/puru-255206/puru-xenon:2.3.5` → `"2.3.5"`.
/// Returns `"unknown"` for `:latest` or images without a tag.
pub fn extract_docker_version(image_tag: &str) -> String {
    match image_tag.rsplit_once(':') {
        Some((_, tag)) if tag != "latest" && !tag.is_empty() => tag.to_string(),
        _ => "unknown".to_string(),
    }
}

/// Match an image string against known updatable services.
/// E.g. `gcr.io/puru-255206/puru-xenon:2.3.5` → `Some("puru-xenon")`.
pub fn image_to_service_name(image: &str) -> Option<&'static str> {
    UPDATABLE_SERVICES
        .iter()
        .find(|&&svc| image.contains(svc))
        .copied()
}

/// Current nucleus version from Cargo.toml at build time.
pub fn current_nucleus_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Resolve the appropriate platform artifact from a nucleus manifest
/// based on the current OS and architecture.
fn resolve_platform_artifact(manifest: &NucleusManifest) -> Option<ArtifactInfo> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let platform = match os {
        "windows" => manifest.platforms.windows.as_ref(),
        "linux" => manifest.platforms.linux.as_ref(),
        "macos" => manifest.platforms.macos.as_ref(),
        _ => None,
    }?;

    // Try architecture-specific first, then universal
    let archs = match arch {
        "x86_64" => platform.x64.as_ref().or(platform.amd64.as_ref()),
        "aarch64" => platform.universal.as_ref(),
        _ => platform.x64.as_ref(),
    }
    .or(platform.universal.as_ref())?;

    // Pick the first available artifact format
    match os {
        "windows" => archs.msi.clone(),
        "linux" => archs.deb.clone(),
        "macos" => archs.dmg.clone(),
        _ => None,
    }
}

fn downloads_dir() -> PathBuf {
    let dir = config::config_dir().join("downloads");
    if !dir.exists() {
        let _ = std::fs::create_dir_all(&dir);
    }
    dir
}

// ── GCS helpers ──────────────────────────────────────────────────────────────

fn get_credentials_path() -> Result<String, NucleusError> {
    let cfg = config::load_config()?;
    cfg.gcs_credentials_path
        .filter(|p| !p.is_empty())
        .ok_or_else(|| {
            NucleusError::GcsConnection(
                "No GCS credentials configured. Set credentials in Settings.".into(),
            )
        })
}

async fn create_gcs_client(
    credentials_path: &str,
) -> Result<google_cloud_storage::client::Client, NucleusError> {
    let cred_file =
        google_cloud_auth::credentials::CredentialsFile::new_from_file(credentials_path.to_string())
            .await
            .map_err(|e| {
                NucleusError::GcsConnection(format!("Failed to load credentials: {}", e))
            })?;

    let client_config = google_cloud_storage::client::ClientConfig::default()
        .with_credentials(cred_file)
        .await
        .map_err(|e| {
            NucleusError::GcsConnection(format!("Failed to configure GCS client: {}", e))
        })?;

    Ok(google_cloud_storage::client::Client::new(client_config))
}

async fn download_gcs_bytes(
    client: &google_cloud_storage::client::Client,
    object_path: &str,
) -> Result<Vec<u8>, NucleusError> {
    client
        .download_object(
            &google_cloud_storage::http::objects::get::GetObjectRequest {
                bucket: RELEASES_BUCKET.to_string(),
                object: object_path.to_string(),
                ..Default::default()
            },
            &google_cloud_storage::http::objects::download::Range::default(),
        )
        .await
        .map_err(|e| NucleusError::GcsConnection(format!("GCS download failed: {}", e)))
}

async fn download_gcs_to_file(
    client: &google_cloud_storage::client::Client,
    object_path: &str,
    local_path: &PathBuf,
) -> Result<f64, NucleusError> {
    let data = download_gcs_bytes(client, object_path).await?;

    if let Some(parent) = local_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    tokio::fs::write(local_path, &data).await?;

    let size_mb = data.len() as f64 / (1024.0 * 1024.0);
    Ok(size_mb)
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Check if a nucleus (app) update is available.
pub async fn check_nucleus_update(channel: &str) -> Result<NucleusUpdateInfo, NucleusError> {
    let cred_path = get_credentials_path()?;
    let client = create_gcs_client(&cred_path).await?;

    let object_path = format!("nucleus/{}/latest.json", channel);
    let bytes = download_gcs_bytes(&client, &object_path).await?;
    let manifest: NucleusManifest = serde_json::from_slice(&bytes)?;

    let current = current_nucleus_version();
    let update_available = is_newer(&manifest.version, current);

    let download_size_mb = if update_available {
        resolve_platform_artifact(&manifest).map(|a| a.size_mb)
    } else {
        None
    };

    Ok(NucleusUpdateInfo {
        update_available,
        current_version: current.to_string(),
        latest_version: manifest.version,
        release_date: manifest.release_date,
        release_notes: manifest.release_notes,
        download_size_mb,
    })
}

/// Check for updates across all running Docker services.
pub async fn check_service_updates() -> Result<Vec<ServiceUpdateInfo>, NucleusError> {
    let cred_path = get_credentials_path()?;
    let client = create_gcs_client(&cred_path).await?;

    let services = crate::services::get_services()
        .await
        .unwrap_or_default();

    let mut updates = Vec::new();

    for svc in &services {
        let service_name = match image_to_service_name(&svc.image) {
            Some(name) => name,
            None => continue, // Not an updatable puru service (e.g. mysql, rabbitmq)
        };

        let current_version = extract_docker_version(&svc.image);

        let object_path = format!("services/{}/latest.json", service_name);
        match download_gcs_bytes(&client, &object_path).await {
            Ok(bytes) => match serde_json::from_slice::<ServiceManifest>(&bytes) {
                Ok(manifest) => {
                    let update_available = current_version == "unknown"
                        || is_newer(&manifest.latest_version, &current_version);

                    let latest_info = manifest.versions.first();

                    updates.push(ServiceUpdateInfo {
                        service: service_name.to_string(),
                        current_version,
                        latest_version: manifest.latest_version,
                        update_available,
                        release_date: latest_info
                            .map(|v| v.release_date.clone())
                            .unwrap_or_default(),
                        changelog: latest_info
                            .map(|v| v.changelog.clone())
                            .unwrap_or_default(),
                        size_mb: latest_info.map(|v| v.size_mb).unwrap_or(0.0),
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to parse manifest for {}: {}",
                        service_name,
                        e
                    );
                }
            },
            Err(e) => {
                tracing::warn!(
                    "Failed to fetch manifest for {}: {}",
                    service_name,
                    e
                );
            }
        }
    }

    Ok(updates)
}

/// Download the nucleus installer for the current platform.
pub async fn download_nucleus_update(channel: &str) -> Result<DownloadResult, NucleusError> {
    let cred_path = get_credentials_path()?;
    let client = create_gcs_client(&cred_path).await?;

    // Fetch manifest
    let object_path = format!("nucleus/{}/latest.json", channel);
    let bytes = download_gcs_bytes(&client, &object_path).await?;
    let manifest: NucleusManifest = serde_json::from_slice(&bytes)?;

    // Resolve artifact for this platform
    let artifact = resolve_platform_artifact(&manifest).ok_or_else(|| {
        NucleusError::NotFound(format!(
            "No installer available for {}/{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    })?;

    // Download artifact
    let local_path = downloads_dir().join(&artifact.file);
    let artifact_object = format!("nucleus/{}/{}", channel, artifact.file);
    let size_mb = download_gcs_to_file(&client, &artifact_object, &local_path).await?;

    tracing::info!(
        "Downloaded nucleus update: {} ({:.1} MB)",
        artifact.file,
        size_mb
    );

    Ok(DownloadResult {
        success: true,
        file_path: local_path.to_string_lossy().to_string(),
        size_mb,
    })
}

/// Download a specific service JAR version.
pub async fn download_service_jar(
    service_name: &str,
    version: &str,
) -> Result<DownloadResult, NucleusError> {
    // Validate service name
    if !UPDATABLE_SERVICES.contains(&service_name) {
        return Err(NucleusError::Validation(format!(
            "Unknown service: '{}'. Valid services: {}",
            service_name,
            UPDATABLE_SERVICES.join(", ")
        )));
    }

    let cred_path = get_credentials_path()?;
    let client = create_gcs_client(&cred_path).await?;

    // Fetch manifest
    let object_path = format!("services/{}/latest.json", service_name);
    let bytes = download_gcs_bytes(&client, &object_path).await?;
    let manifest: ServiceManifest = serde_json::from_slice(&bytes)?;

    // Find the requested version
    let version_info = manifest
        .versions
        .iter()
        .find(|v| v.version == version)
        .ok_or_else(|| {
            NucleusError::NotFound(format!(
                "Version '{}' not found for service '{}'",
                version, service_name
            ))
        })?;

    // Download JAR
    let service_dir = downloads_dir().join(service_name);
    let jar_filename = format!("{}-{}.jar", service_name, version);
    let local_path = service_dir.join(&jar_filename);
    let jar_object = format!("services/{}/{}", service_name, version_info.jar);
    let size_mb = download_gcs_to_file(&client, &jar_object, &local_path).await?;

    tracing::info!(
        "Downloaded {} v{}: {:.1} MB",
        service_name,
        version,
        size_mb
    );

    Ok(DownloadResult {
        success: true,
        file_path: local_path.to_string_lossy().to_string(),
        size_mb,
    })
}

/// List all available versions for a service.
pub async fn list_service_versions(
    service_name: &str,
) -> Result<ServiceManifest, NucleusError> {
    if !UPDATABLE_SERVICES.contains(&service_name) {
        return Err(NucleusError::Validation(format!(
            "Unknown service: '{}'. Valid services: {}",
            service_name,
            UPDATABLE_SERVICES.join(", ")
        )));
    }

    let cred_path = get_credentials_path()?;
    let client = create_gcs_client(&cred_path).await?;

    let object_path = format!("services/{}/latest.json", service_name);
    let bytes = download_gcs_bytes(&client, &object_path).await?;
    let manifest: ServiceManifest = serde_json::from_slice(&bytes)?;

    Ok(manifest)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compare_versions() {
        assert_eq!(compare_versions("1.0.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("2.0.0", "1.0.0"), Ordering::Greater);
        assert_eq!(compare_versions("1.0.0", "2.0.0"), Ordering::Less);
        assert_eq!(compare_versions("1.2.3", "1.2.3"), Ordering::Equal);
        assert_eq!(compare_versions("1.2.4", "1.2.3"), Ordering::Greater);
        assert_eq!(compare_versions("1.3.0", "1.2.9"), Ordering::Greater);
        // Different lengths
        assert_eq!(compare_versions("1.0", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("1.0.1", "1.0"), Ordering::Greater);
        assert_eq!(compare_versions("1", "1.0.0"), Ordering::Equal);
    }

    #[test]
    fn test_is_newer() {
        assert!(is_newer("2.0.0", "1.0.0"));
        assert!(is_newer("1.1.0", "1.0.0"));
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
    }

    #[test]
    fn test_extract_docker_version() {
        assert_eq!(
            extract_docker_version("gcr.io/puru-255206/puru-xenon:2.3.5"),
            "2.3.5"
        );
        assert_eq!(
            extract_docker_version("gcr.io/puru-255206/puru-has:1.0.0"),
            "1.0.0"
        );
        assert_eq!(
            extract_docker_version("gcr.io/puru-255206/puru-xenon:latest"),
            "unknown"
        );
        assert_eq!(
            extract_docker_version("gcr.io/puru-255206/puru-xenon"),
            "unknown"
        );
        assert_eq!(extract_docker_version("mysql:8.0"), "8.0");
    }

    #[test]
    fn test_image_to_service_name() {
        assert_eq!(
            image_to_service_name("gcr.io/puru-255206/puru-xenon:2.3.5"),
            Some("puru-xenon")
        );
        assert_eq!(
            image_to_service_name("gcr.io/puru-255206/puru-has:1.0.0"),
            Some("puru-has")
        );
        assert_eq!(
            image_to_service_name("gcr.io/puru-255206/puru-pacs:latest"),
            Some("puru-pacs")
        );
        // Third-party images should not match
        assert_eq!(image_to_service_name("mysql:8.0"), None);
        assert_eq!(image_to_service_name("rabbitmq:3-management"), None);
        assert_eq!(image_to_service_name("redis:7"), None);
    }
}
