import { Component, inject } from '@angular/core';
import { RouterOutlet, RouterLink, RouterLinkActive, Router, NavigationEnd } from '@angular/router';
import { MatSidenavModule } from '@angular/material/sidenav';
import { MatListModule } from '@angular/material/list';
import { MatIconModule } from '@angular/material/icon';
import { filter, map } from 'rxjs/operators';

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
              <span class="brand-sub">Puru Technologies</span>
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
            <div class="version">v0.1.0</div>
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
  `]
})
export class AppComponent {
  title = 'puru-nucleus';
  showShell = true;

  private router = inject(Router);

  constructor() {
    // Hide sidebar on activation page
    this.router.events.pipe(
      filter((e): e is NavigationEnd => e instanceof NavigationEnd),
      map(e => e.urlAfterRedirects || e.url)
    ).subscribe(url => {
      this.showShell = !url.startsWith('/activation');
    });
  }
}
