//! Puru-scoped process explorer
//!
//! Lists every running process whose binary name matches a Puru-relevant
//! pattern (java, nginx, mysqld, rabbitmq/beam, etc.), enriches each with
//! the TCP ports it's listening on, and exposes a kill action by PID.
//!
//! Used from the Services tab as a panic button when Nucleus's own
//! `stop_service` can't free a port — e.g. a zombie auth/has JVM that
//! escaped a previous stop attempt and is holding 8080.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One process the operator might want to inspect or kill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    /// Short binary name, e.g. "java.exe".
    pub name: String,
    /// Best guess at what this process is — e.g. "puru-auth" when the cmdline
    /// contains "puru-auth.jar". Empty when we can't tell.
    pub label: String,
    /// Truncated command line so the UI can show context without overwhelming.
    pub cmd: String,
    /// Instantaneous CPU usage as percent, rounded.
    pub cpu_pct: f32,
    /// Resident set size in MB.
    pub mem_mb: u64,
    /// TCP ports this process is `LISTEN`ing on (sorted, deduped).
    pub listening_ports: Vec<u16>,
}

const PURU_BINARY_PATTERNS: &[&str] = &[
    "java",        // covers java, java.exe — all backends
    "nginx",       // hydrogen frontend
    "mysqld",      // MySQL server (Linux/macOS)
    "mysql",       // some Windows installers register as `mysql.exe`
    "rabbitmq",    // rabbitmq-server launcher
    "beam.smp",    // erlang VM — what RabbitMQ actually runs as
    "erl",         // erl / erl.exe
    "epmd",        // erlang port mapper
];

/// Heuristic: does this binary name look like something Puru cares about?
fn is_puru_relevant(name_lower: &str) -> bool {
    PURU_BINARY_PATTERNS
        .iter()
        .any(|p| name_lower.contains(p))
}

/// Pull a service hint out of a Java command line.
/// `java -jar /opt/puru/jars/puru-auth.jar` → "puru-auth".
fn label_from_cmd(cmd: &str) -> String {
    for token in cmd.split_whitespace() {
        if let Some(file) = token.rsplit(['/', '\\']).next() {
            if file.starts_with("puru-") && file.ends_with(".jar") {
                return file.trim_end_matches(".jar").to_string();
            }
        }
    }
    String::new()
}

/// Last-resort service name from the binary name alone, so infra/frontend
/// processes (which have no `puru-*.jar` on their command line) still resolve
/// to a friendly name. Mirrors the "Database" / "Message Broker" labels used in
/// the Services list.
fn label_from_binary(name_lower: &str) -> String {
    if name_lower.contains("mysqld") || name_lower.contains("mysql") {
        "Database".to_string()
    } else if name_lower.contains("rabbitmq")
        || name_lower.contains("beam")
        || name_lower.contains("erl")
        || name_lower.contains("epmd")
    {
        "Message Broker".to_string()
    } else if name_lower.contains("nginx") {
        "puru-hydrogen".to_string()
    } else {
        String::new()
    }
}

/// PID → service name, read from the `.pid` files Nucleus writes under the
/// native logs dir (`<service>.pid`). This is authoritative — it matches the
/// exact PID we launched — so it labels a tracked service even when its command
/// line is empty or unparseable (common for SYSTEM-owned JVMs).
fn pid_file_labels() -> HashMap<u32, String> {
    let mut map = HashMap::new();
    let cfg = match crate::config::load_config() {
        Ok(c) => c,
        Err(_) => return map,
    };
    if let Ok(entries) = std::fs::read_dir(cfg.native_logs_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|x| x.to_str()) != Some("pid") {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if let Ok(txt) = std::fs::read_to_string(&path) {
                if let Ok(pid) = txt.trim().parse::<u32>() {
                    map.insert(pid, name.to_string());
                }
            }
        }
    }
    map
}

/// Truncate a long command line for display. Keeps the head and tail so the
/// jar path stays visible.
fn truncate_cmd(cmd: &str, max: usize) -> String {
    if cmd.len() <= max {
        return cmd.to_string();
    }
    let head = max * 2 / 3;
    let tail = max - head - 3;
    format!("{}...{}", &cmd[..head], &cmd[cmd.len() - tail..])
}

/// Map of PID → listening TCP ports. Cross-platform: `netstat -ano` on
/// Windows, `lsof` on macOS/Linux. Both are present on every supported host.
async fn collect_listening_ports() -> HashMap<u32, Vec<u16>> {
    let mut by_pid: HashMap<u32, Vec<u16>> = HashMap::new();

    #[cfg(windows)]
    {
        if let Ok(output) = crate::process::silent_cmd("netstat")
            .args(["-ano", "-p", "TCP"])
            .output()
            .await
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                // Columns: Proto  Local Address  Foreign Address  State  PID
                // We only care about LISTENING rows.
                if !line.contains("LISTENING") {
                    continue;
                }
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.len() < 5 {
                    continue;
                }
                let local = cols[1];
                let pid: u32 = match cols[4].parse() {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                // `local` looks like 0.0.0.0:8080 or [::]:8080
                if let Some(port_str) = local.rsplit(':').next() {
                    if let Ok(port) = port_str.parse::<u16>() {
                        by_pid.entry(pid).or_default().push(port);
                    }
                }
            }
        }
    }

    #[cfg(not(windows))]
    {
        if let Ok(output) = crate::process::silent_cmd("lsof")
            .args(["-nP", "-iTCP", "-sTCP:LISTEN"])
            .output()
            .await
        {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Skip the header row. Columns:
            //   COMMAND  PID  USER  FD  TYPE  DEVICE  SIZE/OFF  NODE  NAME
            // NAME looks like *:8080 or 127.0.0.1:8080 or [::]:8080.
            for line in stdout.lines().skip(1) {
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.len() < 9 {
                    continue;
                }
                let pid: u32 = match cols[1].parse() {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                let name = cols[cols.len() - 1];
                if let Some(port_str) = name.rsplit(':').next() {
                    // lsof sometimes appends " (LISTEN)" — strip non-digits.
                    let cleaned: String =
                        port_str.chars().take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(port) = cleaned.parse::<u16>() {
                        by_pid.entry(pid).or_default().push(port);
                    }
                }
            }
        }
    }

    // Sort + dedupe each PID's port list so the UI is stable across refreshes.
    for ports in by_pid.values_mut() {
        ports.sort_unstable();
        ports.dedup();
    }
    by_pid
}

/// Enumerate all Puru-relevant processes with port/CPU/mem context.
pub async fn list_processes() -> Vec<ProcessInfo> {
    use sysinfo::{PidExt, ProcessExt, System, SystemExt};

    let ports_by_pid = collect_listening_ports().await;
    let pid_labels = pid_file_labels();

    let mut sys = System::new_all();
    sys.refresh_all();
    // sysinfo's CPU usage is computed as delta between refreshes — the first
    // sample after `new_all` is always 0. Sleep briefly and refresh again so
    // the numbers we hand the UI are actually meaningful.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    sys.refresh_processes();

    let mut out = Vec::new();
    for (pid, process) in sys.processes() {
        let name = process.name().to_string();
        if !is_puru_relevant(&name.to_lowercase()) {
            continue;
        }
        let cmd_joined = process.cmd().join(" ");
        let cmd = truncate_cmd(&cmd_joined, 180);
        let pid_u32 = pid.as_u32();
        // Authoritative PID-file match first, then the jar on the command line,
        // then a binary-name guess — so every Puru process shows a name.
        let label = pid_labels
            .get(&pid_u32)
            .cloned()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                let from_cmd = label_from_cmd(&cmd_joined);
                if from_cmd.is_empty() {
                    label_from_binary(&name.to_lowercase())
                } else {
                    from_cmd
                }
            });
        let mem_mb = process.memory() / 1024 / 1024;
        let cpu_pct = (process.cpu_usage() * 10.0).round() / 10.0;
        let listening_ports = ports_by_pid.get(&pid_u32).cloned().unwrap_or_default();

        out.push(ProcessInfo {
            pid: pid_u32,
            name,
            label,
            cmd,
            cpu_pct,
            mem_mb,
            listening_ports,
        });
    }
    // Highest CPU first, then biggest memory, so the heavy hitters surface.
    out.sort_by(|a, b| {
        b.cpu_pct
            .partial_cmp(&a.cpu_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.mem_mb.cmp(&a.mem_mb))
    });
    out
}

/// Force-kill a process by PID. Same semantics as our service-stop path:
/// `taskkill /F /T /PID` on Windows, `kill -9` on Unix. Returns the actual
/// PID killed on success.
pub async fn kill_pid(pid: u32) -> Result<u32, crate::error::NucleusError> {
    #[cfg(windows)]
    {
        let output = crate::process::silent_cmd("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .output()
            .await
            .map_err(|e| {
                crate::error::NucleusError::Internal(format!("taskkill failed: {}", e))
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(crate::error::NucleusError::Internal(format!(
                "taskkill /F /PID {} failed: {}",
                pid,
                stderr.trim()
            )));
        }
    }
    #[cfg(not(windows))]
    {
        // SAFETY: kill(2) accepts any pid_t; the syscall itself validates.
        let rc = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        if rc != 0 {
            return Err(crate::error::NucleusError::Internal(format!(
                "kill(SIGKILL, {}) failed: {}",
                pid,
                std::io::Error::last_os_error()
            )));
        }
    }
    Ok(pid)
}
