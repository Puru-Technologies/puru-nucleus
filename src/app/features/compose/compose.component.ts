import { Component, OnInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { MatCardModule } from '@angular/material/card';
import { MatIconModule } from '@angular/material/icon';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';
import { MatExpansionModule } from '@angular/material/expansion';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
import { MatTabsModule } from '@angular/material/tabs';
import { MatSlideToggleModule } from '@angular/material/slide-toggle';
import { MatDividerModule } from '@angular/material/divider';
import {
  TauriService,
  NucleusConfig,
  FragmentDownloadResult,
  TemplateVariables,
  ComposeUploadResult,
  EnvFileEntry,
  EnvDownloadResult,
  EnvUploadResult,
  ServiceModules,
} from '../../core/services/tauri.service';
import { NotificationService } from '../../core/services/notification.service';

@Component({
  selector: 'app-compose',
  standalone: true,
  imports: [
    CommonModule,
    FormsModule,
    MatCardModule,
    MatIconModule,
    MatButtonModule,
    MatFormFieldModule,
    MatInputModule,
    MatExpansionModule,
    MatProgressSpinnerModule,
    MatTabsModule,
    MatSlideToggleModule,
    MatDividerModule,
  ],
  template: `
    <div class="compose-page p-4">
      <h1>Docker Compose</h1>

      @if (loading) {
        <div class="loading-container">
          <mat-spinner diameter="48"></mat-spinner>
        </div>
      } @else {
        <!-- Status Banner -->
        @if (statusMessage) {
          <div class="status-banner" [class.downloaded]="fragmentsDownloaded" [class.existing]="!fragmentsDownloaded">
            <mat-icon>{{ fragmentsDownloaded ? 'cloud_download' : 'check_circle' }}</mat-icon>
            <span>{{ statusMessage }}</span>
          </div>
        }

        <!-- Variables Panel -->
        <mat-accordion>
          <mat-expansion-panel>
            <mat-expansion-panel-header>
              <mat-panel-title>
                <mat-icon>tune</mat-icon>
                Template Variables
              </mat-panel-title>
              <mat-panel-description>
                Configure placeholders before applying
              </mat-panel-description>
            </mat-expansion-panel-header>

            <div class="variables-grid">
              <mat-form-field appearance="outline">
                <mat-label>Hospital Code</mat-label>
                <input matInput [(ngModel)]="variables.hospital_code">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>Hospital Name</mat-label>
                <input matInput [(ngModel)]="variables.hospital_name">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>Address Line 2</mat-label>
                <input matInput [(ngModel)]="variables.hospital_line2">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>Address Line 3</mat-label>
                <input matInput [(ngModel)]="variables.hospital_line3">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>Registration No.</mat-label>
                <input matInput [(ngModel)]="variables.hospital_reg_no">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>Logo URL</mat-label>
                <input matInput [(ngModel)]="variables.hospital_logo_url">
              </mat-form-field>

              <!-- Barcode prefixes -->
              <div class="barcode-header">
                <span>Barcode Prefixes</span>
                <button mat-stroked-button color="primary" (click)="populateBarcodeDefaults()" type="button">
                  Populate Defaults
                </button>
              </div>

              <mat-form-field appearance="outline">
                <mat-label>Inventory Prefix</mat-label>
                <input matInput [(ngModel)]="variables.barcode_prefix_inventory">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>PPIN Prefix</mat-label>
                <input matInput [(ngModel)]="variables.barcode_prefix_ppin">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>Pathology Prefix</mat-label>
                <input matInput [(ngModel)]="variables.barcode_prefix_pathology">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>Employee Prefix</mat-label>
                <input matInput [(ngModel)]="variables.employee_prefix">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>Sale Invoice Prefix</mat-label>
                <input matInput [(ngModel)]="variables.barcode_prefix_sale">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>Return Invoice Prefix</mat-label>
                <input matInput [(ngModel)]="variables.barcode_prefix_return">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>Server IP</mat-label>
                <input matInput [(ngModel)]="variables.server_ip">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>MySQL Password</mat-label>
                <input matInput type="password" [(ngModel)]="variables.mysql_password">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>RabbitMQ Password</mat-label>
                <input matInput [(ngModel)]="variables.rabbitmq_password">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>Auth Tag</mat-label>
                <input matInput [(ngModel)]="variables.auth_tag">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>Xenon Tag</mat-label>
                <input matInput [(ngModel)]="variables.xenon_tag">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>HAS Tag</mat-label>
                <input matInput [(ngModel)]="variables.has_tag">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>PACS Tag</mat-label>
                <input matInput [(ngModel)]="variables.pacs_tag">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>Argon Tag</mat-label>
                <input matInput [(ngModel)]="variables.argon_tag">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>Comm Tag</mat-label>
                <input matInput [(ngModel)]="variables.comm_tag">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>Realtime Tag</mat-label>
                <input matInput [(ngModel)]="variables.realtime_tag">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>Neon Tag</mat-label>
                <input matInput [(ngModel)]="variables.neon_tag">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>Bridge Tag</mat-label>
                <input matInput [(ngModel)]="variables.bridge_tag">
              </mat-form-field>

              <mat-form-field appearance="outline">
                <mat-label>Hydrogen Tag</mat-label>
                <input matInput [(ngModel)]="variables.hydrogen_tag">
              </mat-form-field>
            </div>

            <mat-divider></mat-divider>

            <div class="modules-section">
              <div class="modules-label">Service Modules</div>
              <div class="modules-hint">
                Controlled from cloud (Oxygen). MySQL and RabbitMQ run on host, not in Docker.
              </div>
              <div class="modules-grid">
                <mat-slide-toggle [ngModel]="modules.auth" disabled>Auth</mat-slide-toggle>
                <mat-slide-toggle [ngModel]="modules.xenon" disabled>Xenon (Backend)</mat-slide-toggle>
                <mat-slide-toggle [ngModel]="modules.has" disabled>HAS</mat-slide-toggle>
                <mat-slide-toggle [ngModel]="modules.pacs" disabled>PACS (Radiology)</mat-slide-toggle>
                <mat-slide-toggle [ngModel]="modules.argon" disabled>Pathology (Argon)</mat-slide-toggle>
                <mat-slide-toggle [ngModel]="modules.comm" disabled>Communication (Comm)</mat-slide-toggle>
                <mat-slide-toggle [ngModel]="modules.realtime" disabled>Realtime</mat-slide-toggle>
                <mat-slide-toggle [ngModel]="modules.neon" disabled>Pharmacy (Neon)</mat-slide-toggle>
                <mat-slide-toggle [ngModel]="modules.bridge" disabled>Bridge</mat-slide-toggle>
                <mat-slide-toggle [ngModel]="modules.integration" disabled>Integration</mat-slide-toggle>
                <mat-slide-toggle [ngModel]="modules.hydrogen" disabled>Hydrogen (Frontend)</mat-slide-toggle>
              </div>
            </div>

            <div class="panel-actions">
              <button mat-raised-button color="primary" (click)="applyVariables()" [disabled]="applying">
                @if (applying) {
                  <mat-spinner diameter="18"></mat-spinner>
                } @else {
                  <mat-icon>find_replace</mat-icon>
                }
                Apply Variables
              </button>
            </div>
          </mat-expansion-panel>
        </mat-accordion>

        <!-- File Tabs: docker-compose.yml + env files -->
        <mat-tab-group (selectedIndexChange)="onTabChange($event)" [selectedIndex]="activeTab">
          <!-- Compose Tab -->
          <mat-tab>
            <ng-template mat-tab-label>
              <mat-icon class="tab-icon">code</mat-icon>
              docker-compose.yml
            </ng-template>

            <div class="tab-content">
              @if (filePath) {
                <div class="file-path">{{ filePath }}</div>
              }
              <textarea
                class="file-editor"
                [(ngModel)]="composeContent"
                spellcheck="false"
                placeholder="Compose file content will appear here..."
              ></textarea>
            </div>
          </mat-tab>

          <!-- Env File Tabs -->
          @for (env of envFiles; track env.name) {
            <mat-tab>
              <ng-template mat-tab-label>
                <mat-icon class="tab-icon">settings</mat-icon>
                {{ env.name }}
              </ng-template>

              <div class="tab-content">
                <div class="file-path">{{ envDir }}/{{ env.name }}</div>
                <textarea
                  class="file-editor env-editor"
                  [(ngModel)]="env.content"
                  spellcheck="false"
                  [placeholder]="env.name + ' content...'"
                ></textarea>
              </div>
            </mat-tab>
          }
        </mat-tab-group>

        <!-- Action Bar -->
        <div class="actions">
          <button mat-stroked-button (click)="resetContent()" [disabled]="saving || uploading">
            <mat-icon>refresh</mat-icon>
            Reset
          </button>
          <button mat-raised-button (click)="saveLocally()" [disabled]="saving || uploading">
            @if (saving) {
              <mat-spinner diameter="18"></mat-spinner>
            } @else {
              <mat-icon>save</mat-icon>
            }
            Save All Locally
          </button>
          <button mat-raised-button color="primary" (click)="saveAndUpload()" [disabled]="saving || uploading">
            @if (uploading) {
              <mat-spinner diameter="18"></mat-spinner>
            } @else {
              <mat-icon>cloud_upload</mat-icon>
            }
            Save & Upload to Cloud
          </button>
        </div>
      }
    </div>
  `,
  styles: [`
    .compose-page {
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

    .status-banner {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 12px 16px;
      border-radius: 8px;
      margin-bottom: 1.5rem;
      font-size: 0.875rem;

      &.downloaded {
        background: #e3f2fd;
        color: #1565c0;
      }

      &.existing {
        background: #e8f5e9;
        color: #2e7d32;
      }

      mat-icon {
        font-size: 20px;
        width: 20px;
        height: 20px;
      }
    }

    mat-accordion {
      margin-bottom: 1.5rem;
    }

    mat-panel-title {
      display: flex;
      align-items: center;
      gap: 8px;

      mat-icon {
        font-size: 20px;
        width: 20px;
        height: 20px;
        color: #666;
      }
    }

    .variables-grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
      gap: 0 1rem;
    }

    .modules-section {
      padding: 1rem 0;
    }

    .modules-label {
      font-size: 0.8rem;
      font-weight: 600;
      color: #666;
      text-transform: uppercase;
      letter-spacing: 0.04em;
      margin-bottom: 0.75rem;
    }

    .modules-hint {
      font-size: 0.75rem;
      color: #888;
      margin-bottom: 0.75rem;
    }

    .modules-grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
      gap: 0.75rem;
    }

    .panel-actions {
      display: flex;
      justify-content: flex-end;
      gap: 0.75rem;
      padding-top: 0.5rem;
    }

    .tab-icon {
      margin-right: 6px;
      font-size: 18px;
      width: 18px;
      height: 18px;
    }

    .tab-content {
      padding-top: 1rem;
    }

    .file-path {
      font-family: 'SF Mono', 'Fira Code', monospace;
      font-size: 0.7rem;
      color: #666;
      margin-bottom: 0.5rem;
    }

    .file-editor {
      width: 100%;
      min-height: 500px;
      font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
      font-size: 0.8rem;
      line-height: 1.5;
      padding: 16px;
      border: 1px solid #ddd;
      border-radius: 8px;
      background: #fafafa;
      resize: vertical;
      tab-size: 2;
      white-space: pre;
      overflow-wrap: normal;
      overflow-x: auto;
    }

    .env-editor {
      min-height: 300px;
    }

    .actions {
      display: flex;
      gap: 0.75rem;
      justify-content: flex-end;
      padding-top: 1rem;
      margin-top: 1.5rem;
      border-top: 1px solid #eee;
    }
  `]
})
export class ComposeComponent implements OnInit {
  private tauri = inject(TauriService);
  private notification = inject(NotificationService);

  loading = true;
  composeContent = '';
  originalContent = '';
  filePath = '';
  envDir = '';
  statusMessage = '';
  fragmentsDownloaded = false;
  activeTab = 0;

  saving = false;
  uploading = false;
  applying = false;

  envFiles: EnvFileEntry[] = [];

  modules: ServiceModules = {
    auth: true,
    xenon: true,
    has: true,
    pacs: true,
    argon: true,
    comm: true,
    realtime: true,
    neon: true,
    mercury: true,
    bridge: true,
    integration: true,
    hydrogen: true,
  };

  variables: TemplateVariables = {
    hospital_code: '',
    hospital_name: '',
    hospital_line2: '',
    hospital_line3: '',
    hospital_reg_no: '',
    hospital_logo_url: 'https://storage.googleapis.com/puru-public-files/logo-new-puru.png',
    barcode_prefix_inventory: '',
    barcode_prefix_ppin: '',
    barcode_prefix_pathology: '',
    employee_prefix: 'EMP',
    barcode_prefix_sale: '2526S',
    barcode_prefix_return: '2526R',
    server_ip: '',
    mysql_password: '',
    rabbitmq_password: 'puru123',
    auth_tag: 'latest',
    xenon_tag: 'latest',
    has_tag: 'latest',
    pacs_tag: 'latest',
    argon_tag: 'latest',
    comm_tag: 'latest',
    realtime_tag: 'latest',
    neon_tag: 'latest',
    bridge_tag: 'latest',
    integration_tag: 'latest',
    hydrogen_tag: 'latest',
  };

  async ngOnInit(): Promise<void> {
    try {
      // Load config to pre-populate variables
      const config = await this.tauri.invokeSilent<NucleusConfig>('get_config');
      this.variables.hospital_code = config.hospital_code;
      this.variables.server_ip = config.server_ip;
      this.variables.mysql_password = config.mysql_password;

      // Fetch modules from Firestore (read-only)
      try {
        this.modules = await this.tauri.invoke<ServiceModules>('get_service_modules');
      } catch {
        // If Firestore unavailable, default to all enabled
      }

      // Download fragment files from GCS (skips existing)
      const fragResult = await this.tauri.invoke<FragmentDownloadResult>('download_compose_template');
      this.fragmentsDownloaded = fragResult.downloaded.length > 0;

      // Assemble compose from fragments + enabled modules
      this.composeContent = await this.tauri.invoke<string>('assemble_compose_file', {
        modules: this.modules,
      });
      this.originalContent = this.composeContent;

      const enabledCount = Object.values(this.modules).filter(Boolean).length;
      this.statusMessage = this.fragmentsDownloaded
        ? `Downloaded ${fragResult.downloaded.length} fragments, assembled from ${enabledCount} services`
        : `Assembled from ${enabledCount} service fragments`;

      // Try to get existing file path for display
      try {
        await this.tauri.invokeSilent<string>('get_compose_content');
        this.filePath = config.docker_compose_path || '';
      } catch {
        // No existing file yet — that's fine, we assembled from fragments
      }

      // Download env templates (skips files that exist)
      try {
        const envResult = await this.tauri.invoke<EnvDownloadResult>('download_env_templates');
        this.envDir = envResult.env_dir;
        if (envResult.files_downloaded.length > 0) {
          this.statusMessage += ` | ${envResult.files_downloaded.length} env files downloaded`;
        }
      } catch {
        // Env download is optional — may not have templates on GCS yet
      }

      // Load env files
      this.envFiles = await this.tauri.invokeSilent<EnvFileEntry[]>('get_env_files');
    } catch (error) {
      // Error handled by TauriService
    } finally {
      this.loading = false;
    }
  }

  onTabChange(index: number): void {
    this.activeTab = index;
  }

  async applyVariables(): Promise<void> {
    this.applying = true;
    try {
      // Apply to compose content
      this.composeContent = await this.tauri.invoke<string>('substitute_compose_variables', {
        content: this.composeContent,
        variables: this.variables,
      });

      // Apply to all env files too
      for (const env of this.envFiles) {
        env.content = await this.tauri.invoke<string>('substitute_compose_variables', {
          content: env.content,
          variables: this.variables,
        });
      }

      this.notification.success('Variables applied to all files');
    } catch (error) {
      // Error handled by TauriService
    } finally {
      this.applying = false;
    }
  }

  populateBarcodeDefaults(): void {
    const code = this.variables.hospital_code || 'XXX';
    const prefix = code.substring(0, 3).toUpperCase();
    this.variables.barcode_prefix_inventory = prefix;
    this.variables.barcode_prefix_ppin = prefix;
    this.variables.barcode_prefix_pathology = '';
    this.variables.employee_prefix = 'EMP';
    this.variables.barcode_prefix_sale = '2526S';
    this.variables.barcode_prefix_return = '2526R';
    this.notification.success('Barcode defaults populated');
  }

  async saveLocally(): Promise<void> {
    this.saving = true;
    try {
      // Save compose
      const path = await this.tauri.invoke<string>('save_compose_content', {
        content: this.composeContent,
      });
      this.filePath = path;
      this.originalContent = this.composeContent;

      // Save all env files
      for (const env of this.envFiles) {
        await this.tauri.invoke<string>('save_env_file', {
          name: env.name,
          content: env.content,
        });
      }

      this.notification.success('All files saved locally');
    } catch (error) {
      // Error handled by TauriService
    } finally {
      this.saving = false;
    }
  }

  async saveAndUpload(): Promise<void> {
    this.uploading = true;
    try {
      // Save all locally first
      const path = await this.tauri.invoke<string>('save_compose_content', {
        content: this.composeContent,
      });
      this.filePath = path;
      this.originalContent = this.composeContent;

      for (const env of this.envFiles) {
        await this.tauri.invoke<string>('save_env_file', {
          name: env.name,
          content: env.content,
        });
      }

      // Upload compose
      const composeResult = await this.tauri.invoke<ComposeUploadResult>('upload_compose_to_cloud');

      // Upload env files
      const envResult = await this.tauri.invoke<EnvUploadResult>('upload_env_files_to_cloud');

      this.notification.success(
        `Uploaded compose + ${envResult.files_uploaded.length} env files to cloud`
      );
    } catch (error) {
      // Error handled by TauriService
    } finally {
      this.uploading = false;
    }
  }

  async resetContent(): Promise<void> {
    try {
      this.composeContent = await this.tauri.invoke<string>('get_compose_content');
      this.originalContent = this.composeContent;
      this.envFiles = await this.tauri.invokeSilent<EnvFileEntry[]>('get_env_files');
      this.notification.success('All files reset from disk');
    } catch (error) {
      // Error handled by TauriService
    }
  }
}
