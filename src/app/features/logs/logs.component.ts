import { Component, OnDestroy, OnInit, inject, ChangeDetectorRef } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import {
  TauriService,
  LogSource,
  LogFileInfo,
  LogFileContent,
  LogTailEvent
} from '../../core/services/tauri.service';

/** Spring Boot / SLF4J line shape we try to parse:
 *  2026-06-22 10:42:01.512  WARN c.p.JwtFilter : message...
 *  ─────── date ────────  ───  ──── level ──── ─ source ─
 *  Date and level groups are optional — lines that don't match (e.g. plain
 *  stdout, stack-trace continuations) are treated as level "" and time null. */
const LOG_LINE_RE = /^(\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2}(?:[.,]\d{3})?)?\s*(ERROR|WARN(?:ING)?|INFO|DEBUG|TRACE|FATAL)?\b/;

interface ParsedLine {
  /** 1-based line number within the loaded page (offset + index + 1). */
  num: number;
  /** Raw text of the line. */
  text: string;
  /** Detected level, normalized to one of LEVELS (or '' if unparsed). */
  level: '' | 'ERROR' | 'WARN' | 'INFO' | 'DEBUG' | 'TRACE' | 'FATAL';
  /** Detected timestamp epoch-ms, or null if unparsed. */
  ts: number | null;
}

interface SourceGroup {
  /** Display label (e.g. "Nucleus", "Containers"). */
  label: string;
  /** source_type from backend ("nucleus" | "system" | "docker" | "container"). */
  type: string;
  sources: LogSource[];
}

type LevelFilter = 'ALL' | 'ERROR' | 'WARN' | 'INFO' | 'DEBUG';
type TimeFilter = 'all' | '10m' | '1h' | '6h' | '24h';

@Component({
  selector: 'app-logs',
  standalone: true,
  imports: [CommonModule, FormsModule],
  template: `
    <div class="shell">
      <!-- ─────────────────────────────────────────── LEFT RAIL ─── -->
      <aside class="rail">
        <div class="rail-header">
          <h2>Logs</h2>
          <button class="btn-icon" (click)="reloadSources()" title="Refresh sources">
            <span class="material-icons">refresh</span>
          </button>
        </div>

        <div class="rail-search">
          <span class="material-icons">search</span>
          <input
            type="text"
            placeholder="Filter files…"
            [(ngModel)]="filenameSearch"
            (ngModelChange)="onFilenameSearchChange()"
          />
          @if (filenameSearch) {
            <button class="search-clear" (click)="filenameSearch = ''; onFilenameSearchChange()">
              <span class="material-icons">close</span>
            </button>
          }
        </div>

        <div class="rail-tree">
          @if (railLoading) {
            <div class="rail-loading"><span class="spinner"></span></div>
          } @else {
            @for (group of sourceGroups; track group.type) {
              <div class="group">
                <button class="group-header" (click)="toggleGroup(group.type)">
                  <span class="material-icons chev">
                    {{ expandedGroups.has(group.type) ? 'expand_more' : 'chevron_right' }}
                  </span>
                  <span class="group-label">{{ group.label }}</span>
                  <span class="group-count">{{ groupItemCount(group) }}</span>
                </button>

                @if (expandedGroups.has(group.type)) {
                  @for (source of group.sources; track source.path) {
                    @if (source.kind === 'container') {
                      <!-- Container: source itself is the "file" -->
                      @if (matchesFilenameSearch(source.name)) {
                        <button
                          class="entry container-entry"
                          [class.selected]="selectedPath === source.path"
                          (click)="selectPath(source.path, source.name)"
                          [title]="source.path">
                          <span class="material-icons entry-icon">memory</span>
                          <span class="entry-name">{{ source.name }}</span>
                        </button>
                      }
                    } @else {
                      <!-- Directory: expand to show files -->
                      <button class="entry dir-entry" (click)="toggleSource(source.path)">
                        <span class="material-icons chev">
                          {{ expandedSources.has(source.path) ? 'expand_more' : 'chevron_right' }}
                        </span>
                        <span class="material-icons entry-icon">folder</span>
                        <span class="entry-name">{{ source.name }}</span>
                      </button>

                      @if (expandedSources.has(source.path)) {
                        @if (filesBySource.get(source.path) === undefined) {
                          <div class="dir-loading"><span class="spinner spinner-sm"></span></div>
                        } @else if ((filesBySource.get(source.path) ?? []).length === 0) {
                          <div class="dir-empty">No log files</div>
                        } @else {
                          @for (f of filteredFilesFor(source.path); track f.path) {
                            <button
                              class="entry file-entry"
                              [class.selected]="selectedPath === f.path"
                              (click)="selectPath(f.path, f.name)"
                              [title]="f.path">
                              <span class="material-icons entry-icon">description</span>
                              <span class="entry-name">{{ f.name }}</span>
                              <span class="entry-meta">{{ formatSize(f.size_bytes) }}</span>
                            </button>
                          }
                        }
                      }
                    }
                  }
                }
              </div>
            }
          }
        </div>
      </aside>

      <!-- ───────────────────────────────────────── RIGHT PANE ─── -->
      <section class="pane">
        @if (!selectedPath) {
          <div class="pane-empty">
            <span class="material-icons">article</span>
            <h3>No log selected</h3>
            <p>Pick a file or container from the left to start reading.</p>
          </div>
        } @else {
          <header class="pane-header">
            <div class="pane-title">
              <span class="material-icons">
                {{ selectedPath.startsWith('container:') ? 'memory' : 'description' }}
              </span>
              <div class="title-text">
                <span class="title-name">{{ selectedName }}</span>
                @if (viewerContent) {
                  <span class="title-meta">
                    Lines {{ viewerContent.offset + 1 }}–{{ viewerContent.offset + viewerContent.lines_returned }}
                    of {{ viewerContent.total_lines }}
                    @if (visibleCount() !== viewerContent.lines_returned) {
                      &nbsp;·&nbsp;{{ visibleCount() }} after filters
                    }
                  </span>
                }
              </div>
            </div>

            <div class="pane-actions">
              <button class="btn-icon" (click)="prevPage()"
                [disabled]="!canPrevPage()" title="Previous page">
                <span class="material-icons">chevron_left</span>
              </button>
              <button class="btn-icon" (click)="nextPage()"
                [disabled]="!canNextPage()" title="Next page">
                <span class="material-icons">chevron_right</span>
              </button>
              <button class="btn-icon" (click)="reloadFile()" title="Reload">
                <span class="material-icons">refresh</span>
              </button>
              <button
                class="btn-icon follow-btn"
                [class.active]="following"
                (click)="toggleFollow()"
                [title]="following ? 'Stop following' : 'Follow tail'">
                <span class="material-icons">{{ following ? 'pause' : 'play_arrow' }}</span>
                <span class="follow-label">{{ following ? 'Live' : 'Follow' }}</span>
              </button>
              <button class="btn-icon" (click)="downloadVisible()" title="Download visible lines">
                <span class="material-icons">download</span>
              </button>
              <button class="btn-icon" (click)="copyVisible()" title="Copy visible lines">
                <span class="material-icons">content_copy</span>
              </button>
              <button class="btn-icon" (click)="closeViewer()" title="Close">
                <span class="material-icons">close</span>
              </button>
            </div>
          </header>

          <div class="pane-filters">
            <div class="level-chips">
              @for (l of LEVELS; track l) {
                <button
                  class="chip"
                  [class.active]="levelFilter === l"
                  (click)="setLevelFilter(l)">
                  {{ l }}
                </button>
              }
            </div>

            <select class="select-sm" [(ngModel)]="timeFilter" (ngModelChange)="onFilterChange()">
              <option value="all">All time</option>
              <option value="10m">Last 10 min</option>
              <option value="1h">Last 1 hour</option>
              <option value="6h">Last 6 hours</option>
              <option value="24h">Last 24 hours</option>
            </select>

            <div class="in-file-search">
              <span class="material-icons">search</span>
              <input
                type="text"
                placeholder="Search in file…"
                [(ngModel)]="inFileSearch"
                (ngModelChange)="onInFileSearchChange()"
              />
              @if (inFileSearch) {
                <span class="match-info">
                  {{ searchMatches.length === 0 ? 'No matches' :
                     (currentMatchIdx + 1) + ' / ' + searchMatches.length }}
                </span>
                <button class="btn-icon match-nav"
                  (click)="jumpMatch(-1)"
                  [disabled]="searchMatches.length === 0">
                  <span class="material-icons">expand_less</span>
                </button>
                <button class="btn-icon match-nav"
                  (click)="jumpMatch(1)"
                  [disabled]="searchMatches.length === 0">
                  <span class="material-icons">expand_more</span>
                </button>
                <button class="btn-icon" (click)="inFileSearch = ''; onInFileSearchChange()">
                  <span class="material-icons">close</span>
                </button>
              }
            </div>
          </div>

          <div class="pane-viewer" #viewerEl>
            @if (viewerLoading) {
              <div class="viewer-loading"><span class="spinner spinner-lg"></span></div>
            } @else if (!viewerContent || viewerContent.lines_returned === 0) {
              <div class="viewer-empty">File is empty.</div>
            } @else if (visibleCount() === 0) {
              <div class="viewer-empty">
                No lines match the current filters.
                <button class="btn-link" (click)="clearFilters()">Clear filters</button>
              </div>
            } @else {
              <div class="lines">
                @for (line of visibleLines(); track line.num) {
                  <div class="line"
                    [class.line-error]="line.level === 'ERROR' || line.level === 'FATAL'"
                    [class.line-warn]="line.level === 'WARN'"
                    [class.line-info]="line.level === 'INFO'"
                    [class.line-debug]="line.level === 'DEBUG' || line.level === 'TRACE'"
                    [class.line-match]="isMatchLine(line.num)"
                    [attr.data-line-num]="line.num">
                    <span class="line-num">{{ line.num }}</span>
                    <span class="line-text" [innerHTML]="renderText(line.text)"></span>
                  </div>
                }
              </div>
            }
          </div>
        }
      </section>
    </div>
  `,
  styles: [`
    :host {
      display: block;
      height: 100%;
    }

    .shell {
      display: flex;
      height: calc(100vh - 64px);
      min-height: 480px;
      background: var(--bg, #f8fafc);
    }

    /* ── Left rail ─────────────────────────────── */
    .rail {
      width: 320px;
      min-width: 320px;
      border-right: 1px solid var(--border, #e2e8f0);
      background: #fff;
      display: flex;
      flex-direction: column;
    }
    .rail-header {
      padding: 14px 16px 8px;
      display: flex;
      justify-content: space-between;
      align-items: center;
      border-bottom: 1px solid var(--border, #e2e8f0);
      h2 {
        margin: 0;
        font-size: 1rem;
        font-weight: 700;
      }
    }
    .rail-search {
      padding: 10px 12px;
      display: flex;
      align-items: center;
      gap: 6px;
      border-bottom: 1px solid var(--border, #e2e8f0);
      .material-icons {
        font-size: 18px;
        color: var(--text-muted, #94a3b8);
      }
      input {
        flex: 1;
        border: none;
        outline: none;
        font-size: 0.8rem;
        background: transparent;
      }
      .search-clear {
        border: none;
        background: transparent;
        cursor: pointer;
        color: var(--text-muted, #94a3b8);
        padding: 2px;
        display: flex;
        .material-icons { font-size: 16px; }
        &:hover { color: var(--text-primary, #0f172a); }
      }
    }
    .rail-tree {
      flex: 1;
      overflow-y: auto;
      padding: 6px 0;
    }
    .rail-loading {
      display: flex;
      justify-content: center;
      padding: 40px 0;
    }

    .group { margin-bottom: 4px; }
    .group-header {
      width: 100%;
      display: flex;
      align-items: center;
      gap: 4px;
      padding: 6px 12px;
      border: none;
      background: transparent;
      cursor: pointer;
      text-align: left;
      font-size: 0.75rem;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.04em;
      color: var(--text-secondary, #475569);
      &:hover { background: rgba(15, 23, 42, 0.03); }
      .chev { font-size: 16px; color: var(--text-muted, #94a3b8); }
      .group-label { flex: 1; }
      .group-count {
        font-weight: 400;
        font-size: 0.7rem;
        color: var(--text-muted, #94a3b8);
      }
    }

    .entry {
      width: 100%;
      display: flex;
      align-items: center;
      gap: 6px;
      padding: 5px 12px 5px 28px;
      border: none;
      background: transparent;
      cursor: pointer;
      text-align: left;
      font-size: 0.8rem;
      color: var(--text-primary, #0f172a);
      &:hover { background: rgba(15, 23, 42, 0.04); }
      &.selected {
        background: rgba(59, 130, 246, 0.08);
        color: #2563eb;
        font-weight: 500;
      }
      .chev { font-size: 14px; color: var(--text-muted, #94a3b8); }
      .entry-icon {
        font-size: 14px;
        color: var(--text-muted, #94a3b8);
      }
      .entry-name {
        flex: 1;
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
      }
      .entry-meta {
        font-size: 0.7rem;
        color: var(--text-muted, #94a3b8);
      }
    }
    .dir-entry .entry-icon { color: #f59e0b; }
    .container-entry .entry-icon { color: #8b5cf6; }

    .dir-loading {
      padding: 6px 0 6px 44px;
    }
    .dir-empty {
      padding: 4px 12px 4px 44px;
      font-size: 0.7rem;
      color: var(--text-muted, #94a3b8);
      font-style: italic;
    }

    /* ── Right pane ─────────────────────────────── */
    .pane {
      flex: 1;
      display: flex;
      flex-direction: column;
      min-width: 0;
    }
    .pane-empty {
      flex: 1;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      gap: 8px;
      color: var(--text-muted, #94a3b8);
      .material-icons { font-size: 48px; }
      h3 { margin: 0; font-size: 1rem; color: var(--text-secondary, #475569); }
      p { margin: 0; font-size: 0.85rem; }
    }

    .pane-header {
      display: flex;
      justify-content: space-between;
      align-items: center;
      padding: 10px 16px;
      background: #fff;
      border-bottom: 1px solid var(--border, #e2e8f0);
    }
    .pane-title {
      display: flex;
      align-items: center;
      gap: 8px;
      min-width: 0;
      .material-icons {
        font-size: 18px;
        color: var(--text-muted, #94a3b8);
      }
    }
    .title-text {
      display: flex;
      flex-direction: column;
      min-width: 0;
    }
    .title-name {
      font-weight: 600;
      font-size: 0.9rem;
    }
    .title-meta {
      font-size: 0.72rem;
      color: var(--text-muted, #94a3b8);
      font-family: 'SF Mono', 'Fira Code', monospace;
    }

    .pane-actions {
      display: flex;
      gap: 2px;
      align-items: center;
    }
    .follow-btn {
      display: flex;
      align-items: center;
      gap: 4px;
      padding: 4px 8px !important;
      border-radius: 4px;
      .follow-label {
        font-size: 0.75rem;
        font-weight: 500;
      }
      &.active {
        background: rgba(239, 68, 68, 0.1);
        color: #dc2626;
        .material-icons { color: #dc2626; }
      }
    }

    .btn-icon {
      border: none;
      background: transparent;
      cursor: pointer;
      padding: 4px;
      border-radius: 4px;
      color: var(--text-secondary, #475569);
      display: flex;
      align-items: center;
      &:hover:not(:disabled) { background: rgba(15, 23, 42, 0.06); }
      &:disabled { opacity: 0.35; cursor: not-allowed; }
      .material-icons { font-size: 18px; }
    }
    .btn-link {
      border: none;
      background: transparent;
      color: #2563eb;
      cursor: pointer;
      padding: 0;
      margin-left: 6px;
      font-size: inherit;
      text-decoration: underline;
    }

    /* ── Filter bar ─────────────────────────────── */
    .pane-filters {
      display: flex;
      gap: 12px;
      align-items: center;
      padding: 8px 16px;
      background: #f8fafc;
      border-bottom: 1px solid var(--border, #e2e8f0);
      flex-wrap: wrap;
    }
    .level-chips {
      display: flex;
      gap: 4px;
    }
    .chip {
      border: 1px solid var(--border, #e2e8f0);
      background: #fff;
      padding: 3px 10px;
      border-radius: 12px;
      font-size: 0.7rem;
      font-weight: 600;
      cursor: pointer;
      color: var(--text-secondary, #475569);
      &:hover { background: rgba(15, 23, 42, 0.04); }
      &.active {
        background: #1e293b;
        color: #fff;
        border-color: #1e293b;
      }
    }
    .select-sm {
      border: 1px solid var(--border, #e2e8f0);
      border-radius: 4px;
      padding: 4px 8px;
      font-size: 0.75rem;
      background: #fff;
      outline: none;
      cursor: pointer;
    }
    .in-file-search {
      flex: 1;
      min-width: 200px;
      display: flex;
      align-items: center;
      gap: 4px;
      padding: 4px 8px;
      background: #fff;
      border: 1px solid var(--border, #e2e8f0);
      border-radius: 4px;
      .material-icons { font-size: 16px; color: var(--text-muted, #94a3b8); }
      input {
        flex: 1;
        border: none;
        outline: none;
        font-size: 0.8rem;
        background: transparent;
      }
      .match-info {
        font-size: 0.7rem;
        color: var(--text-muted, #94a3b8);
        font-family: 'SF Mono', 'Fira Code', monospace;
        padding-right: 4px;
      }
      .match-nav { padding: 2px; .material-icons { font-size: 14px; } }
    }

    /* ── Viewer ─────────────────────────────────── */
    .pane-viewer {
      flex: 1;
      overflow-y: auto;
      background: #fff;
      padding: 8px 0;
    }
    .viewer-loading {
      display: flex;
      justify-content: center;
      padding: 60px 0;
    }
    .viewer-empty {
      padding: 40px;
      text-align: center;
      color: var(--text-muted, #94a3b8);
      font-size: 0.85rem;
    }

    .lines {
      font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
      font-size: 0.72rem;
      line-height: 1.55;
    }
    .line {
      display: flex;
      gap: 0;
      padding: 0;
      border-left: 3px solid transparent;
      &:hover { background: rgba(15, 23, 42, 0.025); }
      &.line-match { background: rgba(250, 204, 21, 0.12); }
    }
    .line-num {
      flex: 0 0 64px;
      text-align: right;
      padding: 0 12px 0 6px;
      color: var(--text-muted, #94a3b8);
      user-select: none;
    }
    .line-text {
      flex: 1;
      white-space: pre-wrap;
      word-break: break-all;
      padding-right: 16px;
      color: var(--text-primary, #0f172a);
    }
    .line-error { border-left-color: #dc2626; .line-text { color: #b91c1c; } }
    .line-warn  { border-left-color: #d97706; .line-text { color: #b45309; } }
    .line-info  { border-left-color: #2563eb; }
    .line-debug { .line-text { color: var(--text-muted, #94a3b8); } }
    :host ::ng-deep .search-hit {
      background: #facc15;
      color: #0f172a;
      border-radius: 2px;
      padding: 0 1px;
    }

    .spinner { /* assumes global spinner class exists; fallback dot */ }
    .spinner-sm {
      width: 14px;
      height: 14px;
      border: 2px solid var(--border, #e2e8f0);
      border-top-color: var(--text-secondary, #475569);
      border-radius: 50%;
      display: inline-block;
      animation: spin 0.8s linear infinite;
    }
    .spinner-lg {
      width: 32px;
      height: 32px;
      border: 3px solid var(--border, #e2e8f0);
      border-top-color: var(--text-secondary, #475569);
      border-radius: 50%;
      display: inline-block;
      animation: spin 0.8s linear infinite;
    }
    @keyframes spin { to { transform: rotate(360deg); } }
  `]
})
export class LogsComponent implements OnInit, OnDestroy {
  private tauri = inject(TauriService);
  private cdr = inject(ChangeDetectorRef);

  readonly LEVELS: LevelFilter[] = ['ALL', 'ERROR', 'WARN', 'INFO', 'DEBUG'];
  private readonly LS_KEY = 'puru.logs.lastSelected';
  private readonly PAGE_SIZE = 500;
  private readonly LIVE_BUFFER_CAP = 5000;

  sources: LogSource[] = [];
  sourceGroups: SourceGroup[] = [];
  filesBySource = new Map<string, LogFileInfo[] | undefined>();
  railLoading = true;

  expandedGroups = new Set<string>();
  expandedSources = new Set<string>();

  selectedPath: string | null = null;
  selectedName = '';

  viewerContent: LogFileContent | null = null;
  viewerLoading = false;
  /** Lines currently in the viewer, with parsed metadata. Recomputed when
   *  viewerContent changes or when Follow mode appends new lines. */
  private parsedLines: ParsedLine[] = [];

  // Left-rail filter
  filenameSearch = '';
  private filenameSearchLower = '';

  // Right-pane filters
  levelFilter: LevelFilter = 'ALL';
  timeFilter: TimeFilter = 'all';
  inFileSearch = '';
  private inFileSearchLower = '';

  // In-file search nav
  searchMatches: number[] = [];
  currentMatchIdx = 0;

  // Live tail
  following = false;
  private streamId: string | null = null;
  private unlisten: UnlistenFn | null = null;

  ngOnInit(): void {
    this.reloadSources();
  }

  async ngOnDestroy(): Promise<void> {
    await this.stopTail();
  }

  // ── Source loading ───────────────────────────────────────────────

  async reloadSources(): Promise<void> {
    this.railLoading = true;
    try {
      this.sources = await this.tauri.invoke<LogSource[]>('get_log_sources');
    } catch {
      this.sources = [];
    }
    this.sourceGroups = this.groupSources(this.sources);
    // Default: expand the first group
    if (this.sourceGroups.length && !this.expandedGroups.size) {
      this.expandedGroups.add(this.sourceGroups[0].type);
    }
    this.railLoading = false;
    this.restoreLastSelection();
  }

  private groupSources(sources: LogSource[]): SourceGroup[] {
    const labels: Record<string, string> = {
      nucleus: 'Puru DC',
      system: 'System',
      docker: 'Docker Compose',
      container: 'Containers'
    };
    const order = ['nucleus', 'system', 'docker', 'container'];
    const map = new Map<string, LogSource[]>();
    for (const s of sources) {
      const t = s.source_type || 'other';
      if (!map.has(t)) map.set(t, []);
      map.get(t)!.push(s);
    }
    return order
      .filter(t => map.has(t))
      .map(t => ({ label: labels[t] ?? t, type: t, sources: map.get(t)! }))
      .concat(
        Array.from(map.keys())
          .filter(t => !order.includes(t))
          .map(t => ({ label: labels[t] ?? t, type: t, sources: map.get(t)! }))
      );
  }

  groupItemCount(group: SourceGroup): number {
    return group.sources.length;
  }

  toggleGroup(type: string): void {
    if (this.expandedGroups.has(type)) this.expandedGroups.delete(type);
    else this.expandedGroups.add(type);
  }

  async toggleSource(path: string): Promise<void> {
    if (this.expandedSources.has(path)) {
      this.expandedSources.delete(path);
      return;
    }
    this.expandedSources.add(path);
    if (this.filesBySource.get(path) === undefined) {
      // Mark as loading by leaving the entry undefined; once fetched, set to array
      try {
        const files = await this.tauri.invoke<LogFileInfo[]>('list_log_files', { path });
        this.filesBySource.set(path, files);
      } catch {
        this.filesBySource.set(path, []);
      }
    }
  }

  matchesFilenameSearch(name: string): boolean {
    return !this.filenameSearchLower || name.toLowerCase().includes(this.filenameSearchLower);
  }

  filteredFilesFor(path: string): LogFileInfo[] {
    const all = this.filesBySource.get(path) ?? [];
    if (!this.filenameSearchLower) return all;
    return all.filter(f => f.name.toLowerCase().includes(this.filenameSearchLower));
  }

  onFilenameSearchChange(): void {
    this.filenameSearchLower = this.filenameSearch.trim().toLowerCase();
  }

  // ── File / container selection ──────────────────────────────────

  async selectPath(path: string, name: string): Promise<void> {
    if (this.selectedPath === path) return;
    await this.stopTail();
    this.selectedPath = path;
    this.selectedName = name;
    localStorage.setItem(this.LS_KEY, JSON.stringify({ path, name }));
    await this.loadCurrentFile();
  }

  private async loadCurrentFile(offset?: number): Promise<void> {
    if (!this.selectedPath) return;
    this.viewerLoading = true;
    this.viewerContent = null;
    this.parsedLines = [];
    try {
      const args: Record<string, unknown> = { path: this.selectedPath };
      if (offset === undefined) {
        args['tail'] = this.PAGE_SIZE;
      } else {
        args['offset'] = offset;
        args['limit'] = this.PAGE_SIZE;
      }
      this.viewerContent = await this.tauri.invoke<LogFileContent>('read_log_file', args);
      this.parsedLines = this.parseContent(this.viewerContent);
      this.recomputeSearchMatches();
    } catch (e) {
      this.viewerContent = {
        path: this.selectedPath,
        content: `Failed to read: ${e}`,
        total_lines: 0,
        offset: 0,
        lines_returned: 0
      };
      this.parsedLines = [];
    } finally {
      this.viewerLoading = false;
    }
  }

  private parseContent(c: LogFileContent): ParsedLine[] {
    if (!c.content) return [];
    const lines = c.content.split('\n');
    return lines.map((text, i) => this.parseLine(text, c.offset + i + 1));
  }

  private parseLine(text: string, num: number): ParsedLine {
    const m = LOG_LINE_RE.exec(text);
    let level: ParsedLine['level'] = '';
    let ts: number | null = null;
    if (m) {
      if (m[2]) {
        const lvl = m[2].toUpperCase();
        level = (lvl === 'WARNING' ? 'WARN' : lvl) as ParsedLine['level'];
      }
      if (m[1]) {
        const iso = m[1].replace(' ', 'T').replace(',', '.');
        const parsed = Date.parse(iso);
        if (!Number.isNaN(parsed)) ts = parsed;
      }
    }
    return { num, text, level, ts };
  }

  reloadFile(): void {
    if (this.selectedPath) this.loadCurrentFile(this.viewerContent?.offset);
  }

  closeViewer(): void {
    this.stopTail();
    this.selectedPath = null;
    this.selectedName = '';
    this.viewerContent = null;
    this.parsedLines = [];
    localStorage.removeItem(this.LS_KEY);
  }

  // ── Pagination ──────────────────────────────────────────────────

  canPrevPage(): boolean {
    return !!this.viewerContent && this.viewerContent.offset > 0 && !this.viewerLoading && !this.following;
  }
  canNextPage(): boolean {
    return !!this.viewerContent
      && this.viewerContent.offset + this.viewerContent.lines_returned < this.viewerContent.total_lines
      && !this.viewerLoading && !this.following;
  }
  prevPage(): void {
    if (!this.viewerContent) return;
    const o = Math.max(0, this.viewerContent.offset - this.PAGE_SIZE);
    this.loadCurrentFile(o);
  }
  nextPage(): void {
    if (!this.viewerContent) return;
    const o = this.viewerContent.offset + this.viewerContent.lines_returned;
    this.loadCurrentFile(o);
  }

  // ── Filters ─────────────────────────────────────────────────────

  setLevelFilter(l: LevelFilter): void {
    this.levelFilter = l;
    this.onFilterChange();
  }

  onFilterChange(): void {
    this.recomputeSearchMatches();
  }

  clearFilters(): void {
    this.levelFilter = 'ALL';
    this.timeFilter = 'all';
    this.inFileSearch = '';
    this.inFileSearchLower = '';
    this.onFilterChange();
  }

  visibleLines(): ParsedLine[] {
    if (this.levelFilter === 'ALL' && this.timeFilter === 'all') {
      return this.parsedLines;
    }
    const cutoff = this.timeCutoff();
    return this.parsedLines.filter(l => {
      if (this.levelFilter !== 'ALL' && l.level !== this.levelFilter) return false;
      if (cutoff !== null && l.ts !== null && l.ts < cutoff) return false;
      return true;
    });
  }

  visibleCount(): number {
    return this.visibleLines().length;
  }

  private timeCutoff(): number | null {
    if (this.timeFilter === 'all') return null;
    const now = Date.now();
    const mins: Record<TimeFilter, number> = { all: 0, '10m': 10, '1h': 60, '6h': 360, '24h': 1440 };
    return now - mins[this.timeFilter] * 60_000;
  }

  // ── In-file search ──────────────────────────────────────────────

  onInFileSearchChange(): void {
    this.inFileSearchLower = this.inFileSearch.trim().toLowerCase();
    this.recomputeSearchMatches();
  }

  private recomputeSearchMatches(): void {
    if (!this.inFileSearchLower) {
      this.searchMatches = [];
      this.currentMatchIdx = 0;
      return;
    }
    const visible = this.visibleLines();
    this.searchMatches = visible
      .filter(l => l.text.toLowerCase().includes(this.inFileSearchLower))
      .map(l => l.num);
    this.currentMatchIdx = 0;
    if (this.searchMatches.length) this.scrollToMatch();
  }

  isMatchLine(num: number): boolean {
    if (!this.searchMatches.length) return false;
    return this.searchMatches[this.currentMatchIdx] === num;
  }

  jumpMatch(dir: 1 | -1): void {
    if (!this.searchMatches.length) return;
    this.currentMatchIdx =
      (this.currentMatchIdx + dir + this.searchMatches.length) % this.searchMatches.length;
    this.scrollToMatch();
  }

  private scrollToMatch(): void {
    if (!this.searchMatches.length) return;
    const num = this.searchMatches[this.currentMatchIdx];
    // Defer so the DOM has applied the .line-match class before scrolling
    queueMicrotask(() => {
      const el = document.querySelector(`.line[data-line-num="${num}"]`);
      if (el) el.scrollIntoView({ block: 'center', behavior: 'auto' });
    });
  }

  renderText(text: string): string {
    const escaped = text
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;');
    if (!this.inFileSearchLower) return escaped;
    const needle = this.inFileSearchLower;
    const lower = escaped.toLowerCase();
    let out = '';
    let i = 0;
    while (i < lower.length) {
      const idx = lower.indexOf(needle, i);
      if (idx === -1) { out += escaped.slice(i); break; }
      out += escaped.slice(i, idx);
      out += `<span class="search-hit">${escaped.slice(idx, idx + needle.length)}</span>`;
      i = idx + needle.length;
    }
    return out;
  }

  // ── Live tail ───────────────────────────────────────────────────

  async toggleFollow(): Promise<void> {
    if (this.following) await this.stopTail();
    else await this.startTail();
  }

  private async startTail(): Promise<void> {
    if (!this.selectedPath || this.following) return;
    try {
      this.streamId = await this.tauri.invoke<string>('tail_log_start', { path: this.selectedPath });
      this.unlisten = await listen<LogTailEvent>(`log-tail-${this.streamId}`, (evt) => {
        this.onTailEvent(evt.payload);
      });
      this.following = true;
    } catch (e) {
      console.error('Failed to start tail:', e);
      this.streamId = null;
      this.following = false;
    }
  }

  private async stopTail(): Promise<void> {
    if (this.unlisten) { this.unlisten(); this.unlisten = null; }
    if (this.streamId) {
      try {
        await this.tauri.invoke('tail_log_stop', { streamId: this.streamId });
      } catch { /* ignore */ }
      this.streamId = null;
    }
    this.following = false;
  }

  private onTailEvent(evt: LogTailEvent): void {
    if (evt.ended) {
      this.stopTail();
      return;
    }
    if (!evt.lines.length || !this.viewerContent) return;

    // Start from the current end-of-content as the next line number
    const nextNum = (this.viewerContent.offset + this.viewerContent.lines_returned) + 1;
    const appended: ParsedLine[] = evt.lines.map((text, i) => this.parseLine(text, nextNum + i));

    this.parsedLines = this.parsedLines.concat(appended);
    // Cap the buffer so live-tailing a chatty service doesn't grow unbounded
    if (this.parsedLines.length > this.LIVE_BUFFER_CAP) {
      const drop = this.parsedLines.length - this.LIVE_BUFFER_CAP;
      this.parsedLines = this.parsedLines.slice(drop);
    }
    // Update LogFileContent counters so the header reflects the new state
    this.viewerContent = {
      ...this.viewerContent,
      lines_returned: this.viewerContent.lines_returned + appended.length,
      total_lines: this.viewerContent.total_lines + appended.length
    };

    this.recomputeSearchMatches();
    this.cdr.detectChanges();

    // Auto-scroll to bottom while Following — naive, doesn't pause on user scroll-up
    queueMicrotask(() => {
      const viewer = document.querySelector('.pane-viewer');
      if (viewer) viewer.scrollTop = viewer.scrollHeight;
    });
  }

  // ── Copy / download ─────────────────────────────────────────────

  copyVisible(): void {
    const text = this.visibleLines().map(l => l.text).join('\n');
    navigator.clipboard?.writeText(text).catch(() => { /* ignore */ });
  }

  downloadVisible(): void {
    const text = this.visibleLines().map(l => l.text).join('\n');
    const blob = new Blob([text], { type: 'text/plain' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${this.selectedName || 'log'}.txt`;
    a.click();
    URL.revokeObjectURL(url);
  }

  // ── Persistence ─────────────────────────────────────────────────

  private restoreLastSelection(): void {
    const raw = localStorage.getItem(this.LS_KEY);
    if (!raw) return;
    try {
      const { path, name } = JSON.parse(raw);
      if (typeof path === 'string' && typeof name === 'string') {
        // Only restore if the path still exists in the current source list
        // (host file may have been rotated away, container may have stopped)
        const exists = path.startsWith('container:')
          ? this.sources.some(s => s.path === path)
          : true; // host files aren't in this.sources directly; trust the path
        if (exists) {
          this.selectPath(path, name);
        }
      }
    } catch { /* corrupt — ignore */ }
  }

  // ── Misc ────────────────────────────────────────────────────────

  formatSize(bytes: number): string {
    if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${bytes} B`;
  }
}
