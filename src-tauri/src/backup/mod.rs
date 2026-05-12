//! Backup management — Per-object structured MySQL dump, GCS upload, history tracking

pub mod binlog;

use chrono::{DateTime, Datelike, Utc};
use mysql_async::prelude::*;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

use crate::config::{self, NucleusConfig};
use crate::error::NucleusError;

// ── Types ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupResult {
    pub success: bool,
    pub backup_id: String,
    pub size_mb: u64,
    pub duration_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupRecord {
    pub id: String,
    #[serde(rename = "type")]
    pub backup_type: BackupType,
    pub status: BackupStatus,
    pub size_mb: u64,
    pub created_at: DateTime<Utc>,
    pub uploaded: bool,
    #[serde(default)]
    pub lan_copied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BackupType {
    Full,
    Partial,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BackupStatus {
    Completed,
    Failed,
    InProgress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub databases: Vec<DatabaseBackupInfo>,
    pub total_objects: usize,
    pub failed_objects: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseBackupInfo {
    pub name: String,
    pub tables: Vec<String>,
    pub views: Vec<String>,
    pub triggers: Vec<String>,
    pub routines: Vec<String>,
    pub events: Vec<String>,
}

// ── Constants ────────────────────────────────────────────────────────────────

const GCS_BUCKET: &str = "puru-automated-backup";

const SYSTEM_DATABASES: &[&str] = &["information_schema", "performance_schema", "mysql", "sys"];

// ── History persistence ──────────────────────────────────────────────────────

#[derive(Serialize, Deserialize)]
struct BackupHistory {
    records: Vec<BackupRecord>,
}

fn history_path() -> PathBuf {
    config::config_dir().join("backups.toml")
}

fn backups_dir() -> PathBuf {
    let dir = config::config_dir().join("backups");
    if !dir.exists() {
        let _ = std::fs::create_dir_all(&dir);
    }
    dir
}

fn load_history() -> Result<Vec<BackupRecord>, NucleusError> {
    let path = history_path();
    if !path.exists() {
        return Ok(vec![]);
    }

    let content = std::fs::read_to_string(&path)?;
    let history: BackupHistory = toml::from_str(&content).map_err(|e| {
        NucleusError::InvalidConfig(format!("Failed to parse backup history: {}", e))
    })?;

    Ok(history.records)
}

fn save_history(records: &[BackupRecord]) -> Result<(), NucleusError> {
    let history = BackupHistory {
        records: records.to_vec(),
    };

    let content = toml::to_string_pretty(&history).map_err(|e| {
        NucleusError::InvalidConfig(format!("Failed to serialize backup history: {}", e))
    })?;

    let path = history_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, content)?;

    Ok(())
}

fn add_record(record: BackupRecord) -> Result<(), NucleusError> {
    let mut records = load_history()?;
    records.push(record);
    save_history(&records)
}

fn update_record(
    id: &str,
    status: BackupStatus,
    size_mb: u64,
    uploaded: bool,
    lan_copied: bool,
) -> Result<(), NucleusError> {
    update_record_with_error(id, status, size_mb, uploaded, lan_copied, None)
}

fn update_record_with_error(
    id: &str,
    status: BackupStatus,
    size_mb: u64,
    uploaded: bool,
    lan_copied: bool,
    error: Option<String>,
) -> Result<(), NucleusError> {
    let mut records = load_history()?;
    if let Some(rec) = records.iter_mut().find(|r| r.id == id) {
        rec.status = status;
        rec.size_mb = size_mb;
        rec.uploaded = uploaded;
        rec.lan_copied = lan_copied;
        rec.error = error;
    }
    save_history(&records)
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Generate backup name: `BTCT-PURU-02-03-2026-02-15-AM`
fn generate_backup_name(hospital_code: &str) -> String {
    let now = Utc::now();
    let date_part = now.format("%d-%m-%Y");
    let time_part = now.format("%I-%M-%p");
    format!("{}-PURU-{}-{}", hospital_code, date_part, time_part)
}

fn backup_zip_filename(name: &str) -> String {
    format!("{}.zip", name)
}

/// GCS path: `{code}/{year}/{Month}/{code}-PURU-DD-MM-YYYY-HH-MM-AM.zip`
fn gcs_object_path(hospital_code: &str, backup_name: &str) -> String {
    let now = Utc::now();
    let year = now.year();
    let month = now.format("%B");
    format!(
        "{}/{}/{}/{}",
        hospital_code,
        year,
        month,
        backup_zip_filename(backup_name)
    )
}

fn zip_err(e: zip::result::ZipError) -> NucleusError {
    NucleusError::Io(std::io::Error::new(std::io::ErrorKind::Other, e))
}

fn mysql_err(e: mysql_async::Error) -> NucleusError {
    NucleusError::MySqlConnection(format!("{}", e))
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
    }
}

/// Escape backticks in MySQL identifiers
fn escape_ident(name: &str) -> String {
    name.replace('`', "``")
}

/// Compress a directory tree into a zip file. All entries are prefixed with `root_name/`.
fn compress_directory_to_zip(
    dir_path: &Path,
    zip_path: &Path,
    root_name: &str,
) -> Result<u64, NucleusError> {
    let zip_file = std::fs::File::create(zip_path)?;
    let mut zip_writer = zip::ZipWriter::new(zip_file);
    let options = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // Add root directory entry
    zip_writer
        .add_directory(format!("{}/", root_name), options)
        .map_err(zip_err)?;

    let mut stack = vec![dir_path.to_path_buf()];
    while let Some(current) = stack.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&current)?
            .filter_map(|e| e.ok())
            .collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let path = entry.path();
            let relative = path
                .strip_prefix(dir_path)
                .map_err(|e| NucleusError::Internal(format!("Path prefix error: {}", e)))?;
            let zip_name = format!(
                "{}/{}",
                root_name,
                relative.to_string_lossy().replace('\\', "/")
            );

            if path.is_dir() {
                zip_writer
                    .add_directory(format!("{}/", zip_name), options)
                    .map_err(zip_err)?;
                stack.push(path);
            } else {
                zip_writer.start_file(&zip_name, options).map_err(zip_err)?;
                let data = std::fs::read(&path)?;
                zip_writer.write_all(&data)?;
            }
        }
    }

    zip_writer.finish().map_err(zip_err)?;
    let size_mb = std::fs::metadata(zip_path)?.len() / (1024 * 1024);
    Ok(size_mb)
}

/// Extract all entries from a zip archive into `out_dir`.
fn extract_backup_zip(zip_path: &Path, out_dir: &Path) -> Result<(), NucleusError> {
    let file = std::fs::File::open(zip_path)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(zip_err)?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(zip_err)?;
        let name = entry.name().to_string();
        let out_path = out_dir.join(&name);

        // Guard against zip-slip (path traversal)
        if !out_path.starts_with(out_dir) {
            continue;
        }

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out_file = std::fs::File::create(&out_path)?;
            std::io::copy(&mut entry, &mut out_file)?;
        }
    }

    Ok(())
}

/// Find the root directory of a structured backup (contains manifest.json).
fn find_backup_root(extract_dir: &Path) -> Option<PathBuf> {
    for entry in std::fs::read_dir(extract_dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() && path.join("manifest.json").exists() {
            return Some(path);
        }
    }
    // manifest.json might be at extract_dir itself
    if extract_dir.join("manifest.json").exists() {
        return Some(extract_dir.to_path_buf());
    }
    None
}

/// Find a legacy single .sql file in an extracted zip.
fn find_legacy_sql(extract_dir: &Path) -> Option<PathBuf> {
    for entry in std::fs::read_dir(extract_dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("sql") {
            return Some(path);
        }
    }
    None
}

// ── MySQL pool ───────────────────────────────────────────────────────────────

fn create_mysql_pool(config: &NucleusConfig) -> Result<mysql_async::Pool, NucleusError> {
    let opts = mysql_async::OptsBuilder::default()
        .ip_or_hostname(config.mysql_host.clone())
        .tcp_port(config.mysql_port)
        .user(Some(config.mysql_user.clone()))
        .pass(Some(config.mysql_password.clone()));

    Ok(mysql_async::Pool::new(opts))
}

// ── Metadata discovery ───────────────────────────────────────────────────────

async fn discover_databases(
    pool: &mysql_async::Pool,
) -> Result<Vec<String>, NucleusError> {
    let mut conn = pool.get_conn().await.map_err(mysql_err)?;
    let rows: Vec<(String,)> = conn
        .query("SHOW DATABASES")
        .await
        .map_err(mysql_err)?;
    Ok(rows
        .into_iter()
        .map(|(db,)| db)
        .filter(|db| !SYSTEM_DATABASES.contains(&db.as_str()))
        .collect())
}

async fn discover_tables_and_views(
    pool: &mysql_async::Pool,
    db: &str,
) -> Result<(Vec<String>, Vec<String>), NucleusError> {
    let mut conn = pool.get_conn().await.map_err(mysql_err)?;
    let rows: Vec<(String, String)> = conn
        .query(format!("SHOW FULL TABLES FROM `{}`", escape_ident(db)))
        .await
        .map_err(mysql_err)?;

    let mut tables = Vec::new();
    let mut views = Vec::new();
    for (name, table_type) in rows {
        match table_type.as_str() {
            "BASE TABLE" => tables.push(name),
            "VIEW" => views.push(name),
            _ => {}
        }
    }
    Ok((tables, views))
}

async fn discover_triggers(
    pool: &mysql_async::Pool,
    db: &str,
) -> Result<Vec<String>, NucleusError> {
    let mut conn = pool.get_conn().await.map_err(mysql_err)?;
    let rows: Vec<(String,)> = conn
        .exec(
            "SELECT TRIGGER_NAME FROM INFORMATION_SCHEMA.TRIGGERS WHERE TRIGGER_SCHEMA = ?",
            (db,),
        )
        .await
        .map_err(mysql_err)?;
    Ok(rows.into_iter().map(|(n,)| n).collect())
}

async fn discover_routines(
    pool: &mysql_async::Pool,
    db: &str,
) -> Result<Vec<(String, String)>, NucleusError> {
    let mut conn = pool.get_conn().await.map_err(mysql_err)?;
    let rows: Vec<(String, String)> = conn
        .exec(
            "SELECT ROUTINE_NAME, ROUTINE_TYPE FROM INFORMATION_SCHEMA.ROUTINES WHERE ROUTINE_SCHEMA = ?",
            (db,),
        )
        .await
        .map_err(mysql_err)?;
    Ok(rows)
}

async fn discover_events(
    pool: &mysql_async::Pool,
    db: &str,
) -> Result<Vec<String>, NucleusError> {
    let mut conn = pool.get_conn().await.map_err(mysql_err)?;
    let rows: Vec<(String,)> = conn
        .exec(
            "SELECT EVENT_NAME FROM INFORMATION_SCHEMA.EVENTS WHERE EVENT_SCHEMA = ?",
            (db,),
        )
        .await
        .map_err(mysql_err)?;
    Ok(rows.into_iter().map(|(n,)| n).collect())
}

// ── Per-object dump ──────────────────────────────────────────────────────────

/// Find the MySQL container name by checking common names.
async fn find_mysql_container() -> Option<String> {
    let candidates = ["mysql", "puru-mysql", "db", "puru-db"];
    for name in &candidates {
        let output = crate::process::silent_cmd("docker")
            .args(["inspect", "--format", "{{.State.Running}}", name])
            .output()
            .await;
        if let Ok(out) = output {
            if out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true" {
                return Some(name.to_string());
            }
        }
    }
    None
}

/// Check if mysqldump is available locally.
async fn has_local_mysqldump() -> bool {
    crate::process::silent_cmd("mysqldump")
        .arg("--version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Dump a single table via `mysqldump` — tries local binary first,
/// falls back to `docker exec` if mysqldump isn't installed locally.
async fn dump_table(
    config: &NucleusConfig,
    db: &str,
    table: &str,
    backup_type: &BackupType,
    output_path: &Path,
) -> Result<(), NucleusError> {
    let mut dump_args = vec![
        format!("-h{}", if has_local_mysqldump().await { config.mysql_host.clone() } else { "127.0.0.1".to_string() }),
        format!("-P{}", config.mysql_port),
        format!("-u{}", config.mysql_user),
        format!("-p{}", config.mysql_password),
        "--skip-triggers".to_string(),
        "--single-transaction".to_string(),
    ];

    if *backup_type == BackupType::Partial {
        dump_args.push("--no-data".to_string());
    }

    dump_args.push(db.to_string());
    dump_args.push(table.to_string());

    let output = if has_local_mysqldump().await {
        // Local mysqldump
        let mut args = dump_args.clone();
        // Use --result-file for local
        args.insert(5, format!("--result-file={}", output_path.display()));
        // Remove -p flag, use env var instead
        args.retain(|a| !a.starts_with("-p") || a.starts_with("-P"));
        crate::process::silent_cmd("mysqldump")
            .env("MYSQL_PWD", &config.mysql_password)
            .args(&args)
            .output()
            .await
            .map_err(|e| NucleusError::MySqlConnection(format!("Failed to run mysqldump: {}", e)))?
    } else {
        // Docker exec — find the MySQL container
        let container = find_mysql_container().await.ok_or_else(|| {
            NucleusError::MySqlConnection(
                "mysqldump not found locally and no running MySQL Docker container found. \
                 Install mysql-client or ensure the MySQL container is running.".to_string()
            )
        })?;

        // Run mysqldump inside the container, capture stdout to file
        let mut docker_args = vec![
            "exec".to_string(),
            container,
            "mysqldump".to_string(),
        ];
        // Inside container, connect to localhost
        let mut inner_args = vec![
            "-h127.0.0.1".to_string(),
            format!("-P{}", config.mysql_port),
            format!("-u{}", config.mysql_user),
            format!("-p{}", config.mysql_password),
            "--skip-triggers".to_string(),
            "--single-transaction".to_string(),
        ];
        if *backup_type == BackupType::Partial {
            inner_args.push("--no-data".to_string());
        }
        inner_args.push(db.to_string());
        inner_args.push(table.to_string());
        docker_args.extend(inner_args);

        let result = crate::process::silent_cmd("docker")
            .args(&docker_args)
            .output()
            .await
            .map_err(|e| NucleusError::MySqlConnection(format!("Failed to run docker exec mysqldump: {}", e)))?;

        // Write stdout to file (docker exec doesn't support --result-file)
        if result.status.success() {
            tokio::fs::write(output_path, &result.stdout).await?;
        }

        result
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NucleusError::MySqlConnection(format!(
            "mysqldump failed for {}.{}: {}",
            db,
            table,
            stderr.trim()
        )));
    }

    Ok(())
}

/// Extract CREATE VIEW DDL via mysql_async.
async fn dump_view_ddl(
    pool: &mysql_async::Pool,
    db: &str,
    name: &str,
) -> Result<String, NucleusError> {
    let mut conn = pool.get_conn().await.map_err(mysql_err)?;
    conn.query_drop(format!("USE `{}`", escape_ident(db)))
        .await
        .map_err(mysql_err)?;

    let row: Option<mysql_async::Row> = conn
        .query_first(format!("SHOW CREATE VIEW `{}`", escape_ident(name)))
        .await
        .map_err(mysql_err)?;

    let ddl: String = row
        .and_then(|r| r.get("Create View"))
        .ok_or_else(|| {
            NucleusError::MySqlConnection(format!("No DDL returned for view {}.{}", db, name))
        })?;

    Ok(format!(
        "-- View: {}\n-- Database: {}\n-- Generated by puru-nucleus backup\n\n{}\n",
        name, db, ddl
    ))
}

/// Extract CREATE TRIGGER DDL via mysql_async.
async fn dump_trigger_ddl(
    pool: &mysql_async::Pool,
    db: &str,
    name: &str,
) -> Result<String, NucleusError> {
    let mut conn = pool.get_conn().await.map_err(mysql_err)?;
    conn.query_drop(format!("USE `{}`", escape_ident(db)))
        .await
        .map_err(mysql_err)?;

    let row: Option<mysql_async::Row> = conn
        .query_first(format!("SHOW CREATE TRIGGER `{}`", escape_ident(name)))
        .await
        .map_err(mysql_err)?;

    let ddl: String = row
        .and_then(|r| r.get("SQL Original Statement"))
        .ok_or_else(|| {
            NucleusError::MySqlConnection(format!("No DDL returned for trigger {}.{}", db, name))
        })?;

    Ok(format!(
        "-- Trigger: {}\n-- Database: {}\n-- Generated by puru-nucleus backup\n\nDELIMITER ;;\n{};;\nDELIMITER ;\n",
        name, db, ddl
    ))
}

/// Extract CREATE PROCEDURE / CREATE FUNCTION DDL via mysql_async.
async fn dump_routine_ddl(
    pool: &mysql_async::Pool,
    db: &str,
    name: &str,
    routine_type: &str, // "PROCEDURE" or "FUNCTION"
) -> Result<String, NucleusError> {
    let mut conn = pool.get_conn().await.map_err(mysql_err)?;
    conn.query_drop(format!("USE `{}`", escape_ident(db)))
        .await
        .map_err(mysql_err)?;

    let show_cmd = format!("SHOW CREATE {} `{}`", routine_type, escape_ident(name));
    let row: Option<mysql_async::Row> = conn
        .query_first(show_cmd)
        .await
        .map_err(mysql_err)?;

    let col_name = format!("Create {}", capitalize(routine_type));
    let ddl: String = row
        .and_then(|r| r.get(col_name.as_str()))
        .ok_or_else(|| {
            NucleusError::MySqlConnection(format!(
                "No DDL returned for {} {}.{}",
                routine_type.to_lowercase(),
                db,
                name
            ))
        })?;

    let label = capitalize(routine_type);
    Ok(format!(
        "-- {}: {}\n-- Database: {}\n-- Generated by puru-nucleus backup\n\nDELIMITER ;;\n{};;\nDELIMITER ;\n",
        label, name, db, ddl
    ))
}

/// Extract CREATE EVENT DDL via mysql_async.
async fn dump_event_ddl(
    pool: &mysql_async::Pool,
    db: &str,
    name: &str,
) -> Result<String, NucleusError> {
    let mut conn = pool.get_conn().await.map_err(mysql_err)?;
    conn.query_drop(format!("USE `{}`", escape_ident(db)))
        .await
        .map_err(mysql_err)?;

    let row: Option<mysql_async::Row> = conn
        .query_first(format!("SHOW CREATE EVENT `{}`", escape_ident(name)))
        .await
        .map_err(mysql_err)?;

    let ddl: String = row
        .and_then(|r| r.get("Create Event"))
        .ok_or_else(|| {
            NucleusError::MySqlConnection(format!("No DDL returned for event {}.{}", db, name))
        })?;

    Ok(format!(
        "-- Event: {}\n-- Database: {}\n-- Generated by puru-nucleus backup\n\nDELIMITER ;;\n{};;\nDELIMITER ;\n",
        name, db, ddl
    ))
}

// ── Orchestrator ─────────────────────────────────────────────────────────────

/// Dump every database into a structured temp directory, returning its path and manifest.
async fn dump_all_databases(
    config: &NucleusConfig,
    backup_type: &BackupType,
    backup_name: &str,
) -> Result<(PathBuf, BackupManifest), NucleusError> {
    let temp_dir = std::env::temp_dir().join(backup_name);
    std::fs::create_dir_all(&temp_dir)?;

    let pool = create_mysql_pool(config)?;
    let mut manifest = BackupManifest {
        databases: vec![],
        total_objects: 0,
        failed_objects: 0,
        errors: vec![],
    };

    let databases = match discover_databases(&pool).await {
        Ok(dbs) => dbs,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&temp_dir);
            pool.disconnect().await.map_err(mysql_err)?;
            return Err(e);
        }
    };

    let mut total_tables_succeeded: usize = 0;

    for db in &databases {
        let db_dir = temp_dir.join(db);
        std::fs::create_dir_all(&db_dir)?;

        let mut db_info = DatabaseBackupInfo {
            name: db.clone(),
            tables: vec![],
            views: vec![],
            triggers: vec![],
            routines: vec![],
            events: vec![],
        };

        // ── Tables & Views ──
        match discover_tables_and_views(&pool, db).await {
            Ok((tables, views)) => {
                // Dump tables via mysqldump
                if !tables.is_empty() {
                    let tables_dir = db_dir.join("tables");
                    std::fs::create_dir_all(&tables_dir)?;
                    for table in &tables {
                        manifest.total_objects += 1;
                        let out_path = tables_dir.join(format!("{}.sql", table));
                        match dump_table(config, db, table, backup_type, &out_path).await {
                            Ok(()) => {
                                db_info.tables.push(table.clone());
                                total_tables_succeeded += 1;
                            }
                            Err(e) => {
                                manifest.failed_objects += 1;
                                let msg = format!("{}.{} (table): {}", db, table, e);
                                tracing::warn!("Skipping {}", msg);
                                manifest.errors.push(msg);
                            }
                        }
                    }
                }

                // Dump views via SHOW CREATE VIEW
                if !views.is_empty() {
                    let views_dir = db_dir.join("views");
                    std::fs::create_dir_all(&views_dir)?;
                    for view in &views {
                        manifest.total_objects += 1;
                        let out_path = views_dir.join(format!("{}.sql", view));
                        match dump_view_ddl(&pool, db, view).await {
                            Ok(ddl) => {
                                std::fs::write(&out_path, ddl)?;
                                db_info.views.push(view.clone());
                            }
                            Err(e) => {
                                manifest.failed_objects += 1;
                                let msg = format!("{}.{} (view): {}", db, view, e);
                                tracing::warn!("Skipping {}", msg);
                                manifest.errors.push(msg);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                let msg = format!("Failed to discover tables for {}: {}", db, e);
                tracing::warn!("{}", msg);
                manifest.errors.push(msg);
                continue;
            }
        }

        // ── Triggers ──
        match discover_triggers(&pool, db).await {
            Ok(triggers) => {
                if !triggers.is_empty() {
                    let triggers_dir = db_dir.join("triggers");
                    std::fs::create_dir_all(&triggers_dir)?;
                    for trigger in &triggers {
                        manifest.total_objects += 1;
                        let out_path = triggers_dir.join(format!("{}.sql", trigger));
                        match dump_trigger_ddl(&pool, db, trigger).await {
                            Ok(ddl) => {
                                std::fs::write(&out_path, ddl)?;
                                db_info.triggers.push(trigger.clone());
                            }
                            Err(e) => {
                                manifest.failed_objects += 1;
                                let msg = format!("{}.{} (trigger): {}", db, trigger, e);
                                tracing::warn!("Skipping {}", msg);
                                manifest.errors.push(msg);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                let msg = format!("Failed to discover triggers for {}: {}", db, e);
                tracing::warn!("{}", msg);
                manifest.errors.push(msg);
            }
        }

        // ── Routines (procedures + functions) ──
        match discover_routines(&pool, db).await {
            Ok(routines) => {
                if !routines.is_empty() {
                    let routines_dir = db_dir.join("routines");
                    std::fs::create_dir_all(&routines_dir)?;
                    for (routine_name, routine_type) in &routines {
                        manifest.total_objects += 1;
                        let out_path = routines_dir.join(format!("{}.sql", routine_name));
                        match dump_routine_ddl(&pool, db, routine_name, routine_type).await {
                            Ok(ddl) => {
                                std::fs::write(&out_path, ddl)?;
                                db_info.routines.push(routine_name.clone());
                            }
                            Err(e) => {
                                manifest.failed_objects += 1;
                                let msg = format!(
                                    "{}.{} ({}): {}",
                                    db,
                                    routine_name,
                                    routine_type.to_lowercase(),
                                    e
                                );
                                tracing::warn!("Skipping {}", msg);
                                manifest.errors.push(msg);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                let msg = format!("Failed to discover routines for {}: {}", db, e);
                tracing::warn!("{}", msg);
                manifest.errors.push(msg);
            }
        }

        // ── Events ──
        match discover_events(&pool, db).await {
            Ok(events) => {
                if !events.is_empty() {
                    let events_dir = db_dir.join("events");
                    std::fs::create_dir_all(&events_dir)?;
                    for event in &events {
                        manifest.total_objects += 1;
                        let out_path = events_dir.join(format!("{}.sql", event));
                        match dump_event_ddl(&pool, db, event).await {
                            Ok(ddl) => {
                                std::fs::write(&out_path, ddl)?;
                                db_info.events.push(event.clone());
                            }
                            Err(e) => {
                                manifest.failed_objects += 1;
                                let msg = format!("{}.{} (event): {}", db, event, e);
                                tracing::warn!("Skipping {}", msg);
                                manifest.errors.push(msg);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                let msg = format!("Failed to discover events for {}: {}", db, e);
                tracing::warn!("{}", msg);
                manifest.errors.push(msg);
            }
        }

        // Remove empty subdirs
        for subdir in &["tables", "views", "triggers", "routines", "events"] {
            let sub_path = db_dir.join(subdir);
            if sub_path.exists() {
                if std::fs::read_dir(&sub_path)
                    .map(|mut d| d.next().is_none())
                    .unwrap_or(false)
                {
                    let _ = std::fs::remove_dir(&sub_path);
                }
            }
        }

        // Remove empty db dir, otherwise record it in the manifest
        if std::fs::read_dir(&db_dir)
            .map(|mut d| d.next().is_none())
            .unwrap_or(false)
        {
            let _ = std::fs::remove_dir(&db_dir);
        } else {
            manifest.databases.push(db_info);
        }
    }

    // Disconnect pool
    pool.disconnect().await.map_err(mysql_err)?;

    // Fail if zero tables succeeded across all databases
    if total_tables_succeeded == 0 {
        let _ = std::fs::remove_dir_all(&temp_dir);
        return Err(NucleusError::MySqlConnection(
            "Backup failed: no tables were successfully dumped".into(),
        ));
    }

    // Write manifest.json
    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    std::fs::write(temp_dir.join("manifest.json"), manifest_json)?;

    Ok((temp_dir, manifest))
}

// ── MySQL restore ────────────────────────────────────────────────────────────

/// Pipe a .sql file into the `mysql` CLI. Optionally target a specific database.
/// Falls back to `docker exec` if mysql client isn't installed locally.
async fn run_mysql_restore(
    config: &NucleusConfig,
    sql_path: &Path,
    database: Option<&str>,
) -> Result<(), NucleusError> {
    let sql_data = tokio::fs::read(sql_path).await?;

    let mut mysql_args = vec![
        format!("-h127.0.0.1"),
        format!("-P{}", config.mysql_port),
        format!("-u{}", config.mysql_user),
        format!("-p{}", config.mysql_password),
    ];
    if let Some(db) = database {
        mysql_args.push(db.to_string());
    }

    let use_local = has_local_mysql().await;

    let mut child = if use_local {
        let mut args = mysql_args.clone();
        args.retain(|a| !a.starts_with("-p") || a.starts_with("-P"));
        args[0] = format!("-h{}", config.mysql_host);
        crate::process::silent_cmd("mysql")
            .env("MYSQL_PWD", &config.mysql_password)
            .args(&args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| NucleusError::MySqlConnection(format!("Failed to run mysql: {}", e)))?
    } else {
        let container = find_mysql_container().await.ok_or_else(|| {
            NucleusError::MySqlConnection(
                "mysql client not found locally and no running MySQL Docker container found.".to_string()
            )
        })?;
        let mut docker_args = vec!["exec".to_string(), "-i".to_string(), container, "mysql".to_string()];
        docker_args.extend(mysql_args);
        crate::process::silent_cmd("docker")
            .args(&docker_args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| NucleusError::MySqlConnection(format!("Failed to run docker exec mysql: {}", e)))?
    };

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&sql_data).await?;
    }

    let output = child.wait_with_output().await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NucleusError::MySqlConnection(format!(
            "mysql restore failed: {}",
            stderr.trim()
        )));
    }

    Ok(())
}

/// Check if mysql client is available locally.
async fn has_local_mysql() -> bool {
    crate::process::silent_cmd("mysql")
        .arg("--version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Execute a single SQL statement via the `mysql` CLI.
/// Falls back to `docker exec` if mysql client isn't installed locally.
async fn run_mysql_command(
    config: &NucleusConfig,
    sql: &str,
) -> Result<(), NucleusError> {
    let output = if has_local_mysql().await {
        crate::process::silent_cmd("mysql")
            .env("MYSQL_PWD", &config.mysql_password)
            .args([
                &format!("-h{}", config.mysql_host),
                &format!("-P{}", config.mysql_port),
                &format!("-u{}", config.mysql_user),
                "-e",
                sql,
            ])
            .output()
            .await
            .map_err(|e| NucleusError::MySqlConnection(format!("Failed to run mysql: {}", e)))?
    } else {
        let container = find_mysql_container().await.ok_or_else(|| {
            NucleusError::MySqlConnection(
                "mysql client not found locally and no running MySQL Docker container found.".to_string()
            )
        })?;
        crate::process::silent_cmd("docker")
            .args([
                "exec", &container, "mysql",
                &format!("-h127.0.0.1"),
                &format!("-P{}", config.mysql_port),
                &format!("-u{}", config.mysql_user),
                &format!("-p{}", config.mysql_password),
                "-e", sql,
            ])
            .output()
            .await
            .map_err(|e| NucleusError::MySqlConnection(format!("Failed to run docker exec mysql: {}", e)))?
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NucleusError::MySqlConnection(format!(
            "mysql command failed: {}",
            stderr.trim()
        )));
    }

    Ok(())
}

/// Restore a structured backup directory: create databases, then restore
/// tables → views → triggers → routines → events in order.
async fn restore_from_directory(
    config: &NucleusConfig,
    root_dir: &Path,
) -> Result<(), NucleusError> {
    for entry in std::fs::read_dir(root_dir)? {
        let entry = entry?;
        let path = entry.path();

        // Skip non-directories (e.g. manifest.json)
        if !path.is_dir() {
            continue;
        }

        let db_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| NucleusError::Internal("Invalid directory name in backup".into()))?;

        // CREATE DATABASE IF NOT EXISTS
        let create_sql = format!("CREATE DATABASE IF NOT EXISTS `{}`", escape_ident(db_name));
        run_mysql_command(config, &create_sql).await?;

        // Restore objects in dependency order
        for subdir in &["tables", "views", "triggers", "routines", "events"] {
            let sub_path = path.join(subdir);
            if !sub_path.exists() {
                continue;
            }

            let mut files: Vec<_> = std::fs::read_dir(&sub_path)?
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        == Some("sql")
                })
                .collect();
            files.sort_by_key(|e| e.file_name());

            for file_entry in files {
                let file_path = file_entry.path();
                let file_name = file_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();

                if let Err(e) = run_mysql_restore(config, &file_path, Some(db_name)).await {
                    tracing::warn!(
                        "Failed to restore {}/{}/{}: {}",
                        db_name,
                        subdir,
                        file_name,
                        e
                    );
                    // Continue on per-file failure
                }
            }
        }
    }

    Ok(())
}

// ── GCS operations ───────────────────────────────────────────────────────────

async fn create_gcs_client(
    credentials_path: &str,
) -> Result<google_cloud_storage::client::Client, NucleusError> {
    let cred_file =
        google_cloud_auth::credentials::CredentialsFile::new_from_file(credentials_path.to_string())
            .await
            .map_err(|e| NucleusError::GcsConnection(format!("Failed to load credentials: {}", e)))?;

    let client_config = google_cloud_storage::client::ClientConfig::default()
        .with_credentials(cred_file)
        .await
        .map_err(|e| NucleusError::GcsConnection(format!("Failed to configure GCS client: {}", e)))?;

    Ok(google_cloud_storage::client::Client::new(client_config))
}

async fn upload_to_gcs(
    client: &google_cloud_storage::client::Client,
    local_path: &Path,
    object_name: &str,
) -> Result<(), NucleusError> {
    let data = tokio::fs::read(local_path).await?;

    let upload_type = google_cloud_storage::http::objects::upload::UploadType::Simple(
        google_cloud_storage::http::objects::upload::Media::new(object_name.to_string()),
    );

    client
        .upload_object(
            &google_cloud_storage::http::objects::upload::UploadObjectRequest {
                bucket: GCS_BUCKET.to_string(),
                ..Default::default()
            },
            data,
            &upload_type,
        )
        .await
        .map_err(|e| NucleusError::GcsConnection(format!("GCS upload failed: {}", e)))?;

    Ok(())
}

async fn download_from_gcs(
    client: &google_cloud_storage::client::Client,
    object_name: &str,
    local_path: &Path,
) -> Result<(), NucleusError> {
    let data = client
        .download_object(
            &google_cloud_storage::http::objects::get::GetObjectRequest {
                bucket: GCS_BUCKET.to_string(),
                object: object_name.to_string(),
                ..Default::default()
            },
            &google_cloud_storage::http::objects::download::Range::default(),
        )
        .await
        .map_err(|e| NucleusError::GcsConnection(format!("GCS download failed: {}", e)))?;

    if let Some(parent) = local_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(local_path, data).await?;

    Ok(())
}

// ── LAN copy ─────────────────────────────────────────────────────────────────

/// Copy a backup zip to a LAN network share (NFS/SMB mount).
/// Uses atomic write: `.tmp` → rename to prevent partial files.
fn copy_zip_to_lan(
    zip_path: &Path,
    lan_path: &str,
    hospital_code: &str,
    backup_name: &str,
) -> Result<(), NucleusError> {
    let dest_dir = PathBuf::from(lan_path)
        .join(hospital_code)
        .join("backups");
    std::fs::create_dir_all(&dest_dir)?;

    let dest_file = dest_dir.join(backup_zip_filename(backup_name));
    let tmp_file = dest_dir.join(format!("{}.zip.tmp", backup_name));

    std::fs::copy(zip_path, &tmp_file)?;
    std::fs::rename(&tmp_file, &dest_file)?;

    tracing::info!("Backup copied to LAN: {}", dest_file.display());
    Ok(())
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Start a backup: dump per-object, zip, upload to GCS.
pub async fn start_backup(
    backup_type: BackupType,
    license: &crate::licensing::License,
) -> Result<BackupResult, NucleusError> {
    // 1. Validate license
    if !license.is_valid() {
        return Err(NucleusError::LicenseExpired(
            license.valid_till.format("%Y-%m-%d").to_string(),
        ));
    }

    // 2. Load config and validate
    let cfg = config::load_config()?;
    if cfg.hospital_code.is_empty() {
        return Err(NucleusError::InvalidConfig(
            "Hospital code is not set. Configure it in Settings.".into(),
        ));
    }
    if cfg.mysql_password.is_empty() {
        return Err(NucleusError::MySqlConnection(
            "MySQL password is not configured. Set it in Settings.".into(),
        ));
    }

    // 3. Generate backup name and zip path
    let backup_name = generate_backup_name(&cfg.hospital_code);
    let zip_path = backups_dir().join(backup_zip_filename(&backup_name));

    tracing::info!("Starting {:?} backup: {}", backup_type, backup_name);
    let start = std::time::Instant::now();

    // 4. Add in-progress record
    add_record(BackupRecord {
        id: backup_name.clone(),
        backup_type: backup_type.clone(),
        status: BackupStatus::InProgress,
        size_mb: 0,
        created_at: Utc::now(),
        uploaded: false,
        lan_copied: false,
        error: None,
    })?;

    // 5. Dump all databases into a structured temp directory
    let (temp_dir, _manifest) =
        match dump_all_databases(&cfg, &backup_type, &backup_name).await {
            Ok(result) => result,
            Err(e) => {
                let _ = update_record_with_error(&backup_name, BackupStatus::Failed, 0, false, false, Some(e.to_string()));
                return Err(e);
            }
        };

    // 6. Compress temp directory → .zip, then remove temp dir
    let size_mb = match compress_directory_to_zip(&temp_dir, &zip_path, &backup_name) {
        Ok(size) => {
            let _ = std::fs::remove_dir_all(&temp_dir);
            size
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&temp_dir);
            let _ = update_record_with_error(&backup_name, BackupStatus::Failed, 0, false, false, Some(e.to_string()));
            return Err(e);
        }
    };

    // 7. Upload .zip to GCS if credentials configured
    let mut uploaded = false;
    if let Some(ref cred_path) = cfg.gcs_credentials_path {
        if !cred_path.is_empty() {
            let object_path = gcs_object_path(&cfg.hospital_code, &backup_name);
            match create_gcs_client(cred_path).await {
                Ok(client) => match upload_to_gcs(&client, &zip_path, &object_path).await {
                    Ok(()) => {
                        uploaded = true;
                        tracing::info!("Backup uploaded to GCS: {}", object_path);
                    }
                    Err(e) => {
                        tracing::warn!("GCS upload failed (backup saved locally): {}", e);
                    }
                },
                Err(e) => {
                    tracing::warn!("GCS client init failed (backup saved locally): {}", e);
                }
            }
        }
    }

    // 7b. Copy to LAN if configured
    let mut lan_copied = false;
    if cfg.lan.enabled && !cfg.lan.path.is_empty() {
        match copy_zip_to_lan(&zip_path, &cfg.lan.path, &cfg.hospital_code, &backup_name) {
            Ok(()) => lan_copied = true,
            Err(e) => tracing::warn!("LAN copy failed (backup saved locally): {}", e),
        }
    }

    // 8. Update record to completed
    update_record(&backup_name, BackupStatus::Completed, size_mb, uploaded, lan_copied)?;

    let duration = start.elapsed().as_secs();
    tracing::info!(
        "Backup {} completed: {}MB, uploaded={}, {}s",
        backup_name,
        size_mb,
        uploaded,
        duration
    );

    Ok(BackupResult {
        success: true,
        backup_id: backup_name,
        size_mb,
        duration_seconds: duration,
    })
}

/// Get backup history sorted by date descending
pub async fn get_backup_history() -> Result<Vec<BackupRecord>, NucleusError> {
    let mut records = load_history()?;
    records.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(records)
}

/// Restore a backup by ID (supports both structured and legacy single-file backups)
pub async fn restore_backup(backup_id: &str) -> Result<(), NucleusError> {
    let cfg = config::load_config()?;

    // 1. Find record in history
    let records = load_history()?;
    let record = records
        .iter()
        .find(|r| r.id == backup_id)
        .ok_or_else(|| NucleusError::NotFound(format!("Backup '{}'", backup_id)))?;

    if record.status != BackupStatus::Completed {
        return Err(NucleusError::InvalidConfig(format!(
            "Backup '{}' is not in completed state (status: {:?})",
            backup_id, record.status
        )));
    }

    // 2. Check local zip file
    let zip_path = backups_dir().join(backup_zip_filename(backup_id));

    if !zip_path.exists() {
        // 3. Try downloading from GCS
        let mut downloaded = false;
        if let Some(ref cred_path) = cfg.gcs_credentials_path {
            if !cred_path.is_empty() {
                let object_path = gcs_object_path(&cfg.hospital_code, backup_id);
                tracing::info!("Downloading backup from GCS: {}", object_path);
                match create_gcs_client(cred_path).await {
                    Ok(client) => match download_from_gcs(&client, &object_path, &zip_path).await {
                        Ok(()) => downloaded = true,
                        Err(e) => tracing::warn!("GCS download failed: {}", e),
                    },
                    Err(e) => tracing::warn!("GCS client init failed: {}", e),
                }
            }
        }

        // 3b. Fallback: try LAN path
        if !downloaded && cfg.lan.enabled && !cfg.lan.path.is_empty() {
            let lan_src = PathBuf::from(&cfg.lan.path)
                .join(&cfg.hospital_code)
                .join("backups")
                .join(backup_zip_filename(backup_id));
            if lan_src.exists() {
                tracing::info!("Restoring backup from LAN: {}", lan_src.display());
                std::fs::copy(&lan_src, &zip_path)?;
                downloaded = true;
            }
        }

        if !downloaded {
            return Err(NucleusError::InvalidConfig(
                "Backup file not found locally, on GCS, or on LAN path.".into(),
            ));
        }
    }

    // 4. Extract zip to a temp directory
    let extract_dir = std::env::temp_dir().join(format!("puru-restore-{}", backup_id));
    if extract_dir.exists() {
        std::fs::remove_dir_all(&extract_dir)?;
    }
    std::fs::create_dir_all(&extract_dir)?;

    extract_backup_zip(&zip_path, &extract_dir)?;

    // 5. Detect structured vs legacy backup and restore accordingly
    let restore_result = if let Some(root) = find_backup_root(&extract_dir) {
        // Structured backup — restore per-database, per-object
        tracing::info!("Restoring structured backup: {}", backup_id);
        restore_from_directory(&cfg, &root).await
    } else if let Some(sql_path) = find_legacy_sql(&extract_dir) {
        // Legacy single-file backup
        tracing::info!("Restoring legacy backup: {}", backup_id);
        run_mysql_restore(&cfg, &sql_path, None).await
    } else {
        Err(NucleusError::NotFound(
            "No restorable content found in backup zip".into(),
        ))
    };

    // 6. Clean up extracted files
    let _ = std::fs::remove_dir_all(&extract_dir);

    restore_result?;

    tracing::info!("Backup {} restored successfully", backup_id);
    Ok(())
}
