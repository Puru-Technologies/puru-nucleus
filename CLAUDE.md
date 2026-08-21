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
releases/mod.rs             Version checking, JAR download to versioned filenames, GCS/JRE management
remote_shell/mod.rs         Audited command execution with allowlist
services/mod.rs             Service management (Docker + native dispatchers)
  services/native.rs        Native JAR process management (start/stop/update/rollback/GC)
  services/jar_manifest.rs  Per-service JAR manifest — fail-safe journal for native-mode updates
telemetry/mod.rs            System metrics (CPU, RAM, disk, services)
daemon/
  mod.rs                    Axum HTTP server setup with graceful shutdown
  auth.rs                   API key middleware
  routes.rs                 REST API route handlers
  scheduler.rs              Background tasks (backup, telemetry, commands, messages, watchdog, LAN binlog)
  commands.rs               Command queue processor
commands/mod.rs             ~50 Tauri IPC command handlers (includes get_jar_manifest,
                            list_locking_processes, force_free_and_restart,
                            rollback_native_service_to for the manifest-driven update flow)
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
  services/                 Service management (start/stop/restart + native JAR update/apply/rollback,
                            with manifest-driven pending/history hooks: loadManifest,
                            loadLockingProcesses, forceFreeAndRestart, rollbackToVersion)
  backups/                  Backup management + history
  alerts/                   Alert display + acknowledgment
  inbox/                    Hospital inbox (messages from admin)
  updates/                  Nucleus + service version updates
  performance/              JVM memory budget (per-service heap, GC, reserves)
  settings/                 Configuration (daemon, backup schedule, MySQL, GCS)
  setup/                    9-step installation wizard
  remote-shell/             Remote command execution with audit log
```

### Native-Mode JAR Management (fail-safe manifest)

Native mode (`deployment_mode = "native"`) runs Spring Boot services as `java -jar` child processes instead of Docker containers. JAR updates use a **versioned-filename + manifest** scheme so that a mid-update crash never leaves disk state broken — the next `start_service` reconciles and completes any in-flight promotion automatically. This solves the Windows "JAR is busy" class of failures (JVM keeps the JAR memory-mapped after `taskkill /F`, blocking rename).

**On-disk layout** (per service, under `jars_dir` — default `C:\PuruNucleus\jars`):
```
puru-auth_20260822-143022-abc1234.jar   # active (currently launched)
puru-auth_20260821-091510-def4567.jar   # previous (history[0], for rollback)
puru-auth_20260820-100000-ghi7890.jar   # older (pruned by jar_history_keep)
puru-auth_20260822-143022-abc1234.jar.meta.json  # GCS build metadata sidecar
puru-auth.manifest.json                  # source of truth — read on every start
```

**Manifest fields:**
- `active` — the JAR `start_service()` launches (immutable pointer between promotions).
- `pending` — a downloaded-but-not-yet-promoted JAR. Set at end of download BEFORE any process is stopped. Its presence is the durable intent that survives a crash.
- `history` — newest-first list of prior actives; feeds the rollback picker.

**Recovery table** (`JarManifest::recover_on_start`, called on every start):
| manifest.active | manifest.pending | on-disk pending | action |
|-----------------|------------------|-----------------|--------|
| set, file OK | none | — | launch active (normal) |
| set | set, file exists | present | promote → launch (crashed mid-apply) |
| set | set | missing | clear pending, launch active (crashed mid-download) |
| missing, history OK | — | — | rollback to history[0], launch |
| missing, no history | none | — | Err "no active JAR — re-pull" |

**Key files:**
- `src-tauri/src/services/jar_manifest.rs` — `JarManifest`, `JarEntry`, `make_versioned_filename`, `recover_on_start`, `promote_pending`, `rollback_one`/`rollback_to`, `gc`. 10 unit tests.
- `src-tauri/src/releases/mod.rs` — `pull_jar_progress` writes to fresh timestamped filename + sets `pending` in manifest. `stage_jar_progress` is an alias.
- `src-tauri/src/services/native.rs` — `start_service` runs `recover_on_start` before launching; `update_service_progress` does download → stop → best-effort `free_file` → start (start does the promotion); `rollback_service` and `rollback_service_to` are pure manifest pointer swaps (no file rename).
- `src-tauri/src/file_lock/mod.rs` — Windows Restart Manager wrapper (`processes_locking`, `free_file`); no-op on non-Windows.

**Config knobs:**
- `jar_history_keep` (default 3) — how many previous JAR versions to retain per service for rollback. Older files are GC'd after each successful update.

**Config gotcha:**
- `jars_dir` layout is service-namespaced by filename prefix (`{service}_...jar`). The prefix scan in `JarManifest::gc` uses `{service}_` — safe today because no two service names share a prefix ending at `_`.

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
| POST | `/api/updates/:service` | Update a service (native: manifest-driven, fail-safe) |
| POST | `/api/rollback/:service` | Rollback a service (native: pointer swap in manifest) |
| POST | `/api/jars/update/:service` | Native JAR update (versioned filename + manifest.pending) |
| POST | `/api/jars/rollback/:service` | Native JAR rollback |
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
puru info                   Show configuration (surfaces jar_history_keep in native mode)
puru version                Show version
puru daemon                 Run daemon mode

# Native-mode JAR management (available only when deployment_mode = "native")
puru pull-jars [all|<svc>]         Download JARs from GCS (writes to versioned filename + manifest.pending)
puru jar-updates                   Check for newer builds vs current manifest.active
puru update <service>              Download → stop → start (one-shot; fail-safe via manifest.pending)
puru rollback <service>            Roll back one step in manifest history (pointer swap, no file rename)
puru rollback <service> --to <file>  Roll back to a specific historical JAR (must appear in manifest.history)
puru jars info <service>           Show manifest (active + pending + history) for a service
puru jars gc <service>             Prune old JAR versions to jar_history_keep (never removes active/pending)
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
- `hospital/{code}` — heartbeat (nucleus field), services, backup_summary, alert_summary
- `hospital/{code}/alerts` — watchdog alerts (severity, category, title, message, acknowledged, resolved)

`acknowledged` and `resolved` are independent: `acknowledged` means a human saw
the alert, `resolved` means the watchdog observed the condition clear. On
recovery the watchdog patches the original alert to `resolved: true` and pushes
a separate `info` alert ("Resolved: …") so the fix is visible, not just the
absence of the problem. `alert_summary` on the hospital doc is the rollup for
the puru-oxygen dashboard — `{critical, warning, open, categories, updated_at}`,
written only when it changes.

Alert categories: `disk_space`, `memory` (RAM), `service_down`,
`service_restart`, `service_recovered`, `service_not_installed`,
`boot_task_missing`.
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

# Native-mode: how many previous JAR versions to retain per service for rollback.
# Older versioned JARs are deleted by the manifest GC after each successful update.
# `0` still keeps the currently-active JAR; only history is trimmed.
jar_history_keep = 3

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

## Windows Build & Deploy Notes

The Windows `.exe` (Tauri bundle + Wix MSI) is the primary shipping target. Build steps:

```powershell
# Prerequisites (one-time, on the Windows build box):
#   - Rust 1.70+ (rustup)
#   - Node.js 18+
#   - WebView2 runtime (comes with Windows 11; auto-installed by MSI on older)
#   - Wix Toolset v3 (Tauri MSI packager depends on it)

npm install
npm run tauri:build                      # produces src-tauri/target/release/bundle/msi/*.msi
                                         # + src-tauri/target/release/bundle/nsis/*.exe
```

**Signing** (release only): sign the MSI and the inner `.exe` with your Authenticode cert BEFORE distributing — unsigned builds trigger SmartScreen on end-user boxes. Configure via `tauri.conf.json > bundle.windows.certificateThumbprint` or sign post-build with `signtool`.

**Deploying a fresh install on a Windows target box:**
1. Run the MSI as Administrator (nucleus needs elevation to write to `C:\PuruNucleus\`, create Windows Services, and register Defender exclusions).
2. First launch runs the setup wizard — pick **native** deployment mode, walk through env-file generation and pull-JARs steps.
3. JARs land in `C:\PuruNucleus\jars\{service}_{ts}-{sha}.jar` alongside a per-service `{service}.manifest.json`. This is the manifest-driven layout — do NOT expect the old `{service}.jar` filename.

**Upgrading nucleus on a box that already has native JARs installed under the OLD (pre-manifest) scheme:**
Nucleus is not in production. The manifest scheme has **no back-compat migration**. On a fresh nucleus build against an old `jars/` dir:
- `start_service` will error with "No active JAR" because `{service}.jar` isn't in a manifest.
- **Fix**: wipe `C:\PuruNucleus\jars\` (or delete only the old `{service}.jar` / `.bak` / `.staged` files) and re-run the setup wizard's Pull JARs step. JARs will re-download under the versioned scheme and everything works.

**Windows-specific behaviours to remember:**
- Windows Restart Manager (`file_lock::processes_locking`) is the authoritative "what has this file open?" query — same mechanism Windows installers use. The manifest scheme sidesteps it for the JAR itself (new download = new filename = no lock conflict), but nucleus still uses `free_file` as a best-effort cleanup for orphan JVMs so GC can delete the old JAR.
- `taskkill /F /T /PID` is the only reliable stop path — headless `java.exe` (spawned with `CREATE_NO_WINDOW`) has no console window, so a graceful `taskkill` without `/F` dispatches `WM_CLOSE` to a nonexistent receiver.
- Port release is racy on Windows — `start_service` polls `wait_for_port_free` for up to 15s after a stop before spawning the new JVM.
- Manifest JSON is written via tmp+rename with retry against ACCESS_DENIED (5) and SHARING_VIOLATION (32) — the same lock window that would block a JAR swap also briefly hits the tiny JSON under AV scanners.

**Testing the fail-safe recovery on Windows:**
1. Trigger an update from the UI, then kill nucleus from Task Manager mid-apply.
2. Restart nucleus. On the next `start_service` for that service, `JarManifest::recover_on_start` should complete the promotion automatically and launch the new JAR. No manual intervention.
3. Verify with `puru jars info <service>` — the pending should have been consumed into active, and the old active pushed to history[0].

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
# Rust tests (~80 tests: config, detection, docker_update, licensing, messaging,
# performance, releases, remote_shell, and jar_manifest — the manifest module has
# 10 tests covering the full state table for recover_on_start + rollback + GC)
cargo test

# Angular tests
ng test
```
