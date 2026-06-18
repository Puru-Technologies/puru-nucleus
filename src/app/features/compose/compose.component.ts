import { Component, OnInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
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
  ],
  template: `
    <div class="compose-page p-4">
      <h1>Docker Compose</h1>

      @if (loading) {
        <div class="loading-container">
          <span class="spinner spinner-lg"></span>
        </div>
      } @else {
        <!-- Status Banner -->
        @if (statusMessage) {
          <div class="status-banner" [class.downloaded]="fragmentsDownloaded" [class.existing]="!fragmentsDownloaded">
            <span class="material-icons">{{ fragmentsDownloaded ? 'cloud_download' : 'check_circle' }}</span>
            <span>{{ statusMessage }}</span>
          </div>
        }

        <!-- Variables Panel -->
        <div class="card panel">
          <button class="panel-header" type="button" (click)="panelOpen = !panelOpen">
            <span class="panel-title">
              <span class="material-icons">tune</span>
              <span class="panel-title-text">
                <span class="panel-title-main">Template Variables</span>
                <span class="panel-title-desc">Configure placeholders before applying</span>
              </span>
            </span>
            <span class="material-icons chevron" [class.open]="panelOpen">expand_more</span>
          </button>

          @if (panelOpen) {
            <div class="panel-body card-pad">
            <div class="variables-grid">
              <div class="field">
                <label>Hospital Code</label>
                <input class="input" [(ngModel)]="variables.hospital_code">
              </div>

              <div class="field">
                <label>Hospital Name</label>
                <input class="input" [(ngModel)]="variables.hospital_name">
              </div>

              <div class="field">
                <label>Address Line 2</label>
                <input class="input" [(ngModel)]="variables.hospital_line2">
              </div>

              <div class="field">
                <label>Address Line 3</label>
                <input class="input" [(ngModel)]="variables.hospital_line3">
              </div>

              <div class="field">
                <label>Registration No.</label>
                <input class="input" [(ngModel)]="variables.hospital_reg_no">
              </div>

              <div class="field">
                <label>Logo URL</label>
                <input class="input" [(ngModel)]="variables.hospital_logo_url">
              </div>

              <!-- Barcode prefixes -->
              <div class="barcode-header">
                <span>Barcode Prefixes</span>
                <button class="btn btn-stroked btn-sm" (click)="populateBarcodeDefaults()" type="button">
                  Populate Defaults
                </button>
              </div>

              <div class="field">
                <label>Inventory Prefix</label>
                <input class="input" [(ngModel)]="variables.barcode_prefix_inventory">
              </div>

              <div class="field">
                <label>PPIN Prefix</label>
                <input class="input" [(ngModel)]="variables.barcode_prefix_ppin">
              </div>

              <div class="field">
                <label>Pathology Prefix</label>
                <input class="input" [(ngModel)]="variables.barcode_prefix_pathology">
              </div>

              <div class="field">
                <label>Employee Prefix</label>
                <input class="input" [(ngModel)]="variables.employee_prefix">
              </div>

              <div class="field">
                <label>Sale Invoice Prefix</label>
                <input class="input" [(ngModel)]="variables.barcode_prefix_sale">
              </div>

              <div class="field">
                <label>Return Invoice Prefix</label>
                <input class="input" [(ngModel)]="variables.barcode_prefix_return">
              </div>

              <div class="field">
                <label>Server IP</label>
                <input class="input" [(ngModel)]="variables.server_ip">
              </div>

              <div class="field">
                <label>MySQL Password</label>
                <input class="input" type="password" [(ngModel)]="variables.mysql_password">
              </div>

              <div class="field">
                <label>RabbitMQ Password</label>
                <input class="input" [(ngModel)]="variables.rabbitmq_password">
              </div>

              <div class="field">
                <label>Auth Tag</label>
                <input class="input" [(ngModel)]="variables.auth_tag">
              </div>

              <div class="field">
                <label>Xenon Tag</label>
                <input class="input" [(ngModel)]="variables.xenon_tag">
              </div>

              <div class="field">
                <label>HAS Tag</label>
                <input class="input" [(ngModel)]="variables.has_tag">
              </div>

              <div class="field">
                <label>PACS Tag</label>
                <input class="input" [(ngModel)]="variables.pacs_tag">
              </div>

              <div class="field">
                <label>Argon Tag</label>
                <input class="input" [(ngModel)]="variables.argon_tag">
              </div>

              <div class="field">
                <label>Comm Tag</label>
                <input class="input" [(ngModel)]="variables.comm_tag">
              </div>

              <div class="field">
                <label>Realtime Tag</label>
                <input class="input" [(ngModel)]="variables.realtime_tag">
              </div>

              <div class="field">
                <label>Neon Tag</label>
                <input class="input" [(ngModel)]="variables.neon_tag">
              </div>

              <div class="field">
                <label>Bridge Tag</label>
                <input class="input" [(ngModel)]="variables.bridge_tag">
              </div>

              <div class="field">
                <label>Hydrogen Tag</label>
                <input class="input" [(ngModel)]="variables.hydrogen_tag">
              </div>
            </div>

            <div class="divider"></div>

            <div class="modules-section">
              <div class="modules-label">Service Modules</div>
              <div class="modules-hint">
                Controlled from cloud (Oxygen). MySQL and RabbitMQ run on host, not in Docker.
              </div>
              <div class="modules-grid">
                <label class="check"><input type="checkbox" [ngModel]="modules.auth" disabled> <span>Auth</span></label>
                <label class="check"><input type="checkbox" [ngModel]="modules.xenon" disabled> <span>Xenon (Backend)</span></label>
                <label class="check"><input type="checkbox" [ngModel]="modules.has" disabled> <span>HAS</span></label>
                <label class="check"><input type="checkbox" [ngModel]="modules.pacs" disabled> <span>PACS (Radiology)</span></label>
                <label class="check"><input type="checkbox" [ngModel]="modules.argon" disabled> <span>Pathology (Argon)</span></label>
                <label class="check"><input type="checkbox" [ngModel]="modules.comm" disabled> <span>Communication (Comm)</span></label>
                <label class="check"><input type="checkbox" [ngModel]="modules.realtime" disabled> <span>Realtime</span></label>
                <label class="check"><input type="checkbox" [ngModel]="modules.neon" disabled> <span>Pharmacy (Neon)</span></label>
                <label class="check"><input type="checkbox" [ngModel]="modules.bridge" disabled> <span>Bridge</span></label>
                <label class="check"><input type="checkbox" [ngModel]="modules.integration" disabled> <span>Integration</span></label>
                <label class="check"><input type="checkbox" [ngModel]="modules.hydrogen" disabled> <span>Hydrogen (Frontend)</span></label>
              </div>
            </div>

            <div class="panel-actions">
              <button class="btn btn-primary" (click)="applyVariables()" [disabled]="applying">
                @if (applying) {
                  <span class="spinner"></span>
                } @else {
                  <span class="material-icons">find_replace</span>
                }
                Apply Variables
              </button>
            </div>
            </div>
          }
        </div>

        <!-- File Tabs: docker-compose.yml + env files -->
        <div class="tabs">
          <button class="tab" type="button" [class.active]="activeTab === 0" (click)="onTabChange(0)">
            <span class="material-icons tab-icon">code</span>
            docker-compose.yml
          </button>
          @for (env of envFiles; track env.name; let i = $index) {
            <button class="tab" type="button" [class.active]="activeTab === i + 1" (click)="onTabChange(i + 1)">
              <span class="material-icons tab-icon">settings</span>
              {{ env.name }}
            </button>
          }
        </div>

        <!-- Compose Panel -->
        @if (activeTab === 0) {
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
        }

        <!-- Env File Panels -->
        @for (env of envFiles; track env.name; let i = $index) {
          @if (activeTab === i + 1) {
            <div class="tab-content">
              <div class="file-path">{{ envDir }}/{{ env.name }}</div>
              <textarea
                class="file-editor env-editor"
                [(ngModel)]="env.content"
                spellcheck="false"
                [placeholder]="env.name + ' content...'"
              ></textarea>
            </div>
          }
        }

        <!-- Action Bar -->
        <div class="actions">
          <button class="btn btn-stroked" (click)="resetContent()" [disabled]="saving || uploading">
            <span class="material-icons">refresh</span>
            Reset
          </button>
          <button class="btn btn-stroked" (click)="saveLocally()" [disabled]="saving || uploading">
            @if (saving) {
              <span class="spinner"></span>
            } @else {
              <span class="material-icons">save</span>
            }
            Save All Locally
          </button>
          <button class="btn btn-primary" (click)="saveAndUpload()" [disabled]="saving || uploading">
            @if (uploading) {
              <span class="spinner"></span>
            } @else {
              <span class="material-icons">cloud_upload</span>
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

      .material-icons {
        font-size: 20px;
        width: 20px;
        height: 20px;
      }
    }

    .panel {
      margin-bottom: 1.5rem;
    }

    .panel-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      width: 100%;
      padding: 16px;
      background: none;
      border: none;
      cursor: pointer;
      text-align: left;
      font: inherit;
    }

    .panel-title {
      display: flex;
      align-items: center;
      gap: 12px;

      .material-icons {
        font-size: 20px;
        width: 20px;
        height: 20px;
        color: #666;
      }
    }

    .panel-title-text {
      display: flex;
      flex-direction: column;
    }

    .panel-title-main {
      font-weight: 600;
    }

    .panel-title-desc {
      font-size: 0.75rem;
      color: #888;
    }

    .chevron {
      transition: transform 0.2s ease;
      color: #666;
    }

    .chevron.open {
      transform: rotate(180deg);
    }

    .panel-body {
      border-top: 1px solid #eee;
    }

    .divider {
      height: 1px;
      background: #eee;
      margin: 1rem 0;
    }

    .check {
      display: flex;
      align-items: center;
      gap: 8px;
      font-size: 0.875rem;
      cursor: default;
    }

    .check input {
      width: 16px;
      height: 16px;
    }

    .tabs {
      display: flex;
      gap: 4px;
      border-bottom: 1px solid #eee;
      margin-bottom: 0.5rem;
      flex-wrap: wrap;
    }

    .tab {
      display: inline-flex;
      align-items: center;
      gap: 4px;
      padding: 10px 14px;
      background: none;
      border: none;
      border-bottom: 2px solid transparent;
      cursor: pointer;
      font: inherit;
      color: #666;
    }

    .tab.active {
      color: var(--primary, #1565c0);
      border-bottom-color: var(--primary, #1565c0);
      font-weight: 600;
    }

    .btn .material-icons,
    .tab .material-icons {
      font-size: 18px;
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
  panelOpen = false;

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
    counter: true,
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
