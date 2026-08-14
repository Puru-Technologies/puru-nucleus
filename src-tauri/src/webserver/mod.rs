//! Native-mode web tier — serves the Angular UI (puru-hydrogen) and reverse-
//! proxies to the backend services via a managed nginx process.
//!
//! In Docker mode nginx runs as a container; in native mode nothing serves the
//! static UI or proxies the services, so nucleus provisions and supervises its
//! own nginx (downloaded from GCS, same as JREs/JARs). The `puru-hydrogen`
//! service in native mode maps to "is this nginx up and serving the html dir".

use crate::config::NucleusConfig;
use crate::error::NucleusError;
use std::path::PathBuf;

/// Fallback nginx version/object used only if the infra manifest has no `nginx`
/// entry. The live path is resolved from the manifest and the extracted folder,
/// so bumping nginx in GCS no longer requires editing constants here.
const NGINX_VERSION: &str = "1.26.3";
const NGINX_GCS_OBJECT: &str = "infra/nginx-1.26.3-windows.zip";
const HTTP_PORT: u16 = 80;
/// Static file server for the puru_data tree (documents, uploads, PACS, …),
/// mirroring the Docker "file-server" container on :81.
const FILE_SERVER_PORT: u16 = 81;

/// Reverse-proxy map: URL location prefix -> backend port. Mirrors the Docker
/// nginx config and the tls module's HTTPS config so both tiers route alike.
const PROXY_ROUTES: &[(&str, u16)] = &[
    ("/authenticate", 8080),
    ("/logout/", 8080),
    ("/config/", 8080),
    ("/allemployees", 8080),
    ("/api/msg/", 8080),
    ("/xenon/", 8081),
    ("/has/", 8082),
    ("/pacs/", 8083),
    ("/argon/", 8084),
    ("/comm/", 8085),
    ("/realtime/", 8086),
    ("/neon/", 8087),
    ("/integration/", 8088),
    ("/mercury/", 8089),
    ("/bridge/", 8094),
];

// ── Paths ────────────────────────────────────────────────────────────────────

/// Base dir that holds the extracted `nginx-<ver>/` folder, e.g. C:\PuruNucleus\nginx
fn nginx_base(config: &NucleusConfig) -> PathBuf {
    config
        .nginx_html_dir()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from(r"C:\PuruNucleus\nginx"))
}

/// Discover an installed nginx dir (`<base>\nginx-*` containing `nginx.exe`),
/// version-agnostic — matches whatever version the archive extracted to.
fn find_nginx_dir(config: &NucleusConfig) -> Option<PathBuf> {
    let base = nginx_base(config);
    for entry in std::fs::read_dir(&base).ok()?.flatten() {
        let dir = entry.path();
        let is_nginx = dir
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("nginx-"))
            .unwrap_or(false);
        if is_nginx && dir.join("nginx.exe").exists() {
            return Some(dir);
        }
    }
    None
}

/// nginx install dir — the discovered one if present, else the fallback target.
fn nginx_dir(config: &NucleusConfig) -> PathBuf {
    find_nginx_dir(config).unwrap_or_else(|| nginx_base(config).join(format!("nginx-{}", NGINX_VERSION)))
}

fn nginx_exe(config: &NucleusConfig) -> PathBuf {
    nginx_dir(config).join("nginx.exe")
}

/// The managed nginx `conf/` directory (holds puru.conf and any extra includes).
pub fn conf_dir(config: &NucleusConfig) -> PathBuf {
    nginx_dir(config).join("conf")
}

fn config_path(config: &NucleusConfig) -> PathBuf {
    // Live alongside the install so nginx's relative prefix (logs/, temp/) resolves.
    conf_dir(config).join("puru.conf")
}

/// nginx wants forward slashes (and quotes) in directives, even on Windows.
fn fwd(path: &std::path::Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

// ── Config generation ──────────────────────────────────────────────────────

/// Generate a self-contained HTTP nginx config: serve the Angular SPA from the
/// html dir and reverse-proxy the backend services. (HTTPS is handled
/// separately by the tls module when certs are provisioned.)
pub fn generate_config(config: &NucleusConfig) -> String {
    let mime = fwd(&nginx_dir(config).join("conf").join("mime.types"));
    let html = fwd(&config.nginx_html_dir());
    let server_name = if config.server_ip.is_empty() {
        "localhost".to_string()
    } else {
        config.server_ip.clone()
    };

    // Each backend location dispatches to @spa when the request looks like a
    // browser navigation (Accept: text/html) instead of an XHR. Without this,
    // a refresh on e.g. /has/patient-dashboard/PPIN proxies the page-load
    // request to HAS, JwtRequestFilter sees no Authorization header (JWT is
    // in localStorage, not cookies), and returns a bare 401 the browser
    // renders as a plain "HTTP ERROR 401" page. The Angular HttpClient
    // doesn't advertise text/html, so XHR proxying is unaffected.
    let mut locations = String::new();
    for (route, port) in PROXY_ROUTES {
        locations.push_str(&format!(
            r#"        location {route} {{
            if ($http_accept ~* "^text/html") {{ return 418; }}
            error_page 418 = @spa;

            proxy_pass http://127.0.0.1:{port};
            proxy_set_header Host $host;
            proxy_set_header X-Real-IP $remote_addr;
        }}
"#,
            route = route,
            port = port,
        ));
    }

    // File server (:81) — serves the whole puru_data directory, like the Docker
    // file-server container. Only emitted when puru_data_path is configured.
    let file_server = match config.puru_data_path.as_deref().filter(|p| !p.is_empty()) {
        Some(data) => format!(
            r#"
    # Static file server for the puru_data tree (documents, uploads, PACS, …)
    server {{
        listen {fport};
        server_name {server_name};
        client_max_body_size 100m;

        location / {{
            root  "{data}";
            autoindex off;
            add_header Access-Control-Allow-Origin *;
        }}
    }}
"#,
            fport = FILE_SERVER_PORT,
            server_name = server_name,
            data = fwd(std::path::Path::new(data)),
        ),
        None => String::new(),
    };

    format!(
        r#"# Auto-generated by puru-nucleus — do not edit by hand.
worker_processes  1;
events {{ worker_connections  1024; }}
http {{
    include       "{mime}";
    default_type  application/octet-stream;
    sendfile      on;
    client_max_body_size 100m;

    server {{
        listen {port};
        server_name {server_name};

        # Angular SPA
        location / {{
            root  "{html}";
            index index.html;
            try_files $uri $uri/ /index.html;
        }}

        # SPA fallback for backend proxy routes — dispatched via `error_page 418`
        # from each /has/, /xenon/, etc. block when the request is a browser
        # navigation (Accept: text/html) rather than an XHR.
        location @spa {{
            root  "{html}";
            try_files /index.html =404;
        }}

        # WebSocket (auth/realtime STOMP + SockJS)
        location /ws {{
            proxy_pass http://127.0.0.1:8080;
            proxy_http_version 1.1;
            proxy_set_header Upgrade $http_upgrade;
            proxy_set_header Connection "upgrade";
            proxy_set_header Host $host;
        }}

        # Backend services
{locations}    }}
{file_server}}}
"#,
        mime = mime,
        port = HTTP_PORT,
        server_name = server_name,
        html = html,
        locations = locations,
        file_server = file_server,
    )
}

// ── Provisioning ───────────────────────────────────────────────────────────

/// Download + extract nginx from GCS if not already present. Returns nginx.exe.
/// The object is resolved from the infra manifest (`nginx` component); the
/// installed directory is then discovered from the extracted folder, so the
/// version is never assumed.
pub async fn ensure_nginx(config: &NucleusConfig) -> Result<PathBuf, NucleusError> {
    if let Some(dir) = find_nginx_dir(config) {
        return Ok(dir.join("nginx.exe"));
    }

    let cred_path = crate::releases::get_credentials_path()?;
    let client = crate::releases::create_gcs_client(&cred_path).await?;

    let base = nginx_base(config);
    tokio::fs::create_dir_all(&base).await?;

    // Prefer the manifest's nginx artifact; fall back to the legacy object path.
    let object = match crate::releases::fetch_infra_manifest(&client).await {
        Ok(manifest) => crate::releases::resolve_infra_artifact(&manifest, "nginx")
            .map(|a| a.object_path)
            .unwrap_or_else(|| NGINX_GCS_OBJECT.to_string()),
        Err(_) => NGINX_GCS_OBJECT.to_string(),
    };
    tracing::info!("nginx not found locally, downloading {}...", object);

    let tmp_zip = std::env::temp_dir().join("puru-nginx.zip");
    crate::releases::download_gcs_to_file(&client, &object, &tmp_zip).await?;

    // The archive contains a top-level nginx-<ver>/ dir, so extract into `base`.
    let output = crate::process::silent_cmd("tar")
        .args(["-xf", &tmp_zip.to_string_lossy(), "-C", &base.to_string_lossy()])
        .output()
        .await?;
    let _ = tokio::fs::remove_file(&tmp_zip).await;

    if !output.status.success() {
        return Err(NucleusError::Internal(format!(
            "Failed to extract nginx: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let dir = find_nginx_dir(config).ok_or_else(|| {
        NucleusError::NotFound(format!(
            "nginx extracted but no nginx-*/nginx.exe found under {}",
            base.display()
        ))
    })?;
    tracing::info!("nginx installed at {}", dir.display());
    Ok(dir.join("nginx.exe"))
}

// ── Process control ────────────────────────────────────────────────────────

/// True if an nginx process is currently running.
pub fn is_running() -> bool {
    #[cfg(windows)]
    {
        crate::process::silent_std_cmd("tasklist")
            .args(["/FI", "IMAGENAME eq nginx.exe", "/NH"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("nginx.exe"))
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        crate::process::silent_std_cmd("pgrep")
            .args(["-x", "nginx"])
            .output()
            .map(|o| !o.stdout.is_empty())
            .unwrap_or(false)
    }
}

/// Provision nginx, (re)write its config, and start/reload it.
pub async fn start(config: &NucleusConfig) -> Result<(), NucleusError> {
    let exe = ensure_nginx(config).await?;
    let dir = nginx_dir(config);
    let conf = config_path(config);

    if !config.nginx_html_dir().join("index.html").exists() {
        return Err(NucleusError::NotFound(format!(
            "UI not deployed at {} — run `pull-jars` first",
            config.nginx_html_dir().display()
        )));
    }

    tokio::fs::write(&conf, generate_config(config)).await?;

    // If already running, reload the (possibly updated) config in place.
    if is_running() {
        let out = crate::process::silent_cmd(&exe.to_string_lossy())
            .current_dir(&dir)
            .args(["-p", &fwd(&dir), "-c", &fwd(&conf), "-s", "reload"])
            .output()
            .await?;
        if out.status.success() {
            tracing::info!("nginx config reloaded");
            return Ok(());
        }
        // Reload failed (e.g. nginx from a different prefix) — fall through to start.
    }

    // Validate config before starting, for a clear error instead of a silent exit.
    let test = crate::process::silent_cmd(&exe.to_string_lossy())
        .current_dir(&dir)
        .args(["-p", &fwd(&dir), "-c", &fwd(&conf), "-t"])
        .output()
        .await?;
    if !test.status.success() {
        return Err(NucleusError::Internal(format!(
            "nginx config test failed: {}",
            String::from_utf8_lossy(&test.stderr)
        )));
    }

    crate::process::silent_cmd(&exe.to_string_lossy())
        .current_dir(&dir)
        .args(["-p", &fwd(&dir), "-c", &fwd(&conf)])
        .spawn()
        .map_err(|e| NucleusError::Internal(format!("Failed to start nginx: {}", e)))?;

    tracing::info!("nginx started on port {}", HTTP_PORT);
    Ok(())
}

/// Stop the managed nginx (graceful quit, then force as a fallback).
pub async fn stop(config: &NucleusConfig) -> Result<(), NucleusError> {
    if !is_running() {
        return Ok(());
    }
    let exe = nginx_exe(config);
    let dir = nginx_dir(config);
    let conf = config_path(config);

    if exe.exists() {
        let _ = crate::process::silent_cmd(&exe.to_string_lossy())
            .current_dir(&dir)
            .args(["-p", &fwd(&dir), "-c", &fwd(&conf), "-s", "quit"])
            .output()
            .await;
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }
    if is_running() {
        #[cfg(windows)]
        let _ = crate::process::silent_cmd("taskkill")
            .args(["/F", "/IM", "nginx.exe"])
            .output()
            .await;
    }
    tracing::info!("nginx stopped");
    Ok(())
}
