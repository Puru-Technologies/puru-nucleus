import { Component, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import { TauriService } from '../../core/services/tauri.service';
import { NotificationService } from '../../core/services/notification.service';
import { open } from '@tauri-apps/plugin-dialog';

interface CredentialsStatus {
  exists: boolean;
  path: string;
}

@Component({
  selector: 'app-activation',
  standalone: true,
  imports: [
    CommonModule,
    FormsModule
  ],
  template: `
    <div class="activation-page">
      <div class="card card-pad activation-card">
        <div class="logo">
          <span class="material-icons">hub</span>
          <h1>PURU NUCLEUS</h1>
        </div>

        <div class="card-content">
          <!-- Step 1: GCS Credentials -->
          @if (!credentialsReady) {
            <h2>Step 1: Service Account</h2>
            <p class="subtitle">
              Upload the GCS service account JSON file received from your admin.
            </p>

            @if (checking) {
              <div class="checking-state">
                <span class="spinner"></span>
                <span>Checking for credentials...</span>
              </div>
            } @else {
              <!-- Action buttons -->
              <div class="creds-actions">
                <button class="btn btn-stroked creds-btn"
                        (click)="browseFile()"
                        [disabled]="importing">
                  <span class="material-icons">folder_open</span>
                  <div class="btn-text">
                    <span class="btn-label">Browse File</span>
                    <span class="btn-hint">Select from this computer</span>
                  </div>
                </button>

                <button class="btn btn-stroked creds-btn"
                        (click)="showPaste = !showPaste"
                        [disabled]="importing">
                  <span class="material-icons">content_paste</span>
                  <div class="btn-text">
                    <span class="btn-label">Paste JSON</span>
                    <span class="btn-hint">From WhatsApp / email</span>
                  </div>
                </button>
              </div>

              <!-- Paste area -->
              @if (showPaste) {
                <div class="paste-area">
                  <div class="field full-width">
                    <label>Paste service account JSON here</label>
                    <textarea class="input"
                              [(ngModel)]="pastedJson"
                              rows="6"
                              placeholder='{"type": "service_account", "project_id": "puru-255206", ...}'></textarea>
                    <span class="field-hint">Paste the full JSON content from the credentials file</span>
                  </div>
                  <button class="btn btn-primary"
                          (click)="savePastedJson()"
                          [disabled]="!pastedJson.trim() || importing">
                    @if (importing) {
                      <span class="spinner"></span>
                    }
                    Save Credentials
                  </button>
                </div>
              }

              @if (importing) {
                <div class="checking-state">
                  <span class="spinner"></span>
                  <span>Importing credentials...</span>
                </div>
              }

              <!-- Not found hint -->
              @if (!showPaste && !importing) {
                <div class="hint-box">
                  <span class="material-icons">info_outline</span>
                  <span>
                    Ask your admin to send the <strong>service-account.json</strong> file.
                    They can share it via WhatsApp, email, or USB drive.
                  </span>
                </div>
              }
            }
          }

          <!-- Step 2: Email Activation -->
          @if (credentialsReady) {
            <div class="creds-saved-banner">
              <span class="material-icons">check_circle</span>
              <span class="banner-title">Service account configured</span>
              <button class="btn-icon" (click)="resetCredentials()" title="Edit">
                <span class="material-icons">edit</span>
              </button>
            </div>

            <h2>Step 2: Activate</h2>
            <p class="subtitle">
              Enter the hospital email and machine name to activate
            </p>

            <div class="field full-width">
              <label>Hospital Email</label>
              <input class="input" type="email"
                     [(ngModel)]="email"
                     [disabled]="activating"
                     placeholder="admin{{'@'}}hospital.com"
                     (keyup.enter)="activate()">
              <span class="field-hint">This email was used during hospital onboarding</span>
            </div>

            <div class="field full-width">
              <label>Machine Name</label>
              <input class="input" type="text"
                     [(ngModel)]="machineName"
                     [disabled]="activating"
                     placeholder="Main Server"
                     (keyup.enter)="activate()">
              <span class="field-hint">A friendly name for this computer (e.g. "Main Server", "Reception PC")</span>
            </div>

            @if (fingerprint) {
              <div class="fingerprint-box">
                <span class="material-icons">fingerprint</span>
                <div class="fp-text">
                  <span class="fp-label">Hardware Fingerprint</span>
                  <code class="fp-value">{{ fingerprint }}</code>
                </div>
              </div>
            }

            <button class="btn btn-primary activate-btn"
                    (click)="activate()"
                    [disabled]="!email || !machineName.trim() || activating">
              @if (activating) {
                <span class="spinner"></span>
                Activating...
              } @else {
                <span class="material-icons">verified</span>
                Activate
              }
            </button>
          }

          @if (error) {
            <div class="error-message">
              <span class="material-icons">error</span>
              <span>{{ error }}</span>
            </div>
          }
        </div>

        <div class="help-text">
          <p>Don't have credentials?</p>
          <a href="mailto:support{{'@'}}purutechnologies.com">Contact Support</a>
        </div>
      </div>
    </div>
  `,
  styles: [`
    .activation-page {
      display: flex;
      justify-content: center;
      align-items: center;
      min-height: 100vh;
      background: linear-gradient(135deg, #1a237e 0%, #3f51b5 100%);
    }

    .activation-card {
      width: 100%;
      max-width: 480px;
      padding: 2rem;
      text-align: center;
    }

    .logo {
      margin-bottom: 2rem;

      .material-icons {
        font-size: 4rem;
        width: 4rem;
        height: 4rem;
        color: #3f51b5;
      }

      h1 {
        margin: 0.5rem 0 0;
        font-size: 1.5rem;
        font-weight: 500;
        letter-spacing: 0.1em;
        color: #1a237e;
      }
    }

    h2 {
      margin: 0 0 0.5rem;
      font-size: 1.25rem;
      font-weight: 500;
      color: #333;
    }

    .subtitle {
      margin: 0 0 1.5rem;
      color: #666;
      font-size: 0.875rem;
    }

    .full-width {
      width: 100%;
    }

    .field-hint {
      display: block;
      font-size: 0.75rem;
      color: #999;
      margin-top: 4px;
    }

    .field-error {
      display: block;
      color: #c62828;
      font-size: 0.75rem;
      margin-top: 4px;
    }

    /* In-button spinner sizing */
    .btn .spinner {
      width: 18px;
      height: 18px;
      margin-right: 0.5rem;
      vertical-align: middle;
    }

    /* ── Credentials actions ───────────────── */
    .creds-actions {
      display: flex;
      gap: 12px;
      margin-bottom: 1.25rem;
    }

    .creds-btn {
      flex: 1;
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 14px 16px !important;
      height: auto !important;
      text-align: left;
      border-radius: 10px !important;

      .material-icons {
        font-size: 24px;
        width: 24px;
        height: 24px;
        color: #3f51b5;
      }

      .btn-text {
        display: flex;
        flex-direction: column;
      }

      .btn-label {
        font-weight: 600;
        font-size: 0.85rem;
        line-height: 1.2;
      }

      .btn-hint {
        font-size: 0.7rem;
        color: #999;
        font-weight: 400;
      }
    }

    /* ── Paste area ────────────────────────── */
    .paste-area {
      margin-bottom: 1rem;
      text-align: left;

      textarea {
        font-family: 'SF Mono', 'Fira Code', monospace;
        font-size: 0.75rem;
      }

      button {
        width: 100%;
        margin-top: 4px;
      }
    }

    /* ── States ─────────────────────────────── */
    .checking-state {
      display: flex;
      align-items: center;
      justify-content: center;
      gap: 12px;
      padding: 1.5rem;
      color: #666;
      font-size: 0.875rem;
    }

    .hint-box {
      display: flex;
      align-items: flex-start;
      gap: 8px;
      padding: 12px;
      background: #fff3e0;
      border-radius: 8px;
      text-align: left;
      font-size: 0.8rem;
      color: #e65100;

      .material-icons {
        font-size: 18px;
        width: 18px;
        height: 18px;
        flex-shrink: 0;
        margin-top: 1px;
      }
    }

    /* ── Credentials saved ─────────────────── */
    .creds-saved-banner {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 10px 14px;
      background: #e8f5e9;
      border-radius: 8px;
      margin-bottom: 1.5rem;

      > .material-icons {
        color: #4caf50;
        font-size: 22px;
        width: 22px;
        height: 22px;
        flex-shrink: 0;
      }

      .banner-title {
        flex: 1;
        font-size: 0.85rem;
        font-weight: 600;
        color: #2e7d32;
      }

      button {
        color: #999;
      }
    }

    /* ── Fingerprint box ───────────────────── */
    .fingerprint-box {
      display: flex;
      align-items: flex-start;
      gap: 10px;
      padding: 12px 14px;
      background: #f5f5f5;
      border: 1px solid #e0e0e0;
      border-radius: 8px;
      margin-bottom: 1rem;
      text-align: left;

      > .material-icons {
        color: #3f51b5;
        font-size: 22px;
        width: 22px;
        height: 22px;
        flex-shrink: 0;
        margin-top: 2px;
      }

      .fp-text {
        display: flex;
        flex-direction: column;
        gap: 4px;
        min-width: 0;
      }

      .fp-label {
        font-size: 0.75rem;
        font-weight: 600;
        color: #666;
      }

      .fp-value {
        font-family: 'SF Mono', 'Fira Code', monospace;
        font-size: 0.7rem;
        color: #333;
        word-break: break-all;
        line-height: 1.4;
      }
    }

    /* ── Activate button ───────────────────── */
    .activate-btn {
      width: 100%;
      margin-top: 1rem;
      padding: 0.75rem;

      .spinner {
        display: inline-block;
        margin-right: 0.5rem;
      }
    }

    /* ── Error ──────────────────────────────── */
    .error-message {
      display: flex;
      align-items: center;
      justify-content: center;
      gap: 0.5rem;
      margin-top: 1rem;
      padding: 0.75rem;
      background: #ffebee;
      border-radius: 4px;
      color: #c62828;
      font-size: 0.875rem;

      .material-icons {
        font-size: 1.25rem;
        width: 1.25rem;
        height: 1.25rem;
      }
    }

    /* ── Help text ─────────────────────────── */
    .help-text {
      margin-top: 1.5rem;
      padding-top: 1.5rem;
      border-top: 1px solid #eee;
      font-size: 0.875rem;
      color: #666;

      p {
        margin: 0 0 0.25rem;
      }

      a {
        color: #3f51b5;
        text-decoration: none;

        &:hover {
          text-decoration: underline;
        }
      }
    }
  `]
})
export class ActivationComponent {
  private tauri = inject(TauriService);
  private notification = inject(NotificationService);
  private router = inject(Router);

  // Step 1 state
  credentialsReady = false;
  checking = true;
  importing = false;
  showPaste = false;
  pastedJson = '';
  designatedPath = '';

  // Step 2 state
  email = '';
  machineName = '';
  fingerprint = '';
  activating = false;
  error: string | null = null;

  constructor() {
    this.checkCredentials();
  }

  private async checkCredentials(): Promise<void> {
    this.checking = true;
    try {
      const status = await this.tauri.invoke<CredentialsStatus>('check_credentials_file');
      this.designatedPath = status.path;
      if (status.exists) {
        this.credentialsReady = true;
        this.loadFingerprint();
      }
    } catch {
      // First run — no config dir yet
    } finally {
      this.checking = false;
    }
  }

  async browseFile(): Promise<void> {
    this.error = null;
    try {
      const selected = await open({
        multiple: false,
        filters: [{
          name: 'JSON',
          extensions: ['json']
        }],
        title: 'Select GCS Service Account JSON'
      });

      if (!selected) return; // user cancelled

      const filePath = typeof selected === 'string' ? selected : selected;
      this.importing = true;

      await this.tauri.invoke('import_credentials_file', { sourcePath: filePath });
      this.credentialsReady = true;
      this.loadFingerprint();
      this.notification.success('Credentials file imported');
    } catch (error) {
      this.error = String(error);
    } finally {
      this.importing = false;
    }
  }

  async savePastedJson(): Promise<void> {
    if (!this.pastedJson.trim()) return;

    this.error = null;
    this.importing = true;

    try {
      await this.tauri.invoke('save_credentials_content', { content: this.pastedJson.trim() });
      this.credentialsReady = true;
      this.loadFingerprint();
      this.showPaste = false;
      this.notification.success('Credentials saved');
    } catch (error) {
      this.error = String(error);
    } finally {
      this.importing = false;
    }
  }

  resetCredentials(): void {
    this.credentialsReady = false;
    this.showPaste = false;
    this.pastedJson = '';
    this.error = null;
  }

  private async loadFingerprint(): Promise<void> {
    try {
      this.fingerprint = await this.tauri.invoke<string>('get_machine_fingerprint');
    } catch {
      // Non-fatal — fingerprint will just not be shown
    }
  }

  async activate(): Promise<void> {
    if (!this.email || !this.machineName.trim()) return;

    this.activating = true;
    this.error = null;

    try {
      await this.tauri.invoke('activate_license', {
        email: this.email,
        machineName: this.machineName.trim()
      });
      this.notification.success('Activation successful!');
      this.router.navigate(['/setup']);
    } catch (error) {
      this.error = String(error);
    } finally {
      this.activating = false;
    }
  }
}
