import { Injectable, inject } from '@angular/core';
import { invoke } from '@tauri-apps/api/core';
import { ErrorSeverity } from '../models/app-error';
import { ConnectionService } from './connection.service';
import { RemoteTransportService } from './remote-transport';
import { NotificationService } from './notification.service';

/**
 * Service for invoking Tauri commands with error handling.
 *
 * Transport-aware: in local mode it calls Tauri IPC; in remote mode it routes
 * to the connected daemon's REST API via {@link RemoteTransportService}.
 */
@Injectable({
  providedIn: 'root'
})
export class TauriService {
  private conn = inject(ConnectionService);
  private remote = inject(RemoteTransportService);
  private notify = inject(NotificationService);

  /**
   * Invoke a Tauri command with automatic error handling
   */
  async invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    try {
      if (this.conn.isRemote()) {
        return await this.remote.execute<T>(command, args ?? {});
      }
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
    if (this.conn.isRemote()) {
      return this.remote.execute<T>(command, args ?? {});
    }
    return invoke<T>(command, args);
  }

  private handleError(message: string, command: string): void {
    const severity = this.parseSeverity(message);
    const action = this.getSuggestedAction(message);

    this.notify.show({
      message: action ? `${message} (${action})` : message,
      severity,
      duration: severity === 'critical' ? 0 : 5000,
    });

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
  get_container_logs: (args: { containerName: string; tail?: number; since?: number; until?: number }) => Promise<string>;

  // Log file reader
  get_log_sources: () => Promise<LogSource[]>;
  list_log_files: (args: { path?: string }) => Promise<LogFileInfo[]>;
  read_log_file: (args: { path: string; tail?: number; offset?: number; limit?: number }) => Promise<LogFileContent>;

  // Telemetry
  get_telemetry_snapshot: () => Promise<TelemetrySnapshot>;

  // Pull settings
  pull_settings: () => Promise<PullSettingsResult>;

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
  installable?: boolean;
}

export interface InstallProgress {
  software: string;
  stage: 'downloading' | 'installing' | 'verifying' | 'completed' | 'failed';
  percent: number;
  message: string;
  bytes_downloaded: number;
  bytes_total: number;
}

export interface InstallResult {
  software: string;
  success: boolean;
  version?: string;
  error?: string;
}

export interface ServiceInfo {
  name: string;
  container_name: string;
  image: string;
  status: 'running' | 'stopped' | 'starting' | 'notinstalled' | 'error';
  health?: 'healthy' | 'unhealthy' | 'starting';
  ports: string[];
  uptime?: string;
  health_response_ms?: number;
  detail?: string;
}

export interface ProcessInfo {
  pid: number;
  name: string;
  label: string;
  cmd: string;
  cpu_pct: number;
  mem_mb: number;
  listening_ports: number[];
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
  lan_copied: boolean;
  error?: string;
}

export interface LanConfig {
  enabled: boolean;
  path: string;
  binlog_enabled: boolean;
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
  deployment_mode: 'docker' | 'native';
  jars_dir?: string;
  jres_dir?: string;
  native_logs_dir?: string;
  puru_data_path?: string;
  daemon?: DaemonConfig;
  lan: LanConfig;
  setup_completed?: boolean;
  production_mode?: boolean;
}

export interface DaemonStatus {
  running: boolean;
  docker_connected: boolean;
  api_port: number;
  api_url?: string;
  backup_schedule_enabled: boolean;
  backup_interval_hours: number;
  telemetry_interval_minutes: number;
  service_installed: boolean;
  service_enabled: boolean;
  service_pid?: number;
  service_detail: string;
  platform: string;
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
  backup_time?: string | null;
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

// Binlog types
export interface BinlogStatus {
  lan_enabled: boolean;
  lan_last_shipped_file?: string;
  lan_last_shipped_at?: string;
  lan_total_shipped: number;
  lan_last_error?: string;
  current_master_file?: string;
  current_master_position?: number;
  files_pending: number;
}

export interface BinlogShipResult {
  files_shipped: number;
  bytes_shipped: number;
  duration_seconds: number;
  errors: string[];
}

// Log file reader types
export interface LogSource {
  name: string;
  /** Host path for kind=directory; "container:<name>" pseudo-path for kind=container. */
  path: string;
  /** Free-form grouping tag — "nucleus" | "system" | "docker" | "container" etc. */
  source_type: string;
  /** "directory" (path contains log files) or "container" (path is a docker container). */
  kind: 'directory' | 'container';
}

export interface LogFileInfo {
  name: string;
  path: string;
  size_bytes: number;
  modified_at: string;
}

export interface LogFileContent {
  path: string;
  content: string;
  total_lines: number;
  offset: number;
  lines_returned: number;
}

/** Payload pushed on "log-tail-{stream_id}" events. `ended` is set on the
 *  final event (file removed, container exited, or stream stopped). */
export interface LogTailEvent {
  stream_id: string;
  lines: string[];
  ended?: string;
}

// Compose template types
export interface FragmentDownloadResult {
  downloaded: string[];
  skipped: string[];
  fragments_dir: string;
}

export interface TemplateVariables {
  hospital_code: string;
  hospital_name: string;
  hospital_line2: string;
  hospital_line3: string;
  hospital_reg_no: string;
  hospital_logo_url: string;
  barcode_prefix_inventory: string;
  barcode_prefix_ppin: string;
  barcode_prefix_pathology: string;
  employee_prefix: string;
  barcode_prefix_sale: string;
  barcode_prefix_return: string;
  server_ip: string;
  mysql_password: string;
  rabbitmq_password: string;
  auth_tag: string;
  xenon_tag: string;
  has_tag: string;
  pacs_tag: string;
  argon_tag: string;
  comm_tag: string;
  realtime_tag: string;
  neon_tag: string;
  bridge_tag: string;
  integration_tag: string;
  hydrogen_tag: string;
}

export interface ServiceModules {
  auth: boolean;
  xenon: boolean;
  has: boolean;
  pacs: boolean;
  argon: boolean;
  comm: boolean;
  realtime: boolean;
  neon: boolean;
  mercury: boolean;
  counter: boolean;
  bridge: boolean;
  integration: boolean;
  hydrogen: boolean;
}

export interface ComposeUploadResult {
  success: boolean;
  gcs_path: string;
}

export interface EnvFileEntry {
  name: string;
  content: string;
}

export interface EnvDownloadResult {
  files_downloaded: string[];
  files_skipped: string[];
  env_dir: string;
}

export interface EnvUploadResult {
  success: boolean;
  files_uploaded: string[];
}

export interface HospitalInfo {
  name: string;
  short_name?: string;
  city?: string;
  email?: string;
}

export interface PullSettingsResult {
  hospital_info: HospitalInfo;
  license: import('../models/license.model').License;
  license_changed: boolean;
}

// Seeding types
export interface SeedSection {
  name: string;
  created: number;
  skipped: number;
  errors: string[];
}

export interface SeedReport {
  sections: SeedSection[];
}

export interface FinaliseReport {
  uploaded: number;
  prefix: string;
  errors: string[];
}

export interface TemplateUpdate {
  path: string;
  status: 'new' | 'changed';
  old_size: number;
  new_size: number;
}

export interface ApplyReport {
  applied: number;
  errors: string[];
}

// Network types
export interface NetworkStatus {
  connected: boolean;
  latency_ms: number | null;
  gcp_reachable: boolean;
  gcp_latency_ms: number | null;
  checked_at: string;
}

export interface SpeedTestResult {
  connected: boolean;
  latency_ms: number | null;
  gcp_reachable: boolean;
  gcp_latency_ms: number | null;
  download_mbps: number | null;
  upload_mbps: number | null;
  download_bytes: number;
  upload_bytes: number;
  duration_ms: number;
  tested_at: string;
}
