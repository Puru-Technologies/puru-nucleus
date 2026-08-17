//! Hospital-specific Jasper template channel.
//!
//! Workflow (all file-based, no microservice/DB change):
//!   1. First seed pulls the GENERIC master templates (see `seed` module).
//!   2. The hospital finalizes the jrxml locally, then `finalise()` uploads the
//!      whole local set to a per-hospital GCS folder — that folder becomes the
//!      hospital's source of truth.
//!   3. `check_updates()` diffs that folder against a local manifest (size/md5/
//!      generation) and reports "update available" — it never auto-applies.
//!   4. `apply_updates()` force-overwrites the local files from the hospital
//!      folder (called only after explicit user confirmation) and rewrites the
//!      manifest.
//!
//! puru-has compiles jrxml straight from `{puru_data}/config-data/templates/...`
//! (hardcoded), so everything lands there and HAS is none the wiser.

use crate::config::{self, NucleusConfig};
use crate::error::NucleusError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Local destination for templates, relative to puru_data_path. Must match
/// `seed`'s TEMPLATES_LOCAL_SUBDIR — puru-has hardcodes this path.
const TEMPLATES_LOCAL_SUBDIR: &[&str] = &["config-data", "templates"];

/// Bucket holding finalized per-hospital artifacts. Same convention as the
/// docker-compose upload (`{code}/setup-files/docker/docker-compose.yml`), so all per-
/// hospital outputs live together.
const HOSPITAL_BUCKET: &str = "puru-255206.appspot.com";

/// Manifest of the last-pulled hospital-folder state, stored alongside the
/// templates. Never uploaded (skipped during finalise).
const MANIFEST_FILE: &str = ".template-manifest.json";

// ── Reports (serialized to the GUI) ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct FinaliseReport {
    pub uploaded: u64,
    pub prefix: String,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TemplateUpdate {
    pub path: String,
    /// "new" (not seen before) or "changed" (content differs).
    pub status: String,
    pub old_size: i64,
    pub new_size: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApplyReport {
    pub applied: u64,
    pub errors: Vec<String>,
}

// ── Manifest ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TemplateEntry {
    size: i64,
    md5_hash: String,
    generation: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct TemplateManifest {
    /// Keyed by relative path under config-data/templates (forward slashes).
    entries: HashMap<String, TemplateEntry>,
}

// ── Paths ────────────────────────────────────────────────────────────────────

/// `{CODE}/setup-files/template/` in the default Firebase bucket — the per-hospital prefix
/// (source of truth once finalized), alongside `{code}/setup-files/docker/docker-compose.yml`.
/// Distinct from the generic `jrxml-templates/` master in the releases bucket.
fn hospital_prefix(code: &str) -> String {
    format!("{}/setup-files/template/", code)
}

fn templates_local_dir(config: &NucleusConfig) -> Result<PathBuf, NucleusError> {
    let root = config.puru_data_path.as_deref().unwrap_or("");
    if root.is_empty() {
        return Err(NucleusError::Validation(
            "puru_data_path is not set in nucleus.toml".into(),
        ));
    }
    let mut dir = PathBuf::from(root);
    for part in TEMPLATES_LOCAL_SUBDIR {
        dir.push(part);
    }
    Ok(dir)
}

fn manifest_path(local_dir: &Path) -> PathBuf {
    local_dir.join(MANIFEST_FILE)
}

fn load_manifest(local_dir: &Path) -> TemplateManifest {
    let path = manifest_path(local_dir);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_manifest(local_dir: &Path, manifest: &TemplateManifest) -> Result<(), NucleusError> {
    let path = manifest_path(local_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(manifest)
        .map_err(|e| NucleusError::Internal(format!("manifest serialize: {}", e)))?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// Build a manifest snapshot from a remote listing (keyed by relative path).
fn manifest_from_remote(prefix: &str, remote: &[crate::releases::GcsObjectMeta]) -> TemplateManifest {
    let mut entries = HashMap::new();
    for obj in remote {
        let rel = obj.name.strip_prefix(prefix).unwrap_or(&obj.name);
        if rel.is_empty() {
            continue;
        }
        entries.insert(
            rel.to_string(),
            TemplateEntry {
                size: obj.size,
                md5_hash: obj.md5_hash.clone(),
                generation: obj.generation,
            },
        );
    }
    TemplateManifest { entries }
}

/// Walk the local templates dir, returning (relative-with-forward-slashes, abs).
/// The manifest file itself is excluded.
fn collect_local_templates(root: &Path) -> Vec<(String, PathBuf)> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else if path.is_file() {
                if let Ok(rel) = path.strip_prefix(base) {
                    let rel_str = rel.to_string_lossy().replace('\\', "/");
                    if rel_str == MANIFEST_FILE {
                        continue;
                    }
                    out.push((rel_str, path.clone()));
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}

fn local_path_for_rel(local_dir: &Path, rel: &str) -> PathBuf {
    let mut path = local_dir.to_path_buf();
    for comp in rel.split('/').filter(|c| !c.is_empty()) {
        path.push(comp);
    }
    path
}

// ── Operations ───────────────────────────────────────────────────────────────

/// Upload every local template file to the hospital's GCS folder, then snapshot
/// that folder into the local manifest. This "finalizes" the hospital set so
/// other installs (and re-pulls) source from it.
pub async fn finalise(config: &NucleusConfig) -> Result<FinaliseReport, NucleusError> {
    if config.hospital_code.is_empty() {
        return Err(NucleusError::Validation(
            "Hospital code not configured. Activate a license first.".into(),
        ));
    }
    let local_dir = templates_local_dir(config)?;
    let prefix = hospital_prefix(&config.hospital_code);

    let files = collect_local_templates(&local_dir);
    if files.is_empty() {
        return Err(NucleusError::NotFound(format!(
            "No templates found at {} — seed the templates first.",
            local_dir.display()
        )));
    }

    let cred_path = crate::releases::get_credentials_path()?;
    let client = crate::releases::create_gcs_client(&cred_path).await?;

    let mut report = FinaliseReport {
        uploaded: 0,
        prefix: prefix.clone(),
        errors: Vec::new(),
    };

    for (rel, abs) in &files {
        let object_path = format!("{}{}", prefix, rel);
        match crate::releases::upload_gcs_file(&client, HOSPITAL_BUCKET, abs, &object_path).await {
            Ok(()) => report.uploaded += 1,
            Err(e) => report.errors.push(format!("{}: {}", rel, e.user_message())),
        }
    }

    // Re-list the folder so the manifest carries GCS-computed md5/generation.
    match crate::releases::list_gcs_objects_with_meta(&client, HOSPITAL_BUCKET, &prefix).await {
        Ok(remote) => {
            let manifest = manifest_from_remote(&prefix, &remote);
            if let Err(e) = save_manifest(&local_dir, &manifest) {
                report.errors.push(format!("manifest: {}", e.user_message()));
            }
        }
        Err(e) => report
            .errors
            .push(format!("snapshot after upload: {}", e.user_message())),
    }

    tracing::info!(
        "Finalised {} templates to {} ({} errors)",
        report.uploaded,
        prefix,
        report.errors.len()
    );
    Ok(report)
}

/// Diff the hospital folder against the local manifest. Returns one entry per
/// new-or-changed remote file. Empty list = local is up to date.
pub async fn check_updates(config: &NucleusConfig) -> Result<Vec<TemplateUpdate>, NucleusError> {
    if config.hospital_code.is_empty() {
        return Err(NucleusError::Validation(
            "Hospital code not configured. Activate a license first.".into(),
        ));
    }
    let local_dir = templates_local_dir(config)?;
    let prefix = hospital_prefix(&config.hospital_code);

    let cred_path = crate::releases::get_credentials_path()?;
    let client = crate::releases::create_gcs_client(&cred_path).await?;

    let remote = crate::releases::list_gcs_objects_with_meta(&client, HOSPITAL_BUCKET, &prefix).await?;
    let manifest = load_manifest(&local_dir);

    let mut updates = Vec::new();
    for obj in &remote {
        let rel = obj.name.strip_prefix(&prefix).unwrap_or(&obj.name);
        if rel.is_empty() {
            continue;
        }
        match manifest.entries.get(rel) {
            None => updates.push(TemplateUpdate {
                path: rel.to_string(),
                status: "new".into(),
                old_size: 0,
                new_size: obj.size,
            }),
            Some(entry) => {
                // md5 is the reliable content signal; generation guards the case
                // where md5 is absent (a blind re-upload bumps generation).
                let changed = entry.md5_hash != obj.md5_hash
                    || entry.generation != obj.generation
                    || entry.size != obj.size;
                if changed {
                    updates.push(TemplateUpdate {
                        path: rel.to_string(),
                        status: "changed".into(),
                        old_size: entry.size,
                        new_size: obj.size,
                    });
                }
            }
        }
    }

    Ok(updates)
}

/// Force-download every file in the hospital folder over the local copies and
/// rewrite the manifest. Call only after the user confirms the overwrite.
pub async fn apply_updates(config: &NucleusConfig) -> Result<ApplyReport, NucleusError> {
    if config.hospital_code.is_empty() {
        return Err(NucleusError::Validation(
            "Hospital code not configured. Activate a license first.".into(),
        ));
    }
    let local_dir = templates_local_dir(config)?;
    let prefix = hospital_prefix(&config.hospital_code);

    let cred_path = crate::releases::get_credentials_path()?;
    let client = crate::releases::create_gcs_client(&cred_path).await?;

    let remote = crate::releases::list_gcs_objects_with_meta(&client, HOSPITAL_BUCKET, &prefix).await?;
    if remote.is_empty() {
        return Err(NucleusError::NotFound(format!(
            "Hospital template folder gs://{}/{} is empty — nothing to apply.",
            HOSPITAL_BUCKET, prefix
        )));
    }

    let mut report = ApplyReport {
        applied: 0,
        errors: Vec::new(),
    };

    for obj in &remote {
        let rel = obj.name.strip_prefix(&prefix).unwrap_or(&obj.name);
        if rel.is_empty() {
            continue;
        }
        let local = local_path_for_rel(&local_dir, rel);
        match crate::releases::download_gcs_to_file_from(&client, HOSPITAL_BUCKET, &obj.name, &local).await {
            Ok(_) => report.applied += 1,
            Err(e) => report.errors.push(format!("{}: {}", rel, e.user_message())),
        }
    }

    // Manifest now reflects the freshly-pulled folder.
    let manifest = manifest_from_remote(&prefix, &remote);
    if let Err(e) = save_manifest(&local_dir, &manifest) {
        report.errors.push(format!("manifest: {}", e.user_message()));
    }

    tracing::info!(
        "Applied {} hospital templates from {} ({} errors)",
        report.applied,
        prefix,
        report.errors.len()
    );
    Ok(report)
}

/// Convenience wrapper used by Tauri commands: load config, then run `op`.
pub async fn with_config<F, Fut, T>(op: F) -> Result<T, NucleusError>
where
    F: FnOnce(NucleusConfig) -> Fut,
    Fut: std::future::Future<Output = Result<T, NucleusError>>,
{
    let config = config::load_config()?;
    op(config).await
}
