//! License validation module

use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};

/// Year that represents "unlimited" license
const MAX_LICENSE_YEAR: i32 = 2055;

/// Hospital info pulled from Firestore (displayed in UI, not persisted to nucleus.toml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HospitalInfo {
    pub name: String,
    pub short_name: Option<String>,
    pub city: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct License {
    pub hospital_name: String,
    pub valid_till: DateTime<Utc>,
    pub features: LicenseFeatures,
    pub limits: LicenseLimits,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseFeatures {
    pub binlog_shipping: bool,
    pub point_in_time_recovery: bool,
    pub priority_support: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LicenseLimits {
    pub backup_retention_days: u32,
    pub max_storage_gb: u32,
}

#[derive(Debug, Clone, Serialize)]
pub enum LicenseStatus {
    Unlimited,
    Active { days_remaining: i64 },
    ExpiringSoon { days_remaining: i64 },
    Expired { days_ago: i64 },
}

impl License {
    /// Check if license represents "unlimited"
    pub fn is_unlimited(&self) -> bool {
        self.valid_till.year() >= MAX_LICENSE_YEAR
    }

    /// Check if license is currently valid (not expired)
    pub fn is_valid(&self) -> bool {
        Utc::now() < self.valid_till
    }

    /// Calculate days remaining until license expires
    pub fn days_remaining(&self) -> i64 {
        (self.valid_till - Utc::now()).num_days()
    }

    /// Get detailed license status
    pub fn status(&self) -> LicenseStatus {
        if self.is_unlimited() {
            return LicenseStatus::Unlimited;
        }

        let days = self.days_remaining();

        if days < 0 {
            LicenseStatus::Expired {
                days_ago: days.abs(),
            }
        } else if days <= 30 {
            LicenseStatus::ExpiringSoon {
                days_remaining: days,
            }
        } else {
            LicenseStatus::Active {
                days_remaining: days,
            }
        }
    }

    /// Check if binlog shipping feature is available
    pub fn can_use_binlog_shipping(&self) -> bool {
        self.is_valid() && self.features.binlog_shipping
    }

    /// Check if point-in-time recovery feature is available
    pub fn can_use_pitr(&self) -> bool {
        self.is_valid() && self.features.point_in_time_recovery
    }

    /// Check if backup retention days is within license limits
    pub fn check_retention_limit(&self, days: u32) -> bool {
        days <= self.limits.backup_retention_days
    }

    /// Check if storage usage is within license limits
    pub fn check_storage_limit(&self, gb: u32) -> bool {
        gb <= self.limits.max_storage_gb
    }
}

// ── Persistence ──────────────────────────────────────────────────────────────

/// Load license from config directory
pub fn load_license() -> Result<Option<License>, crate::error::NucleusError> {
    let license_path = crate::config::config_dir().join("license.toml");

    if !license_path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&license_path)?;
    let license: License = toml::from_str(&content).map_err(|e| {
        crate::error::NucleusError::InvalidConfig(format!("Failed to parse license: {}", e))
    })?;

    Ok(Some(license))
}

/// Save license to config directory
pub fn save_license(license: &License) -> Result<(), crate::error::NucleusError> {
    let license_path = crate::config::config_dir().join("license.toml");

    if let Some(parent) = license_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = toml::to_string_pretty(license).map_err(|e| {
        crate::error::NucleusError::InvalidConfig(format!("Failed to serialize license: {}", e))
    })?;

    std::fs::write(&license_path, content)?;
    Ok(())
}

/// Validate license from Firestore data
pub fn validate_license(license: &License) -> Result<(), String> {
    if !license.is_valid() {
        return Err(format!(
            "License expired on {}",
            license.valid_till.format("%Y-%m-%d")
        ));
    }
    Ok(())
}

/// Default "valid_till" for hospitals without a license field in Firestore.
/// Returns a far-future date so the license is effectively unlimited.
pub fn default_valid_till() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2055-12-31T23:59:59Z")
        .expect("hardcoded date must parse")
        .with_timezone(&Utc)
}

/// Extract a [`License`] from Firestore document fields.
///
/// Reads the `license` map (valid_till, features, limits) from the given fields.
/// Falls back to defaults when the map or individual keys are missing.
pub fn extract_license_from_firestore(
    fields: &std::collections::HashMap<String, serde_json::Value>,
    hospital_name: &str,
) -> License {
    use crate::firestore::convert;

    if let Some(license_val) = fields.get("license") {
        let empty_map = serde_json::Map::new();
        let license_map = convert::get_map_fields(license_val).unwrap_or(&empty_map);

        let valid_till = license_map
            .get("valid_till")
            .and_then(|v| convert::get_optional_timestamp(v))
            .and_then(|ts| chrono::DateTime::parse_from_rfc3339(&ts).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(default_valid_till);

        let features_map = license_map
            .get("features")
            .and_then(|v| convert::get_map_fields(v).ok());

        let features = LicenseFeatures {
            binlog_shipping: features_map
                .as_ref()
                .and_then(|m| m.get("binlog_shipping"))
                .and_then(|v| convert::get_bool(v).ok())
                .unwrap_or(false),
            point_in_time_recovery: features_map
                .as_ref()
                .and_then(|m| m.get("point_in_time_recovery"))
                .and_then(|v| convert::get_bool(v).ok())
                .unwrap_or(false),
            priority_support: features_map
                .as_ref()
                .and_then(|m| m.get("priority_support"))
                .and_then(|v| convert::get_bool(v).ok())
                .unwrap_or(false),
        };

        let limits_map = license_map
            .get("limits")
            .and_then(|v| convert::get_map_fields(v).ok());

        let limits = LicenseLimits {
            backup_retention_days: limits_map
                .as_ref()
                .and_then(|m| m.get("backup_retention_days"))
                .and_then(|v| convert::get_u32(v).ok())
                .unwrap_or(30),
            max_storage_gb: limits_map
                .as_ref()
                .and_then(|m| m.get("max_storage_gb"))
                .and_then(|v| convert::get_u32(v).ok())
                .unwrap_or(100),
        };

        License {
            hospital_name: hospital_name.to_string(),
            valid_till,
            features,
            limits,
            activated_at: Some(Utc::now()),
        }
    } else {
        // No license field in Firestore — create default (unlimited, standard features)
        License {
            hospital_name: hospital_name.to_string(),
            valid_till: default_valid_till(),
            features: LicenseFeatures {
                binlog_shipping: false,
                point_in_time_recovery: false,
                priority_support: false,
            },
            limits: LicenseLimits {
                backup_retention_days: 30,
                max_storage_gb: 100,
            },
            activated_at: Some(Utc::now()),
        }
    }
}

/// Extract [`HospitalInfo`] from Firestore document fields.
pub fn extract_hospital_info(
    fields: &std::collections::HashMap<String, serde_json::Value>,
) -> HospitalInfo {
    use crate::firestore::convert;

    HospitalInfo {
        name: fields
            .get("name")
            .and_then(|v| convert::get_optional_string(v))
            .unwrap_or_default(),
        short_name: fields
            .get("shortName")
            .and_then(|v| convert::get_optional_string(v)),
        city: fields
            .get("city")
            .and_then(|v| convert::get_optional_string(v)),
        email: fields
            .get("email")
            .and_then(|v| convert::get_optional_string(v)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_license() -> License {
        License {
            hospital_name: "Test Hospital".to_string(),
            valid_till: DateTime::parse_from_rfc3339("2055-12-31T23:59:59Z")
                .unwrap()
                .with_timezone(&Utc),
            features: LicenseFeatures {
                binlog_shipping: false,
                point_in_time_recovery: false,
                priority_support: false,
            },
            limits: LicenseLimits {
                backup_retention_days: 30,
                max_storage_gb: 100,
            },
            activated_at: None,
        }
    }

    #[test]
    fn test_unlimited_license() {
        let license = test_license();
        assert!(license.is_unlimited());
        assert!(license.is_valid());
    }

    #[test]
    fn test_expired_license() {
        let mut license = test_license();
        license.valid_till = DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert!(!license.is_valid());
        assert!(matches!(license.status(), LicenseStatus::Expired { .. }));
    }

    #[test]
    fn test_expiring_soon() {
        let mut license = test_license();
        license.valid_till = Utc::now() + chrono::Duration::days(15);
        assert!(license.is_valid());
        assert!(matches!(
            license.status(),
            LicenseStatus::ExpiringSoon { .. }
        ));
    }
}
