import { Injectable } from '@angular/core';
import { invoke } from '@tauri-apps/api/core';
import { MatSnackBar, MatSnackBarConfig } from '@angular/material/snack-bar';
import { AppError, ErrorSeverity } from '../models/app-error';

/**
 * Service for invoking Tauri commands with error handling
 */
@Injectable({
  providedIn: 'root'
})
export class TauriService {
  constructor(private snackBar: MatSnackBar) {}

  /**
   * Invoke a Tauri command with automatic error handling
   */
  async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    try {
      return await invoke<T>(command, args);
    } catch (error) {
      this.handleError(error as string, command);
      throw error;
    }
  }

  /**
   * Invoke without showing error (for silent operations)
   */
  async invokeSilent<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    return invoke<T>(command, args);
  }

  private handleError(message: string, command: string): void {
    const severity = this.parseSeverity(message);
    const action = this.getSuggestedAction(message);

    const config: MatSnackBarConfig = {
      duration: severity === 'critical' ? 0 : 5000,
      panelClass: `snackbar-${severity}`,
      horizontalPosition: 'end',
      verticalPosition: 'bottom'
    };

    this.snackBar.open(message, action || 'Dismiss', config);

    // Log error to Tauri backend
    this.logError(command, message).catch(() => {
      // Ignore logging errors
    });
  }

  private parseSeverity(message: string): ErrorSeverity {
    if (message.includes('not running') || message.includes('Disk space')) {
      return 'critical';
    }
    if (message.includes('not found') || message.includes('expired')) {
      return 'warning';
    }
    return 'error';
  }

  private getSuggestedAction(message: string): string | null {
    if (message.includes('Docker')) return 'Start Docker';
    if (message.includes('License')) return 'Contact Support';
    if (message.includes('credentials')) return 'Check Settings';
    return null;
  }

  private async logError(command: string, message: string): Promise<void> {
    try {
      await invoke('log_error', {
        command,
        message,
        timestamp: Date.now()
      });
    } catch {
      console.error('Failed to log error:', message);
    }
  }
}

/**
 * Available Tauri commands
 */
export interface TauriCommands {
  // System detection
  detect_existing_setup: () => Promise<DetectionResult>;
  get_system_info: () => Promise<SystemInfo>;
  check_prerequisites: () => Promise<PrerequisiteStatus[]>;

  // Service management
  get_services: () => Promise<ServiceInfo[]>;
  start_service: (args: { name: string }) => Promise<void>;
  stop_service: (args: { name: string }) => Promise<void>;
  restart_service: (args: { name: string }) => Promise<void>;

  // Backup
  start_backup: (args: { type: 'full' | 'partial' }) => Promise<BackupResult>;
  get_backup_history: () => Promise<BackupRecord[]>;
  restore_backup: (args: { backupId: string }) => Promise<void>;

  // Config
  get_config: () => Promise<NucleusConfig>;
  save_config: (args: { config: NucleusConfig }) => Promise<void>;
  sync_config_to_cloud: () => Promise<void>;

  // License
  get_license: () => Promise<import('../models/license.model').License>;
  activate_license: (args: { email: string }) => Promise<void>;

  // Logs
  get_container_logs: (args: { containerName: string; tail?: number }) => Promise<string>;

  // Telemetry
  get_telemetry_snapshot: () => Promise<TelemetrySnapshot>;

  // Logging
  log_error: (args: { command: string; message: string; timestamp: number }) => Promise<void>;
}

// Response types
export interface DetectionResult {
  found: boolean;
  containers: ContainerInfo[];
  compose_path?: string;
  databases: DatabaseInfo[];
}

export interface ContainerInfo {
  name: string;
  image: string;
  status: 'running' | 'stopped' | 'exited';
}

export interface DatabaseInfo {
  name: string;
  size_mb: number;
}

export interface SystemInfo {
  os: string;
  arch: string;
  hostname: string;
  cpu_cores: number;
  total_ram_gb: number;
  disk_free_gb: number;
}

export interface PrerequisiteStatus {
  name: string;
  installed: boolean;
  version?: string;
  required_version?: string;
}

export interface ServiceInfo {
  name: string;
  container_name: string;
  image: string;
  status: 'running' | 'stopped' | 'starting' | 'error';
  health?: 'healthy' | 'unhealthy' | 'starting';
  ports: string[];
  uptime?: string;
  health_response_ms?: number;
}

export interface BackupResult {
  success: boolean;
  backup_id: string;
  size_mb: number;
  duration_seconds: number;
}

export interface BackupRecord {
  id: string;
  type: 'full' | 'partial';
  status: 'completed' | 'failed' | 'in_progress';
  size_mb: number;
  created_at: string;
  uploaded: boolean;
}

export interface NucleusConfig {
  hospital_code: string;
  server_ip: string;
  docker_compose_path: string;
  gcs_credentials_path?: string;
  backup_enabled: boolean;
  telemetry_enabled: boolean;
  mysql_host: string;
  mysql_port: number;
  mysql_user: string;
  mysql_password: string;
  auto_update_enabled: boolean;
  release_channel: string;
  daemon?: DaemonConfig;
}

export interface DaemonStatus {
  running: boolean;
  docker_connected: boolean;
  api_port: number;
  api_url?: string;
  backup_schedule_enabled: boolean;
  backup_interval_hours: number;
  telemetry_interval_minutes: number;
}

export interface DaemonConfig {
  port: number;
  api_key: string;
  backup_schedule: BackupScheduleConfig;
  telemetry_interval_minutes: number;
}

export interface BackupScheduleConfig {
  enabled: boolean;
  interval_hours: number;
  backup_type: string;
}

export interface TelemetrySnapshot {
  timestamp: string;
  cpu_percent: number;
  ram_gb: number;
  disk_percent: number;
  services: Record<string, { status: string; health_ms?: number }>;
}

// Release management types
export interface NucleusUpdateInfo {
  update_available: boolean;
  current_version: string;
  latest_version: string;
  release_date: string;
  release_notes: string;
  download_size_mb?: number;
}

export interface ServiceUpdateInfo {
  service: string;
  current_version: string;
  latest_version: string;
  update_available: boolean;
  release_date: string;
  changelog: string;
  size_mb: number;
}

export interface DownloadResult {
  success: boolean;
  file_path: string;
  size_mb: number;
}

export interface ServiceManifest {
  service: string;
  latest_version: string;
  versions: ServiceVersionInfo[];
}

export interface ServiceVersionInfo {
  version: string;
  release_date: string;
  jar: string;
  sha256: string;
  size_mb: number;
  docker_tag: string;
  min_java: string;
  changelog: string;
}

// Docker update types
export interface DockerUpdateResult {
  service: string;
  previous_image: string;
  new_image: string;
  success: boolean;
  rolled_back: boolean;
  message: string;
  duration_seconds: number;
}

export interface UpdateRecord {
  service: string;
  previous_image: string;
  new_image: string;
  success: boolean;
  rolled_back: boolean;
  timestamp: string;
  duration_seconds: number;
  message: string;
}

// Remote shell types
export interface ShellResult {
  command: string;
  stdout: string;
  stderr: string;
  exit_code: number;
  duration_ms: number;
}

export interface ShellAuditEntry {
  command: string;
  exit_code: number;
  duration_ms: number;
  timestamp: string;
  success: boolean;
}
