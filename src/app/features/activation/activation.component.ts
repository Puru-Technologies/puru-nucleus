import { Component, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { MatCardModule } from '@angular/material/card';
import { MatIconModule } from '@angular/material/icon';
import { MatButtonModule } from '@angular/material/button';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatInputModule } from '@angular/material/input';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
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
    FormsModule,
    MatCardModule,
    MatIconModule,
    MatButtonModule,
    MatFormFieldModule,
    MatInputModule,
    MatProgressSpinnerModule
  ],
  template: `
    <div class="activation-page">
      <mat-card class="activation-card">
        <div class="logo">
          <mat-icon>hub</mat-icon>
          <h1>PURU NUCLEUS</h1>
        </div>

        <mat-card-content>
          <!-- Step 1: GCS Credentials -->
          @if (!credentialsReady) {
            <h2>Step 1: Service Account</h2>
            <p class="subtitle">
              Upload the GCS service account JSON file received from your admin.
            </p>

            @if (checking) {
              <div class="checking-state">
                <mat-spinner diameter="24"></mat-spinner>
                <span>Checking for credentials...</span>
              </div>
            } @else {
              <!-- Action buttons -->
              <div class="creds-actions">
                <button mat-stroked-button
                        class="creds-btn"
                        (click)="browseFile()"
                        [disabled]="importing">
                  <mat-icon>folder_open</mat-icon>
                  <div class="btn-text">
                    <span class="btn-label">Browse File</span>
                    <span class="btn-hint">Select from this computer</span>
                  </div>
                </button>

                <button mat-stroked-button
                        class="creds-btn"
                        (click)="showPaste = !showPaste"
                        [disabled]="importing">
                  <mat-icon>content_paste</mat-icon>
                  <div class="btn-text">
                    <span class="btn-label">Paste JSON</span>
                    <span class="btn-hint">From WhatsApp / email</span>
                  </div>
                </button>
              </div>

              <!-- Paste area -->
              @if (showPaste) {
                <div class="paste-area">
                  <mat-form-field appearance="outline" class="full-width">
                    <mat-label>Paste service account JSON here</mat-label>
                    <textarea matInput
                              [(ngModel)]="pastedJson"
                              rows="6"
                              placeholder='{"type": "service_account", "project_id": "puru-255206", ...}'></textarea>
                    <mat-hint>Paste the full JSON content from the credentials file</mat-hint>
                  </mat-form-field>
                  <button mat-raised-button color="primary"
                          (click)="savePastedJson()"
                          [disabled]="!pastedJson.trim() || importing">
                    @if (importing) {
                      <mat-spinner diameter="18"></mat-spinner>
                    }
                    Save Credentials
                  </button>
                </div>
              }

              @if (importing) {
                <div class="checking-state">
                  <mat-spinner diameter="24"></mat-spinner>
                  <span>Importing credentials...</span>
                </div>
              }

              <!-- Not found hint -->
              @if (!showPaste && !importing) {
                <div class="hint-box">
                  <mat-icon>info_outline</mat-icon>
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
              <mat-icon>check_circle</mat-icon>
              <span class="banner-title">Service account configured</span>
              <button mat-icon-button (click)="resetCredentials()">
                <mat-icon>edit</mat-icon>
              </button>
            </div>

            <h2>Step 2: Activate</h2>
            <p class="subtitle">
              Enter the hospital email and machine name to activate
            </p>

            <mat-form-field appearance="outline" class="full-width">
              <mat-label>Hospital Email</mat-label>
              <input matInput type="email"
                     [(ngModel)]="email"
                     [disabled]="activating"
                     placeholder="admin{{'@'}}hospital.com"
                     (keyup.enter)="activate()">
              <mat-icon matPrefix>email</mat-icon>
              <mat-hint>This email was used during hospital onboarding</mat-hint>
            </mat-form-field>

            <mat-form-field appearance="outline" class="full-width">
              <mat-label>Machine Name</mat-label>
              <input matInput type="text"
                     [(ngModel)]="machineName"
                     [disabled]="activating"
                     placeholder="Main Server"
                     (keyup.enter)="activate()">
              <mat-icon matPrefix>computer</mat-icon>
              <mat-hint>A friendly name for this computer (e.g. "Main Server", "Reception PC")</mat-hint>
            </mat-form-field>

            @if (fingerprint) {
              <div class="fingerprint-box">
                <mat-icon>fingerprint</mat-icon>
                <div class="fp-text">
                  <span class="fp-label">Hardware Fingerprint</span>
                  <code class="fp-value">{{ fingerprint }}</code>
                </div>
              </div>
            }

            <button mat-raised-button color="primary"
                    class="activate-btn"
                    (click)="activate()"
                    [disabled]="!email || !machineName.trim() || activating">
              @if (activating) {
                <mat-spinner diameter="20"></mat-spinner>
                Activating...
              } @else {
                <mat-icon>verified</mat-icon>
                Activate
              }
            </button>
          }

          @if (error) {
            <div class="error-message">
              <mat-icon>error</mat-icon>
              <span>{{ error }}</span>
            </div>
          }
        </mat-card-content>

        <div class="help-text">
          <p>Don't have credentials?</p>
          <a href="mailto:support{{'@'}}purutechnologies.com">Contact Support</a>
        </div>
      </mat-card>
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

      mat-icon {
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

      mat-icon {
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

      mat-icon {
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

      > mat-icon {
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

      > mat-icon {
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

      mat-spinner {
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

      mat-icon {
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
