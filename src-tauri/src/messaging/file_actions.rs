//! File actions — applying config files and installing certificates.

use std::path::Path;

use crate::error::NucleusError;

use super::types::FileActionResult;

/// Apply a downloaded config file to the nucleus configuration.
///
/// - `.toml` files are merged into the existing `NucleusConfig`
/// - `.json` files are copied to the config directory as service-specific config
pub async fn apply_config_file(file_path: &str) -> Result<FileActionResult, NucleusError> {
    let path = Path::new(file_path);

    if !path.exists() {
        return Ok(FileActionResult {
            success: false,
            message: format!("File not found: {}", file_path),
        });
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    match ext {
        "toml" => apply_toml_config(path).await,
        "json" => copy_json_config(path).await,
        _ => Ok(FileActionResult {
            success: false,
            message: format!(
                "Unsupported config file extension: .{}. Expected .toml or .json",
                ext
            ),
        }),
    }
}

/// Install a certificate file to the certs directory.
///
/// Accepts `.pem`, `.crt`, `.cer`, and `.key` files.
pub async fn install_certificate(file_path: &str) -> Result<FileActionResult, NucleusError> {
    let path = Path::new(file_path);

    if !path.exists() {
        return Ok(FileActionResult {
            success: false,
            message: format!("File not found: {}", file_path),
        });
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let valid_extensions = ["pem", "crt", "cer", "key"];
    if !valid_extensions.contains(&ext) {
        return Ok(FileActionResult {
            success: false,
            message: format!(
                "Unsupported certificate extension: .{}. Expected .pem, .crt, .cer, or .key",
                ext
            ),
        });
    }

    let certs_dir = crate::config::config_dir().join("certs");
    std::fs::create_dir_all(&certs_dir)?;

    let filename = path
        .file_name()
        .ok_or_else(|| NucleusError::Validation("Invalid file path".into()))?;
    let dest = certs_dir.join(filename);

    std::fs::copy(path, &dest)?;

    Ok(FileActionResult {
        success: true,
        message: format!("Certificate installed to {}", dest.to_string_lossy()),
    })
}

/// Merge a TOML config file into the existing NucleusConfig.
async fn apply_toml_config(path: &Path) -> Result<FileActionResult, NucleusError> {
    let content = std::fs::read_to_string(path)?;

    // Parse the incoming TOML to validate it
    let incoming: toml::Value = toml::from_str(&content).map_err(|e| {
        NucleusError::InvalidConfig(format!("Invalid TOML config: {}", e))
    })?;

    // Load existing config, serialize to TOML value, merge, deserialize back
    let existing = crate::config::load_config()?;
    let mut existing_value: toml::Value =
        toml::Value::try_from(&existing).map_err(|e| {
            NucleusError::InvalidConfig(format!("Failed to serialize existing config: {}", e))
        })?;

    // Merge incoming into existing (top-level keys only)
    if let (toml::Value::Table(ref mut base), toml::Value::Table(ref overlay)) =
        (&mut existing_value, &incoming)
    {
        for (key, value) in overlay {
            base.insert(key.clone(), value.clone());
        }
    }

    // Deserialize the merged config back
    let merged: crate::config::NucleusConfig = existing_value.try_into().map_err(|e| {
        NucleusError::InvalidConfig(format!("Merged config is invalid: {}", e))
    })?;

    crate::config::save_config(&merged)?;

    Ok(FileActionResult {
        success: true,
        message: "Configuration file applied and merged successfully.".into(),
    })
}

/// Copy a JSON config file to the config directory as service-specific config.
async fn copy_json_config(path: &Path) -> Result<FileActionResult, NucleusError> {
    // Validate JSON
    let content = std::fs::read_to_string(path)?;
    let _: serde_json::Value = serde_json::from_str(&content)?;

    let filename = path
        .file_name()
        .ok_or_else(|| NucleusError::Validation("Invalid file path".into()))?;
    let dest = crate::config::config_dir().join(filename);

    std::fs::copy(path, &dest)?;

    Ok(FileActionResult {
        success: true,
        message: format!("Config file copied to {}", dest.to_string_lossy()),
    })
}
