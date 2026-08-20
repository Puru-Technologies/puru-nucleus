import { Component, OnInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import { TauriService, NucleusConfig, PrerequisiteStatus, DetectionResult, InstallProgress, InstallResult, ManualDownloadProgress, ManualDownloadResult } from '../../core/services/tauri.service';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { NotificationService } from '../../core/services/notification.service';
import { ConnectionService } from '../../core/services/connection.service';
import { open } from '@tauri-apps/plugin-dialog';
import { PuruProgressComponent } from '../../core/components/puru-progress.component';

interface SetupStep {
  label: string;
  status: 'pending' | 'in_progress' | 'completed' | 'error';
  message?: string;
  /** Live download progress (0–100) for the JAR-pull step; undefined otherwise. */
  percent?: number;
  progressState?: 'active' | 'complete' | 'hold' | 'fail' | 'indeterminate';
}

@Component({
  selector: 'app-setup',
  standalone: true,
  imports: [
    CommonModule,
    FormsModule,
    PuruProgressComponent
  ],
  template: `
    <div class="setup-page">
      <div class="card card-pad setup-card">
        <div class="card-head">
          <div class="card-title">PURU NUCLEUS - Setup</div>
          <div class="card-subtitle">{{ config?.hospital_code || 'Loading...' }}</div>
        </div>

        <div class="card-body">
          <!-- Phase 1: Infrastructure Configuration -->
          @if (!configSaved) {
            <div class="setup-section">
              <h3>
                <span class="material-icons section-icon">settings</span>
                Step 1: Configure Infrastructure
              </h3>
              <p class="section-desc">
                Set the connection details for your hospital's MySQL database and Docker environment.
                These are required before the automated setup can run.
              </p>

              @if (config) {
                <div class="config-form">
                  <div class="form-row">
                    <div class="field form-field">
                      <label>MySQL Host</label>
                      <input class="input" [(ngModel)]="config.mysql_host" placeholder="127.0.0.1">
                    </div>
                    <div class="field form-field-sm">
                      <label>Port</label>
                      <input class="input" type="number" [(ngModel)]="config.mysql_port" placeholder="3306">
                    </div>
                  </div>

                  <div class="form-row">
                    <div class="field form-field">
                      <label>MySQL User</label>
                      <input class="input" [(ngModel)]="config.mysql_user" placeholder="root">
                    </div>
                    <div class="field form-field">
                      <label>MySQL Password</label>
                      <input class="input" type="password" [(ngModel)]="config.mysql_password">
                      <span class="field-hint">Root password for the MySQL server</span>
                    </div>
                  </div>

                  <div class="mode-selector">
                    <span class="mode-label">Deployment Mode</span>
                    <div class="seg">
                      <button type="button" class="seg-btn" [class.active]="config.deployment_mode==='docker'" (click)="config.deployment_mode='docker'">
                        <span class="material-icons">cloud</span> Docker
                      </button>
                      <button type="button" class="seg-btn" [class.active]="config.deployment_mode==='native'" (click)="config.deployment_mode='native'">
                        <span class="material-icons">computer</span> Native (JAR)
                      </button>
                    </div>
                    <span class="mode-hint">
                      @if (config.deployment_mode === 'native') {
                        Everything runs natively — no Docker. MySQL, RabbitMQ, Nginx &amp; Java must be pre-installed on the host.
                      } @else {
                        All services run as Docker containers.
                      }
                    </span>
                  </div>

                  @if (config.deployment_mode !== 'native') {
                  <div class="browse-row">
                      <div class="field flex-1">
                        <label>Docker Compose Path</label>
                        <input class="input" [(ngModel)]="config.docker_compose_path" placeholder="/home/puru/docker/docker-compose.yml">
                        <span class="field-hint">Path to docker-compose.yml</span>
                      </div>
                      @if (!isRemote) {
                        <button class="btn btn-stroked" type="button" (click)="browseComposePath()">
                          <span class="material-icons">folder_open</span> Browse
                        </button>
                      }
                    </div>
                  }

                  <div class="browse-row">
                    <div class="field flex-1">
                      <label>Puru Data Directory</label>
                      <input class="input" [(ngModel)]="puruDataPath" placeholder="/home/puru/puru-data">
                      <span class="field-hint">Root directory for hospital data, documents, and uploads</span>
                    </div>
                    @if (!isRemote) {
                      <button class="btn btn-stroked" type="button" (click)="browsePuruDataPath()">
                        <span class="material-icons">folder_open</span> Browse
                      </button>
                    }
                  </div>

                  <div class="field full-width">
                    <label>Server IP (local network)</label>
                    <input class="input" [(ngModel)]="config.server_ip" placeholder="192.168.1.100">
                    <span class="field-hint">This machine's IP on the hospital LAN</span>
                  </div>

                  <div class="creds-row">
                    <div class="creds-label">
                      <span class="label-text">GCS Credentials</span>
                      <span class="label-hint">For cloud backups and config sync</span>
                    </div>
                    @if (credsExists) {
                      <span class="creds-ok"><span class="material-icons">check_circle</span> Configured</span>
                    } @else {
                      <span class="creds-missing">Not set</span>
                    }
                    @if (!isRemote) {
                      <button class="btn btn-stroked" type="button" (click)="browseCredentials()">
                        <span class="material-icons">folder_open</span>
                        {{ credsExists ? 'Replace' : 'Browse' }}
                      </button>
                    } @else {
                      <span class="creds-missing">Using credentials configured on the server</span>
                    }
                  </div>

                  @if (configError) {
                    <div class="config-error">
                      <span class="material-icons">error</span>
                      <span>{{ configError }}</span>
                    </div>
                  }
                </div>
              } @else {
                <div class="loading-inline">
                  <span class="spinner" style="width:24px;height:24px"></span>
                  <span>Loading configuration...</span>
                </div>
              }
            </div>
          }

          <!-- Phase 2: Prerequisites + Setup (shown after config is saved) -->
          @if (configSaved) {
            <div class="config-saved-banner">
              <span class="material-icons">check_circle</span>
              <div>
                <strong>Configuration saved</strong>
                <span>MySQL {{ config!.mysql_user }}{{'@'}}{{ config!.mysql_host }}:{{ config!.mysql_port }}</span>
              </div>
              <button class="btn btn-stroked" (click)="editConfig()">Edit</button>
            </div>

            <!-- Mode Badge -->
            <div class="mode-badge" [class.native]="config!.deployment_mode === 'native'">
              <span class="material-icons">{{ config!.deployment_mode === 'native' ? 'computer' : 'cloud' }}</span>
              <span>{{ config!.deployment_mode === 'native' ? 'Native (JAR) Mode' : 'Docker Mode' }}</span>
            </div>

            <!-- Detection Result (Docker only) -->
            @if (detectionResult?.found && config!.deployment_mode !== 'native') {
              <div class="detection-banner">
                <span class="material-icons">info</span>
                <div class="detection-info">
                  <strong>Existing Puru Setup Detected</strong>
                  <p>Found {{ detectionResult!.containers.length }} containers</p>
                </div>
                <div class="detection-actions">
                  <button class="btn btn-stroked" (click)="adoptExisting()" [disabled]="adopting">
                    @if (adopting) {
                      <span class="spinner" style="width:18px;height:18px"></span>
                    }
                    Adopt Existing
                  </button>
                  <button class="btn btn-stroked" (click)="freshInstall()">
                    Fresh Install
                  </button>
                </div>
              </div>
            }

            <!-- Mismatch Warning -->
            @if (mismatchError) {
              <div class="mismatch-error">
                <span class="material-icons">error</span>
                <div>
                  <strong>Hospital Code Mismatch</strong>
                  <p>{{ mismatchError }}</p>
                  <p class="mismatch-hint">Fix the hospital code in Firestore or re-activate with the correct email, then try again.</p>
                </div>
              </div>
            }

            @if (configMismatches.length > 0) {
              <div class="mismatch-panel">
                <div class="mismatch-header">
                  <span class="material-icons">warning</span>
                  <strong>Config values differ from detected environment</strong>
                </div>
                <div class="mismatch-list">
                  @for (m of configMismatches; track m.field) {
                    <div class="mismatch-item">
                      <span class="mismatch-field">{{ m.field }}</span>
                      <div class="mismatch-choices">
                        <button class="btn btn-stroked choice-btn" [class.selected]="m.choice === 'config'"
                                (click)="m.choice = 'config'">
                          Keep: {{ m.config_value }}
                        </button>
                        <button class="btn btn-stroked choice-btn" [class.selected]="m.choice === 'detected'"
                                (click)="m.choice = 'detected'">
                          Use detected: {{ m.detected_value }}
                        </button>
                      </div>
                    </div>
                  }
                </div>
                <div class="mismatch-actions">
                  <button class="btn btn-primary" (click)="applyMismatchChoices()">
                    Apply & Continue
                  </button>
                </div>
              </div>
            }

            <!-- Prerequisites Section -->
            <div class="setup-section">
              <h3>Prerequisites</h3>
              <div class="prereq-list">
                @for (prereq of prerequisites; track prereq.name) {
                  <div class="prereq-row">
                    <span class="material-icons" [class]="prereq.installed ? 'status-success' : 'status-error'">
                      {{ prereq.installed ? 'check_circle' : 'cancel' }}
                    </span>
                    <div class="prereq-text">
                      <span class="prereq-name">{{ prereq.name }}</span>
                      <span class="prereq-line">
                        @if (prereq.installed) {
                          Version {{ prereq.version }}
                        } @else if (prereq.required_version) {
                          Not installed (requires {{ prereq.required_version }}+)
                        } @else {
                          Not installed
                        }
                      </span>
                    </div>
                  </div>
                }
              </div>

              <!-- Re-check / Install Missing Buttons -->
              <div class="prereq-actions">
                @if (!installing && !manualDownloading) {
                  <button class="btn btn-stroked" (click)="recheckPrerequisites()" [disabled]="recheckingPrereqs">
                    <span class="material-icons">{{ recheckingPrereqs ? 'hourglass_empty' : 'refresh' }}</span>
                    {{ recheckingPrereqs ? 'Checking...' : 'Re-check' }}
                  </button>
                }
                @if (installablePrereqs.length > 0 && !installing && !manualDownloading) {
                  <button class="btn btn-primary" (click)="installMissing()">
                    <span class="material-icons">download</span>
                    Install {{ installableNames }}
                  </button>
                  <button class="btn btn-stroked" (click)="downloadInfraManually()"
                          title="Save the installers to this machine's Downloads folder — run them yourself instead of the automated flow.">
                    <span class="material-icons">save_alt</span>
                    Install Manually
                  </button>
                  <span class="install-hint">Automated install, or download to Downloads folder.</span>
                }
              </div>

              <!-- Manual download progress -->
              @if (manualDownloading && manualDownloadProgress) {
                <div class="install-progress-card">
                  <div class="install-progress-header">
                    <span class="material-icons">{{ manualDownloadProgress.stage === 'completed' ? 'check_circle' : manualDownloadProgress.stage === 'failed' ? 'error' : 'downloading' }}</span>
                    <span>
                      <strong>{{ manualDownloadProgress.software }}:</strong>
                      {{ manualDownloadProgress.file }}
                      @if (manualDownloadProgress.bytes_total) {
                        — {{ manualDownloadProgress.percent }}%
                      }
                    </span>
                  </div>
                  <div class="pbar"
                    [class.indeterminate]="manualDownloadProgress.stage === 'start' || !manualDownloadProgress.bytes_total">
                    <div class="pbar-fill" [style.width.%]="manualDownloadProgress.percent || 0"></div>
                  </div>
                </div>
              }

              <!-- Manual download results -->
              @if (manualDownloadResults.length > 0 && !manualDownloading) {
                <div class="install-results">
                  @for (r of manualDownloadResults; track r.software) {
                    <div class="install-result" [class.success]="r.success" [class.failure]="!r.success">
                      <span class="material-icons">{{ r.success ? 'check_circle' : 'cancel' }}</span>
                      <span>
                        {{ r.software }}:
                        @if (r.success) {
                          Saved to {{ r.path }} ({{ r.size_mb.toFixed(1) }} MB)
                        } @else {
                          {{ r.error }}
                        }
                      </span>
                    </div>
                  }
                </div>
              }

              <!-- Install Progress -->
              @if (installing && installProgress) {
                <div class="install-progress-card">
                  <div class="install-progress-header">
                    <span class="material-icons">{{ installProgress.stage === 'completed' ? 'check_circle' : installProgress.stage === 'failed' ? 'error' : 'downloading' }}</span>
                    <span><strong>{{ installProgress.software }}:</strong> {{ installProgress.message }}</span>
                  </div>
                  <div class="pbar"
                    [class.indeterminate]="installProgress.stage === 'installing' || installProgress.stage === 'verifying'">
                    <div class="pbar-fill" [style.width.%]="installProgress.percent"></div>
                  </div>
                </div>
              }

              <!-- Install Results -->
              @if (installResults.length > 0 && !installing) {
                <div class="install-results">
                  @for (result of installResults; track result.software) {
                    <div class="install-result" [class.success]="result.success" [class.failure]="!result.success">
                      <span class="material-icons">{{ result.success ? 'check_circle' : 'cancel' }}</span>
                      <span>{{ result.software }}: {{ result.success ? 'Installed (' + (result.version || 'ok') + ')' : result.error }}</span>
                    </div>
                  }
                </div>
              }
            </div>

            <!-- Enabled Services -->
            @if (enabledServices.length > 0) {
              <div class="setup-section">
                <h3>Enabled Services</h3>
                <div class="enabled-services">
                  @for (svc of enabledServices; track svc) {
                    <span class="service-chip">{{ svc }}</span>
                  }
                </div>
                @if (infraNotes.length > 0) {
                  <div class="infra-notes">
                    @for (note of infraNotes; track note) {
                      <span class="infra-note"><span class="material-icons">info_outline</span> {{ note }}</span>
                    }
                  </div>
                }
              </div>
            }

            <!-- Setup Steps -->
            <div class="setup-section">
              <h3>Installation Progress</h3>
              <div class="steps-list">
                @for (step of steps; track step.label; let i = $index) {
                  <div class="step" [class]="'step-' + step.status">
                    <div class="step-indicator">
                      @if (step.status === 'completed') {
                        <span class="material-icons">check</span>
                      } @else if (step.status === 'in_progress') {
                        <span class="spinner" style="width:18px;height:18px"></span>
                      } @else if (step.status === 'error') {
                        <span class="material-icons">error</span>
                      } @else {
                        <span class="step-number">{{ i + 1 }}</span>
                      }
                    </div>
                    <div class="step-content">
                      <div class="step-label">{{ step.label }}</div>
                      @if (step.message) {
                        <div class="step-message">{{ step.message }}</div>
                      }
                      @if (step.percent != null && step.status === 'in_progress') {
                        <div class="step-progress">
                          <puru-progress [value]="step.percent" [state]="step.progressState || 'active'" [height]="6"></puru-progress>
                        </div>
                      }
                    </div>
                    @if (!setupInProgress) {
                      <button class="step-run" (click)="runSingleStep(i)"
                              [disabled]="singleStepIndex !== null"
                              title="Run only this step (re-run individually if it failed)">
                        @if (singleStepIndex === i) {
                          <span class="spinner" style="width:14px;height:14px"></span>
                        } @else {
                          <span class="material-icons">play_arrow</span>
                        }
                        Run
                      </button>
                    }
                  </div>
                }
              </div>
            </div>

            <!-- Per-service activity (native start/sync step) -->
            @if (nativeActions.length > 0) {
              <div class="setup-section">
                <h3>Service Activity</h3>
                <div class="native-actions">
                  @for (a of nativeActions; track a.service) {
                    <div class="native-action" [class]="'na-' + a.action">
                      <span class="material-icons">
                        @switch (a.action) {
                          @case ('started') { check_circle }
                          @case ('already-running') { check_circle }
                          @case ('removed') { delete_sweep }
                          @case ('not-installed') { cloud_off }
                          @default { error }
                        }
                      </span>
                      <span class="na-name">{{ a.service }}</span>
                      <span class="na-msg">{{ a.message }}</span>
                    </div>
                  }
                </div>
              </div>
            }

            <!-- TLS Setup Result -->
            @if (setupComplete && tlsStatus?.configured) {
              <div class="setup-section">
                <h3>
                  <span class="material-icons section-icon" style="color:#4caf50">lock</span>
                  HTTPS Configured
                </h3>
                <div class="tls-info">
                  <p>HTTPS is ready. Client machines need a one-time certificate install.</p>
                  <div class="tls-actions">
                    <button class="btn btn-primary" (click)="downloadClientScript()">
                      <span class="material-icons">download</span>
                      Download Client Setup Script (.bat)
                    </button>
                    <button class="btn btn-stroked" (click)="copyNginxConfig()">
                      <span class="material-icons">content_copy</span>
                      Copy Nginx Config
                    </button>
                  </div>
                  <div class="tls-urls">
                    <span><strong>CA Download:</strong> {{ tlsStatus.ca_download_url }}</span>
                    <span><strong>HTTPS URL:</strong> https://{{ config!.server_ip }}</span>
                  </div>
                </div>
              </div>
            }

            <!-- Progress Bar -->
            @if (setupInProgress) {
              <div class="progress-section">
                <div class="pbar">
                  <div class="pbar-fill" [style.width.%]="progressPercent"></div>
                </div>
                <span class="progress-text">{{ progressPercent }}% Complete</span>
              </div>
            }
          }
        </div>

        <div class="card-actions">
          <button class="btn btn-stroked" (click)="cancel()">Cancel</button>
          @if (configSaved) {
            <button class="btn btn-stroked" (click)="viewLogs()">View Logs</button>
          }

          <!-- Save config button (Phase 1) -->
          @if (!configSaved && config) {
            <button class="btn btn-primary"
                    (click)="saveAndContinue()"
                    [disabled]="!config.mysql_password || configSaving">
              @if (configSaving) {
                <span class="spinner" style="width:18px;height:18px"></span>
              }
              Save & Continue
            </button>
          }

          <!-- Start Setup button (Phase 2) -->
          @if (configSaved && !setupInProgress && !setupComplete) {
            <button class="btn btn-primary" (click)="startSetup()" [disabled]="!allPrerequisitesMet">
              Start Setup
            </button>
          }
          @if (setupComplete) {
            <button class="btn btn-primary" (click)="finish()">
              Finish
            </button>
          }
          @if (configSaved && !setupInProgress) {
            <button class="btn btn-danger" (click)="resetSetup()">
              <span class="material-icons">restart_alt</span>
              Reset Setup
            </button>
          }
        </div>
      </div>
    </div>
  `,
  styles: [`
    .setup-page {
      display: flex;
      justify-content: center;
      align-items: flex-start;
      min-height: 100vh;
      padding: 2rem;
      background: #f5f5f5;
    }

    .setup-card {
      max-width: 700px;
      width: 100%;
    }

    /* ── Card header / body / actions ─────── */
    .card-head {
      margin-bottom: 1rem;
    }
    .card-title {
      font-size: 1.25rem;
      font-weight: 600;
      color: #333;
    }
    .card-subtitle {
      font-size: 0.875rem;
      color: #666;
      margin-top: 2px;
    }

    .material-icons {
      vertical-align: middle;
    }

    /* ── Segmented control ────────────────── */
    .seg {
      display: inline-flex;
      border: 1px solid var(--border-card);
      border-radius: 8px;
      overflow: hidden;
      width: fit-content;
    }
    .seg-btn {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      padding: 8px 16px;
      border: none;
      background: #fff;
      color: #555;
      font: 500 0.875rem 'Inter', sans-serif;
      cursor: pointer;

      & + .seg-btn { border-left: 1px solid var(--border-card); }

      .material-icons { font-size: 18px; width: 18px; height: 18px; }
    }
    .seg-btn.active {
      background: var(--accent-indigo);
      border-color: var(--accent-indigo);
      color: #fff;
    }

    /* ── Progress bar ─────────────────────── */
    .pbar {
      position: relative;
      width: 100%;
      height: 8px;
      border-radius: 999px;
      background: var(--brand-track);
      overflow: visible;
    }
    .pbar-fill {
      position: relative;
      height: 100%;
      background: var(--brand-blue);
      border-radius: 999px;
      transition: width 0.3s ease;
    }
    /* the wordmark's red dot marks the progress head */
    .pbar-fill::after {
      content: '';
      position: absolute;
      right: 0;
      top: 50%;
      width: 14px;
      height: 14px;
      margin: -7px -7px 0 0;
      border-radius: 50%;
      background: var(--brand-red);
      border: 3px solid var(--bg-card);
      box-sizing: border-box;
    }
    .pbar.indeterminate {
      overflow: hidden;
    }
    .pbar.indeterminate .pbar-fill {
      position: absolute;
      width: 40% !important;
      animation: pbar-indeterminate 1.2s infinite ease-in-out;
    }
    .pbar.indeterminate .pbar-fill::after { display: none; }
    @keyframes pbar-indeterminate {
      0%   { margin-left: -40%; }
      100% { margin-left: 100%; }
    }

    /* ── Divider ──────────────────────────── */
    .divider {
      height: 1px;
      background: var(--border-card);
    }

    .field-hint {
      font-size: 0.7rem;
      color: #999;
    }

    /* ── Config Form ──────────────────────── */
    .section-icon {
      font-size: 20px;
      width: 20px;
      height: 20px;
      vertical-align: middle;
      margin-right: 6px;
      color: #3f51b5;
    }

    .section-desc {
      color: #666;
      font-size: 0.875rem;
      margin-bottom: 1.5rem;
    }

    .config-form {
      display: flex;
      flex-direction: column;
      gap: 0.5rem;
    }

    .form-row {
      display: flex;
      gap: 1rem;
    }

    .form-field {
      flex: 1;
    }

    .form-field-sm {
      width: 120px;
      flex-shrink: 0;
    }

    .full-width {
      width: 100%;
    }

    .flex-1 {
      flex: 1;
    }

    .browse-row {
      display: flex;
      align-items: flex-start;
      gap: 0.75rem;

      button { margin-top: 4px; }
    }

    .config-error {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      padding: 0.75rem;
      background: #ffebee;
      border-radius: 8px;
      color: #c62828;
      font-size: 0.875rem;
      margin-top: 0.5rem;

      .material-icons {
        font-size: 1.25rem;
        width: 1.25rem;
        height: 1.25rem;
      }
    }

    .loading-inline {
      display: flex;
      align-items: center;
      gap: 12px;
      padding: 2rem;
      color: #666;
    }

    /* ── Deployment Mode Selector ────────── */
    .mode-selector {
      display: flex;
      flex-direction: column;
      gap: 8px;
      padding: 12px 0;
      margin-bottom: 0.5rem;
    }

    .mode-label {
      font-size: 0.875rem;
      font-weight: 500;
    }

    .mode-hint {
      font-size: 0.75rem;
      color: #666;
    }

    .mode-badge {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 8px 14px;
      background: #e3f2fd;
      border-radius: 8px;
      margin-bottom: 1.5rem;
      font-size: 0.875rem;
      font-weight: 500;
      color: #1565c0;

      .material-icons {
        font-size: 20px;
        width: 20px;
        height: 20px;
      }

      &.native {
        background: #fff3e0;
        color: #e65100;
      }
    }

    /* ── Config Saved Banner ──────────────── */
    .config-saved-banner {
      display: flex;
      align-items: center;
      gap: 12px;
      padding: 12px 16px;
      background: #e8f5e9;
      border-radius: 8px;
      margin-bottom: 1.5rem;

      .material-icons {
        color: #4caf50;
        font-size: 24px;
        width: 24px;
        height: 24px;
      }

      div {
        flex: 1;
        display: flex;
        flex-direction: column;

        strong {
          font-size: 0.875rem;
        }

        span {
          font-size: 0.75rem;
          color: #666;
          font-family: monospace;
        }
      }
    }

    /* ── Detection Banner ─────────────────── */
    .detection-banner {
      display: flex;
      align-items: center;
      gap: 1rem;
      padding: 1rem;
      background: #e3f2fd;
      border-radius: 8px;
      margin-bottom: 1.5rem;

      .material-icons {
        color: #2196f3;
        font-size: 2rem;
        width: 2rem;
        height: 2rem;
      }

      .detection-info {
        flex: 1;

        strong {
          display: block;
          margin-bottom: 0.25rem;
        }

        p {
          margin: 0;
          color: #666;
          font-size: 0.875rem;
        }
      }

      .detection-actions {
        display: flex;
        gap: 0.5rem;
      }
    }

    .setup-section {
      margin-bottom: 1.5rem;

      h3 {
        font-size: 1rem;
        font-weight: 500;
        margin-bottom: 0.75rem;
        color: #333;
      }
    }

    .status-success {
      color: #4caf50;
    }

    .status-error {
      color: #f44336;
    }

    /* ── Prerequisites list ───────────────── */
    .prereq-list {
      display: flex;
      flex-direction: column;
    }

    .prereq-row {
      display: flex;
      align-items: center;
      gap: 12px;
      padding: 10px 4px;
      border-bottom: 1px solid var(--border-card);

      &:last-child { border-bottom: none; }

      .material-icons { font-size: 22px; width: 22px; height: 22px; flex-shrink: 0; }

      .prereq-text {
        display: flex;
        flex-direction: column;
      }

      .prereq-name { font-weight: 500; font-size: 0.9rem; }
      .prereq-line { font-size: 0.8rem; color: #666; }
    }

    /* ── Install Missing ─────────────────── */

    .prereq-actions {
      display: flex;
      align-items: center;
      gap: 12px;
      margin-top: 12px;
      flex-wrap: wrap;

      .install-hint {
        font-size: 12px;
        color: #666;
      }
    }

    .enabled-services {
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
      margin-top: 8px;

      .service-chip {
        padding: 4px 12px;
        background: #e8f5e9;
        color: #2e7d32;
        border-radius: 16px;
        font-size: 13px;
        font-weight: 500;
      }
    }

    .infra-notes {
      margin-top: 10px;
      display: flex;
      flex-direction: column;
      gap: 4px;

      .infra-note {
        display: flex;
        align-items: center;
        gap: 4px;
        font-size: 12px;
        color: #1565c0;

        .material-icons {
          font-size: 16px;
          width: 16px;
          height: 16px;
        }
      }
    }

    .install-progress-card {
      margin-top: 12px;
      padding: 16px;
      background: #e3f2fd;
      border-radius: 8px;
      border-left: 4px solid #2196f3;

      .install-progress-header {
        display: flex;
        align-items: center;
        gap: 8px;
        margin-bottom: 10px;
        font-size: 14px;

        .material-icons {
          color: #1976d2;
          font-size: 20px;
          width: 20px;
          height: 20px;
        }
      }
    }

    .install-results {
      margin-top: 12px;
      display: flex;
      flex-direction: column;
      gap: 6px;
    }

    .install-result {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 8px 12px;
      border-radius: 6px;
      font-size: 13px;

      &.success {
        background: #e8f5e9;
        color: #2e7d32;
        .material-icons { color: #4caf50; }
      }

      &.failure {
        background: #ffebee;
        color: #c62828;
        .material-icons { color: #f44336; }
      }

      .material-icons {
        font-size: 18px;
        width: 18px;
        height: 18px;
      }
    }

    .steps-list {
      display: flex;
      flex-direction: column;
      gap: 0.5rem;
    }

    /* ── Per-service activity ─────────────── */
    .native-actions {
      display: flex;
      flex-direction: column;
      gap: 6px;
    }

    .native-action {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 8px 12px;
      border-radius: 6px;
      font-size: 13px;
      background: #fafafa;

      .material-icons { font-size: 18px; width: 18px; height: 18px; }

      .na-name { font-weight: 600; min-width: 130px; }
      .na-msg { color: #666; }

      &.na-started, &.na-already-running {
        background: #e8f5e9;
        .material-icons { color: #4caf50; }
      }
      &.na-removed {
        background: #ede7f6;
        .material-icons { color: #673ab7; }
      }
      &.na-not-installed {
        background: #fff3e0;
        .material-icons { color: #e65100; }
      }
      &.na-failed {
        background: #ffebee;
        .material-icons { color: #f44336; }
      }
    }

    .step {
      display: flex;
      align-items: flex-start;
      gap: 0.75rem;
      padding: 0.75rem;
      border-radius: 8px;
      background: #fafafa;

      &.step-completed {
        background: #e8f5e9;
        .step-indicator { color: #4caf50; }
      }

      &.step-in_progress {
        background: #e3f2fd;
      }

      &.step-error {
        background: #ffebee;
        .step-indicator { color: #f44336; }
      }
    }

    .step-indicator {
      width: 24px;
      height: 24px;
      display: flex;
      align-items: center;
      justify-content: center;
      flex-shrink: 0;

      .step-number {
        width: 20px;
        height: 20px;
        border-radius: 50%;
        background: #e0e0e0;
        display: flex;
        align-items: center;
        justify-content: center;
        font-size: 0.75rem;
        font-weight: 500;
      }
    }

    .step-content {
      flex: 1;
    }

    .step-label {
      font-weight: 500;
    }

    .step-message {
      font-size: 0.875rem;
      color: #666;
      margin-top: 0.25rem;
      white-space: pre-line;   /* preserve line breaks in multi-line error detail */
      word-break: break-word;
    }
    /* Failed steps: make the reason stand out and stay readable. */
    .step-error .step-message {
      color: var(--brand-red, #e83a3a);
    }
    .step-progress {
      margin-top: 0.5rem;
      max-width: 420px;
    }
    /* Per-step "Run" button (targeted re-run) — sits at the right of each step. */
    .step { align-items: center; }
    .step-run {
      margin-left: auto;
      display: inline-flex;
      align-items: center;
      gap: 4px;
      padding: 4px 12px;
      font-size: 0.8rem;
      font-weight: 600;
      color: var(--brand-blue, #009efb);
      background: transparent;
      border: 1px solid var(--brand-blue, #009efb);
      border-radius: 6px;
      cursor: pointer;
      white-space: nowrap;
      transition: background 0.15s ease, color 0.15s ease;
    }
    .step-run .material-icons { font-size: 16px; }
    .step-run:hover:not(:disabled) { background: var(--brand-blue, #009efb); color: #fff; }
    .step-run:disabled { opacity: 0.45; cursor: default; }

    .progress-section {
      margin-top: 1.5rem;
      text-align: center;

      .pbar {
        margin-bottom: 0.5rem;
      }

      .progress-text {
        font-size: 0.875rem;
        color: #666;
      }
    }

    /* ── Credentials Row ─────────────────── */
    .creds-row {
      display: flex;
      align-items: center;
      gap: 12px;
      padding: 12px 0;
      margin-bottom: 0.5rem;

      .creds-label {
        flex: 1;
        display: flex;
        flex-direction: column;

        .label-text {
          font-size: 0.875rem;
          font-weight: 500;
        }
        .label-hint {
          font-size: 0.75rem;
          color: #999;
        }
      }

      .creds-ok {
        display: flex;
        align-items: center;
        gap: 4px;
        color: #4caf50;
        font-size: 0.8rem;
        font-weight: 500;

        .material-icons {
          font-size: 16px;
          width: 16px;
          height: 16px;
        }
      }

      .creds-missing {
        color: #999;
        font-size: 0.8rem;
      }
    }

    /* ── TLS Info ─────────────────────────── */
    .tls-info {
      padding: 1rem;
      background: #e8f5e9;
      border-radius: 8px;

      p {
        margin: 0 0 1rem;
        color: #333;
        font-size: 0.875rem;
      }
    }

    .tls-actions {
      display: flex;
      gap: 0.75rem;
      margin-bottom: 1rem;
      flex-wrap: wrap;
    }

    .tls-urls {
      display: flex;
      flex-direction: column;
      gap: 4px;
      font-size: 0.8rem;
      color: #555;
      font-family: monospace;
    }

    .card-actions {
      display: flex;
      justify-content: flex-end;
      gap: 0.5rem;
      padding-top: 1rem;
      margin-top: 1rem;
      border-top: 1px solid #eee;
    }

    /* ── Mismatch UI ─────────────────────── */
    .mismatch-error {
      display: flex;
      gap: 12px;
      padding: 1rem;
      background: #ffebee;
      border-radius: 8px;
      margin-bottom: 1.5rem;
      color: #c62828;

      .material-icons { font-size: 24px; width: 24px; height: 24px; flex-shrink: 0; margin-top: 2px; }
      strong { display: block; margin-bottom: 4px; }
      p { margin: 0; font-size: 0.875rem; }
      .mismatch-hint { color: #666; margin-top: 4px; }
    }

    .mismatch-panel {
      padding: 1rem;
      background: #fff3e0;
      border-radius: 8px;
      margin-bottom: 1.5rem;
    }

    .mismatch-header {
      display: flex;
      align-items: center;
      gap: 8px;
      margin-bottom: 1rem;
      color: #e65100;

      .material-icons { font-size: 20px; width: 20px; height: 20px; }
    }

    .mismatch-list {
      display: flex;
      flex-direction: column;
      gap: 0.75rem;
    }

    .mismatch-item {
      padding: 0.75rem;
      background: #fff;
      border-radius: 6px;
    }

    .mismatch-field {
      font-weight: 500;
      font-size: 0.8rem;
      color: #666;
      text-transform: uppercase;
      letter-spacing: 0.5px;
      display: block;
      margin-bottom: 6px;
    }

    .mismatch-choices {
      display: flex;
      gap: 0.5rem;
      flex-wrap: wrap;
    }

    .choice-btn {
      font-size: 0.8rem !important;
      font-family: monospace;
    }

    .choice-btn.selected {
      background: #e3f2fd !important;
      border-color: #2196f3 !important;
      color: #1565c0 !important;
    }

    .mismatch-actions {
      margin-top: 1rem;
      display: flex;
      justify-content: flex-end;
    }
  `]
})
export class SetupComponent implements OnInit {
  private tauri = inject(TauriService);
  private notification = inject(NotificationService);
  private conn = inject(ConnectionService);
  private router = inject(Router);

  /** When connected to a remote daemon, local-only steps (file pickers, daemon
   *  install) are hidden/skipped — setup runs on the server, not this machine. */
  get isRemote(): boolean {
    return this.conn.isRemote();
  }

  config: NucleusConfig | null = null;
  configSaved = false;
  configSaving = false;
  configError: string | null = null;
  credsExists = false;
  puruDataPath = '';

  prerequisites: PrerequisiteStatus[] = [];
  installablePrereqs: PrerequisiteStatus[] = [];
  installing = false;
  recheckingPrereqs = false;
  installProgress: InstallProgress | null = null;
  installResults: InstallResult[] = [];
  manualDownloading = false;
  manualDownloadProgress: ManualDownloadProgress | null = null;
  manualDownloadResults: ManualDownloadResult[] = [];
  detectionResult: DetectionResult | null = null;
  setupInProgress = false;
  setupComplete = false;
  /** Index of the step currently running via the per-step "Run" button, or null. */
  singleStepIndex: number | null = null;
  tlsStatus: any = null;
  adopting = false;
  mismatchError: string | null = null;
  configMismatches: Array<{ field: string; config_value: string; detected_value: string; choice: 'config' | 'detected' }> = [];
  enabledServices: string[] = [];
  infraNotes: string[] = [];
  nativeActions: { service: string; action: string; success: boolean; message: string }[] = [];

  steps: SetupStep[] = [];

  private dockerSteps: SetupStep[] = [
    { label: 'Check prerequisites', status: 'pending' },
    { label: 'Create MySQL databases', status: 'pending' },
    { label: 'Configure RabbitMQ', status: 'pending' },
    { label: 'Generate configuration files', status: 'pending' },
    { label: 'Pull Docker images', status: 'pending' },
    { label: 'Start services', status: 'pending' },
    { label: 'Health check', status: 'pending' },
    { label: 'Configure backups', status: 'pending' },
    { label: 'Install daemon service', status: 'pending' },
    { label: 'Configure HTTPS (TLS)', status: 'pending' }
  ];

  private nativeSteps: SetupStep[] = [
    { label: 'Check prerequisites', status: 'pending' },
    { label: 'Create MySQL databases', status: 'pending' },
    { label: 'Configure RabbitMQ', status: 'pending' },
    { label: 'Generate environment files', status: 'pending' },
    { label: 'Pull JARs & JRE from cloud', status: 'pending' },
    { label: 'Seed message queues', status: 'pending' },
    { label: 'Start native services', status: 'pending' },
    { label: 'Health check', status: 'pending' },
    { label: 'Seed database & templates', status: 'pending' },
    { label: 'Initialize roles & users', status: 'pending' },
    { label: 'Configure backups', status: 'pending' },
    { label: 'Install daemon service', status: 'pending' },
    { label: 'Configure HTTPS (TLS)', status: 'pending' }
  ];

  get installableNames(): string {
    return this.installablePrereqs.map(p => p.name).join(' & ');
  }

  get allPrerequisitesMet(): boolean {
    return this.prerequisites.length > 0 && this.prerequisites.every(p => p.installed);
  }

  get progressPercent(): number {
    const completed = this.steps.filter(s => s.status === 'completed').length;
    return Math.round((completed / this.steps.length) * 100);
  }

  ngOnInit(): void {
    this.loadConfig();
  }

  async loadConfig(): Promise<void> {
    try {
      this.config = await this.tauri.invoke<NucleusConfig>('get_config');
      this.puruDataPath = this.config.puru_data_path || '';
      await this.checkCredsStatus();

      // Auto-detect existing setup and populate empty fields
      await this.autoDetectAndPopulate();

      // Ensure deployment_mode has a default
      if (!this.config.deployment_mode) {
        this.config.deployment_mode = 'docker';
      }

      // If MySQL password is already set, skip to phase 2 automatically
      if (this.config.mysql_password) {
        this.configSaved = true;
        this.initSteps();
        await this.loadPrerequisites();
        await this.loadEnabledServices();
      }
    } catch {
      // Will show defaults
    }
  }

  /** Detect compose file and populate config fields from env files */
  private async autoDetectAndPopulate(): Promise<void> {
    if (!this.config) return;

    try {
      // Detect existing Docker setup (finds compose path + containers)
      const detection = await this.tauri.invokeSilent<DetectionResult>('detect_existing_setup');
      if (detection?.compose_path && !this.config.docker_compose_path) {
        this.config.docker_compose_path = detection.compose_path;
      }

      // If we have a compose path, detect environment variables from it
      if (this.config.docker_compose_path) {
        const envResult = await this.tauri.invokeSilent<any>('detect_environment', {
          composePath: this.config.docker_compose_path
        });

        if (envResult) {
          // Populate empty fields from detected values
          if (envResult.hospital_info) {
            if (!this.config.server_ip && envResult.hospital_info.server_ip) {
              this.config.server_ip = envResult.hospital_info.server_ip;
            }
            if (!this.config.hospital_code && envResult.hospital_info.code) {
              this.config.hospital_code = envResult.hospital_info.code;
            }
          }
          if (envResult.database) {
            if ((!this.config.mysql_user || this.config.mysql_user === 'root') && envResult.database.username) {
              this.config.mysql_user = envResult.database.username;
            }
            if (!this.config.mysql_password && envResult.database.password) {
              this.config.mysql_password = envResult.database.password;
            }
          }
        }
      }
    } catch {
      // Detection is best-effort — don't block setup
    }
  }

  private async checkCredsStatus(): Promise<void> {
    try {
      const status = await this.tauri.invoke<{ exists: boolean; path: string }>('check_credentials_file');
      this.credsExists = status.exists;
    } catch {
      // Ignore
    }
  }

  async browseComposePath(): Promise<void> {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: 'Docker Compose', extensions: ['yml', 'yaml'] }],
        title: 'Select docker-compose.yml'
      });
      if (!selected || !this.config) return;
      this.config.docker_compose_path = typeof selected === 'string' ? selected : String(selected);
    } catch {
      // User cancelled
    }
  }

  async browsePuruDataPath(): Promise<void> {
    try {
      const selected = await open({
        multiple: false,
        directory: true,
        title: 'Select Puru Data Directory'
      });
      if (!selected || !this.config) return;
      this.puruDataPath = typeof selected === 'string' ? selected : String(selected);
      this.config.puru_data_path = this.puruDataPath;
    } catch {
      // User cancelled
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
      this.credsExists = true;
      this.notification.success('Credentials file imported');
    } catch {
      // Error handled by TauriService
    }
  }

  async saveAndContinue(): Promise<void> {
    if (!this.config) return;

    // Validate required fields
    if (!this.config.mysql_password) {
      this.configError = 'MySQL password is required.';
      return;
    }

    this.configSaving = true;
    this.configError = null;

    try {
      await this.tauri.invoke('save_config', { config: this.config });
      this.configSaved = true;
      this.initSteps();
      this.notification.success('Configuration saved');
      await this.loadPrerequisites();
      await this.loadEnabledServices();
    } catch (error) {
      this.configError = String(error);
    } finally {
      this.configSaving = false;
    }
  }

  private initSteps(): void {
    const isNative = this.config?.deployment_mode === 'native';
    const source = isNative ? this.nativeSteps : this.dockerSteps;
    this.steps = source.map(s => ({ ...s, status: 'pending' as const }));
  }

  editConfig(): void {
    this.configSaved = false;
  }

  private async loadPrerequisites(): Promise<void> {
    try {
      const prereqs = await this.tauri.invoke<PrerequisiteStatus[]>('check_prerequisites');
      const isNative = this.config?.deployment_mode === 'native';
      if (isNative) {
        // Native mode — no Docker needed, filter out Docker prerequisites
        this.prerequisites = prereqs.filter(p => p.name !== 'Docker' && p.name !== 'Docker Compose');
        this.detectionResult = null;
      } else {
        this.prerequisites = prereqs;
        this.detectionResult = await this.tauri.invoke<DetectionResult>('detect_existing_setup');
      }

      this.installablePrereqs = this.prerequisites.filter(p => !p.installed && p.installable);
    } catch {
      // Error handled by TauriService
    }
  }

  async loadEnabledServices(): Promise<void> {
    try {
      const modules = await this.tauri.invokeSilent<Record<string, boolean>>('get_service_modules');
      const serviceNames: Record<string, string> = {
        auth: 'Auth', xenon: 'Xenon', has: 'HAS', pacs: 'PACS', argon: 'Argon (Pathology)',
        comm: 'Comm', realtime: 'Realtime', neon: 'Neon (Medical)', mercury: 'Mercury (HRMS)',
        counter: 'Counter', bridge: 'Bridge', integration: 'Integration', hydrogen: 'Hydrogen (Frontend)',
        dviewer: 'DICOM Viewer (dviewer)',
      };
      this.enabledServices = Object.entries(modules)
        .filter(([_, enabled]) => enabled)
        .map(([key]) => serviceNames[key] || key);

      // Check infra notes
      this.infraNotes = [];
      if (this.config?.deployment_mode === 'native') {
        this.infraNotes.push('Native mode — all services run as host processes, no Docker');
      } else {
        const mysqlOnHost = this.prerequisites.some(p => p.name === 'MySQL' && p.installed);
        const rmqOnHost = this.prerequisites.some(p => p.name === 'RabbitMQ' && p.installed);
        if (mysqlOnHost) this.infraNotes.push('MySQL detected on host — Docker container will be skipped');
        if (rmqOnHost) this.infraNotes.push('RabbitMQ detected on host — Docker container will be skipped');
      }
    } catch {
      // Firestore not reachable or hospital code not set — use defaults
      this.enabledServices = ['Auth', 'Xenon', 'HAS', 'PACS', 'Argon', 'Comm', 'Realtime', 'Neon', 'Mercury', 'Counter', 'Bridge', 'Integration', 'Hydrogen', 'DICOM Viewer (dviewer)'];
    }
  }

  async resetSetup(): Promise<void> {
    const isNative = this.config?.deployment_mode === 'native';
    const confirmMsg = isNative
      ? 'This will delete the generated env files. Continue?'
      : 'This will delete the generated docker-compose.yml and env files. Continue?';
    if (!confirm(confirmMsg)) return;
    try {
      const msg = await this.tauri.invoke<string>('setup_reset');
      this.notification.success(msg);
      this.setupComplete = false;
      this.initSteps();
    } catch {
      // Error handled by TauriService
    }
  }

  async recheckPrerequisites(): Promise<void> {
    this.recheckingPrereqs = true;
    try {
      await this.loadPrerequisites();
      const allOk = this.prerequisites.every(p => p.installed);
      if (allOk) {
        this.notification.success('All prerequisites detected');
      }
    } finally {
      this.recheckingPrereqs = false;
    }
  }

  async installMissing(): Promise<void> {
    const names = this.installablePrereqs.map(p => p.name);
    if (names.length === 0) return;

    // Installing prerequisites registers Windows services and writes under
    // Program Files — that needs administrator. Request UAC up front rather than
    // starting a half-install that would fail at the service-registration step.
    try {
      const elevated = await this.tauri.invoke<boolean>('is_elevated');
      if (!elevated) {
        this.notification.warning('Installing prerequisites needs administrator — restarting as admin…');
        await this.tauri.invoke('restart_as_admin');
        return; // app relaunches elevated; the operator runs Install again
      }
    } catch {
      // Couldn't determine elevation — fall through; the backend gate will catch it.
    }

    this.installing = true;
    this.installProgress = null;
    this.installResults = [];

    // Listen for progress events
    let unlisten: UnlistenFn | null = null;
    try {
      unlisten = await listen<InstallProgress>('install-progress', (event) => {
        this.installProgress = event.payload;
      });

      const results = await this.tauri.invoke<InstallResult[]>('install_prerequisites', { software: names });
      this.installResults = results;

      // Re-check prerequisites
      await this.loadPrerequisites();

      const allOk = results.every(r => r.success);
      if (allOk) {
        this.notification.success('All prerequisites installed successfully');
      } else {
        const failed = results.filter(r => !r.success).map(r => r.software);
        this.notification.error(`Failed to install: ${failed.join(', ')}`);
      }
    } catch (err: any) {
      const msg = String(err?.message || err || '');
      if (msg.includes('ELEVATION_REQUIRED')) {
        this.notification.warning('Administrator required — restarting as admin…');
        try { await this.tauri.invoke('restart_as_admin'); } catch {}
        return;
      }
      this.notification.error(msg || 'Installation failed');
    } finally {
      if (unlisten) unlisten();
      this.installing = false;
    }
  }

  /**
   * Download the missing infra installers straight to the OS Downloads folder
   * so the operator can run them by hand — useful when the automated silent
   * install can't run (no admin, locked-down policy, air-gapped copy flows).
   * No install happens here: it's just files on disk.
   */
  async downloadInfraManually(): Promise<void> {
    const names = this.installablePrereqs.map(p => p.name);
    if (names.length === 0) return;

    this.manualDownloading = true;
    this.manualDownloadProgress = null;
    this.manualDownloadResults = [];

    let unlisten: UnlistenFn | null = null;
    try {
      unlisten = await listen<ManualDownloadProgress>('manual-infra-download-progress', (event) => {
        this.manualDownloadProgress = event.payload;
      });

      const results = await this.tauri.invoke<ManualDownloadResult[]>(
        'download_prerequisites_to_downloads',
        { software: names }
      );
      this.manualDownloadResults = results;

      const okCount = results.filter(r => r.success).length;
      if (okCount === results.length) {
        this.notification.success(`Downloaded ${okCount} installer(s) to your Downloads folder`);
      } else if (okCount > 0) {
        this.notification.warning(`Downloaded ${okCount}/${results.length} installer(s); check errors below`);
      } else {
        this.notification.error('Manual download failed');
      }
    } catch (err: any) {
      this.notification.error(String(err?.message || err || 'Manual download failed'));
    } finally {
      if (unlisten) unlisten();
      this.manualDownloading = false;
    }
  }

  async adoptExisting(): Promise<void> {
    this.adopting = true;
    this.mismatchError = null;
    this.configMismatches = [];

    try {
      const result = await this.tauri.invoke<{
        applied: string[];
        mismatches: Array<{ field: string; config_value: string; detected_value: string }>;
      }>('apply_detected_environment', {
        compose_path: this.config?.docker_compose_path || null,
      });

      // Reload config to pick up applied values
      this.config = await this.tauri.invoke<NucleusConfig>('get_config');

      // Check for hospital_code mismatch — hard block
      const codeMismatch = result.mismatches.find(m => m.field === 'hospital_code');
      if (codeMismatch) {
        this.mismatchError = `License has hospital code "${codeMismatch.config_value}" but the existing installation uses "${codeMismatch.detected_value}". These must match.`;
        return;
      }

      // Other mismatches — let user choose
      const otherMismatches = result.mismatches.filter(m => m.field !== 'hospital_code');
      if (otherMismatches.length > 0) {
        this.configMismatches = otherMismatches.map(m => ({ ...m, choice: 'detected' as const }));
        return; // Wait for user to resolve via applyMismatchChoices()
      }

      // No mismatches — proceed
      this.proceedWithAdopt();
    } catch (error) {
      this.notification.error('Detection failed: ' + String(error));
    } finally {
      this.adopting = false;
    }
  }

  async applyMismatchChoices(): Promise<void> {
    if (!this.config) return;

    // Apply user choices to config
    for (const m of this.configMismatches) {
      const value = m.choice === 'detected' ? m.detected_value : m.config_value;
      switch (m.field) {
        case 'server_ip': this.config.server_ip = value; break;
        case 'mysql_user': this.config.mysql_user = value; break;
        case 'mysql_password': this.config.mysql_password = value; break;
      }
    }

    try {
      await this.tauri.invoke('save_config', { config: this.config });
      this.configMismatches = [];
      this.proceedWithAdopt();
    } catch (error) {
      this.notification.error('Failed to save config: ' + String(error));
    }
  }

  private proceedWithAdopt(): void {
    // Skip all setup steps that would modify the existing installation:
    // 0: Check prerequisites — skip (already running)
    // 1: Create databases — skip (already exist)
    // 2: Configure RabbitMQ — skip (already configured)
    // 3: Generate config files — skip (already have env files)
    // 4: Pull Docker images — skip (DO NOT pull/update)
    // 5: Start services — skip (already running)
    // 6: Health check — skip (already running)
    for (let i = 0; i <= 6; i++) {
      this.steps[i].status = 'completed';
    }
    this.notification.success('Adopting existing setup — skipping pull/update steps');
    this.startSetup(); // Will run: Configure backups → Install daemon → TLS
  }

  freshInstall(): void {
    this.detectionResult = null;
    this.notification.warning('Starting fresh install - existing containers will be removed');
  }

  /**
   * Attach the live progress listeners used during setup (per-service native
   * start/sync + JAR download progress) and return a single detach function.
   * Shared by the full run and single-step re-runs so both behave identically.
   */
  private async attachSetupListeners(): Promise<() => void> {
    const unlisteners: UnlistenFn[] = [];
    try {
      unlisteners.push(await listen<{ service: string; action: string; success: boolean; message: string }>(
        'native-service-progress',
        (event) => {
          const incoming = event.payload;
          const idx = this.nativeActions.findIndex(a => a.service === incoming.service);
          if (idx >= 0) {
            this.nativeActions[idx] = incoming;
          } else {
            this.nativeActions = [...this.nativeActions, incoming];
          }
        }
      ));
    } catch {
      // Events unavailable (e.g. remote) — non-fatal.
    }
    try {
      unlisteners.push(await listen<{ service: string; phase: string; message: string; downloaded: number; total: number; percent: number; index: number; count: number }>(
        'jar-update-progress',
        (event) => {
          const p = event.payload;
          const step = this.steps.find(s => s.label.includes('Pull JARs'));
          if (!step) return;
          const indeterminate = p.phase === 'downloading' && p.total === 0;
          step.progressState = p.phase === 'done' ? 'complete'
            : p.phase === 'error' ? 'fail'
            : indeterminate ? 'indeterminate'
            : 'active';
          step.percent = p.percent;
          step.message = p.message;
        }
      ));
    } catch {
      // Non-fatal.
    }
    return () => unlisteners.forEach(u => u());
  }

  async startSetup(): Promise<void> {
    this.setupInProgress = true;
    this.nativeActions = [];
    const detach = await this.attachSetupListeners();

    try {
      for (let i = 0; i < this.steps.length; i++) {
        if (this.steps[i].status === 'completed') continue;

        this.steps[i].status = 'in_progress';

        try {
          await this.executeStep(i);
          this.steps[i].status = 'completed';
        } catch (error) {
          this.steps[i].status = 'error';
          this.steps[i].message = String(error);
          this.setupInProgress = false;
          return;
        }
      }
    } finally {
      detach();
    }

    this.setupComplete = true;
    this.setupInProgress = false;

    // Record that setup has completed so the shell can hide config screens by
    // default from now on (revealed again via the Configuration unlock button).
    try {
      await this.tauri.invokeSilent('mark_setup_completed');
    } catch { /* non-critical */ }

    // Load TLS status after setup
    try {
      this.tlsStatus = await this.tauri.invokeSilent('get_tls_status');
    } catch { /* non-critical */ }

    this.notification.success('Setup completed successfully!');
  }

  /**
   * Run a single setup step on demand — for re-running an individual step that
   * failed, without re-executing the whole sequence. Steps can depend on earlier
   * ones, so this is a targeted recovery tool, not a substitute for a full run.
   */
  async runSingleStep(index: number): Promise<void> {
    if (this.setupInProgress || this.singleStepIndex !== null) return;
    const step = this.steps[index];
    if (!step) return;

    this.singleStepIndex = index;
    this.nativeActions = [];
    step.status = 'in_progress';
    step.message = undefined;
    step.percent = undefined;
    step.progressState = undefined;

    const detach = await this.attachSetupListeners();
    try {
      await this.executeStep(index);
      step.status = 'completed';
      this.notification.success(`${step.label} — done`);
    } catch (error) {
      step.status = 'error';
      step.message = String(error);
      this.notification.error(`${step.label} failed`);
    } finally {
      detach();
      this.singleStepIndex = null;
    }
  }

  private async executeStep(index: number): Promise<void> {
    const isNative = this.config?.deployment_mode === 'native';

    const dockerCommands = [
      'setup_check_prerequisites',
      'setup_create_databases',
      'setup_configure_rabbitmq',
      'setup_generate_config',
      'setup_pull_images',
      'setup_start_services',
      'setup_health_check',
      'setup_configure_backups',
      'setup_install_daemon',
      'setup_tls'
    ];

    const nativeCommands = [
      'setup_check_prerequisites',
      'setup_create_databases',
      'setup_configure_rabbitmq',
      'setup_generate_env_files',
      'setup_pull_jars',
      'setup_seed_queues',
      'setup_start_native_services',
      'setup_health_check',
      'setup_seed_database',
      'setup_init_auth',
      'setup_configure_backups',
      'setup_install_daemon',
      'setup_tls'
    ];

    const commands = isNative ? nativeCommands : dockerCommands;
    const command = commands[index];

    // The daemon is already installed on the server it runs on — there's no
    // remote equivalent, so skip this step (rather than error) in remote mode.
    if (this.isRemote && command === 'setup_install_daemon') {
      this.steps[index].message = 'Skipped — daemon already running on the server';
      return;
    }

    await this.tauri.invoke(command);
  }

  cancel(): void {
    if (this.setupInProgress) {
      if (confirm('Setup is in progress. Are you sure you want to cancel?')) {
        this.router.navigate(['/dashboard']);
      }
    } else {
      this.router.navigate(['/dashboard']);
    }
  }

  viewLogs(): void {
    this.router.navigate(['/services']);
    this.notification.success('Use the Services page to view container logs');
  }

  async downloadClientScript(): Promise<void> {
    try {
      const script = await this.tauri.invoke<string>('generate_client_setup_script');
      const blob = new Blob([script], { type: 'application/bat' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = 'puru-secure-setup.bat';
      a.click();
      URL.revokeObjectURL(url);
      this.notification.success('Setup script downloaded');
    } catch {
      // Error handled by TauriService
    }
  }

  async copyNginxConfig(): Promise<void> {
    try {
      const config = await this.tauri.invoke<string>('generate_nginx_https_config');
      await navigator.clipboard.writeText(config);
      this.notification.success('Nginx HTTPS config copied to clipboard');
    } catch {
      // Error handled by TauriService
    }
  }

  finish(): void {
    this.router.navigate(['/dashboard']);
  }
}
