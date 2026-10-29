//! Command dispatcher for cloud-initiated commands via Firestore.
//!
//! Dispatches command types to existing module functions.
//! Never panics — catches all errors and returns a CommandResult.

use serde_json::{Map, Value};

use crate::firestore::convert::{get_optional_integer, get_optional_string, get_string};

/// Result of executing a command.
pub struct CommandResult {
    pub success: bool,
    pub message: String,
}

/// Execute a command by type, dispatching to the appropriate module function.
pub async fn execute(command_type: &str, params: &Map<String, Value>) -> CommandResult {
    match command_type {
        "restart_service" => restart_service(params).await,
        "stop_service" => stop_service(params).await,
        "start_service" => start_service(params).await,
        "trigger_backup" => trigger_backup(params).await,
        "restart_all" => restart_all().await,
        "fetch_logs" => fetch_logs(params).await,
        other => CommandResult {
            success: false,
            message: format!("Unknown command type: {}", other),
        },
    }
}

async fn restart_service(params: &Map<String, Value>) -> CommandResult {
    let container_name = match extract_service_name(params) {
        Ok(name) => name,
        Err(e) => return e,
    };

    match crate::services::restart_service(&container_name).await {
        Ok(()) => CommandResult {
            success: true,
            message: format!("Service '{}' restarted successfully", container_name),
        },
        Err(e) => CommandResult {
            success: false,
            message: format!("Failed to restart '{}': {}", container_name, e),
        },
    }
}

async fn stop_service(params: &Map<String, Value>) -> CommandResult {
    let container_name = match extract_service_name(params) {
        Ok(name) => name,
        Err(e) => return e,
    };

    match crate::services::stop_service(&container_name).await {
        Ok(()) => CommandResult {
            success: true,
            message: format!("Service '{}' stopped successfully", container_name),
        },
        Err(e) => CommandResult {
            success: false,
            message: format!("Failed to stop '{}': {}", container_name, e),
        },
    }
}

async fn start_service(params: &Map<String, Value>) -> CommandResult {
    let container_name = match extract_service_name(params) {
        Ok(name) => name,
        Err(e) => return e,
    };

    match crate::services::start_service(&container_name).await {
        Ok(()) => CommandResult {
            success: true,
            message: format!("Service '{}' started successfully", container_name),
        },
        Err(e) => CommandResult {
            success: false,
            message: format!("Failed to start '{}': {}", container_name, e),
        },
    }
}

async fn trigger_backup(params: &Map<String, Value>) -> CommandResult {
    // Load license
    let license = match crate::licensing::load_license() {
        Ok(Some(l)) if l.is_valid() => l,
        Ok(Some(_)) => {
            return CommandResult {
                success: false,
                message: "License expired. Cannot run backup.".into(),
            }
        }
        Ok(None) => {
            return CommandResult {
                success: false,
                message: "No license found. Cannot run backup.".into(),
            }
        }
        Err(e) => {
            return CommandResult {
                success: false,
                message: format!("Failed to load license: {}", e),
            }
        }
    };

    let backup_type_str = params
        .get("backup_type")
        .and_then(|v| get_optional_string(v))
        .unwrap_or_else(|| "full".into());

    let backup_type = match backup_type_str.as_str() {
        "partial" => crate::backup::BackupType::Partial,
        _ => crate::backup::BackupType::Full,
    };

    match crate::backup::start_backup(backup_type, &license).await {
        Ok(result) => CommandResult {
            success: true,
            message: format!(
                "Backup completed: id={}, size={}MB, duration={}s",
                result.backup_id, result.size_mb, result.duration_seconds
            ),
        },
        Err(e) => CommandResult {
            success: false,
            message: format!("Backup failed: {}", e),
        },
    }
}

async fn restart_all() -> CommandResult {
    let services = match crate::services::get_services().await {
        Ok(s) => s,
        Err(e) => {
            return CommandResult {
                success: false,
                message: format!("Failed to list services: {}", e),
            }
        }
    };
    let running: Vec<_> = services
        .iter()
        .filter(|s| matches!(s.status, crate::services::ServiceStatus::Running))
        .collect();

    if running.is_empty() {
        return CommandResult {
            success: true,
            message: "No running services to restart".into(),
        };
    }

    let mut succeeded = 0usize;
    let mut failed = Vec::new();

    for svc in &running {
        match crate::services::restart_service(&svc.container_name).await {
            Ok(()) => succeeded += 1,
            Err(e) => failed.push(format!("{}: {}", svc.name, e)),
        }
    }

    if failed.is_empty() {
        CommandResult {
            success: true,
            message: format!("All {} services restarted successfully", succeeded),
        }
    } else {
        CommandResult {
            success: false,
            message: format!(
                "{} succeeded, {} failed: {}",
                succeeded,
                failed.len(),
                failed.join("; ")
            ),
        }
    }
}

/// Lines returned when the request doesn't say.
const LOG_LINES_DEFAULT: u64 = 500;
/// Hard cap on lines per request.
const LOG_LINES_MAX: u64 = 5000;
/// The result rides inline in the command doc (Firestore caps a doc at 1 MiB),
/// so the gzip payload stays under this; older lines are dropped to fit.
const LOG_INLINE_MAX_GZIP: usize = 700 * 1024;

/// Tail a service's log and return it gzipped + base64 in the result
/// (docs/logging-system.md §5, inline delivery).
/// Params: `service_name` (container name in Docker mode), `lines`
/// (default 500, max 5000), `since_minutes` (optional).
async fn fetch_logs(params: &Map<String, Value>) -> CommandResult {
    let service = match extract_service_name(params) {
        Ok(name) => name,
        Err(e) => return e,
    };
    let lines = params
        .get("lines")
        .and_then(get_optional_integer)
        .map(|n| n.clamp(1, LOG_LINES_MAX as i64) as u64)
        .unwrap_or(LOG_LINES_DEFAULT);
    let since = params
        .get("since_minutes")
        .and_then(get_optional_integer)
        .filter(|m| *m > 0)
        .map(|m| chrono::Utc::now().timestamp() - m * 60);

    let text = match crate::services::get_container_logs(&service, lines, since, None).await {
        Ok(t) => t,
        Err(e) => {
            return CommandResult {
                success: false,
                message: format!("Couldn't read logs for '{}': {}", service, e),
            }
        }
    };

    // Drop the oldest lines until the gzip fits inline.
    let mut all: Vec<&str> = text.lines().collect();
    let mut truncated = false;
    let gz = loop {
        let body = all.join("\n");
        let gz = match gzip(body.as_bytes()) {
            Ok(g) => g,
            Err(e) => {
                return CommandResult {
                    success: false,
                    message: format!("Couldn't compress logs: {}", e),
                }
            }
        };
        if gz.len() <= LOG_INLINE_MAX_GZIP || all.len() <= 1 {
            break (gz, body.len());
        }
        let cut = (all.len() / 4).max(1);
        all.drain(..cut);
        truncated = true;
    };

    use base64::Engine;
    let payload = serde_json::json!({
        "kind": "inline",
        "service": service,
        "lines": all.len(),
        "truncated": truncated,
        "encoding": "gzip+base64",
        "raw_bytes": gz.1,
        "data": base64::engine::general_purpose::STANDARD.encode(&gz.0),
    });
    CommandResult {
        success: true,
        message: payload.to_string(),
    }
}

fn gzip(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    use std::io::Write;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(bytes)?;
    enc.finish()
}

/// Extract the service_name param (Firestore stringValue) from command params.
fn extract_service_name(params: &Map<String, Value>) -> Result<String, CommandResult> {
    params
        .get("service_name")
        .and_then(|v| get_string(v).ok())
        .ok_or_else(|| CommandResult {
            success: false,
            message: "Missing required param: service_name".into(),
        })
}
