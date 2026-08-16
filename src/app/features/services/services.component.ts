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
          @if (isNative && updatableServices.length > 0) {
            <button class="btn btn-stroked" (click)="checkAllUpdates()" [disabled]="batchBusy" title="Check every service for a newer build">
              <span class="material-icons">system_update</span>
              Check updates
            </button>
            @if (availableCount > 0) {
              <button class="btn btn-stroked" (click)="downloadAllUpdates()" [disabled]="batchBusy" title="Download all available updates (services keep running)">
                <span class="material-icons">cloud_download</span>
                Download all ({{ availableCount }})
              </button>
            }
            @if (stagedCount > 0) {
              <button class="btn btn-primary" (click)="applyAllUpdates()" [disabled]="batchBusy" title="Apply all downloaded updates (stops, swaps, restarts each)">
                <span class="material-icons">restart_alt</span>
                Apply all ({{ stagedCount }})
              </button>
            }
          }
          <button class="btn btn-primary" (click)="startAll()" [disabled]="services.length === 0">
            <span class="material-icons">play_arrow</span>
            Start All
          </button>
        </div>
      </div>

      <!-- Port Tools / Process Explorer — Puru-scoped Task Manager -->
      <div class="card pt-card">
        <div class="pt-header" (click)="toggleProcessExplorer()">
          <div class="pt-title">
            <span class="material-icons">build</span>
            <span>Port Tools</span>
            <span class="pt-sub">Find &amp; kill processes holding Puru ports</span>
          </div>
          <div class="pt-header-actions" (click)="$event.stopPropagation()">
            @if (processExplorerOpen) {
              <button class="btn btn-stroked" (click)="refreshProcesses()" [disabled]="processesLoading">
                <span class="material-icons">refresh</span>
                {{ processesLoading ? 'Scanning...' : 'Refresh' }}
              </button>
            }
            <span class="material-icons pt-toggle">
              {{ processExplorerOpen ? 'expand_less' : 'expand_more' }}
            </span>
          </div>
        </div>
        @if (processExplorerOpen) {
          <div class="pt-body">
            @if (processesLoading && processes.length === 0) {
              <div class="pt-loading"><span class="spinner"></span></div>
            } @else if (processes.length === 0) {
              <div class="pt-empty">
                No Puru-relevant processes found (java / nginx / mysqld / rabbitmq / beam).
              </div>
            } @else {
              <table class="data-table pt-table">
                <thead>
                  <tr>
                    <th>PID</th>
                    <th>Process</th>
                    <th>Listening Ports</th>
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
                          @if (p.label) {
                            <span class="pt-label">{{ p.label }}</span>
                          }
                        </div>
                        @if (p.cmd) {
                          <div class="pt-cmd" [title]="p.cmd">{{ p.cmd }}</div>
                        }
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
                          {{ killing[p.pid] ? 'Killing...' : 'Kill' }}
                        </button>
                      </td>
                    </tr>
                  }
                </tbody>
              </table>
              <div class="pt-foot">
                Showing {{ processes.length }} {{ processes.length === 1 ? 'process' : 'processes' }}.
                Kill sends SIGKILL (Unix) or <code>taskkill /F /T</code> (Windows) — the process gets no chance to clean up.
              </div>
            }
          </div>
        }
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
                          @if (isNative && service.name !== 'puru-hydrogen' && service.status !== 'notinstalled') {
                            <button class="menu-item" (click)="checkUpdate(service); openMenu = null">
                              <span class="material-icons menu-green">system_update</span> Check for update
                            </button>
                          }
                          @if (isNative) {
                            <button class="menu-item" (click)="rollbackService(service); openMenu = null">
                              <span class="material-icons menu-orange">undo</span> Rollback
                            </button>
                          }
                        </div>
                      }
                    </div>
                  </td>
                </tr>
                @if (updateFlow[service.name]; as up) {
                  <tr class="update-progress-row" [class.uf-error]="up.phase === 'error'">
                    <td colspan="7">
                      <div class="update-progress">
                        <div class="up-head">
                          <span class="up-msg">
                            <span class="material-icons uf-icon">
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
                            {{ up.message }}
                          </span>
                          <span class="up-actions">
                            @if (up.phase === 'downloading' && (up.total || 0) > 0) {
                              <span class="up-bytes">{{ fmtMB(up.downloaded || 0) }} / {{ fmtMB(up.total || 0) }} MB</span>
                            }
                            @if (up.phase === 'available') {
                              <button class="uf-btn primary" (click)="downloadUpdate(service)" [disabled]="batchBusy">
                                <span class="material-icons">cloud_download</span> Download
                              </button>
                              <button class="uf-btn ghost" (click)="dismissFlow(service)">Later</button>
                            }
                            @if (up.phase === 'staged') {
                              <button class="uf-btn primary" (click)="applyUpdate(service)" [disabled]="batchBusy">
                                <span class="material-icons">restart_alt</span> Apply &amp; restart
                              </button>
                              <button class="uf-btn ghost" (click)="discardUpdate(service)" [disabled]="batchBusy">Discard</button>
                            }
                            @if (up.phase === 'up-to-date' || up.phase === 'done' || up.phase === 'error') {
                              <button class="uf-btn ghost" (click)="dismissFlow(service)">Dismiss</button>
                            }
                          </span>
                        </div>
                        @if (up.phase === 'downloading' || up.phase === 'applying' || up.phase === 'done' || up.phase === 'staged') {
                          <puru-progress [value]="up.percent || 0" [state]="up.progressState || 'active'" [height]="6"></puru-progress>
                        }
                      </div>
                    </td>
                  </tr>
                }
              }
            </tbody>
          </table>
        </div>
      }
    </div>
  `,
  styles: [`
    .update-progress-row td {
      padding: 10px 16px 14px;
      background: var(--bg-subtle, rgba(0,158,251,0.04));
      border-top: none;
    }
    .update-progress { display: flex; flex-direction: column; gap: 7px; }
    .up-head {
      display: flex;
      justify-content: space-between;
      align-items: center;
      gap: 12px;
      font-size: 0.82rem;
    }
    .up-msg { color: var(--text-secondary, #5a6472); font-weight: 500; display: inline-flex; align-items: center; gap: 6px; }
    .up-msg .uf-icon { font-size: 17px; color: var(--brand-blue, #009efb); }
    .update-progress-row.uf-error .uf-icon { color: var(--brand-red, #e83a3a); }
    .up-actions { display: inline-flex; align-items: center; gap: 8px; white-space: nowrap; }
    .up-bytes { color: var(--text-muted, #8a94a3); font-variant-numeric: tabular-nums; }
    .uf-btn {
      display: inline-flex; align-items: center; gap: 4px;
      padding: 3px 10px; font-size: 0.78rem; font-weight: 600;
      border-radius: 6px; cursor: pointer; border: 1px solid transparent;
      transition: background 0.15s ease, color 0.15s ease;
    }
    .uf-btn .material-icons { font-size: 15px; }
    .uf-btn.primary { color: #fff; background: var(--brand-blue, #009efb); }
    .uf-btn.primary:hover:not(:disabled) { filter: brightness(0.94); }
    .uf-btn.ghost { color: var(--text-secondary, #5a6472); background: transparent; border-color: var(--border, #d5dbe3); }
    .uf-btn.ghost:hover:not(:disabled) { background: var(--bg-hover, rgba(0,0,0,0.04)); }
    .uf-btn:disabled { opacity: 0.5; cursor: default; }

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
    /* Transient inline status just below the service name. */
    .action-msg {
      display: inline-flex;
      align-items: center;
      gap: 4px;
      margin-top: 3px;
      font-size: 0.72rem;
      font-weight: 600;
    }
    .action-msg .material-icons { font-size: 13px; }
    .action-msg.am-busy { color: var(--text-muted, #8a94a3); }
    .action-msg.am-ok { color: var(--brand-green, #2e9e5b); }
    .action-msg.am-error { color: var(--brand-red, #e83a3a); }

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

    /* ── Port Tools / Process Explorer ────────── */
    .pt-card {
      margin-bottom: 16px;
      padding: 0 !important;
      overflow: hidden;
    }
    .pt-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 12px 16px;
      cursor: pointer;
      user-select: none;
      background: #f8fafc;
      border-bottom: 1px solid transparent;
      transition: background 0.15s;
      &:hover { background: #f1f5f9; }
    }
    .pt-title {
      display: flex;
      align-items: baseline;
      gap: 8px;
      font-size: 0.9rem;
      font-weight: 600;
      .material-icons { font-size: 18px; align-self: center; color: #64748b; }
    }
    .pt-sub {
      font-weight: 400;
      color: #64748b;
      font-size: 0.78rem;
      margin-left: 4px;
    }
    .pt-header-actions {
      display: flex;
      align-items: center;
      gap: 8px;
    }
    .pt-toggle { color: #64748b; }
    .pt-body { border-top: 1px solid #e2e8f0; }
    .pt-loading {
      display: flex;
      justify-content: center;
      padding: 24px;
    }
    .pt-empty {
      padding: 16px;
      color: #64748b;
      font-size: 0.85rem;
      text-align: center;
    }
    .pt-table th, .pt-table td { font-size: 0.82rem; }
    .pid {
      font-family: 'SF Mono', 'Fira Code', monospace;
      font-size: 0.8rem;
    }
    .pt-name {
      display: flex;
      align-items: center;
      gap: 8px;
    }
    .pt-bin {
      font-family: 'SF Mono', 'Fira Code', monospace;
      color: #1e293b;
      font-size: 0.82rem;
    }
    .pt-label {
      display: inline-block;
      background: #eef2ff;
      color: #4338ca;
      padding: 2px 8px;
      border-radius: 4px;
      font-size: 0.7rem;
      font-weight: 600;
    }
    .pt-cmd {
      font-family: 'SF Mono', 'Fira Code', monospace;
      font-size: 0.7rem;
      color: #94a3b8;
      margin-top: 2px;
      max-width: 380px;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .pt-port {
      display: inline-block;
      background: #f1f5f9;
      color: #334155;
      padding: 2px 8px;
      border-radius: 4px;
      font-family: 'SF Mono', 'Fira Code', monospace;
      font-size: 0.75rem;
      margin-right: 4px;
      margin-bottom: 2px;
    }
    .btn-danger {
      background: #dc2626;
      color: #fff;
      border: 1px solid #dc2626;
      &:hover:not([disabled]) { background: #b91c1c; border-color: #b91c1c; }
      &[disabled] { opacity: 0.55; cursor: not-allowed; }
    }
    .btn-sm {
      padding: 4px 10px;
      font-size: 0.78rem;
      .material-icons { font-size: 14px; }
    }
    .pt-foot {
      padding: 8px 16px 12px;
      font-size: 0.72rem;
      color: #94a3b8;
      code {
        background: #f1f5f9;
        padding: 1px 4px;
        border-radius: 3px;
        font-size: 0.7rem;
      }
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

  sortKey = 'name';
  sortDir: 1 | -1 = 1;
  openMenu: string | null = null;

  // ── Port Tools / Process Explorer ────────────
  processExplorerOpen = false;
  processes: ProcessInfo[] = [];
  processesLoading = false;
  killing: Record<number, boolean> = {};

  get runningCount(): number {
    return this.services.filter(s => s.status === 'running').length;
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
      this.services.map(s => `${s.name}:${s.status}:${s.health || ''}`).join(',');
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
    this.loadServices().then(() => this.loadStagedUpdates());

    // Live progress for the download + apply phases of the update flow.
    listen<JarUpdateProgress>('jar-update-progress', (ev) => {
      const p = ev.payload;
      if (!p.service) return; // batch-wide events (e.g. setup) have no single service
      const cur = this.updateFlow[p.service] ?? { phase: 'downloading', message: '' };
      const indeterminate = p.phase === 'downloading' && p.total === 0;

      if (p.phase === 'downloading') {
        this.updateFlow[p.service] = {
          ...cur, phase: 'downloading', message: p.message,
          downloaded: p.downloaded, total: p.total, percent: p.percent,
          progressState: indeterminate ? 'indeterminate' : 'active',
        };
      } else if (p.phase === 'staged') {
        this.updateFlow[p.service] = {
          ...cur, phase: 'staged', message: p.message, percent: 100, progressState: 'complete',
        };
      } else if (p.phase === 'stopping' || p.phase === 'starting') {
        this.updateFlow[p.service] = {
          ...cur, phase: 'applying', message: p.message, percent: p.percent, progressState: 'active',
        };
      } else if (p.phase === 'done') {
        this.updateFlow[p.service] = { ...cur, phase: 'done', message: p.message, percent: 100, progressState: 'complete' };
        const svc = p.service;
        setTimeout(() => { if (this.updateFlow[svc]?.phase === 'done') delete this.updateFlow[svc]; }, 2500);
      } else if (p.phase === 'error') {
        this.updateFlow[p.service] = { ...cur, phase: 'error', message: p.message, progressState: 'fail' };
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

  /** Services eligible for JAR updates (native, installed, not the hydrogen bundle). */
  get updatableServices(): ServiceInfo[] {
    return this.sortedServices.filter(s =>
      this.isNative && s.name !== 'puru-hydrogen' && s.status !== 'notinstalled');
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
            message: `Downloaded build ${staged.short_sha} (${staged.size_mb.toFixed(1)} MB) — ready to apply`,
          };
        }
      } catch { /* ignore */ }
    }
  }

  /** Step 1: identify. */
  async checkUpdate(service: ServiceInfo): Promise<void> {
    const name = service.name;
    this.updateFlow[name] = { phase: 'checking', message: 'Checking for updates…' };
    try {
      const c = await this.tauri.invoke<JarUpdateCheck>('check_service_update', { serviceName: name });
      this.updateFlow[name] = c.update_available
        ? { phase: 'available', message: `Update available: ${c.current_sha || 'none'} → ${c.latest_sha}`,
            currentSha: c.current_sha, latestSha: c.latest_sha }
        : { phase: 'up-to-date', message: `Up to date (${c.current_sha || c.latest_sha})`, currentSha: c.current_sha };
    } catch (e) {
      this.updateFlow[name] = { phase: 'error', message: String(e), progressState: 'fail' };
    }
  }

  /** Step 2: download (stage) — running service untouched. */
  async downloadUpdate(service: ServiceInfo): Promise<void> {
    const name = service.name;
    const prev = this.updateFlow[name];
    this.updateFlow[name] = { phase: 'downloading', message: 'Starting download…', percent: 0,
      progressState: 'active', latestSha: prev?.latestSha };
    try {
      const s = await this.tauri.invoke<StagedUpdate>('download_service_update', { serviceName: name });
      this.updateFlow[name] = { phase: 'staged', percent: 100, progressState: 'complete',
        latestSha: s.short_sha,
        message: `Downloaded build ${s.short_sha} (${s.size_mb.toFixed(1)} MB) — ready to apply` };
    } catch (e) {
      if (this.updateFlow[name]?.phase !== 'error') {
        this.updateFlow[name] = { phase: 'error', message: String(e), progressState: 'fail' };
      }
    }
  }

  /** Step 3: apply — stop → swap → restart. */
  async applyUpdate(service: ServiceInfo, skipConfirm = false): Promise<void> {
    const name = service.name;
    if (!skipConfirm && !confirm(`Apply the downloaded update for ${name}? This stops the service, swaps the JAR, and restarts it.`)) return;
    this.updateFlow[name] = { phase: 'applying', message: 'Applying update…', percent: 10, progressState: 'active' };
    try {
      const r = await this.tauri.invoke<any>('apply_service_update', { serviceName: name });
      this.updateFlow[name] = { phase: 'done', message: `Applied build ${r.short_sha}`, percent: 100, progressState: 'complete' };
      await this.loadServicesSilent();
      const svc = name;
      setTimeout(() => { if (this.updateFlow[svc]?.phase === 'done') delete this.updateFlow[svc]; }, 2500);
    } catch (e) {
      if (this.updateFlow[name]?.phase !== 'error') {
        this.updateFlow[name] = { phase: 'error', message: String(e), progressState: 'fail' };
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
    if (!confirm(`Apply ${targets.length} downloaded update(s)? Each service is stopped, swapped, and restarted one at a time.`)) return;
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


  async rollbackService(service: ServiceInfo): Promise<void> {
    if (!confirm(`Rollback ${service.name} to previous JAR?`)) return;
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
