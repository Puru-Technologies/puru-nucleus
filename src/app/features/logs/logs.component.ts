import { Component, OnInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import {
  TauriService,
  LogSource,
  LogFileInfo,
  LogFileContent
} from '../../core/services/tauri.service';

@Component({
  selector: 'app-logs',
  standalone: true,
  imports: [
    CommonModule,
    FormsModule
  ],
  template: `
    <div class="page">
      <div class="page-header">
        <div>
          <h1>Log Files</h1>
          <p class="page-subtitle">Browse and read host log files</p>
        </div>
        <div class="header-actions">
          <div class="field source-select">
            <label>Source</label>
            <select class="select" [(ngModel)]="selectedSource" (ngModelChange)="onSourceChange()">
              <option value="all">All Sources</option>
              @for (source of sources; track source.path) {
                <option [value]="source.path">{{ source.name }}</option>
              }
            </select>
          </div>
          <button class="btn btn-stroked" (click)="loadFiles()">
            <span class="material-icons">refresh</span>
            Refresh
          </button>
        </div>
      </div>

      <!-- Log Viewer Panel -->
      @if (viewerContent) {
        <div class="card log-card">
          <div class="log-header">
            <div class="log-title">
              <span class="material-icons">article</span>
              <span>{{ viewerFileName }}</span>
              <span class="log-line-info">
                Lines {{ viewerContent.offset + 1 }}-{{ viewerContent.offset + viewerContent.lines_returned }}
                of {{ viewerContent.total_lines }}
              </span>
            </div>
            <div class="log-actions">
              <button class="btn-icon" (click)="prevPage()" [disabled]="viewerContent.offset === 0 || viewerLoading">
                <span class="material-icons">chevron_left</span>
              </button>
              <button class="btn-icon" (click)="nextPage()"
                [disabled]="viewerContent.offset + viewerContent.lines_returned >= viewerContent.total_lines || viewerLoading">
                <span class="material-icons">chevron_right</span>
              </button>
              <button class="btn-icon" (click)="closeViewer()">
                <span class="material-icons">close</span>
              </button>
            </div>
          </div>
          @if (viewerLoading) {
            <div class="log-loading">
              <span class="spinner"></span>
            </div>
          } @else {
            <pre class="log-content">{{ viewerContent.content || 'File is empty.' }}</pre>
          }
        </div>
      }

      @if (loading) {
        <div class="loading-state">
          <span class="spinner spinner-lg"></span>
        </div>
      } @else if (files.length === 0) {
        <div class="card empty-card">
          <div class="empty-state">
            <div class="empty-icon">
              <span class="material-icons">description</span>
            </div>
            <h3>No Log Files Found</h3>
            <p>No .log, .err, .out, or .gz files found in the selected source directory.</p>
          </div>
        </div>
      } @else {
        <div class="card table-card">
          <table class="data-table files-table">
            <thead>
              <tr>
                <th>File</th>
                <th>Size</th>
                <th>Modified</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              @for (file of files; track file.path) {
                <tr>
                  <td>
                    <div class="name-cell">
                      <span class="material-icons file-icon">description</span>
                      <div class="name-info">
                        <span class="name-primary">{{ file.name }}</span>
                        <span class="name-secondary">{{ file.path }}</span>
                      </div>
                    </div>
                  </td>
                  <td><span class="mono-text">{{ formatSize(file.size_bytes) }}</span></td>
                  <td><span class="mono-text">{{ file.modified_at }}</span></td>
                  <td>
                    <button class="btn btn-stroked btn-sm view-btn" (click)="viewFile(file)">
                      <span class="material-icons">visibility</span>
                      View
                    </button>
                  </td>
                </tr>
              }
            </tbody>
          </table>
        </div>
      }
    </div>
  `,
  styles: [`
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

    .header-actions {
      display: flex;
      gap: 10px;
      align-items: center;
    }

    .source-select {
      width: 200px;
      font-size: 0.85rem;
      margin-bottom: 0;
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
          width: 32px;
          height: 32px;
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
    }

    /* ── Table ────────────────────────────────── */
    .table-card {
      overflow: hidden;
    }

    .files-table {
      width: 100%;
    }

    .name-cell {
      display: flex;
      align-items: center;
      gap: 12px;
    }

    .file-icon {
      color: var(--text-muted);
      font-size: 20px;
      width: 20px;
      height: 20px;
    }
    .material-icons.file-icon {
      font-size: 20px;
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
      max-width: 500px;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .mono-text {
      font-size: 0.8rem;
      font-family: 'SF Mono', 'Fira Code', monospace;
      color: var(--text-secondary);
    }

    .view-btn {
      font-size: 0.8rem;
    }

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
        width: 18px;
        height: 18px;
      }
    }

    .log-line-info {
      font-size: 0.75rem;
      font-weight: 400;
      color: #94a3b8;
      margin-left: 8px;
      font-family: 'SF Mono', 'Fira Code', monospace;
    }

    .log-actions {
      display: flex;
      align-items: center;
      gap: 2px;

      .btn-icon {
        color: #94a3b8;
        &:hover:not(:disabled) { background: rgba(148, 163, 184, 0.15); color: #e2e8f0; }
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
      max-height: 500px;
      overflow-y: auto;
      white-space: pre-wrap;
      word-break: break-all;
    }
  `]
})
export class LogsComponent implements OnInit {
  private tauri = inject(TauriService);

  sources: LogSource[] = [];
  files: LogFileInfo[] = [];
  loading = true;
  selectedSource = 'all';

  viewerContent: LogFileContent | null = null;
  viewerFileName = '';
  viewerFilePath = '';
  viewerLoading = false;
  pageSize = 500;

  displayedColumns = ['name', 'size', 'modified', 'actions'];

  ngOnInit(): void {
    this.loadSources();
  }

  async loadSources(): Promise<void> {
    try {
      this.sources = await this.tauri.invoke<LogSource[]>('get_log_sources');
    } catch {
      this.sources = [];
    }
    await this.loadFiles();
  }

  async loadFiles(): Promise<void> {
    this.loading = true;
    try {
      const args: Record<string, unknown> = {};
      if (this.selectedSource !== 'all') {
        args['path'] = this.selectedSource;
      }
      this.files = await this.tauri.invoke<LogFileInfo[]>('list_log_files', args);
    } catch {
      this.files = [];
    } finally {
      this.loading = false;
    }
  }

  onSourceChange(): void {
    this.loadFiles();
  }

  formatSize(bytes: number): string {
    if (bytes >= 1024 * 1024) {
      return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    }
    if (bytes >= 1024) {
      return `${(bytes / 1024).toFixed(1)} KB`;
    }
    return `${bytes} B`;
  }

  async viewFile(file: LogFileInfo): Promise<void> {
    this.viewerFileName = file.name;
    this.viewerFilePath = file.path;
    this.viewerLoading = true;
    this.viewerContent = { path: file.path, content: '', total_lines: 0, offset: 0, lines_returned: 0 };
    try {
      this.viewerContent = await this.tauri.invoke<LogFileContent>('read_log_file', {
        path: file.path,
        tail: this.pageSize
      });
    } catch (e) {
      this.viewerContent = {
        path: file.path,
        content: `Failed to read file: ${e}`,
        total_lines: 0,
        offset: 0,
        lines_returned: 0
      };
    } finally {
      this.viewerLoading = false;
    }
  }

  async prevPage(): Promise<void> {
    if (!this.viewerContent) return;
    const newOffset = Math.max(0, this.viewerContent.offset - this.pageSize);
    await this.loadPage(newOffset);
  }

  async nextPage(): Promise<void> {
    if (!this.viewerContent) return;
    const newOffset = this.viewerContent.offset + this.viewerContent.lines_returned;
    await this.loadPage(newOffset);
  }

  private async loadPage(offset: number): Promise<void> {
    this.viewerLoading = true;
    try {
      this.viewerContent = await this.tauri.invoke<LogFileContent>('read_log_file', {
        path: this.viewerFilePath,
        offset,
        limit: this.pageSize
      });
    } catch (e) {
      // Keep current content on error
    } finally {
      this.viewerLoading = false;
    }
  }

  closeViewer(): void {
    this.viewerContent = null;
    this.viewerFileName = '';
    this.viewerFilePath = '';
  }
}
