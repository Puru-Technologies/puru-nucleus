//! Interactive terminal setup for servers with no display.
//!
//! `sudo puru-nucleus` on a headless box (or `puru-nucleus setup` anywhere)
//! walks the operator through what the GUI's activation screen and setup
//! wizard do: redeem the onboarding code, name the machine, then either adopt
//! the Puru deployment already on the box or install a fresh one. Every step
//! calls the same backend function the GUI uses; this module only asks the
//! questions and shows progress.
//!
//! "Connect only" (`--connect-only`, or the first choice when a setup is found)
//! is for sites already running Puru in Docker: it starts the daemon (token
//! server, telemetry, remote logs) and changes nothing in the deployment.

use std::future::Future;
use std::time::{Duration, Instant};

use colored::Colorize;
use dialoguer::{theme::ColorfulTheme, Confirm, Input, Password, Select};
use indicatif::{ProgressBar, ProgressStyle};

use crate::config::{self, DeploymentMode, NucleusConfig};

/// MySQL account the services use on a fresh install. Nucleus creates it with
/// a generated password, so the operator never has to choose one.
const SERVICE_DB_USER: &str = "puru";
/// The compose fragments mount /home/puru/data, so the data root lives there.
const DEFAULT_DATA_DIR: &str = "/home/puru/data";

/// True when no graphical session is available (SSH / server console), so the
/// GUI can't open and a bare `puru-nucleus` should run this flow instead.
pub fn is_headless() -> bool {
    #[cfg(target_os = "linux")]
    {
        let set = |k: &str| std::env::var(k).map(|v| !v.is_empty()).unwrap_or(false);
        !set("DISPLAY") && !set("WAYLAND_DISPLAY")
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Entry point. `explicit` is true for `puru-nucleus setup`; false when a bare
/// `puru-nucleus` landed here because there's no display. `connect_only` is
/// `setup --connect-only`.
pub async fn run(explicit: bool, connect_only: bool) {
    banner();

    if !is_root() {
        println!(
            "  {} Setup changes system files and services, so it needs root.\n\n      {}\n",
            "!".yellow().bold(),
            "sudo puru-nucleus".cyan().bold()
        );
        std::process::exit(1);
    }
    if !console::user_attended() {
        println!(
            "  {} This is an interactive setup; run it from a terminal (e.g. over SSH).\n",
            "!".yellow().bold()
        );
        std::process::exit(1);
    }

    match flow(explicit, connect_only).await {
        Ok(()) => {}
        Err(Abort(msg)) => {
            if !msg.is_empty() {
                println!("\n  {} {}\n", "✖".red().bold(), msg);
            }
            println!(
                "  Nothing is lost — run {} again to continue where you left off.\n",
                "sudo puru-nucleus setup".cyan()
            );
            std::process::exit(1);
        }
    }
}

/// Stop the flow with a message for the operator.
struct Abort(String);

impl From<dialoguer::Error> for Abort {
    fn from(e: dialoguer::Error) -> Self {
        Abort(format!("Input cancelled ({})", e))
    }
}

async fn flow(explicit: bool, connect_only: bool) -> Result<(), Abort> {
    let theme = ColorfulTheme::default();

    // ── 1. Activation ────────────────────────────────────────────────────────
    let licensed = matches!(crate::licensing::load_license(), Ok(Some(_)));
    if !licensed {
        section("Activate this server");
        println!(
            "  This server isn't activated yet. You'll need the 10-character\n  onboarding code from the hospital's welcome email.\n"
        );
        check_clock().await;
        activate(&theme).await?;
    } else if let Err(why) = cloud_key_status() {
        // Activated, but the hospital key can't be used: the token server,
        // telemetry and Oxygen commands are all down until it's downloaded again.
        section("Activate this server again");
        println!("  {} {}\n", "!".yellow().bold(), why);
        println!(
            "  Enter a new onboarding code to download the hospital key again.\n  Nothing else on this server changes.\n"
        );
        check_clock().await;
        activate(&theme).await?;
    }

    let config = load_config()?;
    if config.setup_completed {
        if !explicit {
            show_ready(&config).await;
            return Ok(());
        }
        section("Setup");
        let again = Confirm::with_theme(&theme)
            .with_prompt("Setup has already been completed on this server. Run it again?")
            .default(false)
            .interact()?;
        if !again {
            return Ok(());
        }
    }

    if connect_only {
        return connect_existing(&theme).await;
    }

    // ── 2. Existing deployment or fresh install ─────────────────────────────
    section("Looking for an existing Puru setup");
    let spinner = spin("Scanning Docker containers…");
    let detection = crate::services::detect_existing_setup().await.ok();
    spinner.finish_and_clear();

    match detection {
        Some(d) if d.found => {
            println!(
                "  {} Found {} Puru containers{}",
                "●".green(),
                d.containers.len(),
                d.compose_path
                    .as_deref()
                    .map(|p| format!(" — {}", p.dimmed()))
                    .unwrap_or_default()
            );
            for c in d.containers.iter().take(14) {
                println!("      {:<14} {}", c.name, c.status.dimmed());
            }
            println!();
            let choice = Select::with_theme(&theme)
                .with_prompt("What should Nucleus do?")
                .items(&[
                    "Connect only — token server, telemetry and logs; change nothing (recommended)",
                    "Adopt — also set up local domain, backups and HTTPS",
                    "Stop",
                ])
                .default(0)
                .interact()?;
            match choice {
                0 => connect_existing(&theme).await,
                1 => adopt_existing(&theme, d.compose_path).await,
                _ => Err(Abort(
                    "Stopped. A fresh install on top of a running hospital would replace it — remove the old setup first if that's what you want.".into(),
                )),
            }
        }
        _ => {
            println!("  {} No existing Puru containers — this will be a fresh install.\n", "●".cyan());
            fresh_install(&theme).await
        }
    }
}

// ── Activation ───────────────────────────────────────────────────────────────

async fn activate(theme: &ColorfulTheme) -> Result<(), Abort> {
    let redeemed = loop {
        let code: String = Input::with_theme(theme)
            .with_prompt("Onboarding code")
            .validate_with(|s: &String| {
                let s = s.trim();
                if s.len() == 10 && s.chars().all(|c| c.is_ascii_alphanumeric()) {
                    Ok(())
                } else {
                    Err("The code is 10 letters/numbers, e.g. 7KQ9M2XR4T")
                }
            })
            .interact_text()?;

        let spinner = spin("Checking the code…");
        let result = crate::commands::redeem_onboarding_code(code).await;
        spinner.finish_and_clear();

        match result {
            Ok(r) => break r,
            Err(raw) => {
                print_redeem_error(&raw);
                let retry = Confirm::with_theme(theme)
                    .with_prompt("Try another code?")
                    .default(true)
                    .interact()?;
                if !retry {
                    return Err(Abort(String::new()));
                }
            }
        }
    };

    println!();
    println!("  {} Code accepted\n", "✔".green().bold());
    kv("Hospital", &redeemed.hospital_name);
    kv("Code", &redeemed.hospital_code);
    kv("Email", &redeemed.hospital_email);
    println!();

    let machine_name = if redeemed.machine_status == "known" {
        let name = if redeemed.machine_name.is_empty() {
            "This machine".to_string()
        } else {
            redeemed.machine_name.clone()
        };
        println!("  This server is already registered as {}.\n", name.bold());
        name
    } else {
        Input::with_theme(theme)
            .with_prompt("Name this machine")
            .default(default_machine_name())
            .interact_text()?
    };

    let ok = Confirm::with_theme(theme)
        .with_prompt(format!(
            "Activate \"{}\" for {}?",
            machine_name, redeemed.hospital_name
        ))
        .default(true)
        .interact()?;
    if !ok {
        return Err(Abort("Activation cancelled.".into()));
    }

    let spinner = spin("Activating…");
    let result = crate::commands::activate_license(redeemed.hospital_email, machine_name).await;
    spinner.finish_and_clear();
    result.map_err(|e| Abort(format!("Activation failed: {}", e)))?;
    println!("  {} Activated\n", "✔".green().bold());
    Ok(())
}

/// Redeem errors come back as JSON (`{"kind": "...", ...}`) for the GUI.
fn print_redeem_error(raw: &str) {
    let v: serde_json::Value = serde_json::from_str(raw).unwrap_or_default();
    match v.get("kind").and_then(|k| k.as_str()) {
        Some("machine_limit_reached") => {
            println!(
                "\n  {} This hospital already has its maximum number of machines:",
                "✖".red().bold()
            );
            if let Some(list) = v.get("existing_machines").and_then(|m| m.as_array()) {
                for m in list {
                    let name = m.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                    let seen = m
                        .get("last_seen_at")
                        .and_then(|n| n.as_str())
                        .map(|s| format!("last seen {}", s))
                        .unwrap_or_default();
                    println!("      • {} {}", name, seen.dimmed());
                }
            }
            println!("    Ask an admin to reset one in Puru Oxygen, then try again.\n");
        }
        _ => {
            let msg = v
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or(raw);
            println!("\n  {} {}\n", "✖".red().bold(), msg);
        }
    }
}

/// Whether the hospital key (cloud credentials) is present and readable.
pub fn cloud_key_status() -> Result<(), String> {
    let config = config::load_config().map_err(|e| e.user_message())?;
    let path = config
        .gcs_credentials_path
        .filter(|p| !p.is_empty())
        .ok_or("No hospital key is configured on this server.")?;
    if !std::path::Path::new(&path).exists() {
        return Err(format!("The hospital key file is missing ({}).", path));
    }
    crate::secret::read_decrypted(&path).map(|_| ()).map_err(|e| e.to_string())
}

fn default_machine_name() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Main Server".to_string())
}

async fn check_clock() {
    if let Ok(clock) = crate::commands::check_system_clock().await {
        if clock.checked && !clock.ok {
            println!(
                "  {} {}\n    Fix: {}\n",
                "!".yellow().bold(),
                clock.message,
                "sudo timedatectl set-ntp true".cyan()
            );
        }
    }
}

// ── Adopt an existing deployment ─────────────────────────────────────────────

async fn adopt_existing(theme: &ColorfulTheme, compose_path: Option<String>) -> Result<(), Abort> {
    section("Adopting the existing setup");

    let mut config = load_config()?;
    config.deployment_mode = DeploymentMode::Docker;
    if let Some(p) = compose_path {
        config.docker_compose_path = p;
    }
    save_config(&config)?;

    // Hospital code, server IP and DB login come from the setup's env files.
    let applied = crate::commands::apply_detected_environment(None)
        .await
        .map_err(|e| Abort(format!("Couldn't read the existing env files: {}", e)))?;
    if let Some(m) = applied.mismatches.iter().find(|m| m.field == "hospital_code") {
        return Err(Abort(format!(
            "This server was activated for \"{}\" but the existing setup belongs to \"{}\". They must match — use that hospital's onboarding code.",
            m.config_value, m.detected_value
        )));
    }

    let mut config = load_config()?;
    if !mysql_login_ok(&config.mysql_user, &config.mysql_password).await {
        println!(
            "  {} Couldn't log in to MySQL with the account in the env files ({}).",
            "!".yellow().bold(),
            if config.mysql_user.is_empty() { "none" } else { &config.mysql_user }
        );
        ask_mysql_login(theme, &mut config).await?;
    } else {
        println!("  {} MySQL login from the env files works ({})", "✔".green(), config.mysql_user);
    }

    ask_server_ip(theme, &mut config)?;
    save_config(&config)?;

    println!();
    section("Finishing");
    let code = config.hospital_code.clone();
    run_step(theme, "Configure local domain (puru.local)", || local_domain(code.clone())).await?;
    run_step(theme, "Configure backups", crate::commands::setup_configure_backups).await?;
    run_step(theme, "Install and start the daemon", crate::commands::setup_install_daemon).await?;
    run_step(theme, "Configure HTTPS (TLS)", crate::commands::setup_tls).await?;

    finish().await
}

// ── Connect only ─────────────────────────────────────────────────────────────

/// What `connect_existing` found on the server.
struct Existing {
    containers: Vec<crate::services::ContainerInfo>,
    compose_path: Option<String>,
    /// Hospital code the running deployment is configured for, and where it was read.
    hospital_code: Option<(String, String)>,
    /// Daemon port the containers were given in PURU_TOKEN_ENDPOINT, if any.
    token_port: Option<u16>,
}

/// Connect a server that already runs Puru in Docker: start the daemon (token
/// server, telemetry, remote logs) and nothing else. Only nucleus's own files
/// are written; containers, compose, env files, MySQL, nginx and hosts are not
/// touched. Stops without changes if no existing setup is found.
async fn connect_existing(theme: &ColorfulTheme) -> Result<(), Abort> {
    section("Checking the existing Puru setup");
    let spinner = spin("Looking at Docker containers, compose and env files…");
    let found = find_existing().await;
    spinner.finish_and_clear();
    let found = found?;

    let licensed_code = load_config()?.hospital_code;
    kv("Compose", found.compose_path.as_deref().unwrap_or("not found (containers only)"));
    kv("Containers", &found.containers.len().to_string());
    for c in found.containers.iter().take(16) {
        println!("      {:<14} {}", c.name, c.status.dimmed());
    }
    println!();

    match &found.hospital_code {
        Some((code, source)) if code.eq_ignore_ascii_case(&licensed_code) => {
            println!(
                "  {} Hospital {} matches this activation {}",
                "✔".green().bold(),
                code.bold(),
                format!("(from {})", source).dimmed()
            );
        }
        Some((code, source)) => {
            return Err(Abort(format!(
                "This server was activated for \"{}\", but the running setup is configured for \"{}\" (from {}). Use that hospital's onboarding code. Nothing was changed.",
                licensed_code, code, source
            )));
        }
        None => {
            println!(
                "  {} Couldn't read the hospital code from the running setup.",
                "!".yellow().bold()
            );
            let typed: String = Input::with_theme(theme)
                .with_prompt(format!("Type {} to confirm this server belongs to it", licensed_code))
                .interact_text()?;
            if !typed.trim().eq_ignore_ascii_case(&licensed_code) {
                return Err(Abort("The code didn't match. Nothing was changed.".into()));
            }
        }
    }

    println!();
    println!("  Nucleus will only:");
    for line in [
        "start the token server for pacs, realtime and comm",
        "send telemetry and alerts to Puru Oxygen",
        "answer log requests from Puru Oxygen",
    ] {
        println!("    {} {}", "+".green(), line);
    }
    println!("  It will not touch:");
    for line in [
        "the containers (no pull, restart or recreate)",
        "docker-compose.yml or the env files",
        "MySQL, nginx, HTTPS or /etc/hosts",
        "backups or updates (both stay off)",
    ] {
        println!("    {} {}", "·".dimmed(), line);
    }
    println!();
    let ok = Confirm::with_theme(theme)
        .with_prompt("Connect this server?")
        .default(true)
        .interact()?;
    if !ok {
        return Err(Abort("Nothing was changed.".into()));
    }
    println!();

    let mut config = load_config()?;
    config.deployment_mode = DeploymentMode::Docker;
    if let Some(p) = &found.compose_path {
        config.docker_compose_path = p.clone();
    }
    config.backup_enabled = false;
    config.auto_update_enabled = false;
    config.telemetry_enabled = true;
    config.lan.enabled = false;
    let daemon = config.daemon.get_or_insert_with(Default::default);
    daemon.backup_schedule.enabled = false;
    if let Some(port) = found.token_port {
        daemon.port = port;
    }
    let port = daemon.port;
    config.setup_completed = true;
    save_config(&config)?;

    // The watchdog starts services that are down; leave the ones that were
    // already stopped as they are.
    for c in found.containers.iter().filter(|c| !c.status.starts_with("Up")) {
        crate::services::mark_manually_stopped(&c.name);
    }

    run_step(theme, "Start the token server (daemon)", start_daemon).await?;
    run_step(theme, "Check the token server issues a token", || check_token(port)).await?;

    println!();
    println!("  {}", "Connected".green().bold());
    println!();
    show_ready(&config).await;
    Ok(())
}

/// Find the running deployment: every container (running or stopped) that is
/// Puru by registry match or image, the compose file Docker Compose recorded on
/// them (else the usual paths), and the hospital code from the env file or the
/// containers' own environment.
async fn find_existing() -> Result<Existing, Abort> {
    if shell_output("docker", &["info", "--format", "{{.ServerVersion}}"]).await.is_err() {
        return Err(Abort(
            "Docker isn't running (or isn't installed), so there's no existing Puru setup to connect. Nothing was changed.".into(),
        ));
    }
    let detection = crate::services::detect_existing_setup().await.ok();
    let mut containers = detection.as_ref().map(|d| d.containers.clone()).unwrap_or_default();

    // Containers the registry doesn't know but that run a Puru image, plus the
    // compose file each was started from.
    let listing = shell_output(
        "docker",
        &[
            "ps", "-a", "--format",
            "{{.Names}}\t{{.Image}}\t{{.Status}}\t{{.Label \"com.docker.compose.project.config_files\"}}",
        ],
    )
    .await
    .unwrap_or_default();
    let mut label_compose: Option<String> = None;
    for row in listing.lines() {
        let cols: Vec<&str> = row.split('\t').collect();
        if cols.len() < 3 {
            continue;
        }
        let (name, image, status) = (cols[0], cols[1], cols[2]);
        let known = containers.iter().any(|c| c.name == name);
        if !known && !image.to_lowercase().contains("puru") {
            continue;
        }
        if !known {
            containers.push(crate::services::ContainerInfo {
                name: name.to_string(),
                image: image.to_string(),
                status: status.to_string(),
            });
        }
        if label_compose.is_none() {
            label_compose = cols
                .get(3)
                .into_iter()
                .flat_map(|f| f.split(','))
                .map(str::trim)
                .find(|f| !f.is_empty() && std::path::Path::new(f).is_file())
                .map(String::from);
        }
    }

    if containers.is_empty() {
        return Err(Abort(
            "No Puru containers on this server (checked every container, running or stopped). Connect only works on an existing setup, so nothing was changed.".into(),
        ));
    }

    let compose_path = label_compose.or_else(|| detection.and_then(|d| d.compose_path));

    let general_env = compose_path
        .as_deref()
        .and_then(|p| std::path::Path::new(p).parent())
        .map(|d| d.join("env").join("general.env"));
    let mut hospital_code = general_env.as_ref().and_then(|f| {
        read_env_value(f, "hospital.short.code").map(|c| (c, f.display().to_string()))
    });
    if hospital_code.is_none() {
        for c in &containers {
            let env = shell_output(
                "docker",
                &["inspect", "-f", "{{range .Config.Env}}{{println .}}{{end}}", &c.name],
            )
            .await
            .unwrap_or_default();
            if let Some(code) = env_value(&env, "hospital.short.code") {
                hospital_code = Some((code, format!("container {}", c.name)));
                break;
            }
        }
    }

    let token_port = compose_path
        .as_deref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| token_port_in(&s));

    Ok(Existing {
        containers,
        compose_path,
        hospital_code,
        token_port,
    })
}

fn read_env_value(path: &std::path::Path, key: &str) -> Option<String> {
    env_value(&std::fs::read_to_string(path).ok()?, key)
}

/// `key=value` lookup in env-file text; empty and `{{PLACEHOLDER}}` values don't count.
fn env_value(text: &str, key: &str) -> Option<String> {
    text.lines()
        .filter_map(|l| l.trim().strip_prefix(key)?.strip_prefix('='))
        .map(|v| v.trim().trim_matches('"').to_string())
        .find(|v| !v.is_empty() && !v.contains("{{"))
}

/// Port from a filled-in `PURU_TOKEN_ENDPOINT=http://127.0.0.1:<port>/…`.
fn token_port_in(compose: &str) -> Option<u16> {
    compose.lines().find_map(|l| {
        let rest = &l[l.find("PURU_TOKEN_ENDPOINT")?..];
        let after = &rest[rest.find("127.0.0.1:")? + "127.0.0.1:".len()..];
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse().ok()
    })
}

/// Install the daemon and (re)start it so it runs with the config just saved.
async fn start_daemon() -> Result<(), String> {
    let was_running = crate::platform::service_status()
        .await
        .map(|s| s.running)
        .unwrap_or(false);
    crate::commands::setup_install_daemon().await?;
    if was_running {
        crate::platform::stop_service().await?;
        crate::platform::start_service().await?;
    }
    Ok(())
}

/// Ask the token server for a pacs token the way the containers do (loopback,
/// no key). Gives it ~30 s to come up.
async fn check_token(port: u16) -> Result<(), String> {
    let url = format!("http://127.0.0.1:{}/api/gcp/token", port);
    let client = reqwest::Client::new();
    let mut last = String::new();
    for _ in 0..15 {
        match client
            .post(&url)
            .json(&serde_json::json!({ "service": "puru-pacs" }))
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => {
                let v: serde_json::Value = r.json().await.unwrap_or_default();
                let has_token = v
                    .get("access_token")
                    .and_then(|t| t.as_str())
                    .map(|t| !t.is_empty())
                    .unwrap_or(false);
                if has_token {
                    return Ok(());
                }
                last = "it answered without a token".into();
            }
            Ok(r) => {
                let status = r.status();
                last = format!("HTTP {}: {}", status, r.text().await.unwrap_or_default());
            }
            Err(e) => last = format!("not answering yet ({})", e),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    Err(format!("{} — {}", url, last))
}

// ── Fresh install ────────────────────────────────────────────────────────────

async fn fresh_install(theme: &ColorfulTheme) -> Result<(), Abort> {
    section("Prerequisites");
    ensure_docker(theme).await?;

    let mut config = load_config()?;
    ensure_mysql(theme, &mut config).await?;

    config.deployment_mode = DeploymentMode::Docker;
    if config.puru_data_path.as_deref().map(str::is_empty).unwrap_or(true) {
        config.puru_data_path = Some(DEFAULT_DATA_DIR.to_string());
    }
    ask_server_ip(theme, &mut config)?;
    save_config(&config)?;

    println!();
    section("Installing");
    println!("  {}\n", "Pulling images can take 10+ minutes on a slow connection.".dimmed());

    let code = config.hospital_code.clone();
    use crate::commands as c;
    run_step(theme, "Check prerequisites", c::setup_check_prerequisites).await?;
    run_step(theme, "Create MySQL databases", c::setup_create_databases).await?;
    run_step(theme, "Generate configuration files", c::setup_generate_config).await?;
    run_step(theme, "Pull Docker images", c::setup_pull_images).await?;
    run_step(theme, "Start services", c::setup_start_services).await?;
    run_step(theme, "Health check", c::setup_health_check).await?;
    run_step(theme, "Configure local domain (puru.local)", || local_domain(code.clone())).await?;
    run_step(theme, "Seed database & templates", c::setup_seed_database).await?;
    run_step(theme, "Initialize roles & users", c::setup_init_auth).await?;
    run_step(theme, "Configure backups", c::setup_configure_backups).await?;
    run_step(theme, "Install and start the daemon", c::setup_install_daemon).await?;
    run_step(theme, "Configure HTTPS (TLS)", c::setup_tls).await?;

    finish().await
}

/// Docker + Compose V2, installed from Ubuntu's own packages if missing.
async fn ensure_docker(theme: &ColorfulTheme) -> Result<(), Abort> {
    let prereqs = crate::services::check_prerequisites().await.unwrap_or_default();
    let have = |name: &str| prereqs.iter().any(|p| p.name == name && p.installed);
    if have("Docker") && have("Docker Compose") {
        println!("  {} Docker and Docker Compose are installed", "✔".green());
        return Ok(());
    }
    let install = Confirm::with_theme(theme)
        .with_prompt("Docker isn't installed. Install it now (apt: docker.io + docker-compose-v2)?")
        .default(true)
        .interact()?;
    if !install {
        return Err(Abort("Docker is required for a Puru install.".into()));
    }
    apt_install("Installing Docker", &["docker.io", "docker-compose-v2"]).await?;
    shell("systemctl", &["enable", "--now", "docker"]).await?;
    println!("  {} Docker installed", "✔".green());
    Ok(())
}

/// Make sure MySQL is installed and nucleus has its own login, without asking
/// for a password unless it has to:
/// - not installed → install it; root can then log in through the socket;
/// - installed and root can use the socket (Ubuntu's default) → no question;
/// - root needs a password → ask for it once.
/// Then create the `puru` account with a generated password for the services.
async fn ensure_mysql(theme: &ColorfulTheme, config: &mut NucleusConfig) -> Result<(), Abort> {
    if crate::installer::mysql_service_name().is_none() {
        let install = Confirm::with_theme(theme)
            .with_prompt("MySQL isn't installed. Install MySQL 8 now (apt: mysql-server)?")
            .default(true)
            .interact()?;
        if !install {
            return Err(Abort("MySQL is required for a Puru install.".into()));
        }
        apt_install("Installing MySQL", &["mysql-server", "mysql-client"]).await?;
        shell("systemctl", &["enable", "--now", "mysql"]).await?;
        println!("  {} MySQL installed", "✔".green());
    } else {
        println!("  {} MySQL is installed", "✔".green());
    }

    // Root over the unix socket: works on a stock Ubuntu install, no password.
    let root_password = if mysql_root(None, "SELECT 1").await.is_ok() {
        None
    } else {
        println!("  MySQL's root account needs a password on this server.");
        let mut tries = 0;
        loop {
            let pw = Password::with_theme(theme)
                .with_prompt("MySQL root password")
                .interact()?;
            if mysql_root(Some(&pw), "SELECT 1").await.is_ok() {
                break Some(pw);
            }
            tries += 1;
            println!("  {} That password didn't work.", "✖".red());
            if tries >= 3 {
                return Err(Abort("Couldn't log in to MySQL as root.".into()));
            }
        }
    };

    let password = generate_password(24);
    let sql = ["localhost", "127.0.0.1"]
        .iter()
        .map(|host| {
            format!(
                "CREATE USER IF NOT EXISTS '{u}'@'{h}' IDENTIFIED BY '{p}'; \
                 ALTER USER '{u}'@'{h}' IDENTIFIED BY '{p}'; \
                 GRANT ALL PRIVILEGES ON *.* TO '{u}'@'{h}';",
                u = SERVICE_DB_USER,
                h = host,
                p = password
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
        + " FLUSH PRIVILEGES;";
    mysql_root(root_password.as_deref(), &sql)
        .await
        .map_err(|e| Abort(format!("Couldn't create the '{}' MySQL account: {}", SERVICE_DB_USER, e)))?;

    config.mysql_host = "127.0.0.1".into();
    config.mysql_port = 3306;
    config.mysql_user = SERVICE_DB_USER.into();
    config.mysql_password = password;
    println!(
        "  {} MySQL account '{}' ready (password generated, stored in nucleus.toml)",
        "✔".green(),
        SERVICE_DB_USER
    );
    Ok(())
}

/// Ask for a MySQL login (adopt path, when the env files' one doesn't work).
async fn ask_mysql_login(theme: &ColorfulTheme, config: &mut NucleusConfig) -> Result<(), Abort> {
    for _ in 0..3 {
        let user: String = Input::with_theme(theme)
            .with_prompt("MySQL user")
            .default(if config.mysql_user.is_empty() { "root".into() } else { config.mysql_user.clone() })
            .interact_text()?;
        let pw = Password::with_theme(theme)
            .with_prompt(format!("Password for {}", user))
            .interact()?;
        if mysql_login_ok(&user, &pw).await {
            config.mysql_user = user;
            config.mysql_password = pw;
            println!("  {} MySQL login works", "✔".green());
            return Ok(());
        }
        println!("  {} That login didn't work.", "✖".red());
    }
    Err(Abort("Couldn't log in to MySQL.".into()))
}

fn ask_server_ip(theme: &ColorfulTheme, config: &mut NucleusConfig) -> Result<(), Abort> {
    let detected = crate::local_domain::resolve_lan_ip();
    let default = detected.clone().unwrap_or_else(|| config.server_ip.clone());
    if !config.server_ip.is_empty() && Some(&config.server_ip) != detected.as_ref() {
        println!(
            "  {} The cloud has {} for this hospital; this server's LAN address is {}.",
            "i".cyan(),
            config.server_ip,
            detected.as_deref().unwrap_or("unknown")
        );
    }
    let ip: String = Input::with_theme(theme)
        .with_prompt("Server IP (staff PCs reach Puru on this address)")
        .default(default)
        .validate_with(|s: &String| {
            s.trim()
                .parse::<std::net::IpAddr>()
                .map(|_| ())
                .map_err(|_| "Enter an IP address, e.g. 192.168.1.100")
        })
        .interact_text()?;
    config.server_ip = ip.trim().to_string();
    Ok(())
}

// ── Steps ────────────────────────────────────────────────────────────────────

/// Run one setup step with a spinner. On failure, show the error and let the
/// operator retry, skip it, or stop.
async fn run_step<F, Fut, T>(theme: &ColorfulTheme, label: &str, step: F) -> Result<(), Abort>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    loop {
        let spinner = spin(label);
        let started = Instant::now();
        let result = step().await;
        spinner.finish_and_clear();
        let took = format_duration(started.elapsed());

        match result {
            Ok(_) => {
                println!("  {} {}  {}", "✔".green().bold(), label, took.dimmed());
                return Ok(());
            }
            Err(e) => {
                println!("  {} {}  {}", "✖".red().bold(), label, took.dimmed());
                for line in e.lines().take(8) {
                    println!("      {}", line.red());
                }
                let choice = Select::with_theme(theme)
                    .with_prompt("What now?")
                    .items(&["Retry this step", "Skip it and continue", "Stop setup"])
                    .default(0)
                    .interact()?;
                match choice {
                    0 => continue,
                    1 => return Ok(()),
                    _ => return Err(Abort(format!("Stopped at \"{}\".", label))),
                }
            }
        }
    }
}

/// CLI version of the GUI's local-domain step: the hosts entries. (mDNS is
/// advertised by the GUI process, which a headless server doesn't run.)
async fn local_domain(code: String) -> Result<(), String> {
    crate::local_domain::ensure_hosts_entries(&code)
        .map(|_| ())
        .map_err(|e| format!("hosts file: {}", e))
}

async fn finish() -> Result<(), Abort> {
    let mut config = load_config()?;
    config.setup_completed = true;
    save_config(&config)?;

    println!();
    println!("  {}", "Setup complete".green().bold());
    println!();
    show_ready(&config).await;
    Ok(())
}

/// What an operator needs once the server is up.
async fn show_ready(config: &NucleusConfig) {
    let daemon = crate::platform::service_status().await.ok();
    let running = daemon.as_ref().map(|d| d.running).unwrap_or(false);
    let code = config.hospital_code.to_lowercase();

    kv("Hospital", &config.hospital_code);
    kv("Open Puru", &format!("http://{}.puru.local   or   http://{}", code, config.server_ip));
    kv(
        "Daemon",
        &if running {
            "running".green().to_string()
        } else {
            "not running — sudo systemctl start puru-nucleus".red().to_string()
        },
    );
    kv(
        "Cloud key",
        &match cloud_key_status() {
            Ok(()) => "readable".green().to_string(),
            Err(_) => "UNREADABLE — run sudo puru-nucleus setup".red().to_string(),
        },
    );
    kv("Config", &config::config_dir().join("nucleus.toml").display().to_string());
    println!();
    println!("  {}", "Useful commands".bold());
    for (cmd, what) in [
        ("sudo puru-nucleus status", "services and health"),
        ("sudo puru-nucleus logs <service>", "container logs"),
        ("sudo puru-nucleus backup full", "back up now"),
        ("journalctl -u puru-nucleus -f", "daemon log"),
    ] {
        println!("    {:<36} {}", cmd.cyan(), what.dimmed());
    }
    println!();
}

// ── Shell helpers ────────────────────────────────────────────────────────────

async fn apt_install(label: &str, packages: &[&str]) -> Result<(), Abort> {
    let spinner = spin(&format!("{} ({})…", label, packages.join(", ")));
    let _ = shell_quiet("apt-get", &["update", "-qq"]).await;
    let mut args = vec!["install", "-y", "-qq"];
    args.extend_from_slice(packages);
    let result = shell_quiet("apt-get", &args).await;
    spinner.finish_and_clear();
    result.map_err(|e| Abort(format!("{} failed: {}", label, e)))
}

async fn shell(cmd: &str, args: &[&str]) -> Result<(), Abort> {
    shell_quiet(cmd, args)
        .await
        .map_err(|e| Abort(format!("`{} {}` failed: {}", cmd, args.join(" "), e)))
}

/// Run a command and return its stdout.
async fn shell_output(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = tokio::process::Command::new(cmd)
        .args(args)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

async fn shell_quiet(cmd: &str, args: &[&str]) -> Result<(), String> {
    let out = tokio::process::Command::new(cmd)
        .args(args)
        .env("DEBIAN_FRONTEND", "noninteractive")
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(err.lines().rev().take(3).collect::<Vec<_>>().join(" | "))
    }
}

/// Run SQL as MySQL root: over the unix socket when `password` is None,
/// otherwise with that password.
async fn mysql_root(password: Option<&str>, sql: &str) -> Result<(), String> {
    let mut cmd = tokio::process::Command::new("mysql");
    cmd.args(["-u", "root", "-N", "-e", sql]);
    if let Some(pw) = password {
        cmd.env("MYSQL_PWD", pw).args(["-h", "127.0.0.1"]);
    }
    let out = cmd.output().await.map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

async fn mysql_login_ok(user: &str, password: &str) -> bool {
    if user.is_empty() {
        return false;
    }
    tokio::process::Command::new("mysql")
        .args(["-h", "127.0.0.1", "-u", user, "-N", "-e", "SELECT 1"])
        .env("MYSQL_PWD", password)
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn generate_password(len: usize) -> String {
    use rand::Rng;
    const CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut rng = rand::thread_rng();
    (0..len).map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char).collect()
}

// ── Output helpers ───────────────────────────────────────────────────────────

fn banner() {
    println!();
    println!("  {}{}  {}", "puru".bold(), " labs.".bright_blue().bold(), "Nucleus".bold());
    println!("  {}", format!("Hospital server · v{}", env!("CARGO_PKG_VERSION")).dimmed());
    println!();
}

fn section(title: &str) {
    println!("  {}", title.bold().underline());
    println!();
}

fn kv(key: &str, value: &str) {
    println!("    {:<10} {}", key.dimmed(), value);
}

fn spin(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("  {spinner:.cyan} {msg}  {elapsed:.dim}")
            .unwrap()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ "),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(90));
    pb
}

fn format_duration(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 60 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{}s", s.max(1))
    }
}

fn is_root() -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn load_config() -> Result<NucleusConfig, Abort> {
    config::load_config().map_err(|e| Abort(format!("Couldn't read the config: {}", e.user_message())))
}

fn save_config(config: &NucleusConfig) -> Result<(), Abort> {
    config::save_config(config).map_err(|e| Abort(format!("Couldn't save the config: {}", e.user_message())))
}
