import { Component, OnInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { MatCardModule } from '@angular/material/card';
import { MatIconModule } from '@angular/material/icon';
import { MatButtonModule } from '@angular/material/button';
import { MatTableModule } from '@angular/material/table';
import { MatChipsModule } from '@angular/material/chips';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { TauriService, BackupRecord, BackupResult, BinlogStatus, BinlogShipResult } from '../../core/services/tauri.service';
import { NotificationService } from '../../core/services/notification.service';

@Component({
  selector: 'app-backups',
  standalone: true,
  imports: [
    CommonModule,
    MatCardModule,
    MatIconModule,
    MatButtonModule,
    MatTableModule,
    MatChipsModule,
    MatProgressSpinnerModule,
    MatProgressBarModule
  ],
  template: `
    <div class="backups-page p-4">
      <div class="header flex justify-between items-center">
        <h1>Backups</h1>
        <div class="actions">
          <button mat-stroked-button (click)="runBackup('partial')" [disabled]="backupInProgress">
            <mat-icon>backup</mat-icon>
            Partial Backup
          </button>
          <button mat-raised-button color="primary" (click)="runBackup('full')" [disabled]="backupInProgress">
            <mat-icon>cloud_upload</mat-icon>
            Full Backup
          </button>
        </div>
      </div>

      @if (backupInProgress) {
        <mat-card class="backup-progress-card">
          <mat-card-content>
            <div class="flex items-center gap-4">
              <mat-spinner diameter="32"></mat-spinner>
              <div class="flex-1">
                <strong>Backup in progress...</strong>
                <p>Creating {{ currentBackupType }} backup. Please wait.</p>
              </div>
            </div>
            <mat-progress-bar mode="indeterminate"></mat-progress-bar>
          </mat-card-content>
        </mat-card>
      }

      <div class="backup-stats">
        <mat-card class="stat-card">
          <mat-card-content>
            <mat-icon>backup</mat-icon>
            <div class="stat-value">{{ totalBackups }}</div>
            <div class="stat-label">Total Backups</div>
          </mat-card-content>
        </mat-card>

        <mat-card class="stat-card">
          <mat-card-content>
            <mat-icon>cloud_done</mat-icon>
            <div class="stat-value">{{ uploadedBackups }}</div>
            <div class="stat-label">Uploaded to Cloud</div>
          </mat-card-content>
        </mat-card>

        <mat-card class="stat-card">
          <mat-card-content>
            <mat-icon>storage</mat-icon>
            <div class="stat-value">{{ totalSizeGB | number:'1.1-1' }} GB</div>
            <div class="stat-label">Total Size</div>
          </mat-card-content>
        </mat-card>

        <mat-card class="stat-card">
          <mat-card-content>
            <mat-icon>schedule</mat-icon>
            <div class="stat-value">{{ lastBackupTime || 'Never' }}</div>
            <div class="stat-label">Last Backup</div>
          </mat-card-content>
        </mat-card>
      </div>

      <mat-card class="history-card">
        <mat-card-header>
          <mat-card-title>Backup History</mat-card-title>
        </mat-card-header>
        <mat-card-content>
          @if (loading) {
            <div class="loading-container">
              <mat-spinner diameter="48"></mat-spinner>
            </div>
          } @else if (backups.length === 0) {
            <div class="empty-state">
              <mat-icon>backup</mat-icon>
              <p>No backups yet</p>
              <button mat-raised-button color="primary" (click)="runBackup('full')">
                Create First Backup
              </button>
            </div>
          } @else {
            <table mat-table [dataSource]="backups" class="backups-table">
              <ng-container matColumnDef="type">
                <th mat-header-cell *matHeaderCellDef>Type</th>
                <td mat-cell *matCellDef="let backup">
                  <mat-chip>{{ backup.type }}</mat-chip>
                </td>
              </ng-container>

              <ng-container matColumnDef="status">
                <th mat-header-cell *matHeaderCellDef>Status</th>
                <td mat-cell *matCellDef="let backup">
                  <mat-chip [class]="'chip-' + backup.status">
                    {{ backup.status }}
                  </mat-chip>
                </td>
              </ng-container>

              <ng-container matColumnDef="size">
                <th mat-header-cell *matHeaderCellDef>Size</th>
                <td mat-cell *matCellDef="let backup">{{ backup.size_mb }} MB</td>
              </ng-container>

              <ng-container matColumnDef="created_at">
                <th mat-header-cell *matHeaderCellDef>Created</th>
                <td mat-cell *matCellDef="let backup">
                  {{ backup.created_at | date:'short' }}
                </td>
              </ng-container>

              <ng-container matColumnDef="uploaded">
                <th mat-header-cell *matHeaderCellDef>Cloud</th>
                <td mat-cell *matCellDef="let backup">
                  <mat-icon [class]="backup.uploaded ? 'text-success' : 'text-muted'">
                    {{ backup.uploaded ? 'cloud_done' : 'cloud_off' }}
                  </mat-icon>
                </td>
              </ng-container>

              <ng-container matColumnDef="lan">
                <th mat-header-cell *matHeaderCellDef>LAN</th>
                <td mat-cell *matCellDef="let backup">
                  <mat-icon [class]="backup.lan_copied ? 'text-success' : 'text-muted'">
                    {{ backup.lan_copied ? 'lan' : 'lan' }}
                  </mat-icon>
                </td>
              </ng-container>

              <ng-container matColumnDef="actions">
                <th mat-header-cell *matHeaderCellDef>Actions</th>
                <td mat-cell *matCellDef="let backup">
                  <button mat-icon-button (click)="restoreBackup(backup)" [disabled]="backup.status !== 'completed'">
                    <mat-icon>restore</mat-icon>
                  </button>
                </td>
              </ng-container>

              <tr mat-header-row *matHeaderRowDef="displayedColumns"></tr>
              <tr mat-row *matRowDef="let row; columns: displayedColumns;"></tr>
            </table>
          }
        </mat-card-content>
      </mat-card>

      @if (binlogStatus?.lan_enabled) {
        <mat-card class="binlog-card">
          <mat-card-header>
            <mat-card-title>LAN Binlog Shipping</mat-card-title>
          </mat-card-header>
          <mat-card-content>
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
            <button mat-stroked-button (click)="shipBinlogsLanNow()" [disabled]="lanBinlogShipping">
              @if (lanBinlogShipping) {
                <mat-spinner diameter="18"></mat-spinner>
              } @else {
                <mat-icon>send</mat-icon>
              }
              Ship to LAN Now
            </button>
          </mat-card-content>
        </mat-card>
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

      mat-progress-bar {
        margin-top: 1rem;
      }
    }

    .backup-stats {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
      gap: 1rem;
      margin-bottom: 1.5rem;
    }

    .stat-card {
      text-align: center;

      mat-icon {
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

    .history-card {
      mat-card-header {
        margin-bottom: 1rem;
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

      mat-icon {
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

      mat-card-header {
        margin-bottom: 1rem;
      }
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
  `]
})
export class BackupsComponent implements OnInit {
  private tauri = inject(TauriService);
  private notification = inject(NotificationService);

  backups: BackupRecord[] = [];
  loading = true;
  backupInProgress = false;
  currentBackupType: 'full' | 'partial' = 'full';

  displayedColumns = ['type', 'status', 'size', 'created_at', 'uploaded', 'lan', 'actions'];

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

    try {
      const result = await this.tauri.invoke<BackupResult>('start_backup', { type });
      if (result.success) {
        this.notification.success(
          `Backup completed: ${result.size_mb} MB in ${result.duration_seconds}s`
        );
      }
      await this.loadBackups();
    } finally {
      this.backupInProgress = false;
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
