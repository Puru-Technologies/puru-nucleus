//! Environment file parsing and hospital detection
//!
//! Reads `.env` files from the `env/` directory alongside `docker-compose.yml`
//! to extract hospital configuration, database credentials, and RabbitMQ settings.
//! Also detects network mode (host vs bridge) from the compose file.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::error::NucleusError;

// ── Types ────────────────────────────────────────────────────────────────────

/// Parsed hospital information from env files
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HospitalInfo {
    pub code: String,
    pub name: String,
    pub server_ip: String,
    pub address: String,
    pub logo_url: String,
    pub registration_number: String,
}

/// Parsed database credentials from env files
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DatabaseCredentials {
    pub username: String,
    pub password: String,
    pub host: String,
    pub port: u16,
    pub pool_size: u32,
}

/// Parsed RabbitMQ credentials from env files
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RabbitMQCredentials {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
}

/// Network mode detected from docker-compose.yml
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NetworkMode {
    Host,
    Bridge,
    Unknown,
}

/// Complete environment detection result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvDetectionResult {
    pub env_dir: Option<String>,
    pub hospital_info: Option<HospitalInfo>,
    pub database: Option<DatabaseCredentials>,
    pub rabbitmq: Option<RabbitMQCredentials>,
    pub network_mode: NetworkMode,
    pub platform: String,
    pub env_files_found: Vec<String>,
}

// ── Env file parser ──────────────────────────────────────────────────────────

/// Parse a single `.env` file into a key-value map.
/// Handles `key=value` format, ignores comments (#) and blank lines.
/// Supports values with `=` in them (only splits on first `=`).
fn parse_env_file(path: &Path) -> Result<HashMap<String, String>, NucleusError> {
    let content = std::fs::read_to_string(path)?;
    let mut map = HashMap::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Split on first '=' only
        if let Some(pos) = trimmed.find('=') {
            let key = trimmed[..pos].trim().to_string();
            let value = trimmed[pos + 1..].trim().to_string();

            // Remove surrounding quotes if present
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(&value)
                .to_string();

            if !key.is_empty() {
                map.insert(key, value);
            }
        }
    }

    Ok(map)
}

// ── Hospital info extraction ────────────────────────────────────────────────

/// Extract hospital info from general.env
fn extract_hospital_info(env: &HashMap<String, String>) -> HospitalInfo {
    HospitalInfo {
        code: env
            .get("hospital.short.code")
            .or_else(|| env.get("puru.hospital.code"))
            .cloned()
            .unwrap_or_default(),
        name: env
            .get("puru.hospital.name.en")
            .cloned()
            .unwrap_or_default(),
        server_ip: env.get("puru.server.ip").cloned().unwrap_or_default(),
        address: env
            .get("puru.hospital.line2.en")
            .cloned()
            .unwrap_or_default(),
        logo_url: env
            .get("hospital.logo.url")
            .cloned()
            .unwrap_or_default(),
        registration_number: env
            .get("puru.hospital.reg.no")
            .cloned()
            .unwrap_or_default(),
    }
}

/// Extract database credentials from database.env
fn extract_database_credentials(env: &HashMap<String, String>) -> DatabaseCredentials {
    DatabaseCredentials {
        username: env
            .get("spring.datasource.username")
            .cloned()
            .unwrap_or_default(),
        password: env
            .get("spring.datasource.password")
            .cloned()
            .unwrap_or_default(),
        host: "localhost".to_string(), // default; may be overridden by compose detection
        port: 3306,
        pool_size: env
            .get("spring.datasource.hikari.maximumpoolsize")
            .and_then(|v| v.parse().ok())
            .unwrap_or(5),
    }
}

/// Extract RabbitMQ credentials from rabbitmq.env
fn extract_rabbitmq_credentials(env: &HashMap<String, String>) -> RabbitMQCredentials {
    RabbitMQCredentials {
        host: env
            .get("spring.rabbitmq.host")
            .cloned()
            .unwrap_or_else(|| "localhost".to_string()),
        port: env
            .get("spring.rabbitmq.port")
            .and_then(|v| v.parse().ok())
            .unwrap_or(5672),
        username: env
            .get("spring.rabbitmq.username")
            .cloned()
            .unwrap_or_default(),
        password: env
            .get("spring.rabbitmq.password")
            .cloned()
            .unwrap_or_default(),
    }
}

// ── Network mode detection ──────────────────────────────────────────────────

/// Detect network mode from docker-compose.yml content.
/// Checks for `network_mode: "host"` (Linux) vs explicit networks (Windows bridge).
fn detect_network_mode(compose_path: &Path) -> NetworkMode {
    let content = match std::fs::read_to_string(compose_path) {
        Ok(c) => c,
        Err(_) => return NetworkMode::Unknown,
    };

    // Check for host mode — any service with network_mode: "host" or network_mode: host
    if content.contains("network_mode:") {
        let lower = content.to_lowercase();
        if lower.contains("network_mode: \"host\"") || lower.contains("network_mode: host") {
            return NetworkMode::Host;
        }
    }

    // Check for explicit bridge networks
    if content.contains("networks:") {
        // Look for top-level `networks:` definition (bridge mode)
        let lines: Vec<&str> = content.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed == "networks:" && i > 0 {
                // Check indentation — top-level `networks:` has no leading spaces
                if !line.starts_with(' ') && !line.starts_with('\t') {
                    return NetworkMode::Bridge;
                }
            }
        }
    }

    // Check for host.docker.internal (Windows bridge pattern)
    if content.contains("host.docker.internal") {
        return NetworkMode::Bridge;
    }

    NetworkMode::Unknown
}

// ── Compose inline environment parsing ──────────────────────────────────

/// Parse environment variables directly from docker-compose.yml.
/// Handles both `KEY=VALUE` list format and `env_file:` references.
/// This is the fallback when no `env/` directory exists.
fn parse_compose_inline_env(compose_path: &Path) -> HashMap<String, String> {
    let content = match std::fs::read_to_string(compose_path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };

    let mut vars = HashMap::new();
    let mut in_environment = false;
    let mut env_indent = 0;

    for line in content.lines() {
        let trimmed = line.trim();

        // Detect `environment:` block
        if trimmed == "environment:" || trimmed.starts_with("environment:") {
            in_environment = true;
            // Calculate indentation of the environment key
            env_indent = line.len() - line.trim_start().len();
            continue;
        }

        if in_environment {
            let current_indent = line.len() - line.trim_start().len();

            // If we hit a line at same or lesser indent, we've left the environment block
            if !trimmed.is_empty() && current_indent <= env_indent && !trimmed.starts_with('-') {
                in_environment = false;
                continue;
            }

            // Parse `- KEY=VALUE` or `KEY: VALUE` or `- KEY: VALUE`
            let entry = trimmed.trim_start_matches('-').trim();
            if entry.is_empty() || entry.starts_with('#') {
                continue;
            }

            if let Some(pos) = entry.find('=') {
                let key = entry[..pos].trim().to_string();
                let value = entry[pos + 1..].trim().to_string();
                // Remove surrounding quotes
                let value = value
                    .strip_prefix('"').and_then(|v| v.strip_suffix('"'))
                    .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                    .unwrap_or(&value)
                    .to_string();
                if !key.is_empty() {
                    vars.insert(key, value);
                }
            }
        }
    }

    vars
}

// ── Env directory discovery ─────────────────────────────────────────────────

/// Find the env/ directory relative to a docker-compose.yml file.
/// Looks in the same directory as the compose file.
fn find_env_dir(compose_path: &Path) -> Option<PathBuf> {
    let parent = compose_path.parent()?;
    let env_dir = parent.join("env");
    if env_dir.is_dir() {
        Some(env_dir)
    } else {
        None
    }
}

/// List which env files exist in a directory.
fn list_env_files(env_dir: &Path) -> Vec<String> {
    let expected = [
        "general.env",
        "database.env",
        "rabbitmq.env",
        "has.env",
        "mail.env",
        "pacs.env",
        "database-neon.env",
        "rabbitmq-neon.env",
    ];

    expected
        .iter()
        .filter(|f| env_dir.join(f).exists())
        .map(|f| f.to_string())
        .collect()
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Parse environment files from a docker-compose.yml's sibling `env/` directory.
/// Returns hospital info, database creds, RabbitMQ creds, and network mode.
pub fn detect_environment(compose_path: &str) -> Result<EnvDetectionResult, NucleusError> {
    let compose = Path::new(compose_path);

    if !compose.exists() {
        return Err(NucleusError::NotFound(format!(
            "Compose file not found: {}",
            compose_path
        )));
    }

    let network_mode = detect_network_mode(compose);

    let platform = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "unknown"
    }
    .to_string();

    let env_dir = find_env_dir(compose);

    let (env_dir_str, hospital_info, database, rabbitmq, env_files_found) = match env_dir {
        Some(ref dir) => {
            let files = list_env_files(dir);

            // Parse general.env
            let hospital = dir
                .join("general.env")
                .exists()
                .then(|| parse_env_file(&dir.join("general.env")))
                .and_then(|r| r.ok())
                .map(|env| extract_hospital_info(&env));

            // Parse database.env
            let db = dir
                .join("database.env")
                .exists()
                .then(|| parse_env_file(&dir.join("database.env")))
                .and_then(|r| r.ok())
                .map(|env| extract_database_credentials(&env));

            // Parse rabbitmq.env
            let rmq = dir
                .join("rabbitmq.env")
                .exists()
                .then(|| parse_env_file(&dir.join("rabbitmq.env")))
                .and_then(|r| r.ok())
                .map(|env| extract_rabbitmq_credentials(&env));

            (
                Some(dir.to_string_lossy().to_string()),
                hospital,
                db,
                rmq,
                files,
            )
        }
        None => {
            // Fallback: parse inline environment variables from docker-compose.yml
            let inline_vars = parse_compose_inline_env(compose);
            if inline_vars.is_empty() {
                (None, None, None, None, vec![])
            } else {
                tracing::info!(
                    "No env/ directory found, parsed {} inline variables from docker-compose.yml",
                    inline_vars.len()
                );
                let hospital = Some(extract_hospital_info(&inline_vars));
                let db = Some(extract_database_credentials(&inline_vars));
                let rmq = Some(extract_rabbitmq_credentials(&inline_vars));
                (None, hospital, db, rmq, vec!["(inline in docker-compose.yml)".to_string()])
            }
        }
    };

    Ok(EnvDetectionResult {
        env_dir: env_dir_str,
        hospital_info,
        database,
        rabbitmq,
        network_mode,
        platform,
        env_files_found,
    })
}

/// A field where the existing config value differs from the detected env value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigMismatch {
    pub field: String,
    pub config_value: String,
    pub detected_value: String,
}

/// Result of applying detected environment to config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyResult {
    pub applied: Vec<String>,
    pub mismatches: Vec<ConfigMismatch>,
}

/// Parse env files and apply detected values to NucleusConfig.
///
/// - Empty/default config fields are filled from detected values.
/// - Non-empty fields that differ are reported as mismatches but **not**
///   overwritten — the user must fix them manually in Settings.
pub fn apply_detected_config(
    config: &mut crate::config::NucleusConfig,
    detection: &EnvDetectionResult,
) -> ApplyResult {
    let mut applied = Vec::new();
    let mut mismatches = Vec::new();

    if let Some(ref hospital) = detection.hospital_info {
        if !hospital.code.is_empty() {
            if config.hospital_code.is_empty() {
                config.hospital_code = hospital.code.clone();
                applied.push("hospital_code".to_string());
            } else if config.hospital_code != hospital.code {
                mismatches.push(ConfigMismatch {
                    field: "hospital_code".to_string(),
                    config_value: config.hospital_code.clone(),
                    detected_value: hospital.code.clone(),
                });
            }
        }
        if !hospital.server_ip.is_empty() {
            if config.server_ip.is_empty() {
                config.server_ip = hospital.server_ip.clone();
                applied.push("server_ip".to_string());
            } else if config.server_ip != hospital.server_ip {
                mismatches.push(ConfigMismatch {
                    field: "server_ip".to_string(),
                    config_value: config.server_ip.clone(),
                    detected_value: hospital.server_ip.clone(),
                });
            }
        }
    }

    if let Some(ref db) = detection.database {
        if !db.username.is_empty() {
            if config.mysql_user == "root" || config.mysql_user.is_empty() {
                config.mysql_user = db.username.clone();
                applied.push("mysql_user".to_string());
            } else if config.mysql_user != db.username {
                mismatches.push(ConfigMismatch {
                    field: "mysql_user".to_string(),
                    config_value: config.mysql_user.clone(),
                    detected_value: db.username.clone(),
                });
            }
        }
        if !db.password.is_empty() {
            if config.mysql_password.is_empty() {
                config.mysql_password = db.password.clone();
                applied.push("mysql_password".to_string());
            } else if config.mysql_password != db.password {
                mismatches.push(ConfigMismatch {
                    field: "mysql_password".to_string(),
                    config_value: "(existing)".to_string(),
                    detected_value: "(from env file)".to_string(),
                });
            }
        }
    }

    ApplyResult { applied, mismatches }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_env_file_basic() {
        let dir = std::env::temp_dir().join("puru-test-env");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.env");
        std::fs::write(
            &path,
            "# comment\nTZ=Asia/Kolkata\npuru.server.ip=192.168.0.111\n\nhospital.short.code=BTCT\n",
        )
        .unwrap();

        let env = parse_env_file(&path).unwrap();
        assert_eq!(env.get("TZ").unwrap(), "Asia/Kolkata");
        assert_eq!(env.get("puru.server.ip").unwrap(), "192.168.0.111");
        assert_eq!(env.get("hospital.short.code").unwrap(), "BTCT");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_env_with_equals_in_value() {
        let dir = std::env::temp_dir().join("puru-test-env2");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test2.env");
        std::fs::write(
            &path,
            "spring.datasource.url=jdbc:mysql://localhost:3306/puru_im\n",
        )
        .unwrap();

        let env = parse_env_file(&path).unwrap();
        assert_eq!(
            env.get("spring.datasource.url").unwrap(),
            "jdbc:mysql://localhost:3306/puru_im"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_extract_hospital_info() {
        let mut env = HashMap::new();
        env.insert("hospital.short.code".to_string(), "BTCT".to_string());
        env.insert(
            "puru.hospital.name.en".to_string(),
            "Bhagyoday Tirth".to_string(),
        );
        env.insert("puru.server.ip".to_string(), "192.168.0.111".to_string());

        let info = extract_hospital_info(&env);
        assert_eq!(info.code, "BTCT");
        assert_eq!(info.name, "Bhagyoday Tirth");
        assert_eq!(info.server_ip, "192.168.0.111");
    }

    #[test]
    fn test_network_mode_host() {
        let dir = std::env::temp_dir().join("puru-test-compose-host");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("docker-compose.yml");
        std::fs::write(
            &path,
            "services:\n  backend:\n    image: puru-xenon\n    network_mode: \"host\"\n",
        )
        .unwrap();

        assert_eq!(detect_network_mode(&path), NetworkMode::Host);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_network_mode_bridge() {
        let dir = std::env::temp_dir().join("puru-test-compose-bridge");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("docker-compose.yml");
        std::fs::write(
            &path,
            "services:\n  backend:\n    image: puru-xenon\n    ports:\n      - '8081:8081'\nnetworks:\n  puru-net:\n    driver: bridge\n",
        )
        .unwrap();

        assert_eq!(detect_network_mode(&path), NetworkMode::Bridge);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_compose_inline_env() {
        let dir = std::env::temp_dir().join("puru-test-compose-inline");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("docker-compose.yml");
        std::fs::write(
            &path,
            r#"services:
  puru-has:
    image: puru-has:2.0
    environment:
      - hospital.short.code=BTCT
      - puru.server.ip=192.168.1.100
      - spring.datasource.username=puru_admin
      - spring.datasource.password=secret123
      - spring.rabbitmq.host=localhost
    ports:
      - '8082:8082'
"#,
        )
        .unwrap();

        let vars = parse_compose_inline_env(&path);
        assert_eq!(vars.get("hospital.short.code").unwrap(), "BTCT");
        assert_eq!(vars.get("puru.server.ip").unwrap(), "192.168.1.100");
        assert_eq!(vars.get("spring.datasource.username").unwrap(), "puru_admin");
        assert_eq!(vars.get("spring.datasource.password").unwrap(), "secret123");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
