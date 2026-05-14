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
const MYSQL_INSTALL_DIR: &str = r"C:\Program Files\MySQL\MySQL Server 8.0";
#[cfg(target_os = "windows")]
const ERLANG_INSTALL_DIR: &str = r"C:\Program Files\Erlang OTP";

// Fallback URLs (if GitHub API is unreachable)
#[cfg(target_os = "windows")]
const MYSQL_FALLBACK_URL: &str = "https://dev.mysql.com/get/Downloads/MySQL-8.0/mysql-8.0.40-winx64.msi";
const ERLANG_FALLBACK_URL: &str = "https://github.com/erlang/otp/releases/download/OTP-26.2.5.6/otp_win64_26.2.5.6.exe";
const RABBITMQ_FALLBACK_URL: &str = "https://github.com/rabbitmq/rabbitmq-server/releases/download/v3.13.7/rabbitmq-server-3.13.7.exe";

// ── Public API ──────────────────────────────────────────────────────────────

/// Install missing prerequisites. Routes to platform-specific implementation.
pub async fn install_missing(
    app: &tauri::AppHandle,
    software: &[String],
) -> Vec<InstallResult> {
    let mut results = Vec::new();

    let install_mysql = software.iter().any(|s| s.eq_ignore_ascii_case("mysql"));
    let install_rabbitmq = software.iter().any(|s| s.eq_ignore_ascii_case("rabbitmq"));

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

#[cfg(target_os = "windows")]
async fn do_install_mysql(app: &tauri::AppHandle) -> InstallResult {
    let url = MYSQL_FALLBACK_URL.to_string();
    let filename = url.rsplit('/').next().unwrap_or("mysql-installer.msi").to_string();
    install_downloaded(app, "MySQL", &url, &filename, |path| {
        install_mysql_silent(path)
    }).await
}

#[cfg(target_os = "windows")]
async fn do_install_erlang(app: &tauri::AppHandle) -> InstallResult {
    let (url, filename) = resolve_latest_github_asset(
        "erlang/otp",
        |name| name.starts_with("otp_win64_") && name.ends_with(".exe"),
        ERLANG_FALLBACK_URL,
    ).await;
    emit_progress(app, "Erlang", InstallStage::Downloading, 0,
        &format!("Resolved: {}", filename), 0, 0);
    install_downloaded(app, "Erlang", &url, &filename, |path| {
        install_erlang_silent(path)
    }).await
}

#[cfg(target_os = "windows")]
async fn do_install_rabbitmq(app: &tauri::AppHandle) -> InstallResult {
    let (url, filename) = resolve_latest_github_asset(
        "rabbitmq/rabbitmq-server",
        |name| name.starts_with("rabbitmq-server-") && name.ends_with(".exe"),
        RABBITMQ_FALLBACK_URL,
    ).await;
    emit_progress(app, "RabbitMQ", InstallStage::Downloading, 0,
        &format!("Resolved: {}", filename), 0, 0);
    install_downloaded(app, "RabbitMQ", &url, &filename, |path| {
        install_rabbitmq_silent(path)
    }).await
}

/// Download an installer and run it silently (Windows pattern).
#[cfg(target_os = "windows")]
async fn install_downloaded<F>(
    app: &tauri::AppHandle,
    name: &str,
    url: &str,
    filename: &str,
    install_fn: F,
) -> InstallResult
where
    F: FnOnce(&PathBuf) -> Result<(), String>,
{
    // 1. Download
    emit_progress(app, name, InstallStage::Downloading, 0, "Starting download...", 0, 0);

    let temp_dir = std::env::temp_dir();
    let dest = temp_dir.join(filename);

    match download_with_progress(app, name, url, &dest).await {
        Ok(_) => {}
        Err(e) => {
            emit_progress(app, name, InstallStage::Failed, 0, &e, 0, 0);
            return InstallResult {
                software: name.into(),
                success: false,
                version: None,
                error: Some(e),
            };
        }
    }

    // 2. Install
    emit_progress(app, name, InstallStage::Installing, 0, "Running installer (UAC prompt may appear)...", 0, 0);

    match install_fn(&dest) {
        Ok(_) => {}
        Err(e) => {
            emit_progress(app, name, InstallStage::Failed, 0, &e, 0, 0);
            let _ = std::fs::remove_file(&dest);
            return InstallResult {
                software: name.into(),
                success: false,
                version: None,
                error: Some(e),
            };
        }
    }

    // 3. Cleanup installer
    let _ = std::fs::remove_file(&dest);

    // 4. Verify
    emit_progress(app, name, InstallStage::Verifying, 0, "Verifying installation...", 0, 0);

    let version = verify_install_windows(name);
    let success = version.is_some();

    let stage = if success { InstallStage::Completed } else { InstallStage::Failed };
    let msg = if success {
        format!("Installed successfully ({})", version.as_deref().unwrap_or("unknown"))
    } else {
        "Installation could not be verified".into()
    };
    emit_progress(app, name, stage, if success { 100 } else { 0 }, &msg, 0, 0);

    InstallResult {
        software: name.into(),
        success,
        version,
        error: if success { None } else { Some("Installation could not be verified".into()) },
    }
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
            let mysql_bin = format!(r"{}\bin\mysql.exe", MYSQL_INSTALL_DIR);
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

    add_to_system_path(&format!(r"{}\bin", MYSQL_INSTALL_DIR));
    Ok(())
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
    let current = std::env::var("PATH").unwrap_or_default();
    if current.to_lowercase().contains(&new_path.to_lowercase()) {
        return;
    }
    let _ = std::process::Command::new("setx")
        .args(["/M", "PATH", &format!("{};{}", current, new_path)])
        .output();
}

#[cfg(target_os = "windows")]
fn set_system_env(key: &str, value: &str) {
    let _ = std::process::Command::new("setx")
        .args(["/M", key, value])
        .output();
}
