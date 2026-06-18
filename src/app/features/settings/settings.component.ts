import { Component, OnInit, inject, ViewChild, ElementRef } from '@angular/core';
import { CommonModule } from '@angular/common';
import { resetLicenseCache } from '../../core/guards/init.guard';
import { FormsModule } from '@angular/forms';
import { TauriService, NucleusConfig, DaemonStatus, DaemonConfig, PullSettingsResult, SeedReport, FinaliseReport, TemplateUpdate, ApplyReport } from '../../core/services/tauri.service';
import { NotificationService } from '../../core/services/notification.service';
import { ConnectionService } from '../../core/services/connection.service';
import { open } from '@tauri-apps/plugin-dialog';

@Component({
  selector: 'app-settings',
  standalone: true,
  imports: [
    CommonModule,
    FormsModule
  ],
  template: `
    <div class="settings-page p-4">
      <h1>Settings</h1>

      @if (loading) {
        <div class="loading-container">
          <span class="spinner spinner-lg"></span>
        </div>
      } @else if (config) {
        <div class="settings-grid">
          <!-- General Settings -->
          <div class="card card-pad settings-card">
            <div class="card-head">
              <span class="material-icons card-avatar">settings</span>
              <h2 class="card-title">General</h2>
            </div>
            <div class="card-body">
              <div class="field full-width">
                <label>Hospital Code</label>
                <input class="input" [(ngModel)]="config.hospital_code" readonly>
              </div>

              <div class="mode-display">
                <span class="mode-label">Mode:</span>
                <span class="mode-value" [class.native]="config.deployment_mode === 'native'">
                  {{ config.deployment_mode === 'native' ? 'Native (JAR)' : 'Docker' }}
                </span>
              </div>

              <div class="field full-width">
                <label>Server IP</label>
                <input class="input" [(ngModel)]="config.server_ip">
              </div>

              @if (config.deployment_mode !== 'native') {
                <div class="field full-width">
                  <label>Docker Compose Path</label>
                  <input class="input" [(ngModel)]="config.docker_compose_path">
                </div>
              }
            </div>
          </div>

          <!-- Cloud Settings -->
          <div class="card card-pad settings-card">
            <div class="card-head">
              <span class="material-icons card-avatar">cloud</span>
              <h2 class="card-title">Cloud</h2>
            </div>
            <div class="card-body">
              <div class="creds-row">
                @if (credsExists) {
                  <div class="creds-status ok">
                    <span class="material-icons">check_circle</span>
                    <span>{{ credsPath }}</span>
                  </div>
                } @else {
                  <div class="creds-status missing">
                    <span class="material-icons">warning</span>
                    <span>No credentials file found</span>
                  </div>
                }
                @if (!isRemote) {
                  <button class="btn btn-stroked" (click)="browseCredentials()">
                    <span class="material-icons">folder_open</span>
                    {{ credsExists ? 'Replace' : 'Browse' }}
                  </button>
                }
              </div>

              <div class="toggle-row">
                <label class="check">
                  <input type="checkbox" [(ngModel)]="config.backup_enabled">
                  <span>Enable Cloud Backups</span>
                </label>
                <p class="toggle-description">
                  Automatically upload backups to Google Cloud Storage
                </p>
              </div>

              <div class="divider"></div>

              <div class="toggle-row">
                <label class="check">
                  <input type="checkbox" [(ngModel)]="config.telemetry_enabled">
                  <span>Enable Telemetry</span>
                </label>
                <p class="toggle-description">
                  Send health metrics and status updates to cloud dashboard
                </p>
              </div>
            </div>
          </div>

          <!-- LAN Backup Settings -->
          <div class="card card-pad settings-card">
            <div class="card-head">
              <span class="material-icons card-avatar">lan</span>
              <div class="card-title-group">
                <h2 class="card-title">LAN Backup</h2>
                <div class="card-subtitle">Copy backups to network share (NFS/SMB)</div>
              </div>
            </div>
            <div class="card-body">
              <div class="toggle-row">
                <label class="check">
                  <input type="checkbox" [(ngModel)]="config.lan.enabled">
                  <span>Enable LAN Backup</span>
                </label>
                <p class="toggle-description">
                  Copy backup files to a local network share
                </p>
              </div>

              @if (config.lan.enabled) {
                <div class="field full-width">
                  <label>LAN Path</label>
                  <input class="input" [(ngModel)]="config.lan.path"
                         placeholder="/mnt/nas/backups or Z:\\backups">
                </div>

                <div class="lan-validate-row">
                  <button class="btn btn-stroked" (click)="validateLanPath()" [disabled]="lanValidating || !config.lan.path">
                    @if (lanValidating) {
                      <span class="spinner" style="width:14px;height:14px;border-width:2px"></span>
                    } @else {
                      <span class="material-icons">verified</span>
                    }
                    Validate
                  </button>
                  @if (lanValidationResult) {
                    <span class="lan-validation ok">
                      <span class="material-icons">check_circle</span> Path is writable
                    </span>
                  }
                  @if (lanValidationError) {
                    <span class="lan-validation error">
                      <span class="material-icons">error</span> {{ lanValidationError }}
                    </span>
                  }
                </div>

                <div class="divider"></div>

                <div class="toggle-row">
                  <label class="check">
                    <input type="checkbox" [(ngModel)]="config.lan.binlog_enabled">
                    <span>Enable LAN Binlog Shipping</span>
                  </label>
                  <p class="toggle-description">
                    Ship MySQL binary logs to the LAN path for point-in-time recovery
                  </p>
                </div>
              }
            </div>
          </div>

          <!-- Database Settings -->
          <div class="card card-pad settings-card">
            <div class="card-head">
              <span class="material-icons card-avatar">storage</span>
              <h2 class="card-title">Database</h2>
            </div>
            <div class="card-body">
              <div class="field full-width">
                <label>MySQL Host</label>
                <input class="input" [(ngModel)]="config.mysql_host" placeholder="127.0.0.1">
              </div>

              <div class="field full-width">
                <label>MySQL Port</label>
                <input class="input" type="number" [(ngModel)]="config.mysql_port" placeholder="3306">
              </div>

              <div class="field full-width">
                <label>MySQL User</label>
                <input class="input" [(ngModel)]="config.mysql_user" placeholder="root">
              </div>

              <div class="field full-width">
                <label>MySQL Password</label>
                <input class="input" type="password" [(ngModel)]="config.mysql_password">
              </div>
            </div>
          </div>

          <!-- Cloud Sync -->
          <div class="card card-pad settings-card">
            <div class="card-head">
              <span class="material-icons card-avatar">sync</span>
              <div class="card-title-group">
                <h2 class="card-title">Cloud Sync</h2>
                <div class="card-subtitle">Push config to cloud / Pull settings from cloud</div>
              </div>
            </div>
            <div class="card-body">
              @if (syncStatus) {
                <div class="sync-status">
                  <div class="sync-info">
                    <span class="label">Last Synced:</span>
                    <span>{{ syncStatus.lastSynced | date:'short' }}</span>
                  </div>
                  @if (syncStatus.pendingChanges > 0) {
                    <div class="pending-warning">
                      <span class="material-icons">warning</span>
                      <span>{{ syncStatus.pendingChanges }} pending changes</span>
                    </div>
                  }
                </div>
              }
              <div class="sync-actions">
                <button class="btn btn-stroked" (click)="syncConfig()" [disabled]="syncing">
                  @if (syncing) {
                    <span class="spinner" style="width:14px;height:14px;border-width:2px"></span>
                  } @else {
                    <span class="material-icons">cloud_upload</span>
                  }
                  Sync to Cloud
                </button>
                <button class="btn btn-stroked" (click)="pullSettings()" [disabled]="pulling">
                  @if (pulling) {
                    <span class="spinner" style="width:14px;height:14px;border-width:2px"></span>
                  } @else {
                    <span class="material-icons">cloud_download</span>
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
                      <span class="material-icons">check_circle</span>
                      <span>License updated from cloud</span>
                    </div>
                  } @else {
                    <div class="pull-info">
                      <span>License is up to date</span>
                    </div>
                  }
                </div>
              }
            </div>
          </div>

          <!-- Daemon Status -->
          <div class="card card-pad settings-card">
            <div class="card-head">
              <span class="material-icons card-avatar">memory</span>
              <div class="card-title-group">
                <h2 class="card-title">Daemon</h2>
                <div class="card-subtitle">{{ daemonStatusInfo?.platform || 'Background service' }}</div>
              </div>
            </div>
            <div class="card-body">
              @if (isRemote) {
                <div class="service-detail" style="padding:8px 0 12px">
                  Daemon lifecycle (install / start / stop) is managed on the server itself, not from a remote session.
                </div>
              }
              <!-- Service Installation Status -->
              @if (!isRemote) {
              <div class="service-status-panel">
                <div class="service-row">
                  <div class="service-label">
                    <span class="material-icons" [class]="daemonStatusInfo?.service_installed ? 'status-ok' : 'status-warn'">
                      {{ daemonStatusInfo?.service_installed ? 'check_circle' : 'cancel' }}
                    </span>
                    <span>Service {{ daemonStatusInfo?.service_installed ? 'Installed' : 'Not Installed' }}</span>
                  </div>
                  @if (!daemonStatusInfo?.service_installed) {
                    <button class="btn btn-primary" (click)="installDaemon()" [disabled]="daemonAction">
                      @if (daemonAction === 'installing') { <span class="spinner" style="width:14px;height:14px;border-width:2px"></span> }
                      <span class="material-icons">download</span> Install Service
                    </button>
                  } @else {
                    <button class="btn btn-danger" (click)="uninstallDaemon()" [disabled]="daemonAction">
                      Uninstall
                    </button>
                  }
                </div>

                @if (daemonStatusInfo?.service_installed) {
                  <div class="service-row">
                    <div class="service-label">
                      <span class="material-icons" [class]="daemonRunning ? 'status-ok' : 'status-warn'">
                        {{ daemonRunning ? 'play_circle' : 'stop_circle' }}
                      </span>
                      <span>{{ daemonRunning ? 'Running' : 'Stopped' }}</span>
                      @if (daemonStatusInfo?.service_pid) {
                        <span class="daemon-pid">PID {{ daemonStatusInfo?.service_pid }}</span>
                      }
                      @if (daemonRunning && daemonStatusInfo?.api_url) {
                        <span class="daemon-url">{{ daemonStatusInfo?.api_url }}</span>
                      }
                    </div>
                    <div class="daemon-actions">
                      @if (daemonRunning) {
                        <button class="btn btn-stroked" (click)="stopDaemon()" [disabled]="daemonAction">
                          @if (daemonAction === 'stopping') { <span class="spinner" style="width:14px;height:14px;border-width:2px"></span> }
                          Stop
                        </button>
                      } @else {
                        <button class="btn btn-primary" (click)="startDaemon()" [disabled]="daemonAction">
                          @if (daemonAction === 'starting') { <span class="spinner" style="width:14px;height:14px;border-width:2px"></span> }
                          Start
                        </button>
                      }
                      <button class="btn btn-stroked" (click)="restartDaemon()" [disabled]="daemonAction">
                        @if (daemonAction === 'restarting') { <span class="spinner" style="width:14px;height:14px;border-width:2px"></span> }
                        Restart
                      </button>
                      <button class="btn-icon" (click)="checkDaemonStatus()" [disabled]="daemonAction">
                        <span class="material-icons">refresh</span>
                      </button>
                    </div>
                  </div>

                  <div class="service-row">
                    <div class="service-label">
                      <span class="material-icons" [class]="daemonStatusInfo?.service_enabled ? 'status-ok' : 'status-muted'">
                        {{ daemonStatusInfo?.service_enabled ? 'autorenew' : 'block' }}
                      </span>
                      <span>Auto-start on boot: {{ daemonStatusInfo?.service_enabled ? 'Yes' : 'No' }}</span>
                    </div>
                  </div>

                  @if (config.deployment_mode !== 'native') {
                    <div class="service-row">
                      <div class="service-label">
                        <span class="material-icons" [class]="daemonStatusInfo?.docker_connected ? 'status-ok' : 'status-warn'">
                          {{ daemonStatusInfo?.docker_connected ? 'check_circle' : 'warning' }}
                        </span>
                        <span>Docker: {{ daemonStatusInfo?.docker_connected ? 'Connected' : 'Not connected' }}</span>
                      </div>
                    </div>
                  }
                }

                @if (daemonStatusInfo?.service_detail) {
                  <div class="service-detail">{{ daemonStatusInfo?.service_detail }}</div>
                }
              </div>
              }

              @if (daemonError) {
                <div class="daemon-error">
                  <span class="material-icons">error</span>
                  <span>{{ daemonError }}</span>
                  <button class="btn-icon" (click)="daemonError = null"><span class="material-icons">close</span></button>
                </div>
              }

              <div class="divider"></div>

              @if (config) {
                <div class="daemon-config-section">
                  <div class="field full-width">
                    <label>API Port</label>
                    <input class="input" type="number" [(ngModel)]="daemonPort" placeholder="9090">
                  </div>

                  <div class="field full-width">
                    <label>API Key</label>
                    <div class="input-with-suffix">
                      <input class="input" [type]="showApiKey ? 'text' : 'password'" [(ngModel)]="daemonApiKey" placeholder="Leave empty for no auth">
                      <button class="btn-icon" (click)="showApiKey = !showApiKey">
                        <span class="material-icons">{{ showApiKey ? 'visibility_off' : 'visibility' }}</span>
                      </button>
                    </div>
                  </div>

                  <div class="toggle-row">
                    <label class="check">
                      <input type="checkbox" [(ngModel)]="daemonBackupEnabled">
                      <span>Scheduled Backups</span>
                    </label>
                  </div>

                  @if (daemonBackupEnabled) {
                    <div class="inline-fields">
                      <div class="field">
                        <label>Backup Time (HH:MM)</label>
                        <input class="input" [(ngModel)]="daemonBackupTime" placeholder="02:00">
                        <span class="field-hint">Daily at this time (24h format). Leave empty for interval-based.</span>
                      </div>
                      <div class="field">
                        <label>Backup Type</label>
                        <input class="input" [(ngModel)]="daemonBackupType" placeholder="full">
                      </div>
                    </div>
                    <div class="field full-width">
                      <label>Fallback Interval (hours)</label>
                      <input class="input" type="number" [(ngModel)]="daemonBackupIntervalHours" min="1">
                      <span class="field-hint">Used if backup time is not set</span>
                    </div>
                  }

                  <div class="field full-width">
                    <label>Telemetry Interval (minutes)</label>
                    <input class="input" type="number" [(ngModel)]="daemonTelemetryMinutes" min="0" placeholder="15 (0 = disabled)">
                  </div>
                </div>
              }
            </div>
          </div>

          <!-- TLS / HTTPS Card -->
          <div class="card card-pad settings-card">
            <div class="card-head">
              <span class="material-icons card-avatar">lock</span>
              <div class="card-title-group">
                <h2 class="card-title">HTTPS / TLS</h2>
                <div class="card-subtitle">Secure connection for voice calls &amp; screen share</div>
              </div>
            </div>
            <div class="card-body">
              @if (tlsStatus) {
                <div class="tls-status-row">
                  <span class="material-icons" [class]="tlsStatus.configured ? 'status-ok' : 'status-warn'">
                    {{ tlsStatus.configured ? 'check_circle' : 'warning' }}
                  </span>
                  <span>{{ tlsStatus.configured ? 'HTTPS configured' : 'HTTPS not configured' }}</span>
                </div>

                @if (tlsStatus.configured) {
                  <div class="tls-details">
                    <span>CA: {{ tlsStatus.ca_download_url }}</span>
                    <span>Cert: {{ tlsStatus.cert_path }}</span>
                  </div>
                  <div class="tls-actions-row">
                    <button class="btn btn-stroked" (click)="downloadTlsScript()">
                      <span class="material-icons">download</span> Client Setup Script
                    </button>
                    <button class="btn btn-stroked" (click)="copyTlsNginxConfig()">
                      <span class="material-icons">content_copy</span> Nginx Config
                    </button>
                  </div>
                } @else {
                  <p class="tls-hint">
                    Requires <code>mkcert</code> installed on this server.
                    Run setup or click below to generate certificates.
                  </p>
                  <button class="btn btn-primary" (click)="runTlsSetup()" [disabled]="tlsSettingUp">
                    @if (tlsSettingUp) { <span class="spinner" style="width:14px;height:14px;border-width:2px"></span> }
                    <span class="material-icons">lock</span> Setup HTTPS
                  </button>
                }
              } @else {
                <div class="loading-inline">
                  <span class="spinner"></span>
                  <span>Checking TLS status...</span>
                </div>
              }
            </div>
          </div>
          <!-- Data Seeding -->
          <div class="card card-pad settings-card">
            <div class="card-head">
              <span class="material-icons card-avatar">spa</span>
              <div class="card-title-group">
                <h2 class="card-title">Data Seeding</h2>
                <div class="card-subtitle">Populate fresh-install defaults (idempotent — never overwrites existing values)</div>
              </div>
            </div>
            <div class="card-body">
              <p class="seed-hint">
                <span class="material-icons">info_outline</span>
                Run once after the services have started for the first time, so their database tables exist.
              </p>

              <button class="btn btn-primary" (click)="seedAll()" [disabled]="!!seeding">
                @if (seeding === 'all') { <span class="spinner" style="width:14px;height:14px;border-width:2px"></span> }
                <span class="material-icons">auto_fix_high</span> Seed All
              </button>

              <div class="seed-advanced">
                <div class="seed-advanced-header">Advanced — seed individually</div>
                <div class="toggle-row compact">
                  <label class="check"><input type="checkbox" [(ngModel)]="seedDb"><span>Databases (config, ref data, document master)</span></label>
                  <label class="check"><input type="checkbox" [(ngModel)]="seedQueues"><span>RabbitMQ Queues</span></label>
                  <label class="check"><input type="checkbox" [(ngModel)]="seedTemplates"><span>Report Templates</span></label>
                </div>
                <button class="btn btn-stroked" (click)="seedSelected()"
                        [disabled]="!!seeding || (!seedDb && !seedQueues && !seedTemplates)">
                  @if (seeding === 'selected') { <span class="spinner" style="width:14px;height:14px;border-width:2px"></span> }
                  <span class="material-icons">play_arrow</span> Seed Selected
                </button>
              </div>

              @if (seedReport) {
                <div class="seed-results">
                  @for (section of seedReport.sections; track section.name) {
                    <div class="seed-section" [class.has-errors]="section.errors.length > 0">
                      <div class="seed-section-head">
                        <span class="material-icons">{{ section.errors.length > 0 ? 'error' : 'check_circle' }}</span>
                        <span class="seed-section-name">{{ section.name }}</span>
                        <span class="seed-counts">{{ section.created }} created · {{ section.skipped }} skipped</span>
                      </div>
                      @if (section.errors.length > 0) {
                        <ul class="seed-errors">
                          @for (err of section.errors; track err) {
                            <li>{{ err }}</li>
                          }
                        </ul>
                      }
                    </div>
                  }
                </div>
              }

              <!-- Hospital template channel -->
              <div class="seed-advanced">
                <div class="seed-advanced-header">Hospital templates</div>
                <p class="seed-hint">
                  <span class="material-icons">info_outline</span>
                  Finalise uploads the local templates to this hospital's cloud folder. Master jrxml stays generic — per-hospital values come from config.
                </p>
                <div class="tpl-actions">
                  <button class="btn btn-stroked" (click)="finaliseTemplates()" [disabled]="!!tplBusy">
                    @if (tplBusy === 'finalise') { <span class="spinner" style="width:14px;height:14px;border-width:2px"></span> }
                    <span class="material-icons">cloud_upload</span> Finalise Templates
                  </button>
                  <button class="btn btn-stroked" (click)="checkTemplateUpdates()" [disabled]="!!tplBusy">
                    @if (tplBusy === 'check') { <span class="spinner" style="width:14px;height:14px;border-width:2px"></span> }
                    <span class="material-icons">sync</span> Check for Updates
                  </button>
                </div>

                @if (tplChecked && templateUpdates.length === 0) {
                  <div class="tpl-uptodate">
                    <span class="material-icons">check_circle</span> Templates are up to date
                  </div>
                }

                @if (templateUpdates.length > 0) {
                  <div class="tpl-updates">
                    <div class="tpl-updates-head">
                      <span class="material-icons">system_update_alt</span>
                      <span>{{ templateUpdates.length }} update(s) available</span>
                    </div>
                    @for (u of templateUpdates; track u.path) {
                      <div class="tpl-update-row">
                        <span class="tpl-badge" [class.new]="u.status === 'new'">{{ u.status }}</span>
                        <span class="tpl-path">{{ u.path }}</span>
                      </div>
                    }
                    <button class="btn btn-primary tpl-apply" (click)="applyTemplateUpdates()" [disabled]="!!tplBusy">
                      @if (tplBusy === 'apply') { <span class="spinner" style="width:14px;height:14px;border-width:2px"></span> }
                      <span class="material-icons">download</span> Apply (overwrite local)
                    </button>
                  </div>
                }
              </div>
            </div>
          </div>

          <!-- Daemon Log -->
          <div class="card card-pad settings-card daemon-log-card">
            <div class="card-head">
              <span class="material-icons card-avatar">article</span>
              <div class="card-title-group">
                <h2 class="card-title">Daemon Log</h2>
                <div class="card-subtitle">Last 100 lines of daemon.log</div>
              </div>
            </div>
            <div class="card-body">
              <div class="daemon-log-actions">
                <button class="btn btn-stroked" (click)="loadDaemonLog()" [disabled]="daemonLogLoading">
                  @if (daemonLogLoading) {
                    <span class="spinner" style="width:14px;height:14px;border-width:2px"></span>
                  } @else {
                    <span class="material-icons">refresh</span>
                  }
                  {{ daemonLogContent ? 'Refresh' : 'Load Log' }}
                </button>
              </div>
              @if (daemonLogContent) {
                <pre class="daemon-log-output" #daemonLogPre>{{ daemonLogContent }}</pre>
              }
            </div>
          </div>
        </div>

        <div class="actions">
          <button class="btn btn-primary" (click)="saveConfig()">
            <span class="material-icons">save</span>
            Save Changes
          </button>
          <button class="btn btn-stroked" (click)="loadConfig()">
            <span class="material-icons">refresh</span>
            Reset
          </button>
        </div>

        <!-- Danger Zone -->
        <div class="card card-pad settings-card danger-zone-card">
          <div class="card-head">
            <span class="material-icons card-avatar" style="color:#c62828">warning</span>
            <div class="card-title-group">
              <h2 class="card-title">Danger Zone</h2>
              <div class="card-subtitle">Actions that cannot be easily undone</div>
            </div>
          </div>
          <div class="card-body">
            <div class="danger-item">
              <div class="danger-info">
                <strong>Reset Activation</strong>
                <span>Removes license, clears hospital code, and logs deactivation to Firestore. You will need to re-activate with an email.</span>
              </div>
              @if (!confirmingReset) {
                <button class="btn btn-danger" (click)="confirmingReset = true">
                  <span class="material-icons">restart_alt</span> Reset
                </button>
              } @else {
                <div class="confirm-actions">
                  <button class="btn btn-danger" (click)="resetActivation()" [disabled]="resetting">
                    @if (resetting) { <span class="spinner" style="width:14px;height:14px;border-width:2px"></span> }
                    Confirm Reset
                  </button>
                  <button class="btn btn-stroked" (click)="confirmingReset = false">Cancel</button>
                </div>
              }
            </div>
          </div>
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

    .card-head {
      display: flex;
      align-items: center;
      gap: 12px;
      margin-bottom: 1rem;
    }

    .card-avatar {
      background: #e8eaf6;
      padding: 8px;
      border-radius: 50%;
      color: #3f51b5;
      font-size: 24px;
      width: 24px;
      height: 24px;
    }

    .card-title {
      margin: 0;
      font-size: 1.1rem;
      font-weight: 600;
      color: #333;
    }

    .card-title-group {
      display: flex;
      flex-direction: column;
      gap: 2px;
    }

    .card-subtitle {
      font-size: 0.8rem;
      color: var(--text-secondary);
    }

    .divider {
      height: 1px;
      background: var(--border-card);
      margin: 1rem 0;
    }

    .check {
      display: inline-flex;
      align-items: center;
      gap: 8px;
      cursor: pointer;
      font-size: 0.9rem;
    }

    .check input[type="checkbox"] {
      width: 16px;
      height: 16px;
      accent-color: var(--accent-indigo);
      cursor: pointer;
    }

    .field-hint {
      font-size: 0.72rem;
      color: var(--text-muted);
    }

    .field-error {
      font-size: 0.72rem;
      color: var(--status-red);
    }

    .input-with-suffix {
      display: flex;
      align-items: center;
      gap: 4px;
    }

    .input-with-suffix .input {
      flex: 1;
    }

    .mode-display {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 8px 0 16px;

      .mode-label {
        font-size: 0.85rem;
        color: #666;
      }

      .mode-value {
        font-size: 0.85rem;
        font-weight: 600;
        padding: 2px 10px;
        border-radius: 12px;
        background: #e3f2fd;
        color: #1565c0;

        &.native {
          background: #fff3e0;
          color: #e65100;
        }
      }
    }

    .full-width {
      width: 100%;
      margin-bottom: 1rem;
    }

    .toggle-row {
      padding: 1rem 0;

      .check {
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

        .material-icons {
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

        .field {
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

      .material-icons {
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

      .material-icons {
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

    /* ── TLS Card ───────────────────────── */
    .tls-status-row {
      display: flex;
      align-items: center;
      gap: 8px;
      margin-bottom: 1rem;
      font-weight: 500;

      .status-ok { color: #4caf50; }
      .status-warn { color: #ff9800; }
    }

    .tls-details {
      display: flex;
      flex-direction: column;
      gap: 2px;
      font-size: 0.8rem;
      color: #666;
      font-family: monospace;
      margin-bottom: 1rem;
    }

    .tls-actions-row {
      display: flex;
      gap: 0.75rem;
      flex-wrap: wrap;
    }

    .tls-hint {
      font-size: 0.85rem;
      color: #666;
      margin: 0 0 1rem;

      code {
        background: #f5f5f5;
        padding: 1px 6px;
        border-radius: 4px;
        font-size: 0.85em;
      }
    }

    .actions {
      display: flex;
      gap: 0.75rem;
      justify-content: flex-end;
      padding-top: 1rem;
      border-top: 1px solid #eee;
    }

    .service-status-panel {
      display: flex;
      flex-direction: column;
      gap: 0.75rem;
      margin-bottom: 1rem;
    }

    .service-row {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 0.75rem;
      padding: 0.5rem 0;
    }

    .service-label {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      flex-wrap: wrap;
    }

    .status-ok { color: #4caf50; }
    .status-warn { color: #ff9800; }
    .status-muted { color: #999; }

    .daemon-pid {
      font-size: 0.75rem;
      color: #999;
      font-family: monospace;
    }

    .daemon-url {
      font-size: 0.75rem;
      color: #666;
      font-family: monospace;
    }

    .daemon-actions {
      display: flex;
      gap: 0.5rem;
      align-items: center;
    }

    .service-detail {
      font-size: 0.75rem;
      color: #999;
      font-family: monospace;
      padding: 0.25rem 0;
    }

    .daemon-error {
      display: flex;
      align-items: flex-start;
      gap: 0.5rem;
      padding: 0.75rem;
      background: #ffebee;
      border-radius: 8px;
      color: #c62828;
      font-size: 0.85rem;
      margin-bottom: 1rem;

      .material-icons:first-child { font-size: 18px; width: 18px; height: 18px; margin-top: 2px; }
      span { flex: 1; word-break: break-word; }
      button { margin-left: auto; flex-shrink: 0; }
    }

    .daemon-log-card {
      grid-column: 1 / -1;
    }

    .daemon-log-actions {
      margin-bottom: 0.75rem;
    }

    .daemon-log-output {
      background: #1e1e2e;
      color: #cdd6f4;
      font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', monospace;
      font-size: 0.75rem;
      line-height: 1.5;
      padding: 1rem;
      border-radius: 8px;
      max-height: 400px;
      overflow: auto;
      white-space: pre-wrap;
      word-break: break-all;
      margin: 0;
    }

    /* ── Data Seeding ───────────────────── */
    .seed-hint {
      display: flex;
      align-items: flex-start;
      gap: 6px;
      font-size: 0.85rem;
      color: #666;
      margin: 0 0 1rem;

      .material-icons { font-size: 18px; width: 18px; height: 18px; margin-top: 1px; color: #1565c0; }
    }

    .seed-advanced {
      margin-top: 1rem;
      padding-top: 1rem;
      border-top: 1px solid #eee;
    }

    .seed-advanced-header {
      font-size: 0.75rem;
      text-transform: uppercase;
      letter-spacing: 0.5px;
      color: #999;
      margin-bottom: 0.75rem;
    }

    .toggle-row.compact {
      display: flex;
      flex-direction: column;
      gap: 0.5rem;
      padding: 0 0 1rem;
    }

    .seed-results {
      margin-top: 1rem;
      display: flex;
      flex-direction: column;
      gap: 0.5rem;
    }

    .seed-section {
      padding: 0.6rem 0.75rem;
      background: #e8f5e9;
      border-radius: 8px;
      font-size: 0.85rem;

      &.has-errors { background: #ffebee; }
    }

    .seed-section-head {
      display: flex;
      align-items: center;
      gap: 0.5rem;

      .material-icons { font-size: 18px; width: 18px; height: 18px; color: #4caf50; }
      .seed-section-name { font-weight: 500; }
      .seed-counts { margin-left: auto; color: #666; font-size: 0.78rem; }
    }

    .seed-section.has-errors .seed-section-head .material-icons { color: #f44336; }

    .seed-errors {
      margin: 0.5rem 0 0;
      padding-left: 1.5rem;
      color: #c62828;
      font-size: 0.78rem;

      li { margin-bottom: 2px; word-break: break-word; }
    }

    .tpl-actions {
      display: flex;
      gap: 0.75rem;
      flex-wrap: wrap;
      margin-bottom: 0.5rem;
    }

    .tpl-uptodate {
      display: flex;
      align-items: center;
      gap: 6px;
      font-size: 0.85rem;
      color: #4caf50;
      margin-top: 0.75rem;

      .material-icons { font-size: 18px; width: 18px; height: 18px; }
    }

    .tpl-updates {
      margin-top: 1rem;
      padding: 0.75rem;
      background: #fff8e1;
      border-radius: 8px;
    }

    .tpl-updates-head {
      display: flex;
      align-items: center;
      gap: 6px;
      font-weight: 500;
      font-size: 0.85rem;
      color: #e65100;
      margin-bottom: 0.5rem;

      .material-icons { font-size: 18px; width: 18px; height: 18px; }
    }

    .tpl-update-row {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      font-size: 0.8rem;
      padding: 2px 0;
    }

    .tpl-badge {
      text-transform: uppercase;
      font-size: 0.65rem;
      font-weight: 600;
      letter-spacing: 0.5px;
      padding: 1px 6px;
      border-radius: 10px;
      background: #ffe0b2;
      color: #e65100;

      &.new { background: #c8e6c9; color: #2e7d32; }
    }

    .tpl-path {
      font-family: 'SF Mono', 'Fira Code', monospace;
      word-break: break-all;
    }

    .tpl-apply {
      margin-top: 0.75rem;
    }

    .danger-zone-card {
      margin-top: 2rem;
      border: 1px solid #ffcdd2;
    }

    .danger-item {
      display: flex;
      align-items: center;
      gap: 1rem;
    }

    .danger-info {
      flex: 1;

      strong { display: block; margin-bottom: 2px; }
      span { font-size: 0.85rem; color: #666; }
    }

    .confirm-actions {
      display: flex;
      gap: 0.5rem;
    }
  `]
})
export class SettingsComponent implements OnInit {
  private tauri = inject(TauriService);
  private notification = inject(NotificationService);
  private conn = inject(ConnectionService);

  /** Daemon lifecycle and local file pickers have no remote equivalent. */
  get isRemote(): boolean {
    return this.conn.isRemote();
  }

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
  daemonBackupTime = '02:00';
  daemonTelemetryMinutes = 15;

  syncStatus: { lastSynced: Date | null; pendingChanges: number } | null = null;
  pulling = false;
  pullResult: PullSettingsResult | null = null;

  // LAN validation
  lanValidating = false;
  lanValidationResult = false;
  lanValidationError = '';

  // TLS
  tlsStatus: { configured: boolean; ca_download_url?: string; cert_path?: string; key_path?: string } | null = null;
  tlsSettingUp = false;

  // Daemon management
  daemonAction: string | null = null; // 'installing' | 'starting' | 'stopping' | 'restarting' | null
  daemonError: string | null = null;

  // Daemon log
  daemonLogContent = '';
  daemonLogLoading = false;
  @ViewChild('daemonLogPre') daemonLogPre?: ElementRef<HTMLPreElement>;

  // Danger zone
  confirmingReset = false;
  resetting = false;

  // Data seeding
  seeding: 'all' | 'selected' | null = null;
  seedDb = true;
  seedQueues = true;
  seedTemplates = true;
  seedReport: SeedReport | null = null;

  // Hospital template channel
  tplBusy: 'finalise' | 'check' | 'apply' | null = null;
  tplChecked = false;
  templateUpdates: TemplateUpdate[] = [];

  ngOnInit(): void {
    this.loadConfig();
    this.checkDaemonStatus();
    this.loadSyncStatus();
    this.checkCredentials();
    this.loadTlsStatus();
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
        this.daemonBackupTime = this.config.daemon.backup_schedule.backup_time || '';
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
        backup_type: this.daemonBackupType,
        backup_time: this.daemonBackupTime || null
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
      const status = await this.tauri.invokeSilent<{ last_synced: string | null; pending_count: number }>('get_sync_status');
      this.syncStatus = {
        lastSynced: status.last_synced ? new Date(status.last_synced) : null,
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

  async installDaemon(): Promise<void> {
    this.daemonAction = 'installing';
    this.daemonError = null;
    try {
      const msg = await this.tauri.invoke<string>('install_daemon_service');
      this.notification.success(msg);
      await this.checkDaemonStatus();
    } catch (error) {
      this.daemonError = String(error);
    } finally {
      this.daemonAction = null;
    }
  }

  async uninstallDaemon(): Promise<void> {
    if (!confirm('Uninstall the daemon service? Backups, telemetry, and watchdog will stop.')) return;
    this.daemonAction = 'uninstalling';
    this.daemonError = null;
    try {
      const msg = await this.tauri.invoke<string>('uninstall_daemon_service');
      this.notification.success(msg);
      await this.checkDaemonStatus();
    } catch (error) {
      this.daemonError = String(error);
    } finally {
      this.daemonAction = null;
    }
  }

  async startDaemon(): Promise<void> {
    this.daemonAction = 'starting';
    this.daemonError = null;
    try {
      const msg = await this.tauri.invoke<string>('start_daemon');
      this.notification.success(msg);
      await this.checkDaemonStatus();
    } catch (error) {
      this.daemonError = String(error);
    } finally {
      this.daemonAction = null;
    }
  }

  async stopDaemon(): Promise<void> {
    this.daemonAction = 'stopping';
    this.daemonError = null;
    try {
      const msg = await this.tauri.invoke<string>('stop_daemon');
      this.notification.success(msg);
      await this.checkDaemonStatus();
    } catch (error) {
      this.daemonError = String(error);
    } finally {
      this.daemonAction = null;
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
      this.lanValidationError = error instanceof Error ? error.message : String(error);
    } finally {
      this.lanValidating = false;
    }
  }

  async restartDaemon(): Promise<void> {
    this.daemonAction = 'restarting';
    this.daemonError = null;
    try {
      const msg = await this.tauri.invoke<string>('restart_daemon');
      this.notification.success(msg);
      await this.checkDaemonStatus();
    } catch (error) {
      this.daemonError = String(error);
    } finally {
      this.daemonAction = null;
    }
  }

  // ── Daemon Log ──────────────────────────────────────────────────────────

  async loadDaemonLog(): Promise<void> {
    this.daemonLogLoading = true;
    try {
      this.daemonLogContent = await this.tauri.invokeSilent<string>('read_daemon_log', { lines: 100 });
      // Auto-scroll to bottom after render
      setTimeout(() => {
        if (this.daemonLogPre?.nativeElement) {
          this.daemonLogPre.nativeElement.scrollTop = this.daemonLogPre.nativeElement.scrollHeight;
        }
      }, 50);
    } catch (error) {
      this.daemonLogContent = 'Failed to load daemon log: ' + String(error);
    } finally {
      this.daemonLogLoading = false;
    }
  }

  // ── TLS ──────────────────────────────────────────────────────────────────

  async loadTlsStatus(): Promise<void> {
    try {
      this.tlsStatus = await this.tauri.invokeSilent<{ configured: boolean; ca_download_url?: string; cert_path?: string; key_path?: string }>('get_tls_status');
    } catch {
      this.tlsStatus = { configured: false };
    }
  }

  async runTlsSetup(): Promise<void> {
    this.tlsSettingUp = true;
    try {
      await this.tauri.invoke('setup_tls');
      this.notification.success('HTTPS configured. Restart Nginx to apply.');
      await this.loadTlsStatus();
    } catch {
      // Error handled by TauriService
    } finally {
      this.tlsSettingUp = false;
    }
  }

  async downloadTlsScript(): Promise<void> {
    try {
      const script = await this.tauri.invoke<string>('generate_client_setup_script');
      const blob = new Blob([script], { type: 'application/bat' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = 'puru-secure-setup.bat';
      a.click();
      URL.revokeObjectURL(url);
      this.notification.success('Client setup script downloaded');
    } catch {
      // Error handled by TauriService
    }
  }

  async copyTlsNginxConfig(): Promise<void> {
    try {
      const config = await this.tauri.invoke<string>('generate_nginx_https_config');
      await navigator.clipboard.writeText(config);
      this.notification.success('Nginx HTTPS config copied to clipboard');
    } catch {
      // Error handled by TauriService
    }
  }

  // ── Data Seeding ───────────────────────────────────────────────────────────

  async seedAll(): Promise<void> {
    await this.runSeed('all', true, true, true);
  }

  async seedSelected(): Promise<void> {
    if (!this.seedDb && !this.seedQueues && !this.seedTemplates) return;
    await this.runSeed('selected', this.seedDb, this.seedQueues, this.seedTemplates);
  }

  private async runSeed(mode: 'all' | 'selected', db: boolean, queues: boolean, templates: boolean): Promise<void> {
    this.seeding = mode;
    this.seedReport = null;
    try {
      this.seedReport = await this.tauri.invoke<SeedReport>('seed_data', { db, queues, templates });
      const totals = this.seedReport.sections.reduce(
        (acc, s) => ({ created: acc.created + s.created, errors: acc.errors + s.errors.length }),
        { created: 0, errors: 0 }
      );
      if (totals.errors > 0) {
        this.notification.warning(`Seeding finished with ${totals.errors} error(s) — see details below`);
      } else {
        this.notification.success(`Seeding complete — ${totals.created} value(s) created`);
      }
    } catch (error) {
      this.notification.error('Seeding failed: ' + String(error));
    } finally {
      this.seeding = null;
    }
  }

  // ── Hospital template channel ──────────────────────────────────────────────

  async finaliseTemplates(): Promise<void> {
    if (!confirm('Upload the local templates to this hospital\'s cloud folder? This publishes them as the hospital\'s source of truth.')) return;
    this.tplBusy = 'finalise';
    try {
      const report = await this.tauri.invoke<FinaliseReport>('finalise_templates');
      if (report.errors.length > 0) {
        this.notification.warning(`Finalised ${report.uploaded} file(s) with ${report.errors.length} error(s)`);
      } else {
        this.notification.success(`Finalised ${report.uploaded} template(s) to the cloud`);
      }
      // After finalising, local matches the folder — clear any stale update list.
      this.templateUpdates = [];
      this.tplChecked = false;
    } catch (error) {
      this.notification.error('Finalise failed: ' + String(error));
    } finally {
      this.tplBusy = null;
    }
  }

  async checkTemplateUpdates(): Promise<void> {
    this.tplBusy = 'check';
    try {
      this.templateUpdates = await this.tauri.invoke<TemplateUpdate[]>('check_template_updates');
      this.tplChecked = true;
    } catch (error) {
      this.notification.error('Update check failed: ' + String(error));
    } finally {
      this.tplBusy = null;
    }
  }

  async applyTemplateUpdates(): Promise<void> {
    if (!confirm(`Overwrite ${this.templateUpdates.length} local template file(s) with the cloud version? Local edits to these files will be lost.`)) return;
    this.tplBusy = 'apply';
    try {
      const report = await this.tauri.invoke<ApplyReport>('apply_template_updates');
      if (report.errors.length > 0) {
        this.notification.warning(`Applied ${report.applied} file(s) with ${report.errors.length} error(s)`);
      } else {
        this.notification.success(`Applied ${report.applied} template update(s)`);
      }
      this.templateUpdates = [];
      this.tplChecked = false;
    } catch (error) {
      this.notification.error('Apply failed: ' + String(error));
    } finally {
      this.tplBusy = null;
    }
  }

  async resetActivation(): Promise<void> {
    this.resetting = true;
    try {
      await this.tauri.invoke('reset_activation');
      resetLicenseCache();
      this.notification.success('Activation reset. Redirecting to activation...');
      setTimeout(() => window.location.reload(), 1500);
    } catch (error) {
      this.notification.error('Reset failed: ' + String(error));
    } finally {
      this.resetting = false;
      this.confirmingReset = false;
    }
  }
}
