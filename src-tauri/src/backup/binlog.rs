//! Binlog shipping — MySQL binary log replication to LAN network share

use chrono::{DateTime, Utc};
use flate2::write::GzEncoder;
use flate2::Compression;
use mysql_async::prelude::*;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::config::{self, NucleusConfig};
use crate::error::NucleusError;

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinlogStatus {
    pub lan_enabled: bool,
    pub lan_last_shipped_file: Option<String>,
    pub lan_last_shipped_at: Option<DateTime<Utc>>,
    pub lan_total_shipped: u64,
    pub lan_last_error: Option<String>,
    pub current_master_file: Option<String>,
    pub current_master_position: Option<u64>,
    pub files_pending: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinlogShipResult {
    pub files_shipped: u32,
    pub bytes_shipped: u64,
    pub duration_seconds: u64,
    pub errors: Vec<String>,
}

/// Persisted local state for tracking LAN binlog shipping progress
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LanBinlogState {
    pub last_shipped_file: Option<String>,
    pub last_shipped_at: Option<DateTime<Utc>>,
    pub total_shipped: u64,
    pub last_error: Option<String>,
}

// ── LAN state persistence ────────────────────────────────────────────────────

fn lan_state_path() -> PathBuf {
    config::config_dir().join("binlog-lan-state.json")
}

pub(crate) fn load_lan_state() -> LanBinlogState {
    let path = lan_state_path();
    if !path.exists() {
        return LanBinlogState::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => LanBinlogState::default(),
    }
}

fn save_lan_state(state: &LanBinlogState) -> Result<(), NucleusError> {
    let path = lan_state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(state)?;
    std::fs::write(&path, content)?;
    Ok(())
}

// ── MySQL helpers ────────────────────────────────────────────────────────────

fn mysql_err(e: mysql_async::Error) -> NucleusError {
    NucleusError::MySqlConnection(format!("{}", e))
}

#[derive(Debug)]
struct MasterStatus {
    file: String,
    position: u64,
}

async fn get_master_status(pool: &mysql_async::Pool) -> Result<MasterStatus, NucleusError> {
    let mut conn = pool.get_conn().await.map_err(mysql_err)?;
    let row: Option<mysql_async::Row> = conn
        .query_first("SHOW MASTER STATUS")
        .await
        .map_err(mysql_err)?;

    let row = row.ok_or_else(|| {
        NucleusError::MySqlConnection(
            "SHOW MASTER STATUS returned no rows. Is binary logging enabled?".into(),
        )
    })?;

    let file: String = row.get("File").ok_or_else(|| {
        NucleusError::MySqlConnection("Missing 'File' in SHOW MASTER STATUS".into())
    })?;
    let position: u64 = row.get("Position").ok_or_else(|| {
        NucleusError::MySqlConnection("Missing 'Position' in SHOW MASTER STATUS".into())
    })?;

    Ok(MasterStatus { file, position })
}

#[derive(Debug)]
struct BinlogFileInfo {
    name: String,
    #[allow(dead_code)]
    size: u64,
}

async fn list_binlog_files(pool: &mysql_async::Pool) -> Result<Vec<BinlogFileInfo>, NucleusError> {
    let mut conn = pool.get_conn().await.map_err(mysql_err)?;
    let rows: Vec<mysql_async::Row> = conn
        .query("SHOW BINARY LOGS")
        .await
        .map_err(mysql_err)?;

    let mut files = Vec::new();
    for row in rows {
        let name: String = row.get("Log_name").unwrap_or_default();
        let size: u64 = row.get("File_size").unwrap_or(0);
        files.push(BinlogFileInfo { name, size });
    }
    Ok(files)
}

/// Read a binlog file's raw contents via `mysqlbinlog` CLI tool
async fn read_binlog_data(config: &NucleusConfig, file_name: &str) -> Result<Vec<u8>, NucleusError> {
    let output = crate::process::silent_cmd("mysqlbinlog")
        .args([
            "--read-from-remote-server",
            &format!("--host={}", config.mysql_host),
            &format!("--port={}", config.mysql_port),
            &format!("--user={}", config.mysql_user),
            "--raw",
            "--result-file=-",
            file_name,
        ])
        .env("MYSQL_PWD", &config.mysql_password)
        .output()
        .await
        .map_err(|e| NucleusError::MySqlConnection(format!("Failed to run mysqlbinlog: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NucleusError::MySqlConnection(format!(
            "mysqlbinlog failed for {}: {}",
            file_name,
            stderr.trim()
        )));
    }

    Ok(output.stdout)
}

// ── Gzip helper ──────────────────────────────────────────────────────────────

fn gzip_compress(data: &[u8]) -> Result<Vec<u8>, NucleusError> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data)?;
    encoder
        .finish()
        .map_err(|e| NucleusError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Ship unshipped binlog files to LAN network share.
/// Writes gzipped binlog files to `{lan_path}/{hospital_code}/binlogs/{file}.gz`.
/// No retention purge — storage is cheap on LAN, keep all binlogs.
pub async fn ship_binlogs_to_lan(
    license: &crate::licensing::License,
) -> Result<BinlogShipResult, NucleusError> {
    if !license.can_use_binlog_shipping() {
        return Err(NucleusError::LicenseExpired(
            "Binlog shipping requires the binlog_shipping license feature.".into(),
        ));
    }

    let cfg = config::load_config()?;
    if cfg.hospital_code.is_empty() {
        return Err(NucleusError::InvalidConfig(
            "Hospital code is not set. Configure it in Settings.".into(),
        ));
    }

    if !cfg.lan.enabled || cfg.lan.path.is_empty() {
        return Err(NucleusError::InvalidConfig(
            "LAN backup is not enabled or path is not configured.".into(),
        ));
    }

    if !cfg.lan.binlog_enabled {
        return Err(NucleusError::InvalidConfig(
            "LAN binlog shipping is not enabled in configuration.".into(),
        ));
    }

    let start = std::time::Instant::now();
    let mut state = load_lan_state();
    let mut errors = Vec::new();
    let mut files_shipped: u32 = 0;
    let mut bytes_shipped: u64 = 0;

    // Connect to MySQL
    let pool = super::create_mysql_pool(&cfg)?;
    let master = get_master_status(&pool).await?;
    let binlog_files = list_binlog_files(&pool).await?;

    // Find files to ship (after last_shipped_file, excluding current active file)
    let files_to_ship: Vec<&BinlogFileInfo> = if let Some(ref last) = state.last_shipped_file {
        binlog_files
            .iter()
            .filter(|f| f.name.as_str() > last.as_str() && f.name != master.file)
            .collect()
    } else {
        binlog_files
            .iter()
            .filter(|f| f.name != master.file)
            .collect()
    };

    if files_to_ship.is_empty() {
        pool.disconnect().await.map_err(mysql_err)?;
        return Ok(BinlogShipResult {
            files_shipped: 0,
            bytes_shipped: 0,
            duration_seconds: start.elapsed().as_secs(),
            errors,
        });
    }

    // Prepare LAN destination directory
    let dest_dir = PathBuf::from(&cfg.lan.path)
        .join(&cfg.hospital_code)
        .join("binlogs");
    std::fs::create_dir_all(&dest_dir)?;

    for file_info in &files_to_ship {
        // Read binlog data via mysqlbinlog
        let raw_data = match read_binlog_data(&cfg, &file_info.name).await {
            Ok(data) => data,
            Err(e) => {
                let msg = format!("Failed to read {}: {}", file_info.name, e);
                tracing::warn!("LAN binlog ship: {}", msg);
                errors.push(msg);
                continue;
            }
        };

        // Gzip compress
        let compressed = match gzip_compress(&raw_data) {
            Ok(data) => data,
            Err(e) => {
                let msg = format!("Failed to compress {}: {}", file_info.name, e);
                tracing::warn!("LAN binlog ship: {}", msg);
                errors.push(msg);
                continue;
            }
        };

        // Atomic write: .gz.tmp -> rename to .gz
        let dest_file = dest_dir.join(format!("{}.gz", file_info.name));
        let tmp_file = dest_dir.join(format!("{}.gz.tmp", file_info.name));

        if let Err(e) = std::fs::write(&tmp_file, &compressed) {
            let msg = format!("Failed to write {}: {}", file_info.name, e);
            tracing::warn!("LAN binlog ship: {}", msg);
            errors.push(msg);
            let _ = std::fs::remove_file(&tmp_file);
            continue;
        }

        if let Err(e) = std::fs::rename(&tmp_file, &dest_file) {
            let msg = format!("Failed to rename {}: {}", file_info.name, e);
            tracing::warn!("LAN binlog ship: {}", msg);
            errors.push(msg);
            let _ = std::fs::remove_file(&tmp_file);
            continue;
        }

        files_shipped += 1;
        bytes_shipped += compressed.len() as u64;

        // Update state
        state.last_shipped_file = Some(file_info.name.clone());
        state.last_shipped_at = Some(Utc::now());
        state.total_shipped += 1;

        tracing::info!(
            "LAN binlog shipped: {} ({} bytes compressed)",
            file_info.name,
            compressed.len()
        );
    }

    // Update error state
    state.last_error = errors.last().cloned();

    // Save state
    if let Err(e) = save_lan_state(&state) {
        tracing::warn!("Failed to save LAN binlog state: {}", e);
    }

    pool.disconnect().await.map_err(mysql_err)?;

    Ok(BinlogShipResult {
        files_shipped,
        bytes_shipped,
        duration_seconds: start.elapsed().as_secs(),
        errors,
    })
}

/// Point-in-time recovery: restore full backup + replay binlogs up to target timestamp.
///
/// Steps:
/// 1. Restore the specified (or latest) full backup via `restore_backup()`
/// 2. Find shipped binlog `.gz` files on LAN that are newer than the backup
/// 3. Decompress and replay each with `mysqlbinlog --stop-datetime={target}`
pub async fn restore_pitr(
    backup_id: Option<&str>,
    stop_datetime: &str,
) -> Result<PitrResult, NucleusError> {
    let cfg = config::load_config()?;
    if cfg.hospital_code.is_empty() {
        return Err(NucleusError::InvalidConfig(
            "Hospital code is not set.".into(),
        ));
    }
    if !cfg.lan.enabled || cfg.lan.path.is_empty() {
        return Err(NucleusError::InvalidConfig(
            "LAN path must be configured for point-in-time recovery (binlogs are read from LAN).".into(),
        ));
    }

    // Validate stop_datetime format: "YYYY-MM-DD HH:MM:SS"
    if chrono::NaiveDateTime::parse_from_str(stop_datetime, "%Y-%m-%d %H:%M:%S").is_err() {
        return Err(NucleusError::Validation(format!(
            "Invalid datetime format '{}'. Expected: YYYY-MM-DD HH:MM:SS",
            stop_datetime
        )));
    }

    let start = std::time::Instant::now();

    // Step 1: Restore the base backup
    let resolved_id = match backup_id {
        Some(id) => id.to_string(),
        None => {
            let history = super::get_backup_history().await?;
            history
                .iter()
                .find(|r| r.status == super::BackupStatus::Completed)
                .map(|r| r.id.clone())
                .ok_or_else(|| NucleusError::NotFound("No completed backups found".into()))?
        }
    };

    tracing::info!("PITR Step 1: Restoring base backup {}", resolved_id);
    super::restore_backup(&resolved_id).await?;

    // Step 2: Find binlog files on LAN
    let binlog_dir = PathBuf::from(&cfg.lan.path)
        .join(&cfg.hospital_code)
        .join("binlogs");

    if !binlog_dir.exists() {
        return Ok(PitrResult {
            backup_restored: resolved_id,
            binlogs_applied: 0,
            stop_datetime: stop_datetime.to_string(),
            duration_seconds: start.elapsed().as_secs(),
        });
    }

    let mut gz_files: Vec<PathBuf> = std::fs::read_dir(&binlog_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|ext| ext.to_str())
                == Some("gz")
        })
        .map(|e| e.path())
        .collect();

    gz_files.sort();

    if gz_files.is_empty() {
        tracing::info!("PITR: No binlog files found on LAN, restore complete (base backup only)");
        return Ok(PitrResult {
            backup_restored: resolved_id,
            binlogs_applied: 0,
            stop_datetime: stop_datetime.to_string(),
            duration_seconds: start.elapsed().as_secs(),
        });
    }

    // Step 3: Decompress and apply each binlog with --stop-datetime
    let temp_dir = std::env::temp_dir().join("puru-pitr-binlogs");
    if temp_dir.exists() {
        std::fs::remove_dir_all(&temp_dir)?;
    }
    std::fs::create_dir_all(&temp_dir)?;

    let mut applied = 0u32;
    for gz_path in &gz_files {
        let file_stem = gz_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        // Decompress .gz → raw binlog
        let raw_path = temp_dir.join(file_stem);
        decompress_gz(gz_path, &raw_path)?;

        // Apply with mysqlbinlog --stop-datetime piped to mysql
        tracing::info!("PITR: Applying binlog {} (stop at {})", file_stem, stop_datetime);
        match apply_binlog_with_stop(&cfg, &raw_path, stop_datetime).await {
            Ok(()) => applied += 1,
            Err(e) => {
                tracing::warn!("PITR: Failed to apply {}: {}", file_stem, e);
                // Continue — partial replay is better than none
            }
        }

        // Clean up decompressed file
        let _ = std::fs::remove_file(&raw_path);
    }

    let _ = std::fs::remove_dir_all(&temp_dir);

    let result = PitrResult {
        backup_restored: resolved_id,
        binlogs_applied: applied,
        stop_datetime: stop_datetime.to_string(),
        duration_seconds: start.elapsed().as_secs(),
    };

    tracing::info!(
        "PITR complete: backup={}, binlogs={}, target={}, {}s",
        result.backup_restored,
        result.binlogs_applied,
        result.stop_datetime,
        result.duration_seconds
    );

    Ok(result)
}

/// Decompress a .gz file to a raw output path.
fn decompress_gz(gz_path: &Path, out_path: &Path) -> Result<(), NucleusError> {
    use flate2::read::GzDecoder;
    use std::io::Read;

    let file = std::fs::File::open(gz_path)?;
    let mut decoder = GzDecoder::new(file);
    let mut data = Vec::new();
    decoder.read_to_end(&mut data)?;
    std::fs::write(out_path, &data)?;
    Ok(())
}

/// Apply a raw binlog file using `mysqlbinlog --stop-datetime | mysql`.
async fn apply_binlog_with_stop(
    config: &NucleusConfig,
    binlog_path: &Path,
    stop_datetime: &str,
) -> Result<(), NucleusError> {
    // mysqlbinlog --stop-datetime="..." binlog_file | mysql -u... -p...
    let binlog_output = crate::process::silent_cmd("mysqlbinlog")
        .arg(format!("--stop-datetime={}", stop_datetime))
        .arg(binlog_path.to_string_lossy().as_ref())
        .output()
        .await
        .map_err(|e| NucleusError::MySqlConnection(format!("Failed to run mysqlbinlog: {}", e)))?;

    if !binlog_output.status.success() {
        let stderr = String::from_utf8_lossy(&binlog_output.stderr);
        return Err(NucleusError::MySqlConnection(format!(
            "mysqlbinlog failed: {}",
            stderr.trim()
        )));
    }

    if binlog_output.stdout.is_empty() {
        return Ok(()); // No events in this binlog before stop_datetime
    }

    // Pipe SQL output into mysql client
    let use_local = super::has_local_mysql().await;

    let mut child = if use_local {
        crate::process::silent_cmd("mysql")
            .env("MYSQL_PWD", &config.mysql_password)
            .args([
                &format!("-h{}", config.mysql_host),
                &format!("-P{}", config.mysql_port),
                &format!("-u{}", config.mysql_user),
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| NucleusError::MySqlConnection(format!("Failed to spawn mysql: {}", e)))?
    } else {
        let container = super::find_mysql_container().await.ok_or_else(|| {
            NucleusError::MySqlConnection(
                "mysql client not found locally and no running MySQL Docker container found.".into(),
            )
        })?;
        crate::process::silent_cmd("docker")
            .args([
                "exec", "-i", &container, "mysql",
                &format!("-u{}", config.mysql_user),
                &format!("-p{}", config.mysql_password),
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| NucleusError::MySqlConnection(format!("Failed to spawn docker exec mysql: {}", e)))?
    };

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin.write_all(&binlog_output.stdout).await?;
    }

    let output = child.wait_with_output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NucleusError::MySqlConnection(format!(
            "mysql replay failed: {}",
            stderr.trim()
        )));
    }

    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PitrResult {
    pub backup_restored: String,
    pub binlogs_applied: u32,
    pub stop_datetime: String,
    pub duration_seconds: u64,
}

/// Get current binlog status (LAN shipping info + MySQL master status).
pub async fn get_binlog_status() -> Result<BinlogStatus, NucleusError> {
    let cfg = config::load_config()?;
    let lan_state = load_lan_state();
    let lan_enabled = cfg.lan.enabled && cfg.lan.binlog_enabled;

    // Try to get current master status
    let (current_file, current_position, files_pending) = if lan_enabled {
        match super::create_mysql_pool(&cfg) {
            Ok(pool) => {
                let master = get_master_status(&pool).await.ok();
                let binlog_files = list_binlog_files(&pool).await.unwrap_or_default();
                let _ = pool.disconnect().await;

                let pending = if let (Some(ref last), Some(ref master_status)) =
                    (&lan_state.last_shipped_file, &master)
                {
                    binlog_files
                        .iter()
                        .filter(|f| {
                            f.name.as_str() > last.as_str() && f.name != master_status.file
                        })
                        .count() as u32
                } else if let Some(ref master_status) = master {
                    binlog_files
                        .iter()
                        .filter(|f| f.name != master_status.file)
                        .count() as u32
                } else {
                    0
                };

                (
                    master.as_ref().map(|m| m.file.clone()),
                    master.as_ref().map(|m| m.position),
                    pending,
                )
            }
            Err(_) => (None, None, 0),
        }
    } else {
        (None, None, 0)
    };

    Ok(BinlogStatus {
        lan_enabled,
        lan_last_shipped_file: lan_state.last_shipped_file,
        lan_last_shipped_at: lan_state.last_shipped_at,
        lan_total_shipped: lan_state.total_shipped,
        lan_last_error: lan_state.last_error,
        current_master_file: current_file,
        current_master_position: current_position,
        files_pending,
    })
}
