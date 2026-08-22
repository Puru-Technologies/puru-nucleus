//! View / edit the puru-auth centralized `puru_config` table via its REST API
//! (`/config/nucleus/**`). Auth is via `X-Service-Key` — the same header every
//! backend service uses to read `/config/shared`. We look the key up in the
//! local MySQL each call rather than caching it in nucleus config, so a
//! rotation via the UI takes effect on the very next request.

use crate::config::{self, NucleusConfig};
use mysql_async::prelude::*;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// One row of `puru_auth.puru_config` as returned by the auth API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfigEntry {
    pub id: Option<i64>,
    #[serde(rename = "configKey")]
    pub config_key: String,
    #[serde(rename = "configValue", default)]
    pub config_value: Option<String>,
    #[serde(rename = "configType", default)]
    pub config_type: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Request body for update/create — matches ConfigController.ConfigUpdateRequest.
#[derive(Debug, Clone, Serialize)]
pub struct ConfigUpdateBody<'a> {
    pub key: &'a str,
    pub value: &'a str,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub config_type: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<&'a str>,
}

/// Auth base URL — resolved from `puru_config.port.auth` if present, otherwise
/// the well-known 8080 on the same host as MySQL (nucleus + MySQL + auth all
/// co-locate on the hospital box in native/single-node Docker deployments).
async fn resolve_auth_base_url(config: &NucleusConfig) -> String {
    let host = if config.mysql_host.is_empty() {
        "127.0.0.1".to_string()
    } else {
        config.mysql_host.clone()
    };
    let port = load_int_config(config, "port.auth").await.unwrap_or(8080);
    format!("http://{}:{}", host, port)
}

/// Read the service key from the local MySQL `puru_config` table. Every backend
/// service (has, pacs, neon, …) does the same lookup at boot to authenticate
/// against auth's `/config/shared`, so this is the same value they use.
async fn load_service_key(config: &NucleusConfig) -> Result<String, String> {
    let value = load_string_config(config, "puru.service.key")
        .await
        .ok_or_else(|| {
            "puru.service.key is not set in puru_auth.puru_config. Run setup or seed \
             the config table first."
                .to_string()
        })?;
    if value.is_empty() {
        return Err("puru.service.key is empty in puru_auth.puru_config".to_string());
    }
    Ok(value)
}

async fn load_string_config(config: &NucleusConfig, key: &str) -> Option<String> {
    let pool = create_mysql_pool(config);
    let mut conn = pool.get_conn().await.ok()?;
    let row: Option<(Option<String>,)> = conn
        .exec_first(
            "SELECT config_value FROM puru_auth.puru_config WHERE config_key = ?",
            (key,),
        )
        .await
        .ok()
        .flatten();
    let _ = pool.disconnect().await;
    row.and_then(|(v,)| v)
}

async fn load_int_config(config: &NucleusConfig, key: &str) -> Option<u16> {
    load_string_config(config, key)
        .await
        .and_then(|s| s.trim().parse::<u16>().ok())
}

fn create_mysql_pool(config: &NucleusConfig) -> mysql_async::Pool {
    let opts = mysql_async::OptsBuilder::default()
        .ip_or_hostname(config.mysql_host.clone())
        .tcp_port(config.mysql_port)
        .user(Some(config.mysql_user.clone()))
        .pass(Some(config.mysql_password.clone()));
    mysql_async::Pool::new(opts)
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client init failed: {}", e))
}

/// GET /config/nucleus/all — every row in puru_config.
pub async fn list_all() -> Result<Vec<AuthConfigEntry>, String> {
    let cfg = config::load_config().map_err(|e| e.user_message())?;
    let base = resolve_auth_base_url(&cfg).await;
    let key = load_service_key(&cfg).await?;

    let resp = http_client()?
        .get(format!("{}/config/nucleus/all", base))
        .header("X-Service-Key", &key)
        .send()
        .await
        .map_err(|e| format!("Could not reach puru-auth at {}: {}", base, e))?;

    if !resp.status().is_success() {
        return Err(format!(
            "puru-auth returned {} for /config/nucleus/all",
            resp.status()
        ));
    }
    resp.json::<Vec<AuthConfigEntry>>()
        .await
        .map_err(|e| format!("puru-auth response parse: {}", e))
}

/// PUT /config/nucleus/{key} — update value (auto-upserts on the server side
/// via ConfigService.updateConfig, so the same call works for create).
pub async fn update(key: String, value: String, config_type: Option<String>, category: Option<String>) -> Result<(), String> {
    let key_trimmed = key.trim();
    if key_trimmed.is_empty() {
        return Err("Config key is required".to_string());
    }

    let cfg = config::load_config().map_err(|e| e.user_message())?;
    let base = resolve_auth_base_url(&cfg).await;
    let svc_key = load_service_key(&cfg).await?;

    let body = ConfigUpdateBody {
        key: key_trimmed,
        value: &value,
        config_type: config_type.as_deref(),
        category: category.as_deref(),
        description: None,
    };

    let resp = http_client()?
        .put(format!("{}/config/nucleus/{}", base, urlencoding::encode(key_trimmed)))
        .header("X-Service-Key", &svc_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Could not reach puru-auth at {}: {}", base, e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("puru-auth returned {} — {}", status, text));
    }
    Ok(())
}

/// DELETE /config/nucleus/{key}.
pub async fn delete(key: String) -> Result<(), String> {
    let key_trimmed = key.trim();
    if key_trimmed.is_empty() {
        return Err("Config key is required".to_string());
    }
    let cfg = config::load_config().map_err(|e| e.user_message())?;
    let base = resolve_auth_base_url(&cfg).await;
    let svc_key = load_service_key(&cfg).await?;

    let resp = http_client()?
        .delete(format!("{}/config/nucleus/{}", base, urlencoding::encode(key_trimmed)))
        .header("X-Service-Key", &svc_key)
        .send()
        .await
        .map_err(|e| format!("Could not reach puru-auth at {}: {}", base, e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("puru-auth returned {} — {}", status, text));
    }
    Ok(())
}

/// POST /config/nucleus/refresh — auth clears any cached frontend config so a
/// UI refresh sees the change immediately. Kept as a separate step because
/// individual updates already invalidate cache server-side.
pub async fn refresh() -> Result<(), String> {
    let cfg = config::load_config().map_err(|e| e.user_message())?;
    let base = resolve_auth_base_url(&cfg).await;
    let svc_key = load_service_key(&cfg).await?;

    let resp = http_client()?
        .post(format!("{}/config/nucleus/refresh", base))
        .header("X-Service-Key", &svc_key)
        .send()
        .await
        .map_err(|e| format!("Could not reach puru-auth at {}: {}", base, e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("puru-auth returned {} — {}", status, text));
    }
    Ok(())
}
