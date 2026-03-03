import { Component, OnInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { MatCardModule } from '@angular/material/card';
import { MatIconModule } from '@angular/material/icon';
import { MatButtonModule } from '@angular/material/button';
import { MatChipsModule } from '@angular/material/chips';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
import { MatBadgeModule } from '@angular/material/badge';
import {
  HospitalMessage,
  MessageDownloadResult,
  FileActionResult,
  MessageAttachment,
} from '../../core/models/hospital.model';
import { TauriService } from '../../core/services/tauri.service';
import { NotificationService } from '../../core/services/notification.service';

@Component({
  selector: 'app-inbox',
  standalone: true,
  imports: [
    CommonModule,
    MatCardModule,
    MatIconModule,
    MatButtonModule,
    MatChipsModule,
    MatProgressSpinnerModule,
    MatBadgeModule,
  ],
  template: `
    <div class="inbox-page p-4">
      <!-- Header -->
      <div class="header flex justify-between items-center">
        <div class="header-left">
          <h1>Inbox</h1>
          @if (unreadCount > 0) {
            <span class="unread-badge">{{ unreadCount }}</span>
          }
        </div>
        <button mat-stroked-button (click)="loadMessages()">
          <mat-icon>refresh</mat-icon>
          Refresh
        </button>
      </div>

      <!-- Summary cards -->
      <div class="summary-row">
        <mat-card class="summary-card">
          <mat-card-content>
            <mat-icon>mail</mat-icon>
            <span class="count">{{ messages.length }}</span>
            <span class="label">Total</span>
          </mat-card-content>
        </mat-card>

        <mat-card class="summary-card unread" [class.active]="unreadCount > 0">
          <mat-card-content>
            <mat-icon>mark_email_unread</mat-icon>
            <span class="count">{{ unreadCount }}</span>
            <span class="label">Unread</span>
          </mat-card-content>
        </mat-card>

        <mat-card class="summary-card high-priority" [class.active]="highPriorityCount > 0">
          <mat-card-content>
            <mat-icon>priority_high</mat-icon>
            <span class="count">{{ highPriorityCount }}</span>
            <span class="label">High Priority</span>
          </mat-card-content>
        </mat-card>
      </div>

      @if (loading) {
        <div class="loading-container">
          <mat-spinner diameter="48"></mat-spinner>
        </div>
      } @else if (messages.length === 0) {
        <mat-card class="empty-state">
          <mat-card-content>
            <mat-icon>mail_outline</mat-icon>
            <p>No messages</p>
            <span>Your inbox is empty</span>
          </mat-card-content>
        </mat-card>
      } @else {
        <div class="messages-list">
          @for (msg of messages; track msg.id) {
            <mat-card
              class="message-card"
              [class]="'priority-' + msg.priority"
              [class.unread]="!msg.read"
              [class.selected]="selectedMessage?.id === msg.id"
              (click)="selectMessage(msg)">
              <mat-card-content>
                <div class="message-header">
                  <mat-icon class="type-icon">{{ getTypeIcon(msg.message_type) }}</mat-icon>
                  <div class="message-info">
                    <div class="message-subject">{{ msg.subject }}</div>
                    <div class="message-meta">
                      <mat-chip size="small">{{ formatType(msg.message_type) }}</mat-chip>
                      @if (msg.priority !== 'normal') {
                        <span class="priority-label" [class]="'priority-text-' + msg.priority">
                          {{ msg.priority }}
                        </span>
                      }
                      <span class="timestamp">{{ msg.created_at | date:'short' }}</span>
                      @if (msg.created_by) {
                        <span class="sender">{{ msg.created_by }}</span>
                      }
                    </div>
                  </div>
                  @if (!msg.read) {
                    <span class="unread-dot"></span>
                  }
                  @if (msg.attachments.length > 0) {
                    <mat-icon class="attachment-icon">attach_file</mat-icon>
                  }
                </div>

                <!-- Expanded detail -->
                @if (selectedMessage?.id === msg.id) {
                  <div class="message-detail">
                    @if (msg.body) {
                      <p class="message-body">{{ msg.body }}</p>
                    }

                    @if (msg.attachments.length > 0) {
                      <div class="attachments-section">
                        <h4>Attachments</h4>
                        @for (att of msg.attachments; track att.filename; let i = $index) {
                          <div class="attachment-item">
                            <mat-icon>insert_drive_file</mat-icon>
                            <div class="attachment-info">
                              <span class="attachment-name">{{ att.filename }}</span>
                              @if (att.file_size) {
                                <span class="attachment-size">{{ formatFileSize(att.file_size) }}</span>
                              }
                            </div>
                            <div class="attachment-actions">
                              <button mat-stroked-button
                                      [disabled]="downloading.get(msg.id + '_' + i)"
                                      (click)="downloadAttachment(msg.id, i, att.file_url, $event)">
                                @if (downloading.get(msg.id + '_' + i)) {
                                  <mat-spinner diameter="16"></mat-spinner>
                                } @else {
                                  <mat-icon>download</mat-icon>
                                }
                                Download
                              </button>
                              @if (isConfigFile(att.filename)) {
                                <button mat-stroked-button
                                        [disabled]="!downloadedPaths.get(msg.id + '_' + i)"
                                        (click)="applyConfig(downloadedPaths.get(msg.id + '_' + i)!, $event)">
                                  <mat-icon>settings_applications</mat-icon>
                                  Apply Config
                                </button>
                              }
                              @if (isCertFile(att.filename)) {
                                <button mat-stroked-button
                                        [disabled]="!downloadedPaths.get(msg.id + '_' + i)"
                                        (click)="installCert(downloadedPaths.get(msg.id + '_' + i)!, $event)">
                                  <mat-icon>verified_user</mat-icon>
                                  Install Cert
                                </button>
                              }
                            </div>
                          </div>
                        }
                      </div>
                    }
                  </div>
                }
              </mat-card-content>
            </mat-card>
          }
        </div>
      }
    </div>
  `,
  styles: [`
    .inbox-page {
      max-width: 1000px;
      margin: 0 auto;
    }

    h1 {
      margin: 0;
      color: #333;
    }

    .header {
      margin-bottom: 1.5rem;
    }

    .header-left {
      display: flex;
      align-items: center;
      gap: 0.75rem;
    }

    .unread-badge {
      background: #6366f1;
      color: white;
      border-radius: 12px;
      padding: 2px 10px;
      font-size: 0.8rem;
      font-weight: 600;
    }

    /* ── Summary cards ──────────────────────────── */
    .summary-row {
      display: grid;
      grid-template-columns: repeat(3, 1fr);
      gap: 1rem;
      margin-bottom: 1.5rem;
    }

    .summary-card {
      text-align: center;
      opacity: 0.6;
      transition: opacity 0.2s;

      &.active {
        opacity: 1;
      }

      mat-card-content {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: 0.25rem;
      }

      mat-icon {
        font-size: 2rem;
        width: 2rem;
        height: 2rem;
        color: #6366f1;
      }

      .count {
        font-size: 2rem;
        font-weight: 500;
      }

      .label {
        font-size: 0.875rem;
        color: #666;
      }

      &.unread {
        mat-icon, .count { color: #2196f3; }
        &.active { border-left: 4px solid #2196f3; }
      }

      &.high-priority {
        mat-icon, .count { color: #ff9800; }
        &.active { border-left: 4px solid #ff9800; }
      }
    }

    /* ── Loading & empty ────────────────────────── */
    .loading-container {
      display: flex;
      justify-content: center;
      padding: 3rem;
    }

    .empty-state {
      text-align: center;
      padding: 3rem;
      color: #6366f1;

      mat-icon {
        font-size: 4rem;
        width: 4rem;
        height: 4rem;
      }

      p {
        font-size: 1.25rem;
        margin: 1rem 0 0.5rem;
      }

      span {
        color: #666;
      }
    }

    /* ── Message list ───────────────────────────── */
    .messages-list {
      display: flex;
      flex-direction: column;
      gap: 0.75rem;
    }

    .message-card {
      cursor: pointer;
      transition: all 0.15s ease;

      &:hover {
        box-shadow: 0 2px 8px rgba(0, 0, 0, 0.1);
      }

      &.unread {
        border-left: 4px solid #6366f1;

        .message-subject {
          font-weight: 600;
        }
      }

      &.priority-critical {
        border-left: 4px solid #f44336;
      }

      &.priority-high {
        border-left: 4px solid #ff9800;
      }

      &.priority-low {
        opacity: 0.8;
      }

      &.selected {
        box-shadow: 0 2px 12px rgba(99, 102, 241, 0.15);
      }
    }

    .message-header {
      display: flex;
      align-items: flex-start;
      gap: 1rem;
    }

    .type-icon {
      color: #6366f1;
      margin-top: 2px;
    }

    .message-info {
      flex: 1;
    }

    .message-subject {
      font-size: 0.95rem;
      margin-bottom: 0.25rem;
    }

    .message-meta {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      font-size: 0.8rem;
      color: #666;
      flex-wrap: wrap;
    }

    .priority-text-high {
      color: #ff9800;
      font-weight: 500;
      text-transform: uppercase;
      font-size: 0.7rem;
    }

    .priority-text-critical {
      color: #f44336;
      font-weight: 600;
      text-transform: uppercase;
      font-size: 0.7rem;
    }

    .priority-text-low {
      color: #9e9e9e;
      text-transform: uppercase;
      font-size: 0.7rem;
    }

    .timestamp {
      color: #999;
    }

    .sender {
      color: #999;
    }

    .unread-dot {
      width: 8px;
      height: 8px;
      border-radius: 50%;
      background: #6366f1;
      flex-shrink: 0;
      margin-top: 6px;
    }

    .attachment-icon {
      color: #999;
      font-size: 18px;
      width: 18px;
      height: 18px;
      margin-top: 2px;
    }

    /* ── Message detail (expanded) ──────────────── */
    .message-detail {
      margin-top: 1rem;
      padding-top: 1rem;
      border-top: 1px solid #eee;
    }

    .message-body {
      color: #555;
      line-height: 1.6;
      white-space: pre-wrap;
      margin: 0 0 1rem 0;
    }

    .attachments-section {
      h4 {
        margin: 0 0 0.75rem 0;
        font-size: 0.875rem;
        color: #333;
      }
    }

    .attachment-item {
      display: flex;
      align-items: center;
      gap: 0.75rem;
      padding: 0.75rem;
      background: #f8f9fa;
      border-radius: 8px;
      margin-bottom: 0.5rem;

      mat-icon:first-child {
        color: #999;
      }
    }

    .attachment-info {
      flex: 1;
      display: flex;
      flex-direction: column;
    }

    .attachment-name {
      font-weight: 500;
      font-size: 0.875rem;
    }

    .attachment-size {
      font-size: 0.75rem;
      color: #999;
    }

    .attachment-actions {
      display: flex;
      gap: 0.5rem;

      button {
        font-size: 0.8rem;
      }
    }
  `]
})
export class InboxComponent implements OnInit {
  private tauri = inject(TauriService);
  private notification = inject(NotificationService);

  messages: HospitalMessage[] = [];
  selectedMessage: HospitalMessage | null = null;
  loading = true;
  downloading = new Map<string, boolean>();
  downloadedPaths = new Map<string, string>();

  get unreadCount(): number {
    return this.messages.filter(m => !m.read).length;
  }

  get highPriorityCount(): number {
    return this.messages.filter(m => m.priority === 'high' || m.priority === 'critical').length;
  }

  ngOnInit(): void {
    this.loadMessages();
  }

  async loadMessages(): Promise<void> {
    this.loading = true;
    try {
      this.messages = await this.tauri.invoke<HospitalMessage[]>('get_messages');
    } finally {
      this.loading = false;
    }
  }

  async selectMessage(msg: HospitalMessage): Promise<void> {
    if (this.selectedMessage?.id === msg.id) {
      this.selectedMessage = null;
      return;
    }

    this.selectedMessage = msg;

    if (!msg.read) {
      try {
        await this.tauri.invoke('mark_message_read', { messageId: msg.id });
        msg.read = true;
      } catch (error) {
        // Error handled by TauriService
      }
    }
  }

  async downloadAttachment(messageId: string, index: number, fileUrl: string, event: Event): Promise<void> {
    event.stopPropagation();
    const key = `${messageId}_${index}`;
    this.downloading.set(key, true);

    try {
      const result = await this.tauri.invoke<MessageDownloadResult>('download_attachment', {
        messageId,
        attachmentIndex: index,
        fileUrl,
      });

      if (result.success && result.file_path) {
        this.downloadedPaths.set(key, result.file_path);
        this.notification.success(`Downloaded to ${result.file_path}`);
      } else {
        this.notification.error(result.error || 'Download failed');
      }
    } catch (error) {
      // Error handled by TauriService
    } finally {
      this.downloading.set(key, false);
    }
  }

  async applyConfig(filePath: string, event: Event): Promise<void> {
    event.stopPropagation();
    try {
      const result = await this.tauri.invoke<FileActionResult>('apply_config_file', { filePath });
      if (result.success) {
        this.notification.success(result.message);
      } else {
        this.notification.error(result.message);
      }
    } catch (error) {
      // Error handled by TauriService
    }
  }

  async installCert(filePath: string, event: Event): Promise<void> {
    event.stopPropagation();
    try {
      const result = await this.tauri.invoke<FileActionResult>('install_certificate', { filePath });
      if (result.success) {
        this.notification.success(result.message);
      } else {
        this.notification.error(result.message);
      }
    } catch (error) {
      // Error handled by TauriService
    }
  }

  getTypeIcon(type: string): string {
    switch (type) {
      case 'sa_key': return 'key';
      case 'config_update': return 'settings';
      case 'alert': return 'warning';
      case 'file': return 'folder';
      default: return 'info';
    }
  }

  formatType(type: string): string {
    switch (type) {
      case 'sa_key': return 'SA Key';
      case 'config_update': return 'Config';
      case 'alert': return 'Alert';
      case 'file': return 'File';
      default: return 'Info';
    }
  }

  formatFileSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }

  isConfigFile(filename: string): boolean {
    return filename.endsWith('.toml') || filename.endsWith('.json');
  }

  isCertFile(filename: string): boolean {
    return ['.pem', '.crt', '.cer', '.key'].some(ext => filename.endsWith(ext));
  }
}
