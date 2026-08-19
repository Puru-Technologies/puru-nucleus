import { Component, EventEmitter, HostListener, Input, OnInit, Output, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { TauriService } from '../services/tauri.service';

/**
 * Reusable full-screen log viewer dialog: one entry per line (horizontal scroll,
 * toggleable wrap), live search, level filter with colouring, copy, and a
 * refresh that jumps to the latest line. Used from the Services page and the
 * Dashboard's per-service "View logs".
 */
@Component({
  selector: 'app-log-viewer-dialog',
  standalone: true,
  imports: [CommonModule, FormsModule],
  template: `
    <div class="log-modal-backdrop">
      <div class="log-modal" role="dialog" aria-modal="true">
        <div class="log-header">
          <div class="log-title"><span class="material-icons">article</span><span>{{ title }} — logs</span></div>
          <div class="log-head-right">
            <select class="log-select" [(ngModel)]="logTimeFilter" (ngModelChange)="refresh()">
              <option value="tail">Latest 200 lines</option>
              <option value="1h">Last 1 hour</option>
              <option value="6h">Last 6 hours</option>
              <option value="24h">Last 24 hours</option>
              <option value="3d">Last 3 days</option>
              <option value="7d">Last 7 days</option>
            </select>
            <button class="btn-icon log-btn" (click)="close.emit()" title="Close (Esc)"><span class="material-icons">close</span></button>
          </div>
        </div>

        <div class="log-toolbar">
          <div class="log-search">
            <span class="material-icons">search</span>
            <input type="text" [(ngModel)]="logSearch" placeholder="Filter lines…" spellcheck="false" />
            @if (logSearch) { <button class="ls-clear" (click)="logSearch = ''"><span class="material-icons">close</span></button> }
          </div>
          <div class="log-levels">
            <button class="lv-btn" [class.on]="logLevel === 'all'" (click)="logLevel = 'all'">All</button>
            <button class="lv-btn lv-warn" [class.on]="logLevel === 'warn'" (click)="logLevel = 'warn'">Warnings+</button>
            <button class="lv-btn lv-err" [class.on]="logLevel === 'error'" (click)="logLevel = 'error'">Errors</button>
          </div>
          <span class="log-count">{{ logLines.length }} line{{ logLines.length === 1 ? '' : 's' }}</span>
          <div class="log-tools">
            <button class="btn-icon log-btn" (click)="logWrap = !logWrap" [title]="logWrap ? 'No wrap' : 'Wrap lines'"><span class="material-icons">{{ logWrap ? 'wrap_text' : 'notes' }}</span></button>
            <button class="btn-icon log-btn" (click)="copyLogs()" title="Copy all"><span class="material-icons">{{ copied ? 'check' : 'content_copy' }}</span></button>
          </div>
        </div>

        @if (logsLoading) {
          <div class="log-loading"><span class="spinner"></span></div>
        } @else if (logLines.length === 0) {
          <div class="log-empty">{{ logSearch || logLevel !== 'all' ? 'No lines match the filter.' : 'No logs available.' }}</div>
        } @else {
          <div class="log-body" [class.nowrap]="!logWrap">
            @for (ln of logLines; track $index) { <div class="log-line" [class]="ln.cls">{{ ln.text }}</div> }
          </div>
        }

        <div class="log-foot">
          <span class="log-hint">Press Esc to close</span>
          <button class="btn btn-primary" (click)="refresh()" [disabled]="logsLoading">
            <span class="material-icons">refresh</span>{{ logsLoading ? 'Refreshing…' : 'Refresh (jump to latest)' }}
          </button>
        </div>
      </div>
    </div>
  `,
  styles: [`
    .material-icons{font-size:18px;line-height:1;}
    .log-modal-backdrop{position:fixed;inset:0;z-index:60;background:rgba(2,6,23,.55);display:flex;align-items:center;justify-content:center;padding:32px;}
    .log-modal{display:flex;flex-direction:column;width:min(1100px,94vw);height:min(820px,88vh);background:#0f172a;border-radius:12px;overflow:hidden;box-shadow:0 24px 60px rgba(0,0,0,.45);}
    .log-header{display:flex;justify-content:space-between;align-items:center;padding:12px 16px;background:#1e293b;color:#e2e8f0;flex-shrink:0;}
    .log-title{display:flex;align-items:center;gap:8px;font-size:.95rem;font-weight:600;}
    .log-head-right{display:flex;align-items:center;gap:8px;}
    .btn-icon{width:34px;height:34px;border-radius:8px;border:0;background:transparent;display:flex;align-items:center;justify-content:center;cursor:pointer;}
    .log-btn{color:#94a3b8;} .log-btn:hover{background:rgba(255,255,255,.1);color:#fff;}
    .log-select{background:#0f172a;color:#e2e8f0;border:1px solid #334155;border-radius:6px;padding:5px 8px;font-size:.76rem;cursor:pointer;outline:none;}
    .log-select option{background:#1e293b;color:#e2e8f0;}
    .log-toolbar{display:flex;align-items:center;gap:12px;flex-wrap:wrap;padding:8px 14px;background:#131c31;border-bottom:1px solid #1e293b;flex-shrink:0;}
    .log-search{display:flex;align-items:center;gap:6px;flex:1;min-width:160px;background:#0f172a;border:1px solid #334155;border-radius:6px;padding:4px 8px;}
    .log-search .material-icons{font-size:16px;color:#64748b;}
    .log-search input{flex:1;background:transparent;border:none;outline:none;color:#e2e8f0;font-size:.8rem;font-family:inherit;}
    .log-search input::placeholder{color:#64748b;}
    .ls-clear{color:#64748b;display:flex;background:none;border:0;cursor:pointer;} .ls-clear .material-icons{font-size:15px;} .ls-clear:hover{color:#e2e8f0;}
    .log-levels{display:flex;gap:4px;}
    .lv-btn{padding:4px 10px;font-size:.74rem;font-weight:600;border-radius:6px;border:1px solid #334155;background:transparent;color:#94a3b8;cursor:pointer;}
    .lv-btn:hover{border-color:#475569;color:#e2e8f0;} .lv-btn.on{background:#334155;color:#fff;}
    .lv-btn.lv-warn.on{background:#b45309;border-color:#b45309;} .lv-btn.lv-err.on{background:#b91c1c;border-color:#b91c1c;}
    .log-count{font-size:.74rem;color:#64748b;font-variant-numeric:tabular-nums;}
    .log-tools{display:flex;gap:2px;margin-left:auto;}
    .log-loading{display:flex;justify-content:center;align-items:center;flex:1;background:#0f172a;}
    .log-empty{flex:1;display:flex;align-items:center;justify-content:center;color:#64748b;font-size:.85rem;background:#0f172a;}
    .log-body{margin:0;flex:1;overflow:auto;background:#0f172a;color:#e2e8f0;font-family:'SF Mono','Fira Code','Consolas',monospace;font-size:.75rem;line-height:1.55;padding:10px 0;}
    .log-line{padding:1px 16px;white-space:pre-wrap;word-break:break-word;}
    .log-body.nowrap .log-line{white-space:pre;}
    .log-line:hover{background:rgba(148,163,184,.08);}
    .log-line.ll-error{color:#f87171;} .log-line.ll-warn{color:#fbbf24;} .log-line.ll-info{color:#cbd5e1;} .log-line.ll-debug{color:#64748b;}
    .log-foot{display:flex;align-items:center;justify-content:space-between;gap:12px;padding:10px 14px;background:#1e293b;flex-shrink:0;}
    .log-hint{font-size:.74rem;color:#64748b;}
    .btn{display:inline-flex;align-items:center;gap:7px;font-size:13.5px;padding:8px 14px;border-radius:9px;border:1px solid #334155;background:transparent;color:#e2e8f0;cursor:pointer;}
    .btn .material-icons{font-size:16px;}
    .btn-primary{background:#009efb;border-color:#009efb;color:#fff;} .btn-primary:disabled{opacity:.6;cursor:default;}
    .spinner{width:26px;height:26px;border:3px solid #334155;border-top-color:#009efb;border-radius:50%;animation:spin .8s linear infinite;}
    @keyframes spin{to{transform:rotate(360deg);}}
    @media (max-width:820px){.log-modal-backdrop{padding:0;}.log-modal{width:100vw;height:100vh;border-radius:0;}}
  `]
})
export class LogViewerDialogComponent implements OnInit {
  private tauri = inject(TauriService);

  @Input() serviceName = '';
  @Input() title = '';
  @Input() infra = false;
  @Output() close = new EventEmitter<void>();

  logOutput = '';
  logsLoading = false;
  logTimeFilter: 'tail' | '1h' | '6h' | '24h' | '3d' | '7d' = 'tail';
  logSearch = '';
  logLevel: 'all' | 'warn' | 'error' = 'all';
  logWrap = false;
  copied = false;
  private logVersion = 0;

  ngOnInit(): void { this.refresh(); }

  @HostListener('document:keydown.escape')
  onEsc(): void { this.close.emit(); }

  async refresh(): Promise<void> {
    this.logsLoading = true;
    try {
      if (this.infra) {
        this.logOutput = await this.tauri.invoke<string>('get_infra_log', { name: this.serviceName, lines: 300 });
      } else {
        const args: Record<string, unknown> = { containerName: this.serviceName };
        if (this.logTimeFilter === 'tail') { args['tail'] = 200; }
        else { args['tail'] = 0; args['since'] = this.sinceTs(); }
        this.logOutput = await this.tauri.invoke<string>('get_container_logs', args);
      }
    } catch {
      this.logOutput = `Failed to fetch logs for ${this.serviceName}.`;
    } finally {
      this.logsLoading = false;
      this.logVersion++;
      setTimeout(() => { const el = document.querySelector('.log-body') as HTMLElement | null; if (el) el.scrollTop = el.scrollHeight; }, 0);
    }
  }

  private sinceTs(): number {
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

  private _cache: { text: string; cls: string }[] = [];
  private _sig = '';
  get logLines(): { text: string; cls: string }[] {
    const sig = `${this.logVersion}|${this.logLevel}|${this.logSearch}`;
    if (sig !== this._sig) { this._sig = sig; this._cache = this.compute(); }
    return this._cache;
  }

  private compute(): { text: string; cls: string }[] {
    const raw = this.logOutput || '';
    if (!raw.trim()) return [];
    const q = this.logSearch.trim().toLowerCase();
    const out: { text: string; cls: string }[] = [];
    let carry = '';
    for (const line of raw.split(/\r?\n/)) {
      if (line === '') continue;
      let cls = this.lineClass(line);
      if (cls) { carry = cls; }
      else if (/^\s/.test(line) && carry) { cls = carry; }
      else { carry = ''; }
      if (this.logLevel === 'error' && cls !== 'll-error') continue;
      if (this.logLevel === 'warn' && cls !== 'll-error' && cls !== 'll-warn') continue;
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
    try { await navigator.clipboard.writeText(this.logOutput || ''); this.copied = true; setTimeout(() => (this.copied = false), 1500); } catch { /* clipboard unavailable */ }
  }
}
