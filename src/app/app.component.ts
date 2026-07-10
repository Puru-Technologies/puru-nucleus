import { Component, inject, OnInit, OnDestroy } from '@angular/core';
import { RouterOutlet, RouterLink, RouterLinkActive, Router, NavigationEnd } from '@angular/router';
import { filter, map } from 'rxjs/operators';
import { interval, Subscription } from 'rxjs';
import { ConnectionService } from './core/services/connection.service';
import { RemoteTransportService } from './core/services/remote-transport';
import { resetLicenseCache } from './core/guards/init.guard';
import { ToastHostComponent } from './core/components/toast-host.component';
// @ts-ignore
import packageJson from '../../package.json';

@Component({
  selector: 'app-root',
  standalone: true,
  imports: [
    RouterOutlet,
    RouterLink,
    RouterLinkActive,
    ToastHostComponent,
  ],
  template: `
    @if (showShell) {
      <div class="shell">
        <aside class="sidebar">
          <!-- Brand -->
          <div class="brand">
            <div class="brand-icon">
              <span class="material-icons">hub</span>
            </div>
            <div class="brand-text">
              <span class="brand-name">Nucleus</span>
              <span class="brand-sub">{{ hospitalCode || 'Puru Labs Private Limited' }}</span>
            </div>
          </div>

          <!-- Navigation -->
          <nav class="nav">
            <span class="nav-section">Menu</span>

            <a class="nav-item" routerLink="/dashboard" routerLinkActive="active">
              <span class="material-icons">grid_view</span>
              <span>Dashboard</span>
            </a>
            <a class="nav-item" routerLink="/services" routerLinkActive="active">
              <span class="material-icons">dns</span>
              <span>Services</span>
            </a>
            <a class="nav-item" routerLink="/logs" routerLinkActive="active">
              <span class="material-icons">description</span>
              <span>Logs</span>
            </a>
            <a class="nav-item" routerLink="/backups" routerLinkActive="active">
              <span class="material-icons">backup</span>
              <span>Backups</span>
            </a>
            <a class="nav-item" routerLink="/alerts" routerLinkActive="active">
              <span class="material-icons">notifications_none</span>
              <span>Alerts</span>
            </a>
            <a class="nav-item" routerLink="/inbox" routerLinkActive="active">
              <span class="material-icons">mail</span>
              <span>Inbox</span>
              @if (unreadCount > 0) {
                <span class="unread-badge">{{ unreadCount > 99 ? '99+' : unreadCount }}</span>
              }
            </a>
            <a class="nav-item" routerLink="/updates" routerLinkActive="active">
              <span class="material-icons">system_update</span>
              <span>Updates</span>
            </a>

            <span class="nav-section">System</span>

            <a class="nav-item" routerLink="/settings" routerLinkActive="active">
              <span class="material-icons">tune</span>
              <span>Settings</span>
            </a>
            <a class="nav-item" routerLink="/compose" routerLinkActive="active">
              <span class="material-icons">description</span>
              <span>Compose</span>
            </a>
            <a class="nav-item" routerLink="/setup" routerLinkActive="active">
              <span class="material-icons">build</span>
              <span>Setup</span>
            </a>
            <a class="nav-item" routerLink="/master-data" routerLinkActive="active">
              <span class="material-icons">dataset</span>
              <span>Master Data</span>
            </a>
            <a class="nav-item" routerLink="/remote-shell" routerLinkActive="active">
              <span class="material-icons">terminal</span>
              <span>Shell</span>
            </a>
          </nav>

          <!-- Footer -->
          <div class="sidebar-footer">
            <button class="conn-chip" [class.remote]="conn.isRemote()" (click)="goConnect()"
                    [title]="conn.isRemote() ? 'Connected to a remote daemon — click to manage' : 'Managing this machine — click to connect to a remote server'">
              <span class="conn-dot"
                    [class.up]="conn.isRemote() && conn.health()?.reachable"
                    [class.down]="conn.isRemote() && conn.health() && !conn.health()?.reachable"></span>
              @if (conn.isRemote()) {
                <span class="conn-text">{{ conn.activeServer()?.name }}<span class="conn-ver">v{{ conn.health()?.version || '?' }}</span></span>
              } @else {
                <span class="conn-text">Local</span>
              }
              <span class="material-icons">swap_horiz</span>
            </button>
            <div class="version">v{{ version }}</div>
          </div>
        </aside>

        <main class="content">
          @if (!isAdmin && !conn.isRemote()) {
            <div class="admin-banner">
              <span class="material-icons">shield</span>
              <span class="admin-banner-text">
                Not running as administrator — installing services, MySQL/RabbitMQ, and certificates need elevation.
              </span>
              <button class="btn btn-primary btn-sm" (click)="restartAsAdmin()" [disabled]="elevating">
                {{ elevating ? 'Restarting…' : 'Restart as Admin' }}
              </button>
            </div>
          }
          <router-outlet></router-outlet>
        </main>
      </div>
    } @else {
      <!-- Full-screen layout for activation page -->
      <router-outlet></router-outlet>
    }

    <app-toast-host></app-toast-host>
  `,
  styles: [`
    .shell {
      display: flex;
      height: 100vh;
      overflow: hidden;
    }

    .sidebar {
      width: 240px;
      flex-shrink: 0;
      background: var(--bg-sidebar);
      display: flex;
      flex-direction: column;
      overflow-y: auto;
    }

    .content {
      flex: 1;
      height: 100vh;
      overflow-y: auto;
      background: var(--bg-primary);
    }

    .admin-banner {
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 8px 16px;
      background: #fff7ed;
      border-bottom: 1px solid #fed7aa;
      color: #9a3412;
      font-size: 0.8rem;

      .material-icons { font-size: 18px; color: #ea580c; }
      .admin-banner-text { flex: 1; }
      .btn { flex-shrink: 0; }
    }

    /* ── Brand ──────────────────────────────────── */
    .brand {
      display: flex;
      align-items: center;
      gap: 12px;
      padding: 20px 20px 24px;
    }

    .brand-icon {
      width: 36px;
      height: 36px;
      border-radius: 10px;
      background: linear-gradient(135deg, #6366f1, #8b5cf6);
      display: flex;
      align-items: center;
      justify-content: center;
      flex-shrink: 0;

      .material-icons {
        color: white;
        font-size: 20px;
      }
    }

    .brand-text {
      display: flex;
      flex-direction: column;
    }

    .brand-name {
      font-size: 1.1rem;
      font-weight: 700;
      color: #f1f5f9;
      letter-spacing: -0.01em;
    }

    .brand-sub {
      font-size: 0.65rem;
      color: #64748b;
      letter-spacing: 0.02em;
      text-transform: uppercase;
      font-weight: 500;
    }

    /* ── Navigation ─────────────────────────────── */
    .nav {
      flex: 1;
      padding: 0 12px;
      display: flex;
      flex-direction: column;
    }

    .nav-section {
      font-size: 0.65rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.08em;
      color: #475569;
      padding: 16px 12px 8px;
    }

    .nav-item {
      display: flex;
      align-items: center;
      gap: 12px;
      padding: 10px 12px;
      border-radius: 8px;
      color: #94a3b8;
      text-decoration: none;
      font-size: 0.875rem;
      font-weight: 500;
      transition: background-color 0.15s ease, color 0.15s ease;
      margin-bottom: 2px;

      .material-icons {
        font-size: 20px;
        color: inherit;
      }

      &:hover {
        background: var(--bg-sidebar-hover);
        color: #e2e8f0;
      }

      &.active {
        background: var(--bg-sidebar-active);
        color: #a5b4fc;

        .material-icons {
          color: #818cf8;
        }
      }
    }

    .unread-badge {
      background: #ef4444;
      color: white;
      font-size: 0.65rem;
      font-weight: 700;
      min-width: 18px;
      height: 18px;
      border-radius: 9px;
      display: inline-flex;
      align-items: center;
      justify-content: center;
      padding: 0 5px;
      margin-left: auto;
      line-height: 1;
    }

    /* ── Footer ─────────────────────────────────── */
    .sidebar-footer {
      padding: 16px 20px;
      border-top: 1px solid rgba(255, 255, 255, 0.06);
    }

    .version {
      font-size: 0.7rem;
      color: #475569;
      font-weight: 500;
    }

    /* ── Connection chip ────────────────────────── */
    .conn-chip {
      display: flex;
      align-items: center;
      gap: 8px;
      width: 100%;
      padding: 8px 10px;
      margin-bottom: 10px;
      background: rgba(255, 255, 255, 0.04);
      border: 1px solid rgba(255, 255, 255, 0.08);
      border-radius: 8px;
      color: #cbd5e1;
      font-size: 0.75rem;
      font-weight: 500;
      cursor: pointer;
      transition: background 0.15s ease;

      &:hover { background: rgba(255, 255, 255, 0.08); }

      &.remote {
        border-color: rgba(99, 102, 241, 0.4);
        color: #c7d2fe;
      }

      .material-icons {
        font-size: 16px;
        margin-left: auto;
        color: #64748b;
      }
    }

    .conn-dot {
      width: 8px;
      height: 8px;
      border-radius: 50%;
      background: #64748b;
      flex-shrink: 0;

      &.up { background: #22c55e; }
      &.down { background: #ef4444; }
    }

    .conn-text {
      display: flex;
      align-items: center;
      gap: 6px;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .conn-ver {
      font-size: 0.65rem;
      color: #64748b;
    }
  `]
})
export class AppComponent implements OnInit, OnDestroy {
  title = 'puru-nucleus';
  version = packageJson.version;
  hospitalCode = '';
  showShell = true;
  unreadCount = 0;
  isAdmin = true;      // assume true until checked, so the banner doesn't flash
  elevating = false;

  conn = inject(ConnectionService);
  private remote = inject(RemoteTransportService);
  private router = inject(Router);
  private unreadSub?: Subscription;
  private healthSub?: Subscription;

  constructor() {
    // Hide sidebar on activation + connect pages (full-screen layouts)
    this.router.events.pipe(
      filter((e): e is NavigationEnd => e instanceof NavigationEnd),
      map(e => e.urlAfterRedirects || e.url)
    ).subscribe(url => {
      this.showShell = !url.startsWith('/activation') && !url.startsWith('/connect');
    });

    // Re-evaluate identity + license whenever the connection changes.
    this.conn.onChange(() => {
      resetLicenseCache();
      this.hospitalCode = '';
      this.loadHospitalCode();
      this.refreshHealth();
    });

    // Load hospital code for display
    this.loadHospitalCode();
  }

  ngOnInit(): void {
    this.loadUnreadCount();
    this.unreadSub = interval(60_000).subscribe(() => this.loadUnreadCount());
    // Keep the connection indicator live when remote.
    this.refreshHealth();
    this.healthSub = interval(30_000).subscribe(() => this.refreshHealth());
    this.checkElevation();
  }

  /** Detect whether the local app is elevated (Windows admin). */
  private async checkElevation(): Promise<void> {
    if (this.conn.isRemote()) return; // only relevant for the local machine
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      this.isAdmin = await invoke<boolean>('is_elevated');
    } catch {
      this.isAdmin = true; // can't determine → don't nag
    }
  }

  /** Relaunch elevated via UAC; on success the app exits and reopens as admin. */
  async restartAsAdmin(): Promise<void> {
    this.elevating = true;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('restart_as_admin');
      // Success → this instance is exiting; the elevated one is starting.
    } catch {
      // UAC cancelled or failed — stay running.
      this.elevating = false;
    }
  }

  ngOnDestroy(): void {
    this.unreadSub?.unsubscribe();
    this.healthSub?.unsubscribe();
  }

  goConnect(): void {
    this.router.navigate(['/connect']);
  }

  private async refreshHealth(): Promise<void> {
    const server = this.conn.activeServer();
    if (!this.conn.isRemote() || !server) {
      this.conn.setHealth(null);
      return;
    }
    try {
      const h = await this.conn.testConnection(server);
      this.conn.setHealth({
        reachable: true,
        version: h.version,
        dockerConnected: h.docker_connected,
        checkedAt: new Date().toISOString(),
      });
    } catch {
      this.conn.setHealth({ reachable: false, checkedAt: new Date().toISOString() });
    }
  }

  private async loadUnreadCount(): Promise<void> {
    // No REST route for unread count — skip in remote mode.
    if (this.conn.isRemote()) {
      this.unreadCount = 0;
      return;
    }
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      this.unreadCount = await invoke<number>('get_unread_count');
    } catch {
      // Non-fatal
    }
  }

  private async loadHospitalCode(): Promise<void> {
    try {
      const config = this.conn.isRemote()
        ? await this.remote.execute<{ hospital_code: string }>('get_config')
        : await (await import('@tauri-apps/api/core')).invoke<{ hospital_code: string }>('get_config');
      if (config?.hospital_code) {
        this.hospitalCode = config.hospital_code;
        // Update window title (the GUI is still a local Tauri window)
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        await getCurrentWindow().setTitle(`Puru Nucleus — ${config.hospital_code}`);
      }
    } catch {
      // Non-fatal
    }
  }
}
