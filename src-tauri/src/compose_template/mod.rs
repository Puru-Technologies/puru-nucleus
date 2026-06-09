//! Docker Compose template management — download from GCS, substitute variables, upload finalized

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::config;
use crate::error::NucleusError;

// ── Constants ────────────────────────────────────────────────────────────────

const TEMPLATE_BUCKET: &str = "puru-releases";
const BACKUP_BUCKET: &str = "puru-automated-backup";

const ENV_FILES: &[&str] = &[
    "general.env",
    "database.env",
    "database-neon.env",
    "rabbitmq.env",
    "has.env",
    "mail.env",
    "pacs.env",
    "realtime.env",
];

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FragmentDownloadResult {
    pub downloaded: Vec<String>,
    pub skipped: Vec<String>,
    pub fragments_dir: String,
}

/// (fragment_filename, module_key)
/// core.yml is always included. Others are gated by ServiceModules.
const FRAGMENTS: &[(&str, Option<&str>)] = &[
    ("core.yml", None), // always included
    ("auth.yml", Some("auth")),
    ("backend.yml", Some("xenon")),
    ("has.yml", Some("has")),
    ("pacs.yml", Some("pacs")),
    ("pathology.yml", Some("argon")),
    ("comm_server.yml", Some("comm")),
    ("realtime.yml", Some("realtime")),
    ("medical.yml", Some("neon")),
    ("mercury.yml", Some("mercury")),
    ("counter.yml", Some("counter")),
    ("bridge.yml", Some("bridge")),
    ("integration.yml", Some("integration")),
    ("frontend.yml", Some("hydrogen")),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateVariables {
    pub hospital_code: String,
    pub hospital_name: String,
    pub hospital_line2: String,
    pub hospital_line3: String,
    pub hospital_reg_no: String,
    pub hospital_logo_url: String,
    pub barcode_prefix_inventory: String,
    pub barcode_prefix_ppin: String,
    pub barcode_prefix_pathology: String,
    pub employee_prefix: String,
    pub barcode_prefix_sale: String,
    pub barcode_prefix_return: String,
    pub server_ip: String,
    pub mysql_password: String,
    pub rabbitmq_password: String,
    pub auth_tag: String,
    pub xenon_tag: String,
    pub has_tag: String,
    pub pacs_tag: String,
    pub argon_tag: String,
    pub comm_tag: String,
    pub realtime_tag: String,
    pub neon_tag: String,
    pub bridge_tag: String,
    pub integration_tag: String,
    pub hydrogen_tag: String,
}

/// Which service modules are enabled.
/// Core infra (database, rabbitmq, auth, fileserver) is always kept and not toggleable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceModules {
    pub auth: bool,
    pub xenon: bool,
    pub has: bool,
    pub pacs: bool,
    pub argon: bool,
    pub comm: bool,
    pub realtime: bool,
    pub neon: bool,
    pub mercury: bool,
    pub counter: bool,
    pub bridge: bool,
    pub integration: bool,
    pub hydrogen: bool,
}

impl Default for ServiceModules {
    fn default() -> Self {
        Self {
            auth: false,
            xenon: false,
            has: false,
            pacs: false,
            argon: false,
            comm: false,
            realtime: false,
            neon: false,
            mercury: false,
            counter: false,
            bridge: false,
            integration: false,
            hydrogen: false,
        }
    }
}

impl ServiceModules {
    /// All modules enabled — used for Docker mode when no modules are configured.
    pub fn all_enabled() -> Self {
        Self {
            auth: true,
            xenon: true,
            has: true,
            pacs: true,
            argon: true,
            comm: true,
            realtime: true,
            neon: true,
            mercury: true,
            counter: true,
            bridge: true,
            integration: true,
            hydrogen: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeUploadResult {
    pub success: bool,
    pub gcs_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvFileEntry {
    pub name: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvDownloadResult {
    pub files_downloaded: Vec<String>,
    pub files_skipped: Vec<String>,
    pub env_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvUploadResult {
    pub success: bool,
    pub files_uploaded: Vec<String>,
}

// ── Paths ────────────────────────────────────────────────────────────────────

/// Returns `{home}/puru/docker/`
fn docker_dir() -> PathBuf {
    config::home_puru_dir().join("docker")
}

/// Local compose file path — checks config first, falls back to docker_dir()
fn compose_file_path() -> PathBuf {
    let cfg = config::load_config().unwrap_or_default();
    if !cfg.docker_compose_path.is_empty() {
        PathBuf::from(&cfg.docker_compose_path)
    } else {
        docker_dir().join("docker-compose.yml")
    }
}

/// Env directory — sibling `env/` next to compose file
fn env_dir_path() -> PathBuf {
    compose_file_path()
        .parent()
        .map(|p| p.join("env"))
        .unwrap_or_else(|| docker_dir().join("env"))
}

/// Fragments directory — sibling `fragments/` next to compose file
fn fragments_dir() -> PathBuf {
    compose_file_path()
        .parent()
        .map(|p| p.join("fragments"))
        .unwrap_or_else(|| docker_dir().join("fragments"))
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

async fn upload_gcs_bytes(
    client: &google_cloud_storage::client::Client,
    bucket: &str,
    object_name: &str,
    data: Vec<u8>,
) -> Result<(), NucleusError> {
    let upload_type = google_cloud_storage::http::objects::upload::UploadType::Simple(
        google_cloud_storage::http::objects::upload::Media::new(object_name.to_string()),
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

/// Download all fragment files from GCS into the local fragments/ directory.
/// Skips files that already exist locally.
/// Source: `puru-releases/templates/fragments/{name}`
pub async fn download_fragments() -> Result<FragmentDownloadResult, NucleusError> {
    let frag_dir = fragments_dir();
    tokio::fs::create_dir_all(&frag_dir).await?;

    let mut downloaded = Vec::new();
    let mut skipped = Vec::new();

    let cred_path = get_credentials_path()?;
    let client = create_gcs_client(&cred_path).await?;

    for &(name, _) in FRAGMENTS {
        let local_path = frag_dir.join(name);
        if local_path.exists() {
            skipped.push(name.to_string());
            continue;
        }

        let object_path = format!("templates/fragments/{}", name);
        match download_gcs_bytes(&client, TEMPLATE_BUCKET, &object_path).await {
            Ok(bytes) => {
                tokio::fs::write(&local_path, &bytes).await?;
                downloaded.push(name.to_string());
                tracing::info!("Downloaded fragment: {}", name);
            }
            Err(e) => {
                tracing::warn!("Skipping fragment {} (not found on GCS): {}", name, e);
                skipped.push(name.to_string());
            }
        }
    }

    Ok(FragmentDownloadResult {
        downloaded,
        skipped,
        fragments_dir: frag_dir.to_string_lossy().to_string(),
    })
}

/// Read the local compose file content.
pub async fn read_compose_file() -> Result<String, NucleusError> {
    let local_path = compose_file_path();
    if !local_path.exists() {
        return Err(NucleusError::NotFound(
            "Docker compose file not found. Download the template first.".into(),
        ));
    }
    let content = tokio::fs::read_to_string(&local_path).await?;
    Ok(content)
}

/// Substitute template placeholders with actual values.
/// Placeholders use `{{VARIABLE_NAME}}` syntax.
pub fn substitute_variables(content: &str, vars: &TemplateVariables) -> String {
    content
        .replace("{{HOSPITAL_CODE}}", &vars.hospital_code)
        .replace("{{HOSPITAL_NAME}}", &vars.hospital_name)
        .replace("{{HOSPITAL_LINE2}}", &vars.hospital_line2)
        .replace("{{HOSPITAL_LINE3}}", &vars.hospital_line3)
        .replace("{{HOSPITAL_REG_NO}}", &vars.hospital_reg_no)
        .replace("{{HOSPITAL_LOGO_URL}}", &vars.hospital_logo_url)
        .replace("{{BARCODE_PREFIX_INVENTORY}}", &vars.barcode_prefix_inventory)
        .replace("{{BARCODE_PREFIX_PPIN}}", &vars.barcode_prefix_ppin)
        .replace("{{BARCODE_PREFIX_PATHOLOGY}}", &vars.barcode_prefix_pathology)
        .replace("{{EMPLOYEE_PREFIX}}", &vars.employee_prefix)
        .replace("{{BARCODE_PREFIX_SALE}}", &vars.barcode_prefix_sale)
        .replace("{{BARCODE_PREFIX_RETURN}}", &vars.barcode_prefix_return)
        .replace("{{SERVER_IP}}", &vars.server_ip)
        .replace("{{MYSQL_PASSWORD}}", &vars.mysql_password)
        .replace("{{RABBITMQ_PASSWORD}}", &vars.rabbitmq_password)
        .replace("{{AUTH_TAG}}", &vars.auth_tag)
        .replace("{{XENON_TAG}}", &vars.xenon_tag)
        .replace("{{HAS_TAG}}", &vars.has_tag)
        .replace("{{PACS_TAG}}", &vars.pacs_tag)
        .replace("{{ARGON_TAG}}", &vars.argon_tag)
        .replace("{{COMM_TAG}}", &vars.comm_tag)
        .replace("{{REALTIME_TAG}}", &vars.realtime_tag)
        .replace("{{NEON_TAG}}", &vars.neon_tag)
        .replace("{{BRIDGE_TAG}}", &vars.bridge_tag)
        .replace("{{INTEGRATION_TAG}}", &vars.integration_tag)
        .replace("{{HYDROGEN_TAG}}", &vars.hydrogen_tag)
}

/// Assemble a docker-compose.yml from fragment files based on enabled modules.
///
/// 1. Read core.yml from fragments_dir()
/// 2. Append `\nservices:\n`
/// 3. For each enabled module fragment, read and append its content
/// 4. Return the assembled string
pub fn assemble_compose(modules: &ServiceModules) -> Result<String, NucleusError> {
    let frag_dir = fragments_dir();

    // Read core.yml (always included)
    let core_path = frag_dir.join("core.yml");
    if !core_path.exists() {
        return Err(NucleusError::NotFound(
            "core.yml fragment not found. Download fragments first.".into(),
        ));
    }
    let mut assembled = std::fs::read_to_string(&core_path)
        .map_err(|e| NucleusError::Io(e))?;

    assembled.push_str("\nservices:\n");

    // Helper: check if a module key is enabled
    let is_enabled = |key: &str| -> bool {
        match key {
            "auth" => modules.auth,
            "xenon" => modules.xenon,
            "has" => modules.has,
            "pacs" => modules.pacs,
            "argon" => modules.argon,
            "comm" => modules.comm,
            "realtime" => modules.realtime,
            "neon" => modules.neon,
            "mercury" => modules.mercury,
            "counter" => modules.counter,
            "bridge" => modules.bridge,
            "integration" => modules.integration,
            "hydrogen" => modules.hydrogen,
            _ => false,
        }
    };

    for &(filename, module_key) in FRAGMENTS {
        // Skip core.yml (already handled) and disabled modules
        let Some(key) = module_key else { continue };
        if !is_enabled(key) {
            continue;
        }

        let frag_path = frag_dir.join(filename);
        if !frag_path.exists() {
            tracing::warn!("Fragment file {} not found, skipping", filename);
            continue;
        }

        let content = std::fs::read_to_string(&frag_path)
            .map_err(|e| NucleusError::Io(e))?;
        assembled.push_str(&content);
        // Ensure trailing newline between fragments
        if !content.ends_with('\n') {
            assembled.push('\n');
        }
    }

    Ok(assembled)
}

/// Save compose content to the local file path.
pub async fn save_compose_file(content: &str) -> Result<String, NucleusError> {
    let local_path = compose_file_path();

    if let Some(parent) = local_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    tokio::fs::write(&local_path, content).await?;

    let path_str = local_path.to_string_lossy().to_string();
    tracing::info!("Saved compose file to {}", path_str);
    Ok(path_str)
}

/// Upload the finalized compose file to GCS.
/// Target: `puru-automated-backup/{hospital_code}/config/docker-compose.yml`
pub async fn upload_compose_to_gcs(hospital_code: &str) -> Result<ComposeUploadResult, NucleusError> {
    let local_path = compose_file_path();
    if !local_path.exists() {
        return Err(NucleusError::NotFound(
            "Docker compose file not found. Save locally first.".into(),
        ));
    }

    let data = tokio::fs::read(&local_path).await?;

    let cred_path = get_credentials_path()?;
    let client = create_gcs_client(&cred_path).await?;

    let gcs_path = format!("{}/config/docker-compose.yml", hospital_code);
    upload_gcs_bytes(&client, BACKUP_BUCKET, &gcs_path, data).await?;

    tracing::info!("Uploaded compose file to gs://{}/{}", BACKUP_BUCKET, gcs_path);

    Ok(ComposeUploadResult {
        success: true,
        gcs_path: format!("gs://{}/{}", BACKUP_BUCKET, gcs_path),
    })
}

// ── Env file API ─────────────────────────────────────────────────────────────

/// Download env file templates from GCS. Skips files that already exist locally.
/// Source: `puru-releases/templates/env/{file}`
pub async fn download_env_templates() -> Result<EnvDownloadResult, NucleusError> {
    let env_dir = env_dir_path();
    tokio::fs::create_dir_all(&env_dir).await?;

    let mut downloaded = Vec::new();
    let mut skipped = Vec::new();

    let cred_path = get_credentials_path()?;
    let client = create_gcs_client(&cred_path).await?;

    for &name in ENV_FILES {
        let local_path = env_dir.join(name);
        if local_path.exists() {
            skipped.push(name.to_string());
            continue;
        }

        let object_path = format!("templates/env/{}", name);
        match download_gcs_bytes(&client, TEMPLATE_BUCKET, &object_path).await {
            Ok(bytes) => {
                tokio::fs::write(&local_path, &bytes).await?;
                downloaded.push(name.to_string());
                tracing::info!("Downloaded env template: {}", name);
            }
            Err(e) => {
                tracing::warn!("Skipping env template {} (not found on GCS): {}", name, e);
                skipped.push(name.to_string());
            }
        }
    }

    Ok(EnvDownloadResult {
        files_downloaded: downloaded,
        files_skipped: skipped,
        env_dir: env_dir.to_string_lossy().to_string(),
    })
}

/// List all env files with their content.
pub async fn read_env_files() -> Result<Vec<EnvFileEntry>, NucleusError> {
    let env_dir = env_dir_path();
    let mut entries = Vec::new();

    for &name in ENV_FILES {
        let path = env_dir.join(name);
        if path.exists() {
            let content = tokio::fs::read_to_string(&path).await?;
            entries.push(EnvFileEntry {
                name: name.to_string(),
                content,
            });
        }
    }

    Ok(entries)
}

/// Save a single env file by name.
pub async fn save_env_file(name: &str, content: &str) -> Result<String, NucleusError> {
    // Validate name is one of the known env files
    if !ENV_FILES.contains(&name) {
        return Err(NucleusError::Validation(format!(
            "Unknown env file: '{}'. Valid files: {}",
            name,
            ENV_FILES.join(", ")
        )));
    }

    let env_dir = env_dir_path();
    tokio::fs::create_dir_all(&env_dir).await?;

    let path = env_dir.join(name);
    tokio::fs::write(&path, content).await?;

    let path_str = path.to_string_lossy().to_string();
    tracing::info!("Saved env file: {}", path_str);
    Ok(path_str)
}

/// Upload all local env files to GCS.
/// Target: `puru-automated-backup/{hospital_code}/config/env/{file}`
pub async fn upload_env_files_to_gcs(hospital_code: &str) -> Result<EnvUploadResult, NucleusError> {
    let env_dir = env_dir_path();
    if !env_dir.exists() {
        return Err(NucleusError::NotFound(
            "Env directory not found. Download templates first.".into(),
        ));
    }

    let cred_path = get_credentials_path()?;
    let client = create_gcs_client(&cred_path).await?;

    let mut uploaded = Vec::new();

    for &name in ENV_FILES {
        let path = env_dir.join(name);
        if !path.exists() {
            continue;
        }
        let data = tokio::fs::read(&path).await?;
        let gcs_path = format!("{}/config/env/{}", hospital_code, name);
        upload_gcs_bytes(&client, BACKUP_BUCKET, &gcs_path, data).await?;
        uploaded.push(name.to_string());
    }

    tracing::info!("Uploaded {} env files for {}", uploaded.len(), hospital_code);

    Ok(EnvUploadResult {
        success: true,
        files_uploaded: uploaded,
    })
}

/// Build template variables from the current NucleusConfig, with sensible defaults.
pub fn build_variables_from_config(cfg: &config::NucleusConfig) -> TemplateVariables {
    TemplateVariables {
        hospital_code: cfg.hospital_code.clone(),
        hospital_name: String::new(),
        hospital_line2: String::new(),
        hospital_line3: String::new(),
        hospital_reg_no: String::new(),
        hospital_logo_url: "https://storage.googleapis.com/puru-public-files/logo-new-puru.png".to_string(),
        barcode_prefix_inventory: String::new(),
        barcode_prefix_ppin: String::new(),
        barcode_prefix_pathology: String::new(),
        employee_prefix: "EMP".to_string(),
        barcode_prefix_sale: "2526S".to_string(),
        barcode_prefix_return: "2526R".to_string(),
        server_ip: cfg.server_ip.clone(),
        mysql_password: cfg.mysql_password.clone(),
        rabbitmq_password: "puru123".to_string(),
        auth_tag: "latest".to_string(),
        xenon_tag: "latest".to_string(),
        has_tag: "latest".to_string(),
        pacs_tag: "latest".to_string(),
        argon_tag: "latest".to_string(),
        comm_tag: "latest".to_string(),
        realtime_tag: "latest".to_string(),
        neon_tag: "latest".to_string(),
        bridge_tag: "latest".to_string(),
        integration_tag: "latest".to_string(),
        hydrogen_tag: "latest".to_string(),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_substitute_variables() {
        let template = r#"
services:
  xenon:
    image: gcr.io/puru-255206/puru-xenon:{{XENON_TAG}}
    environment:
      HOSPITAL_CODE: {{HOSPITAL_CODE}}
      SERVER_IP: {{SERVER_IP}}
      MYSQL_PASSWORD: {{MYSQL_PASSWORD}}
      RABBITMQ_PASSWORD: {{RABBITMQ_PASSWORD}}
  has:
    image: gcr.io/puru-255206/puru-has:{{HAS_TAG}}
  pacs:
    image: gcr.io/puru-255206/puru-pacs:{{PACS_TAG}}
  argon:
    image: gcr.io/puru-255206/puru-argon:{{ARGON_TAG}}
  comm:
    image: gcr.io/puru-255206/puru-comm:{{COMM_TAG}}
  realtime:
    image: gcr.io/puru-255206/puru-realtime:{{REALTIME_TAG}}
  neon:
    image: gcr.io/puru-255206/puru-neon:{{NEON_TAG}}
  bridge:
    image: gcr.io/puru-255206/puru-bridge:{{BRIDGE_TAG}}
  hydrogen:
    image: gcr.io/puru-255206/puru-hydrogen:{{HYDROGEN_TAG}}
"#;

        let vars = TemplateVariables {
            hospital_code: "BTCT".to_string(),
            hospital_name: "Test Hospital".to_string(),
            hospital_line2: "Test Address".to_string(),
            hospital_line3: String::new(),
            hospital_reg_no: "NH/1234/2025".to_string(),
            hospital_logo_url: "https://example.com/logo.png".to_string(),
            barcode_prefix_inventory: "BDS".to_string(),
            barcode_prefix_ppin: "BDS".to_string(),
            barcode_prefix_pathology: String::new(),
            employee_prefix: "EMP".to_string(),
            barcode_prefix_sale: "2526S".to_string(),
            barcode_prefix_return: "2526R".to_string(),
            server_ip: "192.168.1.100".to_string(),
            mysql_password: "secret123".to_string(),
            rabbitmq_password: "rabbitpw".to_string(),
            auth_tag: "1.0.0".to_string(),
            xenon_tag: "2.3.5".to_string(),
            has_tag: "1.0.0".to_string(),
            pacs_tag: "1.2.0".to_string(),
            argon_tag: "1.1.0".to_string(),
            comm_tag: "1.0.5".to_string(),
            realtime_tag: "1.0.0".to_string(),
            neon_tag: "1.3.0".to_string(),
            bridge_tag: "1.0.2".to_string(),
            integration_tag: "1.0.0".to_string(),
            hydrogen_tag: "3.0.0".to_string(),
        };

        let result = substitute_variables(template, &vars);

        assert!(result.contains("puru-xenon:2.3.5"));
        assert!(result.contains("HOSPITAL_CODE: BTCT"));
        assert!(result.contains("SERVER_IP: 192.168.1.100"));
        assert!(result.contains("MYSQL_PASSWORD: secret123"));
        assert!(result.contains("RABBITMQ_PASSWORD: rabbitpw"));
        assert!(result.contains("puru-has:1.0.0"));
        assert!(result.contains("puru-pacs:1.2.0"));
        assert!(result.contains("puru-argon:1.1.0"));
        assert!(result.contains("puru-comm:1.0.5"));
        assert!(result.contains("puru-realtime:1.0.0"));
        assert!(result.contains("puru-neon:1.3.0"));
        assert!(result.contains("puru-bridge:1.0.2"));
        assert!(result.contains("puru-hydrogen:3.0.0"));
        // No unsubstituted placeholders
        assert!(!result.contains("{{"));
    }

    #[test]
    fn test_build_variables_defaults() {
        let cfg = config::NucleusConfig {
            hospital_code: "TEST".to_string(),
            server_ip: "10.0.0.1".to_string(),
            mysql_password: "mypass".to_string(),
            ..Default::default()
        };

        let vars = build_variables_from_config(&cfg);

        assert_eq!(vars.hospital_code, "TEST");
        assert_eq!(vars.server_ip, "10.0.0.1");
        assert_eq!(vars.mysql_password, "mypass");
        assert_eq!(vars.rabbitmq_password, "puru123");
        // All service tags default to "latest"
        assert_eq!(vars.xenon_tag, "latest");
        assert_eq!(vars.has_tag, "latest");
        assert_eq!(vars.pacs_tag, "latest");
        assert_eq!(vars.argon_tag, "latest");
        assert_eq!(vars.comm_tag, "latest");
        assert_eq!(vars.realtime_tag, "latest");
        assert_eq!(vars.neon_tag, "latest");
        assert_eq!(vars.bridge_tag, "latest");
        assert_eq!(vars.integration_tag, "latest");
        assert_eq!(vars.hydrogen_tag, "latest");
    }

    #[test]
    fn test_substitute_preserves_non_placeholder_braces() {
        let template = r#"
services:
  xenon:
    image: gcr.io/puru-255206/puru-xenon:{{XENON_TAG}}
    environment: {}
    deploy:
      resources:
        limits:
          memory: 512M
    labels:
      com.example: "{value}"
"#;

        let vars = TemplateVariables {
            hospital_code: "X".to_string(),
            hospital_name: String::new(),
            hospital_line2: String::new(),
            hospital_line3: String::new(),
            hospital_reg_no: String::new(),
            hospital_logo_url: String::new(),
            barcode_prefix_inventory: String::new(),
            barcode_prefix_ppin: String::new(),
            barcode_prefix_pathology: String::new(),
            employee_prefix: String::new(),
            barcode_prefix_sale: String::new(),
            barcode_prefix_return: String::new(),
            server_ip: "1.2.3.4".to_string(),
            mysql_password: "p".to_string(),
            rabbitmq_password: "r".to_string(),
            auth_tag: "latest".to_string(),
            xenon_tag: "2.0.0".to_string(),
            has_tag: "latest".to_string(),
            pacs_tag: "latest".to_string(),
            argon_tag: "latest".to_string(),
            comm_tag: "latest".to_string(),
            realtime_tag: "latest".to_string(),
            neon_tag: "latest".to_string(),
            bridge_tag: "latest".to_string(),
            integration_tag: "latest".to_string(),
            hydrogen_tag: "latest".to_string(),
        };

        let result = substitute_variables(template, &vars);

        // YAML braces preserved
        assert!(result.contains("environment: {}"));
        assert!(result.contains("\"{value}\""));
        // Placeholder substituted
        assert!(result.contains("puru-xenon:2.0.0"));
    }

    /// Helper: create a temp fragments directory with test fragment files
    fn setup_test_fragments(dir: &std::path::Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("core.yml"), "version: \"3.8\"").unwrap();
        std::fs::write(
            dir.join("auth.yml"),
            "  auth:\n    image: asia-south2-docker.pkg.dev/puru-255206/puru1/puru-auth:{{AUTH_TAG}}\n    container_name: auth\n    restart: always\n    network_mode: host\n",
        ).unwrap();
        std::fs::write(
            dir.join("backend.yml"),
            "  backend:\n    image: asia-south2-docker.pkg.dev/puru-255206/puru1/puru-xenon:{{XENON_TAG}}\n    container_name: backend\n    restart: always\n    network_mode: host\n",
        ).unwrap();
        std::fs::write(
            dir.join("has.yml"),
            "  has:\n    image: asia-south2-docker.pkg.dev/puru-255206/puru1/puru-has:{{HAS_TAG}}\n    container_name: has\n    restart: always\n    network_mode: host\n",
        ).unwrap();
        std::fs::write(
            dir.join("pacs.yml"),
            "  pacs:\n    image: asia-south2-docker.pkg.dev/puru-255206/puru1/gcp-puru-pacs:{{PACS_TAG}}\n    container_name: pacs\n    restart: always\n    network_mode: host\n",
        ).unwrap();
        std::fs::write(
            dir.join("pathology.yml"),
            "  pathology:\n    image: asia-south2-docker.pkg.dev/puru-255206/puru1/puru-argon:{{ARGON_TAG}}\n    container_name: pathology\n    restart: always\n    network_mode: host\n",
        ).unwrap();
        std::fs::write(
            dir.join("comm_server.yml"),
            "  comm_server:\n    image: asia-south2-docker.pkg.dev/puru-255206/puru1/gcp-puru-comm:{{COMM_TAG}}\n    container_name: comm_server\n    restart: always\n    network_mode: host\n",
        ).unwrap();
        std::fs::write(
            dir.join("realtime.yml"),
            "  realtime:\n    image: asia-south2-docker.pkg.dev/puru-255206/puru1/gcp-puru-realtime:{{REALTIME_TAG}}\n    container_name: realtime\n    restart: always\n    network_mode: host\n",
        ).unwrap();
        std::fs::write(
            dir.join("medical.yml"),
            "  medical:\n    image: asia-south2-docker.pkg.dev/puru-255206/puru1/puru-neon:{{NEON_TAG}}\n    container_name: medical\n    restart: always\n    network_mode: host\n",
        ).unwrap();
        std::fs::write(
            dir.join("bridge.yml"),
            "  bridge:\n    image: asia-south2-docker.pkg.dev/puru-255206/puru1/puru-bridge:{{BRIDGE_TAG}}\n    container_name: bridge\n    restart: always\n    network_mode: host\n",
        ).unwrap();
        std::fs::write(
            dir.join("integration.yml"),
            "  integration:\n    image: asia-south2-docker.pkg.dev/puru-255206/puru1/puru-integration:{{INTEGRATION_TAG}}\n    container_name: integration\n    restart: always\n    network_mode: host\n",
        ).unwrap();
        std::fs::write(
            dir.join("frontend.yml"),
            "  frontend:\n    image: asia-south2-docker.pkg.dev/puru-255206/puru1/puru-hydrogen:{{HYDROGEN_TAG}}\n    container_name: frontend\n    restart: always\n    network_mode: host\n",
        ).unwrap();
    }

    /// Assemble compose using a specific fragments directory (test helper).
    /// Mirrors assemble_compose() logic but uses an explicit dir.
    fn assemble_compose_from_dir(
        frag_dir: &std::path::Path,
        modules: &ServiceModules,
    ) -> Result<String, NucleusError> {
        let core_path = frag_dir.join("core.yml");
        let mut assembled = std::fs::read_to_string(&core_path)
            .map_err(NucleusError::Io)?;
        assembled.push_str("\nservices:\n");

        let is_enabled = |key: &str| -> bool {
            match key {
                "auth" => modules.auth,
                "xenon" => modules.xenon,
                "has" => modules.has,
                "pacs" => modules.pacs,
                "argon" => modules.argon,
                "comm" => modules.comm,
                "realtime" => modules.realtime,
                "neon" => modules.neon,
                "mercury" => modules.mercury,
                "bridge" => modules.bridge,
                "integration" => modules.integration,
                "hydrogen" => modules.hydrogen,
                _ => false,
            }
        };

        for &(filename, module_key) in FRAGMENTS {
            let Some(key) = module_key else { continue };
            if !is_enabled(key) {
                continue;
            }
            let frag_path = frag_dir.join(filename);
            if !frag_path.exists() {
                continue;
            }
            let content = std::fs::read_to_string(&frag_path)
                .map_err(NucleusError::Io)?;
            assembled.push_str(&content);
            if !content.ends_with('\n') {
                assembled.push('\n');
            }
        }

        Ok(assembled)
    }

    #[test]
    fn test_assemble_compose_all_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        let frag_dir = tmp.path().join("fragments");
        setup_test_fragments(&frag_dir);

        let modules = ServiceModules::all_enabled();
        let result = assemble_compose_from_dir(&frag_dir, &modules).unwrap();

        // Should have version header
        assert!(result.contains("version: \"3.8\""));
        // Should have services: key
        assert!(result.contains("services:"));
        // All 11 services present
        assert!(result.contains("auth:"));
        assert!(result.contains("backend:"));
        assert!(result.contains("has:"));
        assert!(result.contains("pacs:"));
        assert!(result.contains("pathology:"));
        assert!(result.contains("comm_server:"));
        assert!(result.contains("realtime:"));
        assert!(result.contains("medical:"));
        assert!(result.contains("bridge:"));
        assert!(result.contains("integration:"));
        assert!(result.contains("frontend:"));
    }

    #[test]
    fn test_assemble_compose_disabled_modules() {
        let tmp = tempfile::tempdir().unwrap();
        let frag_dir = tmp.path().join("fragments");
        setup_test_fragments(&frag_dir);

        let mut modules = ServiceModules::all_enabled();
        modules.pacs = false;
        modules.neon = false;

        let result = assemble_compose_from_dir(&frag_dir, &modules).unwrap();

        // Enabled services present
        assert!(result.contains("backend:"));
        assert!(result.contains("has:"));
        assert!(result.contains("frontend:"));
        // Disabled services absent
        assert!(!result.contains("  pacs:"));
        assert!(!result.contains("  medical:"));
    }

    #[test]
    fn test_assemble_compose_only_core() {
        let tmp = tempfile::tempdir().unwrap();
        let frag_dir = tmp.path().join("fragments");
        setup_test_fragments(&frag_dir);

        let modules = ServiceModules {
            auth: false,
            xenon: false,
            has: false,
            pacs: false,
            argon: false,
            comm: false,
            realtime: false,
            neon: false,
            mercury: false,
            counter: false,
            bridge: false,
            integration: false,
            hydrogen: false,
        };

        let result = assemble_compose_from_dir(&frag_dir, &modules).unwrap();

        // Has version + services: but no service blocks
        assert!(result.contains("version: \"3.8\""));
        assert!(result.contains("services:"));
        assert!(!result.contains("backend:"));
        assert!(!result.contains("has:"));
        assert!(!result.contains("frontend:"));
    }
}
