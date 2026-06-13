import { Component, inject, OnInit, OnDestroy } from '@angular/core';
import { RouterOutlet, RouterLink, RouterLinkActive, Router, NavigationEnd } from '@angular/router';
import { MatSidenavModule } from '@angular/material/sidenav';
import { MatListModule } from '@angular/material/list';
import { MatIconModule } from '@angular/material/icon';
import { filter, map } from 'rxjs/operators';
import { interval, Subscription } from 'rxjs';
import { ConnectionService } from './core/services/connection.service';
import { RemoteTransportService } from './core/services/remote-transport';
import { resetLicenseCache } from './core/guards/init.guard';
// @ts-ignore
import packageJson from '../../package.json';

@Component({
  selector: 'app-root',
  standalone: true,
  imports: [
    RouterOutlet,
    RouterLink,
    RouterLinkActive,
    MatSidenavModule,
    MatListModule,
    MatIconModule,
  ],
  template: `
    @if (showShell) {
      <mat-sidenav-container class="shell">
        <mat-sidenav mode="side" opened class="sidebar">
          <!-- Brand -->
          <div class="brand">
            <div class="brand-icon">
              <mat-icon>hub</mat-icon>
            </div>
            <div class="brand-text">
              <span class="brand-name">Nucleus</span>
              <span class="brand-sub">{{ hospitalCode || 'Puru Technologies' }}</span>
            </div>
          </div>

          <!-- Navigation -->
          <nav class="nav">
            <span class="nav-section">Menu</span>

            <a class="nav-item" routerLink="/dashboard" routerLinkActive="active">
              <mat-icon>grid_view</mat-icon>
              <span>Dashboard</span>
            </a>
            <a class="nav-item" routerLink="/services" routerLinkActive="active">
              <mat-icon>dns</mat-icon>
              <span>Services</span>
            </a>
            <a class="nav-item" routerLink="/logs" routerLinkActive="active">
              <mat-icon>description</mat-icon>
              <span>Logs</span>
            </a>
            <a class="nav-item" routerLink="/backups" routerLinkActive="active">
              <mat-icon>backup</mat-icon>
              <span>Backups</span>
            </a>
            <a class="nav-item" routerLink="/alerts" routerLinkActive="active">
              <mat-icon>notifications_none</mat-icon>
              <span>Alerts</span>
            </a>
            <a class="nav-item" routerLink="/inbox" routerLinkActive="active">
              <mat-icon>mail</mat-icon>
              <span>Inbox</span>
              @if (unreadCount > 0) {
                <span class="unread-badge">{{ unreadCount > 99 ? '99+' : unreadCount }}</span>
              }
            </a>
            <a class="nav-item" routerLink="/updates" routerLinkActive="active">
              <mat-icon>system_update</mat-icon>
              <span>Updates</span>
            </a>

            <span class="nav-section">System</span>

            <a class="nav-item" routerLink="/settings" routerLinkActive="active">
              <mat-icon>tune</mat-icon>
              <span>Settings</span>
            </a>
            <a class="nav-item" routerLink="/compose" routerLinkActive="active">
              <mat-icon>description</mat-icon>
              <span>Compose</span>
            </a>
            <a class="nav-item" routerLink="/setup" routerLinkActive="active">
              <mat-icon>build</mat-icon>
              <span>Setup</span>
            </a>
            <a class="nav-item" routerLink="/remote-shell" routerLinkActive="active">
              <mat-icon>terminal</mat-icon>
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
              <mat-icon>swap_horiz</mat-icon>
            </button>
            <div class="version">v{{ version }}</div>
          </div>
        </mat-sidenav>

        <mat-sidenav-content class="content">
          <router-outlet></router-outlet>
        </mat-sidenav-content>
      </mat-sidenav-container>
    } @else {
      <!-- Full-screen layout for activation page -->
      <router-outlet></router-outlet>
    }
  `,
  styles: [`
    .shell {
      height: 100vh;
    }

    .sidebar {
      width: 240px;
      background: var(--bg-sidebar);
      border-right: none !important;
      display: flex;
      flex-direction: column;
    }

    .content {
      background: var(--bg-primary);
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

      mat-icon {
        color: white;
        font-size: 20px;
        width: 20px;
        height: 20px;
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
      transition: all 0.15s ease;
      margin-bottom: 2px;

      mat-icon {
        font-size: 20px;
        width: 20px;
        height: 20px;
        color: inherit;
      }

      &:hover {
        background: var(--bg-sidebar-hover);
        color: #e2e8f0;
      }

      &.active {
        background: var(--bg-sidebar-active);
        color: #a5b4fc;

        mat-icon {
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

      mat-icon {
        font-size: 16px;
        width: 16px;
        height: 16px;
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
