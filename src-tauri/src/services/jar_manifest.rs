//! Per-service JAR manifest — the fail-safe journal for native-mode updates.
//!
//! Instead of mutating a single `{service}.jar` file (which Windows keeps
//! memory-mapped even after `taskkill /F`, blocking the swap), each downloaded
//! JAR lands under a timestamped filename and a tiny JSON manifest tracks which
//! one is live. Because the manifest is written atomically (tmp + rename of the
//! JSON — never memory-mapped) and the JAR files themselves are append-only,
//! every crash point leaves recoverable state. On the next `start_service()`
//! call, `recover_on_start()` reconciles the manifest with what's on disk and
//! completes any in-flight promotion.
//!
//! Layout (per service, under `jars_dir`):
//! ```text
//!   auth_20260822-143022-abc1234.jar    # active
//!   auth_20260821-091510-def4567.jar    # previous (history[0])
//!   auth_20260820-100000-ghi7890.jar    # older
//!   auth.manifest.json                  # this manifest — source of truth
//! ```
//!
//! The manifest has three roles:
//! - `active`  — the JAR `start_service()` launches.
//! - `pending` — a downloaded-but-not-yet-promoted JAR (set at end of download,
//!   cleared on successful promotion). Its presence signals "the operator asked
//!   for an update — complete it on next start if you haven't already."
//! - `history` — newest-first list of prior actives, retained for rollback and
//!   pruned by `jar_history_keep`.

use crate::config::NucleusConfig;
use crate::error::NucleusError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One entry in the manifest — a specific JAR on disk with its build fingerprint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JarEntry {
    /// Basename only, e.g. `auth_20260822-143022-abc1234.jar`. Resolved against
    /// the service's `jars_dir` at read time.
    pub file: String,
    pub short_sha: String,
    pub built_at: String,
    #[serde(default)]
    pub java_version: String,
    #[serde(default)]
    pub size_mb: f64,
    /// When nucleus pulled this JAR (RFC3339 UTC).
    pub downloaded_at: String,
}

/// The per-service manifest. Written atomically; every state after a partial
/// crash is one of the rows in [`recover_on_start`]'s table.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct JarManifest {
    pub service: String,
    #[serde(default)]
    pub active: Option<JarEntry>,
    #[serde(default)]
    pub pending: Option<JarEntry>,
    /// Newest-first list of prior active JARs still retained on disk.
    #[serde(default)]
    pub history: Vec<JarEntry>,
    #[serde(default)]
    pub updated_at: String,
}

impl JarManifest {
    /// Fresh empty manifest for a service.
    pub fn empty(service: &str) -> Self {
        Self {
            service: service.to_string(),
            active: None,
            pending: None,
            history: Vec::new(),
            updated_at: now_utc(),
        }
    }

    /// Path to the manifest JSON for `service` under `jars_dir`.
    pub fn path(jars_dir: &Path, service: &str) -> PathBuf {
        jars_dir.join(format!("{}.manifest.json", service))
    }

    /// Absolute path to this entry's JAR under `jars_dir` (only valid for
    /// entries in `active`/`pending`/`history` of a manifest with that dir).
    pub fn resolve(jars_dir: &Path, entry: &JarEntry) -> PathBuf {
        jars_dir.join(&entry.file)
    }

    /// Read the manifest for `service`, or return an empty one if the file is
    /// missing or unparseable (a corrupt manifest is treated as "no state" so
    /// the next update rebuilds it — never fatal at boot).
    pub fn read(jars_dir: &Path, service: &str) -> Self {
        let p = Self::path(jars_dir, service);
        match std::fs::read_to_string(&p) {
            Ok(content) => match serde_json::from_str::<Self>(&content) {
                Ok(mut m) => {
                    // Backfill service name if a hand-authored manifest omitted it.
                    if m.service.is_empty() {
                        m.service = service.to_string();
                    }
                    m
                }
                Err(e) => {
                    tracing::warn!(
                        "manifest for {} at {} is unparseable ({}); treating as empty",
                        service,
                        p.display(),
                        e
                    );
                    Self::empty(service)
                }
            },
            Err(_) => Self::empty(service),
        }
    }

    /// Write the manifest atomically: serialize to a temp file next to the
    /// target, then rename. Rename is single-file and small — the Windows lock
    /// window is a fraction of a second even under contention, and the manifest
    /// is never memory-mapped by another process. Rename-with-retry defends
    /// against the transient window.
    pub async fn write_atomic(&self, jars_dir: &Path) -> Result<(), NucleusError> {
        tokio::fs::create_dir_all(jars_dir).await?;
        let final_path = Self::path(jars_dir, &self.service);
        let tmp_path = jars_dir.join(format!("{}.manifest.json.tmp", self.service));

        let json = serde_json::to_vec_pretty(self)
            .map_err(|e| NucleusError::Internal(format!("manifest serialize failed: {}", e)))?;
        tokio::fs::write(&tmp_path, &json).await?;

        rename_with_retry(&tmp_path, &final_path).await
    }

    /// Record a freshly-downloaded JAR as `pending`. Called at the end of the
    /// download step, BEFORE any process is touched — the presence of `pending`
    /// is the durable intent that survives a crash.
    ///
    /// If an older, unpromoted `pending` exists, it becomes an orphan on disk
    /// (cleaned by [`gc`]); the manifest holds only the most recent one.
    pub fn set_pending(&mut self, entry: JarEntry) {
        self.pending = Some(entry);
        self.updated_at = now_utc();
    }

    /// Promote `pending` → `active`, pushing the old active to the head of
    /// `history`. No-op if there is no pending. The caller must have stopped
    /// the old JVM already (or accepted that both may be running briefly).
    pub fn promote_pending(&mut self) {
        let Some(new_active) = self.pending.take() else {
            return;
        };
        if let Some(old_active) = self.active.take() {
            self.history.insert(0, old_active);
        }
        self.active = Some(new_active);
        self.updated_at = now_utc();
    }

    /// Swap `active` with a specific entry from `history` (identified by `file`).
    /// Returns true if the swap happened. The old active goes back into history
    /// at the head. Used for arbitrary-generation rollback.
    pub fn rollback_to(&mut self, file: &str) -> bool {
        let Some(idx) = self.history.iter().position(|e| e.file == file) else {
            return false;
        };
        let Some(old_active) = self.active.take() else {
            // Nothing was active — just promote the target to active.
            let target = self.history.remove(idx);
            self.active = Some(target);
            self.updated_at = now_utc();
            return true;
        };
        let target = self.history.remove(idx);
        self.active = Some(target);
        self.history.insert(0, old_active);
        self.updated_at = now_utc();
        true
    }

    /// Swap `active` with `history[0]` (one-step rollback). Returns true if a
    /// swap happened. Called by the "Rollback" action in the UI/CLI when no
    /// explicit target is given.
    pub fn rollback_one(&mut self) -> bool {
        let target = self.history.get(0).map(|e| e.file.clone());
        match target {
            Some(file) => self.rollback_to(&file),
            None => false,
        }
    }

    /// Reconcile the manifest with what's actually on disk and return the
    /// entry that should be launched, if any. Called at the start of every
    /// `start_service()`. Mutates the manifest and persists it if changed.
    ///
    /// | manifest.active | manifest.pending | on-disk pending | action              |
    /// |-----------------|------------------|-----------------|---------------------|
    /// | set, file OK    | none             | —               | launch active       |
    /// | set             | set, file OK     | present         | promote → launch    |
    /// | set             | set              | missing         | clear pending, launch active |
    /// | none, hist[0]OK | —                | —               | rollback to hist[0], launch  |
    /// | none            | none             | —               | Err "no active JAR" |
    /// | file missing    | none             | —               | try hist[0], else Err |
    pub async fn recover_on_start(
        &mut self,
        jars_dir: &Path,
    ) -> Result<JarEntry, NucleusError> {
        let mut mutated = false;

        // (1) If we have a pending, decide what to do with it.
        if let Some(pending) = self.pending.clone() {
            let pending_path = Self::resolve(jars_dir, &pending);
            if pending_path.exists() {
                // Complete the interrupted promotion.
                tracing::info!(
                    "recover_on_start({}): promoting pending {} → active",
                    self.service,
                    pending.file
                );
                self.promote_pending();
                mutated = true;
            } else {
                // Crash mid-download or someone deleted the file. Drop the
                // stale intent so a fresh update can be attempted.
                tracing::warn!(
                    "recover_on_start({}): pending {} missing on disk; clearing intent",
                    self.service,
                    pending.file
                );
                self.pending = None;
                self.updated_at = now_utc();
                mutated = true;
            }
        }

        // (2) Validate active. If missing on disk, fall back to history head.
        if let Some(active) = self.active.clone() {
            let active_path = Self::resolve(jars_dir, &active);
            if !active_path.exists() {
                tracing::warn!(
                    "recover_on_start({}): active {} missing on disk; rolling back to history",
                    self.service,
                    active.file
                );
                self.active = None;
                if self.rollback_one() {
                    mutated = true;
                }
            }
        }

        if mutated {
            self.write_atomic(jars_dir).await?;
        }

        self.active.clone().ok_or_else(|| {
            NucleusError::NotFound(format!(
                "No active JAR recorded for {}. Pull the JAR first.",
                self.service
            ))
        })
    }

    /// Delete versioned JAR files under `jars_dir` for this service that are
    /// neither `active`, `pending`, nor within the most-recent `keep_count` of
    /// `history`. Returns the number of files removed. Never removes the
    /// active or pending file, even if `keep_count` is 0.
    pub async fn gc(&mut self, jars_dir: &Path, keep_count: usize) -> Result<usize, NucleusError> {
        // Trim manifest history to the retention window.
        let trimmed: Vec<JarEntry> = if self.history.len() > keep_count {
            self.history.split_off(keep_count)
        } else {
            Vec::new()
        };

        // Build the retained-basename set from the manifest.
        let mut retained: std::collections::HashSet<String> = self
            .history
            .iter()
            .chain(self.active.iter())
            .chain(self.pending.iter())
            .map(|e| e.file.clone())
            .collect();
        // The manifest itself is retained too.
        retained.insert(format!("{}.manifest.json", self.service));

        // Scan jars_dir for this service's timestamped JARs and delete unretained.
        let mut removed = 0usize;
        let prefix = format!("{}_", self.service);
        let mut rd = tokio::fs::read_dir(jars_dir).await?;
        while let Some(entry) = rd.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with(&prefix) || !name.ends_with(".jar") {
                continue;
            }
            if retained.contains(&name) {
                continue;
            }
            match tokio::fs::remove_file(entry.path()).await {
                Ok(_) => {
                    tracing::debug!("gc({}): removed {}", self.service, name);
                    removed += 1;
                }
                Err(e) => {
                    tracing::warn!("gc({}): could not remove {}: {}", self.service, name, e);
                }
            }
        }

        // Also delete manifest-dropped entries whose files may still be on disk
        // but named oddly (defensive — the prefix scan above should cover them).
        for e in &trimmed {
            let p = Self::resolve(jars_dir, e);
            if p.exists() {
                if tokio::fs::remove_file(&p).await.is_ok() {
                    removed += 1;
                }
            }
        }

        if removed > 0 {
            self.updated_at = now_utc();
            self.write_atomic(jars_dir).await?;
        }
        Ok(removed)
    }
}

/// Build a new timestamped basename for a downloaded JAR. Format:
/// `{service}_{YYYYMMDD-HHmmss}-{short_sha}.jar`. Short-sha is truncated to 7
/// chars and stripped of non-alnum. The timestamp gives natural ordering; the
/// short_sha lets a human tie a file back to a GCS build without opening the
/// manifest.
pub fn make_versioned_filename(service: &str, short_sha: &str) -> String {
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let sha_safe: String = short_sha
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(7)
        .collect();
    if sha_safe.is_empty() {
        format!("{}_{}.jar", service, ts)
    } else {
        format!("{}_{}-{}.jar", service, ts, sha_safe)
    }
}

/// RFC3339 UTC "now" for manifest timestamps.
fn now_utc() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Rename with retry — same logic as `releases::rename_with_retry`, duplicated
/// here to avoid a services→releases dependency cycle. Windows can briefly hold
/// ACCESS_DENIED (5) / SHARING_VIOLATION (32) on rename right after a kill or
/// under AV scanners; retry with 400ms backoff for ~8s.
async fn rename_with_retry(from: &Path, to: &Path) -> Result<(), NucleusError> {
    let mut last: Option<std::io::Error> = None;
    for _ in 0..20 {
        match tokio::fs::rename(from, to).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                let code = e.raw_os_error().unwrap_or(0);
                if code == 5 || code == 32 {
                    last = Some(e);
                    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                    continue;
                }
                return Err(NucleusError::Io(e));
            }
        }
    }
    Err(NucleusError::Io(last.unwrap_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::Other, "manifest rename failed")
    })))
}

/// Convenience for the common "read manifest for this service under this config's
/// jars_dir" call. Callers that need the jars_dir separately should use
/// `JarManifest::read` directly.
pub fn read_for(config: &NucleusConfig, service: &str) -> JarManifest {
    JarManifest::read(&config.jars_dir(), service)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "puru-manifest-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    fn entry(file: &str, sha: &str) -> JarEntry {
        JarEntry {
            file: file.to_string(),
            short_sha: sha.to_string(),
            built_at: "2026-01-01T00:00:00Z".into(),
            java_version: "21".into(),
            size_mb: 10.0,
            downloaded_at: now_utc(),
        }
    }

    #[tokio::test]
    async fn write_and_read_roundtrip() {
        let dir = tmp_dir();
        let mut m = JarManifest::empty("svc");
        m.active = Some(entry("svc_20260101-000000-aaa.jar", "aaa"));
        m.write_atomic(&dir).await.unwrap();

        let back = JarManifest::read(&dir, "svc");
        assert_eq!(back.service, "svc");
        assert!(back.active.is_some());
        assert_eq!(back.active.unwrap().short_sha, "aaa");
    }

    #[tokio::test]
    async fn promote_moves_pending_to_active_and_pushes_history() {
        let mut m = JarManifest::empty("svc");
        m.active = Some(entry("svc_1.jar", "aaa"));
        m.pending = Some(entry("svc_2.jar", "bbb"));
        m.promote_pending();
        assert_eq!(m.active.as_ref().unwrap().short_sha, "bbb");
        assert!(m.pending.is_none());
        assert_eq!(m.history.len(), 1);
        assert_eq!(m.history[0].short_sha, "aaa");
    }

    #[tokio::test]
    async fn rollback_swaps_active_with_history_head() {
        let mut m = JarManifest::empty("svc");
        m.active = Some(entry("svc_new.jar", "new"));
        m.history = vec![entry("svc_old.jar", "old")];
        assert!(m.rollback_one());
        assert_eq!(m.active.as_ref().unwrap().short_sha, "old");
        assert_eq!(m.history[0].short_sha, "new");
    }

    #[tokio::test]
    async fn rollback_to_specific_file() {
        let mut m = JarManifest::empty("svc");
        m.active = Some(entry("svc_c.jar", "ccc"));
        m.history = vec![
            entry("svc_b.jar", "bbb"),
            entry("svc_a.jar", "aaa"),
        ];
        assert!(m.rollback_to("svc_a.jar"));
        assert_eq!(m.active.as_ref().unwrap().short_sha, "aaa");
        // Old active goes to head; the other history entry stays.
        assert_eq!(m.history[0].short_sha, "ccc");
        assert_eq!(m.history[1].short_sha, "bbb");
    }

    #[tokio::test]
    async fn recover_promotes_pending_when_file_exists() {
        let dir = tmp_dir();
        std::fs::write(dir.join("svc_new.jar"), b"jar").unwrap();
        std::fs::write(dir.join("svc_old.jar"), b"jar").unwrap();

        let mut m = JarManifest::empty("svc");
        m.active = Some(entry("svc_old.jar", "old"));
        m.pending = Some(entry("svc_new.jar", "new"));
        m.write_atomic(&dir).await.unwrap();

        let launched = m.recover_on_start(&dir).await.unwrap();
        assert_eq!(launched.short_sha, "new");
        assert!(m.pending.is_none());
        assert_eq!(m.active.as_ref().unwrap().short_sha, "new");
    }

    #[tokio::test]
    async fn recover_clears_pending_when_file_missing() {
        let dir = tmp_dir();
        std::fs::write(dir.join("svc_old.jar"), b"jar").unwrap();

        let mut m = JarManifest::empty("svc");
        m.active = Some(entry("svc_old.jar", "old"));
        m.pending = Some(entry("svc_ghost.jar", "ghost")); // never made it to disk
        m.write_atomic(&dir).await.unwrap();

        let launched = m.recover_on_start(&dir).await.unwrap();
        assert_eq!(launched.short_sha, "old");
        assert!(m.pending.is_none());
    }

    #[tokio::test]
    async fn recover_falls_back_when_active_file_missing() {
        let dir = tmp_dir();
        std::fs::write(dir.join("svc_prev.jar"), b"jar").unwrap();

        let mut m = JarManifest::empty("svc");
        m.active = Some(entry("svc_lost.jar", "lost")); // file NOT on disk
        m.history = vec![entry("svc_prev.jar", "prev")];
        m.write_atomic(&dir).await.unwrap();

        let launched = m.recover_on_start(&dir).await.unwrap();
        assert_eq!(launched.short_sha, "prev");
    }

    #[tokio::test]
    async fn recover_returns_err_when_no_active_and_no_history() {
        let dir = tmp_dir();
        let mut m = JarManifest::empty("svc");
        let err = m.recover_on_start(&dir).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn gc_removes_files_outside_retention_window() {
        let dir = tmp_dir();
        // 4 files on disk, manifest keeps active + 2 history entries; gc(2) → 1 removed.
        for name in ["svc_a.jar", "svc_b.jar", "svc_c.jar", "svc_d.jar"] {
            std::fs::write(dir.join(name), b"jar").unwrap();
        }
        let mut m = JarManifest::empty("svc");
        m.active = Some(entry("svc_a.jar", "a"));
        m.history = vec![
            entry("svc_b.jar", "b"),
            entry("svc_c.jar", "c"),
            entry("svc_d.jar", "d"),
        ];
        let removed = m.gc(&dir, 2).await.unwrap();
        // history trimmed to 2 (b, c); d dropped and deleted.
        assert_eq!(removed, 1);
        assert!(dir.join("svc_a.jar").exists());
        assert!(dir.join("svc_b.jar").exists());
        assert!(dir.join("svc_c.jar").exists());
        assert!(!dir.join("svc_d.jar").exists());
    }

    #[test]
    fn versioned_filename_contains_service_ts_sha() {
        let name = make_versioned_filename("puru-auth", "abc1234xyz");
        assert!(name.starts_with("puru-auth_"));
        assert!(name.ends_with("-abc1234.jar")); // truncated to 7
    }

    #[test]
    fn versioned_filename_handles_empty_sha() {
        let name = make_versioned_filename("puru-auth", "");
        assert!(name.starts_with("puru-auth_"));
        assert!(name.ends_with(".jar"));
    }
}
