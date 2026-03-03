//! GCS file download for message attachments.

use crate::error::NucleusError;

use super::types::DownloadResult;

/// Download an attachment from a `gs://` URL to the local filesystem.
pub async fn download_attachment(
    message_id: &str,
    attachment_index: u32,
    file_url: &str,
) -> Result<DownloadResult, NucleusError> {
    let (bucket, object_path) = parse_gs_url(file_url)?;

    // Extract filename from the object path
    let default_name = format!("attachment_{}", attachment_index);
    let filename = object_path
        .rsplit('/')
        .next()
        .unwrap_or(&default_name);

    // Build download directory: config_dir/downloads/{message_id}/
    let download_dir = crate::config::config_dir()
        .join("downloads")
        .join(message_id);
    std::fs::create_dir_all(&download_dir)?;

    let dest_path = download_dir.join(filename);

    // Get OAuth token via existing auth
    let token = get_access_token().await?;

    // Download from GCS JSON API
    let encoded_object = urlencoding::encode(&object_path);
    let url = format!(
        "https://storage.googleapis.com/storage/v1/b/{}/o/{}?alt=media",
        bucket, encoded_object
    );

    let http = reqwest::Client::new();
    let resp = http
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| NucleusError::GcsConnection(format!("GCS download failed: {}", e)))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Ok(DownloadResult {
            success: false,
            file_path: None,
            error: Some(format!("GCS download failed ({}): {}", status, body)),
        });
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| NucleusError::GcsConnection(format!("Failed to read response: {}", e)))?;

    std::fs::write(&dest_path, &bytes)?;

    Ok(DownloadResult {
        success: true,
        file_path: Some(dest_path.to_string_lossy().to_string()),
        error: None,
    })
}

/// Parse a `gs://bucket/path/to/object` URL into (bucket, object_path).
fn parse_gs_url(url: &str) -> Result<(String, String), NucleusError> {
    let stripped = url
        .strip_prefix("gs://")
        .ok_or_else(|| NucleusError::Validation(format!("Invalid GCS URL (must start with gs://): {}", url)))?;

    let slash_pos = stripped
        .find('/')
        .ok_or_else(|| NucleusError::Validation(format!("Invalid GCS URL (no object path): {}", url)))?;

    let bucket = stripped[..slash_pos].to_string();
    let object_path = stripped[slash_pos + 1..].to_string();

    if bucket.is_empty() || object_path.is_empty() {
        return Err(NucleusError::Validation(format!(
            "Invalid GCS URL (empty bucket or path): {}",
            url
        )));
    }

    Ok((bucket, object_path))
}

/// Get an OAuth2 access token using the configured service account credentials.
async fn get_access_token() -> Result<String, NucleusError> {
    let config = crate::config::load_config()?;
    let creds_path = config
        .gcs_credentials_path
        .as_deref()
        .filter(|p| !p.is_empty())
        .ok_or_else(|| {
            NucleusError::FirestoreAuth("GCS credentials path not configured.".into())
        })?;

    let token_source = crate::firestore::auth::create_token_source(creds_path).await?;
    crate::firestore::auth::get_bearer_token(token_source.as_ref()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_gs_url_valid() {
        let (bucket, path) = parse_gs_url("gs://my-bucket/path/to/file.txt").unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(path, "path/to/file.txt");
    }

    #[test]
    fn test_parse_gs_url_no_prefix() {
        assert!(parse_gs_url("https://example.com/file").is_err());
    }

    #[test]
    fn test_parse_gs_url_no_path() {
        assert!(parse_gs_url("gs://bucket").is_err());
    }

    #[test]
    fn test_parse_gs_url_empty_bucket() {
        assert!(parse_gs_url("gs:///path").is_err());
    }
}
