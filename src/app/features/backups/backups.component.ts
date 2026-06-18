import { Component, OnInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { TauriService, BackupRecord, BackupResult, BinlogStatus, BinlogShipResult } from '../../core/services/tauri.service';
import { NotificationService } from '../../core/services/notification.service';

@Component({
  selector: 'app-backups',
  standalone: true,
  imports: [
    CommonModule
  ],
  template: `
    <div class="backups-page p-4">
      <div class="header flex justify-between items-center">
        <h1>Backups</h1>
        <div class="actions">
          <button class="btn btn-stroked" (click)="runBackup('partial')" [disabled]="backupInProgress">
            <span class="material-icons">backup</span>
            Partial Backup
          </button>
          <button class="btn btn-primary" (click)="runBackup('full')" [disabled]="backupInProgress">
            <span class="material-icons">cloud_upload</span>
            Full Backup
          </button>
        </div>
      </div>

      @if (backupInProgress) {
        <div class="card card-pad backup-progress-card">
          <div class="flex items-center gap-4">
            <span class="spinner"></span>
            <div class="flex-1">
              <strong>Backup in progress...</strong>
              <p>Creating {{ currentBackupType }} backup. Please wait.</p>
            </div>
          </div>
          <div class="pbar pbar-indeterminate"><div class="pbar-fill"></div></div>
        </div>
      }

      @if (lastError) {
        <div class="card card-pad error-card">
          <div class="error-header">
            <span class="material-icons">error</span>
            <strong>Backup Failed</strong>
            <button class="btn-icon" (click)="lastError = null"><span class="material-icons">close</span></button>
          </div>
          <pre class="error-detail">{{ lastError }}</pre>
        </div>
      }

      <div class="backup-stats">
        <div class="card card-pad stat-card">
          <span class="material-icons">backup</span>
          <div class="stat-value">{{ totalBackups }}</div>
          <div class="stat-label">Total Backups</div>
        </div>

        <div class="card card-pad stat-card">
          <span class="material-icons">cloud_done</span>
          <div class="stat-value">{{ uploadedBackups }}</div>
          <div class="stat-label">Uploaded to Cloud</div>
        </div>

        <div class="card card-pad stat-card">
          <span class="material-icons">storage</span>
          <div class="stat-value">{{ totalSizeGB | number:'1.1-1' }} GB</div>
          <div class="stat-label">Total Size</div>
        </div>

        <div class="card card-pad stat-card">
          <span class="material-icons">schedule</span>
          <div class="stat-value">{{ lastBackupTime || 'Never' }}</div>
          <div class="stat-label">Last Backup</div>
        </div>
      </div>

      <div class="card card-pad history-card">
        <h2 class="card-title">Backup History</h2>
        @if (loading) {
          <div class="loading-container">
            <span class="spinner spinner-lg"></span>
          </div>
        } @else if (backups.length === 0) {
          <div class="empty-state">
            <span class="material-icons">backup</span>
            <p>No backups yet</p>
            <button class="btn btn-primary" (click)="runBackup('full')">
              Create First Backup
            </button>
          </div>
        } @else {
          <table class="data-table backups-table">
            <thead>
              <tr>
                <th>Type</th>
                <th>Status</th>
                <th>Size</th>
                <th>Created</th>
                <th>Cloud</th>
                <th>LAN</th>
                <th>Error</th>
                <th>Actions</th>
              </tr>
            </thead>
            <tbody>
              @for (backup of backups; track backup.id) {
                <tr [class.failed-row]="backup.status === 'failed'">
                  <td><span class="chip">{{ backup.type }}</span></td>
                  <td><span class="chip" [class]="'chip chip-' + backup.status">{{ backup.status }}</span></td>
                  <td>{{ backup.size_mb }} MB</td>
                  <td>{{ backup.created_at | date:'short' }}</td>
                  <td>
                    <span class="material-icons" [class]="backup.uploaded ? 'material-icons text-success' : 'material-icons text-muted'">
                      {{ backup.uploaded ? 'cloud_done' : 'cloud_off' }}
                    </span>
                  </td>
                  <td>
                    <span class="material-icons" [class]="backup.lan_copied ? 'material-icons text-success' : 'material-icons text-muted'">
                      {{ backup.lan_copied ? 'lan' : 'lan' }}
                    </span>
                  </td>
                  <td>
                    @if (backup.status === 'failed') {
                      <span class="error-text" [title]="backup.error || 'Unknown error'">
                        {{ backup.error || 'Unknown error' }}
                      </span>
                    }
                  </td>
                  <td>
                    <button class="btn-icon" (click)="restoreBackup(backup)" [disabled]="backup.status !== 'completed'">
                      <span class="material-icons">restore</span>
                    </button>
                  </td>
                </tr>
              }
            </tbody>
          </table>
        }
      </div>

      @if (binlogStatus?.lan_enabled) {
        <div class="card card-pad binlog-card">
          <h2 class="card-title">LAN Binlog Shipping</h2>
          <div class="binlog-info">
            <div class="binlog-stat">
              <span class="label">Last Shipped:</span>
              <span>{{ binlogStatus?.lan_last_shipped_file || 'None' }}</span>
            </div>
            <div class="binlog-stat">
              <span class="label">Total Shipped:</span>
              <span>{{ binlogStatus?.lan_total_shipped }}</span>
            </div>
            <div class="binlog-stat">
              <span class="label">Pending:</span>
              <span>{{ binlogStatus?.files_pending }}</span>
            </div>
            @if (binlogStatus?.lan_last_error) {
              <div class="binlog-stat error">
                <span class="label">Last Error:</span>
                <span>{{ binlogStatus?.lan_last_error }}</span>
              </div>
            }
          </div>
          <button class="btn btn-stroked" (click)="shipBinlogsLanNow()" [disabled]="lanBinlogShipping">
            @if (lanBinlogShipping) {
              <span class="spinner" style="width:14px;height:14px;border-width:2px"></span>
            } @else {
              <span class="material-icons">send</span>
            }
            Ship to LAN Now
          </button>
        </div>
      }
    </div>
  `,
  styles: [`
    .backups-page {
      max-width: 1400px;
      margin: 0 auto;
    }

    h1 {
      margin: 0;
      color: #333;
    }

    .header {
      margin-bottom: 1.5rem;
    }

    .actions {
      display: flex;
      gap: 0.75rem;
    }

    .backup-progress-card {
      margin-bottom: 1.5rem;
      background: #e3f2fd;
      border-left: 4px solid #2196f3;

      .pbar {
        margin-top: 1rem;
      }
    }

    .pbar {
      width: 100%;
      height: 6px;
      background: #e2e8f0;
      border-radius: 3px;
      overflow: hidden;
    }

    .pbar-fill {
      height: 100%;
      background: var(--accent-indigo);
      border-radius: 3px;
    }

    .pbar-indeterminate .pbar-fill {
      width: 40%;
      animation: pbar-indeterminate 1.2s ease-in-out infinite;
    }

    @keyframes pbar-indeterminate {
      0% { margin-left: -40%; }
      100% { margin-left: 100%; }
    }

    .card-title {
      margin: 0 0 1rem;
      font-size: 1.1rem;
      font-weight: 600;
    }

    .backup-stats {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
      gap: 1rem;
      margin-bottom: 1.5rem;
    }

    .stat-card {
      text-align: center;

      .material-icons {
        font-size: 2rem;
        width: 2rem;
        height: 2rem;
        color: #3f51b5;
        margin-bottom: 0.5rem;
      }

      .stat-value {
        font-size: 1.5rem;
        font-weight: 500;
      }

      .stat-label {
        font-size: 0.875rem;
        color: #666;
      }
    }

    .loading-container {
      display: flex;
      justify-content: center;
      padding: 3rem;
    }

    .empty-state {
      display: flex;
      flex-direction: column;
      align-items: center;
      padding: 3rem;
      color: #666;

      .material-icons {
        font-size: 4rem;
        width: 4rem;
        height: 4rem;
        margin-bottom: 1rem;
      }

      p {
        margin-bottom: 1rem;
      }
    }

    .backups-table {
      width: 100%;
    }

    .chip-completed {
      background: #e8f5e9 !important;
      color: #2e7d32 !important;
    }

    .chip-failed {
      background: #ffebee !important;
      color: #c62828 !important;
    }

    .chip-in_progress {
      background: #fff3e0 !important;
      color: #ef6c00 !important;
    }

    .text-success {
      color: #4caf50;
    }

    .text-muted {
      color: #999;
    }

    .binlog-card {
      margin-top: 1.5rem;
    }

    .binlog-info {
      display: flex;
      flex-wrap: wrap;
      gap: 1.5rem;
      margin-bottom: 1rem;
    }

    .binlog-stat {
      .label {
        color: #666;
        margin-right: 0.5rem;
      }

      &.error {
        color: #f44336;
        width: 100%;
      }
    }

    .error-card {
      margin-bottom: 1.5rem;
      background: #ffebee;
      border-left: 4px solid #f44336;
    }

    .error-header {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      margin-bottom: 0.5rem;
      color: #c62828;

      .material-icons { font-size: 20px; width: 20px; height: 20px; }
      button { margin-left: auto; }
    }

    .error-detail {
      background: #fff;
      border: 1px solid #ffcdd2;
      border-radius: 6px;
      padding: 0.75rem 1rem;
      font-size: 0.8rem;
      color: #333;
      white-space: pre-wrap;
      word-break: break-word;
      max-height: 200px;
      overflow-y: auto;
      margin: 0;
    }

    .error-text {
      color: #c62828;
      font-size: 0.8rem;
      max-width: 200px;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      display: inline-block;
      cursor: help;
    }

    .failed-row {
      background: #fff8f8;
    }
  `]
})
export class BackupsComponent implements OnInit {
  private tauri = inject(TauriService);
  private notification = inject(NotificationService);

  backups: BackupRecord[] = [];
  loading = true;
  backupInProgress = false;
  currentBackupType: 'full' | 'partial' = 'full';
  lastError: string | null = null;

  displayedColumns = ['type', 'status', 'size', 'created_at', 'uploaded', 'lan', 'error', 'actions'];

  binlogStatus: BinlogStatus | null = null;
  lanBinlogShipping = false;

  get totalBackups(): number {
    return this.backups.filter(b => b.status === 'completed').length;
  }

  get uploadedBackups(): number {
    return this.backups.filter(b => b.uploaded).length;
  }

  get totalSizeGB(): number {
    return this.backups.reduce((sum, b) => sum + b.size_mb, 0) / 1024;
  }

  get lastBackupTime(): string | null {
    const completed = this.backups.filter(b => b.status === 'completed');
    if (completed.length === 0) return null;

    const latest = completed.reduce((a, b) =>
      new Date(a.created_at) > new Date(b.created_at) ? a : b
    );
    return new Date(latest.created_at).toLocaleString();
  }

  ngOnInit(): void {
    this.loadBackups();
    this.loadBinlogStatus();
  }

  async loadBackups(): Promise<void> {
    this.loading = true;
    try {
      this.backups = await this.tauri.invoke<BackupRecord[]>('get_backup_history');
    } finally {
      this.loading = false;
    }
  }

  async runBackup(type: 'full' | 'partial'): Promise<void> {
    this.backupInProgress = true;
    this.currentBackupType = type;
    this.lastError = null;

    try {
      const result = await this.tauri.invoke<BackupResult>('start_backup', { type });
      if (result.success) {
        this.notification.success(
          `Backup completed: ${result.size_mb} MB in ${result.duration_seconds}s`
        );
      }
    } catch (error) {
      this.lastError = String(error);
      this.notification.error('Backup failed');
    } finally {
      this.backupInProgress = false;
      await this.loadBackups();
    }
  }

  async loadBinlogStatus(): Promise<void> {
    try {
      this.binlogStatus = await this.tauri.invokeSilent<BinlogStatus>('get_binlog_status');
    } catch {
      this.binlogStatus = null;
    }
  }

  async shipBinlogsLanNow(): Promise<void> {
    this.lanBinlogShipping = true;
    try {
      const result = await this.tauri.invoke<BinlogShipResult>('ship_binlogs_lan');
      this.notification.success(
        `Shipped ${result.files_shipped} binlog files to LAN`
      );
      await this.loadBinlogStatus();
    } catch (error) {
      // Error handled by TauriService
    } finally {
      this.lanBinlogShipping = false;
    }
  }

  async restoreBackup(backup: BackupRecord): Promise<void> {
    if (confirm('Are you sure you want to restore this backup? This will replace current data.')) {
      try {
        await this.tauri.invoke('restore_backup', { backupId: backup.id });
        this.notification.success('Backup restored successfully');
      } catch (error) {
        // Error handled by TauriService
      }
    }
  }
}
