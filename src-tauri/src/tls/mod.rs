//! TLS certificate generation and management for hospital deployments.
//!
//! Uses `mkcert` to generate a local CA + server certificate so that
//! intranet HTTPS works with trusted certs on all client machines.
//!
//! ## Flow
//! 1. `setup_tls()` — generates CA + cert, configures Nginx, exports CA for download
//! 2. `get_tls_status()` — check if TLS is configured
//! 3. `generate_client_setup_script()` — produces a .bat/.sh for client machines
//!
//! Requires `mkcert` binary in PATH or at a known location.

use crate::config::NucleusConfig;
use crate::error::NucleusError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Where Nginx expects certs
const NGINX_CERT_DIR: &str = "/etc/nginx/certs";
const CERT_FILENAME: &str = "server.pem";
const KEY_FILENAME: &str = "server-key.pem";

/// TLS setup status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsStatus {
    pub configured: bool,
    pub server_ip: String,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
    pub ca_download_url: Option<String>,
    pub client_setup_script_url: Option<String>,
    pub checked_at: DateTime<Utc>,
}

/// Result of TLS setup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsSetupResult {
    pub success: bool,
    pub message: String,
    pub ca_path: Option<String>,
    pub cert_path: Option<String>,
    pub key_path: Option<String>,
}

/// Check if mkcert is available
async fn find_mkcert() -> Result<String, NucleusError> {
    // Check common locations
    let candidates = vec![
        "mkcert".to_string(),
        "/usr/local/bin/mkcert".to_string(),
        "/usr/bin/mkcert".to_string(),
    ];

    for path in &candidates {
        let result = Command::new(path).arg("-version").output().await;
        if result.is_ok() {
            return Ok(path.clone());
        }
    }

    Err(NucleusError::Internal(
        "mkcert not found. Install it: https://github.com/nicedaysoftware/mkcert".to_string(),
    ))
}

/// Generate local CA and server certificate
pub async fn setup_tls(config: &NucleusConfig) -> Result<TlsSetupResult, NucleusError> {
    let mkcert = find_mkcert().await?;
    let server_ip = &config.server_ip;

    if server_ip.is_empty() {
        return Err(NucleusError::Validation(
            "server_ip not configured in nucleus.toml".to_string(),
        ));
    }

    // 1. Install local CA (creates rootCA.pem + rootCA-key.pem)
    let output = Command::new(&mkcert)
        .arg("-install")
        .output()
        .await
        .map_err(|e| NucleusError::Internal(format!("mkcert -install failed: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NucleusError::Internal(format!(
            "mkcert -install failed: {}",
            stderr
        )));
    }

    // 2. Find the CA root directory
    let ca_root_output = Command::new(&mkcert)
        .arg("-CAROOT")
        .output()
        .await
        .map_err(|e| NucleusError::Internal(format!("mkcert -CAROOT failed: {}", e)))?;

    let ca_root = String::from_utf8_lossy(&ca_root_output.stdout)
        .trim()
        .to_string();
    let ca_pem_path = PathBuf::from(&ca_root).join("rootCA.pem");

    // 3. Generate server cert for the IP
    let cert_dir = Path::new(NGINX_CERT_DIR);
    tokio::fs::create_dir_all(cert_dir).await.map_err(|e| {
        NucleusError::Internal(format!("Cannot create {}: {}", NGINX_CERT_DIR, e))
    })?;

    // mkcert generates files in CWD, so we run it in a temp dir then move
    let temp_dir = std::env::temp_dir().join("puru-tls-gen");
    tokio::fs::create_dir_all(&temp_dir).await.ok();

    let output = Command::new(&mkcert)
        .current_dir(&temp_dir)
        .arg(server_ip)
        .arg("localhost")
        .arg("127.0.0.1")
        .output()
        .await
        .map_err(|e| NucleusError::Internal(format!("mkcert cert generation failed: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(NucleusError::Internal(format!(
            "mkcert cert generation failed: {}",
            stderr
        )));
    }

    // 4. Find and move the generated cert files
    // mkcert creates files like: 192.168.31.234+2.pem and 192.168.31.234+2-key.pem
    let mut cert_file = None;
    let mut key_file = None;
    let mut entries = tokio::fs::read_dir(&temp_dir).await.map_err(|e| {
        NucleusError::Internal(format!("Cannot read temp dir: {}", e))
    })?;

    while let Some(entry) = entries.next_entry().await.map_err(|e| {
        NucleusError::Internal(format!("Cannot read dir entry: {}", e))
    })? {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.contains(server_ip) && name.ends_with("-key.pem") {
            key_file = Some(entry.path());
        } else if name.contains(server_ip) && name.ends_with(".pem") {
            cert_file = Some(entry.path());
        }
    }

    let cert_src = cert_file.ok_or_else(|| {
        NucleusError::Internal("Generated cert file not found".to_string())
    })?;
    let key_src = key_file.ok_or_else(|| {
        NucleusError::Internal("Generated key file not found".to_string())
    })?;

    let cert_dst = cert_dir.join(CERT_FILENAME);
    let key_dst = cert_dir.join(KEY_FILENAME);

    tokio::fs::copy(&cert_src, &cert_dst).await.map_err(|e| {
        NucleusError::Internal(format!("Cannot copy cert: {}", e))
    })?;
    tokio::fs::copy(&key_src, &key_dst).await.map_err(|e| {
        NucleusError::Internal(format!("Cannot copy key: {}", e))
    })?;

    // Cleanup temp
    tokio::fs::remove_dir_all(&temp_dir).await.ok();

    // 5. Copy root CA to data dir for HTTP download
    // Data root is typically the parent of docker-compose.yml location
    let compose_path = PathBuf::from(&config.docker_compose_path);
    let data_root = compose_path
        .parent()
        .unwrap_or_else(|| Path::new("/home/puru"))
        .join("puru-data");
    let ca_download_dir = data_root.join("uploaded-document");
    tokio::fs::create_dir_all(&ca_download_dir).await.ok();
    let ca_dst = ca_download_dir.join("puru-root-ca.crt");
    tokio::fs::copy(&ca_pem_path, &ca_dst).await.map_err(|e| {
        NucleusError::Internal(format!("Cannot copy root CA: {}", e))
    })?;

    Ok(TlsSetupResult {
        success: true,
        message: format!(
            "TLS configured for {}. Root CA available for download. Restart Nginx to apply.",
            server_ip
        ),
        ca_path: Some(ca_dst.to_string_lossy().to_string()),
        cert_path: Some(cert_dst.to_string_lossy().to_string()),
        key_path: Some(key_dst.to_string_lossy().to_string()),
    })
}

/// Check current TLS status
pub async fn get_tls_status(config: &NucleusConfig) -> TlsStatus {
    let cert_path = PathBuf::from(NGINX_CERT_DIR).join(CERT_FILENAME);
    let key_path = PathBuf::from(NGINX_CERT_DIR).join(KEY_FILENAME);
    let configured = cert_path.exists() && key_path.exists();

    let server_ip = config.server_ip.clone();
    let file_port = 81; // standard file explorer port

    TlsStatus {
        configured,
        server_ip: server_ip.clone(),
        cert_path: if configured {
            Some(cert_path.to_string_lossy().to_string())
        } else {
            None
        },
        key_path: if configured {
            Some(key_path.to_string_lossy().to_string())
        } else {
            None
        },
        ca_download_url: if configured {
            Some(format!("http://{}:{}/puru-root-ca.crt", server_ip, file_port))
        } else {
            None
        },
        client_setup_script_url: if configured {
            Some(format!(
                "http://{}:{}/puru-secure-setup.bat",
                server_ip, file_port
            ))
        } else {
            None
        },
        checked_at: Utc::now(),
    }
}

/// Generate the client setup .bat script content for a given server IP
pub fn generate_client_setup_script(server_ip: &str, file_port: u16, app_port: u16) -> String {
    let ca_url = format!("http://{}:{}/puru-root-ca.crt", server_ip, file_port);
    let http_origin = format!("http://{}:{}", server_ip, app_port);
    let https_origin = format!("https://{}", server_ip);

    format!(
        r#"@echo off
echo ============================================
echo   Puru HMS - Secure Setup
echo   Hospital Server: {server_ip}
echo ============================================
echo.

:: Check admin
net session >nul 2>&1
if %errorlevel% neq 0 (
    echo ERROR: Administrator rights required.
    echo Right-click this file and select "Run as administrator".
    pause
    exit /b 1
)

:: Download root CA
echo Downloading certificate...
curl -s -o "%TEMP%\puru-root-ca.crt" "{ca_url}"
if not exist "%TEMP%\puru-root-ca.crt" (
    echo ERROR: Could not download certificate. Is the server running?
    pause
    exit /b 1
)

:: Install to Trusted Root CAs
echo Installing certificate...
certutil -addstore -f "Root" "%TEMP%\puru-root-ca.crt" >nul 2>&1
if %errorlevel% neq 0 (
    echo ERROR: Certificate installation failed.
    pause
    exit /b 1
)

:: Chrome policy for HTTP fallback
echo Configuring Chrome...
reg add "HKLM\SOFTWARE\Policies\Google\Chrome\OverrideSecurityRestrictionsOnInsecureOrigin" /v 1 /t REG_SZ /d "{http_origin}" /f >nul 2>&1
reg add "HKLM\SOFTWARE\Policies\Google\Chrome\OverrideSecurityRestrictionsOnInsecureOrigin" /v 2 /t REG_SZ /d "{https_origin}" /f >nul 2>&1

:: Cleanup
del "%TEMP%\puru-root-ca.crt" >nul 2>&1

echo.
echo ============================================
echo   Setup complete!
echo   Restart Chrome to apply changes.
echo   Then open: {https_origin}
echo ============================================
pause
"#,
        server_ip = server_ip,
        ca_url = ca_url,
        http_origin = http_origin,
        https_origin = https_origin,
    )
}

/// Generate Nginx HTTPS config block
pub fn generate_nginx_https_config(server_ip: &str) -> String {
    format!(
        r#"# Puru HMS HTTPS — auto-generated by puru-dc
server {{
    listen 443 ssl;
    server_name {server_ip};

    ssl_certificate     /etc/nginx/certs/server.pem;
    ssl_certificate_key /etc/nginx/certs/server-key.pem;
    ssl_protocols       TLSv1.2 TLSv1.3;

    # Angular app
    location / {{
        root /home/puru/puru-data/dist/puru-ui;
        try_files $uri $uri/ /index.html;
    }}

    # Auth service (8080)
    location /authenticate {{ proxy_pass http://127.0.0.1:8080; proxy_set_header Host $host; }}
    location /logout/ {{ proxy_pass http://127.0.0.1:8080; proxy_set_header Host $host; }}
    location /config/ {{ proxy_pass http://127.0.0.1:8080; proxy_set_header Host $host; }}
    location /allemployees {{ proxy_pass http://127.0.0.1:8080; proxy_set_header Host $host; }}
    location /api/msg/ {{ proxy_pass http://127.0.0.1:8080; proxy_set_header Host $host; }}

    # WebSocket (Auth + Realtime)
    location /ws {{
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_set_header Host $host;
    }}

    # Xenon service (8081)
    location /xenon/ {{ proxy_pass http://127.0.0.1:8081; proxy_set_header Host $host; }}

    # HAS service (8082)
    location /has/ {{ proxy_pass http://127.0.0.1:8082; proxy_set_header Host $host; }}

    # PACS service (8083)
    location /pacs/ {{ proxy_pass http://127.0.0.1:8083; proxy_set_header Host $host; }}

    # Argon service (8084)
    location /argon/ {{ proxy_pass http://127.0.0.1:8084; proxy_set_header Host $host; }}

    # Comm service (8085)
    location /comm/ {{ proxy_pass http://127.0.0.1:8085; proxy_set_header Host $host; }}

    # Realtime service (8086)
    location /realtime/ {{ proxy_pass http://127.0.0.1:8086; proxy_set_header Host $host; }}

    # Neon service (8087)
    location /neon/ {{ proxy_pass http://127.0.0.1:8087; proxy_set_header Host $host; }}

    # Mercury service (8089)
    location /mercury/ {{ proxy_pass http://127.0.0.1:8089; proxy_set_header Host $host; }}

    # Bridge service (8094)
    location /bridge/ {{ proxy_pass http://127.0.0.1:8094; proxy_set_header Host $host; }}
}}

# Redirect HTTP to HTTPS
server {{
    listen 80;
    server_name {server_ip};
    return 301 https://$host$request_uri;
}}
"#,
        server_ip = server_ip,
    )
}
