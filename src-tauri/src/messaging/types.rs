//! Data models for the admin-to-hospital messaging system.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    SaKey,
    ConfigUpdate,
    Alert,
    Info,
    File,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageAttachment {
    pub filename: String,
    pub file_url: String,
    pub file_size: Option<u64>,
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HospitalMessage {
    pub id: String,
    pub message_type: MessageType,
    pub priority: Priority,
    pub subject: String,
    pub body: Option<String>,
    pub attachments: Vec<MessageAttachment>,
    pub read: bool,
    pub created_at: String,
    pub created_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadResult {
    pub success: bool,
    pub file_path: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileActionResult {
    pub success: bool,
    pub message: String,
}
