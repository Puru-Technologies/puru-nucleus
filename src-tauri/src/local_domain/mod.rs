//! Friendly hostnames for the hospital server, so staff open the app by a
//! name instead of a raw IP. Two names per site, both derived from the
//! hospital code loaded from `NucleusConfig.hospital_code`:
//!
//!   `<code>.puru.local` — served by our built-in mDNS responder. Zero-touch
//!                         on any Windows 10+ / macOS / iOS / modern-Android
//!                         client on the same subnet: the OS resolves `.local`
//!                         over multicast (RFC 6762). Requires no router
//!                         config, no per-client change.
//!
//!   `<code>.puru`       — an *aspirational* name that only works when the
//!                         hospital's router has a matching DNS entry pointing
//!                         to the server IP. We can't publish this via mDNS
//!                         (mDNS is `.local` only), so nucleus generates a
//!                         printable one-page router-setup sheet for ops to
//!                         hand to the IT contact.
//!
//! Both names go into the SERVER's own hosts file so the server PC can also
//! use them (before mDNS resolves its own hostname on some Windows configs).

use std::io::Write;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::PathBuf;

/// Marker on our hosts lines so `ensure_hosts_entries` is idempotent and
/// `remove_hosts_entries` doesn't nuke unrelated entries the operator added.
const HOSTS_MARKER: &str = "# puru-nucleus";

/// Fleet-wide fallback name — always mapped to 127.0.0.1 on every server so
/// staff on the server PC can type `http://puru.local` regardless of which
/// hospital's install it is. Never advertised via mDNS (would collide across
/// multi-server sites); it's a server-local convenience only.
pub const FLEET_LOCAL_NAME: &str = "puru.local";

/// The two hostname flavours nucleus knows about. Kept in one place so the
/// dashboard, hosts writer, mDNS publisher and router sheet stay in sync.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LocalNames {
    /// e.g. "bth" — lowercased hospital code. Empty when the config hasn't
    /// been activated yet (setup wizard hasn't run); callers treat that as
    /// "no local names to publish".
    pub code: String,
    /// e.g. "bth.puru.local". Published via mDNS on the server's LAN IP.
    pub mdns_name: String,
    /// e.g. "bth.puru". Only resolves once ops adds a router DNS entry.
    pub router_name: String,
}

impl LocalNames {
    pub fn from_code(code: &str) -> Option<Self> {
        let code = code.trim().to_ascii_lowercase();
        if code.is_empty() {
            return None;
        }
        Some(Self {
            mdns_name: format!("{}.puru.local", code),
            router_name: format!("{}.puru", code),
            code,
        })
    }
}

fn hosts_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        // %SystemRoot% is the reliable location — hard-coding C: breaks on the
        // few Windows installs whose system drive is D:/E:.
        let sysroot = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
        PathBuf::from(format!(r"{}\System32\drivers\etc\hosts", sysroot))
    }
    #[cfg(not(target_os = "windows"))]
    {
        PathBuf::from("/etc/hosts")
    }
}

/// One-shot snapshot of everything the dashboard needs to render the
/// "Puru is live at" card.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LiveAtStatus {
    pub names: Option<LocalNames>,
    /// Server's primary LAN IPv4 — the address clients type when the domain
    /// name isn't set up.
    pub lan_ip: Option<String>,
    /// True when both hostname lines are in the server's hosts file with our
    /// marker. False when missing or if only one is present.
    pub hosts_configured: bool,
    /// Full path of the hosts file so the dashboard can point ops to it in
    /// error messages.
    pub hosts_path: String,
}

/// Snapshot for the dashboard card. `hosts_configured` requires ALL three
/// names to be present — the two hospital-specific ones and the fleet
/// `puru.local` fallback — so a partial write from a crashed setup step is
/// treated as "not configured" and the dashboard's auto-heal re-runs.
pub fn status(hospital_code: &str) -> LiveAtStatus {
    let names = LocalNames::from_code(hospital_code);
    let path = hosts_path();
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let fleet_ok = parse_line(&content, FLEET_LOCAL_NAME).0.is_some();
    let hosts_configured = match &names {
        Some(n) => {
            let mdns_ok = parse_line(&content, &n.mdns_name).0.is_some();
            let router_ok = parse_line(&content, &n.router_name).0.is_some();
            fleet_ok && mdns_ok && router_ok
        }
        // No hospital code yet — at minimum the fleet fallback should be there.
        None => fleet_ok,
    };
    LiveAtStatus {
        names,
        lan_ip: resolve_lan_ip(),
        hosts_configured,
        hosts_path: path.display().to_string(),
    }
}

/// Add the three hostname lines to the hosts file (each marked with
/// `# puru-nucleus`) if not already present:
///
///   `127.0.0.1  puru.local             # puru-nucleus` — fleet fallback
///   `127.0.0.1  <code>.puru.local      # puru-nucleus` — mDNS-matched name
///   `127.0.0.1  <code>.puru            # puru-nucleus` — router-DNS name
///
/// Operator hand-added lines for the same names are left alone. Returns the
/// count of lines actually written (0 = already in place).
///
/// The fleet `puru.local` is written even when hospital_code is empty (the
/// machine is un-activated but the server PC should still get the convenience
/// name); the two `<code>.` names are skipped in that case.
pub fn ensure_hosts_entries(hospital_code: &str) -> Result<usize, String> {
    let names = LocalNames::from_code(hospital_code);

    let path = hosts_path();
    let existing = std::fs::read_to_string(&path)
        .map_err(|e| format!("read {}: {}", path.display(), e))?;

    let mut updated = existing.clone();
    if !updated.ends_with('\n') && !updated.is_empty() {
        updated.push('\n');
    }

    // Order matters only for readability in the file: fleet first, then the
    // per-hospital names grouped together.
    let mut targets: Vec<String> = vec![FLEET_LOCAL_NAME.to_string()];
    if let Some(n) = &names {
        targets.push(n.mdns_name.clone());
        targets.push(n.router_name.clone());
    }

    let mut written = 0usize;
    for name in &targets {
        let (found, _) = parse_line(&updated, name);
        if found.is_none() {
            updated.push_str(&format!("127.0.0.1  {}  {}\n", name, HOSTS_MARKER));
            written += 1;
        }
    }
    if written == 0 {
        return Ok(0);
    }

    write_hosts(&path, &updated)?;
    Ok(written)
}

/// Remove only lines we added (marked `# puru-nucleus`) that match the fleet
/// fallback or the current hospital's names. Operator hand-added lines are
/// preserved. Called from uninstall / troubleshooting flows.
pub fn remove_hosts_entries(hospital_code: &str) -> Result<usize, String> {
    let names = LocalNames::from_code(hospital_code);
    let path = hosts_path();
    let existing = std::fs::read_to_string(&path)
        .map_err(|e| format!("read {}: {}", path.display(), e))?;

    let mut targets: Vec<&str> = vec![FLEET_LOCAL_NAME];
    if let Some(n) = &names {
        targets.push(n.mdns_name.as_str());
        targets.push(n.router_name.as_str());
    }

    let mut removed = 0usize;
    let mut out = String::with_capacity(existing.len());
    for line in existing.lines() {
        if is_our_line_for(line, &targets) {
            removed += 1;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if removed == 0 {
        return Ok(0);
    }
    write_hosts(&path, &out)?;
    Ok(removed)
}

fn write_hosts(path: &PathBuf, content: &str) -> Result<(), String> {
    // Direct in-place write — this file has a well-known name and ACL that
    // ships with Windows; a tempfile+rename would break it. Requires admin;
    // caller (setup wizard / auto-heal on dashboard) has already elevated.
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|e| {
            format!(
                "open {} for write: {} (needs administrator)",
                path.display(),
                e
            )
        })?;
    file.write_all(content.as_bytes())
        .map_err(|e| format!("write {}: {}", path.display(), e))?;
    Ok(())
}

/// Extract the target IP of the first line containing `hostname` (case-
/// insensitive), plus whether it's marked as ours. Skips comment lines.
fn parse_line(content: &str, hostname: &str) -> (Option<String>, bool) {
    let want = hostname.to_ascii_lowercase();
    for line in content.lines() {
        let raw = line.trim();
        if raw.is_empty() || raw.starts_with('#') {
            continue;
        }
        let (data, comment) = match raw.split_once('#') {
            Some((d, c)) => (d.trim(), c.trim()),
            None => (raw, ""),
        };
        let mut tokens = data.split_whitespace();
        let Some(ip) = tokens.next() else { continue };
        if tokens.any(|t| t.eq_ignore_ascii_case(&want)) {
            let managed = comment.contains("puru-nucleus");
            return (Some(ip.to_string()), managed);
        }
    }
    (None, false)
}

/// True when `line` is a nucleus-managed line whose hostname matches one of
/// the `targets`. Prevents `remove_hosts_entries` from touching operator-
/// added lines even if they name the same host.
fn is_our_line_for(line: &str, targets: &[&str]) -> bool {
    let raw = line.trim();
    if !raw.contains("puru-nucleus") {
        return false;
    }
    let lower = raw.to_ascii_lowercase();
    targets.iter().any(|t| lower.contains(&t.to_ascii_lowercase()))
}

/// Best-effort primary LAN IPv4 of the server box. Cross-platform trick:
/// ask the kernel "which local address would you route a UDP packet to a
/// public target through?" — nothing is actually sent (UDP `connect` is
/// stateless), but the OS runs the route lookup, which is more reliable
/// than enumerating interfaces (Docker bridges, VirtualBox host-only NICs,
/// VPN adapters all trip up naive enumeration).
pub fn resolve_lan_ip() -> Option<String> {
    for target in ["8.8.8.8:80", "1.1.1.1:80"] {
        let Ok(addr) = target.parse::<SocketAddr>() else {
            continue;
        };
        let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else {
            continue;
        };
        if socket.connect(addr).is_err() {
            continue;
        }
        if let Ok(local) = socket.local_addr() {
            if let IpAddr::V4(v4) = local.ip() {
                if !v4.is_unspecified() && !v4.is_loopback() {
                    return Some(v4.to_string());
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_code_normalizes_case_and_whitespace() {
        let n = LocalNames::from_code("  BTH  ").unwrap();
        assert_eq!(n.code, "bth");
        assert_eq!(n.mdns_name, "bth.puru.local");
        assert_eq!(n.router_name, "bth.puru");
    }

    #[test]
    fn from_code_rejects_blank() {
        assert!(LocalNames::from_code("").is_none());
        assert!(LocalNames::from_code("   ").is_none());
    }

    #[test]
    fn parse_finds_our_line() {
        let hosts = "\
            127.0.0.1 localhost\n\
            127.0.0.1  bth.puru.local  # puru-nucleus\n\
        ";
        let (ip, managed) = parse_line(hosts, "bth.puru.local");
        assert_eq!(ip.as_deref(), Some("127.0.0.1"));
        assert!(managed);
    }

    #[test]
    fn parse_recognises_operator_line() {
        let (ip, managed) = parse_line("192.168.1.50 bth.puru\n", "bth.puru");
        assert_eq!(ip.as_deref(), Some("192.168.1.50"));
        assert!(!managed);
    }

    #[test]
    fn parse_ignores_commented_line() {
        let (ip, _) = parse_line("# 127.0.0.1 bth.puru.local\n", "bth.puru.local");
        assert!(ip.is_none());
    }

    #[test]
    fn is_our_line_only_matches_marked_and_targeted() {
        let targets = ["bth.puru.local", "bth.puru"];
        assert!(is_our_line_for(
            "127.0.0.1 bth.puru.local # puru-nucleus",
            &targets
        ));
        assert!(!is_our_line_for("127.0.0.1 bth.puru.local", &targets)); // no marker
        assert!(!is_our_line_for(
            "127.0.0.1 other.puru.local # puru-nucleus",
            &targets
        )); // marker but wrong host
    }
}
