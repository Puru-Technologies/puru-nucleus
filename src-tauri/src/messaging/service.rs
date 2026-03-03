//! Message fetching and read-marking for the hospital inbox.

use crate::error::NucleusError;
use crate::firestore::convert::*;
use crate::firestore::types::FirestoreDocument;

use super::types::{HospitalMessage, MessageAttachment, MessageType, Priority};

/// Fetch messages from the hospital's inbox subcollection.
pub async fn get_messages(limit: u32) -> Result<Vec<HospitalMessage>, NucleusError> {
    let config = crate::config::load_config()?;
    if config.hospital_code.is_empty() {
        return Err(NucleusError::Validation(
            "Hospital code not configured. Activate your license first.".into(),
        ));
    }

    let client = crate::firestore::FirestoreClient::new_from_config().await?;
    let docs = client.list_inbox(&config.hospital_code, limit).await?;

    let mut messages = Vec::with_capacity(docs.len());
    for doc in &docs {
        match convert_doc_to_message(doc) {
            Ok(msg) => messages.push(msg),
            Err(e) => {
                tracing::warn!("Skipping malformed inbox message: {}", e);
            }
        }
    }

    Ok(messages)
}

/// Get the count of unread messages.
pub async fn get_unread_count() -> Result<u32, NucleusError> {
    let messages = get_messages(100).await?;
    let count = messages.iter().filter(|m| !m.read).count() as u32;
    Ok(count)
}

/// Mark a single message as read.
pub async fn mark_message_read(message_id: &str) -> Result<(), NucleusError> {
    let config = crate::config::load_config()?;
    if config.hospital_code.is_empty() {
        return Err(NucleusError::Validation(
            "Hospital code not configured.".into(),
        ));
    }

    let client = crate::firestore::FirestoreClient::new_from_config().await?;
    client
        .mark_message_read(&config.hospital_code, message_id)
        .await
}

/// Convert a Firestore document into a `HospitalMessage`.
fn convert_doc_to_message(doc: &FirestoreDocument) -> Result<HospitalMessage, NucleusError> {
    let id = doc
        .name
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string();

    let message_type = parse_message_type(
        &doc.fields
            .get("message_type")
            .map(|v| get_string(v).unwrap_or_default())
            .unwrap_or_default(),
    );

    let priority = parse_priority(
        &doc.fields
            .get("priority")
            .map(|v| get_string(v).unwrap_or_default())
            .unwrap_or_else(|| "normal".to_string()),
    );

    let subject = doc
        .fields
        .get("subject")
        .map(|v| get_string(v).unwrap_or_default())
        .unwrap_or_default();

    let body = doc.fields.get("body").and_then(get_optional_string);

    let read = doc
        .fields
        .get("read")
        .and_then(|v| get_bool(v).ok())
        .unwrap_or(false);

    let created_at = doc
        .fields
        .get("created_at")
        .map(|v| {
            get_timestamp(v)
                .or_else(|_| get_string(v))
                .unwrap_or_default()
        })
        .unwrap_or_default();

    let created_by = doc.fields.get("created_by").and_then(get_optional_string);

    // Parse attachments array
    let attachment_values =
        get_array_values(&serde_json::Value::Object(doc.fields.clone().into_iter().collect()), "attachments");

    let mut attachments = Vec::new();
    for att_val in &attachment_values {
        if let Ok(fields) = get_map_fields(att_val) {
            let filename = fields
                .get("filename")
                .map(|v| get_string(v).unwrap_or_default())
                .unwrap_or_default();
            let file_url = fields
                .get("file_url")
                .map(|v| get_string(v).unwrap_or_default())
                .unwrap_or_default();
            let file_size = fields
                .get("file_size")
                .and_then(get_optional_integer)
                .map(|v| v as u64);
            let mime_type = fields.get("mime_type").and_then(get_optional_string);

            attachments.push(MessageAttachment {
                filename,
                file_url,
                file_size,
                mime_type,
            });
        }
    }

    Ok(HospitalMessage {
        id,
        message_type,
        priority,
        subject,
        body,
        attachments,
        read,
        created_at,
        created_by,
    })
}

fn parse_message_type(s: &str) -> MessageType {
    match s {
        "sa_key" => MessageType::SaKey,
        "config_update" => MessageType::ConfigUpdate,
        "alert" => MessageType::Alert,
        "file" => MessageType::File,
        _ => MessageType::Info,
    }
}

fn parse_priority(s: &str) -> Priority {
    match s {
        "low" => Priority::Low,
        "high" => Priority::High,
        "critical" => Priority::Critical,
        _ => Priority::Normal,
    }
}
