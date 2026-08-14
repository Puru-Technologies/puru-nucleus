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
    "puru-auth",
    "puru-neon",
    "puru-argon",
    "puru-comm",
    "puru-realtime",
    "puru-bridge",
    "puru-mercury",
    "puru-counter",
    "puru-integration",
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

pub(crate) fn get_credentials_path() -> Result<String, NucleusError> {
    let cfg = config::load_config()?;
    cfg.gcs_credentials_path
        .filter(|p| !p.is_empty())
        .ok_or_else(|| {
            NucleusError::GcsConnection(
                "No GCS credentials configured. Set credentials in Settings.".into(),
            )
        })
}

/// Translate the opaque google-cloud-auth token-exchange error into an
/// actionable message. The crate decodes Google's response into a success
/// struct, so when Google returns an *error* JSON (bad key, clock skew, …) it
/// fails with "error decoding response body: missing field `access_token`",
/// which hides the real cause. Detect that and explain the likely fixes.
pub(crate) fn explain_gcs_auth_error(raw: &str) -> String {
    if raw.contains("access_token")
        || raw.contains("decoding response body")
        || raw.contains("invalid_grant")
        || raw.contains("unauthorized_client")
    {
        format!(
            "Google rejected the service-account credentials while obtaining an access token ({raw}). \
             Likely causes: (1) the GCS service-account key is invalid, expired, or revoked — re-download it from \
             GCP and re-import it in Settings; or (2) this machine's date/time is wrong — a skewed system clock \
             makes the signed token invalid, so check the date, time, and timezone."
        )
    } else {
        format!("Failed to configure GCS client: {raw}")
    }
}

/// Storage scope we actually need. The google-cloud-storage crate's default
/// `with_credentials()` requests `cloud-platform` + `devstorage.full_control`,
/// and some service-account / org scope policies REJECT those broad scopes at
/// the token endpoint (surfacing as the opaque "missing field `access_token`"),
/// even when narrower scopes — like the `datastore` scope Firestore uses —
/// succeed with the same key. `devstorage.read_write` is enough to pull
/// releases/JARs and upload backups, and is far less likely to be restricted.
const STORAGE_SCOPES: [&str; 1] = ["https://www.googleapis.com/auth/devstorage.read_write"];

pub(crate) async fn create_gcs_client(
    credentials_path: &str,
) -> Result<google_cloud_storage::client::Client, NucleusError> {
    let cred_file =
        google_cloud_auth::credentials::CredentialsFile::new_from_file(credentials_path.to_string())
            .await
            .map_err(|e| {
                NucleusError::GcsConnection(format!("Failed to load credentials: {}", e))
            })?;
    let project_id = cred_file.project_id.clone();

    // Build the token source ourselves with an explicit, narrow scope instead of
    // the crate's broad default — see STORAGE_SCOPES above.
    let token_source = google_cloud_auth::token::DefaultTokenSourceProvider::new_with_credentials(
        google_cloud_auth::project::Config {
            audience: None,
            scopes: Some(&STORAGE_SCOPES),
            sub: None,
        },
        Box::new(cred_file),
    )
    .await
    .map_err(|e| NucleusError::GcsConnection(explain_gcs_auth_error(&e.to_string())))?;

    let mut client_config = google_cloud_storage::client::ClientConfig::default();
    client_config.project_id = project_id;
    client_config.token_source_provider = Some(Box::new(token_source));

    Ok(google_cloud_storage::client::Client::new(client_config))
}

/// List every object key under a prefix in the releases bucket (paginated).
/// Directory-placeholder objects (keys ending in `/`) are filtered out.
pub(crate) async fn list_gcs_objects(
    client: &google_cloud_storage::client::Client,
    prefix: &str,
) -> Result<Vec<String>, NucleusError> {
    use google_cloud_storage::http::objects::list::ListObjectsRequest;

    let mut names = Vec::new();
    let mut page_token: Option<String> = None;

    loop {
        let resp = client
            .list_objects(&ListObjectsRequest {
                bucket: RELEASES_BUCKET.to_string(),
                prefix: Some(prefix.to_string()),
                page_token: page_token.clone(),
                ..Default::default()
            })
            .await
            .map_err(|e| NucleusError::GcsConnection(format!("GCS list failed: {}", e)))?;

        if let Some(items) = resp.items {
            for obj in items {
                if !obj.name.ends_with('/') {
                    names.push(obj.name);
                }
            }
        }

        match resp.next_page_token {
            Some(token) if !token.is_empty() => page_token = Some(token),
            _ => break,
        }
    }

    Ok(names)
}

async fn download_gcs_bytes_from(
    client: &google_cloud_storage::client::Client,
    bucket: &str,
    object_path: &str,
) -> Result<Vec<u8>, NucleusError> {
    client
        .download_object(
            &google_cloud_storage::http::objects::get::GetObjectRequest {
                bucket: bucket.to_string(),
                object: object_path.to_string(),
                ..Default::default()
            },
            &google_cloud_storage::http::objects::download::Range::default(),
        )
        .await
        .map_err(|e| NucleusError::GcsConnection(format!("GCS download failed: {}", e)))
}

async fn download_gcs_bytes(
    client: &google_cloud_storage::client::Client,
    object_path: &str,
) -> Result<Vec<u8>, NucleusError> {
    download_gcs_bytes_from(client, RELEASES_BUCKET, object_path).await
}

/// Download an object from an arbitrary bucket to a local file.
pub(crate) async fn download_gcs_to_file_from(
    client: &google_cloud_storage::client::Client,
    bucket: &str,
    object_path: &str,
    local_path: &PathBuf,
) -> Result<f64, NucleusError> {
    let data = download_gcs_bytes_from(client, bucket, object_path).await?;

    if let Some(parent) = local_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    tokio::fs::write(local_path, &data).await?;

    let size_mb = data.len() as f64 / (1024.0 * 1024.0);
    Ok(size_mb)
}

pub(crate) async fn download_gcs_to_file(
    client: &google_cloud_storage::client::Client,
    object_path: &str,
    local_path: &PathBuf,
) -> Result<f64, NucleusError> {
    download_gcs_to_file_from(client, RELEASES_BUCKET, object_path, local_path).await
}

// ── Infra prerequisite manifest (installers hosted by oxygen) ─────────────────
//
// Nucleus resolves prerequisites through `infra/infra-manifest.json` (written by
// oxygen's /management/releases upload UI) rather than listing the folder — an
// artifact without a manifest entry is invisible. Schema matches oxygen's
// `Release.ts`: component id → { version, platforms: { <key>: { file, sha256? }}}.

const INFRA_MANIFEST_PATH: &str = "infra/infra-manifest.json";

/// One platform's file for an infra component.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct InfraPlatformFile {
    pub file: String,
    #[serde(default)]
    pub sha256: Option<String>,
}

/// A prerequisite component (mysql, erlang, rabbitmq, …) in the infra manifest.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct InfraComponent {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub platforms: std::collections::HashMap<String, InfraPlatformFile>,
}

/// `infra/infra-manifest.json` — component id → component.
pub(crate) type InfraManifest = std::collections::HashMap<String, InfraComponent>;

/// A resolved infra artifact ready to download.
#[derive(Debug, Clone)]
pub(crate) struct InfraArtifact {
    pub component: String,
    pub version: String,
    pub file: String,
    pub object_path: String,
    pub sha256: Option<String>,
}

/// Platform key used in the infra manifest (matches oxygen's `INFRA_PLATFORMS`).
pub(crate) fn infra_platform_key() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", _) => "windows-x64",
        ("linux", _) => "linux-x64",
        ("macos", "aarch64") => "macos-arm64",
        ("macos", _) => "macos-x64",
        _ => "windows-x64",
    }
}

/// Download and parse `infra/infra-manifest.json` from the releases bucket.
pub(crate) async fn fetch_infra_manifest(
    client: &google_cloud_storage::client::Client,
) -> Result<InfraManifest, NucleusError> {
    let bytes = download_gcs_bytes(client, INFRA_MANIFEST_PATH).await?;
    serde_json::from_slice(&bytes)
        .map_err(|e| NucleusError::Validation(format!("infra manifest parse error: {}", e)))
}

/// Resolve a component id to a concrete artifact for this platform, or None if
/// the component (or its platform entry) is missing from the manifest.
pub(crate) fn resolve_infra_artifact(
    manifest: &InfraManifest,
    component: &str,
) -> Option<InfraArtifact> {
    let comp = manifest.get(component)?;
    let pf = comp.platforms.get(infra_platform_key())?;
    Some(InfraArtifact {
        component: component.to_string(),
        version: comp.version.clone(),
        file: pf.file.clone(),
        object_path: format!("infra/{}", pf.file),
        sha256: pf.sha256.clone(),
    })
}

/// Stream-download an infra artifact to `dest`, invoking `on_progress(done,
/// total)` as bytes arrive, and verifying the manifest sha256 when present.
/// `total` is 0 when the object size couldn't be read (progress then shows
/// bytes-only). On sha256 mismatch the partial file is removed and an error
/// returned.
pub(crate) async fn download_infra_artifact<F>(
    client: &google_cloud_storage::client::Client,
    artifact: &InfraArtifact,
    dest: &PathBuf,
    on_progress: F,
) -> Result<(), NucleusError>
where
    F: Fn(u64, u64),
{
    use futures_util::StreamExt;
    use google_cloud_storage::http::objects::download::Range;
    use google_cloud_storage::http::objects::get::GetObjectRequest;
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncWriteExt;

    // Total size (for percent) from object metadata; 0 if unknown.
    let total = list_gcs_objects_with_meta(client, RELEASES_BUCKET, &artifact.object_path)
        .await
        .ok()
        .and_then(|v| v.into_iter().find(|m| m.name == artifact.object_path))
        .map(|m| m.size.max(0) as u64)
        .unwrap_or(0);

    let stream = client
        .download_streamed_object(
            &GetObjectRequest {
                bucket: RELEASES_BUCKET.to_string(),
                object: artifact.object_path.clone(),
                ..Default::default()
            },
            &Range::default(),
        )
        .await
        .map_err(|e| NucleusError::GcsConnection(format!("GCS stream download failed: {}", e)))?;
    futures_util::pin_mut!(stream);

    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = tokio::fs::File::create(dest).await?;
    let mut hasher = Sha256::new();
    let want_hash = artifact.sha256.is_some();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|e| NucleusError::GcsConnection(format!("download stream error: {}", e)))?;
        file.write_all(&chunk).await?;
        if want_hash {
            hasher.update(&chunk);
        }
        downloaded += chunk.len() as u64;
        on_progress(downloaded, total);
    }
    file.flush().await?;

    if let Some(expected) = &artifact.sha256 {
        let got: String = hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect();
        if !got.eq_ignore_ascii_case(expected) {
            let _ = tokio::fs::remove_file(dest).await;
            return Err(NucleusError::Validation(format!(
                "sha256 mismatch for {} (expected {}, got {})",
                artifact.file, expected, got
            )));
        }
    }

    Ok(())
}

/// Metadata for a GCS object — enough to detect content changes cheaply.
#[derive(Debug, Clone)]
pub(crate) struct GcsObjectMeta {
    pub name: String,
    pub size: i64,
    pub md5_hash: String,
    pub generation: i64,
}

/// Like `list_gcs_objects` but also returns each object's size/md5/generation,
/// so callers can diff a remote prefix against a previously-pulled snapshot.
pub(crate) async fn list_gcs_objects_with_meta(
    client: &google_cloud_storage::client::Client,
    bucket: &str,
    prefix: &str,
) -> Result<Vec<GcsObjectMeta>, NucleusError> {
    use google_cloud_storage::http::objects::list::ListObjectsRequest;

    let mut out = Vec::new();
    let mut page_token: Option<String> = None;

    loop {
        let resp = client
            .list_objects(&ListObjectsRequest {
                bucket: bucket.to_string(),
                prefix: Some(prefix.to_string()),
                page_token: page_token.clone(),
                ..Default::default()
            })
            .await
            .map_err(|e| NucleusError::GcsConnection(format!("GCS list failed: {}", e)))?;

        if let Some(items) = resp.items {
            for obj in items {
                if obj.name.ends_with('/') {
                    continue;
                }
                out.push(GcsObjectMeta {
                    name: obj.name,
                    size: obj.size,
                    md5_hash: obj.md5_hash.unwrap_or_default(),
                    generation: obj.generation,
                });
            }
        }

        match resp.next_page_token {
            Some(token) if !token.is_empty() => page_token = Some(token),
            _ => break,
        }
    }

    Ok(out)
}

/// Upload a local file to an object key in the given bucket.
pub(crate) async fn upload_gcs_file(
    client: &google_cloud_storage::client::Client,
    bucket: &str,
    local_path: &std::path::Path,
    object_path: &str,
) -> Result<(), NucleusError> {
    let data = tokio::fs::read(local_path).await?;

    let upload_type = google_cloud_storage::http::objects::upload::UploadType::Simple(
        google_cloud_storage::http::objects::upload::Media::new(object_path.to_string()),
    );

    client
        .upload_object(
            &google_cloud_storage::http::objects::upload::UploadObjectRequest {
                bucket: bucket.to_string(),
                ..Default::default()
            },
            data,
            &upload_type,
        )
        .await
        .map_err(|e| NucleusError::GcsConnection(format!("GCS upload failed: {}", e)))?;

    Ok(())
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

/// Install a downloaded nucleus update.
/// Linux: dpkg -i, Windows: msiexec /quiet, macOS: open dmg
///
/// On success, the caller should exit the app shortly after — the installer
/// replaces the running binary so a restart is needed.
pub async fn install_nucleus_update(file_path: &str) -> Result<String, NucleusError> {
    let path = std::path::Path::new(file_path);
    if !path.exists() {
        return Err(NucleusError::NotFound(format!(
            "Installer not found: {}", file_path
        )));
    }

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    match ext {
        "deb" => {
            let output = crate::process::silent_cmd("sudo")
                .args(["dpkg", "-i", file_path])
                .output()
                .await
                .map_err(|e| NucleusError::Internal(format!("dpkg failed: {}", e)))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(NucleusError::Internal(format!("dpkg install failed: {}", stderr)));
            }

            Ok("Update installed. Application will close — please reopen.".to_string())
        }
        "msi" => {
            // Spawn the MSI installer as a detached process so it can replace our binary
            // after we exit. Using cmd /c with a small delay to let us exit first.
            use std::process::Command;
            let script = format!(
                "timeout /t 3 /nobreak >nul & msiexec /i \"{}\" /quiet /norestart",
                file_path
            );
            Command::new("cmd")
                .args(["/C", &script])
                .spawn()
                .map_err(|e| NucleusError::Internal(format!("Failed to spawn installer: {}", e)))?;

            Ok("Update will install after app closes. Please reopen when done.".to_string())
        }
        "dmg" => {
            // macOS: open the DMG for manual drag-to-Applications
            let _ = crate::process::silent_cmd("open")
                .arg(file_path)
                .output()
                .await;

            Ok("DMG opened — drag to Applications to complete install.".to_string())
        }
        _ => Err(NucleusError::Validation(format!(
            "Unknown installer format: .{}", ext
        ))),
    }
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

// ── Native JAR deployment types ──────────────────────────────────────────────

/// Service → Java version mapping for native mode
const SERVICE_JAVA_VERSIONS: &[(&str, &str)] = &[
    ("puru-xenon", "21"),
    ("puru-has", "21"),
    ("puru-pacs", "21"),
    ("puru-auth", "21"),
    ("puru-neon", "21"),
    ("puru-comm", "21"),
    ("puru-argon", "21"),
    ("puru-bridge", "21"),
    ("puru-realtime", "21"),
    ("puru-mercury", "25"),
    ("puru-counter", "21"),
    ("puru-integration", "21"),
];

/// Metadata from Cloud Build JAR upload (meta.json)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JarBuildMeta {
    pub service: String,
    pub short_sha: String,
    pub commit_sha: String,
    pub build_id: String,
    pub built_at: String,
    /// JAR filename — absent for Angular builds
    #[serde(default)]
    pub jar_file: String,
    /// Original JAR filename — absent for Angular builds
    #[serde(default)]
    pub original_jar: String,
    /// Java version — absent for Angular builds
    #[serde(default)]
    pub java_version: String,
    /// Only present for Angular builds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact: Option<String>,
    /// "angular" for puru-hydrogen, absent for JARs
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub build_type: Option<String>,
}

/// Result of pulling a single JAR
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JarPullResult {
    pub service: String,
    pub success: bool,
    pub local_path: String,
    pub short_sha: String,
    pub size_mb: f64,
    pub message: String,
}

/// Result of pulling multiple JARs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullAllResult {
    pub results: Vec<JarPullResult>,
    pub total_size_mb: f64,
}

/// Update check for a single service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JarUpdateCheck {
    pub service: String,
    pub current_sha: String,
    pub latest_sha: String,
    pub latest_built_at: String,
    pub update_available: bool,
}

/// JRE manifest from GCS (maps java_version → { platform → filename })
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JreManifest(pub std::collections::HashMap<String, std::collections::HashMap<String, String>>);

/// Get Java version for a service name
pub fn java_version_for_service(service: &str) -> Option<&'static str> {
    SERVICE_JAVA_VERSIONS
        .iter()
        .find(|(s, _)| *s == service)
        .map(|(_, v)| *v)
}

/// Resolve platform string for JRE lookup (e.g. "linux-x64")
fn resolve_jre_platform() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let os_str = match os {
        "macos" => "mac",
        other => other,
    };
    let arch_str = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("{}-{}", os_str, arch_str)
}

// ── Native JAR pull functions ───────────────────────────────────────────────

/// Pull latest JAR for a single service from GCS.
/// Downloads to jars_dir, backs up existing JAR as .bak.
pub async fn pull_jar(service_name: &str) -> Result<JarPullResult, NucleusError> {
    if !UPDATABLE_SERVICES.contains(&service_name) && service_name != "puru-hydrogen" {
        return Err(NucleusError::Validation(format!(
            "Unknown service: '{}'. Valid services: {}",
            service_name,
            UPDATABLE_SERVICES.join(", ")
        )));
    }

    let cfg = config::load_config()?;
    let cred_path = get_credentials_path()?;
    let client = create_gcs_client(&cred_path).await?;

    // Check if this is an Angular build (puru-hydrogen)
    if service_name == "puru-hydrogen" {
        return pull_hydrogen_inner(&client, &cfg).await;
    }

    let jars_dir = cfg.jars_dir();

    // Download meta.json first
    let meta_path = format!("jars/{}/latest/meta.json", service_name);
    let meta_bytes = download_gcs_bytes(&client, &meta_path).await?;
    let meta: JarBuildMeta = serde_json::from_slice(&meta_bytes)?;

    // Download JAR
    let jar_gcs_path = format!("jars/{}/latest/{}.jar", service_name, service_name);
    let local_jar = jars_dir.join(format!("{}.jar", service_name));

    // Backup existing JAR
    if local_jar.exists() {
        let bak = jars_dir.join(format!("{}.jar.bak", service_name));
        let _ = tokio::fs::rename(&local_jar, &bak).await;
    }

    let size_mb = download_gcs_to_file(&client, &jar_gcs_path, &local_jar).await?;

    // Save meta locally
    let local_meta = jars_dir.join(format!("{}.meta.json", service_name));
    tokio::fs::write(&local_meta, &meta_bytes).await?;

    tracing::info!(
        "Pulled {} (sha: {}, {:.1} MB)",
        service_name,
        meta.short_sha,
        size_mb
    );

    Ok(JarPullResult {
        service: service_name.to_string(),
        success: true,
        local_path: local_jar.to_string_lossy().to_string(),
        short_sha: meta.short_sha,
        size_mb,
        message: "OK".to_string(),
    })
}

/// Pull a specific build by short SHA (for rollback).
pub async fn pull_jar_by_sha(service_name: &str, sha: &str) -> Result<JarPullResult, NucleusError> {
    if !UPDATABLE_SERVICES.contains(&service_name) {
        return Err(NucleusError::Validation(format!(
            "Unknown service: '{}'",
            service_name
        )));
    }

    let cfg = config::load_config()?;
    let cred_path = get_credentials_path()?;
    let client = create_gcs_client(&cred_path).await?;
    let jars_dir = cfg.jars_dir();

    // Download meta.json from builds/{sha}/
    let meta_path = format!("jars/{}/builds/{}/meta.json", service_name, sha);
    let meta_bytes = download_gcs_bytes(&client, &meta_path).await?;
    let meta: JarBuildMeta = serde_json::from_slice(&meta_bytes)?;

    // Download JAR from builds/{sha}/
    let jar_gcs_path = format!("jars/{}/builds/{}/{}.jar", service_name, sha, service_name);
    let local_jar = jars_dir.join(format!("{}.jar", service_name));

    // Backup existing
    if local_jar.exists() {
        let bak = jars_dir.join(format!("{}.jar.bak", service_name));
        let _ = tokio::fs::rename(&local_jar, &bak).await;
    }

    let size_mb = download_gcs_to_file(&client, &jar_gcs_path, &local_jar).await?;

    // Save meta locally
    let local_meta = jars_dir.join(format!("{}.meta.json", service_name));
    tokio::fs::write(&local_meta, &meta_bytes).await?;

    tracing::info!(
        "Pulled {} (sha: {}, {:.1} MB) [specific build]",
        service_name,
        meta.short_sha,
        size_mb
    );

    Ok(JarPullResult {
        service: service_name.to_string(),
        success: true,
        local_path: local_jar.to_string_lossy().to_string(),
        short_sha: meta.short_sha,
        size_mb,
        message: format!("Rolled back to build {}", sha),
    })
}

/// Pull latest JARs for multiple services.
pub async fn pull_all_jars(services: &[String]) -> Result<PullAllResult, NucleusError> {
    let mut results = Vec::new();
    let mut total_size = 0.0;

    for svc in services {
        match pull_jar(svc).await {
            Ok(r) => {
                total_size += r.size_mb;
                results.push(r);
            }
            Err(e) => {
                results.push(JarPullResult {
                    service: svc.clone(),
                    success: false,
                    local_path: String::new(),
                    short_sha: String::new(),
                    size_mb: 0.0,
                    message: e.user_message(),
                });
            }
        }
    }

    Ok(PullAllResult {
        results,
        total_size_mb: total_size,
    })
}

/// Check which services have newer JARs available.
pub async fn check_jar_updates_available(
    services: &[String],
) -> Result<Vec<JarUpdateCheck>, NucleusError> {
    let cred_path = get_credentials_path()?;
    let client = create_gcs_client(&cred_path).await?;
    let cfg = config::load_config()?;
    let jars_dir = cfg.jars_dir();

    let mut checks = Vec::new();

    for svc in services {
        // Read local meta
        let local_meta_path = jars_dir.join(format!("{}.meta.json", svc));
        let current_sha = if local_meta_path.exists() {
            match tokio::fs::read_to_string(&local_meta_path).await {
                Ok(content) => serde_json::from_str::<JarBuildMeta>(&content)
                    .map(|m| m.short_sha)
                    .unwrap_or_default(),
                Err(_) => String::new(),
            }
        } else {
            String::new()
        };

        // Fetch remote meta
        let remote_meta_path = format!("jars/{}/latest/meta.json", svc);
        match download_gcs_bytes(&client, &remote_meta_path).await {
            Ok(bytes) => match serde_json::from_slice::<JarBuildMeta>(&bytes) {
                Ok(remote_meta) => {
                    let update_available = current_sha.is_empty() || current_sha != remote_meta.short_sha;
                    checks.push(JarUpdateCheck {
                        service: svc.clone(),
                        current_sha: current_sha.clone(),
                        latest_sha: remote_meta.short_sha,
                        latest_built_at: remote_meta.built_at,
                        update_available,
                    });
                }
                Err(e) => {
                    tracing::warn!("Failed to parse meta for {}: {}", svc, e);
                }
            },
            Err(e) => {
                tracing::warn!("Failed to fetch meta for {}: {}", svc, e);
            }
        }
    }

    Ok(checks)
}

/// Read local meta.json to get current build info.
pub fn read_local_jar_meta(service_name: &str) -> Result<Option<JarBuildMeta>, NucleusError> {
    let cfg = config::load_config()?;
    let meta_path = cfg.jars_dir().join(format!("{}.meta.json", service_name));

    if !meta_path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&meta_path)?;
    let meta: JarBuildMeta = serde_json::from_str(&content)?;
    Ok(Some(meta))
}

// ── Hydrogen (Angular) pull ─────────────────────────────────────────────────

/// Internal: Pull latest puru-hydrogen Angular build from GCS.
async fn pull_hydrogen_inner(
    client: &google_cloud_storage::client::Client,
    cfg: &config::NucleusConfig,
) -> Result<JarPullResult, NucleusError> {
    let nginx_dir = cfg.nginx_html_dir();

    // Download meta.json
    let meta_path = "jars/puru-hydrogen/latest/meta.json";
    let meta_bytes = download_gcs_bytes(client, meta_path).await?;
    let meta: JarBuildMeta = serde_json::from_slice(&meta_bytes)?;

    // Download tarball
    let tarball_path = "jars/puru-hydrogen/latest/puru-hydrogen.tar.gz";
    let tmp_tarball = std::env::temp_dir().join("puru-hydrogen.tar.gz");
    let size_mb = download_gcs_to_file(client, tarball_path, &tmp_tarball).await?;

    // Backup existing html dir
    if nginx_dir.exists() {
        let bak = nginx_dir.with_extension("bak");
        let _ = tokio::fs::rename(&nginx_dir, &bak).await;
    }

    // Create dir and extract tarball
    tokio::fs::create_dir_all(&nginx_dir).await?;

    let mut tar_args = vec![
        "-xzf".to_string(),
        tmp_tarball.to_string_lossy().to_string(),
        "-C".to_string(),
        nginx_dir.to_string_lossy().to_string(),
    ];
    if !cfg!(windows) {
        tar_args.push("--no-same-owner".to_string());
    }
    let tar_output = crate::process::silent_cmd("tar")
        .args(&tar_args)
        .output()
        .await?;

    if !tar_output.status.success() {
        // Restore backup on failure
        let bak = nginx_dir.with_extension("bak");
        if bak.exists() {
            let _ = tokio::fs::remove_dir_all(&nginx_dir).await;
            let _ = tokio::fs::rename(&bak, &nginx_dir).await;
        }
        let stderr = String::from_utf8_lossy(&tar_output.stderr);
        return Err(NucleusError::Internal(format!(
            "Failed to extract puru-hydrogen tarball: {}", stderr
        )));
    }

    // Download nginx config template
    match download_gcs_bytes(client, "templates/nginx/puru-hydrogen.conf").await {
        Ok(nginx_conf) => {
            let conf_path = PathBuf::from("/etc/nginx/conf.d/puru.conf");
            if let Some(parent) = conf_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            tokio::fs::write(&conf_path, &nginx_conf).await?;

            // Reload nginx
            let _ = crate::process::silent_cmd("nginx")
                .args(["-s", "reload"])
                .status()
                .await;
        }
        Err(e) => {
            tracing::warn!("Failed to download nginx config template: {}", e);
        }
    }

    // Save meta locally
    let jars_dir = cfg.jars_dir();
    let _ = tokio::fs::create_dir_all(&jars_dir).await;
    let local_meta = jars_dir.join("puru-hydrogen.meta.json");
    tokio::fs::write(&local_meta, &meta_bytes).await?;

    // Clean up temp file
    let _ = tokio::fs::remove_file(&tmp_tarball).await;

    tracing::info!(
        "Pulled puru-hydrogen (sha: {}, {:.1} MB)",
        meta.short_sha,
        size_mb
    );

    Ok(JarPullResult {
        service: "puru-hydrogen".to_string(),
        success: true,
        local_path: nginx_dir.to_string_lossy().to_string(),
        short_sha: meta.short_sha,
        size_mb,
        message: "OK".to_string(),
    })
}

// ── JRE management ──────────────────────────────────────────────────────────

/// Download and extract JRE for given java version if not already present.
pub async fn ensure_jre(java_version: &str) -> Result<PathBuf, NucleusError> {
    let cfg = config::load_config()?;
    let jres_dir = cfg.jres_dir();
    let jre_dir = jres_dir.join(format!("temurin-{}", java_version));
    let java_bin = if cfg!(windows) {
        jre_dir.join("bin").join("java.exe")
    } else {
        jre_dir.join("bin").join("java")
    };

    // Already installed?
    if java_bin.exists() {
        return Ok(java_bin);
    }

    tracing::info!("JRE {} not found locally, downloading...", java_version);

    let cred_path = get_credentials_path()?;
    let client = create_gcs_client(&cred_path).await?;

    // Download manifest
    let manifest_bytes = download_gcs_bytes(&client, "jres/jre-manifest.json").await?;
    let manifest: JreManifest = serde_json::from_slice(&manifest_bytes)?;

    // Resolve platform
    let platform = resolve_jre_platform();
    let filename = manifest
        .0
        .get(java_version)
        .and_then(|platforms| platforms.get(&platform))
        .ok_or_else(|| {
            NucleusError::NotFound(format!(
                "No JRE found for Java {} on platform {}",
                java_version, platform
            ))
        })?;

    // Download archive (tar.gz on Linux/Mac, zip on Windows)
    let gcs_path = format!("jres/{}", filename);
    let tmp_archive = std::env::temp_dir().join(filename);
    let size_mb = download_gcs_to_file(&client, &gcs_path, &tmp_archive).await?;

    tracing::info!(
        "Downloaded JRE {} ({:.1} MB), extracting...",
        java_version,
        size_mb
    );

    // Extract — archives typically have a top-level dir like jdk-21.0.x-jre/
    // We extract to a temp dir and move the contents into our target dir
    let tmp_extract = std::env::temp_dir().join(format!("jre-extract-{}", java_version));
    let _ = tokio::fs::remove_dir_all(&tmp_extract).await;
    tokio::fs::create_dir_all(&tmp_extract).await?;

    let is_zip = filename.ends_with(".zip");

    let extract_output = if is_zip {
        // Use tar on Windows (Windows 10+ tar handles zip) or PowerShell fallback
        crate::process::silent_cmd("tar")
            .args([
                "-xf",
                &tmp_archive.to_string_lossy(),
                "-C",
                &tmp_extract.to_string_lossy(),
            ])
            .output()
            .await?
    } else {
        let mut jre_tar_args = vec![
            "-xzf".to_string(),
            tmp_archive.to_string_lossy().to_string(),
            "-C".to_string(),
            tmp_extract.to_string_lossy().to_string(),
        ];
        if !cfg!(windows) {
            jre_tar_args.push("--no-same-owner".to_string());
        }
        crate::process::silent_cmd("tar")
            .args(&jre_tar_args)
            .output()
            .await?
    };

    if !extract_output.status.success() {
        let stderr = String::from_utf8_lossy(&extract_output.stderr);
        return Err(NucleusError::Internal(format!(
            "Failed to extract JRE {} archive: {}",
            java_version, stderr
        )));
    }

    // Find the extracted directory (there should be exactly one top-level dir)
    let mut entries = tokio::fs::read_dir(&tmp_extract).await?;
    let extracted_dir = if let Some(entry) = entries.next_entry().await? {
        entry.path()
    } else {
        return Err(NucleusError::Internal(
            "JRE tarball extracted empty".into(),
        ));
    };

    // Move to final location
    let parent = jre_dir.parent().ok_or_else(|| {
        NucleusError::Internal(format!("Invalid JRE directory: {}", jre_dir.display()))
    })?;
    tokio::fs::create_dir_all(parent).await?;
    // Remove existing if corrupt
    let _ = tokio::fs::remove_dir_all(&jre_dir).await;
    tokio::fs::rename(&extracted_dir, &jre_dir).await?;

    // Cleanup
    let _ = tokio::fs::remove_file(&tmp_archive).await;
    let _ = tokio::fs::remove_dir_all(&tmp_extract).await;

    // Verify java binary exists
    if !java_bin.exists() {
        return Err(NucleusError::Internal(format!(
            "JRE {} extracted but java binary not found at {}",
            java_version,
            java_bin.display()
        )));
    }

    tracing::info!("JRE {} installed at {}", java_version, jre_dir.display());
    Ok(java_bin)
}

/// Pull JARs + ensure required JREs are present.
pub async fn pull_all_with_jres(services: &[String]) -> Result<PullAllResult, NucleusError> {
    let result = pull_all_jars(services).await?;

    // Collect unique java versions needed from successfully pulled services
    let mut java_versions = std::collections::HashSet::new();
    for r in &result.results {
        if r.success {
            if let Ok(Some(meta)) = read_local_jar_meta(&r.service) {
                java_versions.insert(meta.java_version);
            } else if let Some(ver) = java_version_for_service(&r.service) {
                java_versions.insert(ver.to_string());
            }
        }
    }

    // Download missing JREs (non-JAR artifacts like puru-hydrogen have no java version)
    for version in &java_versions {
        if version.is_empty() {
            continue;
        }
        if let Err(e) = ensure_jre(version).await {
            tracing::warn!("Failed to ensure JRE {}: {}", version, e);
        }
    }

    Ok(result)
}

/// Get list of all updatable service names (for CLI/UI)
pub fn all_updatable_services() -> Vec<String> {
    let mut services: Vec<String> = UPDATABLE_SERVICES.iter().map(|s| s.to_string()).collect();
    services.push("puru-hydrogen".to_string());
    services
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

    #[test]
    fn test_service_java_versions_in_sync_with_updatable() {
        // Every service in UPDATABLE_SERVICES must have a Java version mapping
        for svc in UPDATABLE_SERVICES {
            if *svc == "puru-hydrogen" {
                continue; // Angular, no Java needed
            }
            assert!(
                java_version_for_service(svc).is_some(),
                "Service {} is in UPDATABLE_SERVICES but missing from SERVICE_JAVA_VERSIONS",
                svc
            );
        }
        // Every service in SERVICE_JAVA_VERSIONS must be in UPDATABLE_SERVICES
        for (svc, _) in SERVICE_JAVA_VERSIONS {
            assert!(
                UPDATABLE_SERVICES.contains(svc),
                "Service {} is in SERVICE_JAVA_VERSIONS but missing from UPDATABLE_SERVICES",
                svc
            );
        }
    }
}
