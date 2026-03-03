import { Component, OnInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { MatCardModule } from '@angular/material/card';
import { MatIconModule } from '@angular/material/icon';
import { MatButtonModule } from '@angular/material/button';
import { MatInputModule } from '@angular/material/input';
import { MatFormFieldModule } from '@angular/material/form-field';
import { MatChipsModule } from '@angular/material/chips';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
import { MatExpansionModule } from '@angular/material/expansion';
import { MatTableModule } from '@angular/material/table';
import { TauriService, ShellResult, ShellAuditEntry } from '../../core/services/tauri.service';

@Component({
  selector: 'app-remote-shell',
  standalone: true,
  imports: [
    CommonModule,
    FormsModule,
    MatCardModule,
    MatIconModule,
    MatButtonModule,
    MatInputModule,
    MatFormFieldModule,
    MatChipsModule,
    MatProgressSpinnerModule,
    MatExpansionModule,
    MatTableModule
  ],
  template: `
    <div class="shell-page p-4">
      <div class="header">
        <h1>Remote Shell</h1>
        <span class="subtitle">Execute diagnostic commands on the host</span>
      </div>

      <!-- Command Input -->
      <mat-card class="command-card">
        <mat-card-content>
          <div class="command-input-row">
            <mat-form-field appearance="outline" class="command-field">
              <mat-label>Command</mat-label>
              <input matInput
                     [(ngModel)]="command"
                     (keydown.enter)="executeCommand()"
                     placeholder="e.g. docker ps"
                     spellcheck="false"
                     class="mono-input">
            </mat-form-field>
            <button mat-raised-button color="primary"
                    (click)="executeCommand()"
                    [disabled]="executing || !command.trim()">
              @if (executing) {
                <mat-spinner diameter="18"></mat-spinner>
              } @else {
                <mat-icon>play_arrow</mat-icon>
              }
              Execute
            </button>
          </div>

          <div class="quick-commands">
            <span class="label">Quick:</span>
            @for (cmd of quickCommands; track cmd) {
              <button mat-stroked-button class="quick-chip" (click)="runQuickCommand(cmd)">
                {{ cmd }}
              </button>
            }
          </div>
        </mat-card-content>
      </mat-card>

      <!-- Output -->
      @if (lastResult) {
        <mat-card class="output-card">
          <mat-card-header>
            <mat-card-title class="output-title">
              <code>{{ lastResult.command }}</code>
              <span class="exit-code" [class.success]="lastResult.exit_code === 0" [class.error]="lastResult.exit_code !== 0">
                exit {{ lastResult.exit_code }}
              </span>
              <span class="duration">{{ lastResult.duration_ms }}ms</span>
            </mat-card-title>
          </mat-card-header>
          <mat-card-content>
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
          </mat-card-content>
        </mat-card>
      }

      <!-- Validation Error -->
      @if (validationError) {
        <mat-card class="error-card">
          <mat-card-content>
            <mat-icon>block</mat-icon>
            <span>{{ validationError }}</span>
          </mat-card-content>
        </mat-card>
      }

      <!-- Allowed Commands -->
      <mat-expansion-panel class="allowed-panel">
        <mat-expansion-panel-header>
          <mat-panel-title>
            <mat-icon>info_outline</mat-icon>
            Allowed Commands
          </mat-panel-title>
          <mat-panel-description>
            {{ allowedCommands.length }} commands available
          </mat-panel-description>
        </mat-expansion-panel-header>
        <div class="allowed-list">
          @for (cmd of allowedCommands; track cmd) {
            <code class="allowed-chip" (click)="setCommand(cmd)">{{ cmd }}</code>
          }
        </div>
      </mat-expansion-panel>

      <!-- Audit Log -->
      <h2>Audit Log</h2>

      @if (auditLoading) {
        <div class="loading-container">
          <mat-spinner diameter="24"></mat-spinner>
          <span>Loading audit log...</span>
        </div>
      } @else if (auditLog.length === 0) {
        <mat-card class="empty-state">
          <mat-card-content>
            <mat-icon>history</mat-icon>
            <p>No commands executed yet</p>
            <span>Command execution history will appear here.</span>
          </mat-card-content>
        </mat-card>
      } @else {
        <div class="audit-table-wrapper">
          <table mat-table [dataSource]="auditLog" class="audit-table">
            <ng-container matColumnDef="timestamp">
              <th mat-header-cell *matHeaderCellDef>Time</th>
              <td mat-cell *matCellDef="let entry">{{ entry.timestamp | date:'short' }}</td>
            </ng-container>

            <ng-container matColumnDef="command">
              <th mat-header-cell *matHeaderCellDef>Command</th>
              <td mat-cell *matCellDef="let entry">
                <code class="audit-cmd" (click)="setCommand(entry.command)">{{ entry.command }}</code>
              </td>
            </ng-container>

            <ng-container matColumnDef="exit_code">
              <th mat-header-cell *matHeaderCellDef>Exit</th>
              <td mat-cell *matCellDef="let entry">
                <span [class.exit-success]="entry.exit_code === 0" [class.exit-error]="entry.exit_code !== 0">
                  {{ entry.exit_code }}
                </span>
              </td>
            </ng-container>

            <ng-container matColumnDef="duration_ms">
              <th mat-header-cell *matHeaderCellDef>Duration</th>
              <td mat-cell *matCellDef="let entry">{{ entry.duration_ms }}ms</td>
            </ng-container>

            <tr mat-header-row *matHeaderRowDef="auditColumns"></tr>
            <tr mat-row *matRowDef="let row; columns: auditColumns;" class="audit-row" (click)="setCommand(row.command)"></tr>
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
      align-items: flex-start;
    }

    .command-field {
      flex: 1;
    }

    .mono-input {
      font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace !important;
    }

    .quick-commands {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      flex-wrap: wrap;
      margin-top: 0.5rem;

      .label {
        font-size: 0.75rem;
        color: #999;
        text-transform: uppercase;
        font-weight: 600;
      }
    }

    .quick-chip {
      font-size: 0.75rem !important;
      font-family: 'SF Mono', 'Fira Code', monospace !important;
      min-height: 28px !important;
      line-height: 28px !important;
      padding: 0 10px !important;
    }

    /* Output */
    .output-card {
      margin-bottom: 1rem;
      background: #0f172a !important;
      color: #e2e8f0 !important;
    }

    .output-title {
      display: flex;
      align-items: center;
      gap: 1rem;
      flex-wrap: wrap;

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

      mat-card-content {
        display: flex;
        align-items: center;
        gap: 0.75rem;
        color: #f44336;
      }
    }

    /* Allowed commands */
    .allowed-panel {
      margin-bottom: 1rem;

      mat-panel-title {
        display: flex;
        align-items: center;
        gap: 0.5rem;

        mat-icon {
          font-size: 20px;
          width: 20px;
          height: 20px;
        }
      }
    }

    .allowed-list {
      display: flex;
      flex-wrap: wrap;
      gap: 0.5rem;
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

      mat-icon {
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
