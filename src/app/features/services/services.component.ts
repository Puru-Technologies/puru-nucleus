import { Component, OnInit, OnDestroy, HostListener, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { RouterLink } from '@angular/router';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { TauriService, ServiceInfo, ProcessInfo } from '../../core/services/tauri.service';
import { NotificationService } from '../../core/services/notification.service';
import { PuruProgressComponent } from '../../core/components/puru-progress.component';

/** Live JAR update/download progress emitted by the backend (`jar-update-progress`). */
interface JarUpdateProgress {
  service: string;
  phase: 'downloading' | 'staged' | 'stopping' | 'starting' | 'done' | 'error';
  message: string;
  downloaded: number;
  total: number;
  percent: number;
  index: number;
  count: number;
}

type ProgressBarState = 'active' | 'complete' | 'hold' | 'fail' | 'indeterminate';

interface JarUpdateCheck {
  service: string;
  current_sha: string;
  latest_sha: string;
  latest_built_at: string;
  update_available: boolean;
}

interface StagedUpdate {
  service: string;
  short_sha: string;
  built_at: string;
  size_mb: number;
  staged_path: string;
}

/** A single JAR recorded in the manifest — currently-active, pending, or a
 *  previous version retained for rollback. */
interface JarEntry {
  file: string;
  short_sha: string;
  built_at: string;
  java_version: string;
  size_mb: number;
  downloaded_at: string;
}

/** Per-service JAR manifest snapshot returned by `get_jar_manifest`. */
interface JarManifestView {
  service: string;
  active: JarEntry | null;
  pending: JarEntry | null;
  history: JarEntry[];
  updated_at: string;
}

/** A process still holding a service's active JAR (Restart Manager output).
 *  Empty on non-Windows platforms. */
interface LockingProcess {
  pid: number;
  name: string;
}

/** Per-service state for the split update flow: identify → download → apply. */
type UpdatePhase =
  | 'checking' | 'up-to-date' | 'available'
  | 'downloading' | 'staged'
  | 'applying' | 'done' | 'error';

interface UpdateFlow {
  phase: UpdatePhase;
  message: string;
  currentSha?: string;
  latestSha?: string;
  percent?: number;
  progressState?: ProgressBarState;
  downloaded?: number;
  total?: number;
}

@Component({
  selector: 'app-services',
  standalone: true,
  imports: [CommonModule, RouterLink, FormsModule, PuruProgressComponent],
  template: `
    <div class="page">
      <div class="page-header">
        <div>
          <h1>Services</h1>
          <p class="page-subtitle">
            @if (!loading) {
              {{ runningCount }}/{{ services.length }} running
            } @else {
              Loading…
            }
          </p>
        </div>
        <div class="header-actions">
          <button class="btn btn-stroked" (click)="refreshServices()">
            <span class="material-icons">refresh</span>
            Refresh
          </button>
          @if (isNative && updatableServices.length > 0) {
            <button class="btn btn-stroked" (click)="checkAllUpdates()" [disabled]="batchBusy" title="Check every service for a newer version">
              <span class="material-icons">system_update</span>
              Check for updates
            </button>
            @if (availableCount > 0) {
              <button class="btn btn-stroked" (click)="downloadAllUpdates()" [disabled]="batchBusy" title="Download all available updates (services keep running)">
                <span class="material-icons">cloud_download</span>
                Download all ({{ availableCount }})
              </button>
            }
            @if (stagedCount > 0) {
              <button class="btn btn-primary" (click)="applyAllUpdates()" [disabled]="batchBusy" title="Install all downloaded updates">
                <span class="material-icons">restart_alt</span>
                Install all ({{ stagedCount }})
              </button>
            }
          }
          <button class="btn btn-primary" (click)="startAll()" [disabled]="services.length === 0">
            <span class="material-icons">play_arrow</span>
            Start all
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
            <h3>No services found</h3>
            <p>{{ isNative ? 'Run the setup wizard to install and start services.' : 'No running services were found.' }}</p>
            <button class="btn btn-stroked" routerLink="/setup">
              <span class="material-icons">build</span>
              Run setup wizard
            </button>
          </div>
        </div>
      } @else {
        <div class="card table-card">
          <table class="data-table services-table">
            <thead>
              <tr>
                <th class="sortable" (click)="sortBy('name')">
                  Service
                  <span class="material-icons sort-arrow">{{ sortArrow('name') }}</span>
                </th>
                <th class="sortable" (click)="sortBy('status')">
                  Status <span class="material-icons sort-arrow">{{ sortArrow('status') }}</span>
                </th>
                <th>Uptime</th>
                <th class="update-col"></th>
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
                        <span class="name-primary">{{ displayName(service.name) }}</span>
                        @if (actionMsg[service.name]; as m) {
                          <span class="action-msg" [class]="'am-' + m.kind">
                            @if (m.kind === 'busy') {
                              <span class="spinner" style="width:10px;height:10px;border-width:2px"></span>
                            } @else if (m.kind === 'ok') {
                              <span class="material-icons">check</span>
                            } @else {
                              <span class="material-icons">error_outline</span>
                            }
                            {{ m.text }}
                          </span>
                        }
                      </div>
                    </div>
                  </td>
                  <td>
                    <span [class]="'chip chip-' + service.status">{{ statusLabel(service) }}</span>
                    @if (service.status === 'error') {
                      <div class="svc-detail">Not responding — open logs to see why.</div>
                    }
                  </td>
                  <td><span class="uptime-text">{{ service.uptime || '—' }}</span></td>
                  <td class="update-cell">
                    @if (updateFlow[service.name]; as up) {
                      <div class="uf-inline" [class.uf-err]="up.phase === 'error'">
                        <div class="uf-line">
                          <span class="material-icons uf-ic">
                            @switch (up.phase) {
                              @case ('checking') { hourglass_top }
                              @case ('up-to-date') { check_circle }
                              @case ('available') { new_releases }
                              @case ('downloading') { cloud_download }
                              @case ('staged') { inventory_2 }
                              @case ('applying') { sync }
                              @case ('done') { check_circle }
                              @case ('error') { error }
                            }
                          </span>
                          <span class="uf-txt">{{ up.message }}</span>
                          @if (up.phase === 'available') {
                            <button class="uf-btn primary" (click)="downloadUpdate(service)" [disabled]="batchBusy">Update</button>
                            <button class="uf-btn ghost" (click)="dismissFlow(service)">Later</button>
                          }
                          @if (up.phase === 'staged') {
                            <button class="uf-btn primary" (click)="applyUpdate(service)" [disabled]="batchBusy">Install</button>
                            <button class="uf-btn ghost" (click)="discardUpdate(service)" [disabled]="batchBusy">Discard</button>
                          }
                          @if (up.phase === 'error') {
                            <button class="uf-btn ghost" (click)="dismissFlow(service)">Dismiss</button>
                          }
                        </div>
                        @if (up.phase === 'downloading' || up.phase === 'applying' || up.phase === 'done' || up.phase === 'staged') {
                          <puru-progress [value]="up.percent || 0" [state]="up.progressState || 'active'" [height]="4"></puru-progress>
                        }
                      </div>
                    }
                  </td>
                  <td class="actions-cell">
                    <button class="btn btn-stroked btn-sm inline-logs" (click)="openLogs(service)" title="View logs">
                      <span class="material-icons">article</span>
                      Logs
                    </button>
                    @if (service.status === 'running') {
                      <button class="btn btn-stroked btn-sm inline-stop"
                              (click)="stopAny(service)"
                              [title]="'Stop ' + displayName(service.name)">
                        <span class="material-icons">stop</span>
                        Stop
                      </button>
                    } @else if (service.status === 'stopped' || service.status === 'error') {
                      <button class="btn btn-stroked btn-sm inline-start"
                              (click)="startAny(service)"
                              [title]="'Start ' + displayName(service.name)">
                        <span class="material-icons">play_arrow</span>
                        Start
                      </button>
                    }
                    <div class="menu-wrap">
                      <button class="btn-icon" (click)="toggleMenu(service, $event)">
                        <span class="material-icons">more_vert</span>
                      </button>
                      @if (openMenu === service.name) {
                        <div class="menu" (click)="$event.stopPropagation()">
                          @if (service.infra) {
                            @if (service.status !== 'running') {
                              <button class="menu-item" (click)="controlInfra(service, 'start'); openMenu = null">
                                <span class="material-icons menu-green">play_arrow</span> Start
                              </button>
                            }
                            @if (service.status === 'running') {
                              <button class="menu-item" (click)="controlInfra(service, 'stop'); openMenu = null">
                                <span class="material-icons menu-red">stop</span> Stop
                              </button>
                            }
                            <button class="menu-item" (click)="controlInfra(service, 'restart'); openMenu = null">
                              <span class="material-icons menu-orange">refresh</span> Restart
                            </button>
                            <button class="menu-item" (click)="viewInfraLog(service); openMenu = null">
                              <span class="material-icons">article</span> View logs
                            </button>
                          } @else {
                            @if (service.status === 'stopped' || service.status === 'error') {
                              <button class="menu-item" (click)="startService(service); openMenu = null">
                                <span class="material-icons menu-green">play_arrow</span> Start
                              </button>
                            }
                            @if (service.status === 'notinstalled') {
                              <button class="menu-item" routerLink="/setup" (click)="openMenu = null">
                                <span class="material-icons menu-green">build</span> Install (run setup)
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
                              <span class="material-icons">article</span> View logs
                            </button>
                            @if (isNative && service.name !== 'puru-hydrogen' && service.name !== 'dviewer' && service.status !== 'notinstalled') {
                              <button class="menu-item" (click)="checkUpdate(service); openMenu = null">
                                <span class="material-icons menu-green">system_update</span> Check for update
                              </button>
                            }
                            @if (isNative) {
                              <button class="menu-item" (click)="rollbackService(service); openMenu = null">
                                <span class="material-icons menu-orange">undo</span> Roll back
                              </button>
                            }
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

        <!-- ── Developer Tools (advanced — to be password-protected) ────────── -->
        <div class="card dev-card">
          <div class="dev-header" (click)="devToolsOpen = !devToolsOpen">
            <div class="dev-title">
              <span class="material-icons">lock</span>
              <span>Developer tools</span>
              <span class="dev-sub">Advanced diagnostics — for support staff</span>
            </div>
            <span class="material-icons dev-toggle">{{ devToolsOpen ? 'expand_less' : 'expand_more' }}</span>
          </div>
          @if (devToolsOpen) {
            <div class="dev-body">
              <div class="dev-links">
                <button class="dev-link" (click)="initRoles()" [disabled]="initRolesBusy">
                  <span class="material-icons">admin_panel_settings</span>
                  <div>
                    <span class="dl-title">Initialize roles &amp; permissions</span>
                    <span class="dl-sub">{{ initRolesBusy ? 'Working…' : 'Seed roles, permissions and the root user (via Auth)' }}</span>
                  </div>
                  @if (initRolesBusy) {
                    <span class="spinner" style="width:16px;height:16px;border-width:2px"></span>
                  } @else {
                    <span class="material-icons dl-arrow">chevron_right</span>
                  }
                </button>
                <button class="dev-link" routerLink="/performance">
                  <span class="material-icons">memory</span>
                  <div>
                    <span class="dl-title">Performance</span>
                    <span class="dl-sub">Memory tuning for each service</span>
                  </div>
                  <span class="material-icons dl-arrow">chevron_right</span>
                </button>
                <button class="dev-link" (click)="toggleProcessExplorer()">
                  <span class="material-icons">build</span>
                  <div>
                    <span class="dl-title">Port tools</span>
                    <span class="dl-sub">Find &amp; free processes holding Puru ports</span>
                  </div>
                  <span class="material-icons dl-arrow">{{ processExplorerOpen ? 'expand_less' : 'expand_more' }}</span>
                </button>
              </div>

              @if (processExplorerOpen) {
                <div class="pt-body">
                  <div class="pt-bar">
                    <button class="btn btn-stroked btn-sm" (click)="refreshProcesses()" [disabled]="processesLoading">
                      <span class="material-icons">refresh</span>
                      {{ processesLoading ? 'Scanning…' : 'Refresh' }}
                    </button>
                  </div>
                  @if (processesLoading && processes.length === 0) {
                    <div class="pt-loading"><span class="spinner"></span></div>
                  } @else if (processes.length === 0) {
                    <div class="pt-empty">No Puru-relevant processes found.</div>
                  } @else {
                    <table class="data-table pt-table">
                      <thead>
                        <tr>
                          <th>PID</th>
                          <th>Process</th>
                          <th>Listening ports</th>
                          <th>CPU</th>
                          <th>Memory</th>
                          <th></th>
                        </tr>
                      </thead>
                      <tbody>
                        @for (p of processes; track p.pid) {
                          <tr>
                            <td><span class="pid">{{ p.pid }}</span></td>
                            <td>
                              <div class="pt-name">
                                <span class="pt-bin">{{ p.name }}</span>
                                @if (p.label) { <span class="pt-label">{{ p.label }}</span> }
                              </div>
                              @if (p.cmd) { <div class="pt-cmd" [title]="p.cmd">{{ p.cmd }}</div> }
                            </td>
                            <td>
                              @if (p.listening_ports.length === 0) {
                                <span class="text-muted">&mdash;</span>
                              } @else {
                                @for (port of p.listening_ports; track port) {
                                  <span class="pt-port">{{ port }}</span>
                                }
                              }
                            </td>
                            <td>{{ p.cpu_pct }}%</td>
                            <td>{{ p.mem_mb }} MB</td>
                            <td class="actions-cell">
                              <button class="btn btn-danger btn-sm" (click)="killProcess(p)" [disabled]="!!killing[p.pid]">
                                <span class="material-icons">close</span>
                                {{ killing[p.pid] ? 'Killing…' : 'Kill' }}
                              </button>
                            </td>
                          </tr>
                        }
                      </tbody>
                    </table>
                  }
                </div>
              }
            </div>
          }
        </div>
      }
    </div>

    <!-- ── Log dialog (modal) ───────────────────────────────────────────────── -->
    @if (logContainer) {
      <div class="log-modal-backdrop">
        <div class="log-modal" role="dialog" aria-modal="true">
          <div class="log-header">
            <div class="log-title">
              <span class="material-icons">article</span>
              <span>{{ displayName(logContainer) }} — logs</span>
            </div>
            <div class="log-head-right">
              <select class="log-select" [(ngModel)]="logTimeFilter" (ngModelChange)="refreshLogs()">
                <option value="tail">Latest 200 lines</option>
                <option value="1h">Last 1 hour</option>
                <option value="6h">Last 6 hours</option>
                <option value="24h">Last 24 hours</option>
                <option value="3d">Last 3 days</option>
                <option value="7d">Last 7 days</option>
              </select>
              <button class="btn-icon log-btn" (click)="closeLogs()" title="Close (Esc)">
                <span class="material-icons">close</span>
              </button>
            </div>
          </div>

          <div class="log-toolbar">
            <div class="log-search">
              <span class="material-icons">search</span>
              <input type="text" [(ngModel)]="logSearch" placeholder="Filter lines…" spellcheck="false" />
              @if (logSearch) {
                <button class="ls-clear" (click)="logSearch = ''" title="Clear"><span class="material-icons">close</span></button>
              }
            </div>
            <div class="log-levels">
              <button class="lv-btn" [class.on]="logLevel === 'all'" (click)="logLevel = 'all'">All</button>
              <button class="lv-btn lv-warn" [class.on]="logLevel === 'warn'" (click)="logLevel = 'warn'">Warnings+</button>
              <button class="lv-btn lv-err" [class.on]="logLevel === 'error'" (click)="logLevel = 'error'">Errors</button>
            </div>
            <span class="log-count">{{ logLines.length }} line{{ logLines.length === 1 ? '' : 's' }}</span>
            <div class="log-tools">
              <button class="btn-icon log-btn" (click)="logWrap = !logWrap" [title]="logWrap ? 'No wrap (single line)' : 'Wrap lines'">
                <span class="material-icons">{{ logWrap ? 'wrap_text' : 'notes' }}</span>
              </button>
              <button class="btn-icon log-btn" (click)="copyLogs()" title="Copy all">
                <span class="material-icons">{{ copied ? 'check' : 'content_copy' }}</span>
              </button>
            </div>
          </div>

          @if (logsLoading) {
            <div class="log-loading"><span class="spinner"></span></div>
          } @else if (logLines.length === 0) {
            <div class="log-empty">{{ logSearch || logLevel !== 'all' ? 'No lines match the filter.' : 'No logs available.' }}</div>
          } @else {
            <div class="log-body" [class.nowrap]="!logWrap">
              @for (ln of logLines; track $index) {
                <div class="log-line" [class]="ln.cls">{{ ln.text }}</div>
              }
            </div>
          }

          <div class="log-foot">
            <span class="log-hint">Press Esc to close</span>
            <button class="btn btn-primary" (click)="refreshLogs()" [disabled]="logsLoading">
              <span class="material-icons">refresh</span>
              {{ logsLoading ? 'Refreshing…' : 'Refresh (jump to latest)' }}
            </button>
          </div>
        </div>
      </div>
    }
  `,
  styles: [`
    .page {
      max-width: 1200px;
      margin: 0 auto;
      padding: 28px 32px;
      transition: margin-right 0.2s ease;
    }
    .page.drawer-open { margin-right: 460px; }

    .page-header {
      display: flex;
      justify-content: space-between;
      align-items: flex-start;
      margin-bottom: 20px;

      h1 { font-size: 1.5rem; font-weight: 700; margin-bottom: 2px; }
      .page-subtitle { color: var(--text-secondary); font-size: 0.85rem; }
    }

    .header-actions { display: flex; gap: 10px; flex-wrap: wrap; justify-content: flex-end; }

    .loading-state { display: flex; justify-content: center; padding: 80px 0; }

    /* ── Empty State ──────────────────────────── */
    .empty-card { padding: 0 !important; }
    .empty-state {
      display: flex; flex-direction: column; align-items: center; gap: 12px;
      padding: 60px 20px; text-align: center;
      .empty-icon {
        width: 64px; height: 64px; border-radius: 16px; background: #f1f5f9;
        display: flex; align-items: center; justify-content: center;
        .material-icons { font-size: 32px; color: #94a3b8; }
      }
      h3 { font-size: 1rem; font-weight: 600; color: var(--text-primary); margin: 0; }
      p { font-size: 0.85rem; color: var(--text-secondary); margin: 0; max-width: 340px; }
      button { margin-top: 8px; }
    }

    /* ── Table ────────────────────────────────── */
    .table-card { overflow: visible; }
    .services-table { width: 100%; table-layout: auto; }
    .services-table td { vertical-align: middle; }
    .update-col { width: auto; }
    .actions-cell { text-align: right; width: 1%; white-space: nowrap; }
    .actions-cell .inline-logs,
    .actions-cell .inline-stop,
    .actions-cell .inline-start { margin-right: 6px; vertical-align: middle; }
    .actions-cell .menu-wrap { display: inline-block; vertical-align: middle; }
    .inline-logs .material-icons,
    .inline-stop .material-icons,
    .inline-start .material-icons { font-size: 15px; }
    .inline-stop { color: #b91c1c; border-color: #fecaca; }
    .inline-stop:hover:not([disabled]) { background: #fef2f2; border-color: #fca5a5; }
    .inline-start { color: #047857; border-color: #a7f3d0; }
    .inline-start:hover:not([disabled]) { background: #ecfdf5; border-color: #6ee7b7; }

    .name-cell { display: flex; align-items: center; gap: 12px; }
    .status-dot {
      width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0;
      &.dot-running { background: var(--status-green); box-shadow: 0 0 6px rgba(34, 197, 94, 0.4); }
      &.dot-stopped { background: var(--text-muted); }
      &.dot-starting { background: var(--status-orange); }
      &.dot-notinstalled { background: var(--text-muted); }
      &.dot-error { background: var(--status-red); }
    }
    .name-info { display: flex; flex-direction: column; }
    .name-primary { font-weight: 600; font-size: 0.9rem; color: var(--text-primary); }

    .action-msg {
      display: inline-flex; align-items: center; gap: 4px; margin-top: 3px;
      font-size: 0.72rem; font-weight: 600;
      .material-icons { font-size: 13px; }
      &.am-busy { color: var(--text-muted, #8a94a3); }
      &.am-ok { color: var(--brand-green, #2e9e5b); }
      &.am-error { color: var(--brand-red, #e83a3a); }
    }

    .uptime-text { font-size: 0.85rem; color: var(--text-secondary); font-variant-numeric: tabular-nums; }
    .text-muted { color: var(--text-muted); }

    .svc-detail {
      margin-top: 4px; font-size: 11px; line-height: 1.35;
      color: var(--status-red); max-width: 360px;
    }

    .menu-green { color: var(--status-green) !important; }
    .menu-red { color: var(--status-red) !important; }
    .menu-orange { color: var(--status-orange) !important; }

    /* ── Inline per-service update ────────────── */
    .update-cell { padding-right: 8px; }
    .uf-inline { display: flex; flex-direction: column; gap: 5px; }
    .uf-line { display: flex; align-items: center; gap: 8px; font-size: 0.8rem; flex-wrap: wrap; }
    .uf-ic { font-size: 16px; color: var(--brand-blue, #009efb); flex-shrink: 0; }
    .uf-inline.uf-err .uf-ic { color: var(--brand-red, #e83a3a); }
    .uf-txt { color: var(--text-secondary, #5a6472); font-weight: 500; white-space: nowrap; }
    .uf-btn {
      display: inline-flex; align-items: center; gap: 4px;
      padding: 3px 12px; font-size: 0.76rem; font-weight: 600;
      border-radius: 6px; cursor: pointer; border: 1px solid transparent;
      transition: background 0.15s ease, filter 0.15s ease;
    }
    .uf-btn.primary { color: #fff; background: var(--brand-blue, #009efb); }
    .uf-btn.primary:hover:not(:disabled) { filter: brightness(0.94); }
    .uf-btn.ghost { color: var(--text-secondary, #5a6472); background: transparent; border: 1px solid var(--border, #d5dbe3); }
    .uf-btn.ghost:hover:not(:disabled) { background: var(--bg-hover, rgba(0,0,0,0.04)); }
    .uf-btn:disabled { opacity: 0.5; cursor: default; }

    /* ── Developer Tools ──────────────────────── */
    .dev-card { margin-top: 16px; padding: 0 !important; overflow: hidden; }
    .dev-header {
      display: flex; align-items: center; justify-content: space-between;
      padding: 12px 16px; cursor: pointer; user-select: none;
      background: #f8fafc; transition: background 0.15s;
      &:hover { background: #f1f5f9; }
    }
    .dev-title {
      display: flex; align-items: center; gap: 8px; font-size: 0.9rem; font-weight: 600;
      .material-icons { font-size: 17px; color: #64748b; }
    }
    .dev-sub { font-weight: 400; color: #64748b; font-size: 0.78rem; margin-left: 2px; }
    .dev-toggle { color: #64748b; }
    .dev-body { border-top: 1px solid #e2e8f0; padding: 14px 16px; }
    .dev-links { display: flex; flex-direction: column; gap: 10px; }
    .dev-link {
      display: flex; align-items: center; gap: 12px; width: 100%; text-align: left;
      padding: 12px 14px; border: 1px solid var(--border, #e2e8f0); border-radius: 10px;
      background: #fff; cursor: pointer; transition: border-color 0.15s, background 0.15s;
      &:hover { border-color: #cbd5e1; background: #f8fafc; }
      > .material-icons:first-child { font-size: 20px; color: #64748b; }
      div { display: flex; flex-direction: column; flex: 1; }
      .dl-title { font-size: 0.86rem; font-weight: 600; color: var(--text-primary); }
      .dl-sub { font-size: 0.74rem; color: #64748b; }
      .dl-arrow { color: #94a3b8; font-size: 20px; }
    }

    .pt-body { margin-top: 12px; }
    .pt-bar { display: flex; justify-content: flex-end; margin-bottom: 8px; }
    .pt-loading { display: flex; justify-content: center; padding: 24px; }
    .pt-empty { padding: 16px; color: #64748b; font-size: 0.85rem; text-align: center; }
    .pt-table th, .pt-table td { font-size: 0.82rem; }
    .pid { font-family: 'SF Mono', 'Fira Code', monospace; font-size: 0.8rem; }
    .pt-name { display: flex; align-items: center; gap: 8px; }
    .pt-bin { font-family: 'SF Mono', 'Fira Code', monospace; color: #1e293b; font-size: 0.82rem; }
    .pt-label { display: inline-block; background: #eef2ff; color: #4338ca; padding: 2px 8px; border-radius: 4px; font-size: 0.7rem; font-weight: 600; }
    .pt-cmd { font-family: 'SF Mono', 'Fira Code', monospace; font-size: 0.7rem; color: #94a3b8; margin-top: 2px; max-width: 380px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .pt-port { display: inline-block; background: #f1f5f9; color: #334155; padding: 2px 8px; border-radius: 4px; font-family: 'SF Mono', 'Fira Code', monospace; font-size: 0.75rem; margin-right: 4px; margin-bottom: 2px; }
    .btn-danger {
      background: #dc2626; color: #fff; border: 1px solid #dc2626;
      &:hover:not([disabled]) { background: #b91c1c; border-color: #b91c1c; }
      &[disabled] { opacity: 0.55; cursor: not-allowed; }
    }
    .btn-sm { padding: 4px 10px; font-size: 0.78rem; .material-icons { font-size: 14px; } }

    /* ── Log modal ────────────────────────────── */
    .log-modal-backdrop {
      position: fixed; inset: 0; z-index: 60;
      background: rgba(2, 6, 23, 0.55);
      display: flex; align-items: center; justify-content: center;
      padding: 32px;
    }
    .log-modal {
      display: flex; flex-direction: column;
      width: min(1100px, 94vw); height: min(820px, 88vh);
      background: #0f172a; border-radius: 12px; overflow: hidden;
      box-shadow: 0 24px 60px rgba(0,0,0,0.45);
    }
    .log-header {
      display: flex; justify-content: space-between; align-items: center;
      padding: 12px 16px; background: #1e293b; color: #e2e8f0; flex-shrink: 0;
    }
    .log-title {
      display: flex; align-items: center; gap: 8px; font-size: 0.95rem; font-weight: 600;
      .material-icons { font-size: 18px; }
    }
    .log-head-right { display: flex; align-items: center; gap: 8px; }
    .log-btn { color: #94a3b8; &:hover { background: rgba(255,255,255,0.1); color: #fff; } }
    .log-select {
      background: #0f172a; color: #e2e8f0; border: 1px solid #334155; border-radius: 6px;
      padding: 5px 8px; font-size: 0.76rem; cursor: pointer; outline: none;
      &:hover { border-color: #475569; }
      option { background: #1e293b; color: #e2e8f0; }
    }

    .log-toolbar {
      display: flex; align-items: center; gap: 12px; flex-wrap: wrap;
      padding: 8px 14px; background: #131c31; border-bottom: 1px solid #1e293b; flex-shrink: 0;
    }
    .log-search {
      display: flex; align-items: center; gap: 6px; flex: 1; min-width: 160px;
      background: #0f172a; border: 1px solid #334155; border-radius: 6px; padding: 4px 8px;
      .material-icons { font-size: 16px; color: #64748b; }
      input {
        flex: 1; background: transparent; border: none; outline: none; color: #e2e8f0;
        font-size: 0.8rem; font-family: inherit;
        &::placeholder { color: #64748b; }
      }
      .ls-clear { color: #64748b; display: flex; .material-icons { font-size: 15px; } &:hover { color: #e2e8f0; } }
    }
    .log-levels { display: flex; gap: 4px; }
    .lv-btn {
      padding: 4px 10px; font-size: 0.74rem; font-weight: 600; border-radius: 6px;
      border: 1px solid #334155; background: transparent; color: #94a3b8; cursor: pointer;
      &:hover { border-color: #475569; color: #e2e8f0; }
      &.on { background: #334155; color: #fff; }
      &.lv-warn.on { background: #b45309; border-color: #b45309; }
      &.lv-err.on { background: #b91c1c; border-color: #b91c1c; }
    }
    .log-count { font-size: 0.74rem; color: #64748b; font-variant-numeric: tabular-nums; }
    .log-tools { display: flex; gap: 2px; margin-left: auto; }

    .log-loading { display: flex; justify-content: center; align-items: center; flex: 1; background: #0f172a; }
    .log-empty { flex: 1; display: flex; align-items: center; justify-content: center; color: #64748b; font-size: 0.85rem; background: #0f172a; }

    .log-body {
      margin: 0; flex: 1; overflow: auto;
      background: #0f172a; color: #e2e8f0;
      font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
      font-size: 0.75rem; line-height: 1.55;
      padding: 10px 0;
    }
    .log-line {
      padding: 1px 16px;
      white-space: pre-wrap; word-break: break-word;   /* wrapped mode (default off) */
    }
    .log-body.nowrap .log-line { white-space: pre; }    /* single line each, horizontal scroll */
    .log-line:hover { background: rgba(148, 163, 184, 0.08); }
    .log-line.ll-error { color: #f87171; }
    .log-line.ll-warn { color: #fbbf24; }
    .log-line.ll-info { color: #cbd5e1; }
    .log-line.ll-debug { color: #64748b; }

    .log-foot {
      display: flex; align-items: center; justify-content: space-between; gap: 12px;
      padding: 10px 14px; background: #1e293b; flex-shrink: 0;
    }
    .log-hint { font-size: 0.74rem; color: #64748b; }

    /* Tablet / narrow window: header actions wrap, trim padding, full-screen modal. */
    @media (max-width: 820px) {
      .page { padding: 20px 18px; }
      .page-header { flex-direction: column; align-items: stretch; gap: 12px; }
      .header-actions { justify-content: flex-start; }
      .update-col { width: 1%; }
      .inline-logs,
      .inline-stop,
      .inline-start { display: none; }   /* actions collapse into the ⋮ menu on small screens */
      .log-modal-backdrop { padding: 0; }
      .log-modal { width: 100vw; height: 100vh; border-radius: 0; }
    }

    /* Phone-width: tighten paddings and let the update text truncate. */
    @media (max-width: 560px) {
      .page { padding: 16px 12px; }
      .header-actions .btn { flex: 1 1 auto; justify-content: center; }
      .services-table th, .services-table td { padding-left: 8px; padding-right: 8px; }
      .uf-txt { max-width: 90px; overflow: hidden; text-overflow: ellipsis; }
      .name-primary { font-size: 0.84rem; }
    }
  `]
})
export class ServicesComponent implements OnInit, OnDestroy {
  private tauri = inject(TauriService);
  private notification = inject(NotificationService);

  /** Per-service update flow state (identify → download → apply), keyed by name. */
  updateFlow: Record<string, UpdateFlow> = {};
  private unlistenProgress?: UnlistenFn;
  /** True while a batch (Check/Download/Apply All) is running. */
  batchBusy = false;

  /** Transient inline status shown under a service (e.g. "Restarted"), keyed by name. */
  actionMsg: Record<string, { text: string; kind: 'busy' | 'ok' | 'error' }> = {};

  services: ServiceInfo[] = [];
  loading = true;
  isNative = false;
  logContainer: string | null = null;
  logOutput = '';
  logsLoading = false;
  logTimeFilter: 'tail' | '1h' | '6h' | '24h' | '3d' | '7d' = 'tail';
  /** Log viewer controls. */
  logSearch = '';
  logLevel: 'all' | 'warn' | 'error' = 'all';
  logWrap = false;      // default: one line per entry (horizontal scroll)
  copied = false;
  private logVersion = 0;   // bumped whenever logOutput changes, to bust the line cache

  sortKey = 'name';
  sortDir: 1 | -1 = 1;
  openMenu: string | null = null;

  /** Developer-tools section (advanced; will be password-gated later). */
  devToolsOpen = false;
  /** True while the auth role/permission bootstrap is running. */
  initRolesBusy = false;

  // ── Port Tools / Process Explorer ────────────
  processExplorerOpen = false;
  processes: ProcessInfo[] = [];
  processesLoading = false;
  killing: Record<number, boolean> = {};

  get runningCount(): number {
    return this.services.filter(s => s.status === 'running').length;
  }

  /** Friendly display name — strip the internal "puru-" prefix, Title Case. */
  displayName(name: string): string {
    const stripped = name.replace(/^puru-/, '');
    const special: Record<string, string> = {
      'hydrogen': 'Front End',
      'dviewer': 'DICOM viewer',
      'has': 'HAS',
      'pacs': 'PACS',
      'ris': 'RIS',
    };
    if (special[stripped]) return special[stripped];
    return stripped.charAt(0).toUpperCase() + stripped.slice(1);
  }

  /** Client-friendly status label — no developer jargon. */
  statusLabel(s: ServiceInfo): string {
    switch (s.status as string) {
      case 'running': return 'Running';
      case 'starting': return 'Starting';
      case 'stopped': return 'Stopped';
      case 'error': return 'Needs attention';
      case 'notinstalled': return 'Not installed';
      default: return String(s.status);
    }
  }

  private _sortedCache: ServiceInfo[] = [];
  private _sortSig = '';
  /**
   * Sorted view of the services, memoized. Returns the SAME array reference
   * until the data or sort actually changes, so change detection (which runs on
   * every app poll tick) doesn't re-sort or churn the `@for` on every cycle.
   */
  get sortedServices(): ServiceInfo[] {
    const sig = `${this.sortKey}|${this.sortDir}|` +
      this.services.map(s => `${s.name}:${s.status}:${s.uptime || ''}`).join(',');
    if (sig !== this._sortSig) {
      this._sortSig = sig;
      const dir = this.sortDir;
      const key = this.sortKey;
      this._sortedCache = [...this.services].sort((a, b) =>
        this.sortValue(a, key).localeCompare(this.sortValue(b, key)) * dir
      );
    }
    return this._sortedCache;
  }

  private sortValue(s: ServiceInfo, key: string): string {
    switch (key) {
      case 'status': return s.status || '';
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

  @HostListener('document:keydown.escape')
  onEscape(): void {
    if (this.logContainer) this.closeLogs();
  }

  /** Open the log dialog for a service (routes infra rows to their own reader). */
  openLogs(service: ServiceInfo): void {
    if (service.infra) {
      this.viewInfraLog(service);
    } else {
      this.viewLogs(service);
    }
  }

  /** Parsed, filtered log lines for the dialog — memoized so change detection
   *  (which runs on every poll tick) doesn't re-parse on every cycle. */
  private _logLinesCache: { text: string; cls: string }[] = [];
  private _logLinesSig = '';
  get logLines(): { text: string; cls: string }[] {
    const sig = `${this.logVersion}|${this.logLevel}|${this.logSearch}`;
    if (sig !== this._logLinesSig) {
      this._logLinesSig = sig;
      this._logLinesCache = this.computeLogLines();
    }
    return this._logLinesCache;
  }

  private computeLogLines(): { text: string; cls: string }[] {
    const raw = this.logOutput || '';
    if (!raw.trim()) return [];
    const q = this.logSearch.trim().toLowerCase();
    const out: { text: string; cls: string }[] = [];
    let carry = ''; // level carried onto indented stack-trace continuation lines
    for (const line of raw.split(/\r?\n/)) {
      if (line === '') continue;
      let cls = this.lineClass(line);
      if (cls) {
        carry = cls;
      } else if (/^\s/.test(line) && carry) {
        cls = carry; // "  at com...", "Caused by:" — keep with the error above it
      } else {
        carry = '';
      }
      // Level filter
      if (this.logLevel === 'error' && cls !== 'll-error') continue;
      if (this.logLevel === 'warn' && cls !== 'll-error' && cls !== 'll-warn') continue;
      // Search filter
      if (q && !line.toLowerCase().includes(q)) continue;
      out.push({ text: line, cls });
    }
    return out;
  }

  private lineClass(line: string): string {
    if (/\bERROR\b|\bSEVERE\b|\bFATAL\b|Exception[:\s]|\bat [\w.$]+\(/.test(line)) return 'll-error';
    if (/\bWARN(ING)?\b/.test(line)) return 'll-warn';
    if (/\bDEBUG\b|\bTRACE\b/.test(line)) return 'll-debug';
    if (/\bINFO\b/.test(line)) return 'll-info';
    return '';
  }

  async copyLogs(): Promise<void> {
    try {
      await navigator.clipboard.writeText(this.logOutput || '');
      this.copied = true;
      setTimeout(() => (this.copied = false), 1500);
    } catch { /* clipboard unavailable */ }
  }

  ngOnInit(): void {
    this.loadDeploymentMode();
    this.loadServices().then(() => this.loadStagedUpdates());

    // Live progress for the download + apply phases of the update flow.
    listen<JarUpdateProgress>('jar-update-progress', (ev) => {
      const p = ev.payload;
      if (!p.service) return; // batch-wide events (e.g. setup) have no single service
      const cur = this.updateFlow[p.service] ?? { phase: 'downloading', message: '' };
      const indeterminate = p.phase === 'downloading' && p.total === 0;

      if (p.phase === 'downloading') {
        this.updateFlow[p.service] = {
          ...cur, phase: 'downloading', message: 'Downloading…',
          downloaded: p.downloaded, total: p.total, percent: p.percent,
          progressState: indeterminate ? 'indeterminate' : 'active',
        };
      } else if (p.phase === 'staged') {
        this.updateFlow[p.service] = {
          ...cur, phase: 'staged', message: 'Ready to install', percent: 100, progressState: 'complete',
        };
      } else if (p.phase === 'stopping' || p.phase === 'starting') {
        this.updateFlow[p.service] = {
          ...cur, phase: 'applying', message: 'Installing…', percent: p.percent, progressState: 'active',
        };
      } else if (p.phase === 'done') {
        this.updateFlow[p.service] = { ...cur, phase: 'done', message: 'Updated', percent: 100, progressState: 'complete' };
        const svc = p.service;
        setTimeout(() => { if (this.updateFlow[svc]?.phase === 'done') delete this.updateFlow[svc]; }, 2500);
      } else if (p.phase === 'error') {
        this.updateFlow[p.service] = { ...cur, phase: 'error', message: 'Update failed', progressState: 'fail' };
      }
    }).then((un) => (this.unlistenProgress = un));
  }

  ngOnDestroy(): void {
    this.unlistenProgress?.();
  }

  /** Bytes → MB, one decimal, for the progress caption. */
  fmtMB(bytes: number): string {
    return (bytes / 1_048_576).toFixed(1);
  }

  // ── Split update flow: identify → download (stage) → apply ─────────────────

  /** Services eligible for JAR updates (native, installed, not the hydrogen or dviewer bundle). */
  get updatableServices(): ServiceInfo[] {
    return this.sortedServices.filter(s =>
      this.isNative && !s.infra && s.name !== 'puru-hydrogen' && s.name !== 'dviewer' && s.status !== 'notinstalled');
  }

  /** Start / stop / restart the MySQL or RabbitMQ Windows service (elevated). */
  async controlInfra(service: ServiceInfo, action: 'start' | 'stop' | 'restart'): Promise<void> {
    const name = service.name;
    const busy = action === 'start' ? 'Starting…' : action === 'stop' ? 'Stopping…' : 'Restarting…';
    this.setMsg(name, busy, 'busy');
    try {
      await this.tauri.invoke<string>('control_infra_service', { name, action });
      this.setMsg(name, action === 'stop' ? 'Stopped' : action === 'start' ? 'Started' : 'Restarted', 'ok');
      await this.loadServicesSilent();
    } catch (e) {
      this.setMsg(name, `${action} failed`, 'error');
    }
  }

  /** Show the MySQL/RabbitMQ log in the log panel (the crash reason). */
  async viewInfraLog(service: ServiceInfo): Promise<void> {
    this.logContainer = service.name;
    this.logTimeFilter = 'tail';
    this.logSearch = '';
    this.logLevel = 'all';
    this.logsLoading = true;
    this.logOutput = '';
    try {
      this.logOutput = await this.tauri.invoke<string>('get_infra_log', { name: service.name, lines: 300 });
    } catch (e) {
      this.logOutput = `Failed to fetch ${service.name} log: ${e}`;
    } finally {
      this.logsLoading = false;
      this.logVersion++;
      this.scrollLogsToBottom();
    }
  }
  get availableCount(): number {
    return Object.values(this.updateFlow).filter(f => f.phase === 'available').length;
  }
  get stagedCount(): number {
    return Object.values(this.updateFlow).filter(f => f.phase === 'staged').length;
  }

  /** On load, surface any updates that were downloaded but never applied. */
  private async loadStagedUpdates(): Promise<void> {
    for (const s of this.updatableServices) {
      try {
        const staged = await this.tauri.invoke<StagedUpdate | null>('get_staged_update', { serviceName: s.name });
        if (staged && !this.updateFlow[s.name]) {
          this.updateFlow[s.name] = {
            phase: 'staged', percent: 100, progressState: 'complete',
            latestSha: staged.short_sha,
            message: 'Ready to install',
          };
        }
      } catch { /* ignore */ }
    }
  }

  /** Step 1: identify. */
  async checkUpdate(service: ServiceInfo): Promise<void> {
    const name = service.name;
    this.updateFlow[name] = { phase: 'checking', message: 'Checking…' };
    try {
      const c = await this.tauri.invoke<JarUpdateCheck>('check_service_update', { serviceName: name });
      if (c.update_available) {
        this.updateFlow[name] = { phase: 'available', message: 'Update available',
          currentSha: c.current_sha, latestSha: c.latest_sha };
      } else {
        this.updateFlow[name] = { phase: 'up-to-date', message: 'Up to date', currentSha: c.current_sha };
        // Auto-clear the "Up to date" note — nothing to act on, no Dismiss needed.
        setTimeout(() => { if (this.updateFlow[name]?.phase === 'up-to-date') delete this.updateFlow[name]; }, 2500);
      }
    } catch (e) {
      this.updateFlow[name] = { phase: 'error', message: 'Check failed', progressState: 'fail' };
    }
  }

  /** Step 2: download (stage) — running service untouched. */
  async downloadUpdate(service: ServiceInfo): Promise<void> {
    const name = service.name;
    const prev = this.updateFlow[name];
    this.updateFlow[name] = { phase: 'downloading', message: 'Downloading…', percent: 0,
      progressState: 'active', latestSha: prev?.latestSha };
    try {
      const s = await this.tauri.invoke<StagedUpdate>('download_service_update', { serviceName: name });
      this.updateFlow[name] = { phase: 'staged', percent: 100, progressState: 'complete',
        latestSha: s.short_sha,
        message: 'Ready to install' };
    } catch (e) {
      if (this.updateFlow[name]?.phase !== 'error') {
        this.updateFlow[name] = { phase: 'error', message: 'Download failed', progressState: 'fail' };
      }
    }
  }

  /** Step 3: apply — stop → swap → restart. */
  async applyUpdate(service: ServiceInfo, skipConfirm = false): Promise<void> {
    const name = service.name;
    if (!skipConfirm && !confirm(`Install the downloaded update for ${this.displayName(name)}? The service restarts briefly.`)) return;
    this.updateFlow[name] = { phase: 'applying', message: 'Installing…', percent: 10, progressState: 'active' };
    try {
      const r = await this.tauri.invoke<any>('apply_service_update', { serviceName: name });
      this.updateFlow[name] = { phase: 'done', message: 'Updated', percent: 100, progressState: 'complete' };
      await this.loadServicesSilent();
      const svc = name;
      setTimeout(() => { if (this.updateFlow[svc]?.phase === 'done') delete this.updateFlow[svc]; }, 2500);
    } catch (e) {
      if (this.updateFlow[name]?.phase !== 'error') {
        this.updateFlow[name] = { phase: 'error', message: 'Install failed', progressState: 'fail' };
      }
    }
  }

  /** Discard a downloaded-but-unapplied update. */
  async discardUpdate(service: ServiceInfo): Promise<void> {
    const name = service.name;
    try { await this.tauri.invoke('discard_service_update', { serviceName: name }); } catch { /* ignore */ }
    delete this.updateFlow[name];
  }

  dismissFlow(service: ServiceInfo): void {
    delete this.updateFlow[service.name];
  }

  // ── Update All (same 3-step flow, fanned out) ──────────────────────────────

  async checkAllUpdates(): Promise<void> {
    this.batchBusy = true;
    try { await Promise.all(this.updatableServices.map(s => this.checkUpdate(s))); }
    finally { this.batchBusy = false; }
    const n = this.availableCount;
    this.notification.success(n > 0 ? `${n} update(s) available` : 'All services up to date');
  }

  async downloadAllUpdates(): Promise<void> {
    const targets = this.updatableServices.filter(s => this.updateFlow[s.name]?.phase === 'available');
    this.batchBusy = true;
    try { for (const s of targets) await this.downloadUpdate(s); } // sequential — kinder on bandwidth
    finally { this.batchBusy = false; }
  }

  async applyAllUpdates(): Promise<void> {
    const targets = this.updatableServices.filter(s => this.updateFlow[s.name]?.phase === 'staged');
    if (targets.length === 0) return;
    if (!confirm(`Install ${targets.length} downloaded update(s)? Each service restarts briefly, one at a time.`)) return;
    this.batchBusy = true;
    try { for (const s of targets) await this.applyUpdate(s, true); } // sequential — one restart at a time
    finally { this.batchBusy = false; }
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

  /**
   * Refresh the list WITHOUT flipping the full-page loading spinner — the table
   * stays put and only the changed rows update. Used after actions so the UI
   * doesn't flash the spinner on every start/stop/restart.
   */
  private async loadServicesSilent(): Promise<void> {
    try {
      this.services = await this.tauri.invoke<ServiceInfo[]>('get_services');
    } catch { /* keep the current list on a transient failure */ }
  }

  /** Set a transient inline status under a service. Non-busy states auto-clear. */
  private setMsg(name: string, text: string, kind: 'busy' | 'ok' | 'error'): void {
    this.actionMsg[name] = { text, kind };
    if (kind !== 'busy') {
      setTimeout(() => {
        if (this.actionMsg[name]?.text === text) delete this.actionMsg[name];
      }, kind === 'error' ? 6000 : 3000);
    }
  }

  async refreshServices(): Promise<void> {
    await this.loadServicesSilent();
  }

  /** In native mode use service name; in Docker mode use container_name */
  private svcId(service: ServiceInfo): string {
    return this.isNative ? service.name : service.container_name;
  }

  /** Dispatch a Stop click to whichever backend the row represents — infra
   *  (MySQL / RabbitMQ Windows services) routes through `controlInfra`; native
   *  and Docker services go through `stop_service`. */
  async stopAny(service: ServiceInfo): Promise<void> {
    if (service.infra) {
      await this.controlInfra(service, 'stop');
    } else {
      await this.stopService(service);
    }
  }

  /** Symmetric to `stopAny` — used by the inline Start button when a service
   *  is stopped or errored. NotInstalled rows still route through the setup
   *  wizard from the kebab menu because they need the JAR pulled first. */
  async startAny(service: ServiceInfo): Promise<void> {
    if (service.infra) {
      await this.controlInfra(service, 'start');
    } else {
      await this.startService(service);
    }
  }

  async startService(service: ServiceInfo): Promise<void> {
    const name = service.name;
    this.setMsg(name, 'Starting…', 'busy');
    try {
      await this.tauri.invoke('start_service', { name: this.svcId(service) });
      this.setMsg(name, 'Started', 'ok');
      await this.loadServicesSilent();
    } catch {
      this.setMsg(name, 'Failed to start', 'error');
    }
  }

  async stopService(service: ServiceInfo): Promise<void> {
    const name = service.name;
    this.setMsg(name, 'Stopping…', 'busy');
    try {
      await this.tauri.invoke('stop_service', { name: this.svcId(service) });
      this.setMsg(name, 'Stopped', 'ok');
      await this.loadServicesSilent();
    } catch {
      this.setMsg(name, 'Failed to stop', 'error');
    }
  }

  async restartService(service: ServiceInfo): Promise<void> {
    const name = service.name;
    this.setMsg(name, 'Restarting…', 'busy');
    try {
      await this.tauri.invoke('restart_service', { name: this.svcId(service) });
      this.setMsg(name, 'Restarted', 'ok');
      await this.loadServicesSilent();
    } catch {
      this.setMsg(name, 'Failed to restart', 'error');
    }
  }

  async startAll(): Promise<void> {
    // Only services whose build is installed can be started directly;
    // 'notinstalled' ones need a setup re-run (Pull JARs) first.
    const stoppedServices = this.services.filter(
      s => s.status === 'stopped' || s.status === 'error'
    );
    for (const service of stoppedServices) {
      this.setMsg(service.name, 'Starting…', 'busy');
      try {
        await this.tauri.invoke('start_service', { name: this.svcId(service) });
        this.setMsg(service.name, 'Started', 'ok');
      } catch {
        this.setMsg(service.name, 'Failed to start', 'error');
      }
    }
    await this.loadServicesSilent();
  }

  async viewLogs(service: ServiceInfo): Promise<void> {
    this.logContainer = this.isNative ? service.name : service.container_name;
    this.logTimeFilter = 'tail';
    this.logSearch = '';
    this.logLevel = 'all';
    this.logsLoading = true;
    this.logOutput = '';
    try {
      this.logOutput = await this.tauri.invoke<string>('get_container_logs', this.buildLogArgs());
    } catch {
      this.logOutput = `Failed to fetch logs for ${service.container_name}.`;
    } finally {
      this.logsLoading = false;
      this.logVersion++;
      this.scrollLogsToBottom();
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
      this.logVersion++;
      this.scrollLogsToBottom();
    }
  }

  /** Jump the log view to the newest lines — that's what operators want to see. */
  private scrollLogsToBottom(): void {
    setTimeout(() => {
      const el = document.querySelector('.log-body') as HTMLElement | null;
      if (el) el.scrollTop = el.scrollHeight;
    }, 0);
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


  async rollbackService(service: ServiceInfo): Promise<void> {
    if (!confirm(`Roll back ${this.displayName(service.name)} to the previous version?`)) return;
    const name = service.name;
    this.setMsg(name, 'Rolling back…', 'busy');
    try {
      await this.tauri.invoke('rollback_native_service', { serviceName: name });
      this.setMsg(name, 'Rolled back', 'ok');
      await this.loadServicesSilent();
    } catch {
      this.setMsg(name, 'Rollback failed', 'error');
    }
  }

  // ── Manifest-driven recovery: pending/history + stuck-update surfacing ─────

  /** Per-service manifest snapshot (active + pending + history). Populated
   *  lazily by loadManifest(); consumed by the rollback picker and the
   *  "pending — restart to activate" banner. */
  manifestByService: Record<string, JarManifestView> = {};

  /** Locking-PID panel state, populated on demand by loadLockingProcesses().
   *  Empty array = "we checked and found none"; missing key = "not checked
   *  yet". Restart Manager is Windows-only; on other platforms this is always
   *  empty even when a JAR is locked. */
  lockingByService: Record<string, LockingProcess[]> = {};

  /** Fetch the full JAR manifest for a service — used by the rollback picker
   *  (needs history[]) and to detect the "crashed apply → pending set" case
   *  when the operator opens the UI cold. */
  async loadManifest(name: string): Promise<JarManifestView | null> {
    try {
      const m = await this.tauri.invoke<JarManifestView>('get_jar_manifest', { serviceName: name });
      this.manifestByService[name] = m;
      // If the manifest carries a pending entry but the update flow is idle,
      // surface it as "staged — restart to activate" (same shape the split
      // update flow uses on happy path). Keeps recovery one click away.
      if (m.pending && !this.updateFlow[name]) {
        this.updateFlow[name] = {
          phase: 'staged', percent: 100, progressState: 'complete',
          latestSha: m.pending.short_sha,
          message: 'Ready to install',
        };
      }
      return m;
    } catch {
      return null;
    }
  }

  /** List processes still holding the service's active JAR. Called by the
   *  "stuck update" panel after an apply times out. Cache in
   *  lockingByService[name] so the template can render without re-invoking. */
  async loadLockingProcesses(name: string): Promise<LockingProcess[]> {
    try {
      const list = await this.tauri.invoke<LockingProcess[]>('list_locking_processes', { serviceName: name });
      this.lockingByService[name] = list;
      return list;
    } catch {
      this.lockingByService[name] = [];
      return [];
    }
  }

  /** Force-kill every process holding the service's active JAR (Windows
   *  Restart Manager), then cycle the service so start_service's recovery
   *  logic promotes any pending JAR. Used by the "Force kill & restart"
   *  action in the stuck-update panel. */
  async forceFreeAndRestart(service: ServiceInfo): Promise<void> {
    const name = service.name;
    if (!confirm(`Force-kill every process holding ${this.displayName(name)}'s JAR and restart the service?`)) return;
    this.setMsg(name, 'Force killing…', 'busy');
    try {
      await this.tauri.invoke('force_free_and_restart', { serviceName: name });
      this.setMsg(name, 'Restarted', 'ok');
      delete this.lockingByService[name];
      delete this.updateFlow[name];
      await this.loadServicesSilent();
      await this.loadManifest(name);
    } catch {
      this.setMsg(name, 'Force restart failed', 'error');
    }
  }

  /** Roll back to a specific historical JAR (by filename, from
   *  manifest.history). The plain "Rollback" button walks back one step;
   *  this one lets the operator pick a specific target. */
  async rollbackToVersion(service: ServiceInfo, file: string): Promise<void> {
    if (!confirm(`Roll back ${this.displayName(service.name)} to ${file}?`)) return;
    const name = service.name;
    this.setMsg(name, 'Rolling back…', 'busy');
    try {
      await this.tauri.invoke('rollback_native_service_to', { serviceName: name, file });
      this.setMsg(name, 'Rolled back', 'ok');
      await this.loadServicesSilent();
      await this.loadManifest(name);
    } catch {
      this.setMsg(name, 'Rollback failed', 'error');
    }
  }

  /** Convenience for the template: history entries available as rollback
   *  targets, empty when the service was just installed. */
  rollbackTargets(name: string): JarEntry[] {
    return this.manifestByService[name]?.history ?? [];
  }

  // ── Initialize roles & permissions (auth bootstrap) ────────────────────

  /** Manually run auth's role/permission/root-user bootstrap (GET /init1 via
   *  the backend). Idempotent — skips when already initialized. */
  async initRoles(): Promise<void> {
    if (this.initRolesBusy) return;
    if (!confirm('Initialize roles, permissions and the root user?\n\nThis is safe to run again — it is skipped automatically if already set up.')) {
      return;
    }
    this.initRolesBusy = true;
    try {
      await this.tauri.invoke('setup_init_auth');
      this.notification.success('Roles & permissions initialized');
    } catch {
      // TauriService already surfaced the error toast.
    } finally {
      this.initRolesBusy = false;
    }
  }

  // ── Port Tools / Process Explorer ──────────────────────────────────────

  toggleProcessExplorer(): void {
    this.processExplorerOpen = !this.processExplorerOpen;
    // Lazy-load on first open so the panel doesn't pay the cost
    // (netstat/lsof + sysinfo enumeration) until the operator wants it.
    if (this.processExplorerOpen && this.processes.length === 0) {
      this.refreshProcesses();
    }
  }

  async refreshProcesses(): Promise<void> {
    this.processesLoading = true;
    try {
      this.processes = await this.tauri.invoke<ProcessInfo[]>('list_puru_processes');
    } catch {
      // TauriService already showed the error toast.
    } finally {
      this.processesLoading = false;
    }
  }

  async killProcess(p: ProcessInfo): Promise<void> {
    const portStr = p.listening_ports.length
      ? ` (listening on ${p.listening_ports.join(', ')})`
      : '';
    const labelStr = p.label ? ` — ${p.label}` : '';
    if (!confirm(`Kill PID ${p.pid}${labelStr}${portStr}?\n\nThis sends SIGKILL — no graceful shutdown.`)) {
      return;
    }
    this.killing[p.pid] = true;
    try {
      await this.tauri.invoke('kill_process_by_pid', { pid: p.pid });
      this.notification.success(`Killed PID ${p.pid}`);
      // Drop it from the table optimistically; refresh confirms.
      this.processes = this.processes.filter(x => x.pid !== p.pid);
      await this.refreshProcesses();
    } catch {
      // TauriService already showed the error toast.
    } finally {
      delete this.killing[p.pid];
    }
  }
}
