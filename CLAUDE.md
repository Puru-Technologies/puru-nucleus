# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build and Development Commands

```bash
# ── Rust Backend (src-tauri/) ──────────────────────────────────────

# Build and check
cargo check                           # Type-check without building
cargo build                           # Debug build
cargo build --release                 # Production build (optimized, stripped)
cargo test                            # Run all tests (33 tests)
cargo test config::tests              # Run specific test module
cargo test test_default_daemon_config # Run single test

# Run in different modes
cargo run                             # GUI mode (Tauri window)
cargo run -- status                   # CLI mode
cargo run -- daemon                   # Daemon mode (headless HTTP server)
cargo run -- service status           # Service management

# ── Angular Frontend (root) ────────────────────────────────────────

npm install                           # Install dependencies
ng serve                              # Dev server (localhost:4201)
ng build                              # Production build

# ── Tauri Combined ─────────────────────────────────────────────────

npm run tauri:dev                     # Dev mode (Angular + Tauri)
npm run tauri:build                   # Production bundle (all platforms)
```

## Project Overview

Puru Nucleus is the **control center for hospital deployments** — a Tauri 2.0 desktop application with a Rust backend and Angular 19 frontend. It manages Docker-based hospital software installations, backups, telemetry, and remote administration.

**Three Operating Modes:**
- **GUI** (default): Desktop window with Angular UI — `puru` or `puru-nucleus`
- **CLI**: Terminal commands — `puru status`, `puru backup full`, `puru service status`
- **Daemon**: Headless background service with REST API — `puru daemon`

**Firebase Project:** puru-255206
**Default Daemon Port:** 9090

## Architecture

### Rust Backend (src-tauri/src/)

```
main.rs                     Entry point — routes to GUI/CLI/Daemon mode
cli.rs                      Clap argument parser (Commands enum)
cli_runner.rs               CLI command dispatcher with colored tables
error.rs                    NucleusError enum with severity levels

backup/mod.rs               MySQL dump → ZIP → GCS upload → LAN copy
  backup/binlog.rs            Binlog shipping to LAN (MySQL binary log replication)
config/mod.rs               NucleusConfig (TOML), DaemonConfig, BackupSchedule
detection/mod.rs            Detect existing Puru Docker deployments
docker_update/mod.rs        Docker image update + rollback
firestore/                  Firestore REST API client
  mod.rs                    FirestoreClient (heartbeat, alerts, commands)
  auth.rs                   GCP service account token exchange
  queries.rs                CRUD operations (get/update/create document)
  convert.rs                Firestore value type helpers
  types.rs                  FirestoreDocument, FirestoreValue types
licensing/mod.rs            License validation (expiry, features, limits)
messaging/                  Hospital inbox messaging
  mod.rs, service.rs        Fetch messages from Firestore subcollection
  types.rs                  InboxMessage, MessageAttachment types
  files.rs                  GCS file download (parse gs:// URLs)
  file_actions.rs           Apply config files, install certificates
performance/mod.rs          JVM memory planning for native services (budget, tiers, flags)
platform/                   System service management
  mod.rs                    Platform-agnostic facade (install/uninstall/start/stop/status)
  linux.rs                  systemd unit file management
  macos.rs                  launchd plist management
  windows.rs                Windows Service via sc.exe
releases/mod.rs             Version checking and update downloads
remote_shell/mod.rs         Audited command execution with allowlist
services/mod.rs             Docker service management via bollard
telemetry/mod.rs            System metrics (CPU, RAM, disk, services)
daemon/
  mod.rs                    Axum HTTP server setup with graceful shutdown
  auth.rs                   API key middleware
  routes.rs                 REST API route handlers
  scheduler.rs              Background tasks (backup, telemetry, commands, messages, watchdog, LAN binlog)
  commands.rs               Command queue processor
commands/mod.rs             ~45 Tauri IPC command handlers
```

### Angular Frontend (src/app/)

```
app.component.ts            Root layout with Material sidenav + sidebar navigation
app.routes.ts               Lazy-loaded routes with initGuard

core/
  guards/init.guard.ts      Redirects to /activation if no license
  models/
    hospital.model.ts       Hospital, HospitalMessage, HospitalAlert, TelemetrySummary
    license.model.ts        License, LicenseFeatures, LicenseLimits, status helpers
    app-error.ts            AppError hierarchy (Validation, Network, Docker, etc.)
  services/
    tauri.service.ts         Tauri IPC bridge with error handling
    notification.service.ts  MatSnackBar notifications

features/
  activation/               License activation (Firestore email lookup)
  dashboard/                Hospital overview (license status, system stats)
  services/                 Docker service management (start/stop/restart)
  backups/                  Backup management + history
  alerts/                   Alert display + acknowledgment
  inbox/                    Hospital inbox (messages from admin)
  updates/                  Nucleus + service version updates
  performance/              JVM memory budget (per-service heap, GC, reserves)
  settings/                 Configuration (daemon, backup schedule, MySQL, GCS)
  setup/                    9-step installation wizard
  remote-shell/             Remote command execution with audit log
```

### Daemon REST API (port 9090)

| Method | Path | Handler |
|--------|------|---------|
| GET | `/api/health` | Health check (public) |
| GET | `/api/services` | List Docker services |
| POST | `/api/services/:name/start` | Start service |
| POST | `/api/services/:name/stop` | Stop service |
| POST | `/api/services/:name/restart` | Restart service |
| GET | `/api/services/:name/logs` | Container logs |
| GET | `/api/system` | System info |
| GET | `/api/config` | Current configuration |
| GET | `/api/performance` | JVM memory plan (with measured RSS) |
| PUT | `/api/performance` | Update JVM memory plan |
| POST | `/api/backup` | Trigger backup |
| GET | `/api/backup/list` | Backup history |
| GET | `/api/backup/schedule` | Get backup schedule |
| PUT | `/api/backup/schedule` | Update backup schedule |
| GET | `/api/updates/check` | Check for updates |
| POST | `/api/updates/:service` | Update a service |
| POST | `/api/rollback/:service` | Rollback a service |
| POST | `/api/exec` | Execute shell command |
| GET | `/api/exec/history` | Shell audit log |
| GET | `/api/exec/allowed` | Allowed shell commands |
| GET | `/api/license` | License info |
| GET | `/api/alerts` | List alerts |
| POST | `/api/alerts/:id/ack` | Acknowledge alert |
| POST | `/api/restore` | Trigger restore |
| POST | `/api/lan/binlog/ship` | Ship binlogs to LAN |

All endpoints except `/api/health` require `X-API-Key` header.

### CLI Commands

```
puru detect [--json]        Scan for existing deployment
puru status                 Show all services with health
puru start <service|all>    Start service(s)
puru stop <service|all>     Stop service(s)
puru restart <service|all>  Restart service(s)
puru logs <service> [-n N]  Show container logs
puru health [service]       Health check
puru backup [full|partial]  Run backup
puru backup list            Show backup history
puru restore <id|--latest>  Restore from backup
puru binlog lan-ship        Ship binlogs to LAN
puru binlog status          Binlog shipping status
puru service install        Install as system service
puru service uninstall      Remove system service
puru service start          Start system service
puru service stop           Stop system service
puru service status         Show system service status
puru info                   Show configuration
puru version                Show version
puru daemon                 Run daemon mode
```

### Daemon Background Tasks (scheduler.rs)

1. **Backup Scheduler** — Periodic backups per DaemonConfig schedule
2. **Status Reporter** — Push telemetry snapshot to Firestore every N minutes
3. **Command Listener** — Poll `hospital/{code}/commands` for pending commands
4. **Message Poller** — Poll `hospital/{code}/inbox` for new messages
5. **Watchdog** — 60s health check loop monitoring services, disk, RAM
6. **LAN Binlog Shipper** — Ship MySQL binlog files to LAN network share

### Firestore Collections

**Written by puru-nucleus:**
- `hospital/{code}` — heartbeat (nucleus field), services, backup_summary
- `hospital/{code}/alerts` — watchdog alerts (severity, category, title, message)
- `hospital/{code}/commands/{id}` — command results (status, result, error)
- `hospital/{code}/telemetry/{id}` — telemetry snapshots

**Read by puru-nucleus:**
- `hospital/{code}/commands` — pending commands from admin
- `hospital/{code}/inbox` — messages from admin

### Configuration (nucleus.toml)

Location: `/etc/puru-nucleus/nucleus.toml` (Linux), `/usr/local/etc/puru-nucleus/nucleus.toml` (macOS), `C:\PuruNucleus\nucleus.toml` (Windows)

```toml
hospital_code = "BTCT"
server_ip = "192.168.1.100"
docker_compose_path = "/home/puru/docker/docker-compose.yml"
gcs_credentials_path = "/etc/puru-nucleus/gcs-credentials.json"
backup_enabled = true
telemetry_enabled = true
mysql_host = "127.0.0.1"
mysql_port = 3306
mysql_user = "root"
mysql_password = ""
auto_update_enabled = true
release_channel = "stable"

[daemon]
port = 9090
api_key = "secret"
telemetry_interval_minutes = 15

[daemon.backup_schedule]
enabled = true
interval_hours = 24
backup_type = "full"

[lan]
enabled = false
path = ""
binlog_enabled = false

# Optional. Absent means auto-tune from the box's RAM; see src-tauri/src/performance.
[performance]
enabled = true
auto_tune = true
exit_on_oom = false
```

## Documentation

- `TUTORIALS.md` — User tutorials (setup, CLI, backup/restore, LAN, binlog, updates, etc.)
- `RELEASE.md` — Release pipeline guide (tagging, CI/CD, GCS publishing)
- `docs/backup-technical-document.md` — Deep technical reference for the backup system

## Key Dependencies

**Rust:** tauri 2.0, axum 0.7, bollard 0.15, mysql_async 0.33, reqwest 0.11, clap 4, serde, tokio, chrono, flate2, sysinfo 0.29
**Angular:** Angular 19, @angular/material, @tauri-apps/api 2.0, rxjs 7.8
**Runtime:** Rust 1.70+, Node 18+, Docker (on target machines)

## Testing

```bash
# Rust tests (69 tests covering config, detection, docker_update, licensing, messaging,
# performance, releases, remote_shell)
cargo test

# Angular tests
ng test
```
