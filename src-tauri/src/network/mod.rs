//! Internet connectivity and speed test utilities.
//!
//! - `check_network()` — lightweight connectivity + latency check (hits Google generate_204)
//!   plus GCP Healthcare API reachability (critical for DICOM uploads)
//! - `run_speed_test()` — full download/upload speed test via Cloudflare

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Instant;

use crate::error::NucleusError;

/// GCP Healthcare API base URL — used by puru-pacs for DICOMweb STOW-RS uploads.
const GCP_HEALTHCARE_URL: &str = "https://healthcare.googleapis.com/";

/// Quick connectivity check result (polled every 60s on dashboard).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStatus {
    pub connected: bool,
    pub latency_ms: Option<u64>,
    /// Whether GCP Healthcare API endpoint is reachable (critical for DICOM uploads).
    pub gcp_reachable: bool,
    pub gcp_latency_ms: Option<u64>,
    pub checked_at: DateTime<Utc>,
}

/// Full speed test result (on-demand).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeedTestResult {
    pub connected: bool,
    pub latency_ms: Option<u64>,
    pub gcp_reachable: bool,
    pub gcp_latency_ms: Option<u64>,
    pub download_mbps: Option<f64>,
    pub upload_mbps: Option<f64>,
    pub download_bytes: u64,
    pub upload_bytes: u64,
    pub duration_ms: u64,
    pub tested_at: DateTime<Utc>,
}

/// Check reachability of GCP Healthcare API.
///
/// Sends a GET to `https://healthcare.googleapis.com/` — returns 200 with API
/// discovery info (no auth required for the root endpoint). Any HTTP response
/// means the endpoint is reachable; only network/timeout errors mean it's down.
async fn check_gcp_healthcare(client: &reqwest::Client) -> (bool, Option<u64>) {
    let start = Instant::now();
    match client.get(GCP_HEALTHCARE_URL).send().await {
        Ok(_) => {
            // Any response (200, 401, 404) means the endpoint is reachable
            (true, Some(start.elapsed().as_millis() as u64))
        }
        Err(_) => (false, None),
    }
}

/// Lightweight connectivity + latency check.
///
/// 1. Sends GET to `https://www.google.com/generate_204` for general internet
/// 2. Sends GET to `https://healthcare.googleapis.com/` for GCP Healthcare API
///
/// Network failures return `Ok(NetworkStatus { connected: false })` because
/// "offline" is valid data, not an error.
pub async fn check_network() -> Result<NetworkStatus, NucleusError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .no_proxy()
        .build()
        .map_err(|e| NucleusError::Internal(format!("Failed to build HTTP client: {}", e)))?;

    // ── General internet check ──────────────────────────────────────────────
    let start = Instant::now();
    let result = client
        .get("https://www.google.com/generate_204")
        .send()
        .await;

    let (connected, latency_ms) = match result {
        Ok(_) => (true, Some(start.elapsed().as_millis() as u64)),
        Err(_) => (false, None),
    };

    // ── GCP Healthcare API check ────────────────────────────────────────────
    let (gcp_reachable, gcp_latency_ms) = if connected {
        check_gcp_healthcare(&client).await
    } else {
        (false, None)
    };

    Ok(NetworkStatus {
        connected,
        latency_ms,
        gcp_reachable,
        gcp_latency_ms,
        checked_at: Utc::now(),
    })
}

/// Full speed test: latency + GCP check + download + upload throughput.
///
/// 1. Latency: GET google.com/generate_204
/// 2. GCP Healthcare API: GET healthcare.googleapis.com/
/// 3. Download: GET speed.cloudflare.com/__down?bytes=1000000 (1 MB), measure throughput
/// 4. Upload: POST speed.cloudflare.com/__up with 500 KB body, measure throughput
///
/// Returns `Ok(SpeedTestResult { connected: false, ... })` if offline.
pub async fn run_speed_test() -> Result<SpeedTestResult, NucleusError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .no_proxy()
        .build()
        .map_err(|e| NucleusError::Internal(format!("Failed to build HTTP client: {}", e)))?;

    let overall_start = Instant::now();

    // ── 1. Latency check ────────────────────────────────────────────────────
    let ping_start = Instant::now();
    let ping_result = client
        .get("https://www.google.com/generate_204")
        .send()
        .await;

    let (connected, latency_ms) = match ping_result {
        Ok(_) => (true, Some(ping_start.elapsed().as_millis() as u64)),
        Err(_) => {
            return Ok(SpeedTestResult {
                connected: false,
                latency_ms: None,
                gcp_reachable: false,
                gcp_latency_ms: None,
                download_mbps: None,
                upload_mbps: None,
                download_bytes: 0,
                upload_bytes: 0,
                duration_ms: overall_start.elapsed().as_millis() as u64,
                tested_at: Utc::now(),
            });
        }
    };

    // ── 2. GCP Healthcare API check ─────────────────────────────────────────
    let (gcp_reachable, gcp_latency_ms) = check_gcp_healthcare(&client).await;

    // ── 3. Download test (1 MB) ─────────────────────────────────────────────
    let download_bytes_target: u64 = 1_000_000;
    let dl_start = Instant::now();
    let dl_result = client
        .get(format!(
            "https://speed.cloudflare.com/__down?bytes={}",
            download_bytes_target
        ))
        .send()
        .await;

    let (download_mbps, download_bytes) = match dl_result {
        Ok(resp) => {
            let bytes = resp.bytes().await.unwrap_or_default();
            let elapsed_secs = dl_start.elapsed().as_secs_f64();
            let dl_bytes = bytes.len() as u64;
            if elapsed_secs > 0.0 {
                let mbps = (dl_bytes as f64 * 8.0) / (elapsed_secs * 1_000_000.0);
                (Some(mbps), dl_bytes)
            } else {
                (None, dl_bytes)
            }
        }
        Err(_) => (None, 0),
    };

    // ── 4. Upload test (500 KB) ─────────────────────────────────────────────
    let upload_payload = vec![0u8; 500_000];
    let upload_size = upload_payload.len() as u64;
    let ul_start = Instant::now();
    let ul_result = client
        .post("https://speed.cloudflare.com/__up")
        .body(upload_payload)
        .send()
        .await;

    let (upload_mbps, upload_bytes) = match ul_result {
        Ok(_resp) => {
            let elapsed_secs = ul_start.elapsed().as_secs_f64();
            if elapsed_secs > 0.0 {
                let mbps = (upload_size as f64 * 8.0) / (elapsed_secs * 1_000_000.0);
                (Some(mbps), upload_size)
            } else {
                (None, upload_size)
            }
        }
        Err(_) => (None, 0),
    };

    Ok(SpeedTestResult {
        connected,
        latency_ms,
        gcp_reachable,
        gcp_latency_ms,
        download_mbps,
        upload_mbps,
        download_bytes,
        upload_bytes,
        duration_ms: overall_start.elapsed().as_millis() as u64,
        tested_at: Utc::now(),
    })
}
