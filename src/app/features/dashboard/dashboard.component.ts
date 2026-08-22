import { Component, OnInit, OnDestroy, HostListener, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { Router } from '@angular/router';
import { TauriService, ServiceInfo, SystemInfo, DaemonStatus, NetworkStatus, SpeedTestResult, BackupRecord } from '../../core/services/tauri.service';
import { License } from '../../core/models/license.model';
import { HospitalAlert, alertCategoryLabel } from '../../core/models/hospital.model';
import { PuruLogoComponent } from '../../core/components/puru-logo.component';
import { LogViewerDialogComponent } from '../../core/components/log-viewer-dialog.component';
import { NotificationService } from '../../core/services/notification.service';
import { interval, Subscription } from 'rxjs';

interface ActivityItem { icon: string; kind: 'ok' | 'warn' | 'info'; text: string; time: string; }

@Component({
  selector: 'app-dashboard',
  standalone: true,
  imports: [CommonModule, PuruLogoComponent, LogViewerDialogComponent],
  template: `
    <div class="page">
      <!-- Puru is live at — friendly URLs staff type to open the app -->
      @if (liveAt) {
        <div class="live-at" [class.warn]="!liveAt.hosts?.hosts_configured">
          <div class="la-head">
            <span class="material-icons">public</span>
            <div>
              <div class="la-title">Puru is live at</div>
              <div class="la-sub">
                @if (liveAt.hosts?.hosts_configured && liveAt.mdns?.running) {
                  Staff on this LAN can use any of these URLs.
                } @else if (!liveAt.hosts?.hosts_configured) {
                  Hostnames not configured on this server yet — run the
                  "Configure local domain" step in Setup.
                } @else {
                  mDNS responder not running — clients need to use the IP.
                }
              </div>
            </div>
          </div>
          <div class="la-urls">
            @for (u of liveUrls(); track u.url) {
              <div class="la-url" [class.la-dim]="u.hint">
                <a [href]="u.url" target="_blank" rel="noopener">{{ u.url }}</a>
                <span class="la-note">{{ u.note }}</span>
                <button class="la-copy" (click)="copyUrl(u.url)" title="Copy">
                  <span class="material-icons">{{ copiedUrl === u.url ? 'check' : 'content_copy' }}</span>
                </button>
              </div>
            }
          </div>
        </div>
      }

      <!-- Metric cards -->
      <div class="cards">
        <div class="metric">
          <div class="lab"><span class="material-icons">developer_board</span>CPU</div>
          <div class="val">{{ telemetry ? (telemetry.cpu_percent | number:'1.0-1') : '—' }}<small>%</small></div>
          <div class="bar"><i [class]="barClass(telemetry?.cpu_percent, 50, 80)" [style.width.%]="telemetry?.cpu_percent || 0"></i></div>
        </div>
        <div class="metric hi">
          <div class="lab"><span class="material-icons">memory</span>Memory</div>
          <div class="val">{{ ramPct }}<small>% · {{ ramUsed }}/{{ ramTotal }} GB</small></div>
          <div class="bar"><i [class]="barClass(ramPct, 60, 85)" [style.width.%]="ramPct"></i></div>
        </div>
        <div class="metric">
          <div class="lab"><span class="material-icons">storage</span>Disk</div>
          <div class="val">{{ telemetry ? (telemetry.disk_percent | number:'1.0-1') : '—' }}<small>% · {{ diskFree }} GB free</small></div>
          <div class="bar"><i [class]="barClass(telemetry?.disk_percent, 70, 90)" [style.width.%]="telemetry?.disk_percent || 0"></i></div>
        </div>
        <div class="metric">
          <div class="lab"><span class="material-icons">wifi</span>Network</div>
          <div class="val">{{ networkStatValue }}</div>
          <div class="bar"><i class="g" [style.width.%]="networkStatus?.connected ? 20 : 0"></i></div>
        </div>
        <div class="metric">
          <div class="lab"><span class="material-icons">dns</span>Services</div>
          <div class="val">{{ runningApp }} <small>/ {{ appServices.length }} up</small></div>
          <div class="bar"><i [class]="runningApp === appServices.length ? 'g' : 'a'" [style.width.%]="servicePct"></i></div>
        </div>
        <div class="metric">
          <div class="lab"><span class="material-icons">schedule</span>Uptime</div>
          <div class="val">{{ systemUptime }}</div>
          <div class="bar"><i class="g" style="width:100%"></i></div>
        </div>
      </div>

      <div class="grid2">
        <!-- Services -->
        <div class="panel svc-panel" [class.manage-on]="manageOn">
          <div class="phead">
            <h3>Services</h3>
            <button class="toggle" [class.on]="manageOn" (click)="manageOn = !manageOn">
              <span class="material-icons">tune</span>Manage
            </button>
          </div>
          @if (servicesLoading) {
            <div class="pad-mid"><span class="spinner"></span></div>
          } @else if (appServices.length === 0) {
            <div class="pad-mid muted">No services installed yet.</div>
          } @else {
            <table>
              <thead><tr><th>Service</th><th>Status</th><th>Uptime</th><th></th></tr></thead>
              <tbody>
                @for (s of appServices; track s.name) {
                  <tr>
                    <td>
                      <div class="svc">
                        <span class="sdot" [class]="'d-' + s.status"></span>
                        <span>
                          {{ displayName(s.name) }}
                          @if (actionMsg[s.name]; as m) { <span class="amsg" [class]="'am-' + m.kind">· {{ m.text }}</span> }
                        </span>
                      </div>
                    </td>
                    <td><span class="pill" [class]="'p-' + s.status">{{ statusLabel(s) }}</span></td>
                    <td class="up">{{ s.uptime || '—' }}</td>
                    <td class="kebab-wrap">
                      <button class="kebab" [class.open]="openMenu === s.name" (click)="toggleMenu(s.name, $event)">
                        <span class="material-icons">more_vert</span>
                      </button>
                      @if (openMenu === s.name) {
                        <div class="kmenu" (click)="$event.stopPropagation()">
                          <button (click)="viewLogs(s)"><span class="material-icons">article</span>View logs</button>
                          <button (click)="copyLogs(s)"><span class="material-icons">content_copy</span>Copy logs</button>
                          <div class="div"></div>
                          <button (click)="checkUpdate()"><span class="material-icons">system_update</span>Check update</button>
                          <button (click)="rollback(s)"><span class="material-icons">undo</span>Roll back</button>
                          <div class="div"></div>
                          @if (s.status === 'running') {
                            <button (click)="act(s, 'restart_service', 'Restarting…', 'Restarted')"><span class="material-icons">refresh</span>Restart</button>
                            <button (click)="act(s, 'stop_service', 'Stopping…', 'Stopped')"><span class="material-icons">stop</span>Stop</button>
                          } @else {
                            <button (click)="act(s, 'start_service', 'Starting…', 'Started')"><span class="material-icons">play_arrow</span>Start</button>
                          }
                          <div class="div"></div>
                          <button class="danger" (click)="kill(s)"><span class="material-icons">close</span>Kill process</button>
                        </div>
                      }
                    </td>
                  </tr>
                }
              </tbody>
            </table>
          }
        </div>

        <div class="col">
          <!-- Backups -->
          <div class="panel">
            <div class="phead"><h3>Backups</h3><button class="gobtn" (click)="openBackups()">Open <span class="material-icons">arrow_forward</span></button></div>
            <div class="bigrow"><span class="b">{{ lastBackupTime || 'No backups yet' }}</span>@if (lastBackupTime) {<span class="s">Full · uploaded to cloud</span>}</div>
            <div class="kv"><span>Backups kept</span><span>{{ backupHistory.length }}</span></div>
            <div class="pfoot"><button class="btn primary" [disabled]="busyBackup" (click)="backupNow()"><span class="material-icons">backup</span>{{ busyBackup ? 'Starting…' : 'Back up now' }}</button></div>
          </div>

          <!-- Alerts -->
          <div class="panel">
            <div class="phead"><h3>Alerts @if (openAlerts.length) {<span class="count a">{{ openAlerts.length }}</span>}</h3><button class="gobtn" (click)="goto('/alerts')">All alerts <span class="material-icons">arrow_forward</span></button></div>
            @if (openAlerts.length === 0) {
              <div class="pad-mid muted small">Nothing needs your attention.</div>
            } @else {
              <table class="alerts"><tbody>
                @for (a of openAlerts.slice(0, 3); track a.id) {
                  <tr>
                    <td style="width:3px;padding-right:0"><div class="sev" [class]="'sev-' + a.severity"></div></td>
                    <td><div class="msg">{{ a.title }}</div><div class="sub">{{ alertCategoryLabel(a.category) }} · {{ timeAgo(toDate(a.created_at)) }}</div></td>
                    <td style="text-align:right"><button class="ackbtn" (click)="ack(a)">Ack</button></td>
                  </tr>
                }
              </tbody></table>
            }
            <!-- Recovery is news too: a problem that fixed itself should say so
                 rather than just vanishing from the list. -->
            @if (recentlyResolved.length) {
              <table class="alerts resolved-list"><tbody>
                @for (a of recentlyResolved.slice(0, 2); track a.id) {
                  <tr>
                    <td style="width:3px;padding-right:0"><div class="sev sev-resolved"></div></td>
                    <td><div class="msg">{{ a.title }}</div><div class="sub">Resolved {{ timeAgo(toDate(a.resolved_at || a.created_at)) }}</div></td>
                    <td style="text-align:right"><span class="material-icons resolved-tick">task_alt</span></td>
                  </tr>
                }
              </tbody></table>
            }
          </div>

          <!-- Connectivity -->
          <div class="panel">
            <div class="phead">
              <h3>Connectivity</h3>
              <button class="gobtn" (click)="runSpeedTest()" [disabled]="speedTestRunning">
                @if (speedTestRunning) { <span class="spinner" style="width:14px;height:14px;border-width:2px"></span> }
                @else { <span class="material-icons">speed</span> }
                Speed test
              </button>
            </div>
            <div class="net-rows">
              <div class="net-row"><span class="net-label">Internet</span><span class="net-value" [class]="networkStatus?.connected ? 'nv-green' : 'nv-red'">{{ networkStatus?.connected ? 'Connected' : (networkStatus ? 'Offline' : '—') }}</span></div>
              <div class="net-row"><span class="net-label">Latency</span><span class="net-value" [class]="latencyClass">{{ networkStatus?.latency_ms != null ? networkStatus!.latency_ms + ' ms' : '—' }}</span></div>
              <div class="net-row"><span class="net-label">DICOM server</span><span class="net-value" [class]="gcpClass">
                @if (!networkStatus) { — }
                @else if (networkStatus.gcp_reachable) { Reachable{{ networkStatus.gcp_latency_ms != null ? ' (' + networkStatus.gcp_latency_ms + ' ms)' : '' }} }
                @else { Unreachable }
              </span></div>
              @if (speedTestResult) {
                <div class="net-row"><span class="net-label">Download</span><span class="net-value" [class]="downloadClass">{{ speedTestResult.download_mbps != null ? (speedTestResult.download_mbps | number:'1.1-1') + ' Mbps' : '—' }}</span></div>
                <div class="net-row"><span class="net-label">Upload</span><span class="net-value" [class]="uploadClass">{{ speedTestResult.upload_mbps != null ? (speedTestResult.upload_mbps | number:'1.1-1') + ' Mbps' : '—' }}</span></div>
              }
            </div>
          </div>

          <!-- Recent activity -->
          <div class="panel">
            <div class="phead"><h3>Recent activity</h3></div>
            @if (activity.length === 0) {
              <div class="pad-mid muted small">No recent activity.</div>
            } @else {
              <div class="act">
                @for (a of activity; track $index) {
                  <div class="arow">
                    <div class="aic" [class]="'ai-' + a.kind"><span class="material-icons">{{ a.icon }}</span></div>
                    <div><div class="atext">{{ a.text }}</div><div class="atime">{{ a.time }}</div></div>
                  </div>
                }
              </div>
            }
          </div>
        </div>
      </div>
    </div>

    <!-- Backups drawer -->
    @if (backupsOpen) {
      <div class="scrim" (click)="backupsOpen = false"></div>
      <aside class="drawer">
        <div class="dhead"><h3>Backups</h3><button class="dclose" (click)="backupsOpen = false"><span class="material-icons">close</span></button></div>
        <div class="dbody">
          <div class="hero">
            <div class="t">Last backup</div>
            <div class="v">{{ lastBackupTime || 'No backups yet' }}</div>
          </div>
          <div class="drow">
            <button class="btn primary" [disabled]="busyBackup" (click)="backupNow()"><span class="material-icons">backup</span>Back up now</button>
            <button class="btn" (click)="goto('/backups')"><span class="material-icons">restore</span>Restore</button>
          </div>
          <div class="dlabel">History</div>
          @if (backupHistory.length === 0) {
            <div class="muted small">No backups recorded yet.</div>
          } @else {
            <div class="hist">
              @for (b of backupHistory.slice(0, 12); track b.id) {
                <div class="h">
                  <div class="hi-ic" [class.fail]="b.status !== 'completed'"><span class="material-icons">{{ b.status === 'completed' ? 'check' : 'error_outline' }}</span></div>
                  <div><div>{{ b.type === 'full' ? 'Full backup' : 'Partial backup' }}</div><div class="meta">{{ timeAgo(toDate(b.created_at)) }}</div></div>
                  <div class="sz">{{ b.size_mb ? (b.size_mb | number:'1.0-0') + ' MB' : '' }}</div>
                </div>
              }
            </div>
          }
          <div class="dfoot"><button class="btn wide" (click)="goto('/backups')">Open full backups page <span class="material-icons">arrow_forward</span></button></div>
        </div>
      </aside>
    }

    <!-- Per-service log viewer dialog -->
    @if (logService) {
      <app-log-viewer-dialog
        [serviceName]="logService.name"
        [title]="displayName(logService.name)"
        [infra]="!!logService.infra"
        (close)="logService = null">
      </app-log-viewer-dialog>
    }
  `,
  styles: [`
    .page{max-width:1220px;margin:0 auto;padding:22px 26px 40px;}
    .material-icons{font-size:18px;line-height:1;}

    .topbar{display:flex;align-items:center;justify-content:space-between;gap:16px;padding-bottom:16px;border-bottom:1px solid var(--border);margin-bottom:20px;}
    .brand{display:flex;align-items:center;gap:13px;}
    .bdiv{width:1px;height:26px;background:var(--border);}
    .hname{font-size:22px;font-weight:700;letter-spacing:-.4px;color:var(--text-primary);}
    .top-actions{display:flex;align-items:center;gap:10px;}
    .ibtn{position:relative;height:40px;width:40px;border-radius:10px;border:1px solid var(--border);background:var(--card-bg,#fff);display:flex;align-items:center;justify-content:center;cursor:pointer;color:var(--text-secondary);}
    .ibtn.wide{width:auto;padding:0 15px;gap:8px;font-size:14px;color:var(--text-primary);}
    .ibtn:hover{border-color:var(--brand-blue,#009efb);color:var(--text-primary);}
    .ibtn .dot{position:absolute;top:8px;right:9px;width:8px;height:8px;border-radius:50%;background:var(--brand-blue,#009efb);border:2px solid var(--card-bg,#fff);}
    .prof{position:relative;}
    .pavatar{width:40px;height:40px;border-radius:50%;background:#e6f4fe;color:#0a6cad;display:flex;align-items:center;justify-content:center;font-weight:600;font-size:14px;cursor:pointer;}
    .pmenu{position:absolute;top:calc(100% + 8px);right:0;z-index:20;width:190px;background:var(--card-bg,#fff);border:1px solid var(--border);border-radius:11px;padding:6px;box-shadow:0 14px 40px rgba(2,12,27,.18);}
    .pmenu button{display:flex;width:100%;align-items:center;gap:10px;padding:9px 10px;border:0;background:none;border-radius:7px;font-size:13.5px;color:var(--text-primary);cursor:pointer;text-align:left;font-family:inherit;}
    .pmenu button:hover{background:var(--bg-hover,#f4f7fb);}
    .pmenu .material-icons{color:var(--text-secondary);}

    .cards{display:grid;grid-template-columns:repeat(6,1fr);gap:12px;margin-bottom:18px;}
    .metric{background:var(--card-bg,#fff);border:1px solid var(--border);border-radius:13px;padding:13px 14px;}
    .metric.hi{border-color:var(--status-orange,#c67608);}
    .metric .lab{display:flex;align-items:center;gap:6px;font-size:12px;color:var(--text-secondary);}
    .metric .lab .material-icons{font-size:15px;}
    .metric .val{font-size:22px;font-weight:700;letter-spacing:-.5px;margin-top:7px;font-variant-numeric:tabular-nums;color:var(--text-primary);}
    .metric .val small{font-size:12px;color:var(--text-secondary);font-weight:400;}
    .bar{height:5px;border-radius:3px;background:var(--border);margin-top:9px;overflow:hidden;}
    .bar i{display:block;height:100%;border-radius:3px;background:var(--brand-blue,#009efb);}
    .bar i.g{background:var(--status-green,#149a63);} .bar i.a{background:var(--status-orange,#c67608);} .bar i.r{background:var(--status-red,#d94339);}

    .grid2{display:grid;grid-template-columns:1.55fr 1fr;gap:16px;align-items:start;}
    .col{display:flex;flex-direction:column;gap:16px;}
    .panel{background:var(--card-bg,#fff);border:1px solid var(--border);border-radius:14px;}
    .phead{display:flex;align-items:center;justify-content:space-between;padding:14px 16px 12px;}
    .phead h3{margin:0;font-size:15px;font-weight:600;display:flex;align-items:center;gap:8px;color:var(--text-primary);}
    .count{font-size:12px;font-weight:600;padding:1px 8px;border-radius:20px;} .count.a{background:#fbeed6;color:#c67608;}
    .pad-mid{padding:22px;display:flex;justify-content:center;} .muted{color:var(--text-muted);} .small{font-size:13px;}

    .gobtn,.toggle{display:inline-flex;align-items:center;gap:6px;font-size:13px;padding:5px 12px;border-radius:8px;border:1px solid var(--border);background:var(--card-bg,#fff);cursor:pointer;color:var(--text-primary);}
    .gobtn .material-icons,.toggle .material-icons{font-size:15px;}
    .gobtn:hover{border-color:var(--brand-blue,#009efb);color:var(--brand-blue,#009efb);}
    .toggle.on{border-color:var(--brand-blue,#009efb);background:#e6f4fe;color:#0a6cad;font-weight:500;}

    table{width:100%;border-collapse:collapse;}
    thead th{font-size:11px;letter-spacing:.6px;text-transform:uppercase;color:var(--text-muted);text-align:left;font-weight:500;padding:6px 16px;border-top:1px solid var(--border);border-bottom:1px solid var(--border);}
    tbody td{padding:11px 16px;border-bottom:1px solid var(--border);font-size:14px;vertical-align:middle;color:var(--text-primary);}
    tbody tr:last-child td{border-bottom:0;}
    .svc{display:flex;align-items:center;gap:10px;font-weight:500;}
    .sdot{width:8px;height:8px;border-radius:50%;flex-shrink:0;background:var(--text-muted);}
    .sdot.d-running{background:var(--status-green,#149a63);box-shadow:0 0 0 3px #e2f5ec;}
    .sdot.d-error,.sdot.d-stopped{background:var(--status-red,#d94339);}
    .sdot.d-starting{background:var(--status-orange,#c67608);box-shadow:0 0 0 3px #fbeed6;}
    .amsg{font-size:12px;font-weight:500;margin-left:4px;} .am-busy{color:var(--text-muted);} .am-ok{color:var(--status-green);} .am-error{color:var(--status-red);}
    .pill{display:inline-flex;font-size:12.5px;font-weight:500;padding:3px 10px;border-radius:20px;background:#eef2f7;color:var(--text-secondary);}
    .pill.p-running{background:#e2f5ec;color:#149a63;} .pill.p-error{background:#fbe9e7;color:#d94339;} .pill.p-starting{background:#fbeed6;color:#c67608;}
    .up{font-variant-numeric:tabular-nums;color:var(--text-secondary);}
    .kebab-wrap{position:relative;text-align:right;}
    .kebab{width:32px;height:32px;border-radius:8px;border:1px solid var(--border);background:var(--card-bg,#fff);cursor:pointer;color:var(--text-secondary);display:none;align-items:center;justify-content:center;}
    .svc-panel.manage-on .kebab{display:inline-flex;}
    .kebab:hover,.kebab.open{border-color:var(--brand-blue,#009efb);color:#0a6cad;background:#e6f4fe;}
    .kmenu{position:absolute;top:calc(100% + 6px);right:0;z-index:15;width:182px;background:var(--card-bg,#fff);border:1px solid var(--border);border-radius:11px;padding:6px;box-shadow:0 16px 40px rgba(2,12,27,.2);}
    .kmenu button{display:flex;width:100%;align-items:center;gap:10px;padding:8px 10px;border:0;background:none;border-radius:7px;font-size:13.5px;color:var(--text-primary);cursor:pointer;text-align:left;font-family:inherit;}
    .kmenu button .material-icons{font-size:16px;color:var(--text-secondary);}
    .kmenu button:hover{background:var(--bg-hover,#f4f7fb);}
    .kmenu button.danger{color:var(--status-red,#d94339);} .kmenu button.danger .material-icons{color:var(--status-red,#d94339);}
    .kmenu .div{height:1px;background:var(--border);margin:5px 4px;}

    .bigrow{display:flex;align-items:baseline;gap:8px;padding:2px 16px 4px;}
    .bigrow .b{font-size:20px;font-weight:700;letter-spacing:-.4px;color:var(--text-primary);}
    .bigrow .s{font-size:13px;color:var(--text-secondary);}
    .kv{display:flex;justify-content:space-between;padding:7px 16px;font-size:13.5px;border-top:1px solid var(--border);color:var(--text-primary);}
    .kv span:first-child{color:var(--text-secondary);}
    .pfoot{padding:12px 16px;}
    .btn{display:inline-flex;align-items:center;justify-content:center;gap:7px;font-size:13.5px;padding:8px 14px;border-radius:9px;border:1px solid var(--border);background:var(--card-bg,#fff);cursor:pointer;color:var(--text-primary);}
    .btn .material-icons{font-size:16px;}
    .btn:hover{border-color:var(--brand-blue,#009efb);color:#0a6cad;}
    .btn.primary{background:var(--brand-blue,#009efb);border-color:var(--brand-blue,#009efb);color:#fff;} .btn.primary:hover{filter:brightness(.96);color:#fff;}
    .btn.primary:disabled{opacity:.6;cursor:default;}
    .btn.wide{width:100%;}

    .alerts .msg{color:var(--text-primary);} .alerts .sub{color:var(--text-secondary);font-size:12.5px;}
    .sev{width:3px;height:30px;border-radius:2px;background:var(--text-muted);}
    .sev-critical{background:var(--status-red,#d94339);} .sev-warning{background:var(--status-orange,#c67608);} .sev-info{background:var(--brand-blue,#009efb);}
    .sev-resolved{background:var(--status-green,#2e9e5b);}
    .resolved-list{opacity:.8;} .resolved-list .msg{color:var(--text-secondary);}
    .resolved-tick{font-size:17px;color:var(--status-green,#2e9e5b);}
    .ackbtn{font-size:12.5px;padding:5px 12px;border-radius:7px;border:1px solid var(--border);background:var(--card-bg,#fff);cursor:pointer;color:var(--text-secondary);}
    .ackbtn:hover{color:var(--text-primary);border-color:var(--brand-blue,#009efb);}

    .act{padding:4px 16px 12px;}
    .arow{display:flex;gap:12px;padding:9px 0;border-bottom:1px solid var(--border);} .arow:last-child{border-bottom:0;}
    .aic{width:30px;height:30px;border-radius:8px;display:flex;align-items:center;justify-content:center;flex-shrink:0;}
    .aic .material-icons{font-size:16px;}
    .ai-ok{background:#e2f5ec;color:#149a63;} .ai-warn{background:#fbeed6;color:#c67608;} .ai-info{background:#e6f4fe;color:#0a6cad;}
    .atext{font-size:13.5px;line-height:1.45;color:var(--text-primary);} .atime{font-size:12px;color:var(--text-muted);margin-top:1px;}

    .net-rows{padding:2px 16px 14px;}
    .net-row{display:flex;justify-content:space-between;align-items:center;padding:8px 0;border-bottom:1px solid var(--border);font-size:0.84rem;}
    .net-row:last-child{border-bottom:none;}
    .net-label{color:var(--text-secondary);font-weight:500;}
    .net-value{font-weight:600;color:var(--text-primary);}
    .net-value.nv-green{color:var(--status-green,#149a63);} .net-value.nv-red{color:var(--status-red,#d94339);} .net-value.nv-orange{color:var(--status-orange,#c67608);}

    .scrim{position:fixed;inset:0;background:rgba(10,17,30,.28);z-index:40;}
    .drawer{position:fixed;top:0;right:0;bottom:0;width:440px;z-index:41;background:var(--card-bg,#fff);border-left:1px solid var(--border);box-shadow:-16px 0 44px rgba(2,12,27,.22);display:flex;flex-direction:column;}
    .dhead{display:flex;align-items:center;justify-content:space-between;padding:16px 18px;border-bottom:1px solid var(--border);}
    .dhead h3{margin:0;font-size:17px;font-weight:600;color:var(--text-primary);}
    .dclose{width:34px;height:34px;border-radius:9px;border:1px solid var(--border);background:var(--card-bg,#fff);cursor:pointer;color:var(--text-secondary);display:flex;align-items:center;justify-content:center;}
    .dbody{padding:16px 18px;overflow:auto;flex:1;}
    .hero{background:var(--bg-subtle,#f6f8fc);border:1px solid var(--border);border-radius:12px;padding:14px 16px;margin-bottom:14px;}
    .hero .t{font-size:13px;color:var(--text-secondary);} .hero .v{font-size:20px;font-weight:700;margin-top:3px;color:var(--text-primary);}
    .drow{display:flex;gap:8px;margin-bottom:16px;} .drow .btn{flex:1;}
    .dlabel{font-size:12px;letter-spacing:.5px;text-transform:uppercase;color:var(--text-muted);margin-bottom:4px;}
    .hist{font-size:13.5px;}
    .hist .h{display:flex;align-items:center;gap:10px;padding:11px 2px;border-bottom:1px solid var(--border);color:var(--text-primary);} .hist .h:last-child{border-bottom:0;}
    .hi-ic{width:28px;height:28px;border-radius:7px;background:#e2f5ec;color:#149a63;display:flex;align-items:center;justify-content:center;} .hi-ic.fail{background:#fbe9e7;color:#d94339;}
    .hi-ic .material-icons{font-size:15px;}
    .hist .meta{color:var(--text-muted);font-size:12px;} .hist .sz{margin-left:auto;color:var(--text-secondary);font-variant-numeric:tabular-nums;}
    .dfoot{margin-top:16px;}
    .spinner{width:22px;height:22px;border:3px solid var(--border);border-top-color:var(--brand-blue,#009efb);border-radius:50%;animation:spin .8s linear infinite;} @keyframes spin{to{transform:rotate(360deg);}}

    @media (max-width:1080px){.cards{grid-template-columns:repeat(3,1fr);}.grid2{grid-template-columns:1fr;}.drawer{width:100%;}}

    /* ── Puru is live at ─────────────────────────────────────────── */
    .live-at{background:linear-gradient(90deg,#e6f4fe 0%,#f0f9ff 100%);border:1px solid #b6e3fd;border-radius:12px;padding:16px 20px;margin-bottom:18px;display:flex;flex-wrap:wrap;gap:24px;align-items:center;}
    .live-at.warn{background:#fff7ed;border-color:#fed7aa;}
    .la-head{display:flex;align-items:flex-start;gap:12px;flex-shrink:0;}
    .la-head .material-icons{font-size:24px;color:#0a6cad;flex-shrink:0;margin-top:2px;}
    .live-at.warn .la-head .material-icons{color:#ea580c;}
    .la-title{font-weight:700;font-size:0.95rem;color:var(--text-primary);}
    .la-sub{font-size:0.78rem;color:var(--text-secondary);margin-top:2px;}
    .la-urls{display:flex;flex-wrap:wrap;gap:8px 16px;flex:1;min-width:280px;}
    .la-url{display:flex;align-items:center;gap:8px;padding:6px 10px;background:rgba(255,255,255,0.7);border:1px solid rgba(255,255,255,0.9);border-radius:8px;}
    .la-url.la-dim{background:transparent;border-color:transparent;}
    .la-url a{font-family:'JetBrains Mono','SF Mono',Menlo,monospace;font-size:0.85rem;color:#0a6cad;font-weight:600;text-decoration:none;}
    .la-url a:hover{text-decoration:underline;}
    .la-note{font-size:0.72rem;color:var(--text-muted);}
    .la-copy{background:none;border:none;cursor:pointer;color:var(--text-secondary);padding:2px 4px;border-radius:4px;display:inline-flex;}
    .la-copy:hover{background:rgba(0,0,0,0.05);color:var(--text-primary);}
    .la-copy .material-icons{font-size:15px;}
  `]
})
export class DashboardComponent implements OnInit, OnDestroy {
  private tauri = inject(TauriService);
  private router = inject(Router);
  private notify = inject(NotificationService);

  systemInfo: SystemInfo | null = null;
  services: ServiceInfo[] = [];
  servicesLoading = true;
  license: License | null = null;
  daemonRunning = false;
  telemetry: { cpu_percent: number; ram_gb: number; disk_percent: number } | null = null;
  lastBackupTime: string | null = null;
  backupHistory: BackupRecord[] = [];
  alerts: HospitalAlert[] = [];
  networkStatus: NetworkStatus | null = null;
  speedTestResult: SpeedTestResult | null = null;
  speedTestRunning = false;

  /** Snapshot returned by `get_live_at_status` — hostnames + LAN IP + mDNS. */
  liveAt: {
    hospital_code?: string;
    hosts?: {
      hosts_configured?: boolean;
      lan_ip?: string | null;
      hosts_path?: string;
      names?: { mdns_name?: string; router_name?: string } | null;
    };
    mdns?: { running?: boolean; hostname?: string | null; advertised_ip?: string | null };
  } | null = null;
  copiedUrl: string | null = null;

  manageOn = true;
  openMenu: string | null = null;
  profileOpen = false;
  backupsOpen = false;
  busyBackup = false;
  logService: ServiceInfo | null = null;
  actionMsg: Record<string, { text: string; kind: 'busy' | 'ok' | 'error' }> = {};

  private refreshSub?: Subscription;
  private networkSub?: Subscription;

  // ── Derived ──────────────────────────────────────────────────────────────
  get hospitalName(): string { return this.license?.hospital_name || 'Puru Hospital'; }
  get initials(): string {
    const n = this.hospitalName.trim().split(/\s+/);
    return ((n[0]?.[0] || 'P') + (n[1]?.[0] || '')).toUpperCase();
  }
  /** App services only (drop infra rows + the static bundles for the compact table). */
  get appServices(): ServiceInfo[] {
    return this.services.filter(s => !s.infra && s.name !== 'dviewer');
  }
  get runningApp(): number { return this.appServices.filter(s => s.status === 'running').length; }
  get servicePct(): number { return this.appServices.length ? Math.round(this.runningApp / this.appServices.length * 100) : 0; }
  alertCategoryLabel = alertCategoryLabel;
  /** Unacknowledged and still broken — a resolved alert needs no attention. */
  get openAlerts(): HospitalAlert[] { return this.alerts.filter(a => !a.acknowledged && !a.resolved); }
  /** Conditions that fixed themselves recently, newest first. */
  get recentlyResolved(): HospitalAlert[] {
    return this.alerts
      .filter(a => a.resolved)
      .sort((a, b) => this.toDate(b.resolved_at || b.created_at).getTime()
                    - this.toDate(a.resolved_at || a.created_at).getTime());
  }
  get ramTotal(): string { return this.systemInfo ? this.systemInfo.total_ram_gb.toFixed(0) : '—'; }
  get ramUsed(): string { return this.telemetry ? this.telemetry.ram_gb.toFixed(1) : '—'; }
  get ramPct(): number {
    if (!this.telemetry || !this.systemInfo || !this.systemInfo.total_ram_gb) return 0;
    return Math.round(this.telemetry.ram_gb / this.systemInfo.total_ram_gb * 100);
  }
  get diskFree(): string { return this.systemInfo ? this.systemInfo.disk_free_gb.toFixed(0) : '—'; }
  get networkStatValue(): string {
    if (!this.networkStatus) return '—';
    if (!this.networkStatus.connected) return 'Offline';
    return this.networkStatus.latency_ms != null ? `${this.networkStatus.latency_ms} ms` : 'Online';
  }
  get latencyClass(): string {
    const l = this.networkStatus?.latency_ms;
    if (l == null) return '';
    return l < 100 ? 'nv-green' : l < 300 ? 'nv-orange' : 'nv-red';
  }
  get gcpClass(): string {
    if (!this.networkStatus) return '';
    return this.networkStatus.gcp_reachable ? 'nv-green' : 'nv-red';
  }
  get downloadClass(): string {
    const d = this.speedTestResult?.download_mbps;
    if (d == null) return '';
    return d > 10 ? 'nv-green' : d > 2 ? 'nv-orange' : 'nv-red';
  }
  get uploadClass(): string {
    const u = this.speedTestResult?.upload_mbps;
    if (u == null) return '';
    return u > 5 ? 'nv-green' : u > 1 ? 'nv-orange' : 'nv-red';
  }
  get systemUptime(): string {
    const max = Math.max(0, ...this.appServices.filter(s => s.status === 'running').map(s => this.uptimeSecs(s.uptime)));
    if (!max) return '—';
    const d = Math.floor(max / 86400), h = Math.floor((max % 86400) / 3600), m = Math.floor((max % 3600) / 60);
    if (d) return `${d}d ${h}h`;
    if (h) return `${h}h ${m}m`;
    return `${m}m`;
  }
  get activity(): ActivityItem[] {
    const items: ActivityItem[] = [];
    const lastDone = this.backupHistory.filter(b => b.status === 'completed')
      .sort((a, b) => this.toDate(b.created_at).getTime() - this.toDate(a.created_at).getTime())[0];
    if (lastDone) items.push({ icon: 'check_circle', kind: 'ok', text: 'Backup completed — uploaded to cloud', time: this.timeAgo(this.toDate(lastDone.created_at)) });
    for (const a of this.alerts.slice(0, 3)) {
      if (a.resolved) {
        items.push({ icon: 'task_alt', kind: 'ok', text: a.title, time: this.timeAgo(this.toDate(a.resolved_at || a.created_at)) });
        continue;
      }
      items.push({ icon: a.severity === 'critical' || a.severity === 'warning' ? 'warning_amber' : 'info', kind: a.severity === 'info' ? 'info' : 'warn', text: a.title, time: this.timeAgo(this.toDate(a.created_at)) });
    }
    return items.slice(0, 4);
  }

  private uptimeSecs(u?: string): number {
    if (!u) return 0;
    let s = 0;
    const d = u.match(/(\d+)\s*d/); if (d) s += +d[1] * 86400;
    const h = u.match(/(\d+)\s*h/); if (h) s += +h[1] * 3600;
    const m = u.match(/(\d+)\s*m(?!s)/); if (m) s += +m[1] * 60;
    const sec = u.match(/(\d+)\s*s/); if (sec) s += +sec[1];
    return s;
  }
  toDate(s: string): Date { return new Date(s); }
  barClass(v: number | undefined, warn: number, crit: number): string {
    const x = v ?? 0;
    return x >= crit ? 'r' : x >= warn ? 'a' : 'g';
  }
  displayName(name: string): string {
    const s = name.replace(/^puru-/, '');
    const map: Record<string, string> = { hydrogen: 'Front End', dviewer: 'DICOM viewer', has: 'HAS', pacs: 'PACS', ris: 'RIS' };
    return map[s] || (s.charAt(0).toUpperCase() + s.slice(1));
  }
  statusLabel(s: ServiceInfo): string {
    switch (s.status) {
      case 'running': return 'Running';
      case 'starting': return 'Starting';
      case 'stopped': return 'Stopped';
      case 'error': return 'Needs attention';
      case 'notinstalled': return 'Not installed';
      default: return s.status;
    }
  }

  // ── Lifecycle ────────────────────────────────────────────────────────────
  ngOnInit(): void {
    this.loadFastData();
    this.loadSlowData();
    this.loadLiveAt();
    this.checkNetwork();
    this.refreshSub = interval(30000).subscribe(() => { this.loadFastData(); this.loadSlowData(); this.loadLiveAt(); });
    this.networkSub = interval(60000).subscribe(() => this.checkNetwork());
  }
  ngOnDestroy(): void { this.refreshSub?.unsubscribe(); this.networkSub?.unsubscribe(); }

  @HostListener('document:click')
  closeOverlays(): void { this.openMenu = null; this.profileOpen = false; }

  /** Fetch the friendly-URL snapshot: puru.local status, LAN IP, mDNS. */
  private async loadLiveAt(): Promise<void> {
    try {
      this.liveAt = await this.tauri.invokeSilent<any>('get_live_at_status');
    } catch {
      this.liveAt = null;
    }
  }

  /** URL rows for the "Puru is live at" card. Order: fleet `puru.local`
   *  first (works on the server PC out of the box), then the per-hospital
   *  mDNS name (works on any modern client on the same LAN), then the raw
   *  LAN IP (always works, ugly). The router-DNS name is only listed when
   *  the hosts file confirms it's set — otherwise it'd be a lie. */
  liveUrls(): Array<{ url: string; note: string; hint?: boolean }> {
    if (!this.liveAt) return [];
    const out: Array<{ url: string; note: string; hint?: boolean }> = [];
    const hosts = this.liveAt.hosts;
    const mdns = this.liveAt.mdns;

    if (hosts?.hosts_configured) {
      out.push({ url: 'http://puru.local', note: 'from this server PC' });
    }
    if (mdns?.running && mdns.hostname) {
      out.push({
        url: `http://${mdns.hostname}`,
        note: 'from any client on this LAN',
      });
    }
    if (hosts?.lan_ip) {
      out.push({
        url: `http://${hosts.lan_ip}`,
        note: 'fallback if the domain name isn’t resolving',
        hint: true,
      });
    }
    return out;
  }

  async copyUrl(url: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(url);
      this.copiedUrl = url;
      // Reset the check-mark after a moment so the same URL can be copied again.
      setTimeout(() => { if (this.copiedUrl === url) this.copiedUrl = null; }, 1500);
    } catch {
      this.notify.error('Could not copy — check clipboard permissions.');
    }
  }

  private async loadFastData(): Promise<void> {
    const [lic, sys, tel] = await Promise.allSettled([
      this.tauri.invokeSilent<License>('get_license'),
      this.tauri.invokeSilent<SystemInfo>('get_system_info'),
      this.tauri.invokeSilent<{ cpu_percent: number; ram_gb: number; disk_percent: number }>('get_telemetry_snapshot'),
    ]);
    if (lic.status === 'fulfilled' && lic.value) this.license = lic.value;
    if (sys.status === 'fulfilled') this.systemInfo = sys.value;
    if (tel.status === 'fulfilled') this.telemetry = tel.value;
  }

  private async loadSlowData(): Promise<void> {
    const [svc, dae, bak, alr] = await Promise.allSettled([
      this.tauri.invokeSilent<ServiceInfo[]>('get_services'),
      this.tauri.invokeSilent<DaemonStatus>('get_daemon_status'),
      this.tauri.invokeSilent<BackupRecord[]>('get_backup_history'),
      this.tauri.invokeSilent<HospitalAlert[]>('get_alerts'),
    ]);
    this.servicesLoading = false;
    if (svc.status === 'fulfilled') this.services = svc.value;
    if (dae.status === 'fulfilled') this.daemonRunning = dae.value.running;
    if (bak.status === 'fulfilled' && bak.value) {
      this.backupHistory = [...bak.value].sort((a, b) => this.toDate(b.created_at).getTime() - this.toDate(a.created_at).getTime());
      const done = this.backupHistory.filter(b => b.status === 'completed');
      this.lastBackupTime = done.length ? this.timeAgo(this.toDate(done[0].created_at)) : null;
    }
    if (alr.status === 'fulfilled' && alr.value) this.alerts = alr.value;
  }

  private async checkNetwork(): Promise<void> {
    try { this.networkStatus = await this.tauri.invokeSilent<NetworkStatus>('check_network'); } catch { /* ignore */ }
  }

  async runSpeedTest(): Promise<void> {
    this.speedTestRunning = true;
    try {
      this.speedTestResult = await this.tauri.invoke<SpeedTestResult>('run_speed_test');
      if (this.speedTestResult) {
        this.networkStatus = {
          connected: this.speedTestResult.connected,
          latency_ms: this.speedTestResult.latency_ms,
          gcp_reachable: this.speedTestResult.gcp_reachable,
          gcp_latency_ms: this.speedTestResult.gcp_latency_ms,
          checked_at: this.speedTestResult.tested_at,
        };
      }
    } catch { /* handled by TauriService */ }
    finally { this.speedTestRunning = false; }
  }

  timeAgo(date: Date): string {
    const s = Math.floor((Date.now() - date.getTime()) / 1000);
    if (s < 60) return 'Just now';
    const m = Math.floor(s / 60); if (m < 60) return `${m}m ago`;
    const h = Math.floor(m / 60); if (h < 24) return `${h}h ago`;
    return `${Math.floor(h / 24)}d ago`;
  }

  // ── UI actions ───────────────────────────────────────────────────────────
  goto(route: string): void { this.router.navigate([route]); }
  toggleProfile(e: Event): void { e.stopPropagation(); this.profileOpen = !this.profileOpen; }
  toggleMenu(name: string, e: Event): void { e.stopPropagation(); this.openMenu = this.openMenu === name ? null : name; }
  openBackups(): void { this.backupsOpen = true; }

  private setMsg(name: string, text: string, kind: 'busy' | 'ok' | 'error'): void {
    this.actionMsg[name] = { text, kind };
    if (kind !== 'busy') setTimeout(() => { if (this.actionMsg[name]?.text === text) delete this.actionMsg[name]; }, kind === 'error' ? 6000 : 3000);
  }

  /** Generic start/stop/restart. */
  async act(s: ServiceInfo, cmd: string, busy: string, done: string): Promise<void> {
    this.openMenu = null;
    this.setMsg(s.name, busy, 'busy');
    try { await this.tauri.invoke(cmd, { name: s.name }); this.setMsg(s.name, done, 'ok'); await this.loadSlowData(); }
    catch { this.setMsg(s.name, 'Failed', 'error'); }
  }

  async rollback(s: ServiceInfo): Promise<void> {
    this.openMenu = null;
    if (!confirm(`Roll back ${this.displayName(s.name)} to the previous version?`)) return;
    this.setMsg(s.name, 'Rolling back…', 'busy');
    try { await this.tauri.invoke('rollback_native_service', { serviceName: s.name }); this.setMsg(s.name, 'Rolled back', 'ok'); await this.loadSlowData(); }
    catch { this.setMsg(s.name, 'Rollback failed', 'error'); }
  }

  async kill(s: ServiceInfo): Promise<void> {
    this.openMenu = null;
    const pid = parseInt((s.container_name || '').replace(/[^0-9]/g, ''), 10);
    if (!pid) { this.notify.error(`No process id for ${this.displayName(s.name)}.`); return; }
    if (!confirm(`Kill ${this.displayName(s.name)} (PID ${pid})?\n\nThis sends SIGKILL — no graceful shutdown.`)) return;
    this.setMsg(s.name, 'Killing…', 'busy');
    try { await this.tauri.invoke('kill_process_by_pid', { pid }); this.setMsg(s.name, 'Killed', 'ok'); await this.loadSlowData(); }
    catch { this.setMsg(s.name, 'Kill failed', 'error'); }
  }

  async copyLogs(s: ServiceInfo): Promise<void> {
    this.openMenu = null;
    this.setMsg(s.name, 'Copying logs…', 'busy');
    try {
      const logs = await this.tauri.invoke<string>('get_container_logs', { containerName: s.name, tail: 200 });
      await navigator.clipboard.writeText(logs || '');
      this.setMsg(s.name, 'Logs copied', 'ok');
    } catch { this.setMsg(s.name, 'Copy failed', 'error'); }
  }

  viewLogs(s: ServiceInfo): void { this.openMenu = null; this.logService = s; }
  checkUpdate(): void { this.openMenu = null; this.router.navigate(['/updates']); }

  async backupNow(): Promise<void> {
    this.busyBackup = true;
    try { await this.tauri.invoke('start_backup', { type: 'full' }); this.notify.success('Backup started'); }
    catch { /* handled by TauriService */ }
    finally { this.busyBackup = false; }
  }

  async ack(a: HospitalAlert): Promise<void> {
    try { await this.tauri.invoke('acknowledge_alert', { alertId: a.id }); a.acknowledged = true; }
    catch { /* handled by TauriService */ }
  }
}
