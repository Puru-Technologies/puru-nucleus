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
import { Router } from '@angular/router';
import { TauriService, NucleusConfig, PrerequisiteStatus, DetectionResult } from '../../core/services/tauri.service';
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
    MatDividerModule
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

                  <mat-form-field appearance="outline" class="full-width">
                    <mat-label>Docker Compose Path</mat-label>
                    <input matInput [(ngModel)]="config.docker_compose_path">
                    <mat-hint>Where to write the generated docker-compose.yml</mat-hint>
                  </mat-form-field>

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

            <!-- Detection Result -->
            @if (detectionResult?.found) {
              <div class="detection-banner">
                <mat-icon>info</mat-icon>
                <div class="detection-info">
                  <strong>Existing Puru Setup Detected</strong>
                  <p>Found {{ detectionResult!.containers.length }} containers</p>
                </div>
                <div class="detection-actions">
                  <button mat-stroked-button (click)="adoptExisting()">
                    Adopt Existing
                  </button>
                  <button mat-stroked-button (click)="freshInstall()">
                    Fresh Install
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
                      } @else {
                        Not installed (requires {{ prereq.required_version }})
                      }
                    </span>
                  </mat-list-item>
                }
              </mat-list>
            </div>

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

    mat-card-actions {
      display: flex;
      justify-content: flex-end;
      gap: 0.5rem;
      padding-top: 1rem;
      border-top: 1px solid #eee;
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

  prerequisites: PrerequisiteStatus[] = [];
  detectionResult: DetectionResult | null = null;
  setupInProgress = false;
  setupComplete = false;

  steps: SetupStep[] = [
    { label: 'Check prerequisites', status: 'pending' },
    { label: 'Create MySQL databases', status: 'pending' },
    { label: 'Configure RabbitMQ', status: 'pending' },
    { label: 'Generate configuration files', status: 'pending' },
    { label: 'Pull Docker images', status: 'pending' },
    { label: 'Start services', status: 'pending' },
    { label: 'Health check', status: 'pending' },
    { label: 'Configure backups', status: 'pending' },
    { label: 'Install daemon service', status: 'pending' }
  ];

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
      await this.checkCredsStatus();

      // If MySQL password is already set, skip to phase 2 automatically
      if (this.config.mysql_password) {
        this.configSaved = true;
        await this.loadPrerequisites();
      }
    } catch {
      // Will show defaults
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
      this.notification.success('Configuration saved');
      await this.loadPrerequisites();
    } catch (error) {
      this.configError = String(error);
    } finally {
      this.configSaving = false;
    }
  }

  editConfig(): void {
    this.configSaved = false;
  }

  private async loadPrerequisites(): Promise<void> {
    try {
      const [prereqs, detection] = await Promise.all([
        this.tauri.invoke<PrerequisiteStatus[]>('check_prerequisites'),
        this.tauri.invoke<DetectionResult>('detect_existing_setup')
      ]);

      this.prerequisites = prereqs;
      this.detectionResult = detection;
    } catch {
      // Error handled by TauriService
    }
  }

  async adoptExisting(): Promise<void> {
    // Auto-detect environment from env files and apply to config
    try {
      await this.tauri.invokeSilent('apply_detected_environment', {
        compose_path: this.config?.docker_compose_path || null,
      });
      // Reload config to pick up detected values
      this.config = await this.tauri.invoke<NucleusConfig>('get_config');
    } catch {
      // Non-critical — config can be manually set
    }

    this.steps[0].status = 'completed';
    this.steps[1].status = 'completed';
    this.steps[2].status = 'completed';
    this.notification.success('Adopting existing setup');
    this.startSetup();
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
    this.notification.success('Setup completed successfully!');
  }

  private async executeStep(index: number): Promise<void> {
    const stepCommands = [
      'setup_check_prerequisites',
      'setup_create_databases',
      'setup_configure_rabbitmq',
      'setup_generate_config',
      'setup_pull_images',
      'setup_start_services',
      'setup_health_check',
      'setup_configure_backups',
      'setup_install_daemon'
    ];

    await this.tauri.invoke(stepCommands[index]);
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

  finish(): void {
    this.router.navigate(['/dashboard']);
  }
}
