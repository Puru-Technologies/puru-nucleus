//! Cross-platform installer for MySQL, Erlang, and RabbitMQ.
//!
//! - **Windows**: Downloads MSI/EXE installers, runs silent installs, sets PATH
//! - **macOS**: Uses Homebrew (`brew install`)
//! - **Linux**: Uses apt (`sudo apt-get install`)
//!
//! Emits Tauri events for progress reporting.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallProgress {
    pub software: String,
    pub stage: InstallStage,
    pub percent: u8,
    pub message: String,
    pub bytes_downloaded: u64,
    pub bytes_total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallStage {
    Downloading,
    Installing,
    Verifying,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallResult {
    pub software: String,
    pub success: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

// ── Windows constants ───────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
const MYSQL_BASE: &str = r"C:\Program Files\MySQL";
/// Version-agnostic target for a fresh ZIP extraction when no MySQL is present.
#[cfg(target_os = "windows")]
const MYSQL_DEFAULT_DIR: &str = r"C:\Program Files\MySQL\MySQL Server";
#[cfg(target_os = "windows")]
const ERLANG_INSTALL_DIR: &str = r"C:\Program Files\Erlang OTP";

// Legacy GitHub/vendor fallback URLs — retained for reference now that infra
// downloads resolve through oxygen's infra manifest.
#[cfg(target_os = "windows")]
#[allow(dead_code)]
const MYSQL_FALLBACK_URL: &str = "https://dev.mysql.com/get/Downloads/MySQL-8.0/mysql-8.0.40-winx64.msi";
#[allow(dead_code)]
const ERLANG_FALLBACK_URL: &str = "https://github.com/erlang/otp/releases/download/OTP-26.2.5.6/otp_win64_26.2.5.6.exe";
#[allow(dead_code)]
const RABBITMQ_FALLBACK_URL: &str = "https://github.com/rabbitmq/rabbitmq-server/releases/download/v3.13.7/rabbitmq-server-3.13.7.exe";

// ── Public API ──────────────────────────────────────────────────────────────

/// Install missing prerequisites. Routes to platform-specific implementation.
pub async fn install_missing(
    app: &tauri::AppHandle,
    software: &[String],
) -> Vec<InstallResult> {
    let install_mysql = software.iter().any(|s| s.eq_ignore_ascii_case("mysql"));
    let install_rabbitmq = software.iter().any(|s| s.eq_ignore_ascii_case("rabbitmq"));

    #[cfg(target_os = "windows")]
    {
        install_missing_windows(app, install_mysql, install_rabbitmq).await
    }
    #[cfg(not(target_os = "windows"))]
    {
        install_missing_unix(app, install_mysql, install_rabbitmq).await
    }
}

/// macOS/Linux install path (Homebrew / apt).
#[cfg(not(target_os = "windows"))]
async fn install_missing_unix(
    app: &tauri::AppHandle,
    install_mysql: bool,
    install_rabbitmq: bool,
) -> Vec<InstallResult> {
    let mut results = Vec::new();

    if install_mysql {
        results.push(do_install_mysql(app).await);
    }
    if install_rabbitmq {
        // Erlang first (dependency)
        let erlang_result = do_install_erlang(app).await;
        let erlang_ok = erlang_result.success;
        results.push(erlang_result);
        if erlang_ok {
            results.push(do_install_rabbitmq(app).await);
        } else {
            results.push(InstallResult {
                software: "RabbitMQ".into(),
                success: false,
                version: None,
                error: Some("Skipped — Erlang installation failed".into()),
            });
        }
    }

    results
}

// ══════════════════════════════════════════════════════════════════════════════
//  macOS — Homebrew
// ══════════════════════════════════════════════════════════════════════════════

#[cfg(target_os = "macos")]
async fn do_install_mysql(app: &tauri::AppHandle) -> InstallResult {
    brew_install(app, "MySQL", "mysql").await
}

#[cfg(target_os = "macos")]
async fn do_install_erlang(app: &tauri::AppHandle) -> InstallResult {
    brew_install(app, "Erlang", "erlang").await
}

#[cfg(target_os = "macos")]
async fn do_install_rabbitmq(app: &tauri::AppHandle) -> InstallResult {
    let result = brew_install(app, "RabbitMQ", "rabbitmq").await;
    if result.success {
        // Start RabbitMQ service and enable management plugin
        let _ = tokio::process::Command::new("brew")
            .args(["services", "start", "rabbitmq"])
            .output()
            .await;
        let _ = tokio::process::Command::new("rabbitmq-plugins")
            .args(["enable", "rabbitmq_management"])
            .output()
            .await;
    }
    result
}

#[cfg(target_os = "macos")]
async fn brew_install(app: &tauri::AppHandle, display_name: &str, formula: &str) -> InstallResult {
    // Check if Homebrew is available
    if tokio::process::Command::new("brew").arg("--version").output().await.is_err() {
        emit_progress(app, display_name, InstallStage::Failed, 0,
            "Homebrew not found. Install from https://brew.sh", 0, 0);
        return InstallResult {
            software: display_name.into(),
            success: false,
            version: None,
            error: Some("Homebrew is not installed. Install from https://brew.sh".into()),
        };
    }

    emit_progress(app, display_name, InstallStage::Installing, 0,
        &format!("Running: brew install {}...", formula), 0, 0);

    let output = tokio::process::Command::new("brew")
        .args(["install", formula])
        .output()
        .await;

    match output {
        Ok(out) if out.status.success() => {
            // Verify
            emit_progress(app, display_name, InstallStage::Verifying, 0, "Verifying...", 0, 0);
            let version = verify_install_unix(display_name).await;
            let success = version.is_some();
            let msg = if success {
                format!("Installed ({})", version.as_deref().unwrap_or("ok"))
            } else {
                "Installed but could not verify version".into()
            };
            emit_progress(app, display_name,
                if success { InstallStage::Completed } else { InstallStage::Completed },
                100, &msg, 0, 0);
            InstallResult {
                software: display_name.into(),
                success: true, // brew succeeded even if version check is flaky
                version,
                error: None,
            }
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            emit_progress(app, display_name, InstallStage::Failed, 0, &stderr, 0, 0);
            InstallResult {
                software: display_name.into(),
                success: false,
                version: None,
                error: Some(format!("brew install failed: {}", stderr.lines().last().unwrap_or(&stderr))),
            }
        }
        Err(e) => InstallResult {
            software: display_name.into(),
            success: false,
            version: None,
            error: Some(format!("Failed to run brew: {}", e)),
        },
    }
}

// ══════════════════════════════════════════════════════════════════════════════
//  Linux — apt-get
// ══════════════════════════════════════════════════════════════════════════════

#[cfg(target_os = "linux")]
async fn do_install_mysql(app: &tauri::AppHandle) -> InstallResult {
    apt_install(app, "MySQL", &["mysql-server", "mysql-client"]).await
}

#[cfg(target_os = "linux")]
async fn do_install_erlang(app: &tauri::AppHandle) -> InstallResult {
    apt_install(app, "Erlang", &["erlang"]).await
}

#[cfg(target_os = "linux")]
async fn do_install_rabbitmq(app: &tauri::AppHandle) -> InstallResult {
    let result = apt_install(app, "RabbitMQ", &["rabbitmq-server"]).await;
    if result.success {
        // Enable and start RabbitMQ
        let _ = tokio::process::Command::new("sudo")
            .args(["systemctl", "enable", "rabbitmq-server"])
            .output()
            .await;
        let _ = tokio::process::Command::new("sudo")
            .args(["systemctl", "start", "rabbitmq-server"])
            .output()
            .await;
        // Enable management plugin
        let _ = tokio::process::Command::new("sudo")
            .args(["rabbitmq-plugins", "enable", "rabbitmq_management"])
            .output()
            .await;
    }
    result
}

#[cfg(target_os = "linux")]
async fn apt_install(app: &tauri::AppHandle, display_name: &str, packages: &[&str]) -> InstallResult {
    // Update package list first
    emit_progress(app, display_name, InstallStage::Installing, 0,
        "Updating package list...", 0, 0);

    let update = tokio::process::Command::new("sudo")
        .args(["apt-get", "update", "-y"])
        .output()
        .await;

    if let Ok(out) = &update {
        if !out.status.success() {
            tracing::warn!("apt-get update failed, continuing anyway");
        }
    }

    // Install packages
    let pkg_list = packages.join(" ");
    emit_progress(app, display_name, InstallStage::Installing, 0,
        &format!("Installing {}...", pkg_list), 0, 0);

    let mut args = vec!["apt-get", "install", "-y"];
    args.extend_from_slice(packages);

    let output = tokio::process::Command::new("sudo")
        .args(&args)
        .env("DEBIAN_FRONTEND", "noninteractive")
        .output()
        .await;

    match output {
        Ok(out) if out.status.success() => {
            emit_progress(app, display_name, InstallStage::Verifying, 0, "Verifying...", 0, 0);
            let version = verify_install_unix(display_name).await;
            let msg = format!("Installed ({})", version.as_deref().unwrap_or("ok"));
            emit_progress(app, display_name, InstallStage::Completed, 100, &msg, 0, 0);
            InstallResult {
                software: display_name.into(),
                success: true,
                version,
                error: None,
            }
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            emit_progress(app, display_name, InstallStage::Failed, 0, &stderr, 0, 0);
            InstallResult {
                software: display_name.into(),
                success: false,
                version: None,
                error: Some(format!("apt-get install failed: {}", stderr.lines().last().unwrap_or(&stderr))),
            }
        }
        Err(e) => InstallResult {
            software: display_name.into(),
            success: false,
            version: None,
            error: Some(format!("Failed to run apt-get: {}", e)),
        },
    }
}

// ══════════════════════════════════════════════════════════════════════════════
//  Windows — Download + silent install
// ══════════════════════════════════════════════════════════════════════════════

/// Shared context for a Windows install run: an authenticated GCS client plus
/// the parsed `infra/infra-manifest.json`, fetched once and reused for every
/// component so we don't re-authenticate or re-download the manifest per item.
#[cfg(target_os = "windows")]
struct InfraCtx {
    client: google_cloud_storage::client::Client,
    manifest: crate::releases::InfraManifest,
}

#[cfg(target_os = "windows")]
impl InfraCtx {
    async fn init() -> Result<Self, String> {
        let cred = crate::releases::get_credentials_path().map_err(|e| e.user_message())?;
        let client = crate::releases::create_gcs_client(&cred)
            .await
            .map_err(|e| e.user_message())?;
        let manifest = crate::releases::fetch_infra_manifest(&client)
            .await
            .map_err(|e| e.user_message())?;
        Ok(Self { client, manifest })
    }

    fn resolve(&self, component: &str) -> Result<crate::releases::InfraArtifact, String> {
        crate::releases::resolve_infra_artifact(&self.manifest, component).ok_or_else(|| {
            format!(
                "'{}' is not in the infra manifest for this platform ({})",
                component,
                crate::releases::infra_platform_key()
            )
        })
    }
}

#[cfg(target_os = "windows")]
fn mk_fail(name: &str, err: &str) -> InstallResult {
    InstallResult { software: name.into(), success: false, version: None, error: Some(err.to_string()) }
}

/// Windows install driver: build the infra context once, then install the
/// requested components (Erlang is pulled in as a RabbitMQ dependency).
#[cfg(target_os = "windows")]
async fn install_missing_windows(
    app: &tauri::AppHandle,
    install_mysql: bool,
    install_rabbitmq: bool,
) -> Vec<InstallResult> {
    let mut results = Vec::new();

    let ctx = match InfraCtx::init().await {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("Cannot reach the infra manifest: {}", e);
            if install_mysql {
                emit_progress(app, "MySQL", InstallStage::Failed, 0, &msg, 0, 0);
                results.push(mk_fail("MySQL", &msg));
            }
            if install_rabbitmq {
                emit_progress(app, "RabbitMQ", InstallStage::Failed, 0, &msg, 0, 0);
                results.push(mk_fail("Erlang", &msg));
                results.push(mk_fail("RabbitMQ", &msg));
            }
            return results;
        }
    };

    if install_mysql {
        results.push(do_install_mysql(app, &ctx).await);
    }
    if install_rabbitmq {
        let erlang = do_install_erlang(app, &ctx).await;
        let erlang_ok = erlang.success;
        results.push(erlang);
        if erlang_ok {
            results.push(do_install_rabbitmq(app, &ctx).await);
        } else {
            results.push(mk_fail("RabbitMQ", "Skipped — Erlang installation failed"));
        }
    }

    results
}

/// Resolve + stream-download an infra component to a temp file, emitting
/// throttled `install-progress` events. Returns (temp path, version, filename).
#[cfg(target_os = "windows")]
async fn download_infra_with_progress(
    app: &tauri::AppHandle,
    ctx: &InfraCtx,
    display: &str,
    component: &str,
) -> Result<(PathBuf, String, String), String> {
    let artifact = ctx.resolve(component)?;
    emit_progress(
        app, display, InstallStage::Downloading, 0,
        &format!("Resolved {} (v{})", artifact.file, artifact.version), 0, 0,
    );
    let dest = std::env::temp_dir().join(&artifact.file);
    let last = std::sync::atomic::AtomicU8::new(0);

    crate::releases::download_infra_artifact(&ctx.client, &artifact, &dest, |done, total| {
        let percent = if total > 0 { ((done as f64 / total as f64) * 100.0) as u8 } else { 0 };
        let prev = last.load(std::sync::atomic::Ordering::Relaxed);
        if percent >= prev.saturating_add(2) || (total > 0 && done >= total) {
            last.store(percent, std::sync::atomic::Ordering::Relaxed);
            let md = done as f64 / 1_048_576.0;
            let msg = if total > 0 {
                format!("Downloading… {:.1} / {:.1} MB", md, total as f64 / 1_048_576.0)
            } else {
                format!("Downloading… {:.1} MB", md)
            };
            emit_progress(app, display, InstallStage::Downloading, percent, &msg, done, total);
        }
    })
    .await
    .map_err(|e| e.to_string())?;

    Ok((dest, artifact.version, artifact.file))
}

/// Generic infra install: download → run a silent installer → verify.
#[cfg(target_os = "windows")]
async fn install_infra_component<F>(
    app: &tauri::AppHandle,
    ctx: &InfraCtx,
    display: &str,
    component: &str,
    install_fn: F,
) -> InstallResult
where
    F: FnOnce(&PathBuf) -> Result<(), String>,
{
    let (path, version, _file) =
        match download_infra_with_progress(app, ctx, display, component).await {
            Ok(v) => v,
            Err(e) => {
                emit_progress(app, display, InstallStage::Failed, 0, &e, 0, 0);
                return mk_fail(display, &e);
            }
        };

    emit_progress(app, display, InstallStage::Installing, 0, "Running installer (a UAC prompt may appear)…", 0, 0);
    if let Err(e) = install_fn(&path) {
        emit_progress(app, display, InstallStage::Failed, 0, &e, 0, 0);
        let _ = std::fs::remove_file(&path);
        return mk_fail(display, &e);
    }
    let _ = std::fs::remove_file(&path);

    emit_progress(app, display, InstallStage::Verifying, 0, "Verifying…", 0, 0);
    let verified = verify_install_windows(display);
    let success = verified.is_some();
    emit_progress(
        app, display,
        if success { InstallStage::Completed } else { InstallStage::Failed },
        if success { 100 } else { 0 },
        if success { "Installed successfully" } else { "Installation could not be verified" },
        0, 0,
    );
    InstallResult {
        software: display.into(),
        success,
        version: verified.or(Some(version)),
        error: if success { None } else { Some("Installation could not be verified".into()) },
    }
}

#[cfg(target_os = "windows")]
async fn do_install_erlang(app: &tauri::AppHandle, ctx: &InfraCtx) -> InstallResult {
    install_infra_component(app, ctx, "Erlang", "erlang", install_erlang_silent).await
}

#[cfg(target_os = "windows")]
async fn do_install_rabbitmq(app: &tauri::AppHandle, ctx: &InfraCtx) -> InstallResult {
    let result = install_infra_component(app, ctx, "RabbitMQ", "rabbitmq", install_rabbitmq_silent).await;
    if result.success {
        // Drop the delayed-message-exchange .ez into the plugins dir so the
        // configure step's `rabbitmq-plugins enable` succeeds. Best-effort.
        match install_delayed_exchange_plugin(app, ctx).await {
            Ok(file) => emit_progress(
                app, "RabbitMQ", InstallStage::Completed, 100,
                &format!("Installed + delayed-exchange plugin ({})", file), 0, 0,
            ),
            Err(e) => {
                tracing::warn!("RabbitMQ: delayed-exchange plugin not installed: {}", e);
                emit_progress(
                    app, "RabbitMQ", InstallStage::Completed, 100,
                    &format!("Installed (delayed-exchange plugin skipped — {})", e), 0, 0,
                );
            }
        }
    }
    result
}

/// Download the `rabbitmq_delayed_message_exchange` .ez and place it in the
/// RabbitMQ plugins directory. Enabling happens in `setup_configure_rabbitmq`.
#[cfg(target_os = "windows")]
async fn install_delayed_exchange_plugin(
    app: &tauri::AppHandle,
    ctx: &InfraCtx,
) -> Result<String, String> {
    let artifact = ctx.resolve("rabbitmq-delayed-message-exchange")?;
    let plugins_dir =
        find_rabbitmq_plugins_dir().ok_or_else(|| "RabbitMQ plugins directory not found".to_string())?;
    let dest = plugins_dir.join(&artifact.file);
    emit_progress(app, "RabbitMQ", InstallStage::Installing, 0, &format!("Installing plugin {}", artifact.file), 0, 0);
    crate::releases::download_infra_artifact(&ctx.client, &artifact, &dest, |_, _| {})
        .await
        .map_err(|e| e.to_string())?;
    Ok(artifact.file)
}

#[cfg(target_os = "windows")]
fn find_rabbitmq_plugins_dir() -> Option<PathBuf> {
    let base = std::path::Path::new(r"C:\Program Files\RabbitMQ Server");
    for entry in std::fs::read_dir(base).ok()?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("rabbitmq_server-") {
            let p = base.join(&name).join("plugins");
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    None
}

/// MySQL install: download → set up a running service with a generated root
/// password → persist that password to three sinks (config, Firestore, env).
#[cfg(target_os = "windows")]
async fn do_install_mysql(app: &tauri::AppHandle, ctx: &InfraCtx) -> InstallResult {
    // If MySQL binaries are already on disk (any version), configure that install
    // in place — initialize the data dir, register + start the service, set the
    // root password — without re-downloading. Covers a hand-placed / pre-extracted
    // server such as `MySQL Server 9.7`.
    if let Some(dir) = find_mysql_install_dir() {
        // Case A — already running on 3306: configured and up. Leave it entirely
        // alone; we don't know its root password and must never reset it.
        if tcp_port_open(3306) {
            emit_progress(app, "MySQL", InstallStage::Completed, 100,
                "MySQL is already installed and running", 0, 0);
            return InstallResult {
                software: "MySQL".into(),
                success: true,
                version: verify_install_windows("MySQL"),
                error: None,
            };
        }

        let has_local_data = dir.join("data").exists();
        let existing_service = mysql_service_name();

        // Case B — an initialized data dir or a registered service exists but is
        // stopped: just start it. Do NOT re-initialize or reset the password —
        // that would corrupt an existing (possibly production) data directory.
        if has_local_data || existing_service.is_some() {
            let svc = existing_service.unwrap_or_else(|| "MySQL".to_string());
            emit_progress(app, "MySQL", InstallStage::Installing, 40,
                &format!("Starting existing MySQL service '{}'…", svc), 0, 0);
            let _ = std::process::Command::new("net").args(["start", &svc]).output();
            if wait_tcp(3306, 40) {
                emit_progress(app, "MySQL", InstallStage::Completed, 100,
                    "Started existing MySQL (data + password left untouched)", 0, 0);
                return InstallResult {
                    software: "MySQL".into(),
                    success: true,
                    version: verify_install_windows("MySQL"),
                    error: None,
                };
            }
            let e = format!(
                "MySQL is installed at {} but not reachable on 3306. Start its service ('{}') manually, or set its root password in Settings.",
                dir.display(), svc
            );
            emit_progress(app, "MySQL", InstallStage::Failed, 0, &e, 0, 0);
            return mk_fail("MySQL", &e);
        }

        // Case C — binaries present but never initialized (no data dir, no
        // service, not running): safe to finalize a fresh install + set a new pw.
        emit_progress(app, "MySQL", InstallStage::Installing, 20,
            &format!("MySQL found at {} — initializing service…", dir.display()), 0, 0);
        let root_pw = cloud_or_generated_mysql_password().await;
        if let Err(e) = finalize_mysql_service(&dir, &root_pw) {
            emit_progress(app, "MySQL", InstallStage::Failed, 0, &e, 0, 0);
            return mk_fail("MySQL", &e);
        }
        add_to_system_path(&format!("{}\\bin", dir.display()));

        emit_progress(app, "MySQL", InstallStage::Installing, 90, "Storing root credentials (local + cloud + env)…", 0, 0);
        let sinks = persist_mysql_root_password(&root_pw).await;
        if let Err(e) = &sinks {
            tracing::warn!("MySQL: credential persistence incomplete: {}", e);
        }

        emit_progress(app, "MySQL", InstallStage::Verifying, 0, "Verifying…", 0, 0);
        let verified = verify_install_windows("MySQL");
        let success = verified.is_some();
        let msg = if success {
            match &sinks {
                Ok(n) => format!("Configured MySQL; root password stored in {} location(s)", n),
                Err(_) => "Configured MySQL".to_string(),
            }
        } else {
            "MySQL configured but could not be verified".to_string()
        };
        emit_progress(
            app, "MySQL",
            if success { InstallStage::Completed } else { InstallStage::Failed },
            if success { 100 } else { 0 }, &msg, 0, 0,
        );
        return InstallResult {
            software: "MySQL".into(),
            success,
            version: verified,
            error: if success { None } else { Some("Could not verify MySQL".into()) },
        };
    }

    let (path, version, file) =
        match download_infra_with_progress(app, ctx, "MySQL", "mysql").await {
            Ok(v) => v,
            Err(e) => {
                emit_progress(app, "MySQL", InstallStage::Failed, 0, &e, 0, 0);
                return mk_fail("MySQL", &e);
            }
        };

    emit_progress(app, "MySQL", InstallStage::Installing, 10, "Setting up MySQL server…", 0, 0);
    let root_pw = cloud_or_generated_mysql_password().await;

    let setup = if file.to_lowercase().ends_with(".zip") {
        setup_mysql_from_zip(&path, &root_pw)
    } else {
        setup_mysql_from_msi(&path, &root_pw)
    };
    let _ = std::fs::remove_file(&path);
    if let Err(e) = setup {
        emit_progress(app, "MySQL", InstallStage::Failed, 0, &e, 0, 0);
        return mk_fail("MySQL", &e);
    }

    emit_progress(app, "MySQL", InstallStage::Installing, 90, "Storing root credentials (local + cloud + env)…", 0, 0);
    let sinks = persist_mysql_root_password(&root_pw).await;
    if let Err(e) = &sinks {
        tracing::warn!("MySQL: credential persistence incomplete: {}", e);
    }

    emit_progress(app, "MySQL", InstallStage::Verifying, 0, "Verifying…", 0, 0);
    let verified = verify_install_windows("MySQL");
    let success = verified.is_some();
    let msg = if success {
        match &sinks {
            Ok(n) => format!("Installed; root password stored in {} location(s)", n),
            Err(_) => "Installed".to_string(),
        }
    } else {
        "Installation could not be verified".to_string()
    };
    emit_progress(
        app, "MySQL",
        if success { InstallStage::Completed } else { InstallStage::Failed },
        if success { 100 } else { 0 }, &msg, 0, 0,
    );
    InstallResult {
        software: "MySQL".into(),
        success,
        version: verified.or(Some(version)),
        error: if success { None } else { Some("Installation could not be verified".into()) },
    }
}

/// MSI path: lay down the binaries via msiexec, then configure the service and
/// set the root password (the server MSI does not create a running instance).
#[cfg(target_os = "windows")]
fn setup_mysql_from_msi(msi_path: &PathBuf, root_pw: &str) -> Result<(), String> {
    install_mysql_silent(msi_path)?;
    let dir = find_mysql_install_dir()
        .ok_or_else(|| "MySQL install directory not found after MSI install".to_string())?;
    add_to_system_path(&format!("{}\\bin", dir.display()));
    finalize_mysql_service(&dir, root_pw)
}

/// ZIP path: extract the no-install archive, then initialize + service + set pw.
#[cfg(target_os = "windows")]
fn setup_mysql_from_zip(zip_path: &PathBuf, root_pw: &str) -> Result<(), String> {
    let install_dir = PathBuf::from(MYSQL_DEFAULT_DIR);
    extract_zip_stripped(zip_path, &install_dir)?;
    add_to_system_path(&format!("{}\\bin", install_dir.display()));
    finalize_mysql_service(&install_dir, root_pw)
}

/// Initialize the data dir, (re)install + start the Windows service, and set the
/// root password. Idempotent: skips initialize if the data dir already exists.
#[cfg(target_os = "windows")]
fn finalize_mysql_service(install_dir: &PathBuf, root_pw: &str) -> Result<(), String> {
    let bin = install_dir.join("bin");
    let mysqld = bin.join("mysqld.exe");
    if !mysqld.exists() {
        return Err(format!("mysqld.exe not found under {}", bin.display()));
    }
    let data_dir = install_dir.join("data");

    // my.ini so the service and clients agree on basedir/datadir/port.
    let my_ini = install_dir.join("my.ini");
    let to_ini = |p: &std::path::Path| p.display().to_string().replace('\\', "/");
    let ini = format!(
        "[mysqld]\nbasedir={}\ndatadir={}\nport=3306\n[client]\nport=3306\n",
        to_ini(install_dir), to_ini(&data_dir),
    );
    std::fs::write(&my_ini, ini).map_err(|e| format!("write my.ini: {}", e))?;
    let defaults = format!("--defaults-file={}", my_ini.display());

    if !data_dir.exists() {
        run_ok(&mysqld, &[&defaults, "--initialize-insecure"], "mysqld --initialize-insecure")?;
    }

    // (Re)install and start the service. --remove/start may fail harmlessly.
    let _ = std::process::Command::new(&mysqld).args(["--remove", "MySQL"]).output();
    run_ok(&mysqld, &["--install", "MySQL", &defaults], "mysqld --install")?;
    let _ = std::process::Command::new("net").args(["start", "MySQL"]).output();

    // Wait for the server to actually accept connections (a fixed sleep is too
    // fragile on slow / AV-scanned machines).
    if !wait_tcp(3306, 60) {
        return Err("MySQL service started but nothing is accepting on 3306 after 60s".to_string());
    }

    // Root has an empty password after initialize-insecure — set the real one.
    let mysqladmin = bin.join("mysqladmin.exe");
    run_ok(
        &mysqladmin,
        &["-u", "root", "--host=127.0.0.1", "--port=3306", "password", root_pw],
        "set root password",
    )?;
    Ok(())
}

/// Persist the generated MySQL root password to three sinks: local nucleus.toml,
/// the env file `database.env`, and the Firestore hospital document. Returns the
/// number of sinks written; an Err summarizes any that failed (non-fatal).
#[cfg(target_os = "windows")]
async fn persist_mysql_root_password(root_pw: &str) -> Result<usize, String> {
    let mut sinks = 0usize;
    let mut errors: Vec<String> = Vec::new();

    let mut cfg = crate::config::load_config().map_err(|e| e.user_message())?;
    cfg.mysql_user = "root".to_string();
    cfg.mysql_password = root_pw.to_string();
    let hospital_code = cfg.hospital_code.clone();
    let env_path = cfg.env_dir().join("database.env");

    // Sink 1 — local nucleus.toml
    match crate::config::save_config(&cfg) {
        Ok(_) => sinks += 1,
        Err(e) => errors.push(format!("config: {}", e.user_message())),
    }

    // Sink 2 — env file database.env (update in place if it already exists; when
    // it doesn't, the later env-generation step derives it from config above).
    if env_path.exists() {
        match update_env_value(&env_path, "SPRING_DATASOURCE_PASSWORD", root_pw) {
            Ok(_) => sinks += 1,
            Err(e) => errors.push(format!("database.env: {}", e)),
        }
    }

    // Sink 3 — Firestore hospital document (nested `credentials` map, the value
    // other machines read as the source of truth).
    if !hospital_code.is_empty() {
        match crate::firestore::FirestoreClient::new_from_config().await {
            Ok(client) => match client
                .set_hospital_mysql_password(&hospital_code, root_pw)
                .await
            {
                Ok(_) => sinks += 1,
                Err(e) => errors.push(format!("firestore: {}", e.user_message())),
            },
            Err(e) => errors.push(format!("firestore: {}", e.user_message())),
        }
    }

    if errors.is_empty() {
        Ok(sinks)
    } else {
        Err(format!("{} sink(s) written; issues: {}", sinks, errors.join("; ")))
    }
}

/// Set or replace a `KEY=value` line in an env file, preserving other lines.
#[cfg(target_os = "windows")]
fn update_env_value(path: &PathBuf, key: &str, value: &str) -> Result<(), String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("read: {}", e))?;
    let prefix = format!("{}=", key);
    let mut found = false;
    let mut out = String::new();
    for line in content.lines() {
        if line.trim_start().starts_with(&prefix) {
            out.push_str(&format!("{}={}\n", key, value));
            found = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !found {
        out.push_str(&format!("{}={}\n", key, value));
    }
    std::fs::write(path, out).map_err(|e| format!("write: {}", e))
}

/// Extract a zip, stripping the single top-level vendor folder (e.g.
/// `mysql-8.0.40-winx64/`) so files land directly under `dest`.
#[cfg(target_os = "windows")]
fn extract_zip_stripped(zip_path: &PathBuf, dest: &PathBuf) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| format!("open zip: {}", e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("read zip: {}", e))?;
    std::fs::create_dir_all(dest).map_err(|e| format!("create {}: {}", dest.display(), e))?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("zip entry {}: {}", i, e))?;
        let Some(enclosed) = entry.enclosed_name() else { continue };
        let mut comps = enclosed.components();
        comps.next(); // drop the leading vendor folder
        let rel = comps.as_path().to_path_buf();
        if rel.as_os_str().is_empty() {
            continue;
        }
        let out_path = dest.join(&rel);
        if entry.is_dir() {
            let _ = std::fs::create_dir_all(&out_path);
        } else {
            if let Some(p) = out_path.parent() {
                let _ = std::fs::create_dir_all(p);
            }
            let mut out = std::fs::File::create(&out_path)
                .map_err(|e| format!("create {}: {}", out_path.display(), e))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("extract {}: {}", out_path.display(), e))?;
        }
    }
    Ok(())
}

/// Run a command, returning a rich error on non-zero exit.
#[cfg(target_os = "windows")]
fn run_ok<P: AsRef<std::ffi::OsStr>>(program: P, args: &[&str], label: &str) -> Result<(), String> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{}: could not run: {}", label, e))?;
    if !out.status.success() {
        return Err(format!(
            "{}: exit {:?}: {} {}",
            label,
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).trim(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// The MySQL root password to use at install: reflect the one already published
/// in the cloud (`credentials.mysql_root_password`) if an admin/other machine set
/// it — cloud is the source of truth — otherwise generate a fresh one (which
/// `persist_mysql_root_password` then pushes back up). Safe at install time
/// because the data dir is freshly initialized, so any value can be set.
#[cfg(target_os = "windows")]
async fn cloud_or_generated_mysql_password() -> String {
    if let Ok(cfg) = crate::config::load_config() {
        if !cfg.hospital_code.is_empty() {
            if let Ok(client) = crate::firestore::FirestoreClient::new_from_config().await {
                if let Ok(Some(pw)) = client.get_hospital_mysql_password(&cfg.hospital_code).await {
                    tracing::info!("MySQL: using root password from cloud credentials");
                    return pw;
                }
            }
        }
    }
    generate_password(24)
}

/// Generate a random alphanumeric password (ambiguous chars omitted).
#[cfg(target_os = "windows")]
fn generate_password(len: usize) -> String {
    use rand::Rng;
    const CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut rng = rand::thread_rng();
    (0..len).map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char).collect()
}

// ══════════════════════════════════════════════════════════════════════════════
//  Shared helpers
// ══════════════════════════════════════════════════════════════════════════════

fn emit_progress(
    app: &tauri::AppHandle,
    software: &str,
    stage: InstallStage,
    percent: u8,
    message: &str,
    bytes_downloaded: u64,
    bytes_total: u64,
) {
    use tauri::Emitter;
    let _ = app.emit("install-progress", InstallProgress {
        software: software.into(),
        stage,
        percent,
        message: message.into(),
        bytes_downloaded,
        bytes_total,
    });
}

fn extract_version(text: &str) -> Option<String> {
    for word in text.split_whitespace() {
        let clean = word.trim_matches(|c: char| !c.is_ascii_digit() && c != '.');
        if clean.contains('.') && clean.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            return Some(clean.split('-').next().unwrap_or(clean).to_string());
        }
    }
    None
}

// ── Unix verification (macOS + Linux) ───────────────────────────────────────

#[cfg(not(target_os = "windows"))]
async fn verify_install_unix(software: &str) -> Option<String> {
    let (cmd, args): (&str, &[&str]) = match software {
        "MySQL" => ("mysql", &["--version"]),
        "Erlang" => ("erl", &["-eval", "erlang:display(erlang:system_info(otp_release)), halt().", "-noshell"]),
        "RabbitMQ" => ("rabbitmqctl", &["version"]),
        _ => return None,
    };

    let output = tokio::process::Command::new(cmd)
        .args(args)
        .output()
        .await
        .ok()?;

    if output.status.success() {
        let raw = String::from_utf8_lossy(&output.stdout);
        extract_version(&raw).or_else(|| Some(raw.trim().trim_matches('"').to_string()))
    } else {
        None
    }
}

// ── Windows verification ────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn verify_install_windows(software: &str) -> Option<String> {
    match software {
        "MySQL" => {
            let dir = find_mysql_install_dir()?;
            let mysql_bin = dir.join("bin").join("mysql.exe");
            let output = std::process::Command::new(&mysql_bin)
                .arg("--version")
                .output()
                .ok()?;
            if output.status.success() {
                extract_version(&String::from_utf8_lossy(&output.stdout))
            } else {
                None
            }
        }
        "Erlang" => {
            let erl = format!(r"{}\bin\erl.exe", ERLANG_INSTALL_DIR);
            if std::path::Path::new(&erl).exists() {
                // Try to get actual version
                let output = std::process::Command::new(&erl)
                    .args(["-eval", "erlang:display(erlang:system_info(otp_release)), halt().", "-noshell"])
                    .output()
                    .ok();
                if let Some(out) = output {
                    if out.status.success() {
                        let raw = String::from_utf8_lossy(&out.stdout);
                        return Some(raw.trim().trim_matches('"').to_string());
                    }
                }
                Some("installed".into())
            } else {
                None
            }
        }
        "RabbitMQ" => {
            let rabbitmq_base = r"C:\Program Files\RabbitMQ Server";
            if let Ok(entries) = std::fs::read_dir(rabbitmq_base) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with("rabbitmq_server-") {
                        let ctl = format!(r"{}\{}\sbin\rabbitmqctl.bat", rabbitmq_base, name);
                        if let Ok(output) = std::process::Command::new(&ctl).arg("version").output() {
                            if output.status.success() {
                                return extract_version(&String::from_utf8_lossy(&output.stdout));
                            }
                        }
                    }
                }
            }
            None
        }
        _ => None,
    }
}

// ── GitHub latest release resolver ──────────────────────────────────────────

/// Fetch latest GitHub release asset matching a pattern, with fallback URL.
#[cfg(target_os = "windows")]
#[allow(dead_code)]
async fn resolve_latest_github_asset<F>(
    repo: &str,
    matcher: F,
    fallback_url: &str,
) -> (String, String)
where
    F: Fn(&str) -> bool,
{
    let fallback = (
        fallback_url.to_string(),
        fallback_url.rsplit('/').next().unwrap_or("installer").to_string(),
    );

    let api_url = format!("https://api.github.com/repos/{}/releases/latest", repo);
    let client = reqwest::Client::new();

    let response = match client
        .get(&api_url)
        .header("User-Agent", "puru-nucleus")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        _ => {
            tracing::warn!("Could not reach GitHub API for {}, using fallback", repo);
            return fallback;
        }
    };

    let json: serde_json::Value = match response.json().await {
        Ok(j) => j,
        Err(_) => return fallback,
    };

    let assets = match json.get("assets").and_then(|a| a.as_array()) {
        Some(a) => a,
        None => return fallback,
    };

    for asset in assets {
        if let (Some(name), Some(url)) = (
            asset.get("name").and_then(|n| n.as_str()),
            asset.get("browser_download_url").and_then(|u| u.as_str()),
        ) {
            if matcher(name) {
                tracing::info!("Resolved latest {}: {}", repo, name);
                return (url.to_string(), name.to_string());
            }
        }
    }

    tracing::warn!("No matching asset for {}, using fallback", repo);
    fallback
}

// ── Download with progress (Windows only — macOS/Linux use package managers) ─

#[cfg(target_os = "windows")]
#[allow(dead_code)]
async fn download_with_progress(
    app: &tauri::AppHandle,
    software: &str,
    url: &str,
    dest: &PathBuf,
) -> Result<(), String> {
    use futures_util::StreamExt;
    use std::io::Write;

    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download failed: HTTP {}", response.status()));
    }

    let total = response.content_length().unwrap_or(0);
    let mut downloaded: u64 = 0;
    let mut last_percent: u8 = 0;

    let mut file = std::fs::File::create(dest)
        .map_err(|e| format!("Cannot create temp file: {}", e))?;

    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download error: {}", e))?;
        file.write_all(&chunk)
            .map_err(|e| format!("Write error: {}", e))?;

        downloaded += chunk.len() as u64;

        let percent = if total > 0 {
            ((downloaded as f64 / total as f64) * 100.0) as u8
        } else {
            0
        };

        if percent >= last_percent + 2 || percent == 100 {
            last_percent = percent;
            let mb_down = downloaded as f64 / 1_048_576.0;
            let mb_total = total as f64 / 1_048_576.0;
            let msg = if total > 0 {
                format!("Downloading... {:.1} / {:.1} MB", mb_down, mb_total)
            } else {
                format!("Downloading... {:.1} MB", mb_down)
            };
            emit_progress(app, software, InstallStage::Downloading, percent, &msg, downloaded, total);
        }
    }

    file.flush().map_err(|e| format!("Flush error: {}", e))?;
    Ok(())
}

// ── Windows silent installers ───────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn install_mysql_silent(installer_path: &PathBuf) -> Result<(), String> {
    let output = std::process::Command::new("msiexec")
        .args(["/i", installer_path.to_str().unwrap_or(""), "/quiet", "/norestart"])
        .output()
        .map_err(|e| format!("Failed to run msiexec: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("MySQL installer exited with code {:?}: {}", output.status.code(), stderr));
    }

    // PATH is added by the caller after discovering the real install dir.
    Ok(())
}

/// Locate an existing MySQL install under `C:\Program Files\MySQL` by finding a
/// subdirectory that contains `bin\mysqld.exe`. Version-agnostic — matches
/// `MySQL Server 8.0`, `MySQL Server 9.7`, our own `MySQL Server`, etc.
#[cfg(target_os = "windows")]
fn find_mysql_install_dir() -> Option<PathBuf> {
    let base = std::path::Path::new(MYSQL_BASE);
    for entry in std::fs::read_dir(base).ok()?.flatten() {
        let dir = entry.path();
        if dir.join("bin").join("mysqld.exe").exists() {
            return Some(dir);
        }
    }
    None
}

/// True if something is accepting connections on 127.0.0.1:`port`.
#[cfg(target_os = "windows")]
fn tcp_port_open(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        std::time::Duration::from_millis(800),
    )
    .is_ok()
}

/// Poll 127.0.0.1:`port` until it accepts a connection or `max_secs` elapses.
#[cfg(target_os = "windows")]
fn wait_tcp(port: u16, max_secs: u64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(max_secs);
    loop {
        if tcp_port_open(port) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(700));
    }
}

/// Name of an existing Windows service managing MySQL (`MySQL80`, `MySQL`, …),
/// via `sc query`. None if there is no MySQL service registered.
#[cfg(target_os = "windows")]
fn mysql_service_name() -> Option<String> {
    let out = std::process::Command::new("sc")
        .args(["query", "state=", "all"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if let Some(rest) = line.trim().strip_prefix("SERVICE_NAME:") {
            let name = rest.trim();
            if name.to_lowercase().contains("mysql") {
                return Some(name.to_string());
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn install_erlang_silent(installer_path: &PathBuf) -> Result<(), String> {
    let output = std::process::Command::new(installer_path)
        .args(["/S", &format!("/D={}", ERLANG_INSTALL_DIR)])
        .output()
        .map_err(|e| format!("Failed to run Erlang installer: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Erlang installer exited with code {:?}: {}", output.status.code(), stderr));
    }

    set_system_env("ERLANG_HOME", ERLANG_INSTALL_DIR);
    Ok(())
}

#[cfg(target_os = "windows")]
fn install_rabbitmq_silent(installer_path: &PathBuf) -> Result<(), String> {
    let output = std::process::Command::new(installer_path)
        .args(["/S"])
        .output()
        .map_err(|e| format!("Failed to run RabbitMQ installer: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("RabbitMQ installer exited with code {:?}: {}", output.status.code(), stderr));
    }

    // Add RabbitMQ sbin to PATH
    let rabbitmq_base = r"C:\Program Files\RabbitMQ Server";
    if let Ok(entries) = std::fs::read_dir(rabbitmq_base) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("rabbitmq_server-") {
                add_to_system_path(&format!(r"{}\{}\sbin", rabbitmq_base, name));
                break;
            }
        }
    }

    // Enable management plugin
    let _ = std::process::Command::new("rabbitmq-plugins")
        .args(["enable", "rabbitmq_management"])
        .output();

    Ok(())
}

// ── Windows PATH/env helpers ────────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn add_to_system_path(new_path: &str) {
    const KEY: &str = r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

    // Read the *machine* PATH from the registry — NOT std::env PATH, which is the
    // process-merged system+user+session value; writing that back would promote
    // user/session entries into the system PATH.
    let current = std::process::Command::new("reg")
        .args(["query", KEY, "/v", "Path"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            let text = String::from_utf8_lossy(&o.stdout).to_string();
            text.lines()
                .find(|l| l.contains("REG_") && l.to_lowercase().contains("path"))
                .and_then(|l| l.find("REG_").map(|p| l[p..].to_string()))
                .and_then(|rest| rest.splitn(2, char::is_whitespace).nth(1).map(|v| v.trim().to_string()))
        })
        .unwrap_or_default();

    // Already present (per-entry, case-insensitive)? Then nothing to do.
    let want = new_path.trim_end_matches('\\').to_lowercase();
    if current
        .split(';')
        .any(|e| e.trim().trim_end_matches('\\').to_lowercase() == want)
    {
        return;
    }

    let newval = if current.is_empty() {
        new_path.to_string()
    } else {
        format!("{};{}", current.trim_end_matches(';'), new_path)
    };

    // Write via `reg add` (REG_EXPAND_SZ) — no 1024-char truncation like setx /M.
    let _ = std::process::Command::new("reg")
        .args(["add", KEY, "/v", "Path", "/t", "REG_EXPAND_SZ", "/d", &newval, "/f"])
        .output();
}

#[cfg(target_os = "windows")]
fn set_system_env(key: &str, value: &str) {
    let _ = std::process::Command::new("setx")
        .args(["/M", key, value])
        .output();
}
