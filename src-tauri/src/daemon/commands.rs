//! Command dispatcher for cloud-initiated commands via Firestore.
//!
//! Dispatches command types to existing module functions.
//! Never panics — catches all errors and returns a CommandResult.

use serde_json::{Map, Value};

use crate::firestore::convert::{get_optional_string, get_string};

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
