import { Component, OnInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { MatCardModule } from '@angular/material/card';
import { MatIconModule } from '@angular/material/icon';
import { MatButtonModule } from '@angular/material/button';
import { MatChipsModule } from '@angular/material/chips';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
import { HospitalAlert } from '../../core/models/hospital.model';
import { TauriService } from '../../core/services/tauri.service';
import { NotificationService } from '../../core/services/notification.service';

@Component({
  selector: 'app-alerts',
  standalone: true,
  imports: [
    CommonModule,
    MatCardModule,
    MatIconModule,
    MatButtonModule,
    MatChipsModule,
    MatProgressSpinnerModule
  ],
  template: `
    <div class="alerts-page p-4">
      <div class="header flex justify-between items-center">
        <h1>Alerts</h1>
        <div class="actions">
          <button mat-stroked-button (click)="loadAlerts()">
            <mat-icon>refresh</mat-icon>
            Refresh
          </button>
          @if (unacknowledgedCount > 0) {
            <button mat-stroked-button (click)="acknowledgeAll()">
              <mat-icon>done_all</mat-icon>
              Acknowledge All
            </button>
          }
        </div>
      </div>

      <div class="alert-summary">
        <mat-card class="summary-card critical" [class.active]="criticalCount > 0">
          <mat-card-content>
            <mat-icon>error</mat-icon>
            <span class="count">{{ criticalCount }}</span>
            <span class="label">Critical</span>
          </mat-card-content>
        </mat-card>

        <mat-card class="summary-card warning" [class.active]="warningCount > 0">
          <mat-card-content>
            <mat-icon>warning</mat-icon>
            <span class="count">{{ warningCount }}</span>
            <span class="label">Warning</span>
          </mat-card-content>
        </mat-card>

        <mat-card class="summary-card info" [class.active]="infoCount > 0">
          <mat-card-content>
            <mat-icon>info</mat-icon>
            <span class="count">{{ infoCount }}</span>
            <span class="label">Info</span>
          </mat-card-content>
        </mat-card>
      </div>

      @if (loading) {
        <div class="loading-container">
          <mat-spinner diameter="48"></mat-spinner>
        </div>
      } @else if (alerts.length === 0) {
        <mat-card class="empty-state">
          <mat-card-content>
            <mat-icon>notifications_off</mat-icon>
            <p>No alerts</p>
            <span>All systems are running normally</span>
          </mat-card-content>
        </mat-card>
      } @else {
        <div class="alerts-list">
          @for (alert of alerts; track alert.id) {
            <mat-card class="alert-card" [class]="'severity-' + alert.severity" [class.acknowledged]="alert.acknowledged">
              <mat-card-content>
                <div class="alert-header">
                  <mat-icon>{{ getIcon(alert.severity) }}</mat-icon>
                  <div class="alert-info">
                    <div class="alert-title">{{ alert.title }}</div>
                    <div class="alert-meta">
                      <mat-chip size="small">{{ alert.category }}</mat-chip>
                      <span class="timestamp">{{ alert.created_at | date:'short' }}</span>
                    </div>
                  </div>
                  @if (!alert.acknowledged) {
                    <button mat-icon-button (click)="acknowledgeAlert(alert)">
                      <mat-icon>check</mat-icon>
                    </button>
                  } @else {
                    <mat-icon class="acknowledged-icon">check_circle</mat-icon>
                  }
                </div>
                <p class="alert-message">{{ alert.message }}</p>
              </mat-card-content>
            </mat-card>
          }
        </div>
      }
    </div>
  `,
  styles: [`
    .alerts-page {
      max-width: 1000px;
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

    .alert-summary {
      display: grid;
      grid-template-columns: repeat(3, 1fr);
      gap: 1rem;
      margin-bottom: 1.5rem;
    }

    .summary-card {
      text-align: center;
      opacity: 0.6;
      transition: opacity 0.2s;

      &.active {
        opacity: 1;
      }

      mat-card-content {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 0.25rem;
      }

      mat-icon {
        font-size: 2rem;
        width: 2rem;
        height: 2rem;
      }

      .count {
        font-size: 2rem;
        font-weight: 500;
      }

      .label {
        font-size: 0.875rem;
        color: #666;
      }

      &.critical {
        mat-icon, .count { color: #f44336; }
        &.active { border-left: 4px solid #f44336; }
      }

      &.warning {
        mat-icon, .count { color: #ff9800; }
        &.active { border-left: 4px solid #ff9800; }
      }

      &.info {
        mat-icon, .count { color: #2196f3; }
        &.active { border-left: 4px solid #2196f3; }
      }
    }

    .loading-container {
      display: flex;
      justify-content: center;
      padding: 3rem;
    }

    .empty-state {
      text-align: center;
      padding: 3rem;
      color: #4caf50;

      mat-icon {
        font-size: 4rem;
        width: 4rem;
        height: 4rem;
      }

      p {
        font-size: 1.25rem;
        margin: 1rem 0 0.5rem;
      }

      span {
        color: #666;
      }
    }

    .alerts-list {
      display: flex;
      flex-direction: column;
      gap: 1rem;
    }

    .alert-card {
      &.severity-critical {
        border-left: 4px solid #f44336;
        mat-icon:first-child { color: #f44336; }
      }

      &.severity-warning {
        border-left: 4px solid #ff9800;
        mat-icon:first-child { color: #ff9800; }
      }

      &.severity-info {
        border-left: 4px solid #2196f3;
        mat-icon:first-child { color: #2196f3; }
      }

      &.acknowledged {
        opacity: 0.6;
      }
    }

    .alert-header {
      display: flex;
      align-items: flex-start;
      gap: 1rem;
    }

    .alert-info {
      flex: 1;
    }

    .alert-title {
      font-weight: 500;
      margin-bottom: 0.25rem;
    }

    .alert-meta {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      font-size: 0.875rem;
      color: #666;
    }

    .timestamp {
      color: #999;
    }

    .alert-message {
      margin: 0.75rem 0 0 2.5rem;
      color: #666;
    }

    .acknowledged-icon {
      color: #4caf50;
    }
  `]
})
export class AlertsComponent implements OnInit {
  private tauri = inject(TauriService);
  private notification = inject(NotificationService);

  alerts: HospitalAlert[] = [];
  loading = true;

  get criticalCount(): number {
    return this.alerts.filter(a => a.severity === 'critical' && !a.acknowledged).length;
  }

  get warningCount(): number {
    return this.alerts.filter(a => a.severity === 'warning' && !a.acknowledged).length;
  }

  get infoCount(): number {
    return this.alerts.filter(a => a.severity === 'info' && !a.acknowledged).length;
  }

  get unacknowledgedCount(): number {
    return this.alerts.filter(a => !a.acknowledged).length;
  }

  ngOnInit(): void {
    this.loadAlerts();
  }

  async loadAlerts(): Promise<void> {
    this.loading = true;
    try {
      this.alerts = await this.tauri.invoke<HospitalAlert[]>('get_alerts');
      // Sort by severity and then by date
      this.alerts.sort((a, b) => {
        const severityOrder = { critical: 0, warning: 1, info: 2 };
        if (severityOrder[a.severity] !== severityOrder[b.severity]) {
          return severityOrder[a.severity] - severityOrder[b.severity];
        }
        return new Date(b.created_at).getTime() - new Date(a.created_at).getTime();
      });
    } finally {
      this.loading = false;
    }
  }

  getIcon(severity: string): string {
    switch (severity) {
      case 'critical': return 'error';
      case 'warning': return 'warning';
      case 'info': return 'info';
      default: return 'help';
    }
  }

  async acknowledgeAlert(alert: HospitalAlert): Promise<void> {
    try {
      await this.tauri.invoke('acknowledge_alert', { alertId: alert.id });
      alert.acknowledged = true;
      alert.acknowledged_at = new Date().toISOString();
      this.notification.success('Alert acknowledged');
    } catch (error) {
      // Error handled by TauriService
    }
  }

  async acknowledgeAll(): Promise<void> {
    const unacknowledged = this.alerts.filter(a => !a.acknowledged);
    for (const alert of unacknowledged) {
      try {
        await this.tauri.invoke('acknowledge_alert', { alertId: alert.id });
        alert.acknowledged = true;
        alert.acknowledged_at = new Date().toISOString();
      } catch (error) {
        // Continue with others
      }
    }
    this.notification.success(`Acknowledged ${unacknowledged.length} alerts`);
  }
}
