import { Component, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import { TauriService } from '../../core/services/tauri.service';
import { NotificationService } from '../../core/services/notification.service';
import { PuruLogoComponent } from '../../core/components/puru-logo.component';
import { open } from '@tauri-apps/plugin-dialog';

interface CredentialsStatus {
  exists: boolean;
  path: string;
}

interface SystemClockStatus {
  checked: boolean;
  ok: boolean;
  skew_seconds: number;
  tolerance_seconds: number;
  local_time: string;
  server_time: string | null;
  timezone_offset: string;
  source: string | null;
  message: string;
}

@Component({
  selector: 'app-activation',
  standalone: true,
  imports: [
    CommonModule,
    FormsModule,
    PuruLogoComponent
  ],
  template: `
    <div class="activation-page">
      <div class="card card-pad activation-card">
        <div class="logo">
          <puru-logo variant="full" [size]="52"></puru-logo>
          <div class="product">NUCLEUS</div>
        </div>

        <div class="card-content">
          <!-- System clock warning — a skewed clock makes GCP sign-in fail -->
          @if (clock && clock.checked && !clock.ok) {
            <div class="clock-warning">
              <span class="material-icons">schedule</span>
              <div class="clock-text">
                <span class="clock-title">System clock is out of sync</span>
                <span class="clock-detail">{{ clock.message }}</span>
                <span class="clock-hint">
                  Fix: Windows Settings → Time &amp; language → Date &amp; time →
                  turn on “Set time automatically”, then “Sync now”. Timezone offset: {{ clock.timezone_offset }}.
                </span>
              </div>
              <button class="btn-icon" (click)="checkClock()" [disabled]="checkingClock" title="Re-check">
                <span class="material-icons">refresh</span>
              </button>
            </div>
          }

          <!-- Step 1: GCS Credentials -->
          @if (!credentialsReady) {
            <h2>Activate this machine</h2>
            <p class="subtitle">
              Enter the 10-character code from your email.
            </p>

            @if (checking) {
              <div class="checking-state">
                <span class="spinner"></span>
                <span>Checking for credentials...</span>
              </div>
            } @else {
              <!-- Primary: onboarding code -->
              <div class="field full-width">
                <label>Code</label>
                <input class="input code-input" type="text"
                       [(ngModel)]="code"
                       [disabled]="redeeming"
                       maxlength="10"
                       placeholder="e.g. 7KQ9M2XR4T"
                       (keyup.enter)="redeemCode()">
              </div>

              <button class="btn btn-primary activate-btn"
                      (click)="redeemCode()"
                      [disabled]="!code.trim() || redeeming">
                @if (redeeming) {
                  <span class="spinner"></span>
                  Verifying...
                } @else {
                  <span class="material-icons">vpn_key</span>
                  Continue
                }
              </button>

              <!-- Discreet fallback for edge cases; no jargon in the label -->
              <button class="btn btn-text advanced-toggle" (click)="showAdvanced = !showAdvanced">
                {{ showAdvanced ? 'Hide' : 'Advanced' }}
              </button>

              @if (showAdvanced) {
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
              }

              @if (importing) {
                <div class="checking-state">
                  <span class="spinner"></span>
                  <span>Setting up…</span>
                </div>
              }
            }
          }

          <!-- Step 2: Email Activation -->
          @if (credentialsReady) {
            <div class="creds-saved-banner">
              <span class="material-icons">check_circle</span>
              <span class="banner-title">Authentication complete</span>
              <button class="btn-icon" (click)="resetCredentials()" title="Edit">
                <span class="material-icons">edit</span>
              </button>
            </div>

            <h2>Step 2: Confirm & activate</h2>
            <p class="subtitle">
              Confirm the hospital email and name this machine to activate
            </p>

            @if (hospitalName) {
              <div class="hint-box">
                <span class="material-icons">domain</span>
                <span>Onboarding <strong>{{ hospitalName }}</strong> — make sure the hospital email below is correct before activating.</span>
              </div>
            }

            <div class="field full-width">
              <label>Hospital Email</label>
              <input class="input" type="email"
                     [(ngModel)]="email"
                     [disabled]="activating"
                     placeholder="admin{{'@'}}hospital.com"
                     (keyup.enter)="activate()">
              <span class="field-hint">This email was used during hospital onboarding</span>
            </div>

            @if (machineKnown) {
              <div class="hint-box">
                <span class="material-icons">verified_user</span>
                <span>This machine is already registered as <strong>{{ machineName }}</strong> — re-activating, no new name needed.</span>
              </div>
            } @else {
              <div class="field full-width">
                <label>Machine Name</label>
                <input class="input" type="text"
                       [(ngModel)]="machineName"
                       [disabled]="activating"
                       placeholder="Main Server"
                       (keyup.enter)="activate()">
                <span class="field-hint">A friendly name for this computer (e.g. "Main Server", "Reception PC")</span>
              </div>
            }

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
          <a href="mailto:support{{'@'}}purulabs.com">Contact Support</a>
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
      background: linear-gradient(140deg, #12202e 0%, #0a3a63 52%, #009efb 135%);
    }

    .activation-card {
      width: 100%;
      max-width: 480px;
      padding: 2rem;
      text-align: center;
    }

    .logo {
      margin-bottom: 2rem;
      display: flex;
      flex-direction: column;
      align-items: center;
      gap: 12px;

      .product {
        font-family: var(--font-support);
        font-size: 0.72rem;
        font-weight: 600;
        letter-spacing: 0.42em;
        color: var(--brand-blue);
        padding-left: 0.42em;
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

    /* ── System clock warning ──────────────── */
    .clock-warning {
      display: flex;
      align-items: flex-start;
      gap: 10px;
      padding: 12px 14px;
      background: #fff3e0;
      border: 1px solid #ffcc80;
      border-radius: 8px;
      margin-bottom: 1.25rem;
      text-align: left;

      > .material-icons {
        color: #e65100;
        font-size: 22px;
        width: 22px;
        height: 22px;
        flex-shrink: 0;
        margin-top: 1px;
      }

      .clock-text {
        display: flex;
        flex-direction: column;
        gap: 3px;
        flex: 1;
        min-width: 0;
      }

      .clock-title {
        font-size: 0.85rem;
        font-weight: 700;
        color: #e65100;
      }

      .clock-detail {
        font-size: 0.78rem;
        color: #bf360c;
      }

      .clock-hint {
        font-size: 0.72rem;
        color: #8d6e63;
      }

      .btn-icon { color: #e65100; flex-shrink: 0; }
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
  showAdvanced = false;
  pastedJson = '';
  designatedPath = '';

  // Onboarding code
  code = '';
  redeeming = false;
  machineKnown = false;

  // Step 2 state
  email = '';
  machineName = '';
  fingerprint = '';
  hospitalName = '';
  hospitalCode = '';
  activating = false;
  error: string | null = null;

  // Environment checks
  clock: SystemClockStatus | null = null;
  checkingClock = false;

  constructor() {
    this.checkCredentials();
    this.checkClock();
  }

  /** Verify the system clock is in sync — a skew breaks GCP sign-in. */
  async checkClock(): Promise<void> {
    this.checkingClock = true;
    try {
      this.clock = await this.tauri.invoke<SystemClockStatus>('check_system_clock');
    } catch {
      // Non-fatal — if the check itself fails, don't block activation.
      this.clock = null;
    } finally {
      this.checkingClock = false;
    }
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

  /** Redeem a one-time onboarding code: the backend fetches + stores the key
   *  (encrypted) and returns the hospital email/name for the operator to
   *  confirm before activating. */
  async redeemCode(): Promise<void> {
    const code = this.code.trim().toUpperCase();
    if (!code) return;

    this.error = null;
    this.redeeming = true;
    try {
      const res = await this.tauri.invoke<{
        hospital_email: string;
        hospital_name: string;
        hospital_code: string;
        machine_status: string;
        machine_name: string;
      }>('redeem_onboarding_code', { code });

      this.email = res.hospital_email;
      this.hospitalName = res.hospital_name;
      this.hospitalCode = res.hospital_code;
      // "known" = this machine is already registered → re-activation, no need to
      // name it again. Otherwise it's a new (allowed) machine → ask for a name.
      this.machineKnown = res.machine_status === 'known';
      if (this.machineKnown) {
        this.machineName = res.machine_name || 'This machine';
      }
      this.credentialsReady = true;
      this.loadFingerprint();
      this.notification.success(this.machineKnown ? 'Welcome back — machine recognised' : 'Code accepted');
    } catch (error) {
      this.error = String(error);
    } finally {
      this.redeeming = false;
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
    this.showAdvanced = false;
    this.pastedJson = '';
    this.code = '';
    this.hospitalName = '';
    this.hospitalCode = '';
    this.machineKnown = false;
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
