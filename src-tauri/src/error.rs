//! Custom error types for puru-dc

use thiserror::Error;

#[derive(Error, Debug)]
pub enum NucleusError {
    // Validation errors
    #[error("Validation failed: {0}")]
    Validation(String),

    // Network/connectivity errors
    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Failed to connect to GCS: {0}")]
    GcsConnection(String),

    #[error("Failed to connect to Firestore: {0}")]
    FirestoreConnection(String),

    #[error("Firestore query error: {0}")]
    FirestoreQuery(String),

    #[error("Firestore auth error: {0}")]
    FirestoreAuth(String),

    // Permission errors
    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    // Not found errors
    #[error("{0} not found")]
    NotFound(String),

    // System errors
    #[error("Docker not running")]
    DockerNotRunning,

    #[error("Docker error: {0}")]
    Docker(String),

    #[error("MySQL connection failed: {0}")]
    MySqlConnection(String),

    #[error("Disk space low: {available_gb}GB available, {required_gb}GB required")]
    DiskSpaceLow { available_gb: u64, required_gb: u64 },

    // Config errors
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("License expired on {0}")]
    LicenseExpired(String),

    #[error("License invalid: {0}")]
    LicenseInvalid(String),

    // IO errors
    #[error("File operation failed: {0}")]
    Io(#[from] std::io::Error),

    // JSON errors
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    // Catch-all
    #[error("Unexpected error: {0}")]
    Internal(String),
}

impl NucleusError {
    pub fn severity(&self) -> Severity {
        match self {
            Self::Validation(_) => Severity::Warning,
            Self::NotFound(_) => Severity::Warning,
            Self::LicenseExpired(_) => Severity::Warning,
            Self::DockerNotRunning => Severity::Critical,
            Self::DiskSpaceLow { .. } => Severity::Critical,
            _ => Severity::Error,
        }
    }

    pub fn user_message(&self) -> String {
        match self {
            Self::Network(_) => "Network connection failed. Check your internet.".into(),
            Self::GcsConnection(_) => "Cannot connect to cloud storage. Check credentials.".into(),
            Self::FirestoreConnection(_) => "Cannot connect to Firestore. Check internet and credentials.".into(),
            Self::FirestoreQuery(_) => "Failed to read data from cloud. Try again.".into(),
            Self::FirestoreAuth(_) => "Cloud authentication failed. Check credentials in Settings.".into(),
            Self::DockerNotRunning => "Docker is not running. Please start Docker.".into(),
            Self::MySqlConnection(_) => "Cannot connect to MySQL. Check database settings.".into(),
            Self::LicenseExpired(date) => format!("License expired on {}. Contact support.", date),
            Self::DiskSpaceLow { available_gb, required_gb } => {
                format!(
                    "Disk space low: {}GB available, {}GB required.",
                    available_gb, required_gb
                )
            }
            _ => self.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
    Critical,
}

/// Result type alias for NucleusError
pub type NucleusResult<T> = Result<T, NucleusError>;

// Convert NucleusError to String for Tauri commands
impl From<NucleusError> for String {
    fn from(err: NucleusError) -> Self {
        err.user_message()
    }
}

// For Tauri's invoke handler
impl serde::Serialize for NucleusError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.user_message())
    }
}
