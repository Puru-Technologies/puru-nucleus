//! License validation module

use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};

/// Year that represents "unlimited" license
const MAX_LICENSE_YEAR: i32 = 2055;

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
