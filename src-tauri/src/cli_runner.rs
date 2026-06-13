//! CLI command execution — dispatches clap commands to service/backup/detection functions
//! and renders output with colored tables.

use crate::cli::{BackupArgs, BackupCommands, BinlogArgs, BinlogCommands, Commands, LogFileArgs, LogFileCommands, RestoreArgs, ServiceArgs, ServiceCommands};
use crate::releases;
use crate::network;
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
        Commands::Logs { service, lines, since, until } => cmd_logs(&service, lines, since, until).await,
        Commands::Health { service } => cmd_health(service.as_deref()).await,
        Commands::Backup(args) => cmd_backup(args).await,
        Commands::Binlog(args) => cmd_binlog(args).await,
        Commands::Restore(args) => cmd_restore(args).await,
        Commands::Service(args) => cmd_service(args).await,
        Commands::LogFile(args) => cmd_log_file(args),
        Commands::Info => cmd_info().await,
        Commands::Network { speed, json } => cmd_network(speed, json).await,
        Commands::Pull => cmd_pull().await,
        Commands::PullJars { services } => cmd_pull_jars(&services).await,
        Commands::JarUpdates => cmd_jar_updates().await,
        Commands::Update { service } => cmd_update(&service).await,
        Commands::Rollback { service } => cmd_rollback(&service).await,
        Commands::Seed { db, queues, templates } => cmd_seed(db, queues, templates).await,
        Commands::Version => cmd_version(),
        Commands::Daemon => {
            // Handled in main.rs before we get here
            unreachable!("daemon mode is handled in main.rs");
        }
    }
}

// ── Seed ─────────────────────────────────────────────────────────────────────

async fn cmd_seed(db: bool, queues: bool, templates: bool) {
    // No flags = seed everything
    let all = !db && !queues && !templates;
    let (do_db, do_queues, do_templates) = (db || all, queues || all, templates || all);

    println!();
    println!("  Seeding fresh-install data (existing values are never overwritten)...");
    println!();

    match crate::seed::run_seed(do_db, do_queues, do_templates).await {
        Ok(report) => {
            let mut table = Table::new();
            table.load_preset(presets::UTF8_FULL_CONDENSED);
            table.set_header(vec![
                Cell::new("Section").set_alignment(CellAlignment::Left),
                Cell::new("Created").set_alignment(CellAlignment::Right),
                Cell::new("Skipped").set_alignment(CellAlignment::Right),
                Cell::new("Errors").set_alignment(CellAlignment::Right),
            ]);

            let mut all_errors: Vec<(String, String)> = Vec::new();
            for s in &report.sections {
                let error_cell = if s.errors.is_empty() {
                    Cell::new("0").fg(Color::Green)
                } else {
                    Cell::new(s.errors.len().to_string()).fg(Color::Red)
                };
                table.add_row(vec![
                    Cell::new(&s.name),
                    Cell::new(s.created.to_string()),
                    Cell::new(s.skipped.to_string()),
                    error_cell,
                ]);
                for e in &s.errors {
                    all_errors.push((s.name.clone(), e.clone()));
                }
            }
            println!("{table}");

            if !all_errors.is_empty() {
                println!();
                for (section, error) in &all_errors {
                    println!("  {} [{}] {}", "!".red(), section, error);
                }
            }
            println!();
        }
        Err(e) => {
            eprintln!("  {} Seed failed: {}", "✗".red(), e.user_message());
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

async fn cmd_logs(service: &str, lines: u64, since: Option<String>, until: Option<String>) {
    let since_ts = since.map(|s| match parse_time_arg(&s) {
        Ok(ts) => ts,
        Err(e) => {
            eprintln!("{} Invalid --since value: {}", "Error:".red().bold(), e);
            std::process::exit(1);
        }
    });
    let until_ts = until.map(|s| match parse_time_arg(&s) {
        Ok(ts) => ts,
        Err(e) => {
            eprintln!("{} Invalid --until value: {}", "Error:".red().bold(), e);
            std::process::exit(1);
        }
    });

    match services::get_container_logs(service, lines, since_ts, until_ts).await {
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

/// Parse a time argument into a Unix timestamp.
///
/// Accepts:
/// - Bare number: treated as a Unix timestamp
/// - ISO 8601: `%Y-%m-%dT%H:%M:%S` (parsed as UTC)
/// - Relative duration: `2h`, `1d`, `3d12h`, `30m` (subtracted from now)
fn parse_time_arg(input: &str) -> Result<i64, String> {
    let input = input.trim();

    // Bare number → Unix timestamp
    if let Ok(ts) = input.parse::<i64>() {
        return Ok(ts);
    }

    // ISO 8601 → parse via chrono
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(input, "%Y-%m-%dT%H:%M:%S") {
        return Ok(dt.and_utc().timestamp());
    }

    // Relative duration: e.g. "2h", "1d", "3d12h", "30m"
    let mut remaining = input;
    let mut total_seconds: i64 = 0;
    let mut matched_any = false;

    while !remaining.is_empty() {
        // Find the next digit sequence
        let digit_end = remaining.find(|c: char| !c.is_ascii_digit()).unwrap_or(remaining.len());
        if digit_end == 0 {
            return Err(format!("unexpected character '{}' in duration", &remaining[..1]));
        }
        let value: i64 = remaining[..digit_end]
            .parse()
            .map_err(|_| format!("invalid number in duration: {}", &remaining[..digit_end]))?;
        remaining = &remaining[digit_end..];

        if remaining.is_empty() {
            return Err("duration value missing unit (use h, m, d, s)".into());
        }

        let unit = &remaining[..1];
        let multiplier = match unit {
            "s" => 1,
            "m" => 60,
            "h" => 3600,
            "d" => 86400,
            _ => return Err(format!("unknown duration unit '{}' (use h, m, d, s)", unit)),
        };

        total_seconds += value * multiplier;
        remaining = &remaining[1..];
        matched_any = true;
    }

    if !matched_any {
        return Err(format!("could not parse '{}' as timestamp, datetime, or duration", input));
    }

    let now = chrono::Utc::now().timestamp();
    Ok(now - total_seconds)
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

// ── Binlog ───────────────────────────────────────────────────────────────────

async fn cmd_binlog(args: BinlogArgs) {
    match args.command {
        BinlogCommands::LanShip => {
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

            println!("  Shipping binlogs to LAN...");

            match crate::backup::binlog::ship_binlogs_to_lan(&license).await {
                Ok(result) => {
                    println!();
                    println!("  {} Binlog LAN shipping completed!", "✓".green().bold());

                    let mut table = Table::new();
                    table.load_preset(presets::UTF8_FULL_CONDENSED);
                    table.set_header(vec!["Metric", "Value"]);
                    table.add_row(vec!["Files Shipped", &result.files_shipped.to_string()]);
                    table.add_row(vec![
                        "Bytes Shipped",
                        &format!("{} KB", result.bytes_shipped / 1024),
                    ]);
                    table.add_row(vec![
                        "Duration",
                        &format!("{}s", result.duration_seconds),
                    ]);
                    if !result.errors.is_empty() {
                        table.add_row(vec![
                            "Errors",
                            &result.errors.join("; "),
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
        BinlogCommands::Status => {
            match crate::backup::binlog::get_binlog_status().await {
                Ok(status) => {
                    println!();
                    println!("{}", "BINLOG STATUS".green().bold());
                    println!();

                    let mut table = Table::new();
                    table.load_preset(presets::UTF8_FULL_CONDENSED);
                    table.set_header(vec!["Property", "Value"]);

                    table.add_row(vec![
                        Cell::new("LAN Enabled"),
                        if status.lan_enabled {
                            Cell::new("Yes").fg(Color::Green)
                        } else {
                            Cell::new("No").fg(Color::Red)
                        },
                    ]);

                    table.add_row(vec![
                        "Current Master File",
                        status.current_master_file.as_deref().unwrap_or("—"),
                    ]);
                    table.add_row(vec![
                        "Current Master Position",
                        &status
                            .current_master_position
                            .map(|p| p.to_string())
                            .unwrap_or_else(|| "—".into()),
                    ]);
                    table.add_row(vec![
                        "Files Pending",
                        &status.files_pending.to_string(),
                    ]);
                    table.add_row(vec![
                        "LAN Last Shipped",
                        status.lan_last_shipped_file.as_deref().unwrap_or("—"),
                    ]);
                    table.add_row(vec![
                        "LAN Last Shipped At",
                        &status
                            .lan_last_shipped_at
                            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_else(|| "—".into()),
                    ]);
                    table.add_row(vec![
                        "LAN Total Shipped",
                        &status.lan_total_shipped.to_string(),
                    ]);
                    if let Some(ref err) = status.lan_last_error {
                        table.add_row(vec![
                            Cell::new("LAN Last Error"),
                            Cell::new(err).fg(Color::Red),
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
    }
}

// ── Restore ──────────────────────────────────────────────────────────────────

async fn cmd_restore(args: RestoreArgs) {
    // Point-in-time recovery mode
    if let Some(ref stop_datetime) = args.pitr {
        let backup_id = if args.latest {
            None
        } else {
            args.backup_id.as_deref().map(String::from)
        };

        println!(
            "  Point-in-time recovery to {} ...",
            stop_datetime.cyan()
        );
        if let Some(ref id) = backup_id {
            println!("  Base backup: {}", id.cyan());
        } else {
            println!("  Base backup: {}", "latest".cyan());
        }

        match crate::backup::binlog::restore_pitr(backup_id.as_deref(), stop_datetime).await {
            Ok(result) => {
                println!();
                println!("  {} Point-in-time recovery completed!", "✓".green().bold());
                println!("  Base backup restored: {}", result.backup_restored.cyan());
                println!("  Binlogs applied:      {}", result.binlogs_applied.to_string().cyan());
                println!("  Target time:          {}", result.stop_datetime.cyan());
                println!("  Duration:             {}s", result.duration_seconds);
                println!();
            }
            Err(e) => {
                eprintln!("{} {}", "Error:".red().bold(), e);
                std::process::exit(1);
            }
        }
        return;
    }

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

// ── Log File ─────────────────────────────────────────────────────────────────

fn cmd_log_file(args: LogFileArgs) {
    match args.command {
        LogFileCommands::Sources => {
            let sources = crate::logs::get_known_log_paths();

            println!();
            println!("{}", "LOG SOURCES".green().bold());
            println!();

            let mut table = Table::new();
            table.load_preset(presets::UTF8_FULL_CONDENSED);
            table.set_header(vec!["Name", "Path", "Type"]);

            for s in &sources {
                table.add_row(vec![
                    Cell::new(&s.name),
                    Cell::new(&s.path),
                    Cell::new(&s.source_type),
                ]);
            }
            println!("{table}");
            println!();
        }
        LogFileCommands::List { path } => {
            let files = if let Some(ref p) = path {
                match crate::logs::list_log_files(p) {
                    Ok(f) => f,
                    Err(e) => {
                        eprintln!("{} {}", "Error:".red().bold(), e);
                        std::process::exit(1);
                    }
                }
            } else {
                // Aggregate all sources
                let sources = crate::logs::get_known_log_paths();
                let mut all = Vec::new();
                for source in &sources {
                    if let Ok(files) = crate::logs::list_log_files(&source.path) {
                        all.extend(files);
                    }
                }
                all.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));
                all
            };

            if files.is_empty() {
                println!("{}", "No log files found.".yellow());
                return;
            }

            println!();
            println!("{}", "LOG FILES".green().bold());
            println!();

            let mut table = Table::new();
            table.load_preset(presets::UTF8_FULL_CONDENSED);
            table.set_header(vec!["Name", "Size", "Modified", "Path"]);

            for f in &files {
                let size_str = if f.size_bytes >= 1024 * 1024 {
                    format!("{:.1} MB", f.size_bytes as f64 / (1024.0 * 1024.0))
                } else if f.size_bytes >= 1024 {
                    format!("{:.1} KB", f.size_bytes as f64 / 1024.0)
                } else {
                    format!("{} B", f.size_bytes)
                };

                table.add_row(vec![
                    Cell::new(&f.name),
                    Cell::new(&size_str).set_alignment(CellAlignment::Right),
                    Cell::new(&f.modified_at),
                    Cell::new(&f.path),
                ]);
            }
            println!("{table}");
            println!();
        }
        LogFileCommands::Read { path, tail, offset, limit } => {
            match crate::logs::read_log_file(&path, tail, offset, limit) {
                Ok(content) => {
                    if content.content.is_empty() {
                        println!("{}", "File is empty.".yellow());
                        return;
                    }

                    println!(
                        "{} Lines {}-{} of {} ({})",
                        "FILE:".green().bold(),
                        content.offset + 1,
                        content.offset + content.lines_returned,
                        content.total_lines,
                        content.path
                    );
                    println!();
                    print!("{}", content.content);
                    if !content.content.ends_with('\n') {
                        println!();
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

// ── Pull ─────────────────────────────────────────────────────────────────────

async fn cmd_pull() {
    let cfg = config::load_config().unwrap_or_default();
    if cfg.hospital_code.is_empty() {
        eprintln!(
            "{} Hospital code not configured. Activate a license first.",
            "Error:".red().bold()
        );
        std::process::exit(1);
    }

    println!();
    println!(
        "  Pulling settings for {} ...",
        cfg.hospital_code.cyan().bold()
    );

    let client = match crate::firestore::FirestoreClient::new_from_config().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{} {}", "Error:".red().bold(), e.user_message());
            std::process::exit(1);
        }
    };

    let (info, license) = match client.pull_hospital_settings(&cfg.hospital_code).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{} {}", "Error:".red().bold(), e.user_message());
            std::process::exit(1);
        }
    };

    // Compare with current license
    let current = crate::licensing::load_license().ok().flatten();
    let license_changed = match &current {
        Some(cur) => {
            cur.valid_till != license.valid_till
                || cur.features.binlog_shipping != license.features.binlog_shipping
                || cur.features.point_in_time_recovery != license.features.point_in_time_recovery
                || cur.features.priority_support != license.features.priority_support
                || cur.limits.backup_retention_days != license.limits.backup_retention_days
                || cur.limits.max_storage_gb != license.limits.max_storage_gb
        }
        None => true,
    };

    if license_changed {
        if let Err(e) = crate::licensing::save_license(&license) {
            eprintln!("{} Failed to save license: {}", "Error:".red().bold(), e);
            std::process::exit(1);
        }
    }

    // Display hospital info
    println!();
    println!("{}", "HOSPITAL INFO".green().bold());
    println!();

    let mut table = Table::new();
    table.load_preset(presets::UTF8_FULL_CONDENSED);
    table.set_header(vec!["Field", "Value"]);
    table.add_row(vec!["Name", &info.name]);
    table.add_row(vec![
        "Short Name",
        info.short_name.as_deref().unwrap_or("—"),
    ]);
    table.add_row(vec!["City", info.city.as_deref().unwrap_or("—")]);
    table.add_row(vec!["Email", info.email.as_deref().unwrap_or("—")]);
    println!("{table}");

    // Display license info
    println!();
    println!("{}", "LICENSE".green().bold());
    println!();

    let mut ltable = Table::new();
    ltable.load_preset(presets::UTF8_FULL_CONDENSED);
    ltable.set_header(vec!["Property", "Value"]);

    let status_str = match license.status() {
        crate::licensing::LicenseStatus::Unlimited => "Unlimited".to_string(),
        crate::licensing::LicenseStatus::Active { days_remaining } => {
            format!("Active ({} days remaining)", days_remaining)
        }
        crate::licensing::LicenseStatus::ExpiringSoon { days_remaining } => {
            format!("Expiring Soon ({} days remaining)", days_remaining)
        }
        crate::licensing::LicenseStatus::Expired { days_ago } => {
            format!("Expired ({} days ago)", days_ago)
        }
    };
    ltable.add_row(vec!["Status", &status_str]);
    ltable.add_row(vec![
        "Valid Till",
        &license.valid_till.format("%Y-%m-%d").to_string(),
    ]);
    ltable.add_row(vec![
        "Binlog Shipping",
        if license.features.binlog_shipping {
            "Yes"
        } else {
            "No"
        },
    ]);
    ltable.add_row(vec![
        "Point-in-Time Recovery",
        if license.features.point_in_time_recovery {
            "Yes"
        } else {
            "No"
        },
    ]);
    ltable.add_row(vec![
        "Priority Support",
        if license.features.priority_support {
            "Yes"
        } else {
            "No"
        },
    ]);
    ltable.add_row(vec![
        "Backup Retention",
        &format!("{} days", license.limits.backup_retention_days),
    ]);
    ltable.add_row(vec![
        "Max Storage",
        &format!("{} GB", license.limits.max_storage_gb),
    ]);
    println!("{ltable}");
    println!();

    if license_changed {
        println!(
            "  {} License updated from cloud.",
            "✓".green().bold()
        );
    } else {
        println!("  License is up to date.");
    }
    println!();
}

// ── JAR Pull ────────────────────────────────────────────────────────────────

async fn cmd_pull_jars(services_arg: &str) {
    let services: Vec<String> = if services_arg == "all" {
        releases::all_updatable_services()
    } else {
        services_arg.split(',').map(|s| s.trim().to_string()).collect()
    };

    println!();
    println!(
        "  Pulling {} service(s)...",
        services.len()
    );
    println!();

    match releases::pull_all_with_jres(&services).await {
        Ok(result) => {
            let mut table = Table::new();
            table.load_preset(presets::UTF8_FULL_CONDENSED);
            table.set_header(vec![
                Cell::new("Service").set_alignment(CellAlignment::Left),
                Cell::new("Status").set_alignment(CellAlignment::Center),
                Cell::new("SHA").set_alignment(CellAlignment::Left),
                Cell::new("Size").set_alignment(CellAlignment::Right),
                Cell::new("Message").set_alignment(CellAlignment::Left),
            ]);

            for r in &result.results {
                let status_cell = if r.success {
                    Cell::new("OK").fg(Color::Green)
                } else {
                    Cell::new("FAIL").fg(Color::Red)
                };
                let sha = if r.short_sha.is_empty() {
                    "—".to_string()
                } else {
                    r.short_sha.clone()
                };
                let size = if r.size_mb > 0.0 {
                    format!("{:.1} MB", r.size_mb)
                } else {
                    "—".to_string()
                };
                table.add_row(vec![
                    Cell::new(&r.service),
                    status_cell,
                    Cell::new(&sha),
                    Cell::new(&size),
                    Cell::new(&r.message),
                ]);
            }

            println!("{table}");
            println!();

            let ok_count = result.results.iter().filter(|r| r.success).count();
            let fail_count = result.results.len() - ok_count;
            if fail_count == 0 {
                println!(
                    "  All {} services pulled ({:.1} MB total)",
                    ok_count,
                    result.total_size_mb
                );
            } else {
                println!(
                    "  {}/{} services pulled, {} failed ({:.1} MB total)",
                    ok_count,
                    result.results.len(),
                    fail_count,
                    result.total_size_mb
                );
            }
            println!();
        }
        Err(e) => {
            eprintln!("{} {}", "Error:".red().bold(), e.user_message());
            std::process::exit(1);
        }
    }
}

async fn cmd_jar_updates() {
    let services = releases::all_updatable_services();

    println!();
    println!("  Checking JAR updates for {} services...", services.len());
    println!();

    match releases::check_jar_updates_available(&services).await {
        Ok(checks) => {
            let mut table = Table::new();
            table.load_preset(presets::UTF8_FULL_CONDENSED);
            table.set_header(vec![
                Cell::new("Service").set_alignment(CellAlignment::Left),
                Cell::new("Current SHA").set_alignment(CellAlignment::Left),
                Cell::new("Latest SHA").set_alignment(CellAlignment::Left),
                Cell::new("Built At").set_alignment(CellAlignment::Left),
                Cell::new("Update?").set_alignment(CellAlignment::Center),
            ]);

            for c in &checks {
                let current = if c.current_sha.is_empty() {
                    "not installed".to_string()
                } else {
                    c.current_sha.clone()
                };
                let update_cell = if c.update_available {
                    Cell::new("Yes").fg(Color::Yellow)
                } else {
                    Cell::new("No").fg(Color::Green)
                };
                table.add_row(vec![
                    Cell::new(&c.service),
                    Cell::new(&current),
                    Cell::new(&c.latest_sha),
                    Cell::new(&c.latest_built_at),
                    update_cell,
                ]);
            }

            println!("{table}");
            println!();

            let updates = checks.iter().filter(|c| c.update_available).count();
            if updates > 0 {
                println!(
                    "  {} update(s) available. Run `puru pull-jars` to update.",
                    updates
                );
            } else {
                println!("  All services up to date.");
            }
            println!();
        }
        Err(e) => {
            eprintln!("{} {}", "Error:".red().bold(), e.user_message());
            std::process::exit(1);
        }
    }
}

// ── Update / Rollback (Native) ──────────────────────────────────────────────

async fn cmd_update(service: &str) {
    println!();
    println!("  Updating {} ...", service.cyan().bold());

    match services::update_native_service(service).await {
        Ok(result) => {
            println!();
            println!("  {} Updated {} to build {}", "✓".green().bold(), service, result.short_sha.cyan());
            println!("  Size: {:.1} MB", result.size_mb);
            println!();
        }
        Err(e) => {
            eprintln!("{} {}", "Error:".red().bold(), e);
            std::process::exit(1);
        }
    }
}

async fn cmd_rollback(service: &str) {
    println!();
    println!("  Rolling back {} ...", service.cyan().bold());

    match services::rollback_native_service(service).await {
        Ok(()) => {
            println!();
            println!("  {} Rolled back {} to previous JAR", "✓".green().bold(), service);
            println!();
        }
        Err(e) => {
            eprintln!("{} {}", "Error:".red().bold(), e);
            std::process::exit(1);
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

    let mode_str = format!("{:?}", cfg.deployment_mode);
    table.add_row(vec!["Deployment Mode", &mode_str]);
    table.add_row(vec!["Hospital Code", &cfg.hospital_code]);
    table.add_row(vec!["Server IP", &cfg.server_ip]);
    if cfg.deployment_mode == config::DeploymentMode::Docker {
        table.add_row(vec!["Docker Compose", &cfg.docker_compose_path]);
    } else {
        table.add_row(vec!["JARs Dir", &cfg.jars_dir().display().to_string()]);
        table.add_row(vec!["JREs Dir", &cfg.jres_dir().display().to_string()]);
        table.add_row(vec!["Native Logs", &cfg.native_logs_dir().display().to_string()]);
    }
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

// ── Network ──────────────────────────────────────────────────────────────────

async fn cmd_network(speed: bool, json: bool) {
    if speed {
        // Full speed test
        println!("{}", "Running speed test...".cyan());
        match network::run_speed_test().await {
            Ok(result) => {
                if json {
                    println!("{}", serde_json::to_string_pretty(&result).unwrap());
                    return;
                }

                if !result.connected {
                    println!(
                        "\n  {} {}\n",
                        "●".red(),
                        "Disconnected — no internet access".red().bold()
                    );
                    return;
                }

                let mut table = Table::new();
                table
                    .load_preset(presets::UTF8_FULL_CONDENSED)
                    .set_header(vec![
                        Cell::new("Metric").set_alignment(CellAlignment::Left),
                        Cell::new("Value").set_alignment(CellAlignment::Right),
                    ]);

                table.add_row(vec![
                    Cell::new("Internet"),
                    Cell::new("Connected").fg(Color::Green),
                ]);

                if let Some(ms) = result.latency_ms {
                    let color = if ms < 100 { Color::Green } else if ms < 300 { Color::Yellow } else { Color::Red };
                    table.add_row(vec![
                        Cell::new("Latency"),
                        Cell::new(format!("{} ms", ms)).fg(color),
                    ]);
                }

                // DICOM Server
                if result.gcp_reachable {
                    let gcp_text = match result.gcp_latency_ms {
                        Some(ms) => format!("Reachable ({} ms)", ms),
                        None => "Reachable".to_string(),
                    };
                    table.add_row(vec![
                        Cell::new("DICOM Server"),
                        Cell::new(gcp_text).fg(Color::Green),
                    ]);
                } else {
                    table.add_row(vec![
                        Cell::new("DICOM Server"),
                        Cell::new("Unreachable").fg(Color::Red),
                    ]);
                }

                if let Some(dl) = result.download_mbps {
                    let color = if dl > 10.0 { Color::Green } else if dl > 2.0 { Color::Yellow } else { Color::Red };
                    table.add_row(vec![
                        Cell::new("Download"),
                        Cell::new(format!("{:.2} Mbps", dl)).fg(color),
                    ]);
                }

                if let Some(ul) = result.upload_mbps {
                    let color = if ul > 5.0 { Color::Green } else if ul > 1.0 { Color::Yellow } else { Color::Red };
                    table.add_row(vec![
                        Cell::new("Upload"),
                        Cell::new(format!("{:.2} Mbps", ul)).fg(color),
                    ]);
                }

                table.add_row(vec![
                    Cell::new("Test Duration"),
                    Cell::new(format!("{} ms", result.duration_ms)),
                ]);

                println!("\n{}\n", table);
            }
            Err(e) => {
                eprintln!("{} {}", "Error:".red().bold(), e);
            }
        }
    } else {
        // Quick connectivity check
        match network::check_network().await {
            Ok(status) => {
                if json {
                    println!("{}", serde_json::to_string_pretty(&status).unwrap());
                    return;
                }

                if status.connected {
                    let latency = status.latency_ms.unwrap_or(0);
                    let latency_str = if latency < 100 {
                        format!("{}", latency).green()
                    } else if latency < 300 {
                        format!("{}", latency).yellow()
                    } else {
                        format!("{}", latency).red()
                    };
                    println!(
                        "\n  {} {} (latency: {} ms)",
                        "●".green(),
                        "Connected".green().bold(),
                        latency_str
                    );

                    // DICOM Server status
                    if status.gcp_reachable {
                        let gcp_latency = status.gcp_latency_ms.map(|ms| format!(" ({} ms)", ms)).unwrap_or_default();
                        println!(
                            "  {} DICOM Server: {}{}",
                            "●".green(),
                            "Reachable".green(),
                            gcp_latency.dimmed()
                        );
                    } else {
                        println!(
                            "  {} DICOM Server: {} — DICOM uploads will fail",
                            "●".red(),
                            "Unreachable".red().bold()
                        );
                    }
                    println!();
                } else {
                    println!(
                        "\n  {} {}",
                        "●".red(),
                        "Disconnected — no internet access".red().bold()
                    );
                    println!(
                        "  {} DICOM Server: {}\n",
                        "●".red(),
                        "Unreachable".red()
                    );
                }
                println!(
                    "  {} Run {} for download/upload speeds.\n",
                    "Tip:".dimmed(),
                    "puru network --speed".cyan()
                );
            }
            Err(e) => {
                eprintln!("{} {}", "Error:".red().bold(), e);
            }
        }
    }
}

// ── Version ──────────────────────────────────────────────────────────────────

fn cmd_version() {
    println!(
        "puru-nucleus {}",
        env!("CARGO_PKG_VERSION")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_time_arg_unix_timestamp() {
        assert_eq!(parse_time_arg("1709553600").unwrap(), 1709553600);
    }

    #[test]
    fn test_parse_time_arg_iso8601() {
        let result = parse_time_arg("2026-03-04T10:00:00").unwrap();
        // 2026-03-04T10:00:00 UTC
        let expected = chrono::NaiveDateTime::parse_from_str("2026-03-04T10:00:00", "%Y-%m-%dT%H:%M:%S")
            .unwrap()
            .and_utc()
            .timestamp();
        assert_eq!(result, expected);
    }

    #[test]
    fn test_parse_time_arg_relative_hours() {
        let now = chrono::Utc::now().timestamp();
        let result = parse_time_arg("2h").unwrap();
        // Should be approximately now - 7200, allow 2 seconds tolerance
        assert!((result - (now - 7200)).abs() < 2);
    }

    #[test]
    fn test_parse_time_arg_relative_days() {
        let now = chrono::Utc::now().timestamp();
        let result = parse_time_arg("1d").unwrap();
        assert!((result - (now - 86400)).abs() < 2);
    }

    #[test]
    fn test_parse_time_arg_relative_compound() {
        let now = chrono::Utc::now().timestamp();
        let result = parse_time_arg("3d12h").unwrap();
        let expected = now - (3 * 86400 + 12 * 3600);
        assert!((result - expected).abs() < 2);
    }

    #[test]
    fn test_parse_time_arg_invalid() {
        assert!(parse_time_arg("abc").is_err());
        assert!(parse_time_arg("2x").is_err());
        assert!(parse_time_arg("").is_err());
    }
}
