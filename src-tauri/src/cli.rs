//! CLI argument parsing using clap
//!
//! Provides the `puru` command-line interface for managing hospital deployments.
//! Usage: `puru status`, `puru backup`, `puru health`, etc.

use clap::{Parser, Subcommand, Args};

#[derive(Parser)]
#[command(
    name = "puru",
    version,
    about = "Puru Nucleus — Hospital Deployment Control Center",
    long_about = "Control center for managing Puru hospital Docker deployments.\n\
                  Run without arguments to launch the GUI, or use subcommands for CLI mode."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Scan for existing Puru Docker deployment
    Detect {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Show status of all Puru services
    Status,

    /// Start a Docker service (or "all")
    Start {
        /// Service/container name, or "all"
        service: String,
    },

    /// Stop a Docker service (or "all")
    Stop {
        /// Service/container name, or "all"
        service: String,
    },

    /// Restart a Docker service (or "all")
    Restart {
        /// Service/container name, or "all"
        service: String,
    },

    /// Show container logs
    Logs {
        /// Service/container name
        service: String,

        /// Number of lines to show
        #[arg(short = 'n', long, default_value = "100")]
        lines: u64,
    },

    /// Check health of services
    Health {
        /// Specific service name (omit for all)
        service: Option<String>,
    },

    /// Backup operations
    Backup(BackupArgs),

    /// Restore from a backup
    Restore(RestoreArgs),

    /// Show nucleus configuration info
    Info,

    /// Show version
    Version,

    /// Manage the puru-nucleus system service (install/uninstall/start/stop/status)
    Service(ServiceArgs),

    /// Run in daemon mode (background service)
    Daemon,
}

#[derive(Args)]
pub struct BackupArgs {
    #[command(subcommand)]
    pub command: Option<BackupCommands>,

    /// Backup only databases (no data dirs)
    #[arg(long)]
    pub db_only: bool,

    /// Upload to GCS after backup
    #[arg(long)]
    pub upload: bool,
}

#[derive(Subcommand)]
pub enum BackupCommands {
    /// Run a full backup
    Full,

    /// Run a partial backup (important tables only)
    Partial,

    /// List backup history
    List {
        /// Show remote GCS backups
        #[arg(long)]
        remote: bool,
    },
}

#[derive(Args)]
pub struct ServiceArgs {
    #[command(subcommand)]
    pub command: ServiceCommands,
}

#[derive(Subcommand)]
pub enum ServiceCommands {
    /// Install puru-nucleus as a system service
    Install,

    /// Uninstall the puru-nucleus system service
    Uninstall,

    /// Start the puru-nucleus system service
    Start,

    /// Stop the puru-nucleus system service
    Stop,

    /// Show system service status
    Status,
}

#[derive(Args)]
pub struct RestoreArgs {
    /// Backup ID to restore
    pub backup_id: Option<String>,

    /// Restore the latest backup
    #[arg(long)]
    pub latest: bool,

    /// Restore only databases
    #[arg(long)]
    pub db_only: bool,
}
