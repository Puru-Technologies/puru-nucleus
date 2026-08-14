import { Component, inject, OnInit, OnDestroy } from '@angular/core';
import { RouterOutlet, RouterLink, RouterLinkActive, Router, NavigationEnd } from '@angular/router';
import { filter, map } from 'rxjs/operators';
import { interval, Subscription } from 'rxjs';
import { ConnectionService } from './core/services/connection.service';
import { RemoteTransportService } from './core/services/remote-transport';
import { resetLicenseCache } from './core/guards/init.guard';
import { ToastHostComponent } from './core/components/toast-host.component';
import { PuruLogoComponent } from './core/components/puru-logo.component';
// @ts-ignore
import packageJson from '../../package.json';

interface SystemClockStatus {
  checked: boolean;
  ok: boolean;
  skew_seconds: number;
  tolerance_seconds: number;
  local_time: string;
  server_time: string | null;
  timezone_offset: string;
  source: string | null;
  message: string;
}

interface CommandActivity {
  id: string;
  command_type: string;
  status: string; // pending | executing | completed | failed
  message: string;
  created_at: string;
}

@Component({
  selector: 'app-root',
  standalone: true,
  imports: [
    RouterOutlet,
    RouterLink,
    RouterLinkActive,
    ToastHostComponent,
    PuruLogoComponent,
  ],
  template: `
    @if (showShell) {
      <div class="shell">
        <aside class="sidebar">
          <!-- Brand -->
          <div class="brand">
            <puru-logo variant="normal" theme="dark" [size]="28"></puru-logo>
            <span class="brand-sub">{{ hospitalCode ? ('Nucleus · ' + hospitalCode) : 'Nucleus' }}</span>
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
          @if (clock && clock.checked && !clock.ok && !conn.isRemote()) {
            <div class="clock-banner">
              <span class="material-icons">schedule</span>
              <span class="clock-banner-text">
                {{ clock.message }} (offset {{ clock.timezone_offset }}) —
                sync via Settings → Time &amp; language → “Set time automatically”.
              </span>
              <button class="btn btn-sm" (click)="checkClock()" [disabled]="checkingClock">
                {{ checkingClock ? 'Checking…' : 'Re-check' }}
              </button>
            </div>
          }
          @if (commandBanner) {
            <div class="cmd-banner"
                 [class.running]="isCommandRunning(commandBanner)"
                 [class.done]="commandBanner.status === 'completed'"
                 [class.failed]="commandBanner.status === 'failed'">
              @if (isCommandRunning(commandBanner)) {
                <span class="spinner"></span>
              } @else {
                <span class="material-icons">
                  {{ commandBanner.status === 'completed' ? 'check_circle' : 'error' }}
                </span>
              }
              <span class="cmd-banner-text">
                <strong>{{ commandLabel(commandBanner) }}</strong>
                @if (commandBanner.message) { <span class="cmd-msg">— {{ commandBanner.message }}</span> }
              </span>
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

    .clock-banner {
      display: flex;
      align-items: center;
      gap: 10px;
      padding: 8px 16px;
      background: #fff7ed;
      border-bottom: 1px solid #fed7aa;
      color: #9a3412;
      font-size: 0.8rem;

      .material-icons { font-size: 18px; color: #ea580c; }
      .clock-banner-text { flex: 1; }
      .btn {
        flex-shrink: 0;
        background: #ea580c;
        color: #fff;
        border: none;
      }
    }

    /* ── Cloud command activity banner ──────────── */
    .cmd-banner {
      display: flex;
      align-items: center;
      gap: 14px;
      padding: 16px 22px;
      font-size: 1rem;
      font-weight: 500;
      border-bottom: 1px solid transparent;
      box-shadow: 0 2px 6px rgba(0, 0, 0, 0.05);

      .cmd-banner-text { flex: 1; }
      .cmd-banner-text strong { font-weight: 700; }
      .cmd-msg { color: inherit; opacity: 0.85; font-weight: 400; }
      .material-icons { font-size: 26px; }
      .spinner {
        width: 22px; height: 22px;
        border: 3px solid rgba(0, 158, 251, 0.3);
        border-top-color: var(--brand-blue, #009efb);
        border-radius: 50%;
        animation: spin 0.7s linear infinite;
        flex-shrink: 0;
      }

      &.running { background: #e6f4fe; border-bottom-color: #b6e3fd; color: #075985; }
      &.done    { background: #ecfdf5; border-bottom-color: #a7f3d0; color: #065f46;
                  .material-icons { color: #059669; } }
      &.failed  { background: #fef2f2; border-bottom-color: #fecaca; color: #991b1b;
                  .material-icons { color: #dc2626; } }
    }

    /* ── Brand ──────────────────────────────────── */
    .brand {
      display: flex;
      flex-direction: column;
      align-items: flex-start;
      gap: 7px;
      padding: 22px 20px 24px;
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
      font-size: 0.6rem;
      color: #64748b;
      letter-spacing: 0.14em;
      text-transform: uppercase;
      font-weight: 600;
      padding-left: 3px;
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
  clock: SystemClockStatus | null = null;
  checkingClock = false;
  commandBanner: CommandActivity | null = null;

  conn = inject(ConnectionService);
  private remote = inject(RemoteTransportService);
  private router = inject(Router);
  private unreadSub?: Subscription;
  private healthSub?: Subscription;
  private commandSub?: Subscription;
  private commandProcSub?: Subscription;
  private heartbeatSub?: Subscription;
  private commandDismissTimer?: any;

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
    this.checkClock();
    // Surface cloud-command activity as a top banner on every screen.
    this.refreshCommands();
    this.commandSub = interval(4_000).subscribe(() => this.refreshCommands());
    // Execute pending commands locally when no daemon is running, so commands
    // work whenever the app is open (the daemon no-ops this when it's up).
    this.processCommands();
    this.commandProcSub = interval(6_000).subscribe(() => this.processCommands());
    // Status heartbeat: mark online immediately on startup and every minute, so
    // the cloud dashboard shows this machine up while the app is open.
    this.sendHeartbeat();
    this.heartbeatSub = interval(60_000).subscribe(() => this.sendHeartbeat());
  }

  /** Push an online + telemetry heartbeat to the cloud. */
  private async sendHeartbeat(): Promise<void> {
    if (this.conn.isRemote()) return;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('send_status_heartbeat');
    } catch {
      // offline — nothing to report
    }
  }

  /** Process pending cloud commands locally (no-op if a daemon is running). */
  private async processCommands(): Promise<void> {
    if (this.conn.isRemote()) return;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('process_pending_commands');
    } catch {
      // offline / unsupported — the daemon (if any) handles it
    }
  }

  /** Poll recent cloud commands and drive the top activity banner. */
  private async refreshCommands(): Promise<void> {
    let list: CommandActivity[] = [];
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      list = await invoke<CommandActivity[]>('get_command_activity');
    } catch {
      return; // offline / unsupported (e.g. remote) — leave the banner as-is
    }
    const newest = list && list.length ? list[0] : null;
    if (!newest) return;

    if (this.isCommandRunning(newest)) {
      // In-flight: show and cancel any pending auto-dismiss.
      clearTimeout(this.commandDismissTimer);
      this.commandDismissTimer = undefined;
      this.commandBanner = newest;
      return;
    }

    // Finished: show the result if we were tracking it, or if it just completed,
    // then auto-dismiss after a few seconds.
    const wasTracking = this.commandBanner?.id === newest.id;
    const ageSecs = this.commandAgeSecs(newest);
    if (wasTracking || ageSecs <= 20) {
      this.commandBanner = newest;
      if (!this.commandDismissTimer) {
        this.commandDismissTimer = setTimeout(() => {
          this.commandBanner = null;
          this.commandDismissTimer = undefined;
        }, 8_000);
      }
    }
  }

  isCommandRunning(c: CommandActivity): boolean {
    return c.status === 'pending' || c.status === 'executing';
  }

  /** Human label for the command + its state. */
  commandLabel(c: CommandActivity): string {
    const names: Record<string, string> = {
      restart_service: 'Restart service',
      stop_service: 'Stop service',
      start_service: 'Start service',
      trigger_backup: 'Backup',
      restart_all: 'Restart all services',
    };
    const base = names[c.command_type] || c.command_type || 'Command';
    switch (c.status) {
      case 'pending': return `Queued: ${base}`;
      case 'executing': return `${base}…`;
      case 'completed': return `${base} — done`;
      case 'failed': return `${base} — failed`;
      default: return base;
    }
  }

  private commandAgeSecs(c: CommandActivity): number {
    const t = c.created_at ? Date.parse(c.created_at) : NaN;
    return isNaN(t) ? 9999 : (Date.now() - t) / 1000;
  }

  /** Verify the local system clock is in sync — a skew breaks cloud sign-in. */
  async checkClock(): Promise<void> {
    if (this.conn.isRemote()) return; // the local clock only gates local GCP auth
    this.checkingClock = true;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      this.clock = await invoke<SystemClockStatus>('check_system_clock');
    } catch {
      this.clock = null; // can't determine → don't nag
    } finally {
      this.checkingClock = false;
    }
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
    this.commandSub?.unsubscribe();
    this.commandProcSub?.unsubscribe();
    this.heartbeatSub?.unsubscribe();
    clearTimeout(this.commandDismissTimer);
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
