//! At-rest encryption for the GCP service-account key.
//!
//! The key JSON is stored as AES-256-GCM ciphertext in the **same** file
//! (`gcs-credentials.json`) with no marker — it just looks like opaque bytes,
//! so nothing signals that it's encrypted. The AES key is derived from this
//! machine's hardware fingerprint (recomputed live) mixed with an embedded
//! salt, so only nucleus running on this hardware can decrypt it; a copied file
//! won't decrypt elsewhere.
//!
//! Known ceiling: the fingerprint is also persisted in plaintext elsewhere on
//! the box (local license, Firestore), so the embedded salt is the real secret
//! and a determined attacker with disk + binary could recompute the key. This
//! defeats casual file theft, not a determined on-box attacker.
//!
//! Decryption also tries the fingerprints of the box's other network cards, so
//! a change in which card counts as primary doesn't lock the token server out
//! of the key; the file is then re-encrypted for the current hardware. The
//! fingerprint saved in the license is never tried: a copied nucleus folder
//! must not unlock the key on another machine.

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::error::NucleusError;

/// Salt embedded in the binary and mixed into key derivation. The on-disk
/// fingerprint alone is not enough to derive the key — an attacker also needs
/// this binary. Changing it invalidates all existing ciphertext (the operator
/// then re-enters credentials). Any length is fine; it is hashed.
// Keeps the pre-rename "puru-dc" name on purpose: changing it would make every
// key already encrypted on disk unreadable.
const EMBEDDED_SALT: &[u8] = b"puru-dc::gcs-key-at-rest::v1::e7b1c9a4f2d6::do-not-share";

const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

/// Derive the 32-byte AES key: `SHA-256(live_hardware_fingerprint || SALT)`.
fn derive_key() -> Result<[u8; 32], NucleusError> {
    let fp = crate::licensing::fingerprint::get_cached_fingerprint()?;
    Ok(key_for(&fp))
}

fn key_for(fp: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(fp.as_bytes());
    hasher.update(EMBEDDED_SALT);
    let digest = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest);
    key
}

fn cipher() -> Result<Aes256Gcm, NucleusError> {
    let key = derive_key()?;
    Aes256Gcm::new_from_slice(&key)
        .map_err(|e| NucleusError::Internal(format!("cipher init failed: {}", e)))
}

/// Encrypt plaintext → `nonce(12) || ciphertext+tag(16)`.
pub fn encrypt(plaintext: &[u8]) -> Result<Vec<u8>, NucleusError> {
    let c = cipher()?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = c
        .encrypt(nonce, plaintext)
        .map_err(|e| NucleusError::Internal(format!("encrypt failed: {}", e)))?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypt `nonce(12) || ciphertext+tag(16)` → plaintext. Errors if the data
/// isn't our ciphertext or the key (hardware) differs.
pub fn decrypt(data: &[u8]) -> Result<Vec<u8>, NucleusError> {
    decrypt_any(data).map(|(plain, _)| plain)
}

/// Try the live key, then the fallback fingerprints. The bool is true when a
/// fallback matched (the file should be re-encrypted for this hardware).
fn decrypt_any(data: &[u8]) -> Result<(Vec<u8>, bool), NucleusError> {
    if data.len() < NONCE_LEN + TAG_LEN {
        return Err(NucleusError::Internal("ciphertext too short".into()));
    }
    let (nonce_bytes, ct) = data.split_at(NONCE_LEN);
    let nonce = Nonce::from_slice(nonce_bytes);
    if let Ok(plain) = cipher()?.decrypt(nonce, ct) {
        return Ok((plain, false));
    }
    for fp in crate::licensing::fingerprint::candidate_fingerprints().iter().skip(1) {
        let c = Aes256Gcm::new_from_slice(&key_for(fp))
            .map_err(|e| NucleusError::Internal(format!("cipher init failed: {}", e)))?;
        if let Ok(plain) = c.decrypt(nonce, ct) {
            return Ok((plain, true));
        }
    }
    Err(NucleusError::Internal(
        "the cloud key file can't be read on this machine (it was saved on different hardware, or is damaged). \
         Run `sudo puru-nucleus setup` and enter a new onboarding code to download it again."
            .into(),
    ))
}

/// Encrypt `content` and write the ciphertext to `path` (creating parent dirs).
/// Used when the operator imports/pastes the service-account JSON.
pub fn write_encrypted(path: &str, content: &[u8]) -> Result<(), NucleusError> {
    if let Some(parent) = std::path::Path::new(path).parent() {
        std::fs::create_dir_all(parent).map_err(NucleusError::Io)?;
    }
    let ct = encrypt(content)?;
    std::fs::write(path, ct).map_err(NucleusError::Io)?;
    Ok(())
}

/// Read the encrypted credentials file at `path` and return the decrypted JSON
/// string. Every consumer of the SA key goes through here instead of reading
/// the file directly.
pub fn read_decrypted(path: &str) -> Result<String, NucleusError> {
    let data = std::fs::read(path).map_err(NucleusError::Io)?;
    let (plain, fallback) = decrypt_any(&data)?;
    if fallback {
        // Best effort: bind the file to the current hardware so the next read
        // takes the fast path. Readers still work if this fails.
        match write_encrypted(path, &plain) {
            Ok(()) => tracing::info!("Credentials at {} re-encrypted for this machine's current hardware", path),
            Err(e) => tracing::warn!("Couldn't re-encrypt credentials at {}: {}", path, e),
        }
    }
    String::from_utf8(plain)
        .map_err(|e| NucleusError::Internal(format!("decrypted credentials not UTF-8: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        // Requires a real hardware fingerprint; skip where unavailable (CI).
        if derive_key().is_err() {
            return;
        }
        let pt = br#"{"type":"service_account","project_id":"x"}"#;
        let ct = encrypt(pt).unwrap();
        assert_ne!(&ct[..], &pt[..]);
        assert_eq!(decrypt(&ct).unwrap(), pt);
    }

    #[test]
    fn decrypt_rejects_garbage() {
        if derive_key().is_err() {
            return;
        }
        assert!(decrypt(b"not-our-ciphertext-but-long-enough-bytes").is_err());
    }
}
