//! Hardware fingerprint generation for machine-bound licensing.
//!
//! Algorithm (matches puru-pixel Python implementation):
//! ```text
//! SHA-256( "mac_address:os_machine_id:os_name" )
//! ```

use sha2::{Digest, Sha256};
use std::sync::OnceLock;

use crate::error::NucleusError;

/// Process-lifetime cache for the hardware fingerprint.
static FINGERPRINT_CACHE: OnceLock<String> = OnceLock::new();

/// Get the primary NIC MAC address formatted as `aa:bb:cc:dd:ee:ff`.
pub fn get_mac_address_string() -> Result<String, NucleusError> {
    let mac = mac_address::get_mac_address()
        .map_err(|e| NucleusError::InvalidConfig(format!("Failed to get MAC address: {}", e)))?
        .ok_or_else(|| NucleusError::InvalidConfig("No MAC address found".into()))?;

    Ok(mac.to_string().to_lowercase())
}

/// Get the OS machine ID (platform-specific).
///
/// - macOS: IOPlatformUUID via `ioreg`
/// - Linux: `/etc/machine-id`
/// - Windows: Registry `MachineGuid`
pub fn get_machine_id() -> Result<String, NucleusError> {
    #[cfg(target_os = "macos")]
    {
        get_machine_id_macos()
    }
    #[cfg(target_os = "linux")]
    {
        get_machine_id_linux()
    }
    #[cfg(target_os = "windows")]
    {
        get_machine_id_windows()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Err(NucleusError::InvalidConfig(
            "Unsupported platform for machine ID".into(),
        ))
    }
}

#[cfg(target_os = "macos")]
fn get_machine_id_macos() -> Result<String, NucleusError> {
    let output = std::process::Command::new("ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .map_err(|e| NucleusError::InvalidConfig(format!("Failed to run ioreg: {}", e)))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.contains("IOPlatformUUID") {
            // Format: "IOPlatformUUID" = "XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX"
            if let Some(uuid) = line.split('"').nth(3) {
                return Ok(uuid.to_string());
            }
        }
    }

    Err(NucleusError::InvalidConfig(
        "Could not find IOPlatformUUID in ioreg output".into(),
    ))
}

#[cfg(target_os = "linux")]
fn get_machine_id_linux() -> Result<String, NucleusError> {
    std::fs::read_to_string("/etc/machine-id")
        .map(|s| s.trim().to_string())
        .map_err(|e| NucleusError::InvalidConfig(format!("Failed to read /etc/machine-id: {}", e)))
}

#[cfg(target_os = "windows")]
fn get_machine_id_windows() -> Result<String, NucleusError> {
    let output = std::process::Command::new("reg")
        .args([
            "query",
            r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Cryptography",
            "/v",
            "MachineGuid",
        ])
        .output()
        .map_err(|e| {
            NucleusError::InvalidConfig(format!("Failed to query Windows registry: {}", e))
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.contains("MachineGuid") {
            // Format: "    MachineGuid    REG_SZ    xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
            if let Some(guid) = line.split_whitespace().last() {
                return Ok(guid.to_string());
            }
        }
    }

    Err(NucleusError::InvalidConfig(
        "Could not find MachineGuid in Windows registry".into(),
    ))
}

/// Get the OS name matching Python's `platform.system()` output.
pub fn get_os_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "Darwin",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
}

/// Generate a hardware fingerprint: `SHA-256("mac:machine_id:os_name")`.
///
/// Returns a 64-character lowercase hex string.
pub fn generate_fingerprint() -> Result<String, NucleusError> {
    let mac = get_mac_address_string()?;
    let machine_id = get_machine_id()?;
    let os_name = get_os_name();

    let input = format!("{}:{}:{}", mac, machine_id, os_name);
    let hash = Sha256::digest(input.as_bytes());

    Ok(format!("{:x}", hash))
}

/// Get the hardware fingerprint with process-lifetime caching.
///
/// The first call computes and caches the fingerprint; subsequent calls
/// return the cached value.
pub fn get_cached_fingerprint() -> Result<String, NucleusError> {
    if let Some(cached) = FINGERPRINT_CACHE.get() {
        return Ok(cached.clone());
    }

    let fp = generate_fingerprint()?;
    // Another thread may have raced us; that's fine — OnceLock handles it.
    let _ = FINGERPRINT_CACHE.set(fp.clone());
    Ok(fp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_os_name() {
        let name = get_os_name();
        assert!(
            ["Darwin", "Windows", "Linux"].contains(&name),
            "Unexpected OS name: {}",
            name
        );
    }

    #[test]
    fn test_fingerprint_is_64_char_hex() {
        // This test requires a real MAC address and machine ID,
        // so it will only pass on a real machine (not in CI containers without NICs).
        if let Ok(fp) = generate_fingerprint() {
            assert_eq!(fp.len(), 64, "Fingerprint should be 64 hex chars");
            assert!(
                fp.chars().all(|c| c.is_ascii_hexdigit()),
                "Fingerprint should be hex: {}",
                fp
            );
        }
    }

    #[test]
    fn test_fingerprint_deterministic() {
        if let Ok(fp1) = generate_fingerprint() {
            let fp2 = generate_fingerprint().unwrap();
            assert_eq!(fp1, fp2, "Fingerprint should be deterministic");
        }
    }

    #[test]
    fn test_cached_fingerprint_matches() {
        if let Ok(fp) = generate_fingerprint() {
            let cached = get_cached_fingerprint().unwrap();
            assert_eq!(fp, cached);
        }
    }
}
