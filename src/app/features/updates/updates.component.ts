import { Component, OnInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { MatCardModule } from '@angular/material/card';
import { MatIconModule } from '@angular/material/icon';
import { MatButtonModule } from '@angular/material/button';
import { MatChipsModule } from '@angular/material/chips';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
import { TauriService, NucleusUpdateInfo, ServiceUpdateInfo, DockerUpdateResult, UpdateRecord } from '../../core/services/tauri.service';
import { NotificationService } from '../../core/services/notification.service';

@Component({
  selector: 'app-updates',
  standalone: true,
  imports: [
    CommonModule,
    MatCardModule,
    MatIconModule,
    MatButtonModule,
    MatChipsModule,
    MatProgressSpinnerModule
  ],
  template: `
    <div class="updates-page p-4">
      <div class="header flex justify-between items-center">
        <h1>Updates</h1>
        <button mat-stroked-button (click)="checkAll()" [disabled]="checking">
          @if (checking) {
            <mat-spinner diameter="18"></mat-spinner>
          } @else {
            <mat-icon>refresh</mat-icon>
          }
          Check for Updates
        </button>
      </div>

      <!-- Nucleus Update Card -->
      <mat-card class="nucleus-card">
        <mat-card-header>
          <mat-icon mat-card-avatar>hub</mat-icon>
          <mat-card-title>Nucleus</mat-card-title>
          <mat-card-subtitle>Desktop application</mat-card-subtitle>
        </mat-card-header>
        <mat-card-content>
          @if (nucleusLoading) {
            <div class="loading-row">
              <mat-spinner diameter="24"></mat-spinner>
              <span>Checking for updates...</span>
            </div>
          } @else if (nucleusError) {
            <div class="error-row">
              <mat-icon>error_outline</mat-icon>
              <span>{{ nucleusError }}</span>
            </div>
          } @else if (nucleusInfo) {
            <div class="version-row">
              <div class="version-info">
                <span class="label">Current Version</span>
                <span class="value">v{{ nucleusInfo.current_version }}</span>
              </div>
              @if (nucleusInfo.update_available) {
                <div class="version-info">
                  <span class="label">Latest Version</span>
                  <span class="value new">v{{ nucleusInfo.latest_version }}</span>
                </div>
                <mat-chip class="update-chip">Update Available</mat-chip>
              } @else {
                <mat-chip class="up-to-date-chip">Up to Date</mat-chip>
              }
            </div>

            @if (nucleusInfo.update_available) {
              <div class="update-details">
                <div class="release-meta">
                  <span class="release-date">Released: {{ nucleusInfo.release_date }}</span>
                  @if (nucleusInfo.download_size_mb) {
                    <span class="download-size">{{ nucleusInfo.download_size_mb | number:'1.1-1' }} MB</span>
                  }
                </div>
                <p class="release-notes">{{ nucleusInfo.release_notes }}</p>
                <div class="nucleus-actions">
                  <button mat-raised-button (click)="downloadNucleus()" [disabled]="nucleusDownloading || nucleusInstalling">
                    @if (nucleusDownloading) {
                      <mat-spinner diameter="18"></mat-spinner>
                    } @else {
                      <mat-icon>download</mat-icon>
                    }
                    Download
                  </button>
                  <button mat-raised-button color="primary" (click)="downloadAndInstallNucleus()" [disabled]="nucleusDownloading || nucleusInstalling">
                    @if (nucleusInstalling) {
                      <mat-spinner diameter="18"></mat-spinner>
                    } @else {
                      <mat-icon>system_update_alt</mat-icon>
                    }
                    Download &amp; Install
                  </button>
                </div>
              </div>
            }
          } @else {
            <div class="empty-row">
              <mat-icon>info_outline</mat-icon>
              <span>Click "Check for Updates" to get started</span>
            </div>
          }
        </mat-card-content>
      </mat-card>

      <!-- Service Updates -->
      <h2>Service Updates</h2>

      @if (servicesLoading) {
        <div class="loading-container">
          <mat-spinner diameter="36"></mat-spinner>
          <span>Checking services...</span>
        </div>
      } @else if (servicesError) {
        <mat-card class="error-card">
          <mat-card-content>
            <mat-icon>error_outline</mat-icon>
            <span>{{ servicesError }}</span>
          </mat-card-content>
        </mat-card>
      } @else if (serviceUpdates.length === 0 && !servicesLoading) {
        <mat-card class="empty-state">
          <mat-card-content>
            <mat-icon>inventory_2</mat-icon>
            <p>No services found</p>
            <span>No running Puru services detected. Make sure Docker is running.</span>
          </mat-card-content>
        </mat-card>
      } @else {
        <div class="services-list">
          @for (svc of serviceUpdates; track svc.service) {
            <mat-card class="service-card" [class.has-update]="svc.update_available">
              <mat-card-content>
                <div class="service-row">
                  <div class="service-name">
                    <mat-icon>{{ svc.update_available ? 'upgrade' : 'check_circle' }}</mat-icon>
                    <span>{{ svc.service }}</span>
                  </div>
                  <div class="service-versions">
                    <div class="version-col">
                      <span class="label">Current</span>
                      <span class="value">{{ svc.current_version }}</span>
                    </div>
                    <mat-icon class="arrow">arrow_forward</mat-icon>
                    <div class="version-col">
                      <span class="label">Latest</span>
                      <span class="value" [class.new]="svc.update_available">{{ svc.latest_version }}</span>
                    </div>
                  </div>
                  <div class="service-status">
                    @if (svc.update_available) {
                      <mat-chip class="update-chip">Update Available</mat-chip>
                    } @else {
                      <mat-chip class="up-to-date-chip">Current</mat-chip>
                    }
                  </div>
                  <div class="service-actions">
                    @if (svc.update_available) {
                      <button mat-raised-button color="primary"
                              (click)="updateDockerService(svc)"
                              [disabled]="updatingService === svc.service || rollingBackService === svc.service">
                        @if (updatingService === svc.service) {
                          <mat-spinner diameter="16"></mat-spinner>
                        } @else {
                          <mat-icon>system_update_alt</mat-icon>
                        }
                        Update
                      </button>
                      <button mat-stroked-button
                              (click)="downloadServiceJar(svc)"
                              [disabled]="downloadingService === svc.service">
                        @if (downloadingService === svc.service) {
                          <mat-spinner diameter="16"></mat-spinner>
                        } @else {
                          <mat-icon>download</mat-icon>
                        }
                        JAR
                      </button>
                    } @else {
                      <button mat-stroked-button
                              (click)="rollbackDockerService(svc.service)"
                              [disabled]="rollingBackService === svc.service || updatingService === svc.service">
                        @if (rollingBackService === svc.service) {
                          <mat-spinner diameter="16"></mat-spinner>
                        } @else {
                          <mat-icon>undo</mat-icon>
                        }
                        Rollback
                      </button>
                    }
                  </div>
                </div>
                @if (svc.update_available && svc.changelog) {
                  <div class="changelog">
                    <span class="label">Changes:</span> {{ svc.changelog }}
                  </div>
                }
              </mat-card-content>
            </mat-card>
          }
        </div>
      }

      <!-- Update History -->
      <h2>Update History</h2>

      @if (historyLoading) {
        <div class="loading-container">
          <mat-spinner diameter="24"></mat-spinner>
          <span>Loading history...</span>
        </div>
      } @else if (updateHistory.length === 0) {
        <mat-card class="empty-state">
          <mat-card-content>
            <mat-icon>history</mat-icon>
            <p>No update history</p>
            <span>Docker update records will appear here after updates are performed.</span>
          </mat-card-content>
        </mat-card>
      } @else {
        <div class="history-list">
          @for (record of updateHistory; track record.timestamp) {
            <mat-card class="history-card" [class.success]="record.success && !record.rolled_back" [class.failed]="!record.success || record.rolled_back">
              <mat-card-content>
                <div class="history-row">
                  <div class="history-service">
                    <mat-icon>{{ record.success && !record.rolled_back ? 'check_circle' : 'error' }}</mat-icon>
                    <span class="service-label">{{ record.service }}</span>
                  </div>
                  <div class="history-images">
                    <code>{{ shortenImage(record.previous_image) }}</code>
                    <mat-icon class="arrow">arrow_forward</mat-icon>
                    <code>{{ shortenImage(record.new_image) }}</code>
                  </div>
                  <div class="history-badges">
                    @if (record.rolled_back) {
                      <mat-chip class="rollback-chip">Rolled Back</mat-chip>
                    } @else if (record.success) {
                      <mat-chip class="up-to-date-chip">Success</mat-chip>
                    } @else {
                      <mat-chip class="update-chip">Failed</mat-chip>
                    }
                  </div>
                  <div class="history-meta">
                    <span class="timestamp">{{ record.timestamp | date:'short' }}</span>
                    <span class="duration">{{ record.duration_seconds }}s</span>
                  </div>
                </div>
              </mat-card-content>
            </mat-card>
          }
        </div>
      }
    </div>
  `,
  styles: [`
    .updates-page {
      max-width: 1000px;
      margin: 0 auto;
    }

    h1 {
      margin: 0;
      color: #333;
    }

    h2 {
      margin: 2rem 0 1rem;
      color: #333;
      font-size: 1.25rem;
    }

    .header {
      margin-bottom: 1.5rem;
    }

    /* Nucleus card */
    .nucleus-card {
      mat-card-header {
        margin-bottom: 1rem;
      }

      mat-icon[mat-card-avatar] {
        background: linear-gradient(135deg, #6366f1, #8b5cf6);
        padding: 8px;
        border-radius: 50%;
        color: white;
      }
    }

    .version-row {
      display: flex;
      align-items: center;
      gap: 2rem;
      flex-wrap: wrap;
    }

    .version-info {
      display: flex;
      flex-direction: column;
      gap: 0.25rem;

      .label {
        font-size: 0.75rem;
        color: #666;
        text-transform: uppercase;
        letter-spacing: 0.05em;
      }

      .value {
        font-size: 1.25rem;
        font-weight: 600;
        color: #333;

        &.new {
          color: #4caf50;
        }
      }
    }

    .update-chip {
      background: #fff3e0 !important;
      color: #e65100 !important;
    }

    .up-to-date-chip {
      background: #e8f5e9 !important;
      color: #2e7d32 !important;
    }

    .update-details {
      margin-top: 1rem;
      padding-top: 1rem;
      border-top: 1px solid #eee;
    }

    .release-meta {
      display: flex;
      gap: 1.5rem;
      font-size: 0.875rem;
      color: #666;
      margin-bottom: 0.5rem;
    }

    .release-notes {
      color: #444;
      margin: 0.5rem 0 1rem;
      line-height: 1.5;
    }

    .nucleus-actions {
      display: flex;
      gap: 0.75rem;
    }

    /* Status rows */
    .loading-row, .error-row, .empty-row {
      display: flex;
      align-items: center;
      gap: 0.75rem;
      padding: 0.5rem 0;
    }

    .error-row {
      color: #f44336;
    }

    .empty-row {
      color: #666;
    }

    .loading-container {
      display: flex;
      align-items: center;
      justify-content: center;
      gap: 1rem;
      padding: 2rem;
      color: #666;
    }

    .error-card mat-card-content {
      display: flex;
      align-items: center;
      gap: 0.75rem;
      color: #f44336;
    }

    .empty-state {
      text-align: center;
      padding: 2rem;
      color: #666;

      mat-icon {
        font-size: 3rem;
        width: 3rem;
        height: 3rem;
        color: #bbb;
      }

      p {
        font-size: 1.1rem;
        margin: 0.75rem 0 0.25rem;
        color: #444;
      }

      span {
        font-size: 0.875rem;
      }
    }

    /* Service cards */
    .services-list {
      display: flex;
      flex-direction: column;
      gap: 0.75rem;
    }

    .service-card {
      &.has-update {
        border-left: 4px solid #ff9800;
      }
    }

    .service-row {
      display: flex;
      align-items: center;
      gap: 1.5rem;
      flex-wrap: wrap;
    }

    .service-name {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      min-width: 160px;
      font-weight: 500;

      mat-icon {
        font-size: 20px;
        width: 20px;
        height: 20px;
      }

      .has-update & mat-icon {
        color: #ff9800;
      }

      :not(.has-update) & mat-icon {
        color: #4caf50;
      }
    }

    .service-versions {
      display: flex;
      align-items: center;
      gap: 0.75rem;
      flex: 1;

      .arrow {
        color: #ccc;
        font-size: 18px;
        width: 18px;
        height: 18px;
      }
    }

    .version-col {
      display: flex;
      flex-direction: column;
      gap: 0.125rem;

      .label {
        font-size: 0.65rem;
        color: #999;
        text-transform: uppercase;
      }

      .value {
        font-size: 0.9rem;
        font-weight: 500;
        font-family: 'SF Mono', 'Fira Code', monospace;

        &.new {
          color: #4caf50;
        }
      }
    }

    .service-status {
      min-width: 140px;
    }

    .service-actions {
      display: flex;
      gap: 0.5rem;
      min-width: 160px;
    }

    .changelog {
      margin-top: 0.75rem;
      padding-top: 0.5rem;
      border-top: 1px solid #f5f5f5;
      font-size: 0.875rem;
      color: #666;

      .label {
        font-weight: 500;
        color: #444;
      }
    }

    /* History section */
    .history-list {
      display: flex;
      flex-direction: column;
      gap: 0.5rem;
    }

    .history-card {
      &.success {
        border-left: 4px solid #4caf50;
      }

      &.failed {
        border-left: 4px solid #f44336;
      }
    }

    .history-row {
      display: flex;
      align-items: center;
      gap: 1.5rem;
      flex-wrap: wrap;
    }

    .history-service {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      min-width: 140px;

      mat-icon {
        font-size: 20px;
        width: 20px;
        height: 20px;
      }

      .success & mat-icon {
        color: #4caf50;
      }

      .failed & mat-icon {
        color: #f44336;
      }

      .service-label {
        font-weight: 500;
      }
    }

    .history-images {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      flex: 1;

      code {
        font-size: 0.8rem;
        background: #f5f5f5;
        padding: 2px 6px;
        border-radius: 4px;
        font-family: 'SF Mono', 'Fira Code', monospace;
      }

      .arrow {
        font-size: 16px;
        width: 16px;
        height: 16px;
        color: #ccc;
      }
    }

    .history-badges {
      min-width: 100px;
    }

    .rollback-chip {
      background: #fce4ec !important;
      color: #c62828 !important;
    }

    .history-meta {
      display: flex;
      flex-direction: column;
      gap: 0.125rem;
      min-width: 100px;
      text-align: right;

      .timestamp {
        font-size: 0.75rem;
        color: #999;
      }

      .duration {
        font-size: 0.7rem;
        color: #bbb;
      }
    }
  `]
})
export class UpdatesComponent implements OnInit {
  private tauri = inject(TauriService);
  private notification = inject(NotificationService);

  nucleusInfo: NucleusUpdateInfo | null = null;
  serviceUpdates: ServiceUpdateInfo[] = [];
  updateHistory: UpdateRecord[] = [];

  checking = false;
  nucleusLoading = false;
  servicesLoading = false;
  nucleusDownloading = false;
  nucleusInstalling = false;
  downloadingService: string | null = null;
  updatingService: string | null = null;
  rollingBackService: string | null = null;
  historyLoading = false;

  nucleusError: string | null = null;
  servicesError: string | null = null;

  ngOnInit(): void {
    this.checkAll();
    this.loadHistory();
  }

  async checkAll(): Promise<void> {
    this.checking = true;
    this.nucleusError = null;
    this.servicesError = null;

    const results = await Promise.allSettled([
      this.checkNucleus(),
      this.checkServices()
    ]);

    this.checking = false;

    const updatesAvailable =
      (this.nucleusInfo?.update_available ? 1 : 0) +
      this.serviceUpdates.filter(s => s.update_available).length;

    if (updatesAvailable > 0) {
      this.notification.success(`${updatesAvailable} update(s) available`);
    }
  }

  private async checkNucleus(): Promise<void> {
    this.nucleusLoading = true;
    this.nucleusError = null;
    try {
      this.nucleusInfo = await this.tauri.invokeSilent<NucleusUpdateInfo>('check_nucleus_update');
    } catch (error) {
      this.nucleusError = error as string;
    } finally {
      this.nucleusLoading = false;
    }
  }

  private async checkServices(): Promise<void> {
    this.servicesLoading = true;
    this.servicesError = null;
    try {
      this.serviceUpdates = await this.tauri.invokeSilent<ServiceUpdateInfo[]>('check_service_updates');
    } catch (error) {
      this.servicesError = error as string;
    } finally {
      this.servicesLoading = false;
    }
  }

  async downloadNucleus(): Promise<void> {
    this.nucleusDownloading = true;
    try {
      const result = await this.tauri.invoke<{ success: boolean; file_path: string; size_mb: number }>('download_nucleus_update');
      this.notification.success(`Update downloaded (${result.size_mb.toFixed(1)} MB)`);
    } catch (error) {
      // Error handled by TauriService
    } finally {
      this.nucleusDownloading = false;
    }
  }

  async downloadAndInstallNucleus(): Promise<void> {
    if (!confirm('This will download and install the update. The application will close after installation. Continue?')) {
      return;
    }
    this.nucleusInstalling = true;
    try {
      const msg = await this.tauri.invoke<string>('download_and_install_nucleus_update');
      this.notification.success(msg || 'Update installed. Application will close shortly...');
    } catch (error) {
      // Error handled by TauriService
    } finally {
      this.nucleusInstalling = false;
    }
  }

  async downloadServiceJar(svc: ServiceUpdateInfo): Promise<void> {
    this.downloadingService = svc.service;
    try {
      const result = await this.tauri.invoke<{ success: boolean; file_path: string; size_mb: number }>(
        'download_service_jar',
        { serviceName: svc.service, version: svc.latest_version }
      );
      this.notification.success(`${svc.service} JAR downloaded (${result.size_mb.toFixed(1)} MB)`);
    } catch (error) {
      // Error handled by TauriService
    } finally {
      this.downloadingService = null;
    }
  }

  async updateDockerService(svc: ServiceUpdateInfo): Promise<void> {
    this.updatingService = svc.service;
    try {
      const result = await this.tauri.invoke<DockerUpdateResult>(
        'update_docker_service',
        { serviceName: svc.service, newTag: svc.latest_version }
      );
      if (result.success) {
        this.notification.success(result.message);
      } else {
        this.notification.warning(result.message);
      }
      await this.checkServices();
      await this.loadHistory();
    } catch (error) {
      // Error handled by TauriService
    } finally {
      this.updatingService = null;
    }
  }

  async rollbackDockerService(serviceName: string): Promise<void> {
    this.rollingBackService = serviceName;
    try {
      const result = await this.tauri.invoke<DockerUpdateResult>(
        'rollback_docker_service',
        { serviceName }
      );
      if (result.success) {
        this.notification.success(result.message);
      } else {
        this.notification.warning(result.message);
      }
      await this.checkServices();
      await this.loadHistory();
    } catch (error) {
      // Error handled by TauriService
    } finally {
      this.rollingBackService = null;
    }
  }

  async loadHistory(): Promise<void> {
    this.historyLoading = true;
    try {
      this.updateHistory = await this.tauri.invokeSilent<UpdateRecord[]>('get_update_history');
    } catch (error) {
      // Non-fatal
    } finally {
      this.historyLoading = false;
    }
  }

  shortenImage(image: string): string {
    // gcr.io/puru-255206/puru-xenon:2.3.5 → puru-xenon:2.3.5
    const parts = image.split('/');
    return parts[parts.length - 1] || image;
  }
}
