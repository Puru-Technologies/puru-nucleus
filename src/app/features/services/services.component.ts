import { Component, OnInit, HostListener, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { RouterLink } from '@angular/router';
import { TauriService, ServiceInfo } from '../../core/services/tauri.service';
import { NotificationService } from '../../core/services/notification.service';

@Component({
  selector: 'app-services',
  standalone: true,
  imports: [CommonModule, RouterLink, FormsModule],
  template: `
    <div class="page">
      <div class="page-header">
        <div>
          <h1>Services</h1>
          <p class="page-subtitle">
            @if (!loading) {
              {{ runningCount }}/{{ services.length }} {{ isNative ? 'services' : 'containers' }} running
              @if (isNative) { <span class="mode-tag native">Native</span> }
            } @else {
              Loading services...
            }
          </p>
        </div>
        <div class="header-actions">
          <button class="btn btn-stroked" (click)="refreshServices()">
            <span class="material-icons">refresh</span>
            Refresh
          </button>
          <button class="btn btn-primary" (click)="startAll()" [disabled]="services.length === 0">
            <span class="material-icons">play_arrow</span>
            Start All
          </button>
        </div>
      </div>

      @if (loading) {
        <div class="loading-state">
          <span class="spinner spinner-lg"></span>
        </div>
      } @else if (services.length === 0) {
        <div class="card empty-card">
          <div class="empty-state">
            <div class="empty-icon">
              <span class="material-icons">cloud_off</span>
            </div>
            <h3>No Services Found</h3>
            <p>{{ isNative ? 'No JARs installed. Run the setup wizard to pull and start services.' : 'Docker may not be installed or no Puru containers are running.' }}</p>
            <button class="btn btn-stroked" routerLink="/setup">
              <span class="material-icons">build</span>
              Run Setup Wizard
            </button>
          </div>
        </div>
      } @else {
        <!-- Log Viewer Panel -->
        @if (logContainer) {
          <div class="card log-card">
            <div class="log-header">
              <div class="log-title">
                <span class="material-icons">article</span>
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
                <button class="btn-icon log-btn" (click)="refreshLogs()" [disabled]="logsLoading">
                  <span class="material-icons">refresh</span>
                </button>
                <button class="btn-icon log-btn" (click)="closeLogs()">
                  <span class="material-icons">close</span>
                </button>
              </div>
            </div>
            @if (logsLoading) {
              <div class="log-loading">
                <span class="spinner"></span>
              </div>
            } @else {
              <pre class="log-content">{{ logOutput || 'No logs available.' }}</pre>
            }
          </div>
        }

        <div class="card table-card">
          <table class="data-table services-table">
            <thead>
              <tr>
                <th class="sortable" (click)="sortBy('name')">
                  {{ isNative ? 'Service' : 'Container' }}
                  <span class="material-icons sort-arrow">{{ sortArrow('name') }}</span>
                </th>
                <th class="sortable" (click)="sortBy('image')">
                  {{ isNative ? 'Build' : 'Image' }}
                  <span class="material-icons sort-arrow">{{ sortArrow('image') }}</span>
                </th>
                <th class="sortable" (click)="sortBy('status')">
                  Status <span class="material-icons sort-arrow">{{ sortArrow('status') }}</span>
                </th>
                <th class="sortable" (click)="sortBy('health')">
                  Health <span class="material-icons sort-arrow">{{ sortArrow('health') }}</span>
                </th>
                <th>Ports</th>
                <th>Uptime</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              @for (service of sortedServices; track service.name) {
                <tr>
                  <td>
                    <div class="name-cell">
                      <div class="status-dot" [class]="'dot-' + service.status"></div>
                      <div class="name-info">
                        <span class="name-primary">{{ service.name }}</span>
                        <span class="name-secondary">{{ service.container_name }}</span>
                      </div>
                    </div>
                  </td>
                  <td><span class="image-text">{{ shortenImage(service.image) }}</span></td>
                  <td>
                    <span [class]="'chip chip-' + service.status">{{ service.status }}</span>
                    @if (service.detail) {
                      <div class="svc-detail">{{ service.detail }}</div>
                    }
                  </td>
                  <td>
                    @if (service.health) {
                      <span [class]="'chip chip-' + service.health">{{ service.health }}</span>
                      @if (service.health_response_ms != null) {
                        <span class="health-latency">{{ service.health_response_ms }}ms</span>
                      }
                    } @else {
                      <span class="text-muted">&mdash;</span>
                    }
                  </td>
                  <td><span class="port-text">{{ service.ports.join(', ') || '—' }}</span></td>
                  <td>{{ service.uptime || '—' }}</td>
                  <td class="actions-cell">
                    <div class="menu-wrap">
                      <button class="btn-icon" (click)="toggleMenu(service, $event)">
                        <span class="material-icons">more_vert</span>
                      </button>
                      @if (openMenu === service.name) {
                        <div class="menu" (click)="$event.stopPropagation()">
                          @if (service.status === 'stopped' || service.status === 'error') {
                            <button class="menu-item" (click)="startService(service); openMenu = null">
                              <span class="material-icons menu-green">play_arrow</span> Start
                            </button>
                          }
                          @if (service.status === 'notinstalled') {
                            <button class="menu-item" routerLink="/setup" (click)="openMenu = null">
                              <span class="material-icons menu-green">build</span> Install (Run Setup)
                            </button>
                          }
                          @if (service.status === 'running') {
                            <button class="menu-item" (click)="stopService(service); openMenu = null">
                              <span class="material-icons menu-red">stop</span> Stop
                            </button>
                          }
                          <button class="menu-item" (click)="restartService(service); openMenu = null">
                            <span class="material-icons menu-orange">refresh</span> Restart
                          </button>
                          <button class="menu-item" (click)="viewLogs(service); openMenu = null">
                            <span class="material-icons">article</span> View Logs
                          </button>
                          @if (isNative) {
                            <button class="menu-item" (click)="updateService(service); openMenu = null">
                              <span class="material-icons menu-green">system_update</span> Update
                            </button>
                            <button class="menu-item" (click)="rollbackService(service); openMenu = null">
                              <span class="material-icons menu-orange">undo</span> Rollback
                            </button>
                          }
                        </div>
                      }
                    </div>
                  </td>
                </tr>
              }
            </tbody>
          </table>
        </div>
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

    .mode-tag {
      font-size: 0.7rem;
      font-weight: 600;
      padding: 1px 8px;
      border-radius: 10px;
      margin-left: 6px;
      vertical-align: middle;

      &.native {
        background: #fff3e0;
        color: #e65100;
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
        .material-icons {
          font-size: 32px;
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
      /* visible so the row action dropdown isn't clipped at the card edge */
      overflow: visible;
    }

    .services-table {
      width: 100%;
    }

    .actions-cell {
      text-align: right;
      width: 48px;
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
      &.dot-notinstalled { background: var(--text-muted); }
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

    .svc-detail {
      margin-top: 4px;
      font-size: 11px;
      line-height: 1.3;
      color: var(--status-red);
      max-width: 320px;
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

      .material-icons {
        font-size: 18px;
      }
    }

    .log-actions {
      display: flex;
      align-items: center;
      gap: 6px;
    }

    .log-btn {
      color: #94a3b8;
      &:hover { background: rgba(255, 255, 255, 0.1); color: #fff; }
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
  isNative = false;
  logContainer: string | null = null;
  logOutput = '';
  logsLoading = false;
  logTimeFilter: 'tail' | '1h' | '6h' | '24h' | '3d' | '7d' = 'tail';

  sortKey = 'name';
  sortDir: 1 | -1 = 1;
  openMenu: string | null = null;

  get runningCount(): number {
    return this.services.filter(s => s.status === 'running').length;
  }

  get sortedServices(): ServiceInfo[] {
    const dir = this.sortDir;
    const key = this.sortKey;
    return [...this.services].sort((a, b) =>
      this.sortValue(a, key).localeCompare(this.sortValue(b, key)) * dir
    );
  }

  private sortValue(s: ServiceInfo, key: string): string {
    switch (key) {
      case 'image': return s.image || '';
      case 'status': return s.status || '';
      case 'health': return s.health || '';
      default: return s.name || '';
    }
  }

  sortBy(key: string): void {
    if (this.sortKey === key) {
      this.sortDir = this.sortDir === 1 ? -1 : 1;
    } else {
      this.sortKey = key;
      this.sortDir = 1;
    }
  }

  sortArrow(key: string): string {
    if (this.sortKey !== key) return '';
    return this.sortDir === 1 ? 'arrow_upward' : 'arrow_downward';
  }

  toggleMenu(s: ServiceInfo, ev: Event): void {
    ev.stopPropagation();
    this.openMenu = this.openMenu === s.name ? null : s.name;
  }

  @HostListener('document:click')
  closeMenus(): void {
    this.openMenu = null;
  }

  ngOnInit(): void {
    this.loadDeploymentMode();
    this.loadServices();
  }

  private async loadDeploymentMode(): Promise<void> {
    try {
      const mode = await this.tauri.invoke<string>('get_deployment_mode');
      this.isNative = mode === 'Native';
    } catch {
      this.isNative = false;
    }
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

  /** In native mode use service name; in Docker mode use container_name */
  private svcId(service: ServiceInfo): string {
    return this.isNative ? service.name : service.container_name;
  }

  async startService(service: ServiceInfo): Promise<void> {
    try {
      await this.tauri.invoke('start_service', { name: this.svcId(service) });
      this.notification.success(`Started ${service.name}`);
      await this.loadServices();
    } catch (error) {
      // Error handled by TauriService
    }
  }

  async stopService(service: ServiceInfo): Promise<void> {
    try {
      await this.tauri.invoke('stop_service', { name: this.svcId(service) });
      this.notification.success(`Stopped ${service.name}`);
      await this.loadServices();
    } catch (error) {
      // Error handled by TauriService
    }
  }

  async restartService(service: ServiceInfo): Promise<void> {
    try {
      await this.tauri.invoke('restart_service', { name: this.svcId(service) });
      this.notification.success(`Restarted ${service.name}`);
      await this.loadServices();
    } catch (error) {
      // Error handled by TauriService
    }
  }

  async startAll(): Promise<void> {
    // Only services whose build is installed can be started directly;
    // 'notinstalled' ones need a setup re-run (Pull JARs) first.
    const stoppedServices = this.services.filter(
      s => s.status === 'stopped' || s.status === 'error'
    );
    for (const service of stoppedServices) {
      try {
        await this.tauri.invoke('start_service', { name: this.svcId(service) });
      } catch (error) {
        // Continue with other services
      }
    }
    this.notification.success(`Started ${stoppedServices.length} services`);
    await this.loadServices();
  }

  async viewLogs(service: ServiceInfo): Promise<void> {
    this.logContainer = this.isNative ? service.name : service.container_name;
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

  async updateService(service: ServiceInfo): Promise<void> {
    if (!confirm(`Update ${service.name}? This will stop the service, pull a new JAR, and restart.`)) return;
    try {
      const result = await this.tauri.invoke<any>('update_native_service', { serviceName: service.name });
      this.notification.success(`Updated ${service.name} to build ${result.short_sha}`);
      await this.loadServices();
    } catch (error) {
      // Error handled by TauriService
    }
  }

  async rollbackService(service: ServiceInfo): Promise<void> {
    if (!confirm(`Rollback ${service.name} to previous JAR?`)) return;
    try {
      await this.tauri.invoke('rollback_native_service', { serviceName: service.name });
      this.notification.success(`Rolled back ${service.name}`);
      await this.loadServices();
    } catch (error) {
      // Error handled by TauriService
    }
  }
}
