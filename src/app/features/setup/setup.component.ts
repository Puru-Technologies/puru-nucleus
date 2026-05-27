import { Component, OnInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { MatCardModule } from '@angular/material/card';
import { MatIconModule } from '@angular/material/icon';
import { MatButtonModule } from '@angular/material/button';
import { MatStepperModule } from '@angular/material/stepper';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatListModule } from '@angular/material/list';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';
import { MatDividerModule } from '@angular/material/divider';
import { MatButtonToggleModule } from '@angular/material/button-toggle';
import { Router } from '@angular/router';
import { TauriService, NucleusConfig, PrerequisiteStatus, DetectionResult, InstallProgress, InstallResult } from '../../core/services/tauri.service';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { NotificationService } from '../../core/services/notification.service';
import { open } from '@tauri-apps/plugin-dialog';

interface SetupStep {
  label: string;
  status: 'pending' | 'in_progress' | 'completed' | 'error';
  message?: string;
}

@Component({
  selector: 'app-setup',
  standalone: true,
  imports: [
    CommonModule,
    FormsModule,
    MatCardModule,
    MatIconModule,
    MatButtonModule,
    MatStepperModule,
    MatProgressSpinnerModule,
    MatProgressBarModule,
    MatListModule,
    MatFormFieldModule,
    MatInputModule,
    MatDividerModule,
    MatButtonToggleModule
  ],
  template: `
    <div class="setup-page">
      <mat-card class="setup-card">
        <mat-card-header>
          <mat-card-title>PURU NUCLEUS - Setup</mat-card-title>
          <mat-card-subtitle>{{ config?.hospital_code || 'Loading...' }}</mat-card-subtitle>
        </mat-card-header>

        <mat-card-content>
          <!-- Phase 1: Infrastructure Configuration -->
          @if (!configSaved) {
            <div class="setup-section">
              <h3>
                <mat-icon class="section-icon">settings</mat-icon>
                Step 1: Configure Infrastructure
              </h3>
              <p class="section-desc">
                Set the connection details for your hospital's MySQL database and Docker environment.
                These are required before the automated setup can run.
              </p>

              @if (config) {
                <div class="config-form">
                  <div class="form-row">
                    <mat-form-field appearance="outline" class="form-field">
                      <mat-label>MySQL Host</mat-label>
                      <input matInput [(ngModel)]="config.mysql_host" placeholder="127.0.0.1">
                    </mat-form-field>
                    <mat-form-field appearance="outline" class="form-field-sm">
                      <mat-label>Port</mat-label>
                      <input matInput type="number" [(ngModel)]="config.mysql_port" placeholder="3306">
                    </mat-form-field>
                  </div>

                  <div class="form-row">
                    <mat-form-field appearance="outline" class="form-field">
                      <mat-label>MySQL User</mat-label>
                      <input matInput [(ngModel)]="config.mysql_user" placeholder="root">
                    </mat-form-field>
                    <mat-form-field appearance="outline" class="form-field">
                      <mat-label>MySQL Password</mat-label>
                      <input matInput type="password" [(ngModel)]="config.mysql_password">
                      <mat-hint>Root password for the MySQL server</mat-hint>
                    </mat-form-field>
                  </div>

                  <div class="mode-selector">
                    <span class="mode-label">Deployment Mode</span>
                    <mat-button-toggle-group [(ngModel)]="config.deployment_mode" class="mode-toggle">
                      <mat-button-toggle value="docker">
                        <mat-icon>cloud</mat-icon> Docker
                      </mat-button-toggle>
                      <mat-button-toggle value="native">
                        <mat-icon>computer</mat-icon> Native (JAR)
                      </mat-button-toggle>
                    </mat-button-toggle-group>
                    <span class="mode-hint">
                      @if (config.deployment_mode === 'native') {
                        Runs services as Java processes — no Docker required. Best for low-spec hardware.
                      } @else {
                        Runs services as Docker containers. Requires Docker Desktop or Docker Engine.
                      }
                    </span>
                  </div>

                  @if (config.deployment_mode !== 'native') {
                    <div class="browse-row">
                      <mat-form-field appearance="outline" class="flex-1">
                        <mat-label>Docker Compose Path</mat-label>
                        <input matInput [(ngModel)]="config.docker_compose_path" placeholder="/home/puru/docker/docker-compose.yml">
                        <mat-hint>Path to docker-compose.yml</mat-hint>
                      </mat-form-field>
                      <button mat-stroked-button type="button" (click)="browseComposePath()">
                        <mat-icon>folder_open</mat-icon> Browse
                      </button>
                    </div>
                  }

                  <div class="browse-row">
                    <mat-form-field appearance="outline" class="flex-1">
                      <mat-label>Puru Data Directory</mat-label>
                      <input matInput [(ngModel)]="puruDataPath" placeholder="/home/puru/puru-data">
                      <mat-hint>Root directory for hospital data, documents, and uploads</mat-hint>
                    </mat-form-field>
                    <button mat-stroked-button type="button" (click)="browsePuruDataPath()">
                      <mat-icon>folder_open</mat-icon> Browse
                    </button>
                  </div>

                  <mat-form-field appearance="outline" class="full-width">
                    <mat-label>Server IP (local network)</mat-label>
                    <input matInput [(ngModel)]="config.server_ip" placeholder="192.168.1.100">
                    <mat-hint>This machine's IP on the hospital LAN</mat-hint>
                  </mat-form-field>

                  <div class="creds-row">
                    <div class="creds-label">
                      <span class="label-text">GCS Credentials</span>
                      <span class="label-hint">For cloud backups and config sync</span>
                    </div>
                    @if (credsExists) {
                      <span class="creds-ok"><mat-icon>check_circle</mat-icon> Configured</span>
                    } @else {
                      <span class="creds-missing">Not set</span>
                    }
                    <button mat-stroked-button type="button" (click)="browseCredentials()">
                      <mat-icon>folder_open</mat-icon>
                      {{ credsExists ? 'Replace' : 'Browse' }}
                    </button>
                  </div>

                  @if (configError) {
                    <div class="config-error">
                      <mat-icon>error</mat-icon>
                      <span>{{ configError }}</span>
                    </div>
                  }
                </div>
              } @else {
                <div class="loading-inline">
                  <mat-spinner diameter="24"></mat-spinner>
                  <span>Loading configuration...</span>
                </div>
              }
            </div>
          }

          <!-- Phase 2: Prerequisites + Setup (shown after config is saved) -->
          @if (configSaved) {
            <div class="config-saved-banner">
              <mat-icon>check_circle</mat-icon>
              <div>
                <strong>Configuration saved</strong>
                <span>MySQL {{ config!.mysql_user }}{{'@'}}{{ config!.mysql_host }}:{{ config!.mysql_port }}</span>
              </div>
              <button mat-stroked-button (click)="editConfig()">Edit</button>
            </div>

            <!-- Mode Badge -->
            <div class="mode-badge" [class.native]="config!.deployment_mode === 'native'">
              <mat-icon>{{ config!.deployment_mode === 'native' ? 'computer' : 'cloud' }}</mat-icon>
              <span>{{ config!.deployment_mode === 'native' ? 'Native (JAR) Mode' : 'Docker Mode' }}</span>
            </div>

            <!-- Detection Result (Docker only) -->
            @if (detectionResult?.found && config!.deployment_mode !== 'native') {
              <div class="detection-banner">
                <mat-icon>info</mat-icon>
                <div class="detection-info">
                  <strong>Existing Puru Setup Detected</strong>
                  <p>Found {{ detectionResult!.containers.length }} containers</p>
                </div>
                <div class="detection-actions">
                  <button mat-stroked-button (click)="adoptExisting()" [disabled]="adopting">
                    @if (adopting) {
                      <mat-spinner diameter="18"></mat-spinner>
                    }
                    Adopt Existing
                  </button>
                  <button mat-stroked-button (click)="freshInstall()">
                    Fresh Install
                  </button>
                </div>
              </div>
            }

            <!-- Mismatch Warning -->
            @if (mismatchError) {
              <div class="mismatch-error">
                <mat-icon>error</mat-icon>
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
                  <mat-icon>warning</mat-icon>
                  <strong>Config values differ from detected environment</strong>
                </div>
                <div class="mismatch-list">
                  @for (m of configMismatches; track m.field) {
                    <div class="mismatch-item">
                      <span class="mismatch-field">{{ m.field }}</span>
                      <div class="mismatch-choices">
                        <button mat-stroked-button [class.selected]="m.choice === 'config'"
                                (click)="m.choice = 'config'" class="choice-btn">
                          Keep: {{ m.config_value }}
                        </button>
                        <button mat-stroked-button [class.selected]="m.choice === 'detected'"
                                (click)="m.choice = 'detected'" class="choice-btn">
                          Use detected: {{ m.detected_value }}
                        </button>
                      </div>
                    </div>
                  }
                </div>
                <div class="mismatch-actions">
                  <button mat-raised-button color="primary" (click)="applyMismatchChoices()">
                    Apply & Continue
                  </button>
                </div>
              </div>
            }

            <!-- Prerequisites Section -->
            <div class="setup-section">
              <h3>Prerequisites</h3>
              <mat-list>
                @for (prereq of prerequisites; track prereq.name) {
                  <mat-list-item>
                    <mat-icon matListItemIcon [class]="prereq.installed ? 'status-success' : 'status-error'">
                      {{ prereq.installed ? 'check_circle' : 'cancel' }}
                    </mat-icon>
                    <span matListItemTitle>{{ prereq.name }}</span>
                    <span matListItemLine>
                      @if (prereq.installed) {
                        Version {{ prereq.version }}
                      } @else if (prereq.required_version) {
                        Not installed (requires {{ prereq.required_version }}+)
                      } @else {
                        Not installed
                      }
                    </span>
                  </mat-list-item>
                }
              </mat-list>

              <!-- Re-check / Install Missing Buttons -->
              <div class="prereq-actions">
                @if (!installing) {
                  <button mat-stroked-button (click)="recheckPrerequisites()" [disabled]="recheckingPrereqs">
                    <mat-icon>{{ recheckingPrereqs ? 'hourglass_empty' : 'refresh' }}</mat-icon>
                    {{ recheckingPrereqs ? 'Checking...' : 'Re-check' }}
                  </button>
                }
                @if (installablePrereqs.length > 0 && !installing) {
                  <button mat-raised-button color="accent" (click)="installMissing()">
                    <mat-icon>download</mat-icon>
                    Install {{ installableNames }}
                  </button>
                  <span class="install-hint">Downloads and installs automatically.</span>
                }
              </div>

              <!-- Install Progress -->
              @if (installing && installProgress) {
                <div class="install-progress-card">
                  <div class="install-progress-header">
                    <mat-icon>{{ installProgress.stage === 'completed' ? 'check_circle' : installProgress.stage === 'failed' ? 'error' : 'downloading' }}</mat-icon>
                    <span><strong>{{ installProgress.software }}:</strong> {{ installProgress.message }}</span>
                  </div>
                  <mat-progress-bar
                    [mode]="installProgress.stage === 'installing' || installProgress.stage === 'verifying' ? 'indeterminate' : 'determinate'"
                    [value]="installProgress.percent">
                  </mat-progress-bar>
                </div>
              }

              <!-- Install Results -->
              @if (installResults.length > 0 && !installing) {
                <div class="install-results">
                  @for (result of installResults; track result.software) {
                    <div class="install-result" [class.success]="result.success" [class.failure]="!result.success">
                      <mat-icon>{{ result.success ? 'check_circle' : 'cancel' }}</mat-icon>
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
                      <span class="infra-note"><mat-icon>info_outline</mat-icon> {{ note }}</span>
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
                        <mat-icon>check</mat-icon>
                      } @else if (step.status === 'in_progress') {
                        <mat-spinner diameter="20"></mat-spinner>
                      } @else if (step.status === 'error') {
                        <mat-icon>error</mat-icon>
                      } @else {
                        <span class="step-number">{{ i + 1 }}</span>
                      }
                    </div>
                    <div class="step-content">
                      <div class="step-label">{{ step.label }}</div>
                      @if (step.message) {
                        <div class="step-message">{{ step.message }}</div>
                      }
                    </div>
                  </div>
                }
              </div>
            </div>

            <!-- TLS Setup Result -->
            @if (setupComplete && tlsStatus?.configured) {
              <div class="setup-section">
                <h3>
                  <mat-icon class="section-icon" style="color:#4caf50">lock</mat-icon>
                  HTTPS Configured
                </h3>
                <div class="tls-info">
                  <p>HTTPS is ready. Client machines need a one-time certificate install.</p>
                  <div class="tls-actions">
                    <button mat-raised-button color="primary" (click)="downloadClientScript()">
                      <mat-icon>download</mat-icon>
                      Download Client Setup Script (.bat)
                    </button>
                    <button mat-stroked-button (click)="copyNginxConfig()">
                      <mat-icon>content_copy</mat-icon>
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
                <mat-progress-bar mode="determinate" [value]="progressPercent"></mat-progress-bar>
                <span class="progress-text">{{ progressPercent }}% Complete</span>
              </div>
            }
          }
        </mat-card-content>

        <mat-card-actions>
          <button mat-button (click)="cancel()">Cancel</button>
          @if (configSaved) {
            <button mat-button (click)="viewLogs()">View Logs</button>
          }

          <!-- Save config button (Phase 1) -->
          @if (!configSaved && config) {
            <button mat-raised-button color="primary"
                    (click)="saveAndContinue()"
                    [disabled]="!config.mysql_password || configSaving">
              @if (configSaving) {
                <mat-spinner diameter="18"></mat-spinner>
              }
              Save & Continue
            </button>
          }

          <!-- Start Setup button (Phase 2) -->
          @if (configSaved && !setupInProgress && !setupComplete) {
            <button mat-raised-button color="primary" (click)="startSetup()" [disabled]="!allPrerequisitesMet">
              Start Setup
            </button>
          }
          @if (setupComplete) {
            <button mat-raised-button color="primary" (click)="finish()">
              Finish
            </button>
          }
          @if (configSaved && !setupInProgress) {
            <button mat-stroked-button color="warn" (click)="resetSetup()">
              <mat-icon>restart_alt</mat-icon>
              Reset Setup
            </button>
          }
        </mat-card-actions>
      </mat-card>
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

      mat-icon {
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

    .mode-toggle {
      mat-button-toggle {
        mat-icon {
          font-size: 18px;
          width: 18px;
          height: 18px;
          margin-right: 4px;
          vertical-align: middle;
        }
      }
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

      mat-icon {
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

      mat-icon {
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

      mat-icon {
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

        mat-icon {
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

        mat-icon {
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
        mat-icon { color: #4caf50; }
      }

      &.failure {
        background: #ffebee;
        color: #c62828;
        mat-icon { color: #f44336; }
      }

      mat-icon {
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
    }

    .progress-section {
      margin-top: 1.5rem;
      text-align: center;

      mat-progress-bar {
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

        mat-icon {
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

    mat-card-actions {
      display: flex;
      justify-content: flex-end;
      gap: 0.5rem;
      padding-top: 1rem;
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

      mat-icon { font-size: 24px; width: 24px; height: 24px; flex-shrink: 0; margin-top: 2px; }
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

      mat-icon { font-size: 20px; width: 20px; height: 20px; }
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
  private router = inject(Router);

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
  detectionResult: DetectionResult | null = null;
  setupInProgress = false;
  setupComplete = false;
  tlsStatus: any = null;
  adopting = false;
  mismatchError: string | null = null;
  configMismatches: Array<{ field: string; config_value: string; detected_value: string; choice: 'config' | 'detected' }> = [];
  enabledServices: string[] = [];
  infraNotes: string[] = [];

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
    { label: 'Start services', status: 'pending' },
    { label: 'Health check', status: 'pending' },
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
      const isNative = this.config?.deployment_mode === 'native';
      const prereqs = await this.tauri.invoke<PrerequisiteStatus[]>('check_prerequisites');

      if (isNative) {
        // In native mode, filter out Docker/Docker Compose — not needed
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
      };
      this.enabledServices = Object.entries(modules)
        .filter(([_, enabled]) => enabled)
        .map(([key]) => serviceNames[key] || key);

      // Check infra notes
      this.infraNotes = [];
      if (this.config?.deployment_mode === 'native') {
        this.infraNotes.push('Native mode — services will run as Java processes (no Docker)');
      } else {
        const mysqlOnHost = this.prerequisites.some(p => p.name === 'MySQL' && p.installed);
        const rmqOnHost = this.prerequisites.some(p => p.name === 'RabbitMQ' && p.installed);
        if (mysqlOnHost) this.infraNotes.push('MySQL detected on host — Docker container will be skipped');
        if (rmqOnHost) this.infraNotes.push('RabbitMQ detected on host — Docker container will be skipped');
      }
    } catch {
      // Firestore not reachable or hospital code not set — use defaults
      this.enabledServices = ['Auth', 'Xenon', 'HAS', 'PACS', 'Argon', 'Comm', 'Realtime', 'Neon', 'Mercury', 'Counter', 'Bridge', 'Integration', 'Hydrogen'];
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
      this.notification.error(err?.message || err || 'Installation failed');
    } finally {
      if (unlisten) unlisten();
      this.installing = false;
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

  async startSetup(): Promise<void> {
    this.setupInProgress = true;

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

    this.setupComplete = true;
    this.setupInProgress = false;

    // Load TLS status after setup
    try {
      this.tlsStatus = await this.tauri.invokeSilent('get_tls_status');
    } catch { /* non-critical */ }

    this.notification.success('Setup completed successfully!');
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
      'setup_start_native_services',
      'setup_health_check',
      'setup_configure_backups',
      'setup_install_daemon',
      'setup_tls'
    ];

    const commands = isNative ? nativeCommands : dockerCommands;
    await this.tauri.invoke(commands[index]);
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
