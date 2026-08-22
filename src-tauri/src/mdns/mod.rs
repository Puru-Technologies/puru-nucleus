//! mDNS responder — advertises `<hospital_code>.puru.local` on the LAN so
//! clients on the same subnet resolve the friendly hostname without any
//! per-client configuration. Uses the pure-Rust `mdns-sd` crate; runs as a
//! background task owned by main.rs.
//!
//! Design notes:
//! - We publish a single A record via a `_http._tcp` service entry — the
//!   `mdns-sd` API doesn't expose bare host announcements, so wrapping it as
//!   a fake "http on port 80" service is the standard trick. Browsers only
//!   care about the A record for name resolution; the service metadata is
//!   inert.
//! - The advertised IP is the server's LAN address (`resolve_lan_ip`), NOT
//!   loopback — clients on other machines need a reachable IP, not 127.0.0.1.
//! - Restart-on-change: if the LAN IP changes (DHCP renew, cable swap) we
//!   re-register the record. Poll interval is deliberately slow (60s) since
//!   IP flips are rare and multicast traffic isn't free.
//! - Shutdown is via a broadcast channel so the tokio task exits cleanly and
//!   `mdns-sd` sends a goodbye packet (clients evict the cached record faster).

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};

use mdns_sd::{ServiceDaemon, ServiceInfo};

use crate::local_domain::{resolve_lan_ip, LocalNames};

/// Fake service type — required by mdns-sd but not used for discovery. The
/// value that actually matters is the host record `<code>.puru.local`.
const SERVICE_TYPE: &str = "_http._tcp.local.";

/// Handle to the running responder so the app can stop it (and to make the
/// state observable from a diagnostic command). Cheap to Clone.
#[derive(Clone)]
pub struct MdnsHandle {
    inner: Arc<Mutex<Option<RunningState>>>,
}

struct RunningState {
    daemon: ServiceDaemon,
    /// Fully-qualified service name we registered — needed to unregister on
    /// shutdown or when the IP changes and we re-register.
    registered_fullname: String,
    /// IP the current advertisement is bound to, so we know when to refresh.
    advertised_ip: String,
    /// Advertised hostname (e.g. `bth.puru.local`) — kept for the status API.
    hostname: String,
    /// Signals the watcher loop to exit.
    _shutdown: broadcast::Sender<()>,
}

/// Snapshot of what the responder is currently publishing (or None if idle).
#[derive(Debug, Clone, serde::Serialize)]
pub struct MdnsStatus {
    pub running: bool,
    pub hostname: Option<String>,
    pub advertised_ip: Option<String>,
}

impl MdnsHandle {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn status(&self) -> MdnsStatus {
        let guard = self.inner.lock().await;
        match guard.as_ref() {
            Some(s) => MdnsStatus {
                running: true,
                hostname: Some(s.hostname.clone()),
                advertised_ip: Some(s.advertised_ip.clone()),
            },
            None => MdnsStatus {
                running: false,
                hostname: None,
                advertised_ip: None,
            },
        }
    }

    /// Start (or restart) the responder for the given hospital code. Idempotent
    /// on already-correct state; a code / IP change triggers a re-register.
    pub async fn start(&self, hospital_code: &str) -> Result<MdnsStatus, String> {
        let Some(names) = LocalNames::from_code(hospital_code) else {
            return Err("Hospital code not set — activate this machine first.".into());
        };
        let ip = resolve_lan_ip()
            .ok_or_else(|| "Could not determine the server's LAN IP.".to_string())?;

        // If we're already advertising the same (hostname, ip), no-op — the
        // dashboard hitting this every load shouldn't churn the daemon.
        {
            let guard = self.inner.lock().await;
            if let Some(s) = guard.as_ref() {
                if s.hostname == names.mdns_name && s.advertised_ip == ip {
                    return Ok(MdnsStatus {
                        running: true,
                        hostname: Some(names.mdns_name),
                        advertised_ip: Some(ip),
                    });
                }
            }
        }

        // Anything else — stop first, then start fresh.
        self.stop_inner().await;

        let daemon = ServiceDaemon::new()
            .map_err(|e| format!("mdns daemon init failed: {}", e))?;

        // Register: service type stays fixed, instance name = hospital code
        // so `bth.puru._http._tcp.local.` is unique across sites (though we
        // only care about the host record). Host = `<code>.puru` (mdns-sd
        // appends `.local.` internally). Port 80 is nominal.
        let service_info = ServiceInfo::new(
            SERVICE_TYPE,
            &format!("{}.puru", names.code),
            &format!("{}.puru.local.", names.code),
            ip.as_str(),
            80,
            None,
        )
        .map_err(|e| format!("mdns ServiceInfo build failed: {}", e))?;

        let fullname = service_info.get_fullname().to_string();
        daemon
            .register(service_info)
            .map_err(|e| format!("mdns register failed: {}", e))?;

        // Background watcher: if the LAN IP flips (DHCP renew, cable swap),
        // re-register so clients don't keep hitting the stale address.
        let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);
        let daemon_clone = daemon.clone();
        let handle_clone = self.inner.clone();
        let hostname_clone = names.mdns_name.clone();
        let code_clone = names.code.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(60));
            // First tick fires immediately — skip it, we already registered.
            ticker.tick().await;
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => break,
                    _ = ticker.tick() => {
                        let Some(new_ip) = resolve_lan_ip() else { continue };
                        let mut guard = handle_clone.lock().await;
                        let Some(state) = guard.as_mut() else { break };
                        if state.advertised_ip == new_ip { continue }
                        // IP changed — unregister old, register new. If any
                        // step fails, log and keep polling; the responder
                        // stays on the old (now-stale) address until we
                        // succeed rather than dropping the ad entirely.
                        let _ = daemon_clone.unregister(&state.registered_fullname);
                        let svc = match ServiceInfo::new(
                            SERVICE_TYPE,
                            &format!("{}.puru", code_clone),
                            &format!("{}.puru.local.", code_clone),
                            new_ip.as_str(),
                            80,
                            None,
                        ) {
                            Ok(s) => s,
                            Err(e) => {
                                tracing::warn!("mdns: rebuild ServiceInfo on IP change failed: {}", e);
                                continue;
                            }
                        };
                        let new_full = svc.get_fullname().to_string();
                        if let Err(e) = daemon_clone.register(svc) {
                            tracing::warn!("mdns: re-register on IP change failed: {}", e);
                            continue;
                        }
                        tracing::info!(
                            "mdns: re-advertised {} on new LAN IP {} (was {})",
                            hostname_clone, new_ip, state.advertised_ip
                        );
                        state.registered_fullname = new_full;
                        state.advertised_ip = new_ip;
                    }
                }
            }
        });

        let mut guard = self.inner.lock().await;
        *guard = Some(RunningState {
            daemon,
            registered_fullname: fullname,
            advertised_ip: ip.clone(),
            hostname: names.mdns_name.clone(),
            _shutdown: shutdown_tx,
        });

        Ok(MdnsStatus {
            running: true,
            hostname: Some(names.mdns_name),
            advertised_ip: Some(ip),
        })
    }

    /// Stop the responder. `mdns-sd` sends a goodbye packet on unregister so
    /// clients evict the cached A record within a few seconds instead of
    /// waiting out the TTL.
    pub async fn stop(&self) {
        self.stop_inner().await;
    }

    async fn stop_inner(&self) {
        let mut guard = self.inner.lock().await;
        if let Some(state) = guard.take() {
            // Drops the shutdown Sender → all receivers get RecvError::Closed,
            // watcher loop exits.
            let _ = state.daemon.unregister(&state.registered_fullname);
            let _ = state.daemon.shutdown();
        }
    }
}

impl Default for MdnsHandle {
    fn default() -> Self {
        Self::new()
    }
}
