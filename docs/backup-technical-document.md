# Puru Nucleus Backup System -- Technical Document

## 1. Overview

Puru Nucleus provides a multi-tier backup system for hospital MySQL databases deployed on-premises. The system supports:

- **Full Backups** -- per-object structured MySQL dumps (schema + data) compressed into ZIP archives
- **Partial Backups** -- schema-only dumps (no row data)
- **Binlog Shipping** -- continuous replication of MySQL binary logs for point-in-time recovery

Each backup can be stored in up to three locations:

| Tier | Medium | Requires Internet | Purpose |
|------|--------|-------------------|---------|
| Local | Host filesystem | No | Immediate restore, primary copy |
| GCS | Google Cloud Storage bucket | Yes | Off-site cloud backup |
| LAN | NFS/SMB network share | No | On-premises redundancy without internet |

---

## 2. Architecture

### 2.1 Source Files

```
src-tauri/src/
  backup/
    mod.rs          # Core backup/restore logic (dump, compress, upload, copy, history)
    binlog.rs       # Binlog shipping to LAN (MySQL binary log replication)
  config/mod.rs     # NucleusConfig, DaemonConfig, BackupSchedule, LanConfig
  commands/mod.rs   # Tauri IPC command handlers (GUI bridge)
  daemon/
    mod.rs          # Axum HTTP server setup
    routes.rs       # REST API route handlers
    scheduler.rs    # Background task loops (backup, binlog, watchdog, etc.)
    commands.rs     # Firestore remote command dispatcher
  cli.rs            # Clap CLI argument definitions
  cli_runner.rs     # CLI command implementations
  licensing/mod.rs  # License validation (feature gating)
  error.rs          # NucleusError enum
```

### 2.2 Trigger Pathways

A backup can be triggered through five pathways, all converging on the same `start_backup()` function:

```
                  +------------------+
                  | start_backup()   |
                  | backup/mod.rs    |
                  +--------+---------+
                           ^
           +-------+-------+-------+--------+
           |       |       |       |        |
        GUI    CLI     Daemon   REST API  Firestore
       (IPC)  (clap)  (scheduler) (Axum)  (remote cmd)
```

| Pathway | Entry Point | Trigger |
|---------|-------------|---------|
| **GUI** | `commands::start_backup` | User clicks "Full Backup" / "Partial Backup" in Angular UI |
| **CLI** | `cli_runner::cmd_backup` | `puru backup full` or `puru backup partial` |
| **Daemon Scheduler** | `scheduler::backup_scheduler` | Automatic periodic loop (default: every 24h) |
| **REST API** | `routes::trigger_backup` | `POST /api/backup {"type":"full"}` |
| **Firestore Command** | `commands::execute("trigger_backup")` | Admin pushes command to `hospital/{code}/commands` |

### 2.3 Data Structures

```rust
// --- Backup Record (persisted in backups.toml) ---
pub struct BackupRecord {
    pub id: String,              // e.g. "BTCT-PURU-02-03-2026-02-15-AM"
    pub backup_type: BackupType, // Full | Partial
    pub status: BackupStatus,    // InProgress | Completed | Failed
    pub size_mb: u64,
    pub created_at: DateTime<Utc>,
    pub uploaded: bool,          // GCS upload succeeded
    pub lan_copied: bool,        // LAN copy succeeded
}

// --- Backup Result (returned to caller) ---
pub struct BackupResult {
    pub success: bool,
    pub backup_id: String,
    pub size_mb: u64,
    pub duration_seconds: u64,
}

// --- Backup Manifest (written inside ZIP) ---
pub struct BackupManifest {
    pub databases: Vec<DatabaseBackupInfo>,
    pub total_objects: usize,
    pub failed_objects: usize,
    pub errors: Vec<String>,
}

pub struct DatabaseBackupInfo {
    pub name: String,
    pub tables: Vec<String>,
    pub views: Vec<String>,
    pub triggers: Vec<String>,
    pub routines: Vec<String>,
    pub events: Vec<String>,
}
```

---

## 3. Backup Process -- Step by Step

### 3.1 Full Backup Flow

The following describes every step that occurs when `start_backup(BackupType::Full, &license)` is called.

#### Step 1: License Validation

```rust
if !license.is_valid() {
    return Err(NucleusError::LicenseExpired(...));
}
```

The license is loaded from `{config_dir}/license.json`. The `valid_till` timestamp is compared against `Utc::now()`.

#### Step 2: Configuration Validation

```rust
let cfg = config::load_config()?;  // reads {config_dir}/nucleus.toml
```

Validates:
- `hospital_code` is non-empty (used for directory naming and GCS paths)
- `mysql_password` is non-empty (required for mysqldump)

#### Step 3: Generate Backup Name

```rust
fn generate_backup_name(hospital_code: &str) -> String {
    // Format: "{CODE}-PURU-DD-MM-YYYY-HH-MM-AM/PM"
    let now = Utc::now();
    format!("{}-PURU-{}-{}", hospital_code, now.format("%d-%m-%Y"), now.format("%I-%M-%p"))
}
```

**Example:** Hospital code `BTCT` at 2:15 AM on March 2, 2026 produces:
```
BTCT-PURU-02-03-2026-02-15-AM
```

The ZIP file will be stored at:
```
{config_dir}/backups/BTCT-PURU-02-03-2026-02-15-AM.zip
```

#### Step 4: Add In-Progress Record

A `BackupRecord` with `status: InProgress` is appended to `{config_dir}/backups.toml`:

```toml
[[records]]
id = "BTCT-PURU-02-03-2026-02-15-AM"
type = "full"
status = "in_progress"
size_mb = 0
created_at = "2026-03-02T02:15:00Z"
uploaded = false
lan_copied = false
```

This allows the UI to show a backup is in progress.

#### Step 5: Dump All Databases

**5a.** Create temp directory: `{system_temp}/BTCT-PURU-02-03-2026-02-15-AM/`

**5b.** Connect to MySQL using `mysql_async::Pool`:
```rust
let opts = mysql_async::OptsBuilder::default()
    .ip_or_hostname(config.mysql_host.clone())  // "127.0.0.1"
    .tcp_port(config.mysql_port)                // 3306
    .user(Some(config.mysql_user.clone()))       // "root"
    .pass(Some(config.mysql_password.clone()));  // "secret"
```

**5c.** Discover databases via `SHOW DATABASES`, filtering out system databases:
```rust
const SYSTEM_DATABASES: &[&str] = &["information_schema", "performance_schema", "mysql", "sys"];
```

Typical hospital databases: `puru_im`, `puru_has`, `puru_med`, `puru_dicom`, `puru_path`, `puru_bridge`, `puru_auth`, `puru_gh`.

**5d.** For each database, create a subdirectory and dump five object types:

| Object Type | Discovery Query | Dump Method | Output Path |
|-------------|----------------|-------------|-------------|
| Tables | `SHOW FULL TABLES` (type=BASE TABLE) | `mysqldump` CLI with `--skip-triggers --single-transaction` | `{db}/tables/{name}.sql` |
| Views | `SHOW FULL TABLES` (type=VIEW) | `SHOW CREATE VIEW` via mysql_async | `{db}/views/{name}.sql` |
| Triggers | `INFORMATION_SCHEMA.TRIGGERS` | `SHOW CREATE TRIGGER` via mysql_async | `{db}/triggers/{name}.sql` |
| Routines | `INFORMATION_SCHEMA.ROUTINES` | `SHOW CREATE PROCEDURE/FUNCTION` via mysql_async | `{db}/routines/{name}.sql` |
| Events | `INFORMATION_SCHEMA.EVENTS` | `SHOW CREATE EVENT` via mysql_async | `{db}/events/{name}.sql` |

For **partial backups**, the `--no-data` flag is added to mysqldump, so only table schemas are exported.

Each object is dumped independently. If one object fails, it is recorded in `manifest.errors` and skipped -- the backup continues with remaining objects.

**5e.** Write `manifest.json` at the temp directory root.

**5f.** If zero tables succeeded across all databases, the backup is marked as failed.

#### Step 6: Compress to ZIP

```rust
fn compress_directory_to_zip(dir_path: &Path, zip_path: &Path, root_name: &str) -> Result<u64>
```

- Uses `zip` crate with Deflate compression
- All entries are prefixed with the backup name (e.g. `BTCT-PURU-02-03-2026-02-15-AM/puru_has/tables/patient.sql`)
- The temp directory is removed after compression
- Returns size in MB

#### Step 7: Upload to GCS (Cloud)

Only executes if `gcs_credentials_path` is configured and non-empty.

```
GCS Bucket:  puru-automated-backup
Object Path: {hospital_code}/{year}/{month_name}/{backup_name}.zip
Example:     BTCT/2026/March/BTCT-PURU-02-03-2026-02-15-AM.zip
```

Uses `google-cloud-storage` crate with service account credentials. This step is **non-fatal** -- if GCS upload fails, the backup is still saved locally and the error is logged.

#### Step 7b: Copy to LAN (Network Share)

Only executes if `cfg.lan.enabled == true` and `cfg.lan.path` is non-empty.

```
LAN Path:    {lan_path}/{hospital_code}/backups/{backup_name}.zip
Example:     /mnt/nas/backups/BTCT/backups/BTCT-PURU-02-03-2026-02-15-AM.zip
```

Uses **atomic write** pattern:
1. Copy ZIP to `{backup_name}.zip.tmp`
2. Rename `.tmp` to `.zip`

This prevents partial files on the network share if the copy is interrupted. This step is **non-fatal**.

#### Step 8: Update Record

```rust
update_record(&backup_name, BackupStatus::Completed, size_mb, uploaded, lan_copied)?;
```

The `backups.toml` record is updated with final status, actual size, and upload/copy results.

### 3.2 Complete Example Timeline

```
T+0.0s   License validated (valid until 2027-01-01)
T+0.0s   Config loaded: hospital_code=BTCT, mysql=root@127.0.0.1:3306
T+0.0s   Backup name generated: BTCT-PURU-02-03-2026-02-15-AM
T+0.0s   In-progress record written to backups.toml
T+0.1s   Connected to MySQL, discovered 8 databases
T+0.2s   Dumping puru_has: 45 tables, 3 views, 12 triggers, 8 routines, 1 event
T+5.0s   Dumping puru_auth: 5 tables, 0 views, 2 triggers, 0 routines, 0 events
T+5.5s   Dumping puru_med: 22 tables, 1 view, 5 triggers, 3 routines, 0 events
T+8.0s   ... (remaining 5 databases)
T+12.0s  manifest.json written: 180 objects, 0 failed
T+12.0s  MySQL pool disconnected
T+12.1s  Compressing to ZIP: BTCT-PURU-02-03-2026-02-15-AM.zip
T+15.0s  ZIP created: 48 MB, temp directory removed
T+15.1s  Uploading to GCS: BTCT/2026/March/BTCT-PURU-02-03-2026-02-15-AM.zip
T+25.0s  GCS upload complete
T+25.1s  Copying to LAN: /mnt/nas/backups/BTCT/backups/BTCT-PURU-02-03-2026-02-15-AM.zip
T+28.0s  LAN copy complete
T+28.0s  Record updated: Completed, 48 MB, uploaded=true, lan_copied=true
T+28.0s  Return: BackupResult { success: true, backup_id: "BTCT-PURU-...", size_mb: 48, duration_seconds: 28 }
```

---

## 4. Restore Process -- Step by Step

Function: `restore_backup(backup_id)` in `backup/mod.rs`

### Step 1: Find Record

Looks up `backup_id` in `backups.toml`. Validates status is `Completed`.

### Step 2: Locate ZIP File

Checks local path first: `{config_dir}/backups/{backup_id}.zip`

### Step 3: Download if Not Local

Three-tier fallback:

```
Local file exists?
  YES -> use it
  NO  -> Try GCS download
           Success? -> use downloaded file
           Fail?    -> Try LAN path
                        Exists? -> copy from LAN to local
                        No?     -> Error: "not found locally, on GCS, or on LAN"
```

GCS path: `{hospital_code}/{year}/{month}/{backup_id}.zip`
LAN path: `{lan_path}/{hospital_code}/backups/{backup_id}.zip`

### Step 4: Extract ZIP

Extracts to `{system_temp}/puru-restore-{backup_id}/`. Includes **zip-slip protection** -- path traversal entries are skipped.

### Step 5: Detect Format and Restore

**Structured backup** (contains `manifest.json`):
- For each database subdirectory:
  1. `CREATE DATABASE IF NOT EXISTS \`{db_name}\``
  2. Restore in dependency order: tables -> views -> triggers -> routines -> events
  3. Each `.sql` file piped into `mysql` CLI targeting the specific database

**Legacy single-file backup** (contains a `.sql` file):
- The entire SQL file is piped into `mysql` without specifying a database

### Step 6: Cleanup

The extracted temp directory is removed regardless of success or failure.

---

## 5. Binlog Shipping to LAN

### 5.1 Purpose

Binary log (binlog) shipping provides **point-in-time recovery**. While full backups are periodic snapshots, binlogs capture every write transaction between snapshots. By shipping binlogs to a LAN share, a hospital can recover to any point in time even without internet.

### 5.2 Prerequisites

| Requirement | Config Field | Check |
|-------------|-------------|-------|
| License feature | `license.features.binlog_shipping` | `license.can_use_binlog_shipping()` |
| Hospital code set | `hospital_code` | Non-empty |
| LAN enabled | `lan.enabled` | `true` |
| LAN path set | `lan.path` | Non-empty |
| LAN binlog enabled | `lan.binlog_enabled` | `true` |
| MySQL binlog enabled | MySQL server config | `SHOW MASTER STATUS` returns rows |

### 5.3 Process

```rust
pub async fn ship_binlogs_to_lan(license: &License) -> Result<BinlogShipResult, NucleusError>
```

1. **Load state** from `{config_dir}/binlog-lan-state.json`
2. **Connect to MySQL** and query:
   - `SHOW MASTER STATUS` -> current active binlog file + position
   - `SHOW BINARY LOGS` -> list all binlog files with sizes
3. **Determine files to ship:**
   - All files **after** `last_shipped_file` (lexicographic comparison)
   - **Excluding** the current active master file (it's still being written to)
   - If never shipped before: all files except the current active one
4. **For each file:**
   - Read raw data via `mysqlbinlog --read-from-remote-server --raw --result-file=-`
   - Gzip compress using `flate2`
   - Atomic write to LAN: write `.gz.tmp`, then rename to `.gz`
   - Update state: `last_shipped_file`, `last_shipped_at`, `total_shipped`
5. **Save state** to `binlog-lan-state.json`
6. **Disconnect** MySQL pool

**Destination:** `{lan_path}/{hospital_code}/binlogs/{binlog_filename}.gz`

**Example:**
```
/mnt/nas/backups/BTCT/binlogs/mysql-bin.000001.gz
/mnt/nas/backups/BTCT/binlogs/mysql-bin.000002.gz
/mnt/nas/backups/BTCT/binlogs/mysql-bin.000003.gz
```

### 5.4 State Persistence

File: `{config_dir}/binlog-lan-state.json`

```json
{
  "last_shipped_file": "mysql-bin.000042",
  "last_shipped_at": "2026-03-02T14:30:00Z",
  "total_shipped": 42,
  "last_error": null
}
```

### 5.5 No Retention Purge

Unlike GCS binlog shipping (which purges entries older than `retain_hours`), LAN binlog shipping keeps **all** files permanently. The rationale is that LAN/NAS storage is cheap and the hospital may need deep recovery.

### 5.6 Trigger Pathways

| Pathway | Entry Point | When |
|---------|-------------|------|
| Daemon scheduler | `lan_binlog_shipping_loop()` | Periodic (same interval as backup schedule, min 1h) |
| GUI | `commands::ship_binlogs_lan` | User clicks "Ship to LAN Now" button |
| CLI | `puru binlog lan-ship` | Manual command |
| REST API | `POST /api/lan/binlog/ship` | Remote trigger |

---

## 6. Storage Layout

### 6.1 Local Filesystem

```
{config_dir}/                           # /etc/puru-nucleus/ (Linux)
                                        # C:\PuruNucleus\ (Windows)
  nucleus.toml                          # Main configuration
  license.json                          # License file
  gcs-credentials.json                  # GCS service account (optional)
  backups.toml                          # Backup history records
  binlog-lan-state.json                 # LAN binlog shipping state
  backups/
    BTCT-PURU-02-03-2026-02-15-AM.zip   # Full backup
    BTCT-PURU-01-03-2026-10-30-PM.zip   # Another backup
```

### 6.2 Inside a Backup ZIP

```
BTCT-PURU-02-03-2026-02-15-AM/
  manifest.json                         # Backup manifest (databases, objects, errors)
  puru_has/
    tables/
      patient.sql                       # mysqldump output (CREATE TABLE + INSERT)
      appointment.sql
      ward.sql
      ...
    views/
      v_patient_summary.sql             # SHOW CREATE VIEW output
    triggers/
      trg_patient_audit.sql             # SHOW CREATE TRIGGER output
    routines/
      sp_calculate_bill.sql             # SHOW CREATE PROCEDURE output
      fn_patient_age.sql                # SHOW CREATE FUNCTION output
    events/
      evt_daily_cleanup.sql             # SHOW CREATE EVENT output
  puru_auth/
    tables/
      users.sql
      roles.sql
  puru_med/
    tables/
      ...
  ...
```

### 6.3 GCS (Cloud)

```
gs://puru-automated-backup/
  BTCT/
    2026/
      January/
        BTCT-PURU-15-01-2026-11-00-PM.zip
      February/
        BTCT-PURU-14-02-2026-11-00-PM.zip
      March/
        BTCT-PURU-02-03-2026-02-15-AM.zip
  HOSP2/
    2026/
      March/
        HOSP2-PURU-02-03-2026-03-00-AM.zip
```

### 6.4 LAN (Network Share)

```
/mnt/nas/backups/                       # or Z:\backups\ on Windows
  BTCT/
    backups/
      BTCT-PURU-02-03-2026-02-15-AM.zip
      BTCT-PURU-01-03-2026-10-30-PM.zip
    binlogs/
      mysql-bin.000001.gz
      mysql-bin.000002.gz
      mysql-bin.000003.gz
      ...
```

---

## 7. Configuration Reference

### 7.1 TOML Configuration

File: `{config_dir}/nucleus.toml`

```toml
# --- Core settings ---
hospital_code = "BTCT"
server_ip = "192.168.1.100"
docker_compose_path = "/home/puru/docker/docker-compose.yml"
mysql_host = "127.0.0.1"
mysql_port = 3306
mysql_user = "root"
mysql_password = "hospital_db_pass"

# --- Cloud settings ---
gcs_credentials_path = "/etc/puru-nucleus/gcs-credentials.json"
backup_enabled = true
telemetry_enabled = true

# --- Daemon settings ---
[daemon]
port = 9090
api_key = "my-secret-api-key"
telemetry_interval_minutes = 15

[daemon.backup_schedule]
enabled = true
interval_hours = 24
backup_type = "full"

# --- LAN Backup settings ---
[lan]
enabled = true
path = "/mnt/nas/backups"
binlog_enabled = true
```

### 7.2 Configuration Fields

| Section | Field | Type | Default | Description |
|---------|-------|------|---------|-------------|
| (root) | `hospital_code` | String | `""` | Hospital identifier, used in all paths |
| (root) | `mysql_host` | String | `"127.0.0.1"` | MySQL server host |
| (root) | `mysql_port` | u16 | `3306` | MySQL server port |
| (root) | `mysql_user` | String | `"root"` | MySQL user for backup |
| (root) | `mysql_password` | String | `""` | MySQL password |
| (root) | `gcs_credentials_path` | Option | `None` | Path to GCS service account JSON |
| (root) | `backup_enabled` | bool | `true` | Enable cloud backup upload |
| `[daemon.backup_schedule]` | `enabled` | bool | `true` | Enable automatic backups |
| `[daemon.backup_schedule]` | `interval_hours` | u32 | `24` | Hours between automatic backups |
| `[daemon.backup_schedule]` | `backup_type` | String | `"full"` | `"full"` or `"partial"` |
| `[lan]` | `enabled` | bool | `false` | Enable LAN backup copy |
| `[lan]` | `path` | String | `""` | Mounted NFS/SMB share path |
| `[lan]` | `binlog_enabled` | bool | `false` | Enable binlog shipping to LAN |

---

## 8. API Reference

### 8.1 Tauri IPC Commands (GUI)

| Command | Parameters | Returns | Purpose |
|---------|-----------|---------|---------|
| `start_backup` | `{ type: "full" \| "partial" }` | `BackupResult` | Trigger backup |
| `get_backup_history` | none | `BackupRecord[]` | List all backups |
| `restore_backup` | `{ backupId: string }` | void | Restore a backup |
| `validate_lan_path` | `{ path: string }` | void or error | Validate LAN path is writable |
| `ship_binlogs_lan` | none | `BinlogShipResult` | Ship binlogs to LAN now |
| `get_binlog_status` | none | `BinlogStatus` | Get binlog shipping status |

### 8.2 REST API (Daemon)

| Method | Path | Body | Response | Auth |
|--------|------|------|----------|------|
| `POST` | `/api/backup` | `{"type": "full"}` | `BackupResult` | API key |
| `GET` | `/api/backup/list` | -- | `BackupRecord[]` | API key |
| `GET` | `/api/backup/schedule` | -- | `BackupSchedule` | API key |
| `PUT` | `/api/backup/schedule` | `{"enabled":true, "interval_hours":12}` | `BackupSchedule` | API key |
| `POST` | `/api/restore` | `{"backup_id":"..."}` or `{"latest":true}` | `{"ok":true}` | API key |
| `POST` | `/api/lan/binlog/ship` | -- | `BinlogShipResult` | API key |

### 8.3 CLI Commands

```bash
# Backup
puru backup                    # Full backup (default)
puru backup full               # Explicit full backup
puru backup partial            # Schema-only backup
puru backup list               # Show backup history table

# Restore
puru restore <backup_id>       # Restore specific backup
puru restore --latest          # Restore most recent completed backup

# Binlog
puru binlog lan-ship           # Ship binlogs to LAN now
puru binlog status             # Show binlog shipping status
```

---

## 9. Daemon Background Tasks

The daemon (`puru daemon`) spawns six background tasks via `scheduler::start_all()`:

| # | Task | Interval | Purpose |
|---|------|----------|---------|
| 1 | `backup_scheduler` | `interval_hours` (default 24h) | Automatic periodic backups |
| 2 | `status_reporter` | `telemetry_interval_minutes` (default 15m) | Push telemetry + backup summary to Firestore |
| 3 | `command_listener` | 10 seconds | Poll Firestore for remote commands |
| 4 | `message_poller` | 60 seconds | Poll Firestore inbox for messages |
| 5 | `watchdog_loop` | 60 seconds | Monitor services, disk, RAM; auto-restart |
| 6 | `lan_binlog_shipping_loop` | `interval_hours` (min 1h) | Ship binlog files to LAN |

All tasks skip the first tick (they don't run immediately on daemon start). Each task independently loads config and checks prerequisites on every cycle.

---

## 10. Error Handling

### 10.1 Backup Error States

| Error | Handling | Record State |
|-------|----------|-------------|
| License expired/missing | Return error before starting | No record created |
| Hospital code empty | Return error before starting | No record created |
| MySQL password empty | Return error before starting | No record created |
| MySQL connection failed | Mark record as Failed | `status: Failed` |
| Zero tables dumped | Mark record as Failed, clean up temp dir | `status: Failed` |
| ZIP compression failed | Mark record as Failed, clean up temp dir | `status: Failed` |
| GCS upload failed | **Non-fatal**: log warning, continue | `uploaded: false` |
| LAN copy failed | **Non-fatal**: log warning, continue | `lan_copied: false` |

### 10.2 Restore Error States

| Error | Handling |
|-------|----------|
| Backup ID not found in history | Return `NotFound` error |
| Backup not in Completed state | Return `InvalidConfig` error |
| ZIP not found (local, GCS, LAN) | Return `InvalidConfig` error |
| Individual SQL file restore fails | Log warning, skip file, continue with next |

### 10.3 Binlog Error States

| Error | Handling |
|-------|----------|
| License missing binlog feature | Return error |
| LAN not enabled/configured | Return `InvalidConfig` error |
| MySQL connection failed | Return error, state preserved |
| `mysqlbinlog` CLI failed for one file | Log warning, skip file, continue |
| Gzip compression failed | Log warning, skip file, continue |
| LAN write failed | Log warning, skip file, continue |
| LAN rename failed | Log warning, clean up `.tmp`, continue |

---

## 11. Atomicity and Safety

### 11.1 Atomic Writes

Both LAN backup copy and LAN binlog shipping use the **write-then-rename** pattern:

```rust
// LAN backup copy
std::fs::copy(zip_path, &tmp_file)?;    // write to .zip.tmp
std::fs::rename(&tmp_file, &dest_file)?; // atomic rename

// LAN binlog shipping
std::fs::write(&tmp_file, &compressed)?;  // write to .gz.tmp
std::fs::rename(&tmp_file, &dest_file)?;  // atomic rename
```

This prevents partial files on the network share if the copy is interrupted (network disconnect, power failure).

### 11.2 ZIP-Slip Protection

During restore, extracted paths are validated:

```rust
if !out_path.starts_with(out_dir) {
    continue; // Skip path traversal entries
}
```

### 11.3 State Isolation

LAN binlog state (`binlog-lan-state.json`) is completely independent from any future GCS binlog state. Each has its own tracking, allowing LAN and GCS to operate independently even if one fails.

---

## 12. Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `mysql_async` | 0.33 | Async MySQL driver for metadata queries |
| `zip` | 0.6 | ZIP compression/extraction |
| `flate2` | 1.x | Gzip compression for binlog files |
| `google-cloud-storage` | 0.15 | GCS upload/download |
| `google-cloud-auth` | 0.13 | GCS authentication |
| `chrono` | 0.4 | Timestamps and date formatting |
| `serde` / `serde_json` / `toml` | -- | Serialization for config, history, state |
| `tokio` | 1.x | Async runtime, process spawning, intervals |
| `tracing` | 0.1 | Structured logging |

External CLI tools required on PATH:
- `mysqldump` -- for table data export
- `mysqlbinlog` -- for binlog data reading
- `mysql` -- for restore operations
