import { Component, OnInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { MatCardModule } from '@angular/material/card';
import { MatIconModule } from '@angular/material/icon';
import { MatButtonModule } from '@angular/material/button';
import { MatTableModule } from '@angular/material/table';
import { MatChipsModule } from '@angular/material/chips';
import { MatMenuModule } from '@angular/material/menu';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
import { FormsModule } from '@angular/forms';
import { RouterLink } from '@angular/router';
import { TauriService, ServiceInfo } from '../../core/services/tauri.service';
import { NotificationService } from '../../core/services/notification.service';

@Component({
  selector: 'app-services',
  standalone: true,
  imports: [
    CommonModule,
    RouterLink,
    MatCardModule,
    MatIconModule,
    MatButtonModule,
    MatTableModule,
    MatChipsModule,
    MatMenuModule,
    MatProgressSpinnerModule,
    FormsModule
  ],
  template: `
    <div class="page">
      <div class="page-header">
        <div>
          <h1>Services</h1>
          <p class="page-subtitle">
            @if (!loading) {
              {{ runningCount }}/{{ services.length }} containers running
            } @else {
              Loading containers...
            }
          </p>
        </div>
        <div class="header-actions">
          <button mat-stroked-button (click)="refreshServices()">
            <mat-icon>refresh</mat-icon>
            Refresh
          </button>
          <button mat-flat-button color="primary" (click)="startAll()" [disabled]="services.length === 0">
            <mat-icon>play_arrow</mat-icon>
            Start All
          </button>
        </div>
      </div>

      @if (loading) {
        <div class="loading-state">
          <mat-spinner diameter="40"></mat-spinner>
        </div>
      } @else if (services.length === 0) {
        <mat-card class="empty-card">
          <div class="empty-state">
            <div class="empty-icon">
              <mat-icon>cloud_off</mat-icon>
            </div>
            <h3>No Docker Services Found</h3>
            <p>Docker may not be installed or no Puru containers are running.</p>
            <button mat-stroked-button routerLink="/setup">
              <mat-icon>build</mat-icon>
              Run Setup Wizard
            </button>
          </div>
        </mat-card>
      } @else {
        <!-- Log Viewer Panel -->
        @if (logContainer) {
          <mat-card class="log-card">
            <div class="log-header">
              <div class="log-title">
                <mat-icon>article</mat-icon>
                <span>Logs: {{ logContainer }}</span>
              </div>
              <div class="log-actions">
                <select class="log-time-select" [(ngModel)]="logTimeFilter" (ngModelChange)="refreshLogs()">
                  <option value="tail">Last 200 lines</option>
                  <option value="1h">Last 1 hour</option>
                  <option value="6h">Last 6 hours</option>
                  <option value="24h">Last 24 hours</option>
                  <option value="3d">Last 3 days</option>
                  <option value="7d">Last 7 days</option>
                </select>
                <button mat-icon-button (click)="refreshLogs()" [disabled]="logsLoading">
                  <mat-icon>refresh</mat-icon>
                </button>
                <button mat-icon-button (click)="closeLogs()">
                  <mat-icon>close</mat-icon>
                </button>
              </div>
            </div>
            @if (logsLoading) {
              <div class="log-loading">
                <mat-spinner diameter="24"></mat-spinner>
              </div>
            } @else {
              <pre class="log-content">{{ logOutput || 'No logs available.' }}</pre>
            }
          </mat-card>
        }

        <mat-card class="table-card">
          <table mat-table [dataSource]="services" class="services-table">
            <!-- Container Name -->
            <ng-container matColumnDef="name">
              <th mat-header-cell *matHeaderCellDef>Container</th>
              <td mat-cell *matCellDef="let service">
                <div class="name-cell">
                  <div class="status-dot" [class]="'dot-' + service.status"></div>
                  <div class="name-info">
                    <span class="name-primary">{{ service.name }}</span>
                    <span class="name-secondary">{{ service.container_name }}</span>
                  </div>
                </div>
              </td>
            </ng-container>

            <!-- Image -->
            <ng-container matColumnDef="image">
              <th mat-header-cell *matHeaderCellDef>Image</th>
              <td mat-cell *matCellDef="let service">
                <span class="image-text">{{ shortenImage(service.image) }}</span>
              </td>
            </ng-container>

            <!-- Status -->
            <ng-container matColumnDef="status">
              <th mat-header-cell *matHeaderCellDef>Status</th>
              <td mat-cell *matCellDef="let service">
                <mat-chip [class]="'chip-' + service.status">
                  {{ service.status }}
                </mat-chip>
              </td>
            </ng-container>

            <!-- Health -->
            <ng-container matColumnDef="health">
              <th mat-header-cell *matHeaderCellDef>Health</th>
              <td mat-cell *matCellDef="let service">
                @if (service.health) {
                  <mat-chip [class]="'chip-' + service.health">
                    {{ service.health }}
                  </mat-chip>
                  @if (service.health_response_ms != null) {
                    <span class="health-latency">{{ service.health_response_ms }}ms</span>
                  }
                } @else {
                  <span class="text-muted">&mdash;</span>
                }
              </td>
            </ng-container>

            <!-- Ports -->
            <ng-container matColumnDef="ports">
              <th mat-header-cell *matHeaderCellDef>Ports</th>
              <td mat-cell *matCellDef="let service">
                <span class="port-text">{{ service.ports.join(', ') || '—' }}</span>
              </td>
            </ng-container>

            <!-- Uptime -->
            <ng-container matColumnDef="uptime">
              <th mat-header-cell *matHeaderCellDef>Uptime</th>
              <td mat-cell *matCellDef="let service">
                {{ service.uptime || '—' }}
              </td>
            </ng-container>

            <!-- Actions -->
            <ng-container matColumnDef="actions">
              <th mat-header-cell *matHeaderCellDef></th>
              <td mat-cell *matCellDef="let service">
                <button mat-icon-button [matMenuTriggerFor]="actionMenu">
                  <mat-icon>more_vert</mat-icon>
                </button>
                <mat-menu #actionMenu="matMenu">
                  @if (service.status !== 'running') {
                    <button mat-menu-item (click)="startService(service)">
                      <mat-icon class="menu-green">play_arrow</mat-icon>
                      Start
                    </button>
                  }
                  @if (service.status === 'running') {
                    <button mat-menu-item (click)="stopService(service)">
                      <mat-icon class="menu-red">stop</mat-icon>
                      Stop
                    </button>
                  }
                  <button mat-menu-item (click)="restartService(service)">
                    <mat-icon class="menu-orange">refresh</mat-icon>
                    Restart
                  </button>
                  <button mat-menu-item (click)="viewLogs(service)">
                    <mat-icon>article</mat-icon>
                    View Logs
                  </button>
                </mat-menu>
              </td>
            </ng-container>

            <tr mat-header-row *matHeaderRowDef="displayedColumns"></tr>
            <tr mat-row *matRowDef="let row; columns: displayedColumns;"></tr>
          </table>
        </mat-card>
      }
    </div>
  `,
  styles: [`
    .page {
      max-width: 1200px;
      margin: 0 auto;
      padding: 28px 32px;
    }

    .page-header {
      display: flex;
      justify-content: space-between;
      align-items: flex-start;
      margin-bottom: 20px;

      h1 {
        font-size: 1.5rem;
        font-weight: 700;
        margin-bottom: 2px;
      }
      .page-subtitle {
        color: var(--text-secondary);
        font-size: 0.85rem;
      }
    }

    .header-actions {
      display: flex;
      gap: 10px;
    }

    .loading-state {
      display: flex;
      justify-content: center;
      padding: 80px 0;
    }

    /* ── Empty State ──────────────────────────── */
    .empty-card {
      padding: 0 !important;
    }
    .empty-state {
      display: flex;
      flex-direction: column;
      align-items: center;
      gap: 12px;
      padding: 60px 20px;
      text-align: center;

      .empty-icon {
        width: 64px;
        height: 64px;
        border-radius: 16px;
        background: #f1f5f9;
        display: flex;
        align-items: center;
        justify-content: center;
        mat-icon {
          font-size: 32px;
          width: 32px;
          height: 32px;
          color: #94a3b8;
        }
      }

      h3 {
        font-size: 1rem;
        font-weight: 600;
        color: var(--text-primary);
        margin: 0;
      }
      p {
        font-size: 0.85rem;
        color: var(--text-secondary);
        margin: 0;
        max-width: 340px;
      }
      button {
        margin-top: 8px;
      }
    }

    /* ── Table ────────────────────────────────── */
    .table-card {
      padding: 0 !important;
      overflow: hidden;
    }

    .services-table {
      width: 100%;
    }

    .name-cell {
      display: flex;
      align-items: center;
      gap: 12px;
    }

    .status-dot {
      width: 8px;
      height: 8px;
      border-radius: 50%;
      flex-shrink: 0;

      &.dot-running { background: var(--status-green); box-shadow: 0 0 6px rgba(34, 197, 94, 0.4); }
      &.dot-stopped { background: var(--status-red); }
      &.dot-starting { background: var(--status-orange); }
      &.dot-error { background: var(--status-red); }
    }

    .name-info {
      display: flex;
      flex-direction: column;
    }
    .name-primary {
      font-weight: 600;
      font-size: 0.85rem;
      color: var(--text-primary);
    }
    .name-secondary {
      font-size: 0.7rem;
      color: var(--text-muted);
      font-family: 'SF Mono', 'Fira Code', monospace;
    }

    .image-text {
      font-size: 0.75rem;
      color: var(--text-secondary);
      font-family: 'SF Mono', 'Fira Code', monospace;
    }

    .port-text {
      font-size: 0.8rem;
      font-family: 'SF Mono', 'Fira Code', monospace;
      color: var(--text-secondary);
    }

    .text-muted {
      color: var(--text-muted);
    }

    .health-latency {
      font-size: 11px;
      color: var(--text-secondary);
      margin-left: 6px;
      font-family: 'SF Mono', 'Fira Code', monospace;
    }

    .menu-green { color: var(--status-green) !important; }
    .menu-red { color: var(--status-red) !important; }
    .menu-orange { color: var(--status-orange) !important; }

    /* ── Log Viewer ──────────────────────────── */
    .log-card {
      margin-bottom: 16px;
      padding: 0 !important;
      overflow: hidden;
    }

    .log-header {
      display: flex;
      justify-content: space-between;
      align-items: center;
      padding: 8px 16px;
      background: #1e293b;
      color: #e2e8f0;
    }

    .log-title {
      display: flex;
      align-items: center;
      gap: 8px;
      font-size: 0.85rem;
      font-weight: 600;

      mat-icon {
        font-size: 18px;
        width: 18px;
        height: 18px;
      }
    }

    .log-actions {
      display: flex;
      align-items: center;
      gap: 6px;

      button {
        color: #94a3b8;
      }
    }

    .log-time-select {
      background: #1e293b;
      color: #e2e8f0;
      border: 1px solid #334155;
      border-radius: 4px;
      padding: 4px 8px;
      font-size: 0.75rem;
      font-family: 'SF Mono', 'Fira Code', monospace;
      cursor: pointer;
      outline: none;

      &:hover {
        border-color: #475569;
      }
      &:focus {
        border-color: #6366f1;
      }

      option {
        background: #1e293b;
        color: #e2e8f0;
      }
    }

    .log-loading {
      display: flex;
      justify-content: center;
      padding: 24px;
      background: #0f172a;
    }

    .log-content {
      margin: 0;
      padding: 16px;
      background: #0f172a;
      color: #e2e8f0;
      font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
      font-size: 0.75rem;
      line-height: 1.6;
      max-height: 400px;
      overflow-y: auto;
      white-space: pre-wrap;
      word-break: break-all;
    }
  `]
})
export class ServicesComponent implements OnInit {
  private tauri = inject(TauriService);
  private notification = inject(NotificationService);

  services: ServiceInfo[] = [];
  loading = true;
  logContainer: string | null = null;
  logOutput = '';
  logsLoading = false;
  logTimeFilter: 'tail' | '1h' | '6h' | '24h' | '3d' | '7d' = 'tail';

  displayedColumns = ['name', 'image', 'status', 'health', 'ports', 'uptime', 'actions'];

  get runningCount(): number {
    return this.services.filter(s => s.status === 'running').length;
  }

  ngOnInit(): void {
    this.loadServices();
  }

  async loadServices(): Promise<void> {
    this.loading = true;
    try {
      this.services = await this.tauri.invoke<ServiceInfo[]>('get_services');
    } finally {
      this.loading = false;
    }
  }

  async refreshServices(): Promise<void> {
    await this.loadServices();
    this.notification.success('Services refreshed');
  }

  shortenImage(image: string): string {
    // Trim long registry prefixes for display
    return image
      .replace(/^asia-south2-docker\.pkg\.dev\/puru-255206\/puru1\//, '')
      .replace(/^gcr\.io\/puru-255206\//, '');
  }

  getStatusIcon(status: string): string {
    switch (status) {
      case 'running': return 'check_circle';
      case 'stopped': return 'stop_circle';
      case 'starting': return 'pending';
      case 'error': return 'error';
      default: return 'help';
    }
  }

  async startService(service: ServiceInfo): Promise<void> {
    try {
      await this.tauri.invoke('start_service', { name: service.container_name });
      this.notification.success(`Started ${service.container_name}`);
      await this.loadServices();
    } catch (error) {
      // Error handled by TauriService
    }
  }

  async stopService(service: ServiceInfo): Promise<void> {
    try {
      await this.tauri.invoke('stop_service', { name: service.container_name });
      this.notification.success(`Stopped ${service.container_name}`);
      await this.loadServices();
    } catch (error) {
      // Error handled by TauriService
    }
  }

  async restartService(service: ServiceInfo): Promise<void> {
    try {
      await this.tauri.invoke('restart_service', { name: service.container_name });
      this.notification.success(`Restarted ${service.container_name}`);
      await this.loadServices();
    } catch (error) {
      // Error handled by TauriService
    }
  }

  async startAll(): Promise<void> {
    const stoppedServices = this.services.filter(s => s.status !== 'running');
    for (const service of stoppedServices) {
      try {
        await this.tauri.invoke('start_service', { name: service.container_name });
      } catch (error) {
        // Continue with other services
      }
    }
    this.notification.success(`Started ${stoppedServices.length} services`);
    await this.loadServices();
  }

  async viewLogs(service: ServiceInfo): Promise<void> {
    this.logContainer = service.container_name;
    this.logTimeFilter = 'tail';
    this.logsLoading = true;
    this.logOutput = '';
    try {
      this.logOutput = await this.tauri.invoke<string>('get_container_logs', this.buildLogArgs());
    } catch {
      this.logOutput = `Failed to fetch logs for ${service.container_name}.`;
    } finally {
      this.logsLoading = false;
    }
  }

  async refreshLogs(): Promise<void> {
    if (!this.logContainer) return;
    this.logsLoading = true;
    try {
      this.logOutput = await this.tauri.invoke<string>('get_container_logs', this.buildLogArgs());
    } catch {
      this.logOutput = 'Failed to refresh logs.';
    } finally {
      this.logsLoading = false;
    }
  }

  private buildLogArgs(): Record<string, unknown> {
    const args: Record<string, unknown> = { containerName: this.logContainer };
    if (this.logTimeFilter === 'tail') {
      args['tail'] = 200;
    } else {
      args['tail'] = 0;
      args['since'] = this.getLogSinceTimestamp();
    }
    return args;
  }

  private getLogSinceTimestamp(): number {
    const now = Math.floor(Date.now() / 1000);
    switch (this.logTimeFilter) {
      case '1h': return now - 3600;
      case '6h': return now - 21600;
      case '24h': return now - 86400;
      case '3d': return now - 259200;
      case '7d': return now - 604800;
      default: return 0;
    }
  }

  closeLogs(): void {
    this.logContainer = null;
    this.logOutput = '';
  }
}
