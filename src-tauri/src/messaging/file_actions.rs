//! File actions — copying config/data files and installing certificates.

use std::path::Path;

use crate::error::NucleusError;

use super::types::FileActionResult;

/// Copy a downloaded file (JSON, SQL, etc.) to the config directory.
///
/// Validates JSON syntax for `.json` files; other supported types are copied as-is.
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
        "json" => copy_validated_json(path).await,
        "sql" => copy_file_to_config_dir(path).await,
        _ => Ok(FileActionResult {
            success: false,
            message: format!(
                "Unsupported file extension: .{}. Expected .json or .sql",
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

/// Copy a JSON file to the config directory after validating its syntax.
async fn copy_validated_json(path: &Path) -> Result<FileActionResult, NucleusError> {
    let content = std::fs::read_to_string(path)?;
    let _: serde_json::Value = serde_json::from_str(&content)?;

    copy_file_to_config_dir(path).await
}

/// Copy a file as-is to the config directory.
async fn copy_file_to_config_dir(path: &Path) -> Result<FileActionResult, NucleusError> {
    let filename = path
        .file_name()
        .ok_or_else(|| NucleusError::Validation("Invalid file path".into()))?;
    let dest = crate::config::config_dir().join(filename);

    std::fs::copy(path, &dest)?;

    Ok(FileActionResult {
        success: true,
        message: format!("File copied to {}", dest.to_string_lossy()),
    })
}
