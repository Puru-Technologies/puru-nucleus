import { Component, OnInit, OnDestroy, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { RouterLink } from '@angular/router';
import { TauriService, ServiceInfo, SystemInfo, DaemonStatus, NetworkStatus, SpeedTestResult, BackupRecord } from '../../core/services/tauri.service';
import { License, getLicenseStatus, LicenseStatus } from '../../core/models/license.model';
import { interval, Subscription } from 'rxjs';

@Component({
  selector: 'app-dashboard',
  standalone: true,
  imports: [
    CommonModule,
    RouterLink,
  ],
  template: `
    <div class="page">
      <!-- Page Header -->
      <div class="page-header">
        <h1>Dashboard</h1>
        <p class="page-subtitle">
          @if (license?.hospital_name) {
            {{ license?.hospital_name }} &mdash;
          }
          System overview and quick actions
        </p>
      </div>

      <!-- License Banner -->
      @if (licenseStatus && licenseStatus.status !== 'active' && licenseStatus.status !== 'unlimited') {
        <div class="license-banner" [class]="'banner-' + licenseStatus.status">
          <span class="material-icons">{{ licenseStatus.icon }}</span>
          <div class="banner-text">
            <strong>{{ licenseStatus.status === 'expired' ? 'License Expired' : 'License Expiring Soon' }}</strong>
            <span>{{ licenseStatus.message }}</span>
          </div>
          <button class="btn btn-stroked" routerLink="/settings">Contact Support</button>
        </div>
      }

      <!-- Stats Row -->
      <div class="stats-row">
        @if (systemInfo) {
          <div class="stat-card">
            <div class="stat-icon" [class]="cpuClass"><span class="material-icons">memory</span></div>
            <div class="stat-body">
              <span class="stat-value">{{ telemetry?.cpu_percent | number:'1.0-0' }}%</span>
              <span class="stat-label">CPU ({{ systemInfo.cpu_cores }} cores)</span>
            </div>
          </div>
          <div class="stat-card">
            <div class="stat-icon" [class]="ramClass"><span class="material-icons">storage</span></div>
            <div class="stat-body">
              <span class="stat-value">{{ telemetry?.ram_gb | number:'1.1-1' }} / {{ systemInfo.total_ram_gb | number:'1.0-0' }} GB</span>
              <span class="stat-label">RAM Usage</span>
            </div>
          </div>
          <div class="stat-card">
            <div class="stat-icon" [class]="diskClass"><span class="material-icons">hard_drive</span></div>
            <div class="stat-body">
              <span class="stat-value">{{ telemetry?.disk_percent | number:'1.0-0' }}%</span>
              <span class="stat-label">Disk ({{ systemInfo.disk_free_gb | number:'1.0-0' }} GB free)</span>
            </div>
          </div>
          <div class="stat-card">
            <div class="stat-icon" [class]="services.length > 0 ? 'si-green' : 'si-muted'">
              <span class="material-icons">dns</span>
            </div>
            <div class="stat-body">
              <span class="stat-value">{{ runningCount }}/{{ services.length }}</span>
              <span class="stat-label">Services Up</span>
            </div>
          </div>
          <div class="stat-card">
            <div class="stat-icon" [class]="daemonRunning ? 'si-green' : 'si-muted'">
              <span class="material-icons">router</span>
            </div>
            <div class="stat-body">
              <span class="stat-value">{{ daemonRunning ? 'Online' : 'Offline' }}</span>
              <span class="stat-label">Daemon</span>
            </div>
          </div>
          <div class="stat-card">
            <div class="stat-icon" [class]="networkStatClass">
              <span class="material-icons">{{ networkStatus?.connected ? 'wifi' : 'wifi_off' }}</span>
            </div>
            <div class="stat-body">
              <span class="stat-value">{{ networkStatValue }}</span>
              <span class="stat-label">Network</span>
            </div>
          </div>
          <div class="stat-card">
            <div class="stat-icon" [class]="lastBackupTime ? 'si-green' : 'si-muted'">
              <span class="material-icons">backup</span>
            </div>
            <div class="stat-body">
              <span class="stat-value">{{ lastBackupTime || 'Never' }}</span>
              <span class="stat-label">Last Backup</span>
            </div>
          </div>
        } @else {
          @for (_ of [1,2,3,4,5,6]; track $index) {
            <div class="stat-card stat-skeleton">
              <div class="skeleton-circle"></div>
              <div class="skeleton-lines">
                <div class="skeleton-line w60"></div>
                <div class="skeleton-line w40"></div>
              </div>
            </div>
          }
        }
      </div>

      <!-- Main Grid -->
      <div class="grid-2">
        <!-- Services Card -->
        <div class="card dash-card">
          <div class="card-top">
            <h3>Services</h3>
            <button class="btn btn-stroked btn-sm" routerLink="/services">
              Manage
              <span class="material-icons">arrow_forward</span>
            </button>
          </div>
          @if (services.length > 0) {
            <div class="service-list">
              @for (service of services.slice(0, 8); track service.name) {
                <div class="svc-row">
                  <div class="svc-indicator" [class]="'ind-' + service.status"></div>
                  <span class="svc-name">{{ service.name }}</span>
                  <span class="svc-container">{{ service.container_name }}</span>
                  <span class="svc-status" [class]="'st-' + service.status">{{ service.status }}</span>
                </div>
              }
            </div>
            @if (services.length > 8) {
              <div class="card-footer-link">
                <a routerLink="/services">View all {{ services.length }} services</a>
              </div>
            }
          } @else {
            <div class="empty-state">
              <span class="material-icons">cloud_off</span>
              <span>No Docker services detected</span>
              <a routerLink="/setup" class="empty-link">Run Setup</a>
            </div>
          }
        </div>

        <!-- License & Quick Actions -->
        <div class="right-stack">
          <!-- License Card -->
          <div class="card dash-card">
            <div class="card-top">
              <h3>License</h3>
            </div>
            @if (license && licenseStatus) {
              <div class="license-row">
                <div class="license-badge" [class]="'lb-' + licenseStatus.status">
                  <span class="material-icons">{{ licenseStatus.icon }}</span>
                </div>
                <div class="license-info">
                  <span class="license-plan">{{ license.hospital_name }}</span>
                  <span class="license-msg" [class]="'lt-' + licenseStatus.status">{{ licenseStatus.message }}</span>
                </div>
              </div>
              <div class="feature-tags">
                <span class="ftag" [class.active]="license.features.binlog_shipping">
                  <span class="material-icons">{{ license.features.binlog_shipping ? 'check' : 'close' }}</span>
                  Binlog Shipping
                </span>
                <span class="ftag" [class.active]="license.features.point_in_time_recovery">
                  <span class="material-icons">{{ license.features.point_in_time_recovery ? 'check' : 'close' }}</span>
                  PITR
                </span>
                <span class="ftag" [class.active]="license.features.priority_support">
                  <span class="material-icons">{{ license.features.priority_support ? 'check' : 'close' }}</span>
                  Priority Support
                </span>
              </div>
              @if (license.machine_name || license.machine_fingerprint) {
                <div class="machine-row">
                  <span class="material-icons">computer</span>
                  <span class="machine-name">{{ license.machine_name || 'Unknown' }}</span>
                  @if (license.machine_fingerprint) {
                    <code class="machine-fp">{{ license.machine_fingerprint.slice(0, 16) }}...</code>
                  }
                </div>
              }
            } @else {
              <div class="empty-state sm">
                <span class="material-icons">license</span>
                <span>No license activated</span>
                <a routerLink="/settings" class="empty-link">Activate License</a>
              </div>
            }
          </div>

          <!-- Network Card -->
          <div class="card dash-card">
            <div class="card-top">
              <h3>Network</h3>
              <button class="btn btn-stroked btn-sm" (click)="runSpeedTest()" [disabled]="speedTestRunning">
                @if (speedTestRunning) {
                  <span class="spinner" style="width:14px;height:14px;border-width:2px"></span>
                } @else {
                  <span class="material-icons">speed</span>
                }
                Speed Test
              </button>
            </div>
            <div class="network-rows">
              <div class="net-row">
                <span class="net-label">Internet</span>
                <span class="net-value" [class]="networkStatus?.connected ? 'nv-green' : 'nv-red'">
                  {{ networkStatus?.connected ? 'Connected' : (networkStatus ? 'Offline' : '—') }}
                </span>
              </div>
              <div class="net-row">
                <span class="net-label">Latency</span>
                <span class="net-value" [class]="latencyClass">
                  {{ networkStatus?.latency_ms != null ? networkStatus!.latency_ms + ' ms' : '—' }}
                </span>
              </div>
              <div class="net-row">
                <span class="net-label">DICOM Server</span>
                <span class="net-value" [class]="gcpClass">
                  @if (!networkStatus) {
                    —
                  } @else if (networkStatus.gcp_reachable) {
                    Reachable{{ networkStatus.gcp_latency_ms != null ? ' (' + networkStatus.gcp_latency_ms + ' ms)' : '' }}
                  } @else {
                    Unreachable
                  }
                </span>
              </div>
              @if (speedTestResult) {
                <div class="net-row">
                  <span class="net-label">Download</span>
                  <span class="net-value" [class]="downloadClass">
                    {{ speedTestResult.download_mbps != null ? (speedTestResult.download_mbps | number:'1.1-1') + ' Mbps' : '—' }}
                  </span>
                </div>
                <div class="net-row">
                  <span class="net-label">Upload</span>
                  <span class="net-value" [class]="uploadClass">
                    {{ speedTestResult.upload_mbps != null ? (speedTestResult.upload_mbps | number:'1.1-1') + ' Mbps' : '—' }}
                  </span>
                </div>
              }
            </div>
          </div>

          <!-- Quick Actions -->
          <div class="card dash-card">
            <div class="card-top">
              <h3>Quick Actions</h3>
            </div>
            <div class="actions-grid">
              <button class="action-btn" (click)="runBackup()">
                <div class="ab-icon ab-blue"><span class="material-icons">backup</span></div>
                <span>Backup</span>
              </button>
              <button class="action-btn" (click)="restartServices()">
                <div class="ab-icon ab-orange"><span class="material-icons">refresh</span></div>
                <span>Restart All</span>
              </button>
              <button class="action-btn" (click)="syncConfig()">
                <div class="ab-icon ab-purple"><span class="material-icons">cloud_sync</span></div>
                <span>Sync</span>
              </button>
              <button class="action-btn" routerLink="/alerts">
                <div class="ab-icon ab-green"><span class="material-icons">notifications_none</span></div>
                <span>Alerts</span>
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  `,
  styles: [`
    .page {
      max-width: 1200px;
      margin: 0 auto;
      padding: 28px 32px;
    }

    /* ── License Banner ─────────────────────────── */
    .license-banner {
      display: flex;
      align-items: center;
      gap: 12px;
      padding: 12px 16px;
      border-radius: var(--radius-md);
      margin-bottom: 20px;
      font-size: 0.875rem;

      .material-icons { font-size: 20px; width: 20px; height: 20px; }
      .banner-text {
        flex: 1;
        display: flex;
        flex-direction: column;
        gap: 2px;
        strong { font-size: 0.8rem; }
        span { color: inherit; opacity: 0.8; font-size: 0.8rem; }
      }

      &.banner-expired {
        background: var(--status-red-bg);
        color: #991b1b;
        border: 1px solid #fecaca;
      }
      &.banner-expiring_soon {
        background: var(--status-orange-bg);
        color: #92400e;
        border: 1px solid #fde68a;
      }
    }

    /* ── Stats Row ──────────────────────────────── */
    .stats-row {
      display: grid;
      /* responsive auto-fit — cards wrap instead of forcing 6/7 columns and
         overflowing on narrow widths */
      grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
      gap: 14px;
      margin-bottom: 20px;
    }

    .stat-card {
      display: flex;
      align-items: center;
      gap: 12px;
      min-width: 0;
      background: var(--bg-card);
      border: 1px solid var(--border-card);
      border-radius: var(--radius-md);
      padding: 14px 16px;
      box-shadow: var(--shadow-sm);
    }

    .stat-icon {
      width: 42px;
      height: 42px;
      border-radius: 10px;
      display: flex;
      align-items: center;
      justify-content: center;
      flex-shrink: 0;

      .material-icons {
        font-size: 22px;
        width: 22px;
        height: 22px;
        color: white;
      }

      &.si-blue { background: linear-gradient(135deg, #3b82f6, #6366f1); }
      &.si-purple { background: linear-gradient(135deg, #8b5cf6, #a855f7); }
      &.si-green { background: linear-gradient(135deg, #22c55e, #16a34a); }
      &.si-orange { background: linear-gradient(135deg, #f59e0b, #d97706); }
      &.si-orange { background: linear-gradient(135deg, #f59e0b, #d97706); }
      &.si-red { background: linear-gradient(135deg, #ef4444, #dc2626); }
      &.si-muted { background: #cbd5e1; }
    }

    .stat-body {
      display: flex;
      flex-direction: column;
      min-width: 0;
    }
    .stat-value {
      font-size: 1.15rem;
      font-weight: 700;
      color: var(--text-primary);
      line-height: 1.2;
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }
    .stat-label {
      font-size: 0.75rem;
      color: var(--text-secondary);
      font-weight: 500;
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }

    /* ── Skeleton ───────────────────────────────── */
    .stat-skeleton {
      .skeleton-circle {
        width: 42px; height: 42px;
        border-radius: 10px;
        background: #e2e8f0;
        animation: pulse 1.5s ease-in-out infinite;
      }
      .skeleton-lines {
        display: flex;
        flex-direction: column;
        gap: 6px;
      }
      .skeleton-line {
        height: 12px;
        border-radius: 4px;
        background: #e2e8f0;
        animation: pulse 1.5s ease-in-out infinite;
        &.w60 { width: 60px; height: 16px; }
        &.w40 { width: 48px; }
      }
    }

    @keyframes pulse {
      0%, 100% { opacity: 1; }
      50% { opacity: 0.5; }
    }

    /* ── Grid ───────────────────────────────────── */
    .grid-2 {
      display: grid;
      /* minmax(0,…) lets columns shrink below content width so wide/unbreakable
         content (container names, fingerprints) can't blow the grid out */
      grid-template-columns: minmax(0, 1.2fr) minmax(0, 1fr);
      gap: 20px;
    }

    .dash-card { min-width: 0; }

    .right-stack {
      display: flex;
      flex-direction: column;
      gap: 20px;
      min-width: 0;
    }

    /* ── Card Top Bar ───────────────────────────── */
    .dash-card {
      padding: 0 !important;

      .card-top {
        display: flex;
        justify-content: space-between;
        align-items: center;
        padding: 16px 20px 12px;

        h3 {
          font-size: 0.9rem;
          font-weight: 600;
          color: var(--text-primary);
          margin: 0;
        }
      }
    }

    .btn-sm {
      font-size: 0.8rem !important;
      padding: 0 12px !important;
      height: 32px !important;
      line-height: 32px !important;
      .material-icons {
        font-size: 16px;
        width: 16px;
        height: 16px;
        margin-left: 4px;
      }
    }

    /* ── Service List ───────────────────────────── */
    .service-list {
      padding: 0 20px 16px;
      display: flex;
      flex-direction: column;
    }

    .svc-row {
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 9px 0;
      border-bottom: 1px solid var(--border-card);
      font-size: 0.825rem;
      min-width: 0;

      &:last-child { border-bottom: none; }
    }

    .svc-indicator {
      width: 8px;
      height: 8px;
      border-radius: 50%;
      flex-shrink: 0;

      &.ind-running { background: var(--status-green); box-shadow: 0 0 6px rgba(34, 197, 94, 0.4); }
      &.ind-stopped { background: var(--status-red); }
      &.ind-starting { background: var(--status-orange); }
      &.ind-notinstalled { background: var(--text-muted); }
      &.ind-error { background: var(--status-red); }
    }

    .svc-name {
      font-weight: 600;
      color: var(--text-primary);
      flex-shrink: 0;
      max-width: 140px;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .svc-container {
      flex: 1;
      min-width: 0;
      color: var(--text-muted);
      font-family: 'SF Mono', 'Fira Code', monospace;
      font-size: 0.75rem;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .svc-status { flex-shrink: 0; }

    .svc-status {
      font-size: 0.7rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.04em;

      &.st-running { color: var(--status-green); }
      &.st-stopped { color: var(--status-red); }
      &.st-starting { color: var(--status-orange); }
      &.st-notinstalled { color: var(--text-muted); }
      &.st-error { color: var(--status-red); }
    }

    .card-footer-link {
      padding: 10px 20px;
      border-top: 1px solid var(--border-card);
      text-align: center;

      a {
        font-size: 0.8rem;
        color: var(--accent-indigo);
        text-decoration: none;
        font-weight: 500;
        &:hover { text-decoration: underline; }
      }
    }

    /* ── Empty State ────────────────────────────── */
    .empty-state {
      display: flex;
      flex-direction: column;
      align-items: center;
      gap: 8px;
      padding: 40px 20px;
      color: var(--text-muted);

      .material-icons {
        font-size: 36px;
        width: 36px;
        height: 36px;
        color: #cbd5e1;
      }

      span {
        font-size: 0.875rem;
      }

      .empty-link {
        font-size: 0.8rem;
        color: var(--accent-indigo);
        text-decoration: none;
        font-weight: 500;
        &:hover { text-decoration: underline; }
      }

      &.sm {
        padding: 24px 20px;
        .material-icons { font-size: 24px; width: 24px; height: 24px; }
      }
    }

    /* ── License ────────────────────────────────── */
    .license-row {
      display: flex;
      align-items: center;
      gap: 14px;
      padding: 0 20px 12px;
    }

    .license-badge {
      width: 40px;
      height: 40px;
      border-radius: 10px;
      display: flex;
      align-items: center;
      justify-content: center;
      flex-shrink: 0;

      .material-icons { font-size: 22px; width: 22px; height: 22px; color: white; }

      &.lb-active, &.lb-unlimited { background: linear-gradient(135deg, #22c55e, #16a34a); }
      &.lb-expiring_soon { background: linear-gradient(135deg, #f59e0b, #d97706); }
      &.lb-expired { background: linear-gradient(135deg, #ef4444, #dc2626); }
    }

    .license-info {
      display: flex;
      flex-direction: column;
    }
    .license-plan {
      font-weight: 600;
      font-size: 0.9rem;
      color: var(--text-primary);
    }
    .license-msg {
      font-size: 0.75rem;
      font-weight: 500;
      &.lt-active, &.lt-unlimited { color: var(--status-green); }
      &.lt-expiring_soon { color: var(--status-orange); }
      &.lt-expired { color: var(--status-red); }
    }

    .feature-tags {
      display: flex;
      gap: 8px;
      padding: 0 20px 16px;
      flex-wrap: wrap;
    }

    .ftag {
      display: inline-flex;
      align-items: center;
      gap: 4px;
      font-size: 0.7rem;
      font-weight: 500;
      padding: 4px 10px;
      border-radius: 6px;
      background: #f1f5f9;
      color: var(--text-muted);

      .material-icons {
        font-size: 14px;
        width: 14px;
        height: 14px;
      }

      &.active {
        background: var(--status-green-bg);
        color: #15803d;
      }
    }

    /* ── Machine Row ──────────────────────────── */
    .machine-row {
      display: flex;
      align-items: center;
      gap: 8px;
      padding: 8px 20px 14px;
      font-size: 0.8rem;

      > .material-icons {
        font-size: 16px;
        width: 16px;
        height: 16px;
        color: var(--text-muted);
      }

      .machine-name {
        font-weight: 600;
        color: var(--text-primary);
      }

      .machine-fp {
        font-family: 'SF Mono', 'Fira Code', monospace;
        font-size: 0.7rem;
        color: var(--text-muted);
        background: #f1f5f9;
        padding: 2px 6px;
        border-radius: 4px;
      }
    }

    /* ── Network Card ──────────────────────────── */
    .network-rows {
      padding: 0 20px 16px;
    }

    .net-row {
      display: flex;
      justify-content: space-between;
      align-items: center;
      padding: 8px 0;
      border-bottom: 1px solid var(--border-card);
      font-size: 0.825rem;

      &:last-child { border-bottom: none; }
    }

    .net-label {
      color: var(--text-secondary);
      font-weight: 500;
    }

    .net-value {
      font-weight: 600;
      color: var(--text-primary);

      &.nv-green { color: var(--status-green); }
      &.nv-red { color: var(--status-red); }
      &.nv-orange { color: var(--status-orange); }
    }

    /* ── Quick Actions ──────────────────────────── */
    .actions-grid {
      display: grid;
      grid-template-columns: repeat(4, 1fr);
      gap: 12px;
      padding: 0 20px 20px;
    }

    .action-btn {
      display: flex;
      flex-direction: column;
      align-items: center;
      gap: 8px;
      padding: 16px 8px;
      border: 1px solid var(--border-light);
      border-radius: var(--radius-md);
      background: var(--bg-card);
      cursor: pointer;
      transition: border-color 0.15s ease, background-color 0.15s ease,
                  box-shadow 0.15s ease, transform 0.15s ease;

      span {
        font-size: 0.7rem;
        font-weight: 600;
        color: var(--text-secondary);
      }

      &:hover {
        border-color: var(--accent-indigo);
        background: var(--accent-indigo-light);
        transform: translateY(-1px);
        box-shadow: var(--shadow-md);
      }
    }

    .ab-icon {
      width: 36px;
      height: 36px;
      border-radius: 10px;
      display: flex;
      align-items: center;
      justify-content: center;

      .material-icons {
        font-size: 20px;
        width: 20px;
        height: 20px;
        color: white;
      }

      &.ab-blue { background: linear-gradient(135deg, #3b82f6, #6366f1); }
      &.ab-orange { background: linear-gradient(135deg, #f59e0b, #ef4444); }
      &.ab-purple { background: linear-gradient(135deg, #8b5cf6, #6366f1); }
      &.ab-green { background: linear-gradient(135deg, #22c55e, #14b8a6); }
    }
  `]
})
export class DashboardComponent implements OnInit, OnDestroy {
  private tauri = inject(TauriService);

  systemInfo: SystemInfo | null = null;
  services: ServiceInfo[] = [];
  license: License | null = null;
  licenseStatus: LicenseStatus | null = null;
  daemonRunning = false;
  telemetry: { cpu_percent: number; ram_gb: number; disk_percent: number } | null = null;
  lastBackupTime: string | null = null;

  get cpuClass(): string {
    const v = this.telemetry?.cpu_percent ?? 0;
    return v > 80 ? 'si-red' : v > 50 ? 'si-orange' : 'si-blue';
  }
  get ramClass(): string {
    if (!this.telemetry || !this.systemInfo) return 'si-purple';
    const pct = (this.telemetry.ram_gb / this.systemInfo.total_ram_gb) * 100;
    return pct > 85 ? 'si-red' : pct > 60 ? 'si-orange' : 'si-purple';
  }
  get diskClass(): string {
    const v = this.telemetry?.disk_percent ?? 0;
    return v > 90 ? 'si-red' : v > 70 ? 'si-orange' : 'si-green';
  }
  networkStatus: NetworkStatus | null = null;
  speedTestResult: SpeedTestResult | null = null;
  speedTestRunning = false;

  private refreshSub?: Subscription;
  private networkSub?: Subscription;

  get runningCount(): number {
    return this.services.filter(s => s.status === 'running').length;
  }

  get networkStatValue(): string {
    if (!this.networkStatus) return '—';
    if (!this.networkStatus.connected) return 'Offline';
    return this.networkStatus.latency_ms != null ? `${this.networkStatus.latency_ms} ms` : 'Online';
  }

  get networkStatClass(): string {
    if (!this.networkStatus) return 'si-muted';
    if (!this.networkStatus.connected) return 'si-orange';
    if (this.networkStatus.latency_ms != null && this.networkStatus.latency_ms > 200) return 'si-orange';
    return 'si-green';
  }

  get latencyClass(): string {
    if (!this.networkStatus?.latency_ms) return '';
    if (this.networkStatus.latency_ms < 100) return 'nv-green';
    if (this.networkStatus.latency_ms < 300) return 'nv-orange';
    return 'nv-red';
  }

  get gcpClass(): string {
    if (!this.networkStatus) return '';
    return this.networkStatus.gcp_reachable ? 'nv-green' : 'nv-red';
  }

  get downloadClass(): string {
    if (!this.speedTestResult?.download_mbps) return '';
    if (this.speedTestResult.download_mbps > 10) return 'nv-green';
    if (this.speedTestResult.download_mbps > 2) return 'nv-orange';
    return 'nv-red';
  }

  get uploadClass(): string {
    if (!this.speedTestResult?.upload_mbps) return '';
    if (this.speedTestResult.upload_mbps > 5) return 'nv-green';
    if (this.speedTestResult.upload_mbps > 1) return 'nv-orange';
    return 'nv-red';
  }

  ngOnInit(): void {
    // Load fast data first (license, system info) — renders immediately
    this.loadFastData();
    // Then load slow data (services, daemon status) — renders when ready
    this.loadSlowData();
    this.checkNetwork();

    // Refresh every 30 seconds
    this.refreshSub = interval(30000).subscribe(() => {
      this.loadFastData();
      this.loadSlowData();
    });

    // Network connectivity poll every 60 seconds
    this.networkSub = interval(60000).subscribe(() => {
      this.checkNetwork();
    });
  }

  ngOnDestroy(): void {
    this.refreshSub?.unsubscribe();
    this.networkSub?.unsubscribe();
  }

  /** Fast calls — license + system info + telemetry. */
  private async loadFastData(): Promise<void> {
    const [licenseResult, sysResult, telResult] = await Promise.allSettled([
      this.tauri.invokeSilent<License>('get_license'),
      this.tauri.invokeSilent<SystemInfo>('get_system_info'),
      this.tauri.invokeSilent<{ cpu_percent: number; ram_gb: number; disk_percent: number }>('get_telemetry_snapshot'),
    ]);

    if (licenseResult.status === 'fulfilled' && licenseResult.value) {
      this.license = licenseResult.value;
      this.licenseStatus = getLicenseStatus(licenseResult.value);
    }
    if (sysResult.status === 'fulfilled') {
      this.systemInfo = sysResult.value;
    }
    if (telResult.status === 'fulfilled') {
      this.telemetry = telResult.value;
    }
  }

  /** Slow calls — Docker services + daemon probe + backup history. */
  private async loadSlowData(): Promise<void> {
    const [servicesResult, daemonResult, backupResult] = await Promise.allSettled([
      this.tauri.invokeSilent<ServiceInfo[]>('get_services'),
      this.tauri.invokeSilent<DaemonStatus>('get_daemon_status'),
      this.tauri.invokeSilent<BackupRecord[]>('get_backup_history'),
    ]);

    if (servicesResult.status === 'fulfilled') {
      this.services = servicesResult.value;
    }
    if (daemonResult.status === 'fulfilled') {
      this.daemonRunning = daemonResult.value.running;
    }
    if (backupResult.status === 'fulfilled' && backupResult.value) {
      const completed = backupResult.value
        .filter(b => b.status === 'completed')
        .sort((a, b) => new Date(b.created_at).getTime() - new Date(a.created_at).getTime());
      if (completed.length > 0) {
        this.lastBackupTime = this.timeAgo(new Date(completed[0].created_at));
      }
    }
  }

  private timeAgo(date: Date): string {
    const seconds = Math.floor((Date.now() - date.getTime()) / 1000);
    if (seconds < 60) return 'Just now';
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.floor(hours / 24);
    return `${days}d ago`;
  }

  async runBackup(): Promise<void> {
    try {
      await this.tauri.invoke('start_backup', { type: 'full' });
    } catch (error) {
      // Error handled by TauriService
    }
  }

  async restartServices(): Promise<void> {
    for (const service of this.services) {
      try {
        await this.tauri.invoke('restart_service', { name: service.name });
      } catch (error) {
        // Continue with other services
      }
    }
    await this.loadSlowData();
  }

  async syncConfig(): Promise<void> {
    try {
      await this.tauri.invoke('sync_config_to_cloud');
    } catch (error) {
      // Error handled by TauriService
    }
  }

  private async checkNetwork(): Promise<void> {
    try {
      this.networkStatus = await this.tauri.invokeSilent<NetworkStatus>('check_network');
    } catch {
      // Silently ignore — will show as unknown
    }
  }

  async runSpeedTest(): Promise<void> {
    this.speedTestRunning = true;
    try {
      this.speedTestResult = await this.tauri.invoke<SpeedTestResult>('run_speed_test');
      // Also update network status from speed test result
      if (this.speedTestResult) {
        this.networkStatus = {
          connected: this.speedTestResult.connected,
          latency_ms: this.speedTestResult.latency_ms,
          gcp_reachable: this.speedTestResult.gcp_reachable,
          gcp_latency_ms: this.speedTestResult.gcp_latency_ms,
          checked_at: this.speedTestResult.tested_at
        };
      }
    } catch {
      // Error handled by TauriService
    } finally {
      this.speedTestRunning = false;
    }
  }
}
