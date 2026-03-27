import { Component, OnInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { MatCardModule } from '@angular/material/card';
import { MatIconModule } from '@angular/material/icon';
import { MatButtonModule } from '@angular/material/button';
import { MatTableModule } from '@angular/material/table';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
import { MatSelectModule } from '@angular/material/select';
import { MatFormFieldModule } from '@angular/material/form-field';
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
    FormsModule,
    MatCardModule,
    MatIconModule,
    MatButtonModule,
    MatTableModule,
    MatProgressSpinnerModule,
    MatSelectModule,
    MatFormFieldModule
  ],
  template: `
    <div class="page">
      <div class="page-header">
        <div>
          <h1>Log Files</h1>
          <p class="page-subtitle">Browse and read host log files</p>
        </div>
        <div class="header-actions">
          <mat-form-field class="source-select" appearance="outline" subscriptSizing="dynamic">
            <mat-label>Source</mat-label>
            <mat-select [(ngModel)]="selectedSource" (ngModelChange)="onSourceChange()">
              <mat-option value="all">All Sources</mat-option>
              @for (source of sources; track source.path) {
                <mat-option [value]="source.path">{{ source.name }}</mat-option>
              }
            </mat-select>
          </mat-form-field>
          <button mat-stroked-button (click)="loadFiles()">
            <mat-icon>refresh</mat-icon>
            Refresh
          </button>
        </div>
      </div>

      <!-- Log Viewer Panel -->
      @if (viewerContent) {
        <mat-card class="log-card">
          <div class="log-header">
            <div class="log-title">
              <mat-icon>article</mat-icon>
              <span>{{ viewerFileName }}</span>
              <span class="log-line-info">
                Lines {{ viewerContent.offset + 1 }}-{{ viewerContent.offset + viewerContent.lines_returned }}
                of {{ viewerContent.total_lines }}
              </span>
            </div>
            <div class="log-actions">
              <button mat-icon-button (click)="prevPage()" [disabled]="viewerContent.offset === 0 || viewerLoading">
                <mat-icon>chevron_left</mat-icon>
              </button>
              <button mat-icon-button (click)="nextPage()"
                [disabled]="viewerContent.offset + viewerContent.lines_returned >= viewerContent.total_lines || viewerLoading">
                <mat-icon>chevron_right</mat-icon>
              </button>
              <button mat-icon-button (click)="closeViewer()">
                <mat-icon>close</mat-icon>
              </button>
            </div>
          </div>
          @if (viewerLoading) {
            <div class="log-loading">
              <mat-spinner diameter="24"></mat-spinner>
            </div>
          } @else {
            <pre class="log-content">{{ viewerContent.content || 'File is empty.' }}</pre>
          }
        </mat-card>
      }

      @if (loading) {
        <div class="loading-state">
          <mat-spinner diameter="40"></mat-spinner>
        </div>
      } @else if (files.length === 0) {
        <mat-card class="empty-card">
          <div class="empty-state">
            <div class="empty-icon">
              <mat-icon>description</mat-icon>
            </div>
            <h3>No Log Files Found</h3>
            <p>No .log, .err, .out, or .gz files found in the selected source directory.</p>
          </div>
        </mat-card>
      } @else {
        <mat-card class="table-card">
          <table mat-table [dataSource]="files" class="files-table">
            <ng-container matColumnDef="name">
              <th mat-header-cell *matHeaderCellDef>File</th>
              <td mat-cell *matCellDef="let file">
                <div class="name-cell">
                  <mat-icon class="file-icon">description</mat-icon>
                  <div class="name-info">
                    <span class="name-primary">{{ file.name }}</span>
                    <span class="name-secondary">{{ file.path }}</span>
                  </div>
                </div>
              </td>
            </ng-container>

            <ng-container matColumnDef="size">
              <th mat-header-cell *matHeaderCellDef>Size</th>
              <td mat-cell *matCellDef="let file">
                <span class="mono-text">{{ formatSize(file.size_bytes) }}</span>
              </td>
            </ng-container>

            <ng-container matColumnDef="modified">
              <th mat-header-cell *matHeaderCellDef>Modified</th>
              <td mat-cell *matCellDef="let file">
                <span class="mono-text">{{ file.modified_at }}</span>
              </td>
            </ng-container>

            <ng-container matColumnDef="actions">
              <th mat-header-cell *matHeaderCellDef></th>
              <td mat-cell *matCellDef="let file">
                <button mat-stroked-button class="view-btn" (click)="viewFile(file)">
                  <mat-icon>visibility</mat-icon>
                  View
                </button>
              </td>
            </ng-container>

            <tr mat-header-row *matHeaderRowDef="displayedColumns"></tr>
            <tr mat-row *matRowDef="let row; columns: displayedColumns;"></tr>
          </table>
        </mat-card>
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
        mat-icon {
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
      padding: 0 !important;
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

      mat-icon {
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

      button {
        color: #94a3b8;
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
