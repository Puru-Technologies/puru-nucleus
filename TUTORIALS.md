# Puru Nucleus — User Tutorials

## Table of Contents

1. [First-Time Setup](#1-first-time-setup)
2. [CLI Quick Start](#2-cli-quick-start)
3. [Daemon Mode Setup](#3-daemon-mode-setup)
4. [Service Management](#4-service-management)
5. [Backup & Restore](#5-backup--restore)
   - [What Gets Backed Up](#51-what-gets-backed-up)
   - [Backup Types](#52-backup-types)
   - [Storage Tiers](#53-storage-tiers)
   - [Running a Backup](#54-running-a-backup)
   - [Backup History Table](#55-reading-the-backup-history-table-gui)
   - [Automatic Backups](#56-automatic-backups)
   - [LAN Backups](#57-setting-up-lan-backups)
   - [Restoring a Backup](#58-restoring-a-backup)
   - [Binlog Shipping](#59-binlog-shipping-point-in-time-recovery)
   - [Complete Setup Example](#510-complete-setup-example)
   - [Troubleshooting](#511-backup-troubleshooting)
6. [Remote Shell](#6-remote-shell)
7. [Monitoring & Alerts](#7-monitoring--alerts)
8. [Messaging (Inbox)](#8-messaging-inbox)
9. [Updates](#9-updates)

---

## 1. First-Time Setup

### Prerequisites

- Docker installed and running
- MySQL database accessible
- Internet connectivity for cloud features

### GUI Setup Wizard

1. Launch `puru-nucleus` (double-click or run without arguments)
2. The app redirects to the **Activation** page
3. Enter your hospital email (provided by Puru Technologies)
4. The system looks up your hospital and activates the license
5. After activation, the **Setup Wizard** runs 9 steps:

| Step | What it does |
|------|-------------|
| 1. Prerequisites | Verifies Docker, MySQL, network |
| 2. Create Databases | Sets up required MySQL databases |
| 3. Configure RabbitMQ | Configures message queues |
| 4. Generate Config | Creates nucleus.toml with your hospital settings |
| 5. Pull Images | Downloads Docker images from GCP registry |
| 6. Start Services | Launches all Puru Docker containers |
| 7. Health Check | Verifies all services are healthy |
| 8. Configure Backups | Sets up automated backup schedule |
| 9. Install Daemon | Registers puru-nucleus as a system service |

### CLI Setup (alternative)

```bash
# Check prerequisites
puru detect

# If everything looks good, use the GUI for the full wizard
# The CLI is best for day-to-day operations after initial setup
```

---

## 2. CLI Quick Start

After installation, the `puru` command is available in your terminal.

### Check Everything

```bash
# Quick status overview
puru status

# Output:
# PURU NUCLEUS — BTCT Server
# ==========================
# SERVICES  5/5
#
# ┌──────────┬──────────┬──────┬────────┬────────┐
# │ Service  │ Status   │ Port │ Health │ Uptime │
# ├──────────┼──────────┼──────┼────────┼────────┤
# │ puru-has │ ● Running│ 8081 │ OK 45ms│ 5d 3h  │
# │ puru-pacs│ ● Running│ 8082 │ OK 12ms│ 5d 3h  │
# ...
#
# SYSTEM  CPU: 15% | RAM: 6.2 GB | Disk: 43%
```

### Common Operations

```bash
# View configuration
puru info

# Check service health
puru health

# View container logs
puru logs puru-xenon -n 200

# Scan for existing deployment
puru detect --json
```

---

## 3. Daemon Mode Setup

The daemon provides 24/7 monitoring, automated backups, and remote management.

### Install as System Service

```bash
# Install (requires root/admin)
sudo puru service install

# Check status
puru service status

# Start the service
sudo puru service start

# Verify it's running
puru service status
# Output:
# ┌───────────┬──────────────────┐
# │ Property  │ Value            │
# ├───────────┼──────────────────┤
# │ Platform  │ Linux (systemd)  │
# │ Installed │ Yes              │
# │ Running   │ Yes              │
# │ Enabled   │ Yes              │
# │ PID       │ 12345            │
# └───────────┴──────────────────┘
```

### Configure Daemon

Edit `nucleus.toml` to configure:

```toml
[daemon]
port = 9090                      # REST API port
api_key = "your-secret-key"      # Required for API access
telemetry_interval_minutes = 15  # How often to push status to cloud

[daemon.backup_schedule]
enabled = true
interval_hours = 24              # Backup every 24 hours
backup_type = "full"             # "full" or "partial"
```

### Daemon Background Tasks

When running, the daemon automatically:
- **Scheduled backups** — Runs at configured interval
- **Status reporting** — Pushes heartbeat + telemetry to Firestore
- **Command listener** — Polls for remote commands from admin
- **Message poller** — Checks for new inbox messages
- **Watchdog** — Monitors services, disk, RAM every 60 seconds

### Managing the Service

```bash
sudo puru service stop       # Stop daemon
sudo puru service start      # Start daemon
sudo puru service status     # Check status
sudo puru service uninstall  # Remove completely
```

---

## 4. Service Management

### List Services

```bash
puru status                  # Table view with health and uptime
```

### Control Services

```bash
# Individual service
puru start puru-xenon
puru stop puru-xenon
puru restart puru-xenon

# All services at once
puru start all
puru stop all
puru restart all
```

### View Logs

```bash
puru logs puru-xenon          # Last 100 lines (default)
puru logs puru-xenon -n 500   # Last 500 lines
puru logs puru-has -n 50      # Last 50 lines
```

### Health Checks

```bash
puru health                   # All services
puru health puru-xenon        # Single service
```

### From the GUI

1. Open Puru Nucleus → **Services** page.
2. Each service shows a status pill: **Running**, **Stopped**, **Starting**, or **Error**.
3. Use the **Start / Stop / Restart** buttons per service.
4. Click a service row to view container logs.

---

## 5. Backup & Restore

### 5.1 What Gets Backed Up

Puru Nucleus backs up **all hospital MySQL databases** — patient records, appointments, pharmacy, pathology, authentication, and every other database running in your deployment.

A backup captures:

| What | Description |
|------|-------------|
| **Tables** | All data rows + table structure |
| **Views** | Saved queries that present data from tables |
| **Triggers** | Automatic actions that run on INSERT/UPDATE/DELETE |
| **Stored Procedures & Functions** | Custom database logic |
| **Events** | Scheduled database tasks |

Each backup produces a single `.zip` file named with your hospital code and timestamp:
```
BTCT-PURU-02-03-2026-02-15-AM.zip
```

### 5.2 Backup Types

| Type | What It Contains | When to Use |
|------|-----------------|-------------|
| **Full Backup** | All data + all schema | Daily backups, before major changes |
| **Partial Backup** | Schema only (no patient data) | Quick structure snapshots, testing |

### 5.3 Storage Tiers

Backups can be saved to up to three locations:

| Location | Requires Internet? | Description |
|----------|-------------------|-------------|
| **Local** | No | On the hospital server itself (always saved) |
| **Cloud (GCS)** | Yes | Google Cloud Storage for off-site safety |
| **LAN** | No | A shared network drive (NFS/SMB) on your local network |

If cloud upload or LAN copy fails, the backup still succeeds — the local copy is always preserved.

### 5.4 Running a Backup

#### GUI

1. Open Puru Nucleus → **Backups** page.
2. Click **Full Backup** (or **Partial Backup** for schema only).
3. A progress bar appears — wait for it to complete.
4. A green notification shows the result:
   ```
   Backup completed: 48 MB in 28s
   ```
5. The backup appears in the **Backup History** table.

#### CLI

```bash
puru backup                    # Full backup (default)
puru backup full               # Same as above
puru backup partial            # Schema-only backup
puru backup list               # Show all backups
```

#### REST API

```bash
# Trigger a full backup
curl -X POST http://localhost:9090/api/backup \
  -H "X-API-Key: your-api-key" \
  -H "Content-Type: application/json" \
  -d '{"type": "full"}'

# List backup history
curl http://localhost:9090/api/backup/list \
  -H "X-API-Key: your-api-key"
```

### 5.5 Reading the Backup History Table (GUI)

| Column | Meaning |
|--------|---------|
| **Type** | `full` or `partial` |
| **Status** | `completed`, `failed`, or `in_progress` |
| **Size** | Size of the ZIP file in MB |
| **Created** | Date and time the backup was taken |
| **Cloud** | Green cloud icon = uploaded to GCS; grey = not uploaded |
| **LAN** | Green icon = copied to network share; grey = not copied |
| **Actions** | Restore button (only available for completed backups) |

### 5.6 Automatic Backups

Automatic backups run when the daemon is active.

1. Go to **Settings** → **Daemon** card.
2. Enable **Backup Schedule** and set:
   - **Interval (hours)** — default 24
   - **Backup Type** — `full` or `partial`
3. Click **Save Configuration**.

Or edit `nucleus.toml` directly:

```toml
[daemon.backup_schedule]
enabled = true
interval_hours = 24
backup_type = "full"
```

The daemon must be running (`puru service status`) for scheduled backups to execute.

### 5.7 Setting Up LAN Backups

LAN backup copies your backup ZIP files to a network drive (NAS, file server, or another computer's shared folder). This works without internet.

#### Mount the Network Share

**Linux (NFS):**
```bash
sudo mount -t nfs 192.168.1.50:/backups /mnt/nas/backups

# Auto-mount on boot — add to /etc/fstab:
echo "192.168.1.50:/backups /mnt/nas/backups nfs defaults 0 0" | sudo tee -a /etc/fstab
```

**Linux (SMB/CIFS):**
```bash
sudo mount -t cifs //192.168.1.50/backups /mnt/nas/backups -o username=user,password=pass

# Auto-mount — add to /etc/fstab:
echo "//192.168.1.50/backups /mnt/nas/backups cifs credentials=/etc/samba/creds,_netdev 0 0" | sudo tee -a /etc/fstab
```

**Windows:**
```powershell
net use Z: \\192.168.1.50\backups /persistent:yes
```

#### Enable LAN Backup in Nucleus

1. Go to **Settings** → **LAN Backup** card.
2. Toggle **Enable LAN Backup** on.
3. Enter the **LAN Path**:
   - Linux: `/mnt/nas/backups`
   - Windows: `Z:\backups`
4. Click **Validate** — confirm "Path is writable" appears.
5. Optionally toggle **Enable LAN Binlog Shipping** (see Section 5.9).
6. Click **Save Configuration**.

#### LAN Storage Layout

```
/mnt/nas/backups/
  └── BTCT/                                  ← your hospital code
      ├── backups/                           ← full/partial backup ZIPs
      │   ├── BTCT-PURU-02-03-2026-02-15-AM.zip
      │   └── BTCT-PURU-01-03-2026-10-30-PM.zip
      └── binlogs/                           ← binary log files (if enabled)
          ├── mysql-bin.000001.gz
          └── mysql-bin.000002.gz
```

### 5.8 Restoring a Backup

> **Warning:** Restoring replaces all current data in the affected databases. Take a fresh backup first.

#### GUI

1. Go to **Backups** → find the backup in the history table.
2. Click the **Restore** button (circular arrow icon).
3. Confirm when prompted.
4. Wait for the notification confirming success.

#### CLI

```bash
puru restore BTCT-PURU-02-03-2026-02-15-AM   # Restore by ID
puru restore --latest                          # Restore most recent
```

#### REST API

```bash
curl -X POST http://localhost:9090/api/restore \
  -H "X-API-Key: your-api-key" \
  -H "Content-Type: application/json" \
  -d '{"backup_id": "BTCT-PURU-02-03-2026-02-15-AM"}'
```

#### Restore Fallback Chain

The restore searches for the ZIP file in this order:

1. **Local** — checks the server's backup directory
2. **Cloud (GCS)** — downloads from Google Cloud Storage
3. **LAN** — copies from the network share

You can restore even if the local file was deleted, as long as cloud or LAN has a copy.

### 5.9 Binlog Shipping (Point-in-Time Recovery)

#### What Are Binlogs?

MySQL binary logs record every write transaction between backups. They enable recovery to any point in time, not just the last backup snapshot.

**Example:** Full backup at 10:00 PM, server fails at 3:00 PM next day — without binlogs you lose 17 hours. With binlogs, you can recover up to the moment of failure.

#### Enable Binlog Shipping

Prerequisites:
- LAN Backup enabled (Section 5.7)
- MySQL binary logging enabled on the server
- License with binlog shipping feature

Steps:
1. **Settings** → **LAN Backup** card → toggle **Enable LAN Binlog Shipping**.
2. **Save Configuration**.

The daemon ships binlogs automatically at the same interval as your backup schedule.

#### Manual Binlog Shipping

**GUI:** On the Backups page → **LAN Binlog Shipping** card → **Ship to LAN Now**.

**CLI:**
```bash
puru binlog lan-ship           # Ship pending binlogs
puru binlog status             # Check shipping status
```

**REST API:**
```bash
curl -X POST http://localhost:9090/api/lan/binlog/ship \
  -H "X-API-Key: your-api-key"
```

#### Monitoring Binlog Status

The LAN Binlog Shipping card on the Backups page shows:

| Field | Meaning |
|-------|---------|
| **Last Shipped** | Name of the most recently shipped binlog file |
| **Total Shipped** | How many binlog files have been shipped |
| **Pending** | Files waiting to be shipped |
| **Last Error** | Error message if the last attempt failed |

### 5.10 Complete Setup Example

Setting up backups at hospital "CITY" from scratch:

```bash
# 1. Mount NAS
sudo mount -t nfs 192.168.1.50:/backups /mnt/nas/backups

# 2. Configure via GUI:
#    Settings → MySQL credentials → Save
#    Settings → LAN Backup → enable, path=/mnt/nas/backups, validate → Save
#    Settings → Daemon → backup schedule on, 24h, full → Save

# 3. Install daemon
puru service install
puru service start
puru service status

# 4. Run first backup from GUI or CLI
puru backup full

# 5. Verify
puru backup list
ls /mnt/nas/backups/CITY/backups/
```

Result: three layers of protection — local + cloud (if configured) + LAN.

### 5.11 Backup Troubleshooting

| Problem | Cause | Fix |
|---------|-------|-----|
| "License expired" | License past valid date | Contact Puru Technologies support |
| "MySQL password empty" | MySQL not configured | Settings → enter MySQL credentials → Save |
| LAN column grey | LAN not enabled or path not mounted | Settings → LAN Backup → enable + validate path |
| Cloud column grey | No internet or GCS not configured | Check internet; set GCS credentials in Settings |
| Binlog "Last Error" red | MySQL binlog off or LAN full | Enable MySQL binlog; free space on NAS |
| Backup takes too long | Large database (>10 GB) | Schedule backups during off-hours |
| Daemon not running | System service stopped | `puru service start` |

---

## 6. Remote Shell

The remote shell allows executing pre-approved commands on the hospital server.

### From the GUI

1. Open **Remote Shell** page.
2. Select a command from the allowed list or type a custom command.
3. Click **Execute**.
4. View stdout, stderr, exit code, and execution time.

### From the CLI

```bash
puru exec "docker ps"
```

### From the REST API

```bash
curl -X POST http://localhost:9090/api/exec \
  -H "X-API-Key: your-api-key" \
  -H "Content-Type: application/json" \
  -d '{"command": "docker ps"}'

# View execution history
curl http://localhost:9090/api/exec/history \
  -H "X-API-Key: your-api-key"

# See allowed commands
curl http://localhost:9090/api/exec/allowed \
  -H "X-API-Key: your-api-key"
```

### Security

- Only allowlisted commands can run (configured in the backend).
- Every execution is logged with timestamp, command, exit code, and duration.
- The audit log is viewable from the GUI and REST API.

---

## 7. Monitoring & Alerts

### How Monitoring Works

When the daemon is running, a **watchdog** task runs every 60 seconds:

1. Checks all Docker services — restarts any that are stopped unexpectedly.
2. Monitors disk usage — alerts when free space drops below threshold.
3. Monitors RAM usage — alerts on high memory pressure.
4. Pushes telemetry (CPU, RAM, disk, service status) to Firestore.

### Viewing Alerts

#### GUI

Open **Alerts** page to see all alerts with severity, category, and timestamp. Click **Acknowledge** to dismiss.

#### REST API

```bash
# List alerts
curl http://localhost:9090/api/alerts \
  -H "X-API-Key: your-api-key"

# Acknowledge an alert
curl -X POST http://localhost:9090/api/alerts/ALERT_ID/ack \
  -H "X-API-Key: your-api-key"
```

### Dashboard

The **Dashboard** page shows a summary:
- License status and expiry
- System stats (CPU, RAM, disk)
- Service count and health
- Recent alerts

---

## 8. Messaging (Inbox)

The hospital inbox receives messages from the Puru Technologies admin team via Firestore.

### Viewing Messages

1. Open **Inbox** page.
2. Messages are listed with timestamp, subject, and read status.
3. Click a message to view full content.

Messages can include attachments:
- **Config files** — applied automatically when accepted
- **Certificates** — installed to the appropriate location
- **General files** — downloaded to a specified path

### How It Works

The daemon polls `hospital/{code}/inbox` in Firestore every 60 seconds. New messages appear in the GUI automatically.

---

## 9. Updates

### Checking for Updates

#### GUI

Open **Updates** page to see:
- **Nucleus Update** — whether a new version of Puru Nucleus is available
- **Service Updates** — whether individual Docker services have newer images

#### CLI

```bash
puru version                   # Show current version
```

#### REST API

```bash
curl http://localhost:9090/api/updates/check \
  -H "X-API-Key: your-api-key"
```

### Updating a Service

#### GUI

1. Go to **Updates** page.
2. If an update is available, click **Update** next to the service.
3. The system pulls the new Docker image, stops the old container, starts the new one.
4. If the new version fails health checks, it **automatically rolls back**.

#### REST API

```bash
# Update a service
curl -X POST http://localhost:9090/api/updates/puru-xenon \
  -H "X-API-Key: your-api-key"

# Rollback a service
curl -X POST http://localhost:9090/api/rollback/puru-xenon \
  -H "X-API-Key: your-api-key"
```

### Update Safety

- Updates use a **pull → stop → start → health check → rollback** pattern.
- If the new version fails health checks, the previous image is restored automatically.
- Update history is logged with previous/new image, success status, and duration.

### Release Channels

Configure in Settings or `nucleus.toml`:

```toml
release_channel = "stable"     # or "beta"
auto_update_enabled = true
```

- **stable** — production releases only
- **beta** — includes pre-release versions