//! Low-level Firestore REST API operations.

use reqwest::Client;
use serde_json::{json, Value};

use crate::error::NucleusError;

use super::types::{FirestoreDocument, ListDocumentsResponse, RunQueryResponse};

const BASE_URL: &str =
    "https://firestore.googleapis.com/v1/projects/puru-255206/databases/(default)/documents";

/// GET a single document by path (e.g. "hospital/ABC").
pub async fn get_document(
    client: &Client,
    token: &str,
    path: &str,
) -> Result<FirestoreDocument, NucleusError> {
    let url = format!("{}/{}", BASE_URL, path);

    let resp = client
        .get(&url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| NucleusError::FirestoreConnection(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(NucleusError::FirestoreQuery(format!(
            "GET {} failed ({}): {}",
            path, status, body
        )));
    }

    resp.json::<FirestoreDocument>()
        .await
        .map_err(|e| NucleusError::FirestoreQuery(format!("Failed to parse document: {}", e)))
}

/// Run a structured query to find documents where `field == value` (string equality).
pub async fn query_collection(
    client: &Client,
    token: &str,
    collection: &str,
    field: &str,
    value: &str,
) -> Result<Vec<FirestoreDocument>, NucleusError> {
    let url = format!("{}:runQuery", BASE_URL);

    let body = json!({
        "structuredQuery": {
            "from": [{"collectionId": collection}],
            "where": {
                "fieldFilter": {
                    "field": {"fieldPath": field},
                    "op": "EQUAL",
                    "value": {"stringValue": value}
                }
            }
        }
    });

    let resp = client
        .post(&url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| NucleusError::FirestoreConnection(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(NucleusError::FirestoreQuery(format!(
            "Query on {} failed ({}): {}",
            collection, status, body
        )));
    }

    let results: Vec<RunQueryResponse> = resp
        .json()
        .await
        .map_err(|e| NucleusError::FirestoreQuery(format!("Failed to parse query result: {}", e)))?;

    Ok(results.into_iter().filter_map(|r| r.document).collect())
}

/// Run a structured query on a subcollection under a parent document.
/// Finds documents where `field == value` (string equality), ordered by `created_at ASC`.
pub async fn query_subcollection(
    client: &Client,
    token: &str,
    parent: &str,      // e.g. "hospital/ABC"
    collection: &str,  // e.g. "commands"
    field: &str,       // e.g. "status"
    value: &str,       // e.g. "pending"
) -> Result<Vec<FirestoreDocument>, NucleusError> {
    let url = format!("{}/{}:runQuery", BASE_URL, parent);

    let body = json!({
        "structuredQuery": {
            "from": [{"collectionId": collection}],
            "where": {
                "fieldFilter": {
                    "field": {"fieldPath": field},
                    "op": "EQUAL",
                    "value": {"stringValue": value}
                }
            },
            "orderBy": [{
                "field": {"fieldPath": "created_at"},
                "direction": "ASCENDING"
            }]
        }
    });

    let resp = client
        .post(&url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| NucleusError::FirestoreConnection(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(NucleusError::FirestoreQuery(format!(
            "Query on {}/{} failed ({}): {}",
            parent, collection, status, body
        )));
    }

    let results: Vec<RunQueryResponse> = resp
        .json()
        .await
        .map_err(|e| NucleusError::FirestoreQuery(format!("Failed to parse query result: {}", e)))?;

    Ok(results.into_iter().filter_map(|r| r.document).collect())
}

/// Run a structured query on a subcollection, ordered by a field with a limit.
/// Returns all documents ordered descending.
pub async fn query_subcollection_ordered(
    client: &Client,
    token: &str,
    parent: &str,
    collection: &str,
    order_field: &str,
    limit: u32,
) -> Result<Vec<FirestoreDocument>, NucleusError> {
    let url = format!("{}/{}:runQuery", BASE_URL, parent);

    let body = json!({
        "structuredQuery": {
            "from": [{"collectionId": collection}],
            "orderBy": [{
                "field": {"fieldPath": order_field},
                "direction": "DESCENDING"
            }],
            "limit": limit
        }
    });

    let resp = client
        .post(&url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| NucleusError::FirestoreConnection(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(NucleusError::FirestoreQuery(format!(
            "Ordered query on {}/{} failed ({}): {}",
            parent, collection, status, body
        )));
    }

    let results: Vec<RunQueryResponse> = resp
        .json()
        .await
        .map_err(|e| NucleusError::FirestoreQuery(format!("Failed to parse query result: {}", e)))?;

    Ok(results.into_iter().filter_map(|r| r.document).collect())
}

/// List documents in a subcollection (e.g. "hospital/ABC/alerts").
pub async fn list_subcollection(
    client: &Client,
    token: &str,
    parent: &str,
    collection: &str,
) -> Result<Vec<FirestoreDocument>, NucleusError> {
    let url = format!("{}/{}/{}", BASE_URL, parent, collection);

    let resp = client
        .get(&url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| NucleusError::FirestoreConnection(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(NucleusError::FirestoreQuery(format!(
            "List {}/{} failed ({}): {}",
            parent, collection, status, body
        )));
    }

    let result: ListDocumentsResponse = resp
        .json()
        .await
        .map_err(|e| NucleusError::FirestoreQuery(format!("Failed to parse list result: {}", e)))?;

    Ok(result.documents.unwrap_or_default())
}

/// CREATE a new document in a collection (Firestore auto-generates the document ID).
pub async fn create_document(
    client: &Client,
    token: &str,
    parent: &str,    // e.g. "hospital/ABC"
    collection: &str, // e.g. "alerts"
    fields: Value,
) -> Result<String, NucleusError> {
    let url = format!("{}/{}/{}", BASE_URL, parent, collection);

    let body = json!({ "fields": fields });

    let resp = client
        .post(&url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| NucleusError::FirestoreConnection(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(NucleusError::FirestoreQuery(format!(
            "CREATE {}/{}/{} failed ({}): {}",
            parent, collection, "", status, body
        )));
    }

    // Extract the auto-generated document ID from the response
    let doc: FirestoreDocument = resp
        .json()
        .await
        .map_err(|e| NucleusError::FirestoreQuery(format!("Failed to parse create result: {}", e)))?;

    // doc.name is like "projects/puru-255206/databases/(default)/documents/hospital/ABC/alerts/xyz123"
    let doc_id = doc
        .name
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string();

    Ok(doc_id)
}

/// PATCH a document with specific fields. Only fields in `field_paths` are updated.
pub async fn patch_document(
    client: &Client,
    token: &str,
    path: &str,
    fields: Value,
    field_paths: &[&str],
) -> Result<(), NucleusError> {
    let mut url = format!("{}/{}", BASE_URL, path);

    // Build updateMask query params
    for (i, fp) in field_paths.iter().enumerate() {
        let sep = if i == 0 { '?' } else { '&' };
        url.push_str(&format!("{}updateMask.fieldPaths={}", sep, fp));
    }

    let body = json!({ "fields": fields });

    let resp = client
        .patch(&url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| NucleusError::FirestoreConnection(e.to_string()))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(NucleusError::FirestoreQuery(format!(
            "PATCH {} failed ({}): {}",
            path, status, body
        )));
    }

    Ok(())
}
