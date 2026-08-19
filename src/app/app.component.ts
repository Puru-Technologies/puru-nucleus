import { Component, HostListener, inject, OnInit, OnDestroy } from '@angular/core';
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
        <header class="topbar">
          <div class="tb-brand">
            <puru-logo variant="normal" [size]="22"></puru-logo>
            <div class="bdiv"></div>
            <span class="hname">{{ hospitalName || hospitalCode || 'Puru DC' }}<span class="hdot">.</span></span>
          </div>

          <nav class="topnav">
            @if (navRevealed) {
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
            <a class="nav-item" routerLink="/performance" routerLinkActive="active">
              <span class="material-icons">speed</span>
              <span>Performance</span>
            </a>
            <a class="nav-item" routerLink="/updates" routerLinkActive="active">
              <span class="material-icons">system_update</span>
              <span>Updates</span>
            </a>
            }
          </nav>

          <div class="topright">
            @if (navRevealed) {
            <div class="adv">
              <button class="adv-btn" (click)="toggleAdv($event)" title="Advanced &amp; support screens">
                <span class="material-icons">{{ showConfigNav ? 'lock_open' : 'lock' }}</span>
                <span>Advanced</span>
                <span class="material-icons caret">expand_more</span>
              </button>
              @if (advOpen) {
                <div class="adv-menu" (click)="$event.stopPropagation()">
                  @if (showConfigNav) {
                    <a routerLink="/settings" routerLinkActive="active" (click)="advOpen = false"><span class="material-icons">tune</span>Settings</a>
                    <a routerLink="/setup" routerLinkActive="active" (click)="advOpen = false"><span class="material-icons">build</span>Setup</a>
                    @if (!isNative) {
                      <a routerLink="/compose" routerLinkActive="active" (click)="advOpen = false"><span class="material-icons">description</span>Compose</a>
                    }
                    <a routerLink="/master-data" routerLinkActive="active" (click)="advOpen = false"><span class="material-icons">dataset</span>Master data</a>
                    <a routerLink="/remote-shell" routerLinkActive="active" (click)="advOpen = false"><span class="material-icons">terminal</span>Shell</a>
                    @if (setupCompleted && !productionMode) {
                      <div class="adv-div"></div>
                      <button (click)="configUnlocked = false; advOpen = false"><span class="material-icons">lock</span>Lock advanced screens</button>
                    }
                  } @else if (canUnlockConfig) {
                    <button (click)="configUnlocked = true"><span class="material-icons">lock_open</span>Unlock config screens</button>
                  } @else {
                    <div class="adv-empty">No advanced screens available.</div>
                  }
                </div>
              }
            </div>
            }
            <button class="conn-chip" [class.remote]="conn.isRemote()" (click)="goConnect()"
                    [title]="conn.isRemote() ? 'Connected to a remote daemon — click to manage' : 'Managing this machine — click to connect to a remote server'">
              <span class="conn-dot"
                    [class.up]="conn.isRemote() && conn.health()?.reachable"
                    [class.down]="conn.isRemote() && conn.health() && !conn.health()?.reachable"></span>
              @if (conn.isRemote()) {
                <span class="conn-text">{{ conn.activeServer()?.name }}</span>
              } @else {
                <span class="conn-text">Local</span>
              }
              <span class="material-icons">swap_horiz</span>
            </button>
            <button class="nav-toggle" [class.on]="navRevealed" (click)="toggleNav($event)"
                    [title]="navRevealed ? 'Hide menu' : 'Show menu'">
              <span class="material-icons">{{ navRevealed ? 'close' : 'menu' }}</span>
            </button>
          </div>
        </header>

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
      flex-direction: column;
      height: 100vh;
      overflow: hidden;
    }

    /* ── Top header (replaces the old sidebar) ── */
    .topbar {
      display: flex;
      align-items: center;
      gap: 20px;
      height: 60px;
      padding: 0 20px;
      background: var(--card-bg, #fff);
      border-bottom: 1px solid var(--border);
      flex-shrink: 0;
      z-index: 30;
    }
    .topbar .tb-brand { display: flex; flex-direction: row; align-items: center; gap: 13px; flex-shrink: 0; }
    .topbar .wm { font-size: 22px; font-weight: 700; letter-spacing: -0.6px; color: var(--text-primary); white-space: nowrap; line-height: 1; }
    .topbar .wm-accent { color: var(--brand-blue, #009efb); }
    .topbar .bdiv { width: 1px; height: 24px; background: var(--border-light, #e2e8f0); }
    .topbar .hname { font-family: var(--font-brand); font-size: 20px; font-weight: 700; color: var(--text-primary); white-space: nowrap; letter-spacing: -0.01em; }
    .topbar .hdot { color: var(--brand-red, #e83a3a); font-weight: 700; }
    .topnav { display: flex; align-items: center; gap: 2px; flex: 1; overflow-x: auto; }
    .topnav::-webkit-scrollbar { height: 0; }
    .topnav a { display: flex; align-items: center; gap: 7px; padding: 8px 12px; border-radius: 8px; font-size: 14px; color: var(--text-secondary); text-decoration: none; white-space: nowrap; }
    .topnav a .material-icons { font-size: 18px; }
    .topnav a:hover { background: var(--bg-hover, #f4f7fb); color: var(--text-primary); }
    .topnav a.active { background: #e6f4fe; color: #0a6cad; font-weight: 500; }
    .topnav .unread-badge { background: var(--status-red, #d94339); color: #fff; font-size: 10px; font-weight: 700; padding: 0 5px; border-radius: 10px; min-width: 16px; text-align: center; line-height: 16px; }
    .topright { display: flex; align-items: center; gap: 10px; flex-shrink: 0; }
    .adv { position: relative; }
    .adv-btn { display: flex; align-items: center; gap: 6px; padding: 7px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--card-bg, #fff); cursor: pointer; font-size: 13.5px; color: var(--text-primary); }
    .adv-btn .material-icons { font-size: 16px; } .adv-btn .caret { color: var(--text-muted); font-size: 18px; }
    .adv-btn:hover { border-color: var(--brand-blue, #009efb); }
    .adv-menu { position: absolute; top: calc(100% + 6px); right: 0; z-index: 50; width: 212px; background: var(--card-bg, #fff); border: 1px solid var(--border); border-radius: 11px; padding: 6px; box-shadow: 0 14px 40px rgba(2,12,27,.18); }
    .adv-menu a, .adv-menu button { display: flex; width: 100%; align-items: center; gap: 10px; padding: 9px 10px; border: 0; background: none; border-radius: 7px; font-size: 13.5px; color: var(--text-primary); cursor: pointer; text-align: left; text-decoration: none; font-family: inherit; }
    .adv-menu a .material-icons, .adv-menu button .material-icons { font-size: 17px; color: var(--text-secondary); }
    .adv-menu a:hover, .adv-menu button:hover { background: var(--bg-hover, #f4f7fb); }
    .adv-menu a.active { background: #e6f4fe; color: #0a6cad; }
    .adv-div { height: 1px; background: var(--border); margin: 5px 4px; }
    .adv-empty { padding: 10px; font-size: 13px; color: var(--text-muted); }
    .topright .conn-chip { display: flex; align-items: center; gap: 7px; padding: 7px 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--card-bg, #fff); color: var(--text-secondary); cursor: pointer; font-size: 13px; }
    .topright .conn-chip:hover { border-color: var(--brand-blue, #009efb); }
    .topright .conn-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--text-muted); flex-shrink: 0; }
    .topright .conn-dot.up { background: var(--status-green, #149a63); } .topright .conn-dot.down { background: var(--status-red, #d94339); }
    .topright .conn-chip .material-icons { font-size: 16px; }
    .topright .conn-ver { color: var(--text-muted); margin-left: 4px; font-size: 11px; }
    .nav-toggle { width: 38px; height: 38px; border-radius: 9px; border: 1px solid var(--border); background: var(--card-bg, #fff); cursor: pointer; color: var(--text-secondary); display: flex; align-items: center; justify-content: center; }
    .nav-toggle:hover { border-color: var(--brand-blue, #009efb); color: var(--text-primary); }
    .nav-toggle.on { background: #e6f4fe; border-color: var(--brand-blue, #009efb); color: #0a6cad; }
    .nav-toggle .material-icons { font-size: 20px; }

    .content {
      flex: 1;
      min-height: 0;
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
    .nav-section-row {
      display: flex;
      align-items: center;
      justify-content: space-between;
    }
    .nav-lock {
      background: none; border: none; cursor: pointer; color: #475569;
      display: inline-flex; padding: 0; line-height: 1;
    }
    .nav-lock .material-icons { font-size: 15px; }
    .nav-lock:hover { color: #94a3b8; }
    /* The "unlock configuration" button reuses .nav-item; make it button-shaped. */
    .nav-unlock {
      width: 100%; background: none; border: none; text-align: left;
      cursor: pointer; font-family: inherit; margin-top: 8px;
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
  title = 'puru-dc';
  version = packageJson.version;
  hospitalCode = '';
  hospitalName = '';
  advOpen = false;
  /** The page navigation is hidden by default (operator view) and only shown
   *  when the menu button is clicked — regular users never need the nav. */
  navRevealed = false;
  showShell = true;
  unreadCount = 0;
  isAdmin = true;      // assume true until checked, so the banner doesn't flash
  elevating = false;
  clock: SystemClockStatus | null = null;
  checkingClock = false;
  commandBanner: CommandActivity | null = null;

  // ── Config-screen visibility ────────────────────────────────────────────────
  isNative = false;
  productionMode = false;
  setupCompleted = false;
  /** Session-only: reveals the config screens after setup via a button click. */
  configUnlocked = false;

  /** Show the Configuration nav section? Hidden in production; before setup it's
   *  always available (Setup is needed); after setup it's hidden until unlocked. */
  get showConfigNav(): boolean {
    if (this.productionMode) return false;
    if (!this.setupCompleted) return true;
    return this.configUnlocked;
  }
  /** Offer the "unlock configuration" button (post-setup, non-prod, still locked). */
  get canUnlockConfig(): boolean {
    return !this.productionMode && this.setupCompleted && !this.configUnlocked;
  }

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

  /** Toggle the Advanced menu in the top bar. */
  toggleAdv(e: Event): void { e.stopPropagation(); this.advOpen = !this.advOpen; }

  /** Reveal / hide the page navigation (hidden by default for operators). */
  toggleNav(e: Event): void { e.stopPropagation(); this.navRevealed = !this.navRevealed; if (!this.navRevealed) this.advOpen = false; }

  @HostListener('document:click')
  closeAdv(): void { this.advOpen = false; }

  private async loadHospitalCode(): Promise<void> {
    try {
      type ShellConfig = { hospital_code: string; deployment_mode?: string; production_mode?: boolean; setup_completed?: boolean };
      const config = this.conn.isRemote()
        ? await this.remote.execute<ShellConfig>('get_config')
        : await (await import('@tauri-apps/api/core')).invoke<ShellConfig>('get_config');
      this.isNative = config?.deployment_mode === 'native';
      this.productionMode = !!config?.production_mode;
      this.setupCompleted = !!config?.setup_completed;
      if (config?.hospital_code) {
        this.hospitalCode = config.hospital_code;
        // Update window title (the GUI is still a local Tauri window)
        const { getCurrentWindow } = await import('@tauri-apps/api/window');
        await getCurrentWindow().setTitle(`Puru DC — ${config.hospital_code}`);
      }
      // Hospital display name for the header (best-effort; falls back to code).
      try {
        const lic = this.conn.isRemote()
          ? await this.remote.execute<{ hospital_name?: string }>('get_license')
          : await (await import('@tauri-apps/api/core')).invoke<{ hospital_name?: string }>('get_license');
        if (lic?.hospital_name) this.hospitalName = lic.hospital_name;
      } catch { /* non-fatal */ }
    } catch {
      // Non-fatal
    }
  }
}
