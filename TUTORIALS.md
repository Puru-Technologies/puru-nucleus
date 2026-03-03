# Puru Nucleus — User Tutorials

## Table of Contents

1. [First-Time Setup](#1-first-time-setup)
2. [CLI Quick Start](#2-cli-quick-start)
3. [Daemon Mode Setup](#3-daemon-mode-setup)
4. [Service Management](#4-service-management)
5. [Backup & Restore](#5-backup--restore)
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
puru health                   #