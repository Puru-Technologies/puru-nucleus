/**
 * License model for puru-dc
 * Matches Firestore schema at /hospitals/{hospitalId}/license
 */

export const MAX_LICENSE_DATE = '2055-12-31T23:59:59Z';
export const MAX_LICENSE_YEAR = 2055;

export interface LicenseFeatures {
  binlog_shipping: boolean;
  point_in_time_recovery: boolean;
  priority_support: boolean;
}

export interface LicenseLimits {
  backup_retention_days: number;
  max_storage_gb: number;
}

export interface License {
  hospital_name: string;
  valid_till: string;           // ISO date string (2055 = unlimited)
  features: LicenseFeatures;
  limits: LicenseLimits;
  activated_at?: string;
  machine_fingerprint?: string;
  machine_name?: string;
}

export type LicenseStatusType = 'active' | 'expiring_soon' | 'expired' | 'unlimited';

export interface LicenseStatus {
  status: LicenseStatusType;
  message: string;
  icon: string;
  daysRemaining?: number;
}

/**
 * Check if a license date represents "unlimited"
 */
export function isUnlimited(validTill: string): boolean {
  return new Date(validTill).getFullYear() >= MAX_LICENSE_YEAR;
}

/**
 * Check if a license is currently valid (not expired)
 */
export function isLicenseValid(license: License): boolean {
  return new Date(license.valid_till) > new Date();
}

/**
 * Calculate days remaining until license expires
 */
export function getDaysRemaining(validTill: string): number {
  const expiryDate = new Date(validTill);
  const now = new Date();
  return Math.ceil((expiryDate.getTime() - now.getTime()) / (1000 * 60 * 60 * 24));
}

/**
 * Get detailed license status with message and icon
 */
export function getLicenseStatus(license: License): LicenseStatus {
  if (isUnlimited(license.valid_till)) {
    return {
      status: 'unlimited',
      message: 'Unlimited license',
      icon: 'all_inclusive'
    };
  }

  const daysRemaining = getDaysRemaining(license.valid_till);

  if (daysRemaining < 0) {
    return {
      status: 'expired',
      message: `Expired ${Math.abs(daysRemaining)} days ago`,
      icon: 'cancel',
      daysRemaining
    };
  }

  if (daysRemaining <= 30) {
    return {
      status: 'expiring_soon',
      message: `Expires in ${daysRemaining} days`,
      icon: 'warning',
      daysRemaining
    };
  }

  const validTill = new Date(license.valid_till);
  return {
    status: 'active',
    message: `Valid till ${validTill.toLocaleDateString()}`,
    icon: 'check_circle',
    daysRemaining
  };
}

/**
 * Create a default unlimited license
 */
export function createDefaultLicense(): License {
  return {
    hospital_name: 'Puru Hospital',
    valid_till: MAX_LICENSE_DATE,
    features: {
      binlog_shipping: false,
      point_in_time_recovery: false,
      priority_support: false
    },
    limits: {
      backup_retention_days: 30,
      max_storage_gb: 100
    }
  };
}

/**
 * Create a license that expires on a specific date
 */
export function createTimedLicense(
  validTill: Date,
  features?: Partial<LicenseFeatures>,
  limits?: Partial<LicenseLimits>
): License {
  return {
    hospital_name: 'Puru Hospital',
    valid_till: validTill.toISOString(),
    features: {
      binlog_shipping: features?.binlog_shipping ?? false,
      point_in_time_recovery: features?.point_in_time_recovery ?? false,
      priority_support: features?.priority_support ?? false
    },
    limits: {
      backup_retention_days: limits?.backup_retention_days ?? 30,
      max_storage_gb: limits?.max_storage_gb ?? 100
    },
    activated_at: new Date().toISOString()
  };
}
