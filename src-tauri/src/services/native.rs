//! Native JAR process management — runs services as `java -jar` child processes
//!
//! Used when `deployment_mode = "native"` in nucleus.toml.
//! Each service runs as a child process with stdout/stderr redirected to log files.
//! PID files track running processes.

use crate::config::NucleusConfig;
use crate::error::NucleusError;
use crate::releases;
use crate::services::{HealthStatus, ServiceInfo, ServiceStatus};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Short-lived cache of the enabled-service list fetched from Firestore. The
/// services screen refetches on every load/refresh, and each fetch is a token
/// exchange + query (1–3s); caching for a few seconds keeps repeat loads snappy
/// without going noticeably stale when modules change in the admin console.
static ENABLED_SERVICES_CACHE: Mutex<Option<(Instant, Vec<String>)>> = Mutex::new(None);
const ENABLED_SERVICES_TTL: Duration = Duration::from_secs(20);

/// Resolve the enabled-service names, using a short in-memory cache to avoid a
/// Firestore round-trip on every `get_services` call. Falls back to all
/// updatable services when the cloud can't be reached.
async fn enabled_services_cached(config: &NucleusConfig) -> Vec<String> {
    if let Ok(guard) = ENABLED_SERVICES_CACHE.lock() {
        if let Some((fetched_at, list)) = guard.as_ref() {
            if fetched_at.elapsed() < ENABLED_SERVICES_TTL {
                return list.clone();
            }
        }
    }

    // The hospital's real service set is its Firestore module selection. When
    // that can't be fetched (offline, token/credential hiccup in this process),
    // fall back to what's actually INSTALLED on this box — NOT the full catalog.
    // Otherwise a fetch failure dumps every known service as a "Not installed"
    // row, which is confusing on a client machine that only runs a few.
    let list = if !config.hospital_code.is_empty() {
        match crate::firestore::FirestoreClient::new_from_config().await {
            Ok(client) => match client.fetch_modules(&config.hospital_code).await {
                Ok(modules) => modules.enabled_service_names(),
                Err(_) => installed_service_names(config),
            },
            Err(_) => installed_service_names(config),
        }
    } else {
        installed_service_names(config)
    };

    if let Ok(mut guard) = ENABLED_SERVICES_CACHE.lock() {
        *guard = Some((Instant::now(), list.clone()));
    }
    list
}

// ── Constants ────────────────────────────────────────────────────────────────

/// Service port mapping for native mode (same ports as Docker)
const SERVICE_PORTS: &[(&str, u16)] = &[
    ("puru-xenon", 8081),
    ("puru-has", 8082),
    ("puru-pacs", 8083),
    ("puru-argon", 8084),
    ("puru-comm", 8085),
    ("puru-realtime", 8086),
    ("puru-neon", 8087),
    ("puru-bridge", 8094),
    ("puru-auth", 8080),
    ("puru-mercury", 8089),
    ("puru-integration", 8088),
    ("puru-counter", 8095),
    // dviewer is not a JAR — it's a static bundle served by the shared nginx.
    // Listed here so the Services page shows :3000 in the port column; the
    // start_service dispatch above short-circuits before any JAR flow reads it.
    ("dviewer", 3000),
];

/// Separate management/actuator port for services that run actuator on a
/// dedicated port (Spring Boot `management.server.port`) instead of the app
/// port. Health/readiness is probed here. Convention: mgmt port = app port +
/// 1000 (8081 → 9081, 8082 → 9082, …). Services NOT listed are assumed to
/// expose actuator on their app port (see SERVICE_PORTS) and fall back to it —
/// if absent there, the probe reports a "actuator not reachable" error.
const SERVICE_MGMT_PORTS: &[(&str, u16)] = &[
    ("puru-auth", 9080),
    ("puru-xenon", 9081),
    ("puru-has", 9082),
    ("puru-pacs", 9083),
    ("puru-argon", 9084),
    ("puru-comm", 9085),
    ("puru-realtime", 9086),
    ("puru-neon", 9087),
    ("puru-integration", 9088),
    ("puru-mercury", 9089),
    ("puru-bridge", 9094),
    ("puru-counter", 9095),
];

/// MySQL database used by each service (same mapping as the Docker compose generator)
const SERVICE_DBS: &[(&str, &str)] = &[
    ("puru-auth", "puru_auth"),
    ("puru-xenon", "puru_im"),
    ("puru-has", "puru_has"),
    ("puru-pacs", "puru_dicom"),
    ("puru-argon", "puru_path"),
    ("puru-comm", "puru_im"),
    ("puru-realtime", "puru_im"),
    ("puru-neon", "puru_med"),
    ("puru-mercury", "puru_im"),
    ("puru-counter", "puru_im"),
    ("puru-bridge", "puru_bridge"),
    ("puru-integration", "puru_im"),
];

/// Aggregate health endpoint for Spring Boot services (liveness-ish).
const HEALTH_ENDPOINT: &str = "/actuator/health";

/// Readiness probe — UP only once the app has *fully* started and is accepting
/// traffic. A crash-looping or never-fully-started service never reaches this,
/// which is exactly what distinguishes "the JVM is alive" from "the app works".
const READINESS_ENDPOINT: &str = "/actuator/health/readiness";

/// An alive process that hasn't reported ready within this many seconds is
/// treated as failing (stuck / crash-looping), not merely "starting".
const STARTUP_GRACE_SECS: u64 = 150;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn pid_path(config: &NucleusConfig, service: &str) -> PathBuf {
    config.native_logs_dir().join(format!("{}.pid", service))
}

fn log_path(config: &NucleusConfig, service: &str) -> PathBuf {
    config.native_logs_dir().join(format!("{}.log", service))
}

/// Full path to the currently-active JAR for `service` as recorded in the
/// per-service manifest. Returns None if the manifest has no active entry
/// (fresh install with nothing pulled, or a service that only ever downloaded
/// but never promoted). Callers that need to know "is a JAR installed?" should
/// use [`is_installed`] instead — it also handles hydrogen/dviewer.
fn active_jar_path(config: &NucleusConfig, service: &str) -> Option<PathBuf> {
    let jars_dir = config.jars_dir();
    let manifest = super::jar_manifest::JarManifest::read(&jars_dir, service);
    manifest.active.map(|e| jars_dir.join(e.file))
}

/// Services with an installed artifact on this box — a JAR present, or the
/// hydrogen/dviewer static bundle deployed. Used as the fallback service list
/// when the hospital's Firestore module selection can't be fetched, so the UI
/// never shows the entire catalog as "Not installed" rows.
fn installed_service_names(config: &NucleusConfig) -> Vec<String> {
    let jars_dir = config.jars_dir();
    let mut names: Vec<String> = releases::all_updatable_services()
        .into_iter()
        .filter(|s| s != "puru-hydrogen" && s != "dviewer")
        .filter(|s| {
            // Installed = manifest has an active entry whose file is on disk,
            // OR (for freshly-downloaded services) a pending entry — either way
            // the service is ready to (be) launched.
            let manifest = super::jar_manifest::JarManifest::read(&jars_dir, s);
            let has_active = manifest
                .active
                .as_ref()
                .map(|e| jars_dir.join(&e.file).exists())
                .unwrap_or(false);
            let has_pending = manifest
                .pending
                .as_ref()
                .map(|e| jars_dir.join(&e.file).exists())
                .unwrap_or(false);
            has_active || has_pending
        })
        .collect();
    if config.nginx_html_dir().join("index.html").exists() {
        names.push("puru-hydrogen".into());
    }
    if config.dviewer_dir().join("index.html").exists() {
        names.push("dviewer".into());
    }
    names
}

fn service_port(service: &str) -> Option<u16> {
    SERVICE_PORTS
        .iter()
        .find(|(s, _)| *s == service)
        .map(|(_, p)| *p)
}

/// Port to probe actuator on: the dedicated management port if the service has
/// one, otherwise the app port.
fn health_probe_port(service: &str) -> Option<u16> {
    SERVICE_MGMT_PORTS
        .iter()
        .find(|(s, _)| *s == service)
        .map(|(_, p)| *p)
        .or_else(|| service_port(service))
}

fn db_for_service(service: &str) -> Option<&'static str> {
    SERVICE_DBS
        .iter()
        .find(|(s, _)| *s == service)
        .map(|(_, d)| *d)
}

/// Whether a service's build is present on disk. For JAR services that means
/// the manifest has an `active` or `pending` entry with the file present on
/// disk; for puru-hydrogen/dviewer it means the static bundle is laid down.
/// Used by the setup re-run to decide what needs pulling.
pub(crate) fn is_installed(config: &NucleusConfig, service: &str) -> bool {
    if service == "puru-hydrogen" {
        return config.nginx_html_dir().join("index.html").exists();
    }
    if service == "dviewer" {
        return config.dviewer_dir().join("index.html").exists();
    }
    let jars_dir = config.jars_dir();
    let manifest = super::jar_manifest::JarManifest::read(&jars_dir, service);
    let has = |e: &super::jar_manifest::JarEntry| jars_dir.join(&e.file).exists();
    manifest.active.as_ref().map_or(false, has) || manifest.pending.as_ref().map_or(false, has)
}

/// Whether a JAR service's process is currently alive (by its PID file).
pub(crate) fn is_running(config: &NucleusConfig, service: &str) -> bool {
    read_pid(config, service)
        .map(is_process_alive)
        .unwrap_or(false)
}

/// Poll a TCP port on 127.0.0.1 until nothing answers (= port is free for
/// `bind`), giving up after `max_secs`. Used to gate `start_service` so we
/// don't spawn a JVM that's just going to die with "Port X already in use"
/// because the OS hasn't released the listening socket from a just-killed
/// instance yet. Returns true if the port is free, false on timeout.
async fn wait_for_port_free(port: u16, max_secs: u64) -> bool {
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(max_secs);
    loop {
        let busy = std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            std::time::Duration::from_millis(200),
        )
        .is_ok();
        if !busy {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }
}

/// Wait until a service's actuator reports readiness `UP`, or `max_secs`
/// elapses. Used to gate dependent services behind puru-auth at startup so the
/// rest of the fleet doesn't come up before the token authority is accepting
/// traffic. Returns true if it became ready, false on timeout / unreachable.
pub(crate) async fn wait_for_ready(service: &str, max_secs: u64) -> bool {
    let Some(port) = health_probe_port(service) else {
        return false;
    };
    let url = format!("http://127.0.0.1:{}{}", port, READINESS_ENDPOINT);
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(max_secs);
    loop {
        let (_reachable, up, _ms) = probe_url(&client, &url).await;
        if up == Some(true) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }
}

/// Read PID from pid file, returns None if file doesn't exist or is invalid
fn read_pid(config: &NucleusConfig, service: &str) -> Option<u32> {
    let path = pid_path(config, service);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
}

/// Check if a process with the given PID is alive
fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // kill(pid, 0) checks if process exists without sending a signal
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(windows)]
    {
        // Use tasklist to check if PID exists — no extra dependencies needed.
        // silent_std_cmd suppresses the console window the watchdog would
        // otherwise flash for every service on every health cycle.
        crate::process::silent_std_cmd("tasklist")
            .args(["/FI", &format!("PID eq {}", pid), "/NH"])
            .output()
            .map(|o| {
                let out = String::from_utf8_lossy(&o.stdout);
                // tasklist output contains the PID if the process exists,
                // otherwise it says "no tasks" or "INFO: No tasks"
                out.contains(&pid.to_string()) && !out.contains("No tasks")
            })
            .unwrap_or(false)
    }
}

/// Load environment variables from env files
fn load_env_files(config: &NucleusConfig, service: &str) -> std::collections::HashMap<String, String> {
    let env_dir = config.env_dir();
    let mut vars = std::collections::HashMap::new();

    // Load general.env, database.env, etc. in order
    let env_files = ["general.env", "database.env", "rabbitmq.env"];
    for filename in &env_files {
        let path = env_dir.join(filename);
        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((key, value)) = line.split_once('=') {
                    vars.insert(key.trim().to_string(), value.trim().to_string());
                }
            }
        }
    }

    // database.env carries a shared URL template — `jdbc:mysql://host:{}/{}`.
    // Fill in the port and the per-service database here; each service has its
    // own schema, so a literal shared URL can never be correct for all of them.
    if let Some(url) = vars.get("SPRING_DATASOURCE_URL").cloned() {
        if url.contains("{}") {
            match db_for_service(service) {
                Some(db) => {
                    let port = vars
                        .get("MYSQL_PORT")
                        .cloned()
                        .unwrap_or_else(|| "3306".to_string());
                    let fixed = url.replacen("{}", &port, 1).replacen("{}", db, 1);
                    vars.insert("SPRING_DATASOURCE_URL".to_string(), fixed);
                }
                None => {
                    // Unknown service — a malformed URL is worse than none at all
                    vars.remove("SPRING_DATASOURCE_URL");
                }
            }
        }
    }

    // GCP auth: service JVMs fetch short-lived, scoped access tokens from the
    // local daemon's loopback token endpoint. The SA key (encrypted at rest)
    // never leaves nucleus. We deliberately do NOT set
    // GOOGLE_APPLICATION_CREDENTIALS or SPRING_CLOUD_GCP_CREDENTIALS_LOCATION.
    //
    // The former `GCP_AUTH_APPROACH=2` toggle is gone — puru-pacs and
    // puru-realtime hard-switched to broker-only, so a switch value would be
    // ignored downstream and setting it is just misleading dead config.
    let (daemon_port, daemon_key) = config
        .daemon
        .as_ref()
        .map(|d| (d.port, d.api_key.clone()))
        .unwrap_or((9090, String::new()));
    vars.insert(
        "PURU_TOKEN_ENDPOINT".to_string(),
        format!("http://127.0.0.1:{}/api/gcp/token", daemon_port),
    );
    vars.insert("PURU_TOKEN_KEY".to_string(), daemon_key);
    vars.insert("PURU_SERVICE".to_string(), service.to_string());

    // Schema bootstrap. The services' bundled application.properties ship with
    // `spring.jpa.hibernate.ddl-auto` COMMENTED OUT, so on a database with the
    // MySQL dialect Spring Boot defaults to `none` — Hibernate never creates the
    // schema. A fresh install therefore starts every service against an empty
    // database and it crashes at boot (e.g. auth's SecurityAuditorAware queries
    // `puru_user` before any table exists). We inject `ddl-auto=update` here so
    // each service creates/upgrades its own schema on boot; the seed step then
    // fills in config/ref_data rows. `update` only adds tables/columns, never
    // drops, so it is safe to leave on for the lifetime of the deployment.
    // (SPRING_JPA_HIBERNATE_DDL_AUTO → spring.jpa.hibernate.ddl-auto via Spring
    // relaxed binding; a value already present in an env file still wins because
    // env-file vars are inserted above and we only set it when absent.)
    vars.entry("SPRING_JPA_HIBERNATE_DDL_AUTO".to_string())
        .or_insert_with(|| "update".to_string());

    vars
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Start a service as a java -jar process.
pub async fn start_service(name: &str, config: &NucleusConfig) -> Result<(), NucleusError> {
    // puru-hydrogen is the Angular UI: not a JAR, but static files served by a
    // managed nginx. Starting it means provisioning + (re)starting nginx.
    if name == "puru-hydrogen" {
        return crate::webserver::start(config).await;
    }
    // dviewer rides on the same nginx as hydrogen (extra server block on :3000).
    // Bringing it up = ensure nginx is up; the block is emitted unconditionally.
    if name == "dviewer" {
        return crate::webserver::start(config).await;
    }

    // Check if already running
    if let Some(pid) = read_pid(config, name) {
        if is_process_alive(pid) {
            return Err(NucleusError::Validation(format!(
                "Service {} is already running (PID {})",
                name, pid
            )));
        }
        // Clean up stale PID file from a dead process
        let _ = std::fs::remove_file(pid_path(config, name));
    }

    // Self-heal the RabbitMQ app user before launching. A broker reset (the
    // Khepri virgin-node behaviour on a 4.x upgrade) silently drops `puru`,
    // which makes every service abort on boot with ACCESS_REFUSED. Best-effort:
    // if the management API is off we just log and proceed.
    if let Err(e) = crate::infra::ensure_rabbitmq_user().await {
        tracing::warn!("RabbitMQ user self-heal skipped for {}: {}", name, e);
    }

    // Resolve java binary path
    let java_version = releases::java_version_for_service(name).ok_or_else(|| {
        NucleusError::Validation(format!("Unknown service: {}", name))
    })?;

    let jre_dir = config.jres_dir().join(format!("temurin-{}", java_version));
    let java_bin = if cfg!(windows) {
        jre_dir.join("bin").join("java.exe")
    } else {
        jre_dir.join("bin").join("java")
    };

    if !java_bin.exists() {
        return Err(NucleusError::NotFound(format!(
            "JRE {} not found. Run `puru pull-jars` first.",
            java_version
        )));
    }

    // Reconcile the manifest with disk before we launch. If an update was
    // interrupted mid-apply (nucleus crash, power cut), this promotes the
    // pending JAR to active so the operator's intent is honoured on this
    // start. Also validates the recorded active file actually exists,
    // falling back to history if not. See `JarManifest::recover_on_start`.
    let jars_dir = config.jars_dir();
    let mut manifest = super::jar_manifest::JarManifest::read(&jars_dir, name);
    let active_entry = manifest.recover_on_start(&jars_dir).await.map_err(|e| {
        NucleusError::NotFound(format!(
            "No JAR available to start {}: {}. Run `puru pull-jars` first.",
            name, e
        ))
    })?;
    let jar = jars_dir.join(&active_entry.file);
    if !jar.exists() {
        // recover_on_start already tried to fall back; if we're still pointing
        // at a missing file, disk state is broken.
        return Err(NucleusError::NotFound(format!(
            "Active JAR for {} at {} is missing on disk. Run `puru pull-jars` to redownload.",
            name,
            jar.display()
        )));
    }

    // Force a single instance: if a previous or duplicate JVM is still holding
    // this service's JAR (a crash left a zombie, or a double-start), kill it
    // before launching so we never end up with two JVMs mapping one jar — the
    // exact condition that later locks the file during an update swap.
    // Best-effort: a failure here should not block the start.
    match crate::file_lock::free_file(&jar, 8).await {
        Ok(n) if n > 0 => {
            tracing::warn!("start {}: cleared {} stale JAR holder(s) first", name, n)
        }
        Ok(_) => {}
        Err(e) => tracing::warn!("start {}: could not fully clear JAR holders: {}", name, e),
    }

    // Ensure logs directory exists
    let logs_dir = config.native_logs_dir();
    tokio::fs::create_dir_all(&logs_dir).await?;

    // Make sure the app port has actually been released by the previous
    // instance (or any other holder). Without this, a quick restart on
    // Windows often races the kernel: we taskkill /F the old JVM, spawn the
    // new one within ~1s, and the bind fails with "Port X already in use"
    // before the OS has released the listening socket. 15s is enough to
    // cover the normal release window AND to surface a clear error if a
    // rogue process is holding the port.
    if let Some(port) = service_port(name) {
        if !wait_for_port_free(port, 15).await {
            return Err(NucleusError::Validation(format!(
                "Port {} is still in use after 15s — kill the holder and retry",
                port
            )));
        }
    }

    // Open log file (append mode)
    let log_file_path = log_path(config, name);
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file_path)?;
    let stderr_file = log_file.try_clone()?;

    // Load env vars
    let env_vars = load_env_files(config, name);

    // JVM options from the memory plan. These must precede `-jar`: everything
    // after it is an argument to the application, not to the VM. An empty vec
    // (tuning switched off) reproduces the old bare `java -jar` launch exactly.
    let vm_args = crate::performance::jvm_args(name, config);
    if !vm_args.is_empty() {
        tracing::info!("Starting {} with JVM options: {}", name, vm_args.join(" "));
    }

    // Spawn process from the config dir — services resolve relative paths
    // (e.g. credential files) against their working directory
    let child = crate::process::silent_cmd(&java_bin)
        .args(&vm_args)
        .args(["-jar", &jar.to_string_lossy()])
        .envs(env_vars)
        .current_dir(crate::config::config_dir())
        .stdout(log_file)
        .stderr(stderr_file)
        .spawn()
        .map_err(|e| NucleusError::Internal(format!("Failed to start {}: {}", name, e)))?;

    // Write PID file
    if let Some(pid) = child.id() {
        // Bind the JVM to our lifetime before anything else. If puru-dc is
        // force-killed or quarantined, this JVM goes with it instead of
        // lingering as an orphan that holds the port and answers health checks
        // with nothing supervising it.
        crate::process::tie_to_supervisor_lifetime(pid);

        let pid_file = pid_path(config, name);
        std::fs::write(&pid_file, pid.to_string())?;
        tracing::info!("Started {} (PID {})", name, pid);
    }

    Ok(())
}

/// Stop a running service. SIGTERM → 30s wait → SIGKILL.
pub async fn stop_service(name: &str, config: &NucleusConfig) -> Result<(), NucleusError> {
    if name == "puru-hydrogen" {
        return crate::webserver::stop(config).await;
    }
    // dviewer shares nginx with hydrogen; stopping dviewer independently would
    // require killing the shared web tier. Treat as a no-op — operators bring
    // the web tier down by stopping puru-hydrogen.
    if name == "dviewer" {
        tracing::info!("dviewer is served by the shared nginx (with puru-hydrogen); stop puru-hydrogen to bring down the web tier");
        return Ok(());
    }

    let pid = read_pid(config, name).ok_or_else(|| {
        NucleusError::NotFound(format!("No PID file for {}. Is it running?", name))
    })?;

    if !is_process_alive(pid) {
        // Clean up stale PID file
        let _ = std::fs::remove_file(pid_path(config, name));
        return Ok(());
    }

    tracing::info!("Stopping {} (PID {})...", name, pid);

    #[cfg(unix)]
    {
        // Send SIGTERM for graceful shutdown
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }

        // Wait up to 30 seconds for graceful shutdown
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(30);
        loop {
            if !is_process_alive(pid) {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                tracing::warn!("{} did not stop within 30s, sending SIGKILL", name);
                unsafe {
                    libc::kill(pid as i32, libc::SIGKILL);
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
    }

    #[cfg(windows)]
    {
        // Headless `java.exe` (spawned with CREATE_NO_WINDOW) has no console
        // window, so `taskkill` without `/F` would dispatch WM_CLOSE to a
        // nonexistent receiver — a no-op that just wastes the grace window.
        // Force-kill the JVM and its child processes directly. `start_service`
        // polls the app port before spawning, so any kernel-held socket from
        // the freshly killed JVM is waited out there, not here.
        let _ = crate::process::silent_cmd("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .output()
            .await;
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    }

    // Remove PID file
    let _ = std::fs::remove_file(pid_path(config, name));
    tracing::info!("Stopped {}", name);
    Ok(())
}

/// Restart = stop + start.
pub async fn restart_service(name: &str, config: &NucleusConfig) -> Result<(), NucleusError> {
    // Stop if running (ignore errors — might not be running)
    let _ = stop_service(name, config).await;
    // Brief pause between stop and start
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    start_service(name, config).await
}

/// List all services with status (running/stopped), PID, port, health.
pub async fn get_services(config: &NucleusConfig) -> Result<Vec<ServiceInfo>, NucleusError> {
    // Enabled services come from Firestore, but through a short-lived cache so
    // repeated services-screen loads/refreshes don't pay a token exchange +
    // query every time (that round-trip was the bulk of the load latency).
    let enabled_services = enabled_services_cached(config).await;

    let mut services = Vec::new();

    // Collect probe targets for concurrent health checks
    let mut probe_targets: Vec<(String, u16)> = Vec::new();

    for svc_name in &enabled_services {
        let pid = read_pid(config, svc_name);
        let alive = pid.map(is_process_alive).unwrap_or(false);

        // puru-hydrogen is static files served by an external web server,
        // not a JAR process — judge it by deployment + whether port 80 answers
        let status = if svc_name == "puru-hydrogen" {
            let deployed = config.nginx_html_dir().join("index.html").exists();
            let served = std::net::TcpStream::connect_timeout(
                &std::net::SocketAddr::from(([127, 0, 0, 1], 80)),
                std::time::Duration::from_millis(500),
            )
            .is_ok();
            if deployed && served {
                ServiceStatus::Running
            } else if deployed {
                ServiceStatus::Stopped // deployed but no web server on :80
            } else {
                ServiceStatus::NotInstalled // enabled but not deployed
            }
        } else if svc_name == "dviewer" {
            // dviewer is the OHIF static bundle on :3000, served by the same
            // bundled nginx as puru-hydrogen.
            let deployed = config.dviewer_dir().join("index.html").exists();
            let served = std::net::TcpStream::connect_timeout(
                &std::net::SocketAddr::from(([127, 0, 0, 1], crate::webserver::DVIEWER_PORT)),
                std::time::Duration::from_millis(500),
            )
            .is_ok();
            if deployed && served {
                ServiceStatus::Running
            } else if deployed {
                ServiceStatus::Stopped
            } else {
                ServiceStatus::NotInstalled
            }
        } else if alive {
            ServiceStatus::Running
        } else if is_installed(config, svc_name) {
            ServiceStatus::Stopped
        } else {
            ServiceStatus::NotInstalled // enabled but JAR not pulled yet
        };

        let port = service_port(svc_name);

        // Health/readiness is probed on the management port if the service uses
        // a dedicated one, else the app port.
        if alive {
            if let Some(p) = health_probe_port(svc_name) {
                probe_targets.push((svc_name.clone(), p));
            }
        }

        let image = if let Ok(Some(meta)) = releases::read_local_jar_meta(svc_name) {
            let artifact = if meta.jar_file.is_empty() {
                "angular"
            } else {
                meta.jar_file.as_str()
            };
            format!("{}@{}", artifact, meta.short_sha)
        } else if svc_name == "puru-hydrogen" {
            "angular".to_string()
        } else if svc_name == "dviewer" {
            "static".to_string()
        } else {
            "not installed".to_string()
        };

        let port_str = port.map(|p| format!("{}:{}", p, p)).unwrap_or_default();

        // Human-readable uptime from the process start time, for the UI.
        let uptime = if alive {
            pid.and_then(process_uptime_secs).map(fmt_uptime)
        } else {
            None
        };

        services.push(ServiceInfo {
            name: svc_name.clone(),
            container_name: pid.map(|p| format!("PID:{}", p)).unwrap_or_default(),
            image,
            status,
            health: None,
            ports: vec![port_str],
            uptime,
            health_response_ms: None,
            detail: None,
            infra: false,
        });
    }

    // Run HTTP health/readiness probes concurrently, then derive the *displayed*
    // status from the verdict — not from PID liveness alone. A live JVM that
    // isn't ready (crash-looping, failing to start beans, dependency down) must
    // not read as a green "Running".
    if !probe_targets.is_empty() {
        let probe_results = probe_native_health(&probe_targets).await;
        for svc in &mut services {
            // Only services we actually probed (alive + known port). Leave
            // hydrogen and port-less services on their PID-based status.
            let Some(outcome) = probe_results.get(&svc.name) else { continue };
            if svc.status != ServiceStatus::Running {
                continue;
            }

            svc.health_response_ms = if outcome.response_ms > 0 {
                Some(outcome.response_ms)
            } else {
                None
            };

            // Still inside the startup grace window? (compute before mutating svc)
            let booting = read_pid(config, &svc.name)
                .and_then(process_uptime_secs)
                .map_or(false, |u| u < STARTUP_GRACE_SECS);
            let probe_port = health_probe_port(&svc.name).unwrap_or(0);

            match outcome.up {
                // Ready/UP — genuinely healthy.
                Some(true) => {
                    svc.health = Some(HealthStatus::Healthy);
                    svc.status = ServiceStatus::Running;
                    svc.detail = None;
                }
                // Answered but DOWN / not-ready — degraded. Within the startup
                // grace we call it Starting; past it, it's a real failure.
                Some(false) => {
                    svc.health = Some(HealthStatus::Unhealthy);
                    if booting {
                        svc.status = ServiceStatus::Starting;
                        svc.detail = Some("Started, not ready yet".into());
                    } else {
                        svc.status = ServiceStatus::Error;
                        svc.detail = Some(format!(
                            "Not ready — actuator on :{} reports DOWN. Check the service log.",
                            probe_port
                        ));
                    }
                }
                // No readable verdict — the actuator gave us nothing usable. This
                // means actuator is disabled/secured/absent (reachable) or the
                // management port isn't listening (unreachable). Surface it as an
                // error after the grace window instead of a misleading "Running".
                None => {
                    svc.health = None;
                    if booting {
                        svc.status = ServiceStatus::Starting;
                        svc.detail = Some("Starting — actuator not responding yet".into());
                    } else if outcome.reachable {
                        svc.status = ServiceStatus::Error;
                        svc.detail = Some(format!(
                            "Actuator not readable on :{} — enable and publicly expose \
                             /actuator/health/readiness (management endpoint appears disabled or secured).",
                            probe_port
                        ));
                    } else {
                        svc.status = ServiceStatus::Error;
                        svc.detail = Some(format!(
                            "Actuator unreachable on :{} — is the management endpoint enabled, \
                             or its port mapped in Nucleus?",
                            probe_port
                        ));
                    }
                }
            }
        }
    }

    // Prepend infra status rows (Database + Message Broker) so operators see
    // MySQL / RabbitMQ health alongside the app services.
    let mut all = crate::infra::infra_rows(config).await;
    all.append(&mut services);

    Ok(all)
}

/// Read log file tail for a service.
pub async fn get_logs(
    name: &str,
    tail: u64,
    config: &NucleusConfig,
) -> Result<String, NucleusError> {
    let path = log_path(config, name);

    if !path.exists() {
        return Ok(format!("No log file found for {}", name));
    }

    // Read the file and return last N lines. Decode lossily — service stdout on
    // Windows/Java is often not valid UTF-8, and a strict read would fail the
    // whole log view with "stream did not contain valid UTF-8".
    let bytes = tokio::fs::read(&path).await?;
    let content = String::from_utf8_lossy(&bytes).into_owned();
    let lines: Vec<&str> = content.lines().collect();
    let start = if lines.len() > tail as usize {
        lines.len() - tail as usize
    } else {
        0
    };

    Ok(lines[start..].join("\n"))
}

/// Update a service in one shot: download the new JAR → stop old → start new.
/// The download step records the new JAR as `pending` in the manifest before
/// touching any process, so a crash between download and restart still leaves
/// disk state consistent — the next `start_service` promotes it automatically.
pub async fn update_service(name: &str, config: &NucleusConfig) -> Result<releases::JarPullResult, NucleusError> {
    update_service_progress(name, config, |_, _, _| {}).await
}

/// Like [`update_service`], but reports progress through `on(phase, downloaded,
/// total)` so the UI can show a proper progress bar. `phase` is one of
/// `"downloading"`, `"stopping"`, `"starting"`; `downloaded`/`total` are bytes
/// during the download phase (0 otherwise).
///
/// Ordering: **download first, then swap.** The download writes to a fresh
/// timestamped file and updates the manifest with `pending_jar` BEFORE any
/// process is stopped. That way, if the runtime hand-off fails at any point
/// (kill hangs, restart fails, machine loses power), the operator's intent to
/// upgrade is durable — the next `start_service` picks up the pending entry.
pub async fn update_service_progress<F>(
    name: &str,
    config: &NucleusConfig,
    on: F,
) -> Result<releases::JarPullResult, NucleusError>
where
    F: Fn(&str, u64, u64) + Send + Sync,
{
    tracing::info!("Updating {} ...", name);

    // Phase 1 (safe, no process impact): download to a new timestamped file
    // and record it as pending in the manifest.
    on("downloading", 0, 0);
    let pull_result =
        releases::pull_jar_progress(name, |d, t| on("downloading", d, t)).await?;

    // Phase 2 (runtime hand-off, best-effort): stop old, free its lock, start
    // new. start_service's recover_on_start does the manifest promotion. If any
    // of these steps fails, the pending manifest entry survives — the next
    // start-up promotes.
    let was_running = read_pid(config, name)
        .map(is_process_alive)
        .unwrap_or(false);

    if was_running {
        on("stopping", 0, 0);
        stop_service(name, config).await?;
        // Free the now-superseded JAR so any zombie holder is cleared. This is
        // best-effort: even if it times out the old JAR is just orphaned, not
        // an obstacle — the new JAR lives under a different filename.
        if let Some(old_active) = active_jar_path(config, name) {
            if old_active.exists() {
                if let Err(e) = crate::file_lock::free_file(&old_active, 20).await {
                    tracing::warn!(
                        "update {}: could not fully clear old JAR holders ({}); \
                         leaving as orphan for GC",
                        name,
                        e
                    );
                }
            }
        }
        on("starting", 0, 0);
        start_service(name, config).await?;
    }

    // GC old versions once we're on the new one — keeps disk from growing
    // unboundedly across many updates. Non-fatal if it fails.
    prune_old_jars(name, config).await;

    tracing::info!("Updated {} to build {}", name, pull_result.short_sha);
    Ok(pull_result)
}

/// Apply a previously downloaded update: stop → free old JAR (best-effort) →
/// start new (start_service promotes pending → active via recovery). Reports
/// phase progress via `on(phase, 0, 0)` ("stopping"/"starting"). Returns the
/// build info of the just-promoted JAR.
pub async fn apply_update_progress<F>(
    name: &str,
    config: &NucleusConfig,
    on: F,
) -> Result<releases::JarPullResult, NucleusError>
where
    F: Fn(&str, u64, u64) + Send + Sync,
{
    let staged = releases::staged_update_info(name).ok_or_else(|| {
        NucleusError::Validation(format!(
            "No downloaded update to apply for {}. Download it first.",
            name
        ))
    })?;

    let was_running = read_pid(config, name).map(is_process_alive).unwrap_or(false);

    if was_running {
        on("stopping", 0, 0);
        stop_service(name, config).await?;
    }

    // Fail-safe: kill any process still holding the *old* active JAR so it
    // becomes deletable by GC. Best-effort — the swap itself no longer depends
    // on this because we're not renaming the old file, just leaving the new
    // one as the new active.
    if let Some(old_active) = active_jar_path(config, name) {
        if old_active.exists() {
            if let Err(e) = crate::file_lock::free_file(&old_active, 20).await {
                tracing::warn!(
                    "apply {}: could not fully clear old JAR holders ({}); \
                     leaving as orphan for GC",
                    name,
                    e
                );
            }
        }
    }

    // start_service's recover_on_start sees `pending` in the manifest and
    // promotes it → active before spawning the JVM.
    on("starting", 0, 0);
    start_service(name, config).await?;

    // Post-promote GC.
    prune_old_jars(name, config).await;

    // Resolve the now-active entry for the return value.
    let jars_dir = config.jars_dir();
    let manifest = super::jar_manifest::JarManifest::read(&jars_dir, name);
    let active = manifest.active.unwrap_or_else(|| super::jar_manifest::JarEntry {
        file: String::new(),
        short_sha: staged.short_sha.clone(),
        built_at: staged.built_at.clone(),
        java_version: String::new(),
        size_mb: staged.size_mb,
        downloaded_at: String::new(),
    });

    tracing::info!("Applied update for {} → build {}", name, active.short_sha);
    Ok(releases::JarPullResult {
        service: name.to_string(),
        success: true,
        local_path: jars_dir.join(&active.file).to_string_lossy().to_string(),
        short_sha: active.short_sha,
        size_mb: active.size_mb,
        message: "Applied".to_string(),
    })
}

/// Rollback to the immediately-previous JAR in the manifest history. Returns
/// an error if there is no history. Under the manifest scheme this is a pure
/// pointer swap: no file rename, no chance of a "JAR is busy" failure. The
/// caller can then rollback again (further back in history) if needed.
pub async fn rollback_service(name: &str, config: &NucleusConfig) -> Result<(), NucleusError> {
    let jars_dir = config.jars_dir();
    let mut manifest = super::jar_manifest::JarManifest::read(&jars_dir, name);

    if !manifest.rollback_one() {
        return Err(NucleusError::NotFound(format!(
            "No previous JAR recorded for {}. Cannot rollback.",
            name
        )));
    }
    manifest.write_atomic(&jars_dir).await?;

    let was_running = read_pid(config, name).map(is_process_alive).unwrap_or(false);
    if was_running {
        stop_service(name, config).await?;
        start_service(name, config).await?;
    }

    tracing::info!(
        "Rolled back {} to previous JAR ({})",
        name,
        manifest.active.as_ref().map(|e| e.file.as_str()).unwrap_or("?")
    );
    Ok(())
}

/// Rollback to a specific JAR file (must appear in the manifest's history).
/// Used by the UI's "rollback to version N" picker. Same pointer-swap
/// mechanics as [`rollback_service`], different target.
pub async fn rollback_service_to(
    name: &str,
    file: &str,
    config: &NucleusConfig,
) -> Result<(), NucleusError> {
    let jars_dir = config.jars_dir();
    let mut manifest = super::jar_manifest::JarManifest::read(&jars_dir, name);

    if !manifest.rollback_to(file) {
        return Err(NucleusError::NotFound(format!(
            "No history entry '{}' for {}.",
            file, name
        )));
    }
    manifest.write_atomic(&jars_dir).await?;

    let was_running = read_pid(config, name).map(is_process_alive).unwrap_or(false);
    if was_running {
        stop_service(name, config).await?;
        start_service(name, config).await?;
    }

    tracing::info!("Rolled back {} to {}", name, file);
    Ok(())
}

/// Best-effort GC: trim per-service JAR history to `config.jar_history_keep`
/// and delete any versioned JARs (and their meta sidecars) that fall outside
/// the retention window. Never fails the caller.
async fn prune_old_jars(name: &str, config: &NucleusConfig) {
    let jars_dir = config.jars_dir();
    let mut manifest = super::jar_manifest::JarManifest::read(&jars_dir, name);
    match manifest.gc(&jars_dir, config.jar_history_keep).await {
        Ok(n) if n > 0 => tracing::debug!("gc {}: removed {} old JAR(s)", name, n),
        Ok(_) => {}
        Err(e) => tracing::warn!("gc {}: failed: {}", name, e),
    }
    // Also delete orphan `.meta.json` sidecars whose JAR is gone.
    let prefix = format!("{}_", name);
    if let Ok(mut rd) = tokio::fs::read_dir(&jars_dir).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            let n = entry.file_name().to_string_lossy().to_string();
            if !n.starts_with(&prefix) || !n.ends_with(".jar.meta.json") {
                continue;
            }
            let jar_name = n.trim_end_matches(".meta.json").to_string();
            if !jars_dir.join(&jar_name).exists() {
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
    }
}

/// Tear a service down: stop the process (if running) and delete its on-disk
/// artifacts (all versioned JARs + meta sidecars, manifest, pid file). The
/// .log file is kept for audit. Used when a service has been de-selected in
/// the cloud config.
pub(crate) async fn remove_service(config: &NucleusConfig, name: &str) -> Result<(), NucleusError> {
    // Stop first — ignore "not running" errors, we just want it down.
    let _ = stop_service(name, config).await;

    let jars = config.jars_dir();

    // Delete every file for this service under jars_dir:
    //   - versioned JARs `{name}_...jar` and their `.meta.json` sidecars
    //   - the manifest `{name}.manifest.json`
    //   - legacy `{name}.meta.json` (hydrogen/dviewer only, but harmless to try)
    let prefix = format!("{}_", name);
    if let Ok(mut rd) = tokio::fs::read_dir(&jars).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            let n = entry.file_name().to_string_lossy().to_string();
            if n.starts_with(&prefix)
                && (n.ends_with(".jar") || n.ends_with(".jar.meta.json"))
            {
                let _ = tokio::fs::remove_file(entry.path()).await;
            }
        }
    }
    let manifest_path = super::jar_manifest::JarManifest::path(&jars, name);
    if manifest_path.exists() {
        if let Err(e) = tokio::fs::remove_file(&manifest_path).await {
            tracing::warn!("Failed to remove {}: {}", manifest_path.display(), e);
        }
    }
    let legacy_meta = jars.join(format!("{}.meta.json", name));
    if legacy_meta.exists() {
        let _ = tokio::fs::remove_file(&legacy_meta).await;
    }
    let pid = pid_path(config, name);
    if pid.exists() {
        let _ = tokio::fs::remove_file(&pid).await;
    }

    tracing::info!("Removed native service {}", name);
    Ok(())
}

// ── Health probing ──────────────────────────────────────────────────────────

/// Outcome of probing a service's actuator endpoints.
struct ProbeOutcome {
    /// The port answered at all (the web server is up).
    reachable: bool,
    /// Verdict from the actuator status: `Some(true)` = UP/ready,
    /// `Some(false)` = answered but DOWN/not-ready, `None` = answered but the
    /// status couldn't be read (endpoint secured/absent) or no answer at all.
    up: Option<bool>,
    response_ms: u64,
}

/// Process uptime in seconds (epoch now − process start time), or None if the
/// process can't be found. Used to tell "still booting" from "stuck".
/// Format an uptime in seconds as a short human string ("3d 4h", "5h 12m",
/// "8m", "42s") for the Services page.
fn fmt_uptime(secs: u64) -> String {
    let d = secs / 86_400;
    let h = (secs % 86_400) / 3_600;
    let m = (secs % 3_600) / 60;
    if d > 0 {
        format!("{}d {}h", d, h)
    } else if h > 0 {
        format!("{}h {}m", h, m)
    } else if m > 0 {
        format!("{}m", m)
    } else {
        format!("{}s", secs)
    }
}

fn process_uptime_secs(pid: u32) -> Option<u64> {
    use sysinfo::{Pid, PidExt, ProcessExt, System, SystemExt};
    let mut sys = System::new();
    let spid = Pid::from_u32(pid);
    sys.refresh_process(spid);
    let proc_ = sys.process(spid)?;
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    Some(now.saturating_sub(proc_.start_time()))
}

/// GET a URL and interpret the actuator response into (reachable, up?, ms).
async fn probe_url(client: &reqwest::Client, url: &str) -> (bool, Option<bool>, u64) {
    use std::time::Instant;
    let start = Instant::now();
    match client.get(url).send().await {
        Ok(resp) => {
            let ms = start.elapsed().as_millis() as u64;
            let code = resp.status();
            let body = resp.text().await.unwrap_or_default();
            // Actuator JSON status is authoritative (a 503 with {"status":"DOWN"}
            // is a real verdict, as is {"status":"UP"}).
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(s) = json.get("status").and_then(|v| v.as_str()) {
                    return (true, Some(s == "UP"), ms);
                }
            }
            if code.is_success() {
                (true, Some(true), ms)
            } else if matches!(code.as_u16(), 401 | 403 | 404 | 405) {
                // Answered, but the endpoint is secured/absent — can't read a verdict.
                (true, None, ms)
            } else {
                (true, Some(false), ms)
            }
        }
        Err(_) => (false, None, 0),
    }
}

async fn probe_native_health(
    targets: &[(String, u16)],
) -> std::collections::HashMap<String, ProbeOutcome> {
    use futures_util::future::join_all;

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return std::collections::HashMap::new(),
    };

    let futures: Vec<_> = targets
        .iter()
        .map(|(name, port)| {
            let name = name.clone();
            let port = *port;
            let client = client.clone();
            async move {
                // Prefer readiness — it only reports UP once fully started.
                let ready_url = format!("http://127.0.0.1:{}{}", port, READINESS_ENDPOINT);
                let (r1, up1, ms1) = probe_url(&client, &ready_url).await;
                let outcome = if up1.is_some() {
                    ProbeOutcome { reachable: r1, up: up1, response_ms: ms1 }
                } else {
                    // Readiness missing/secured/unreachable → fall back to the
                    // aggregate health endpoint so services that haven't adopted
                    // readiness probes yet behave as before (no regression).
                    let health_url = format!("http://127.0.0.1:{}{}", port, HEALTH_ENDPOINT);
                    let (r2, up2, ms2) = probe_url(&client, &health_url).await;
                    ProbeOutcome {
                        reachable: r1 || r2,
                        up: up2,
                        response_ms: if r2 { ms2 } else { ms1 },
                    }
                };
                (name, outcome)
            }
        })
        .collect();

    join_all(futures).await.into_iter().collect()
}
