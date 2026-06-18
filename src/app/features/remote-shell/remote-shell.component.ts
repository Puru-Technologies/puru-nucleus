import { Component, OnInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { TauriService, ShellResult, ShellAuditEntry } from '../../core/services/tauri.service';

@Component({
  selector: 'app-remote-shell',
  standalone: true,
  imports: [
    CommonModule,
    FormsModule
  ],
  template: `
    <div class="shell-page p-4">
      <div class="header">
        <h1>Remote Shell</h1>
        <span class="subtitle">Execute diagnostic commands on the host</span>
      </div>

      <!-- Command Input -->
      <div class="card card-pad command-card">
        <div class="command-input-row">
          <div class="field command-field">
            <label>Command</label>
            <input class="input mono-input"
                   [(ngModel)]="command"
                   (keydown.enter)="executeCommand()"
                   placeholder="e.g. docker ps"
                   spellcheck="false">
          </div>
          <button class="btn btn-primary execute-btn"
                  (click)="executeCommand()"
                  [disabled]="executing || !command.trim()">
            @if (executing) {
              <span class="spinner"></span>
            } @else {
              <span class="material-icons">play_arrow</span>
            }
            Execute
          </button>
        </div>

        <div class="quick-commands">
          <span class="label">Quick:</span>
          @for (cmd of quickCommands; track cmd) {
            <button class="btn btn-stroked btn-sm quick-chip" (click)="runQuickCommand(cmd)">
              {{ cmd }}
            </button>
          }
        </div>
      </div>

      <!-- Output -->
      @if (lastResult) {
        <div class="card card-pad output-card">
          <div class="output-title">
            <code>{{ lastResult.command }}</code>
            <span class="exit-code" [class.success]="lastResult.exit_code === 0" [class.error]="lastResult.exit_code !== 0">
              exit {{ lastResult.exit_code }}
            </span>
            <span class="duration">{{ lastResult.duration_ms }}ms</span>
          </div>
          <div class="output-body">
            @if (lastResult.stdout) {
              <pre class="output-pre">{{ lastResult.stdout }}</pre>
            }
            @if (lastResult.stderr) {
              <div class="stderr-section">
                <span class="stderr-label">stderr:</span>
                <pre class="output-pre stderr">{{ lastResult.stderr }}</pre>
              </div>
            }
            @if (!lastResult.stdout && !lastResult.stderr) {
              <div class="empty-output">No output</div>
            }
          </div>
        </div>
      }

      <!-- Validation Error -->
      @if (validationError) {
        <div class="card card-pad error-card">
          <div class="error-content">
            <span class="material-icons">block</span>
            <span>{{ validationError }}</span>
          </div>
        </div>
      }

      <!-- Allowed Commands -->
      <div class="card allowed-panel">
        <button type="button" class="allowed-header" (click)="allowedOpen = !allowedOpen">
          <span class="panel-title">
            <span class="material-icons">info_outline</span>
            Allowed Commands
          </span>
          <span class="panel-description">
            {{ allowedCommands.length }} commands available
          </span>
          <span class="material-icons chevron" [class.open]="allowedOpen">expand_more</span>
        </button>
        @if (allowedOpen) {
          <div class="allowed-list">
            @for (cmd of allowedCommands; track cmd) {
              <code class="allowed-chip" (click)="setCommand(cmd)">{{ cmd }}</code>
            }
          </div>
        }
      </div>

      <!-- Audit Log -->
      <h2>Audit Log</h2>

      @if (auditLoading) {
        <div class="loading-container">
          <span class="spinner"></span>
          <span>Loading audit log...</span>
        </div>
      } @else if (auditLog.length === 0) {
        <div class="card card-pad empty-state">
          <span class="material-icons">history</span>
          <p>No commands executed yet</p>
          <span>Command execution history will appear here.</span>
        </div>
      } @else {
        <div class="audit-table-wrapper">
          <table class="data-table audit-table">
            <thead>
              <tr>
                <th>Time</th>
                <th>Command</th>
                <th>Exit</th>
                <th>Duration</th>
              </tr>
            </thead>
            <tbody>
              @for (entry of auditLog; track entry) {
                <tr class="audit-row" (click)="setCommand(entry.command)">
                  <td>{{ entry.timestamp | date:'short' }}</td>
                  <td>
                    <code class="audit-cmd">{{ entry.command }}</code>
                  </td>
                  <td>
                    <span [class.exit-success]="entry.exit_code === 0" [class.exit-error]="entry.exit_code !== 0">
                      {{ entry.exit_code }}
                    </span>
                  </td>
                  <td>{{ entry.duration_ms }}ms</td>
                </tr>
              }
            </tbody>
          </table>
        </div>
      }
    </div>
  `,
  styles: [`
    .shell-page {
      max-width: 1000px;
      margin: 0 auto;
    }

    .header {
      margin-bottom: 1.5rem;

      h1 {
        margin: 0;
        color: #333;
      }

      .subtitle {
        font-size: 0.875rem;
        color: #666;
      }
    }

    h2 {
      margin: 2rem 0 1rem;
      color: #333;
      font-size: 1.25rem;
    }

    /* Command input */
    .command-card {
      margin-bottom: 1rem;
    }

    .command-input-row {
      display: flex;
      gap: 1rem;
      align-items: flex-end;
    }

    .command-field {
      flex: 1;
      margin-bottom: 0;
    }

    .execute-btn {
      height: 38px;
    }

    .mono-input {
      font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace !important;
    }

    .quick-commands {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      flex-wrap: wrap;
      margin-top: 0.75rem;

      .label {
        font-size: 0.75rem;
        color: #999;
        text-transform: uppercase;
        font-weight: 600;
      }
    }

    .quick-chip {
      font-family: 'SF Mono', 'Fira Code', monospace !important;
    }

    /* Output */
    .output-card {
      margin-bottom: 1rem;
      background: #0f172a !important;
      color: #e2e8f0 !important;
      border-color: #1e293b !important;
    }

    .output-title {
      display: flex;
      align-items: center;
      gap: 1rem;
      flex-wrap: wrap;
      margin-bottom: 0.75rem;

      code {
        font-family: 'SF Mono', 'Fira Code', monospace;
        font-size: 0.875rem;
        color: #93c5fd;
      }
    }

    .exit-code {
      font-size: 0.75rem;
      padding: 2px 8px;
      border-radius: 4px;
      font-weight: 600;

      &.success {
        background: rgba(34, 197, 94, 0.2);
        color: #4ade80;
      }

      &.error {
        background: rgba(239, 68, 68, 0.2);
        color: #f87171;
      }
    }

    .duration {
      font-size: 0.75rem;
      color: #64748b;
    }

    .output-pre {
      font-family: 'SF Mono', 'Fira Code', monospace;
      font-size: 0.8rem;
      line-height: 1.5;
      margin: 0;
      white-space: pre-wrap;
      word-break: break-all;
      color: #cbd5e1;
      max-height: 400px;
      overflow-y: auto;
    }

    .stderr-section {
      margin-top: 0.75rem;
      padding-top: 0.75rem;
      border-top: 1px solid rgba(255, 255, 255, 0.1);
    }

    .stderr-label {
      font-size: 0.7rem;
      color: #f87171;
      text-transform: uppercase;
      font-weight: 600;
      letter-spacing: 0.05em;
    }

    .output-pre.stderr {
      color: #fca5a5;
    }

    .empty-output {
      color: #64748b;
      font-style: italic;
      padding: 0.5rem 0;
    }

    /* Error */
    .error-card {
      margin-bottom: 1rem;

      .error-content {
        display: flex;
        align-items: center;
        gap: 0.75rem;
        color: #f44336;
      }
    }

    /* Allowed commands */
    .allowed-panel {
      margin-bottom: 1rem;
      overflow: hidden;
    }

    .allowed-header {
      display: flex;
      align-items: center;
      gap: 1rem;
      width: 100%;
      padding: 14px 16px;
      background: transparent;
      border: none;
      cursor: pointer;
      text-align: left;
      font: inherit;
      color: var(--text-primary);

      .panel-title {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        font-weight: 600;

        .material-icons {
          font-size: 20px;
        }
      }

      .panel-description {
        color: #666;
        font-size: 0.875rem;
        margin-left: auto;
      }

      .chevron {
        transition: transform 0.2s;
        &.open { transform: rotate(180deg); }
      }
    }

    .allowed-list {
      display: flex;
      flex-wrap: wrap;
      gap: 0.5rem;
      padding: 0 16px 16px;
    }

    .allowed-chip {
      font-family: 'SF Mono', 'Fira Code', monospace;
      font-size: 0.8rem;
      background: #f1f5f9;
      padding: 4px 10px;
      border-radius: 4px;
      cursor: pointer;
      transition: background 0.15s;

      &:hover {
        background: #e2e8f0;
      }
    }

    /* Audit log */
    .loading-container {
      display: flex;
      align-items: center;
      justify-content: center;
      gap: 1rem;
      padding: 2rem;
      color: #666;
    }

    .empty-state {
      text-align: center;
      padding: 2rem;
      color: #666;

      .material-icons {
        font-size: 3rem;
        width: 3rem;
        height: 3rem;
        color: #bbb;
      }

      p {
        font-size: 1.1rem;
        margin: 0.75rem 0 0.25rem;
        color: #444;
      }

      span {
        font-size: 0.875rem;
      }
    }

    .audit-table-wrapper {
      overflow-x: auto;
    }

    .audit-table {
      width: 100%;
    }

    .audit-cmd {
      font-family: 'SF Mono', 'Fira Code', monospace;
      font-size: 0.8rem;
      cursor: pointer;

      &:hover {
        color: #6366f1;
      }
    }

    .audit-row {
      cursor: pointer;

      &:hover {
        background: #f8fafc;
      }
    }

    .exit-success {
      color: #4caf50;
      font-weight: 600;
    }

    .exit-error {
      color: #f44336;
      font-weight: 600;
    }
  `]
})
export class RemoteShellComponent implements OnInit {
  private tauri = inject(TauriService);

  command = '';
  executing = false;
  lastResult: ShellResult | null = null;
  validationError: string | null = null;

  allowedCommands: string[] = [];
  auditLog: ShellAuditEntry[] = [];
  auditLoading = false;
  auditColumns = ['timestamp', 'command', 'exit_code', 'duration_ms'];

  allowedOpen = false;

  quickCommands = [
    'docker ps',
    'docker stats --no-stream',
    'df -h',
    'free -m',
    'uptime',
    'top -bn1'
  ];

  ngOnInit(): void {
    this.loadAllowedCommands();
    this.loadAuditLog();
  }

  async executeCommand(): Promise<void> {
    if (!this.command.trim()) return;

    this.executing = true;
    this.validationError = null;
    this.lastResult = null;

    try {
      this.lastResult = await this.tauri.invokeSilent<ShellResult>(
        'execute_shell_command',
        { command: this.command.trim() }
      );
      await this.loadAuditLog();
    } catch (error) {
      this.validationError = error as string;
    } finally {
      this.executing = false;
    }
  }

  runQuickCommand(cmd: string): void {
    this.command = cmd;
    this.executeCommand();
  }

  setCommand(cmd: string): void {
    this.command = cmd;
  }

  private async loadAllowedCommands(): Promise<void> {
    try {
      this.allowedCommands = await this.tauri.invokeSilent<string[]>('get_allowed_shell_commands');
    } catch {
      // Non-fatal
    }
  }

  async loadAuditLog(): Promise<void> {
    this.auditLoading = true;
    try {
      this.auditLog = await this.tauri.invokeSilent<ShellAuditEntry[]>('get_shell_audit_log');
    } catch {
      // Non-fatal
    } finally {
      this.auditLoading = false;
    }
  }
}
