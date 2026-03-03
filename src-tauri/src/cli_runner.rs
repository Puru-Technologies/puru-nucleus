//! CLI command execution — dispatches clap commands to service/backup/detection functions
//! and renders output with colored tables.

use crate::cli::{BackupArgs, BackupCommands, Commands, RestoreArgs, ServiceArgs, ServiceCommands};
use crate::config;
use crate::services;
use comfy_table::{presets, Cell, CellAlignment, Color, Table};
use colored::Colorize;

/// Run the given CLI command, printing results to stdout.
pub async fn run(command: Commands) {
    match command {
        Commands::Detect { json } => cmd_detect(json).await,
        Commands::Status => cmd_status().await,
        Commands::Start { service } => cmd_start(&service).await,
        Commands::Stop { service } => cmd_stop(&service).await,
        Commands::Restart { service } => cmd_restart(&service).await,
        Commands::Logs { service, lines } => cmd_logs(&service, lines).await,
        Commands::Health { service } => cmd_health(service.as_deref()).await,
        Commands::Backup(args) => cmd_backup(args).await,
        Commands::Restore(args) => cmd_restore(args).await,
        Commands::Service(args) => cmd_service(args).await,
        Commands::Info => cmd_info().await,
        Commands::Version => cmd_version(),
        Commands::Daemon => {
            // Handled in main.rs before we get here
            unreachable!("daemon mode is handled in main.rs");
        }
    }
}

// ── Detect ───────────────────────────────────────────────────────────────────

async fn cmd_detect(json: bool) {
    match services::detect_existing_setup().await {
        Ok(result) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&result).unwrap());
                return;
            }

            if !result.found {
                println!("{}", "No existing Puru deployment found.".yellow());
                return;
            }

            println!("{}", "EXISTING PURU DEPLOYMENT DETECTED".green().bold());
            println!();

            if let Some(ref path) = result.compose_path {
                println!("  Docker Compose: {}", path.cyan());
            }

            if !result.containers.is_empty() {
                println!();
                let mut table = Table::new();
                table.load_preset(presets::UTF8_FULL_CONDENSED);
                table.set_header(vec!["Container", "Image", "Status"]);

                for c in &result.containers {
                    let status_cell = match c.status.as_str() {
                        "running" => Cell::new(&c.status).fg(Color::Green),
                        _ => Cell::new(&c.status).fg(Color::Red),
                    };
                    table.add_row(vec![
                        Cell::new(&c.name),
                        Cell::new(&c.image),
                        status_cell,
                    ]);
                }
                println!("{table}");
            }

            if !result.databases.is_empty() {
                println!();
                println!("  {}", "Databases:".bold());
                let total_mb: u64 = result.databases.iter().map(|d| d.size_mb).sum();
                for db in &result.databases {
                    println!("    {} ({} MB)", db.name.cyan(), db.size_mb);
                }
                println!("    Total: {} MB", total_mb);
            }
        }
        Err(e) => {
            eprintln!("{} {}", "Error:".red().bold(), e);
            std::process::exit(1);
        }
    }
}

// ── Status ───────────────────────────────────────────────────────────────────

async fn cmd_status() {
    let cfg = config::load_config().unwrap_or_default();
    let hospital = if cfg.hospital_code.is_empty() {
        "Unconfigured".to_string()
    } else {
        cfg.hospital_code.clone()
    };

    println!();
    println!(
        "{} — {} Server",
        "PURU NUCLEUS".green().bold(),
        hospital.cyan().bold()
    );
    println!("{}", "=".repeat(60));

    match services::get_services().await {
        Ok(svcs) => {
            if svcs.is_empty() {
                println!();
                println!("{}", "  No Puru services found. Is Docker running?".yellow());
                println!();
                return;
            }

            let running = svcs.iter().filter(|s| s.status == services::ServiceStatus::Running).count();

            println!();
            println!(
                "  SERVICES  {}/{}",
                running.to_string().green().bold(),
                svcs.len()
            );
            println!();

            let mut table = Table::new();
            table.load_preset(presets::UTF8_FULL_CONDENSED);
            table.set_header(vec!["Service", "Status", "Port", "Health", "Uptime"]);

            for s in &svcs {
                let status_cell = match s.status {
                    services::ServiceStatus::Running => {
                        Cell::new("● Running").fg(Color::Green)
                    }
                    services::ServiceStatus::Stopped => {
                        Cell::new("○ Stopped").fg(Color::Red)
                    }
                    services::ServiceStatus::Starting => {
                        Cell::new("◐ Starting").fg(Color::Yellow)
                    }
                    services::ServiceStatus::Error => {
                        Cell::new("✗ Error").fg(Color::Red)
                    }
                };

                let health_str = match (&s.health, s.health_response_ms) {
                    (Some(services::HealthStatus::Healthy), Some(ms)) => {
                        format!("OK {}ms", ms)
                    }
                    (Some(services::HealthStatus::Healthy), None) => "OK".to_string(),
                    (Some(services::HealthStatus::Unhealthy), Some(ms)) => {
                        format!("FAIL {}ms", ms)
                    }
                    (Some(services::HealthStatus::Unhealthy), None) => "FAIL".to_string(),
                    (Some(services::HealthStatus::Starting), _) => "Starting".to_string(),
                    (None, _) => "—".to_string(),
                };
                let health_cell = match &s.health {
                    Some(services::HealthStatus::Healthy) => {
                        Cell::new(&health_str).fg(Color::Green)
                    }
                    Some(services::HealthStatus::Unhealthy) => {
                        Cell::new(&health_str).fg(Color::Red)
                    }
                    _ => Cell::new(&health_str),
                };

                let port_str = if s.ports.is_empty() {
                    "—".to_string()
                } else {
                    s.ports.join(", ")
                };

                let uptime_str = s.uptime.as_deref().unwrap_or("—");

                table.add_row(vec![
                    Cell::new(&s.name),
                    status_cell,
                    Cell::new(&port_str),
                    health_cell,
                    Cell::new(uptime_str),
                ]);
            }
            println!("{table}");

            // System info
            if let Ok(snap) = crate::telemetry::collect_snapshot().await {
                println!();
                println!(
                    "  SYSTEM  CPU: {:.0}% | RAM: {:.1} GB | Disk: {:.0}%",
                    snap.cpu_percent, snap.ram_gb, snap.disk_percent
                );
            }
            println!();
        }
        Err(e) => {
            eprintln!("{} {}", "Error:".red().bold(), e);
            std::process::exit(1);
        }
    }
}

// ── Start / Stop / Restart ──────────────────────────────────────────────────

async fn cmd_start(service: &str) {
    if service == "all" {
        match services::get_services().await {
            Ok(svcs) => {
                let stopped: Vec<_> = svcs
                    .iter()
                    .filter(|s| s.status != services::ServiceStatus::Running)
                    .collect();
                if stopped.is_empty() {
                    println!("{}", "All services are already running.".green());
                    return;
                }
                for s in &stopped {
                    print!("  Starting {}... ", s.container_name);
                    match services::start_service(&s.container_name).await {
                        Ok(()) => println!("{}", "OK".green()),
                        Err(e) => println!("{} {}", "FAILED".red(), e),
                    }
                }
                println!();
                println!(
                    "{}",
                    format!("Started {} services.", stopped.len()).green().bold()
                );
            }
            Err(e) => {
                eprintln!("{} {}", "Error:".red().bold(), e);
                std::process::exit(1);
            }
        }
    } else {
        print!("  Starting {}... ", service);
        match services::start_service(service).await {
            Ok(()) => println!("{}", "OK".green()),
            Err(e) => {
                println!("{}", "FAILED".red());
                eprintln!("  {}", e);
                std::process::exit(1);
            }
        }
    }
}

async fn cmd_stop(service: &str) {
    if service == "all" {
        match services::get_services().await {
            Ok(svcs) => {
                let running: Vec<_> = svcs
                    .iter()
                    .filter(|s| s.status == services::ServiceStatus::Running)
                    .collect();
                if running.is_empty() {
                    println!("{}", "No services are running.".yellow());
                    return;
                }
                for s in &running {
                    print!("  Stopping {}... ", s.container_name);
                    match services::stop_service(&s.container_name).await {
                        Ok(()) => println!("{}", "OK".green()),
                        Err(e) => println!("{} {}", "FAILED".red(), e),
                    }
                }
                println!();
                println!(
                    "{}",
                    format!("Stopped {} services.", running.len()).green().bold()
                );
            }
            Err(e) => {
                eprintln!("{} {}", "Error:".red().bold(), e);
                std::process::exit(1);
            }
        }
    } else {
        print!("  Stopping {}... ", service);
        match services::stop_service(service).await {
            Ok(()) => println!("{}", "OK".green()),
            Err(e) => {
                println!("{}", "FAILED".red());
                eprintln!("  {}", e);
                std::process::exit(1);
            }
        }
    }
}

async fn cmd_restart(service: &str) {
    if service == "all" {
        match services::get_services().await {
            Ok(svcs) => {
                for s in &svcs {
                    print!("  Restarting {}... ", s.container_name);
                    match services::restart_service(&s.container_name).await {
                        Ok(()) => println!("{}", "OK".green()),
                        Err(e) => println!("{} {}", "FAILED".red(), e),
                    }
                }
                println!();
                println!(
                    "{}",
                    format!("Restarted {} services.", svcs.len()).green().bold()
                );
            }
            Err(e) => {
                eprintln!("{} {}", "Error:".red().bold(), e);
                std::process::exit(1);
            }
        }
    } else {
        print!("  Restarting {}... ", service);
        match services::restart_service(service).await {
            Ok(()) => println!("{}", "OK".green()),
            Err(e) => {
                println!("{}", "FAILED".red());
                eprintln!("  {}", e);
                std::process::exit(1);
            }
        }
    }
}

// ── Logs ─────────────────────────────────────────────────────────────────────

async fn cmd_logs(service: &str, lines: u64) {
    match services::get_container_logs(service, lines).await {
        Ok(logs) => {
            if logs.is_empty() {
                println!("{}", "No logs available.".yellow());
            } else {
                print!("{}", logs);
            }
        }
        Err(e) => {
            eprintln!("{} {}", "Error:".red().bold(), e);
            std::process::exit(1);
        }
    }
}

// ── Health ───────────────────────────────────────────────────────────────────

async fn cmd_health(service: Option<&str>) {
    match services::get_services().await {
        Ok(svcs) => {
            let targets: Vec<_> = if let Some(name) = service {
                svcs.into_iter()
                    .filter(|s| {
                        s.container_name == name
                            || s.name.eq_ignore_ascii_case(name)
                    })
                    .collect()
            } else {
                svcs
            };

            if targets.is_empty() {
                if let Some(name) = service {
                    eprintln!("{} Service '{}' not found.", "Error:".red().bold(), name);
                } else {
                    println!("{}", "No Puru services found.".yellow());
                }
                std::process::exit(1);
            }

            println!();
            println!("{}", "HEALTH CHECK".green().bold());
            println!();

            let mut table = Table::new();
            table.load_preset(presets::UTF8_FULL_CONDENSED);
            table.set_header(vec!["Service", "Status", "Health", "Latency"]);

            let mut all_healthy = true;

            for s in &targets {
                let status_str = match s.status {
                    services::ServiceStatus::Running => "Running",
                    services::ServiceStatus::Stopped => "Stopped",
                    services::ServiceStatus::Starting => "Starting",
                    services::ServiceStatus::Error => "Error",
                };

                let (health_str, health_color) = match &s.health {
                    Some(services::HealthStatus::Healthy) => ("OK", Color::Green),
                    Some(services::HealthStatus::Unhealthy) => {
                        all_healthy = false;
                        ("FAIL", Color::Red)
                    }
                    Some(services::HealthStatus::Starting) => ("Starting", Color::Yellow),
                    None => {
                        if s.status != services::ServiceStatus::Running {
                            all_healthy = false;
                        }
                        ("—", Color::White)
                    }
                };

                let latency = match s.health_response_ms {
                    Some(ms) => format!("{}ms", ms),
                    None => "—".to_string(),
                };

                table.add_row(vec![
                    Cell::new(&s.name),
                    Cell::new(status_str),
                    Cell::new(health_str).fg(health_color),
                    Cell::new(&latency).set_alignment(CellAlignment::Right),
                ]);
            }

            println!("{table}");
            println!();

            if all_healthy {
                println!("  {}", "All services healthy.".green().bold());
            } else {
                println!("  {}", "Some services are unhealthy or stopped.".red().bold());
            }
            println!();
        }
        Err(e) => {
            eprintln!("{} {}", "Error:".red().bold(), e);
            std::process::exit(1);
        }
    }
}

// ── Backup ───────────────────────────────────────────────────────────────────

async fn cmd_backup(args: BackupArgs) {
    match args.command {
        Some(BackupCommands::List { remote: _ }) => {
            match crate::backup::get_backup_history().await {
                Ok(records) => {
                    if records.is_empty() {
                        println!("{}", "No backup history found.".yellow());
                        return;
                    }

                    println!();
                    println!("{}", "BACKUP HISTORY".green().bold());
                    println!();

                    let mut table = Table::new();
                    table.load_preset(presets::UTF8_FULL_CONDENSED);
                    table.set_header(vec!["ID", "Type", "Status", "Size", "Date", "Uploaded"]);

                    for r in &records {
                        let status_cell = match r.status {
                            crate::backup::BackupStatus::Completed => {
                                Cell::new("Completed").fg(Color::Green)
                            }
                            crate::backup::BackupStatus::Failed => {
                                Cell::new("Failed").fg(Color::Red)
                            }
                            crate::backup::BackupStatus::InProgress => {
                                Cell::new("In Progress").fg(Color::Yellow)
                            }
                        };

                        let type_str = match r.backup_type {
                            crate::backup::BackupType::Full => "Full",
                            crate::backup::BackupType::Partial => "Partial",
                        };

                        let uploaded_str = if r.uploaded { "Yes" } else { "No" };

                        table.add_row(vec![
                            Cell::new(&r.id),
                            Cell::new(type_str),
                            status_cell,
                            Cell::new(format!("{} MB", r.size_mb)),
                            Cell::new(r.created_at.format("%Y-%m-%d %H:%M").to_string()),
                            Cell::new(uploaded_str),
                        ]);
                    }
                    println!("{table}");
                    println!();
                }
                Err(e) => {
                    eprintln!("{} {}", "Error:".red().bold(), e);
                    std::process::exit(1);
                }
            }
        }
        Some(BackupCommands::Full) | Some(BackupCommands::Partial) | None => {
            let backup_type = match args.command {
                Some(BackupCommands::Partial) => crate::backup::BackupType::Partial,
                _ => crate::backup::BackupType::Full,
            };

            let type_label = match backup_type {
                crate::backup::BackupType::Full => "full",
                crate::backup::BackupType::Partial => "partial",
            };

            println!("  Starting {} backup...", type_label);

            let license = match crate::licensing::load_license() {
                Ok(Some(lic)) => lic,
                Ok(None) => {
                    eprintln!(
                        "{} No license activated. Run the GUI to activate a license first.",
                        "Error:".red().bold()
                    );
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("{} Failed to load license: {}", "Error:".red().bold(), e);
                    std::process::exit(1);
                }
            };

            match crate::backup::start_backup(backup_type, &license).await {
                Ok(result) => {
                    if result.success {
                        println!();
                        println!("  {} Backup completed!", "✓".green().bold());
                        println!("  ID:       {}", result.backup_id);
                        println!("  Size:     {} MB", result.size_mb);
                        println!("  Duration: {}s", result.duration_seconds);
                        println!();
                    } else {
                        eprintln!("  {} Backup failed.", "✗".red().bold());
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("{} {}", "Error:".red().bold(), e);
                    std::process::exit(1);
                }
            }
        }
    }
}

// ── Restore ──────────────────────────────────────────────────────────────────

async fn cmd_restore(args: RestoreArgs) {
    let backup_id = if args.latest {
        // Find the latest completed backup
        match crate::backup::get_backup_history().await {
            Ok(records) => {
                match records
                    .iter()
                    .find(|r| r.status == crate::backup::BackupStatus::Completed)
                {
                    Some(r) => r.id.clone(),
                    None => {
                        eprintln!("{} No completed backups found.", "Error:".red().bold());
                        std::process::exit(1);
                    }
                }
            }
            Err(e) => {
                eprintln!("{} {}", "Error:".red().bold(), e);
                std::process::exit(1);
            }
        }
    } else if let Some(id) = args.backup_id {
        id
    } else {
        eprintln!(
            "{} Specify a backup ID or use --latest",
            "Error:".red().bold()
        );
        std::process::exit(1);
    };

    println!("  Restoring backup {}...", backup_id.cyan());

    match crate::backup::restore_backup(&backup_id).await {
        Ok(()) => {
            println!();
            println!("  {} Restore completed!", "✓".green().bold());
            println!();
        }
        Err(e) => {
            eprintln!("{} {}", "Error:".red().bold(), e);
            std::process::exit(1);
        }
    }
}

// ── Service ─────────────────────────────────────────────────────────────────

async fn cmd_service(args: ServiceArgs) {
    use crate::platform;

    match args.command {
        ServiceCommands::Install => {
            println!(
                "  Installing puru-nucleus as a system service ({})...",
                platform::platform_name()
            );
            match platform::install_service().await {
                Ok(result) => {
                    println!("  {} {}", "✓".green().bold(), result.message);
                }
                Err(e) => {
                    eprintln!("{} {}", "Error:".red().bold(), e);
                    std::process::exit(1);
                }
            }
        }
        ServiceCommands::Uninstall => {
            println!("  Uninstalling puru-nucleus system service...");
            match platform::uninstall_service().await {
                Ok(result) => {
                    println!("  {} {}", "✓".green().bold(), result.message);
                }
                Err(e) => {
                    eprintln!("{} {}", "Error:".red().bold(), e);
                    std::process::exit(1);
                }
            }
        }
        ServiceCommands::Start => {
            print!("  Starting puru-nucleus service... ");
            match platform::start_service().await {
                Ok(_) => println!("{}", "OK".green()),
                Err(e) => {
                    println!("{}", "FAILED".red());
                    eprintln!("  {}", e);
                    std::process::exit(1);
                }
            }
        }
        ServiceCommands::Stop => {
            print!("  Stopping puru-nucleus service... ");
            match platform::stop_service().await {
                Ok(_) => println!("{}", "OK".green()),
                Err(e) => {
                    println!("{}", "FAILED".red());
                    eprintln!("  {}", e);
                    std::process::exit(1);
                }
            }
        }
        ServiceCommands::Status => {
            match platform::service_status().await {
                Ok(status) => {
                    println!();
                    println!("{}", "PURU NUCLEUS SERVICE STATUS".green().bold());
                    println!("{}", "=".repeat(50));
                    println!();

                    let mut table = Table::new();
                    table.load_preset(presets::UTF8_FULL_CONDENSED);
                    table.set_header(vec!["Property", "Value"]);

                    table.add_row(vec![
                        Cell::new("Platform"),
                        Cell::new(platform::platform_name()),
                    ]);
                    table.add_row(vec![
                        Cell::new("Installed"),
                        if status.installed {
                            Cell::new("Yes").fg(Color::Green)
                        } else {
                            Cell::new("No").fg(Color::Red)
                        },
                    ]);
                    table.add_row(vec![
                        Cell::new("Running"),
                        if status.running {
                            Cell::new("Yes").fg(Color::Green)
                        } else {
                            Cell::new("No").fg(Color::Red)
                        },
                    ]);
                    table.add_row(vec![
                        Cell::new("Enabled"),
                        if status.enabled {
                            Cell::new("Yes").fg(Color::Green)
                        } else {
                            Cell::new("No").fg(Color::Yellow)
                        },
                    ]);

                    if let Some(pid) = status.pid {
                        table.add_row(vec![
                            Cell::new("PID"),
                            Cell::new(pid.to_string()),
                        ]);
                    }

                    table.add_row(vec![
                        Cell::new("Detail"),
                        Cell::new(&status.detail),
                    ]);

                    println!("{table}");
                    println!();
                }
                Err(e) => {
                    eprintln!("{} {}", "Error:".red().bold(), e);
                    std::process::exit(1);
                }
            }
        }
    }
}

// ── Info ─────────────────────────────────────────────────────────────────────

async fn cmd_info() {
    let cfg = config::load_config().unwrap_or_default();

    println!();
    println!("{}", "PURU NUCLEUS — Configuration".green().bold());
    println!("{}", "=".repeat(50));
    println!();

    let mut table = Table::new();
    table.load_preset(presets::UTF8_FULL_CONDENSED);
    table.set_header(vec!["Setting", "Value"]);

    table.add_row(vec!["Hospital Code", &cfg.hospital_code]);
    table.add_row(vec!["Server IP", &cfg.server_ip]);
    table.add_row(vec!["Docker Compose", &cfg.docker_compose_path]);
    table.add_row(vec![
        "MySQL",
        &format!(
            "{}@{}:{}",
            cfg.mysql_user, cfg.mysql_host, cfg.mysql_port
        ),
    ]);
    table.add_row(vec![
        "Backup Enabled",
        if cfg.backup_enabled { "Yes" } else { "No" },
    ]);
    table.add_row(vec![
        "Telemetry",
        if cfg.telemetry_enabled {
            "Yes"
        } else {
            "No"
        },
    ]);
    table.add_row(vec![
        "Auto Update",
        if cfg.auto_update_enabled {
            "Yes"
        } else {
            "No"
        },
    ]);
    table.add_row(vec!["Release Channel", &cfg.release_channel]);
    table.add_row(vec![
        "Config Dir",
        &config::config_dir().display().to_string(),
    ]);

    if let Some(ref d) = cfg.daemon {
        table.add_row(vec!["Daemon Port", &d.port.to_string()]);
        table.add_row(vec![
            "Backup Schedule",
            &format!(
                "{} (every {}h)",
                if d.backup_schedule.enabled {
                    "Enabled"
                } else {
                    "Disabled"
                },
                d.backup_schedule.interval_hours
            ),
        ]);
    }

    println!("{table}");
    println!();

    if let Some(ref creds) = cfg.gcs_credentials_path {
        println!("  GCS Credentials: {}", creds);
    } else {
        println!(
            "  GCS Credentials: {}",
            "Not configured".yellow()
        );
    }
    println!();
}

// ── Version ──────────────────────────────────────────────────────────────────

fn cmd_version() {
    println!(
        "puru-nucleus {}",
        env!("CARGO_PKG_VERSION")
    );
}
