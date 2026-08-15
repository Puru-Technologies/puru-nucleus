//! Token management for Firestore REST API via google-cloud-auth.

use google_cloud_auth::credentials::CredentialsFile;
use google_cloud_auth::project::{create_token_source_from_credentials, Config};
use google_cloud_auth::token_source::TokenSource;

use crate::error::NucleusError;

const FIRESTORE_SCOPE: &str = "https://www.googleapis.com/auth/datastore";
/// Read-only GCS scope for downloading objects (inbox attachments, files). Note:
/// a Firestore (`datastore`) token yields a 403 "Insufficient Permission" against
/// Storage, so Storage requests must use this scope.
const STORAGE_READ_SCOPE: &str = "https://www.googleapis.com/auth/devstorage.read_only";

/// Create a token source from a service account key file on disk.
pub async fn create_token_source(
    credentials_path: &str,
) -> Result<Box<dyn TokenSource>, NucleusError> {
    create_scoped_token_source(credentials_path, FIRESTORE_SCOPE).await
}

/// Create a GCS-storage-read-scoped token source (for downloading objects).
pub async fn create_storage_token_source(
    credentials_path: &str,
) -> Result<Box<dyn TokenSource>, NucleusError> {
    create_scoped_token_source(credentials_path, STORAGE_READ_SCOPE).await
}

/// Create a token source for the given OAuth scope from a service account key.
async fn create_scoped_token_source(
    credentials_path: &str,
    scope: &str,
) -> Result<Box<dyn TokenSource>, NucleusError> {
    let credentials = CredentialsFile::new_from_file(credentials_path.to_string())
        .await
        .map_err(|e| NucleusError::FirestoreAuth(format!("Failed to load credentials: {}", e)))?;

    let config = Config {
        scopes: Some(std::slice::from_ref(&scope)),
        audience: None,
        sub: None,
    };

    let ts = create_token_source_from_credentials(&credentials, &config)
        .await
        .map_err(|e| {
            NucleusError::FirestoreAuth(format!("Failed to create token source: {}", e))
        })?;

    Ok(ts)
}

/// Get a bearer token string from an existing token source.
pub async fn get_bearer_token(source: &dyn TokenSource) -> Result<String, NucleusError> {
    let token = source
        .token()
        .await
        .map_err(|e| NucleusError::FirestoreAuth(format!("Failed to get token: {}", e)))?;
    Ok(token.access_token)
}
