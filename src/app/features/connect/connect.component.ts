import { Component, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import { ConnectionService, SavedServer } from '../../core/services/connection.service';
import { resetLicenseCache } from '../../core/guards/init.guard';

@Component({
  selector: 'app-connect',
  standalone: true,
  imports: [
    CommonModule,
    FormsModule,
  ],
  template: `
    <div class="connect-page">
      <div class="card card-pad connect-card">
        <div class="card-head">
          <span class="material-icons avatar">lan</span>
          <div class="card-titles">
            <div class="card-title">Connect to a Nucleus server</div>
            <div class="card-subtitle">Manage a remote daemon over the LAN, or stay on this machine</div>
          </div>
        </div>

        <div class="card-body">
          <!-- Current mode -->
          <div class="current-mode" [class.remote]="conn.isRemote()">
            <span class="material-icons">{{ conn.isRemote() ? 'cloud' : 'computer' }}</span>
            <span>
              Currently:
              <strong>{{ conn.isRemote() ? ('Remote · ' + (conn.activeServer()?.name || '')) : 'Local (this machine)' }}</strong>
            </span>
            @if (conn.isRemote()) {
              <button class="btn btn-stroked" (click)="useLocal()">Switch to Local</button>
            }
          </div>

          <!-- Saved servers -->
          @if (conn.servers().length > 0) {
            <div class="servers">
              <div class="servers-head">Saved servers</div>
              @for (s of conn.servers(); track s.id) {
                <div class="server-row" [class.active]="s.id === conn.activeServerId() && conn.isRemote()">
                  <div class="server-info">
                    <span class="server-name">{{ s.name }}</span>
                    <span class="server-addr">{{ s.host }}:{{ s.port }}</span>
                    @if (!s.apiKey) {
                      <span class="no-key"><span class="material-icons">key_off</span> no key</span>
                    }
                  </div>
                  <div class="server-actions">
                    <button class="btn btn-stroked btn-sm" (click)="connect(s)" [disabled]="!s.apiKey">Connect</button>
                    <button class="btn-icon" (click)="edit(s)" title="Edit"><span class="material-icons">edit</span></button>
                    <button class="btn-icon" (click)="remove(s)" title="Remove"><span class="material-icons">delete</span></button>
                  </div>
                </div>
              }
            </div>
          }

          <!-- Add / edit form -->
          <div class="add-form">
            <div class="add-head">{{ editingId ? 'Edit server' : 'Add a server' }}</div>
            <div class="form-row">
              <div class="field flex-2">
                <label>Name</label>
                <input class="input" [(ngModel)]="form.name" placeholder="Bargad Hospital">
              </div>
            </div>
            <div class="form-row">
              <div class="field flex-2">
                <label>Host / IP</label>
                <input class="input" [(ngModel)]="form.host" placeholder="192.168.1.100">
              </div>
              <div class="field flex-1">
                <label>Port</label>
                <input class="input" type="number" [(ngModel)]="form.port" placeholder="9090">
              </div>
            </div>
            <div class="field full-width">
              <label>API Key</label>
              <div class="key-input">
                <input class="input" [type]="showKey ? 'text' : 'password'" [(ngModel)]="form.apiKey"
                       placeholder="Required to control a remote server">
                <button class="btn-icon" (click)="showKey = !showKey">
                  <span class="material-icons">{{ showKey ? 'visibility_off' : 'visibility' }}</span>
                </button>
              </div>
              <span class="hint">Find it in the server's Settings → Daemon → API Key</span>
            </div>

            <!-- Unencrypted warning -->
            <div class="warn">
              <span class="material-icons">warning</span>
              <span>Connections are <strong>unencrypted HTTP</strong> — only use on a trusted hospital LAN. An API key is required for anything beyond a health check.</span>
            </div>

            @if (testResult) {
              <div class="test-ok">
                <span class="material-icons">check_circle</span>
                <span>Reachable — v{{ testResult.version }}, Docker {{ testResult.docker_connected ? 'connected' : 'not connected' }}</span>
              </div>
            }
            @if (testError) {
              <div class="test-err">
                <span class="material-icons">error</span>
                <span>{{ testError }}</span>
              </div>
            }
          </div>
        </div>

        <div class="actions">
          <button class="btn btn-stroked" (click)="useLocal()">Use Local Mode</button>
          <span class="spacer"></span>
          <button class="btn btn-ghost" (click)="test()" [disabled]="!canSubmit() || testing">
            @if (testing) { <span class="spinner" style="width:14px;height:14px;border-width:2px"></span> }
            Test
          </button>
          <button class="btn btn-primary" (click)="saveAndConnect()" [disabled]="!canSubmit()">
            {{ editingId ? 'Save & Connect' : 'Add & Connect' }}
          </button>
        </div>
      </div>
    </div>
  `,
  styles: [`
    .connect-page {
      display: flex;
      justify-content: center;
      align-items: flex-start;
      min-height: 100vh;
      padding: 2rem;
      background: #f5f5f5;
    }
    .connect-card { max-width: 640px; width: 100%; }

    .card-head { display: flex; align-items: center; gap: 14px; }
    .avatar {
      background: #e8eaf6; padding: 8px; border-radius: 50%; color: #3f51b5;
      font-size: 24px; width: 24px; height: 24px;
      display: inline-flex; align-items: center; justify-content: center; box-sizing: content-box;
    }
    .card-titles { display: flex; flex-direction: column; gap: 2px; }
    .card-title { font-size: 1.1rem; font-weight: 600; }
    .card-subtitle { font-size: 0.85rem; color: #666; }
    .card-body { margin-top: 0.5rem; }

    .current-mode {
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 12px 14px;
      background: #eef2ff;
      border-radius: 8px;
      margin: 1rem 0 1.5rem;
      font-size: 0.875rem;
      &.remote { background: #ecfdf5; }
      .material-icons { color: #6366f1; }
      button { margin-left: auto; }
    }

    .servers { margin-bottom: 1.5rem; }
    .servers-head, .add-head {
      font-size: 0.7rem; text-transform: uppercase; letter-spacing: 0.5px;
      color: #999; margin-bottom: 0.5rem; font-weight: 600;
    }
    .server-row {
      display: flex; align-items: center; gap: 10px;
      padding: 10px 12px; border: 1px solid #e0e0e0; border-radius: 8px; margin-bottom: 8px;
      &.active { border-color: #6366f1; background: #f5f3ff; }
    }
    .server-info { flex: 1; display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
    .server-name { font-weight: 500; }
    .server-addr { font-family: monospace; font-size: 0.8rem; color: #666; }
    .no-key { display: inline-flex; align-items: center; gap: 3px; font-size: 0.7rem; color: #e65100;
      .material-icons { font-size: 14px; width: 14px; height: 14px; } }
    .server-actions { display: flex; align-items: center; gap: 4px; }

    .add-form { border-top: 1px solid #eee; padding-top: 1rem; }
    .form-row { display: flex; gap: 1rem; }
    .flex-1 { flex: 1; }
    .flex-2 { flex: 2; }
    .full-width { width: 100%; }
    .key-input { display: flex; align-items: center; gap: 4px; }
    .key-input .input { flex: 1; }

    .warn, .test-ok, .test-err {
      display: flex; align-items: flex-start; gap: 8px;
      padding: 10px 12px; border-radius: 8px; font-size: 0.8rem; margin-top: 0.5rem;
      .material-icons { font-size: 18px; width: 18px; height: 18px; flex-shrink: 0; margin-top: 1px; }
    }
    .warn { background: #fff8e1; color: #8d6e00; .material-icons { color: #f9a825; } }
    .test-ok { background: #e8f5e9; color: #2e7d32; .material-icons { color: #4caf50; } }
    .test-err { background: #ffebee; color: #c62828; .material-icons { color: #f44336; } }

    .actions { display: flex; align-items: center; gap: 8px; padding: 0.5rem 0 0; margin-top: 1rem; }
    .spacer { flex: 1; }
  `]
})
export class ConnectComponent {
  conn = inject(ConnectionService);
  private router = inject(Router);

  showKey = false;
  testing = false;
  editingId: string | null = null;
  testResult: { version: string; docker_connected: boolean } | null = null;
  testError: string | null = null;

  form: { name: string; host: string; port: number; apiKey: string } = {
    name: '',
    host: '',
    port: 9090,
    apiKey: '',
  };

  canSubmit(): boolean {
    return !!this.form.name.trim() && !!this.form.host.trim() && !!this.form.port;
  }

  private toServer(): SavedServer {
    return {
      id: this.editingId ?? this.conn.newId(),
      name: this.form.name.trim(),
      host: this.form.host.trim(),
      port: Number(this.form.port) || 9090,
      apiKey: this.form.apiKey.trim(),
    };
  }

  async test(): Promise<void> {
    this.testing = true;
    this.testResult = null;
    this.testError = null;
    try {
      const h = await this.conn.testConnection(this.toServer());
      this.testResult = { version: h.version, docker_connected: h.docker_connected };
    } catch (e: any) {
      this.testError = `Could not reach ${this.form.host}:${this.form.port} — ${e?.message || e}`;
    } finally {
      this.testing = false;
    }
  }

  saveAndConnect(): void {
    const server = this.toServer();
    this.conn.upsertServer(server);
    if (!server.apiKey) {
      // Saved, but remote mode requires a key — keep the form open with a hint.
      this.testError = 'Saved. Add an API key to connect (remote control requires one).';
      this.editingId = server.id;
      return;
    }
    this.connect(server);
  }

  connect(server: SavedServer): void {
    this.conn.connect(server.id);
    resetLicenseCache();
    this.router.navigate(['/dashboard']);
  }

  edit(server: SavedServer): void {
    this.editingId = server.id;
    this.form = { name: server.name, host: server.host, port: server.port, apiKey: server.apiKey };
    this.testResult = null;
    this.testError = null;
  }

  remove(server: SavedServer): void {
    if (!confirm(`Remove server "${server.name}"?`)) return;
    this.conn.removeServer(server.id);
    if (this.editingId === server.id) this.resetForm();
  }

  useLocal(): void {
    this.conn.goLocal();
    resetLicenseCache();
    this.router.navigate(['/dashboard']);
  }

  private resetForm(): void {
    this.editingId = null;
    this.form = { name: '', host: '', port: 9090, apiKey: '' };
    this.testResult = null;
    this.testError = null;
  }
}
