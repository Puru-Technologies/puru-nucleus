import { Component, OnInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { MatCardModule } from '@angular/material/card';
import { MatIconModule } from '@angular/material/icon';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';
import { MatSlideToggleModule } from '@angular/material/slide-toggle';
import { MatDividerModule } from '@angular/material/divider';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
import { TauriService, NucleusConfig, DaemonStatus, DaemonConfig, PullSettingsResult } from '../../core/services/tauri.service';
import { NotificationService } from '../../core/services/notification.service';
import { open } from '@tauri-apps/plugin-dialog';

@Component({
  selector: 'app-settings',
  standalone: true,
  imports: [
    CommonModule,
    FormsModule,
    MatCardModule,
    MatIconModule,
    MatButtonModule,
    MatFormFieldModule,
    MatInputModule,
    MatSlideToggleModule,
    MatDividerModule,
    MatProgressSpinnerModule
  ],
  template: `
    <div class="settings-page p-4">
      <h1>Settings</h1>

      @if (loading) {
        <div class="loading-container">
          <mat-spinner diameter="48"></mat-spinner>
        </div>
      } @else if (config) {
        <div class="settings-grid">
          <!-- General Settings -->
          <mat-card class="settings-card">
            <mat-card-header>
              <mat-icon mat-card-avatar>settings</mat-icon>
              <mat-card-title>General</mat-card-title>
            </mat-card-header>
            <mat-card-content>
              <mat-form-field appearance="outline" class="full-width">
                <mat-label>Hospital Code</mat-label>
                <input matInput [(ngModel)]="config.hospital_code" readonly>
              </mat-form-field>

              <mat-form-field appearance="outline" class="full-width">
                <mat-label>Server IP</mat-label>
                <input matInput [(ngModel)]="config.server_ip">
              </mat-form-field>

              <mat-form-field appearance="outline" class="full-width">
                <mat-label>Docker Compose Path</mat-label>
                <input matInput [(ngModel)]="config.docker_compose_path">
              </mat-form-field>
            </mat-card-content>
          </mat-card>

          <!-- Cloud Settings -->
          <mat-card class="settings-card">
            <mat-card-header>
              <mat-icon mat-card-avatar>cloud</mat-icon>
              <mat-card-title>Cloud</mat-card-title>
            </mat-card-header>
            <mat-card-content>
              <div class="creds-row">
                @if (credsExists) {
                  <div class="creds-status ok">
                    <mat-icon>check_circle</mat-icon>
                    <span>{{ credsPath }}</span>
                  </div>
                } @else {
                  <div class="creds-status missing">
                    <mat-icon>warning</mat-icon>
                    <span>No credentials file found</span>
                  </div>
                }
                <button mat-stroked-button (click)="browseCredentials()">
                  <mat-icon>folder_open</mat-icon>
                  {{ credsExists ? 'Replace' : 'Browse' }}
                </button>
              </div>

              <div class="toggle-row">
                <mat-slide-toggle [(ngModel)]="config.backup_enabled">
                  Enable Cloud Backups
                </mat-slide-toggle>
                <p class="toggle-description">
                  Automatically upload backups to Google Cloud Storage
                </p>
              </div>

              <mat-divider></mat-divider>

              <div class="toggle-row">
                <mat-slide-toggle [(ngModel)]="config.telemetry_enabled">
                  Enable Telemetry
                </mat-slide-toggle>
                <p class="toggle-description">
                  Send health metrics and status updates to cloud dashboard
                </p>
              </div>
            </mat-card-content>
          </mat-card>

          <!-- LAN Backup Settings -->
          <mat-card class="settings-card">
            <mat-card-header>
              <mat-icon mat-card-avatar>lan</mat-icon>
              <mat-card-title>LAN Backup</mat-card-title>
              <mat-card-subtitle>Copy backups to network share (NFS/SMB)</mat-card-subtitle>
            </mat-card-header>
            <mat-card-content>
              <div class="toggle-row">
                <mat-slide-toggle [(ngModel)]="config.lan.enabled">
                  Enable LAN Backup
                </mat-slide-toggle>
                <p class="toggle-description">
                  Copy backup files to a local network share
                </p>
              </div>

              @if (config.lan.enabled) {
                <mat-form-field appearance="outline" class="full-width">
                  <mat-label>LAN Path</mat-label>
                  <input matInput [(ngModel)]="config.lan.path"
                         placeholder="/mnt/nas/backups or Z:\\backups">
                </mat-form-field>

                <div class="lan-validate-row">
                  <button mat-stroked-button (click)="validateLanPath()" [disabled]="lanValidating || !config.lan.path">
                    @if (lanValidating) {
                      <mat-spinner diameter="18"></mat-spinner>
                    } @else {
                      <mat-icon>verified</mat-icon>
                    }
                    Validate
                  </button>
                  @if (lanValidationResult) {
                    <span class="lan-validation ok">
                      <mat-icon>check_circle</mat-icon> Path is writable
                    </span>
                  }
                  @if (lanValidationError) {
                    <span class="lan-validation error">
                      <mat-icon>error</mat-icon> {{ lanValidationError }}
                    </span>
                  }
                </div>

                <mat-divider></mat-divider>

                <div class="toggle-row">
                  <mat-slide-toggle [(ngModel)]="config.lan.binlog_enabled">
                    Enable LAN Binlog Shipping
                  </mat-slide-toggle>
                  <p class="toggle-description">
                    Ship MySQL binary logs to the LAN path for point-in-time recovery
                  </p>
                </div>
              }
            </mat-card-content>
          </mat-card>

          <!-- Database Settings -->
          <mat-card class="settings-card">
            <mat-card-header>
              <mat-icon mat-card-avatar>storage</mat-icon>
              <mat-card-title>Database</mat-card-title>
            </mat-card-header>
            <mat-card-content>
              <mat-form-field appearance="outline" class="full-width">
                <mat-label>MySQL Host</mat-label>
                <input matInput [(ngModel)]="config.mysql_host" placeholder="127.0.0.1">
              </mat-form-field>

              <mat-form-field appearance="outline" class="full-width">
                <mat-label>MySQL Port</mat-label>
                <input matInput type="number" [(ngModel)]="config.mysql_port" placeholder="3306">
              </mat-form-field>

              <mat-form-field appearance="outline" class="full-width">
                <mat-label>MySQL User</mat-label>
                <input matInput [(ngModel)]="config.mysql_user" placeholder="root">
              </mat-form-field>

              <mat-form-field appearance="outline" class="full-width">
                <mat-label>MySQL Password</mat-label>
                <input matInput type="password" [(ngModel)]="config.mysql_password">
              </mat-form-field>
            </mat-card-content>
          </mat-card>

          <!-- Cloud Sync -->
          <mat-card class="settings-card">
            <mat-card-header>
              <mat-icon mat-card-avatar>sync</mat-icon>
              <mat-card-title>Cloud Sync</mat-card-title>
              <mat-card-subtitle>Push config to cloud / Pull settings from cloud</mat-card-subtitle>
            </mat-card-header>
            <mat-card-content>
              @if (syncStatus) {
                <div class="sync-status">
                  <div class="sync-info">
                    <span class="label">Last Synced:</span>
                    <span>{{ syncStatus.lastSynced | date:'short' }}</span>
                  </div>
                  @if (syncStatus.pendingChanges > 0) {
                    <div class="pending-warning">
                      <mat-icon>warning</mat-icon>
                      <span>{{ syncStatus.pendingChanges }} pending changes</span>
                    </div>
                  }
                </div>
              }
              <div class="sync-actions">
                <button mat-stroked-button (click)="syncConfig()" [disabled]="syncing">
                  @if (syncing) {
                    <mat-spinner diameter="18"></mat-spinner>
                  } @else {
                    <mat-icon>cloud_upload</mat-icon>
                  }
                  Sync to Cloud
                </button>
                <button mat-stroked-button (click)="pullSettings()" [disabled]="pulling">
                  @if (pulling) {
                    <mat-spinner diameter="18"></mat-spinner>
                  } @else {
                    <mat-icon>cloud_download</mat-icon>
                  }
                  Pull from Cloud
                </button>
              </div>
              @if (pullResult) {
                <div class="pull-result">
                  <div class="pull-info">
                    <span class="label">Hospital:</span>
                    <span>{{ pullResult.hospital_info.name }}</span>
                  </div>
                  @if (pullResult.hospital_info.city) {
                    <div class="pull-info">
                      <span class="label">City:</span>
                      <span>{{ pullResult.hospital_info.city }}</span>
                    </div>
                  }
                  @if (pullResult.license_changed) {
                    <div class="pull-updated">
                      <mat-icon>check_circle</mat-icon>
                      <span>License updated from cloud</span>
                    </div>
                  } @else {
                    <div class="pull-info">
                      <span>License is up to date</span>
                    </div>
                  }
                </div>
              }
            </mat-card-content>
          </mat-card>

          <!-- Daemon Status -->
          <mat-card class="settings-card">
            <mat-card-header>
              <mat-icon mat-card-avatar>memory</mat-icon>
              <mat-card-title>Daemon</mat-card-title>
              <mat-card-subtitle>REST API server &amp; background tasks</mat-card-subtitle>
            </mat-card-header>
            <mat-card-content>
              <div class="daemon-status">
                <div class="status-indicator" [class.running]="daemonRunning">
                  <mat-icon>{{ daemonRunning ? 'check_circle' : 'error' }}</mat-icon>
                  <span>{{ daemonRunning ? 'Running' : 'Stopped' }}</span>
                  @if (daemonRunning && daemonStatusInfo?.api_url) {
                    <span class="daemon-url">{{ daemonStatusInfo?.api_url }}</span>
                  }
                </div>
                <div class="daemon-actions">
                  @if (daemonRunning) {
                    <button mat-stroked-button (click)="stopDaemon()">Stop</button>
                  } @else {
                    <button mat-raised-button color="primary" (click)="startDaemon()">Start</button>
                  }
                  <button mat-stroked-button (click)="restartDaemon()">Restart</button>
                </div>
              </div>

              <mat-divider></mat-divider>

              @if (config) {
                <div class="daemon-config-section">
                  <mat-form-field appearance="outline" class="full-width">
                    <mat-label>API Port</mat-label>
                    <input matInput type="number" [(ngModel)]="daemonPort" placeholder="9090">
                  </mat-form-field>

                  <mat-form-field appearance="outline" class="full-width">
                    <mat-label>API Key</mat-label>
                    <input matInput [type]="showApiKey ? 'text' : 'password'" [(ngModel)]="daemonApiKey" placeholder="Leave empty for no auth">
                    <button mat-icon-button matSuffix (click)="showApiKey = !showApiKey">
                      <mat-icon>{{ showApiKey ? 'visibility_off' : 'visibility' }}</mat-icon>
                    </button>
                  </mat-form-field>

                  <div class="toggle-row">
                    <mat-slide-toggle [(ngModel)]="daemonBackupEnabled">
                      Scheduled Backups
                    </mat-slide-toggle>
                  </div>

                  @if (daemonBackupEnabled) {
                    <div class="inline-fields">
                      <mat-form-field appearance="outline">
                        <mat-label>Interval (hours)</mat-label>
                        <input matInput type="number" [(ngModel)]="daemonBackupIntervalHours" min="1">
                      </mat-form-field>
                      <mat-form-field appearance="outline">
                        <mat-label>Backup Type</mat-label>
                        <input matInput [(ngModel)]="daemonBackupType" placeholder="full">
                      </mat-form-field>
                    </div>
                  }

                  <mat-form-field appearance="outline" class="full-width">
                    <mat-label>Telemetry Interval (minutes)</mat-label>
                    <input matInput type="number" [(ngModel)]="daemonTelemetryMinutes" min="0" placeholder="15 (0 = disabled)">
                  </mat-form-field>
                </div>
              }
            </mat-card-content>
          </mat-card>
        </div>

        <div class="actions">
          <button mat-raised-button color="primary" (click)="saveConfig()">
            <mat-icon>save</mat-icon>
            Save Changes
          </button>
          <button mat-stroked-button (click)="loadConfig()">
            <mat-icon>refresh</mat-icon>
            Reset
          </button>
        </div>
      }
    </div>
  `,
  styles: [`
    .settings-page {
      max-width: 1200px;
      margin: 0 auto;
    }

    h1 {
      margin-bottom: 1.5rem;
      color: #333;
    }

    .loading-container {
      display: flex;
      justify-content: center;
      padding: 3rem;
    }

    .settings-grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(400px, 1fr));
      gap: 1.5rem;
      margin-bottom: 1.5rem;
    }

    .settings-card {
      mat-card-header {
        margin-bottom: 1rem;
      }

      mat-icon[mat-card-avatar] {
        background: #e8eaf6;
        padding: 8px;
        border-radius: 50%;
        color: #3f51b5;
      }
    }

    .full-width {
      width: 100%;
      margin-bottom: 1rem;
    }

    .toggle-row {
      padding: 1rem 0;

      mat-slide-toggle {
        margin-bottom: 0.5rem;
      }

      .toggle-description {
        margin: 0;
        font-size: 0.875rem;
        color: #666;
        padding-left: 3rem;
      }
    }

    .sync-status {
      margin-bottom: 1rem;

      .sync-info {
        display: flex;
        gap: 0.5rem;
        margin-bottom: 0.5rem;

        .label {
          color: #666;
        }
      }

      .pending-warning {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        color: #ff9800;
        font-size: 0.875rem;
      }
    }

    .sync-actions {
      display: flex;
      gap: 0.75rem;
      margin-bottom: 1rem;
    }

    .pull-result {
      margin-top: 1rem;
      padding: 0.75rem;
      background: #f5f5f5;
      border-radius: 8px;

      .pull-info {
        display: flex;
        gap: 0.5rem;
        margin-bottom: 0.25rem;
        font-size: 0.875rem;

        .label {
          color: #666;
        }
      }

      .pull-updated {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        color: #4caf50;
        font-size: 0.875rem;
        margin-top: 0.5rem;

        mat-icon {
          font-size: 18px;
          width: 18px;
          height: 18px;
        }
      }
    }

    .daemon-status {
      .status-indicator {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        margin-bottom: 1rem;
        font-weight: 500;

        &.running {
          color: #4caf50;
        }

        &:not(.running) {
          color: #f44336;
        }

        .daemon-url {
          font-size: 0.7rem;
          font-family: 'SF Mono', 'Fira Code', monospace;
          color: #666;
          font-weight: 400;
          margin-left: 0.5rem;
        }
      }

      .daemon-actions {
        display: flex;
        gap: 0.75rem;
        margin-bottom: 1rem;
      }
    }

    .daemon-config-section {
      padding-top: 1rem;

      .inline-fields {
        display: flex;
        gap: 1rem;

        mat-form-field {
          flex: 1;
        }
      }
    }

    .creds-row {
      display: flex;
      align-items: center;
      gap: 12px;
      margin-bottom: 1rem;
      padding: 8px 0;
    }

    .creds-status {
      flex: 1;
      display: flex;
      align-items: center;
      gap: 8px;
      font-size: 0.8rem;

      mat-icon {
        font-size: 18px;
        width: 18px;
        height: 18px;
      }

      span {
        font-family: 'SF Mono', 'Fira Code', monospace;
        font-size: 0.7rem;
        word-break: break-all;
      }

      &.ok {
        color: #4caf50;
      }

      &.missing {
        color: #ff9800;
      }
    }

    .lan-validate-row {
      display: flex;
      align-items: center;
      gap: 1rem;
      margin-bottom: 1rem;
    }

    .lan-validation {
      display: flex;
      align-items: center;
      gap: 4px;
      font-size: 0.85rem;

      mat-icon {
        font-size: 18px;
        width: 18px;
        height: 18px;
      }

      &.ok {
        color: #4caf50;
      }

      &.error {
        color: #f44336;
      }
    }

    .actions {
      display: flex;
      gap: 0.75rem;
      justify-content: flex-end;
      padding-top: 1rem;
      border-top: 1px solid #eee;
    }
  `]
})
export class SettingsComponent implements OnInit {
  private tauri = inject(TauriService);
  private notification = inject(NotificationService);

  config: NucleusConfig | null = null;
  loading = true;
  syncing = false;
  daemonRunning = false;
  credsExists = false;
  credsPath = '';
  showApiKey = false;
  daemonStatusInfo: DaemonStatus | null = null;

  // Daemon config fields (bound to UI, synced to config on save)
  daemonPort = 9090;
  daemonApiKey = '';
  daemonBackupEnabled = true;
  daemonBackupIntervalHours = 24;
  daemonBackupType = 'full';
  daemonTelemetryMinutes = 15;

  syncStatus: { lastSynced: Date; pendingChanges: number } | null = null;
  pulling = false;
  pullResult: PullSettingsResult | null = null;

  // LAN validation
  lanValidating = false;
  lanValidationResult = false;
  lanValidationError = '';

  ngOnInit(): void {
    this.loadConfig();
    this.checkDaemonStatus();
    this.loadSyncStatus();
    this.checkCredentials();
  }

  async checkCredentials(): Promise<void> {
    try {
      const status = await this.tauri.invokeSilent<{ exists: boolean; path: string }>('check_credentials_file');
      this.credsExists = status.exists;
      this.credsPath = status.path;
    } catch {
      // Ignore
    }
  }

  async browseCredentials(): Promise<void> {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: 'JSON', extensions: ['json'] }],
        title: 'Select GCS Service Account JSON'
      });
      if (!selected) return;
      const filePath = typeof selected === 'string' ? selected : selected;
      await this.tauri.invoke('import_credentials_file', { sourcePath: filePath });
      this.notification.success('Credentials file updated');
      await this.checkCredentials();
    } catch (error) {
      // Error handled by TauriService
    }
  }

  async loadConfig(): Promise<void> {
    this.loading = true;
    try {
      this.config = await this.tauri.invoke<NucleusConfig>('get_config');
      // Populate daemon config fields from loaded config
      if (this.config.daemon) {
        this.daemonPort = this.config.daemon.port;
        this.daemonApiKey = this.config.daemon.api_key;
        this.daemonBackupEnabled = this.config.daemon.backup_schedule.enabled;
        this.daemonBackupIntervalHours = this.config.daemon.backup_schedule.interval_hours;
        this.daemonBackupType = this.config.daemon.backup_schedule.backup_type;
        this.daemonTelemetryMinutes = this.config.daemon.telemetry_interval_minutes;
      }
    } finally {
      this.loading = false;
    }
  }

  async saveConfig(): Promise<void> {
    if (!this.config) return;

    // Sync daemon config fields back to config object
    this.config.daemon = {
      port: this.daemonPort,
      api_key: this.daemonApiKey,
      backup_schedule: {
        enabled: this.daemonBackupEnabled,
        interval_hours: this.daemonBackupIntervalHours,
        backup_type: this.daemonBackupType
      },
      telemetry_interval_minutes: this.daemonTelemetryMinutes
    };

    try {
      await this.tauri.invoke('save_config', { config: this.config });
      this.notification.success('Settings saved');
    } catch (error) {
      // Error handled by TauriService
    }
  }

  async syncConfig(): Promise<void> {
    this.syncing = true;
    try {
      await this.tauri.invoke('sync_config_to_cloud');
      this.notification.success('Config synced to cloud');
      await this.loadSyncStatus();
    } finally {
      this.syncing = false;
    }
  }

  async pullSettings(): Promise<void> {
    this.pulling = true;
    this.pullResult = null;
    try {
      this.pullResult = await this.tauri.invoke<PullSettingsResult>('pull_settings');
      if (this.pullResult.license_changed) {
        this.notification.success('License updated from cloud');
      } else {
        this.notification.success('Settings pulled — license is up to date');
      }
    } catch (error) {
      // Error handled by TauriService
    } finally {
      this.pulling = false;
    }
  }

  async loadSyncStatus(): Promise<void> {
    try {
      const status = await this.tauri.invokeSilent<{ last_synced: string; pending_count: number }>('get_sync_status');
      this.syncStatus = {
        lastSynced: new Date(status.last_synced),
        pendingChanges: status.pending_count
      };
    } catch {
      this.syncStatus = null;
    }
  }

  async checkDaemonStatus(): Promise<void> {
    try {
      const status = await this.tauri.invokeSilent<DaemonStatus>('get_daemon_status');
      this.daemonRunning = status.running;
      this.daemonStatusInfo = status;
    } catch {
      this.daemonRunning = false;
      this.daemonStatusInfo = null;
    }
  }

  async startDaemon(): Promise<void> {
    try {
      await this.tauri.invoke('start_daemon');
      this.daemonRunning = true;
      this.notification.success('Daemon started');
    } catch (error) {
      // Error handled by TauriService
    }
  }

  async stopDaemon(): Promise<void> {
    try {
      await this.tauri.invoke('stop_daemon');
      this.daemonRunning = false;
      this.notification.success('Daemon stopped');
    } catch (error) {
      // Error handled by TauriService
    }
  }

  async validateLanPath(): Promise<void> {
    if (!this.config?.lan.path) return;
    this.lanValidating = true;
    this.lanValidationResult = false;
    this.lanValidationError = '';

    try {
      await this.tauri.invoke('validate_lan_path', { path: this.config.lan.path });
      this.lanValidationResult = true;
    } catch (error) {
      this.lanValidationError = error as string;
    } finally {
      this.lanValidating = false;
    }
  }

  async restartDaemon(): Promise<void> {
    try {
      await this.tauri.invoke('restart_daemon');
      this.daemonRunning = true;
      this.notification.success('Daemon restarted');
    } catch (error) {
      // Error handled by TauriService
    }
  }
}
