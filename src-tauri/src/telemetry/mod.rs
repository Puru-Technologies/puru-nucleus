//! Telemetry collection and reporting

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Daily telemetry summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetrySummary {
    pub date: String,
    pub services: HashMap<String, ServiceTelemetry>,
    pub system: SystemTelemetry,
    pub backup: BackupTelemetry,
    pub alerts: AlertSummary,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceTelemetry {
    pub uptime_percent: f64,
    pub avg_health_ms: u64,
    pub restart_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemTelemetry {
    pub avg_cpu: f64,
    pub max_cpu: f64,
    pub avg_ram: f64,
    pub max_ram: f64,
    pub avg_disk: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupTelemetry {
    pub full_count: u32,
    pub partial_count: u32,
    pub failed_count: u32,
    pub total_size_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertSummary {
    pub critical: u32,
    pub warning: u32,
    pub info: u32,
}

/// Snapshot for local storage (detailed)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetrySnapshot {
    pub timestamp: DateTime<Utc>,
    pub cpu_percent: f64,
    pub ram_gb: f64,
    pub disk_percent: f64,
    pub services: HashMap<String, ServiceSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceSnapshot {
    pub status: String,
    pub health_ms: Option<u64>,
}

/// Physical RAM, in GB. Distinct from disk space — the two are easy to confuse
/// in alert text, so anything user-facing built from this must say "RAM".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RamUsage {
    pub total_gb: f64,
    pub used_gb: f64,
    /// Memory the OS can hand to a process right now. On Linux this counts
    /// reclaimable page cache, so it is much larger — and much more honest —
    /// than `total - used`.
    pub available_gb: f64,
    pub used_percent: f64,
}

/// Usage of one mounted filesystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskUsage {
    /// Mount point / drive letter, e.g. `C:\` or `/var`.
    pub mount: String,
    pub total_gb: f64,
    pub free_gb: f64,
    pub used_percent: f64,
}

/// Current physical RAM usage.
pub fn ram_usage() -> RamUsage {
    use sysinfo::{System, SystemExt};

    let mut sys = System::new();
    sys.refresh_memory();

    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let total_gb = sys.total_memory() as f64 / GB;
    let used_gb = sys.used_memory() as f64 / GB;
    let available_gb = sys.available_memory() as f64 / GB;

    RamUsage {
        total_gb,
        used_gb,
        available_gb,
        used_percent: if total_gb > 0.0 {
            ((total_gb - available_gb) / total_gb) * 100.0
        } else {
            0.0
        },
    }
}

/// The fullest mounted disk. `disk_percent` in a snapshot averages every disk
/// together, which hides a single drive about to fill up — alerts name the
/// specific drive instead.
pub fn fullest_disk() -> Option<DiskUsage> {
    use sysinfo::{DiskExt, System, SystemExt};

    let mut sys = System::new();
    sys.refresh_disks_list();

    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    sys.disks()
        .iter()
        .filter(|d| d.total_space() > 0)
        .map(|d| {
            let total = d.total_space() as f64;
            let free = d.available_space() as f64;
            DiskUsage {
                mount: d.mount_point().to_string_lossy().to_string(),
                total_gb: total / GB,
                free_gb: free / GB,
                used_percent: ((total - free) / total) * 100.0,
            }
        })
        .max_by(|a, b| {
            a.used_percent
                .partial_cmp(&b.used_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

/// Collect a telemetry snapshot with current system + Docker service metrics
pub async fn collect_snapshot() -> Result<TelemetrySnapshot, crate::error::NucleusError> {
    use sysinfo::{CpuExt, DiskExt, System, SystemExt};

    let mut sys = System::new_all();
    sys.refresh_all();

    let cpu_percent = sys.global_cpu_info().cpu_usage() as f64;
    let ram_gb = sys.used_memory() as f64 / 1024.0 / 1024.0 / 1024.0;

    let disk_percent = {
        let disks = sys.disks();
        if !disks.is_empty() {
            let total: u64 = disks.iter().map(|d| d.total_space()).sum();
            let available: u64 = disks.iter().map(|d| d.available_space()).sum();
            if total > 0 {
                ((total - available) as f64 / total as f64) * 100.0
            } else {
                0.0
            }
        } else {
            0.0
        }
    };

    // Collect Docker service statuses
    let services = collect_service_snapshots().await;

    Ok(TelemetrySnapshot {
        timestamp: Utc::now(),
        cpu_percent,
        ram_gb,
        disk_percent,
        services,
    })
}

/// Query Docker for current status of all Puru services
async fn collect_service_snapshots() -> HashMap<String, ServiceSnapshot> {
    let mut map = HashMap::new();

    // Gracefully handle Docker not available
    let services = match crate::services::get_services().await {
        Ok(s) => s,
        Err(_) => return map,
    };

    for svc in &services {
        let status = format!("{:?}", svc.status).to_lowercase();
        map.insert(
            svc.name.clone(),
            ServiceSnapshot {
                status,
                health_ms: None, // would need a health-check endpoint ping
            },
        );
    }

    map
}
