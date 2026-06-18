import { Component, OnInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { HospitalAlert } from '../../core/models/hospital.model';
import { TauriService } from '../../core/services/tauri.service';
import { NotificationService } from '../../core/services/notification.service';

@Component({
  selector: 'app-alerts',
  standalone: true,
  imports: [
    CommonModule
  ],
  template: `
    <div class="alerts-page p-4">
      <div class="header flex justify-between items-center">
        <h1>Alerts</h1>
        <div class="actions">
          <button class="btn btn-stroked" (click)="loadAlerts()">
            <span class="material-icons">refresh</span>
            Refresh
          </button>
          @if (unacknowledgedCount > 0) {
            <button class="btn btn-stroked" (click)="acknowledgeAll()">
              <span class="material-icons">done_all</span>
              Acknowledge All
            </button>
          }
        </div>
      </div>

      <div class="alert-summary">
        <div class="card card-pad summary-card critical" [class.active]="criticalCount > 0">
          <div class="summary-content">
            <span class="material-icons">error</span>
            <span class="count">{{ criticalCount }}</span>
            <span class="label">Critical</span>
          </div>
        </div>

        <div class="card card-pad summary-card warning" [class.active]="warningCount > 0">
          <div class="summary-content">
            <span class="material-icons">warning</span>
            <span class="count">{{ warningCount }}</span>
            <span class="label">Warning</span>
          </div>
        </div>

        <div class="card card-pad summary-card info" [class.active]="infoCount > 0">
          <div class="summary-content">
            <span class="material-icons">info</span>
            <span class="count">{{ infoCount }}</span>
            <span class="label">Info</span>
          </div>
        </div>
      </div>

      @if (loading) {
        <div class="loading-container">
          <span class="spinner spinner-lg"></span>
        </div>
      } @else if (alerts.length === 0) {
        <div class="card card-pad empty-state">
          <span class="material-icons">notifications_off</span>
          <p>No alerts</p>
          <span>All systems are running normally</span>
        </div>
      } @else {
        <div class="alerts-list">
          @for (alert of alerts; track alert.id) {
            <div class="card card-pad alert-card" [class]="'card card-pad alert-card severity-' + alert.severity" [class.acknowledged]="alert.acknowledged">
              <div class="alert-header">
                <span class="material-icons">{{ getIcon(alert.severity) }}</span>
                <div class="alert-info">
                  <div class="alert-title">{{ alert.title }}</div>
                  <div class="alert-meta">
                    <span class="chip">{{ alert.category }}</span>
                    <span class="timestamp">{{ alert.created_at | date:'short' }}</span>
                  </div>
                </div>
                @if (!alert.acknowledged) {
                  <button class="btn-icon" (click)="acknowledgeAlert(alert)">
                    <span class="material-icons">check</span>
                  </button>
                } @else {
                  <span class="material-icons acknowledged-icon">check_circle</span>
                }
              </div>
              <p class="alert-message">{{ alert.message }}</p>
            </div>
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

      .summary-content {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 0.25rem;
      }

      .material-icons {
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
        .material-icons, .count { color: #f44336; }
        &.active { border-left: 4px solid #f44336; }
      }

      &.warning {
        .material-icons, .count { color: #ff9800; }
        &.active { border-left: 4px solid #ff9800; }
      }

      &.info {
        .material-icons, .count { color: #2196f3; }
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

      .material-icons {
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
        .material-icons:first-child { color: #f44336; }
      }

      &.severity-warning {
        border-left: 4px solid #ff9800;
        .material-icons:first-child { color: #ff9800; }
      }

      &.severity-info {
        border-left: 4px solid #2196f3;
        .material-icons:first-child { color: #2196f3; }
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
