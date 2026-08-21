/**
 * Hospital model for puru-dc
 * Matches Firestore schema at /hospital/{shortName}
 */

import { License } from './license.model';

export type CredentialsStatus = 'not_created' | 'active' | 'revoked';

export interface NucleusStatus {
  version: string;
  last_seen: string;
  online: boolean;
}

export interface ConfigSyncStatus {
  pending_count: number;
  last_synced: string;
}

export interface Hospital {
  // Existing fields (from puru-oxygen/puru-online)
  name: string;
  shortName: string;           // Also used as document ID
  email: string;               // Unique - activation key
  city: string;
  address: string;
  propertyCluster: string;
  serverIp: string;
  dataLocation: string;
  onlineAllowed: boolean;
  autoUploadModality: string[];
  onlineApproachType: string;

  // License fields (for puru-dc)
  license?: License;

  // Nucleus management fields
  credentials_status?: CredentialsStatus;
  service_account?: string;
  nucleus?: NucleusStatus;
  config_sync?: ConfigSyncStatus;

  // Platform & network
  platform?: 'linux' | 'windows' | 'macos';
  network_mode?: 'host' | 'bridge';
  modules?: string[];

  // Timestamps
  created_at?: any;
  updated_at?: any;
}

/**
 * Backup configuration stored in hospital document
 */
export interface BackupConfig {
  gcs_bucket: string;
  retention_days: number;
  schedules: BackupSchedule[];
}

export interface BackupSchedule {
  type: 'full' | 'partial';
  time: string;             // HH:MM format
  days: string[];           // ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun']
  tables?: string[];        // For partial backups
}

/**
 * Message in hospital inbox
 */
export type MessageType = 'sa_key' | 'config_update' | 'alert' | 'info' | 'file';
export type MessagePriority = 'low' | 'normal' | 'high' | 'critical';

export interface MessageAttachment {
  filename: string;
  file_url: string;
  file_size?: number;
  mime_type?: string;
}

export interface HospitalMessage {
  id: string;
  message_type: MessageType;
  priority: MessagePriority;
  subject: string;
  body?: string;
  attachments: MessageAttachment[];
  read: boolean;
  created_at: string;
  created_by?: string;
}

export interface MessageDownloadResult {
  success: boolean;
  file_path?: string;
  error?: string;
}

export interface FileActionResult {
  success: boolean;
  message: string;
}

/**
 * Alert stored in hospital alerts subcollection
 */
/**
 * Categories the watchdog writes (src-tauri/src/daemon/scheduler.rs). Typed as
 * a union of the known values plus `string`, so a category added on the Rust
 * side still parses instead of breaking the build.
 */
export type AlertCategory =
  | 'disk_space'
  | 'memory'
  | 'service_down'
  | 'service_restart'
  | 'service_recovered'
  | 'service_not_installed'
  | 'boot_task_missing'
  | 'backup'
  | 'license'
  | 'network'
  | (string & {});

export interface HospitalAlert {
  id: string;
  severity: 'critical' | 'warning' | 'info';
  category: AlertCategory;
  title: string;
  message: string;
  /** A human has seen it. */
  acknowledged: boolean;
  created_at: string;
  acknowledged_at?: string;
  /** The condition cleared — set by the watchdog, not by a human. */
  resolved: boolean;
  resolved_at?: string;
}

/** Human label for an alert category, for chips and filters. */
export function alertCategoryLabel(category: AlertCategory): string {
  switch (category) {
    case 'disk_space': return 'Hard disk';
    case 'memory': return 'RAM';
    case 'service_down': return 'Service down';
    case 'service_restart': return 'Service restart';
    case 'service_recovered': return 'Service recovered';
    case 'service_not_installed': return 'Not installed';
    case 'boot_task_missing': return 'Boot task';
    default: return category;
  }
}

/**
 * Daily telemetry summary
 */
export interface TelemetrySummary {
  date: string;
  services: Record<string, ServiceTelemetry>;
  system: SystemTelemetry;
  backup: BackupTelemetry;
  alerts: AlertSummary;
  generated_at: string;
}

export interface ServiceTelemetry {
  uptime_percent: number;
  avg_health_ms: number;
  restart_count: number;
}

export interface SystemTelemetry {
  avg_cpu: number;
  max_cpu: number;
  avg_ram: number;
  max_ram: number;
  avg_disk: number;
}

export interface BackupTelemetry {
  full_count: number;
  partial_count: number;
  failed_count: number;
  total_size_mb: number;
}

export interface AlertSummary {
  critical: number;
  warning: number;
  info: number;
}
