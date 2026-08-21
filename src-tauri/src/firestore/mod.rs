//! Firestore client facade for puru-dc.
//!
//! Provides a high-level API for hospital document operations
//! against the `puru-255206` Firestore project.

pub mod auth;
pub mod convert;
pub mod queries;
pub mod types;

use google_cloud_auth::token_source::TokenSource;
use reqwest::Client;
use serde_json::json;

use crate::config::{ConfigSyncStatus, NucleusConfig};
use crate::error::NucleusError;

use self::convert::*;
use self::types::FirestoreDocument;

/// Enriched daemon info pushed alongside telemetry.
pub struct DaemonInfo {
    pub port: u16,
    pub uptime_seconds: u64,
    pub scheduled_backups_enabled: bool,
    pub last_backup_at: Option<String>,
    pub last_backup_status: String,
    pub total_backups: usize,
    pub deployment_mode: String,
}

/// High-level Firestore client backed by a service account token source.
pub struct FirestoreClient {
    http: Client,
    token_source: Box<dyn TokenSource>,
}

impl FirestoreClient {
    /// Create a new client from an explicit credentials file path.
    pub async fn new(credentials_path: &str) -> Result<Self, NucleusError> {
        let token_source = auth::create_token_source(credentials_path).await?;
        Ok(Self {
            http: Client::new(),
            token_source,
        })
    }

    /// Create a new client using the `gcs_credentials_path` from NucleusConfig.
    pub async fn new_from_config() -> Result<Self, NucleusError> {
        let config = crate::config::load_config()?;
        let creds_path = config
            .gcs_credentials_path
            .as_deref()
            .filter(|p| !p.is_empty())
            .ok_or_else(|| {
                NucleusError::FirestoreAuth("GCS credentials path not configured. Set it in Settings.".into())
            })?;
        Self::new(creds_path).await
    }

    /// Get a fresh bearer token.
    async fn token(&self) -> Result<String, NucleusError> {
        auth::get_bearer_token(self.token_source.as_ref()).await
    }

    // ── Domain methods ──────────────────────────────────────────────────

    /// Find a hospital document by email (queries the `hospital` collection).
    pub async fn find_hospital_by_email(
        &self,
        email: &str,
    ) -> Result<FirestoreDocument, NucleusError> {
        let token = self.token().await?;
        let docs =
            queries::query_collection(&self.http, &token, "hospital", "email", email).await?;

        docs.into_iter().next().ok_or_else(|| {
            NucleusError::NotFound(format!("No hospital found with email: {}", email))
        })
    }

    /// Get a hospital document by its short code.
    pub async fn get_hospital(&self, code: &str) -> Result<FirestoreDocument, NucleusError> {
        let token = self.token().await?;
        let path = format!("hospital/{}", code);
        queries::get_document(&self.http, &token, &path).await
    }

    /// List all alerts for a hospital.
    pub async fn list_alerts(
        &self,
        code: &str,
    ) -> Result<Vec<FirestoreDocument>, NucleusError> {
        let token = self.token().await?;
        let parent = format!("hospital/{}", code);
        queries::list_subcollection(&self.http, &token, &parent, "alerts").await
    }

    /// Acknowledge an alert by setting `acknowledged: true` and `acknowledged_at: now`.
    pub async fn acknowledge_alert(
        &self,
        code: &str,
        alert_id: &str,
    ) -> Result<(), NucleusError> {
        let token = self.token().await?;
        let path = format!("hospital/{}/alerts/{}", code, alert_id);
        let now = chrono::Utc::now().to_rfc3339();

        let fields = json!({
            "acknowledged": boolean_value(true),
            "acknowledged_at": timestamp_value(&now),
        });

        queries::patch_document(
            &self.http,
            &token,
            &path,
            fields,
            &["acknowledged", "acknowledged_at"],
        )
        .await
    }

    /// Sync the local nucleus config to the hospital's cloud document.
    /// Store a single string field on the hospital document.
    pub async fn set_hospital_string_field(
        &self,
        code: &str,
        field: &str,
        value: &str,
    ) -> Result<(), NucleusError> {
        let token = self.token().await?;
        let path = format!("hospital/{}", code);
        let fields = serde_json::json!({ field: string_value(value) });
        queries::patch_document(&self.http, &token, &path, fields, &[field]).await
    }

    /// Store the MySQL root password in the hospital doc's nested `credentials`
    /// map (`credentials.mysql_root_password` + set_by + updated_at), merging
    /// without clobbering other `credentials.*` fields. This is the cloud sink of
    /// the 3-place password store and the value other machines read.
    pub async fn set_hospital_mysql_password(
        &self,
        code: &str,
        password: &str,
    ) -> Result<(), NucleusError> {
        let token = self.token().await?;
        let path = format!("hospital/{}", code);
        let now = chrono::Utc::now().to_rfc3339();
        let mut cred = serde_json::Map::new();
        cred.insert("mysql_root_password".to_string(), string_value(password));
        cred.insert("mysql_set_by".to_string(), string_value("puru-dc"));
        cred.insert("mysql_updated_at".to_string(), timestamp_value(&now));
        let fields = serde_json::json!({ "credentials": map_value(cred) });
        queries::patch_document(
            &self.http,
            &token,
            &path,
            fields,
            &[
                "credentials.mysql_root_password",
                "credentials.mysql_set_by",
                "credentials.mysql_updated_at",
            ],
        )
        .await
    }

    /// Read the MySQL root password from the hospital doc's `credentials` map,
    /// if an admin (or another machine) has set one. Cloud is the source of truth.
    pub async fn get_hospital_mysql_password(
        &self,
        code: &str,
    ) -> Result<Option<String>, NucleusError> {
        let doc = self.get_hospital(code).await?;
        Ok(doc
            .fields
            .get("credentials")
            .and_then(|c| c.get("mapValue"))
            .and_then(|m| m.get("fields"))
            .and_then(|f| f.get("mysql_root_password"))
            .and_then(|v| v.get("stringValue"))
            .and_then(|s| s.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty()))
    }

    pub async fn sync_config(
        &self,
        code: &str,
        config: &NucleusConfig,
    ) -> Result<(), NucleusError> {
        let token = self.token().await?;
        let path = format!("hospital/{}", code);
        let now = chrono::Utc::now().to_rfc3339();

        let mut nucleus_fields = serde_json::Map::new();
        nucleus_fields.insert("version".into(), string_value(env!("CARGO_PKG_VERSION")));
        nucleus_fields.insert("last_seen".into(), timestamp_value(&now));
        nucleus_fields.insert("online".into(), boolean_value(true));
        nucleus_fields.insert(
            "deployment_mode".into(),
            string_value(&format!("{:?}", config.deployment_mode).to_lowercase()),
        );

        let mut sync_fields = serde_json::Map::new();
        sync_fields.insert("pending_count".into(), integer_value(0));
        sync_fields.insert("last_synced".into(), timestamp_value(&now));

        let fields = json!({
            "nucleus": map_value(nucleus_fields),
            "config_sync": map_value(sync_fields),
            "serverIp": string_value(&config.server_ip),
        });

        queries::patch_document(
            &self.http,
            &token,
            &path,
            fields,
            &["nucleus", "config_sync", "serverIp"],
        )
        .await
    }

    /// Push a telemetry snapshot to the hospital's cloud document.
    pub async fn push_telemetry(
        &self,
        code: &str,
        snapshot: &crate::telemetry::TelemetrySnapshot,
    ) -> Result<(), NucleusError> {
        let token = self.token().await?;
        let path = format!("hospital/{}", code);
        let now = chrono::Utc::now().to_rfc3339();

        let mut telemetry_fields = serde_json::Map::new();
        telemetry_fields.insert(
            "cpu_percent".into(),
            convert::string_value(&format!("{:.1}", snapshot.cpu_percent)),
        );
        telemetry_fields.insert(
            "ram_gb".into(),
            convert::string_value(&format!("{:.2}", snapshot.ram_gb)),
        );
        telemetry_fields.insert(
            "disk_percent".into(),
            convert::string_value(&format!("{:.1}", snapshot.disk_percent)),
        );
        telemetry_fields.insert("last_reported".into(), convert::timestamp_value(&now));

        let fields = json!({
            "telemetry": convert::map_value(telemetry_fields),
        });

        queries::patch_document(&self.http, &token, &path, fields, &["telemetry"]).await
    }

    /// Read the config_sync map from a hospital document.
    pub async fn get_sync_status(
        &self,
        code: &str,
    ) -> Result<ConfigSyncStatus, NucleusError> {
        let token = self.token().await?;
        let path = format!("hospital/{}", code);
        let doc = queries::get_document(&self.http, &token, &path).await?;

        let Some(config_sync_val) = doc.fields.get("config_sync") else {
            return Ok(ConfigSyncStatus::default());
        };

        let map = match get_map_fields(config_sync_val) {
            Ok(m) => m,
            Err(_) => return Ok(ConfigSyncStatus::default()),
        };

        let last_synced = map
            .get("last_synced")
            .and_then(get_optional_timestamp)
            .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc));

        let pending_count = map
            .get("pending_count")
            .and_then(|v| get_integer(v).ok())
            .unwrap_or(0) as usize;

        Ok(ConfigSyncStatus {
            last_synced,
            pending_count,
        })
    }

    /// Push enriched status to the hospital document.
    /// Combines telemetry, nucleus info, services, and backup summary in one PATCH.
    pub async fn push_status(
        &self,
        code: &str,
        snapshot: &crate::telemetry::TelemetrySnapshot,
        daemon_info: &DaemonInfo,
        services: &[crate::services::ServiceInfo],
    ) -> Result<(), NucleusError> {
        let token = self.token().await?;
        let path = format!("hospital/{}", code);
        let now = chrono::Utc::now().to_rfc3339();

        // Telemetry map
        let mut telemetry_fields = serde_json::Map::new();
        telemetry_fields.insert(
            "cpu_percent".into(),
            string_value(&format!("{:.1}", snapshot.cpu_percent)),
        );
        telemetry_fields.insert(
            "ram_gb".into(),
            string_value(&format!("{:.2}", snapshot.ram_gb)),
        );
        telemetry_fields.insert(
            "disk_percent".into(),
            string_value(&format!("{:.1}", snapshot.disk_percent)),
        );
        telemetry_fields.insert("last_reported".into(), timestamp_value(&now));

        // Enriched nucleus map
        let mut nucleus_fields = serde_json::Map::new();
        nucleus_fields.insert("version".into(), string_value(env!("CARGO_PKG_VERSION")));
        nucleus_fields.insert("last_seen".into(), timestamp_value(&now));
        nucleus_fields.insert("online".into(), boolean_value(true));
        nucleus_fields.insert("port".into(), integer_value(daemon_info.port as i64));
        nucleus_fields.insert(
            "uptime_seconds".into(),
            integer_value(daemon_info.uptime_seconds as i64),
        );
        nucleus_fields.insert(
            "scheduled_backups_enabled".into(),
            boolean_value(daemon_info.scheduled_backups_enabled),
        );
        nucleus_fields.insert(
            "deployment_mode".into(),
            string_value(&daemon_info.deployment_mode),
        );

        // Machine fingerprint (from local license, if available)
        if let Ok(Some(license)) = crate::licensing::load_license() {
            if let Some(ref fp) = license.machine_fingerprint {
                nucleus_fields.insert("machine_fingerprint".into(), string_value(fp));
            }
            if let Some(ref name) = license.machine_name {
                nucleus_fields.insert("machine_name".into(), string_value(name));
            }
        }

        // Services map: { "Xenon (Backend)": { status: "running", container_name: "backend" } }
        let mut services_map = serde_json::Map::new();
        for svc in services {
            let mut entry = serde_json::Map::new();
            entry.insert("status".into(), string_value(&format!("{:?}", svc.status).to_lowercase()));
            entry.insert("container_name".into(), string_value(&svc.container_name));
            services_map.insert(svc.name.clone(), map_value(entry));
        }

        // Backup summary map
        let mut backup_fields = serde_json::Map::new();
        if let Some(ref ts) = daemon_info.last_backup_at {
            backup_fields.insert("last_backup_at".into(), string_value(ts));
        } else {
            backup_fields.insert("last_backup_at".into(), null_value());
        }
        backup_fields.insert(
            "last_backup_status".into(),
            string_value(&daemon_info.last_backup_status),
        );
        backup_fields.insert(
            "total_backups".into(),
            integer_value(daemon_info.total_backups as i64),
        );

        let fields = json!({
            "telemetry": map_value(telemetry_fields),
            "nucleus": map_value(nucleus_fields),
            "services": map_value(services_map),
            "backup_summary": map_value(backup_fields),
        });

        queries::patch_document(
            &self.http,
            &token,
            &path,
            fields,
            &["telemetry", "nucleus", "services", "backup_summary"],
        )
        .await
    }

    /// Lightweight liveness heartbeat — refreshes ONLY `nucleus.last_seen` and
    /// `nucleus.online` via a nested field mask, without re-collecting telemetry
    /// or services. Called every 60s by the watchdog so the cloud dashboard's
    /// online status stays current between the (15-min) rich status pushes.
    pub async fn push_heartbeat(&self, code: &str) -> Result<(), NucleusError> {
        let token = self.token().await?;
        let path = format!("hospital/{}", code);
        let now = chrono::Utc::now().to_rfc3339();

        let mut nucleus_fields = serde_json::Map::new();
        nucleus_fields.insert("last_seen".into(), timestamp_value(&now));
        nucleus_fields.insert("online".into(), boolean_value(true));

        let fields = json!({ "nucleus": map_value(nucleus_fields) });
        queries::patch_document(
            &self.http,
            &token,
            &path,
            fields,
            &["nucleus.last_seen", "nucleus.online"],
        )
        .await
    }

    /// Push an alert to the hospital's alerts subcollection.
    pub async fn push_alert(
        &self,
        code: &str,
        severity: &str,
        category: &str,
        title: &str,
        message: &str,
    ) -> Result<String, NucleusError> {
        let token = self.token().await?;
        let parent = format!("hospital/{}", code);
        let now = chrono::Utc::now().to_rfc3339();

        let fields = json!({
            "severity": string_value(severity),
            "category": string_value(category),
            "title": string_value(title),
            "message": string_value(message),
            "acknowledged": boolean_value(false),
            // Written explicitly (rather than left absent) so puru-oxygen can
            // query `resolved == false` for the "still broken" list.
            "resolved": boolean_value(false),
            "created_at": timestamp_value(&now),
        });

        queries::create_document(&self.http, &token, &parent, "alerts", fields).await
    }

    /// Mirror the watchdog's currently-open alerts onto `hospital/{code}` as an
    /// `alert_summary` map.
    ///
    /// Same idea as `services` / `backup_summary`: the puru-oxygen hospital list
    /// can badge every site from one document read instead of a subcollection
    /// query per hospital. The alerts subcollection stays the source of truth
    /// for detail; this is the rollup.
    pub async fn push_alert_summary(
        &self,
        code: &str,
        critical: i64,
        warning: i64,
        categories: &[String],
    ) -> Result<(), NucleusError> {
        let token = self.token().await?;
        let path = format!("hospital/{}", code);
        let now = chrono::Utc::now().to_rfc3339();

        let mut summary = serde_json::Map::new();
        summary.insert("critical".into(), integer_value(critical));
        summary.insert("warning".into(), integer_value(warning));
        summary.insert("open".into(), integer_value(critical + warning));
        summary.insert(
            "categories".into(),
            array_value(categories.iter().map(|c| string_value(c)).collect()),
        );
        summary.insert("updated_at".into(), timestamp_value(&now));

        let fields = json!({ "alert_summary": map_value(summary) });
        queries::patch_document(&self.http, &token, &path, fields, &["alert_summary"]).await
    }

    /// Mark an alert resolved — the condition that raised it has cleared.
    ///
    /// Separate from `acknowledge_alert`: acknowledging says a human saw it,
    /// resolving says the machine is healthy again. An alert can be resolved
    /// without ever having been acknowledged.
    pub async fn resolve_alert(&self, code: &str, alert_id: &str) -> Result<(), NucleusError> {
        let token = self.token().await?;
        let path = format!("hospital/{}/alerts/{}", code, alert_id);
        let now = chrono::Utc::now().to_rfc3339();

        let fields = json!({
            "resolved": boolean_value(true),
            "resolved_at": timestamp_value(&now),
        });

        queries::patch_document(&self.http, &token, &path, fields, &["resolved", "resolved_at"])
            .await
    }

    /// Log deactivation event and clear nucleus heartbeat from hospital doc.
    pub async fn log_deactivation(
        &self,
        code: &str,
        fingerprint: &str,
    ) -> Result<(), NucleusError> {
        let token = self.token().await?;
        let now = chrono::Utc::now().to_rfc3339();

        // Write audit log entry
        let parent = format!("hospital/{}", code);
        let fields = json!({
            "action": string_value("nucleus_deactivated"),
            "machine_fingerprint": string_value(fingerprint),
            "timestamp": timestamp_value(&now),
        });
        let _ = queries::create_document(&self.http, &token, &parent, "audit_log", fields).await;

        // Clear nucleus heartbeat from hospital doc
        let path = format!("hospital/{}", code);
        let clear_fields = json!({
            "nucleus": {"nullValue": null},
        });
        let _ = queries::patch_document(
            &self.http, &token, &path, clear_fields, &["nucleus"]
        ).await;

        Ok(())
    }

    /// Register a machine in the hospital's `nucleus_machines` array.
    ///
    /// Reads the hospital doc, checks if the fingerprint already exists in the array,
    /// and either updates the existing entry or appends a new one.
    pub async fn register_machine(
        &self,
        code: &str,
        fingerprint: &str,
        machine_name: &str,
    ) -> Result<(), NucleusError> {
        let doc = self.get_hospital(code).await?;
        let token = self.token().await?;
        let path = format!("hospital/{}", code);
        let now = chrono::Utc::now().to_rfc3339();

        let mut machines: Vec<serde_json::Value> = Vec::new();
        let mut found = false;

        // Rebuild the array, updating the matching entry if found
        if let Some(arr_val) = doc.fields.get("nucleus_machines") {
            if let Some(items) = arr_val
                .get("arrayValue")
                .and_then(|a| a.get("values"))
                .and_then(|v| v.as_array())
            {
                for item in items {
                    let item_fp = convert::get_map_fields(item)
                        .ok()
                        .and_then(|m| m.get("fingerprint"))
                        .and_then(|v| convert::get_optional_string(v));

                    if item_fp.as_deref() == Some(fingerprint) {
                        // Update existing entry
                        found = true;
                        let mut entry = serde_json::Map::new();
                        entry.insert("fingerprint".into(), string_value(fingerprint));
                        entry.insert("name".into(), string_value(machine_name));
                        // Preserve original activated_at if present
                        let activated_at = convert::get_map_fields(item)
                            .ok()
                            .and_then(|m| m.get("activated_at"))
                            .and_then(|v| convert::get_optional_timestamp(v));
                        entry.insert(
                            "activated_at".into(),
                            timestamp_value(activated_at.as_deref().unwrap_or(&now)),
                        );
                        entry.insert("last_seen_at".into(), timestamp_value(&now));
                        machines.push(map_value(entry));
                    } else {
                        machines.push(item.clone());
                    }
                }
            }
        }

        // Append new entry if not found
        if !found {
            let mut entry = serde_json::Map::new();
            entry.insert("fingerprint".into(), string_value(fingerprint));
            entry.insert("name".into(), string_value(machine_name));
            entry.insert("activated_at".into(), timestamp_value(&now));
            entry.insert("last_seen_at".into(), timestamp_value(&now));
            machines.push(map_value(entry));
        }

        let fields = json!({
            "nucleus_machines": convert::array_value(machines),
        });

        queries::patch_document(
            &self.http,
            &token,
            &path,
            fields,
            &["nucleus_machines"],
        )
        .await
    }

    /// Pull hospital settings (info + license) from Firestore.
    ///
    /// Fetches `hospital/{code}` and extracts:
    /// - `name`, `shortName`, `city`, `email` → `HospitalInfo`
    /// - `license` map → `License` (via shared `extract_license_from_firestore`)
    pub async fn pull_hospital_settings(
        &self,
        code: &str,
    ) -> Result<(crate::licensing::HospitalInfo, crate::licensing::License), NucleusError> {
        let doc = self.get_hospital(code).await?;

        let hospital_name = doc
            .fields
            .get("name")
            .and_then(|v| convert::get_optional_string(v))
            .unwrap_or_else(|| code.to_string());

        let hospital_info = crate::licensing::extract_hospital_info(&doc.fields);
        let license = crate::licensing::extract_license_from_firestore(&doc.fields, &hospital_name);

        Ok((hospital_info, license))
    }

    /// List messages from the hospital's inbox subcollection, ordered by created_at DESC.
    pub async fn list_inbox(
        &self,
        code: &str,
        limit: u32,
    ) -> Result<Vec<FirestoreDocument>, NucleusError> {
        let token = self.token().await?;
        let parent = format!("hospital/{}", code);
        queries::query_subcollection_ordered(
            &self.http,
            &token,
            &parent,
            "inbox",
            "created_at",
            limit,
        )
        .await
    }

    /// Mark an inbox message as read.
    pub async fn mark_message_read(
        &self,
        code: &str,
        message_id: &str,
    ) -> Result<(), NucleusError> {
        let token = self.token().await?;
        let path = format!("hospital/{}/inbox/{}", code, message_id);

        let fields = json!({
            "read": boolean_value(true),
        });

        queries::patch_document(&self.http, &token, &path, fields, &["read"]).await
    }

    /// Fetch the `modules` map from a hospital document.
    /// Returns a `ServiceModules` with defaults for any missing fields.
    pub async fn fetch_modules(
        &self,
        code: &str,
    ) -> Result<crate::compose_template::ServiceModules, NucleusError> {
        let doc = self.get_hospital(code).await?;

        let modules_val = doc.fields.get("modules");
        let map = modules_val.and_then(|v| get_map_fields(v).ok());

        let read_bool = |key: &str, default: bool| -> bool {
            map.and_then(|m| m.get(key))
                .and_then(|v| v.get("booleanValue"))
                .and_then(|v| v.as_bool())
                .unwrap_or(default)
        };

        // Default to false — only pull/deploy services explicitly enabled in oxygen
        Ok(crate::compose_template::ServiceModules {
            auth: read_bool("auth", false),
            xenon: read_bool("xenon", false),
            has: read_bool("has", false),
            pacs: read_bool("pacs", false),
            argon: read_bool("argon", false),
            comm: read_bool("comm", false),
            realtime: read_bool("realtime", false),
            neon: read_bool("neon", false),
            mercury: read_bool("mercury", false),
            counter: read_bool("counter", false),
            bridge: read_bool("bridge", false),
            integration: read_bool("integration", false),
            hydrogen: read_bool("hydrogen", false),
            dviewer: read_bool("dviewer", false),
        })
    }

    /// Poll pending commands from the hospital's commands subcollection.
    pub async fn poll_pending_commands(
        &self,
        code: &str,
    ) -> Result<Vec<FirestoreDocument>, NucleusError> {
        let token = self.token().await?;
        let parent = format!("hospital/{}", code);
        queries::query_subcollection(&self.http, &token, &parent, "commands", "status", "pending")
            .await
    }

    /// List the most recent commands (any status), newest first — for the GUI's
    /// command-activity banner.
    pub async fn list_recent_commands(
        &self,
        code: &str,
        limit: u32,
    ) -> Result<Vec<FirestoreDocument>, NucleusError> {
        let token = self.token().await?;
        let parent = format!("hospital/{}", code);
        queries::query_subcollection_ordered(&self.http, &token, &parent, "commands", "created_at", limit)
            .await
    }

    /// Update a command document's status and result fields.
    pub async fn update_command_status(
        &self,
        code: &str,
        command_id: &str,
        status: &str,
        result: Option<&str>,
        error: Option<&str>,
    ) -> Result<(), NucleusError> {
        let token = self.token().await?;
        let path = format!("hospital/{}/commands/{}", code, command_id);
        let now = chrono::Utc::now().to_rfc3339();

        let mut fields = serde_json::Map::new();
        fields.insert("status".into(), string_value(status));
        fields.insert("executed_at".into(), timestamp_value(&now));

        let mut field_paths = vec!["status", "executed_at"];

        if let Some(r) = result {
            fields.insert("result".into(), string_value(r));
            field_paths.push("result");
        }
        if let Some(e) = error {
            fields.insert("error".into(), string_value(e));
            field_paths.push("error");
        }

        queries::patch_document(
            &self.http,
            &token,
            &path,
            serde_json::Value::Object(fields),
            &field_paths,
        )
        .await
    }
}
