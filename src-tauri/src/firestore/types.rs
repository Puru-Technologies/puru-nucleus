//! Firestore REST API wire format types

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A Firestore document as returned by the REST API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FirestoreDocument {
    /// Full resource name, e.g. "projects/puru-255206/databases/(default)/documents/hospital/ABC"
    #[serde(default)]
    pub name: String,
    /// Field name → Firestore typed value (e.g. {"stringValue": "hello"})
    #[serde(default)]
    pub fields: HashMap<String, serde_json::Value>,
}

/// Response from `:runQuery`
#[derive(Debug, Clone, Deserialize)]
pub struct RunQueryResponse {
    pub document: Option<FirestoreDocument>,
}

/// Response from list (GET on a collection)
#[derive(Debug, Clone, Deserialize)]
pub struct ListDocumentsResponse {
    pub documents: Option<Vec<FirestoreDocument>>,
}
