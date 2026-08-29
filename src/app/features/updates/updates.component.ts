import { Component, OnInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { TauriService, NucleusUpdateInfo, ServiceUpdateInfo, DockerUpdateResult, UpdateRecord } from '../../core/services/tauri.service';
import { NotificationService } from '../../core/services/notification.service';

@Component({
  selector: 'app-updates',
  standalone: true,
  imports: [
    CommonModule
  ],
  template: `
    <div class="updates-page p-4">
      <div class="header flex justify-between items-center">
        <h1>Updates</h1>
        <button class="btn btn-stroked" (click)="checkAll()" [disabled]="checking">
          @if (checking) {
            <span class="spinner" style="width:18px;height:18px;border-width:2px"></span>
          } @else {
            <span class="material-icons">refresh</span>
          }
          Check for Updates
        </button>
      </div>

      <!-- Nucleus Update Card -->
      <div class="card card-pad nucleus-card">
        <div class="card-header">
          <span class="material-icons card-avatar">hub</span>
          <div class="card-titles">
            <div class="card-title">Puru DC</div>
            <div class="card-subtitle">Desktop application</div>
          </div>
        </div>
        <div class="card-content">
          @if (nucleusLoading) {
            <div class="loading-row">
              <span class="spinner" style="width:24px;height:24px;border-width:2px"></span>
              <span>Checking for updates...</span>
            </div>
          } @else if (nucleusError) {
            <div class="error-row">
              <span class="material-icons">error_outline</span>
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
                <span class="chip update-chip">Update Available</span>
              } @else {
                <span class="chip up-to-date-chip">Up to Date</span>
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
                  <button class="btn btn-stroked" (click)="downloadNucleus()" [disabled]="nucleusDownloading || nucleusInstalling">
                    @if (nucleusDownloading) {
                      <span class="spinner" style="width:18px;height:18px;border-width:2px"></span>
                    } @else {
                      <span class="material-icons">download</span>
                    }
                    Download
                  </button>
                  <button class="btn btn-primary" (click)="downloadAndInstallNucleus()" [disabled]="nucleusDownloading || nucleusInstalling">
                    @if (nucleusInstalling) {
                      <span class="spinner" style="width:18px;height:18px;border-width:2px"></span>
                    } @else {
                      <span class="material-icons">system_update_alt</span>
                    }
                    Download &amp; Install
                  </button>
                </div>
              </div>
            }
          } @else {
            <div class="empty-row">
              <span class="material-icons">info_outline</span>
              <span>Click "Check for Updates" to get started</span>
            </div>
          }
        </div>
      </div>

      <!-- Service Updates -->
      <h2>Service Updates</h2>

      @if (servicesLoading) {
        <div class="loading-container">
          <span class="spinner spinner-lg"></span>
          <span>Checking services...</span>
        </div>
      } @else if (servicesError) {
        <div class="card card-pad error-card">
          <div class="card-content">
            <span class="material-icons">error_outline</span>
            <span>{{ servicesError }}</span>
          </div>
        </div>
      } @else if (serviceUpdates.length === 0 && !servicesLoading) {
        <div class="card card-pad empty-state">
          <div class="card-content">
            <span class="material-icons">inventory_2</span>
            <p>No services found</p>
            <span>No running Puru services detected. Make sure Docker is running.</span>
          </div>
        </div>
      } @else {
        <div class="services-list">
          @for (svc of serviceUpdates; track svc.service) {
            <div class="card card-pad service-card" [class.has-update]="svc.update_available">
              <div class="card-content">
                <div class="service-row">
                  <div class="service-name">
                    <span class="material-icons">{{ svc.update_available ? 'upgrade' : 'check_circle' }}</span>
                    <span>{{ svc.service }}</span>
                  </div>
                  <div class="service-versions">
                    <div class="version-col">
                      <span class="label">Current</span>
                      <span class="value">{{ svc.current_version }}</span>
                    </div>
                    <span class="material-icons arrow">arrow_forward</span>
                    <div class="version-col">
                      <span class="label">Latest</span>
                      <span class="value" [class.new]="svc.update_available">{{ svc.latest_version }}</span>
                    </div>
                  </div>
                  <div class="service-status">
                    @if (svc.update_available) {
                      <span class="chip update-chip">Update Available</span>
                    } @else {
                      <span class="chip up-to-date-chip">Current</span>
                    }
                  </div>
                  <div class="service-actions">
                    @if (svc.update_available) {
                      <button class="btn btn-primary"
                              (click)="updateDockerService(svc)"
                              [disabled]="updatingService === svc.service || rollingBackService === svc.service">
                        @if (updatingService === svc.service) {
                          <span class="spinner" style="width:16px;height:16px;border-width:2px"></span>
                        } @else {
                          <span class="material-icons">system_update_alt</span>
                        }
                        Update
                      </button>
                      <button class="btn btn-stroked"
                              (click)="downloadServiceJar(svc)"
                              [disabled]="downloadingService === svc.service">
                        @if (downloadingService === svc.service) {
                          <span class="spinner" style="width:16px;height:16px;border-width:2px"></span>
                        } @else {
                          <span class="material-icons">download</span>
                        }
                        JAR
                      </button>
                    } @else {
                      <button class="btn btn-stroked"
                              (click)="rollbackDockerService(svc.service)"
                              [disabled]="rollingBackService === svc.service || updatingService === svc.service">
                        @if (rollingBackService === svc.service) {
                          <span class="spinner" style="width:16px;height:16px;border-width:2px"></span>
                        } @else {
                          <span class="material-icons">undo</span>
                        }
                        Rollback
                      </button>
                    }
                  </div>
                </div>
                <div class="build-meta">
                  @if (svc.installed_at) {
                    <span class="bm-item" [title]="svc.installed_at">
                      <span class="material-icons">history</span>
                      Installed on this box: <b>{{ relativeTime(svc.installed_at) }}</b>
                      <span class="bm-abs">({{ svc.installed_at | date:'medium' }})</span>
                    </span>
                  }
                  @if (svc.release_date) {
                    <span class="bm-item bm-dim" [title]="svc.release_date">
                      <span class="material-icons">cloud</span>
                      Latest cloud build: {{ svc.release_date | date:'medium' }}
                    </span>
                  }
                </div>
                @if (svc.update_available && svc.changelog) {
                  <div class="changelog">
                    <span class="label">Changes:</span> {{ svc.changelog }}
                  </div>
                }
              </div>
            </div>
          }
        </div>
      }

      <!-- Update History -->
      <h2>Update History</h2>

      @if (historyLoading) {
        <div class="loading-container">
          <span class="spinner" style="width:24px;height:24px;border-width:2px"></span>
          <span>Loading history...</span>
        </div>
      } @else if (updateHistory.length === 0) {
        <div class="card card-pad empty-state">
          <div class="card-content">
            <span class="material-icons">history</span>
            <p>No update history</p>
            <span>Docker update records will appear here after updates are performed.</span>
          </div>
        </div>
      } @else {
        <div class="history-list">
          @for (record of updateHistory; track record.timestamp) {
            <div class="card card-pad history-card" [class.success]="record.success && !record.rolled_back" [class.failed]="!record.success || record.rolled_back">
              <div class="card-content">
                <div class="history-row">
                  <div class="history-service">
                    <span class="material-icons">{{ record.success && !record.rolled_back ? 'check_circle' : 'error' }}</span>
                    <span class="service-label">{{ record.service }}</span>
                  </div>
                  <div class="history-images">
                    <code>{{ shortenImage(record.previous_image) }}</code>
                    <span class="material-icons arrow">arrow_forward</span>
                    <code>{{ shortenImage(record.new_image) }}</code>
                  </div>
                  <div class="history-badges">
                    @if (record.rolled_back) {
                      <span class="chip rollback-chip">Rolled Back</span>
                    } @else if (record.success) {
                      <span class="chip up-to-date-chip">Success</span>
                    } @else {
                      <span class="chip update-chip">Failed</span>
                    }
                  </div>
                  <div class="history-meta">
                    <span class="timestamp">{{ record.timestamp | date:'short' }}</span>
                    <span class="duration">{{ record.duration_seconds }}s</span>
                  </div>
                </div>
              </div>
            </div>
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
    .card-header {
      display: flex;
      align-items: center;
      gap: 0.75rem;
      margin-bottom: 1rem;
    }

    .card-avatar {
      background: linear-gradient(135deg, #6366f1, #8b5cf6);
      padding: 8px;
      border-radius: 50%;
      color: white;
    }

    .card-titles {
      display: flex;
      flex-direction: column;
    }

    .card-title {
      font-size: 1.1rem;
      font-weight: 600;
      color: #333;
    }

    .card-subtitle {
      font-size: 0.8rem;
      color: #666;
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

    .error-card .card-content {
      display: flex;
      align-items: center;
      gap: 0.75rem;
      color: #f44336;
    }

    .empty-state {
      text-align: center;
      padding: 2rem;
      color: #666;

      .material-icons {
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

      .material-icons {
        font-size: 20px;
        width: 20px;
        height: 20px;
      }

      .has-update & .material-icons {
        color: #ff9800;
      }

      :not(.has-update) & .material-icons {
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

    .build-meta {
      margin-top: 0.6rem;
      padding-top: 0.5rem;
      border-top: 1px solid #f5f5f5;
      display: flex;
      flex-wrap: wrap;
      gap: 6px 16px;
      font-size: 0.8rem;
      color: #555;
    }
    .bm-item {
      display: inline-flex;
      align-items: center;
      gap: 6px;
    }
    .bm-item .material-icons {
      font-size: 15px;
      color: #666;
    }
    .bm-item b { color: #111; font-weight: 600; }
    .bm-abs { color: #999; font-size: 0.72rem; }
    .bm-dim { color: #999; }
    .bm-dim .material-icons { color: #999; }

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

      .material-icons {
        font-size: 20px;
        width: 20px;
        height: 20px;
      }

      .success & .material-icons {
        color: #4caf50;
      }

      .failed & .material-icons {
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

  isNative = false;

  /** "5 min ago" / "2 days ago" from an ISO timestamp. Handles the "just now"
   *  / future-date edge cases so an install that happened seconds ago doesn't
   *  render as "in 0 sec". */
  relativeTime(iso?: string | null): string {
    if (!iso) return '';
    const then = new Date(iso).getTime();
    if (isNaN(then)) return '';
    const diffMs = Date.now() - then;
    if (diffMs < 60_000) return 'just now';
    const mins = Math.floor(diffMs / 60_000);
    if (mins < 60) return `${mins} min ago`;
    const hours = Math.floor(mins / 60);
    if (hours < 24) return `${hours} hour${hours === 1 ? '' : 's'} ago`;
    const days = Math.floor(hours / 24);
    if (days < 30) return `${days} day${days === 1 ? '' : 's'} ago`;
    const months = Math.floor(days / 30);
    if (months < 12) return `${months} month${months === 1 ? '' : 's'} ago`;
    const years = Math.floor(days / 365);
    return `${years} year${years === 1 ? '' : 's'} ago`;
  }

  ngOnInit(): void {
    this.detectMode();
    this.checkAll();
    this.loadHistory();
  }

  private async detectMode(): Promise<void> {
    try {
      const mode = await this.tauri.invoke<string>('get_deployment_mode');
      this.isNative = mode === 'Native';
    } catch {
      this.isNative = false;
    }
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
      // Native mode dispatch. Docker mode uses `update_docker_service` (pull
      // new tag + recreate container). Native mode has no docker tag —
      // hydrogen/dviewer just re-download the tarball via `pull_single_jar`
      // (which routes through `pull_hydrogen_inner` / `pull_dviewer_inner`),
      // and JAR services use `update_native_service` (manifest-driven).
      if (this.isNative) {
        if (svc.service === 'puru-hydrogen' || svc.service === 'dviewer') {
          const pull = await this.tauri.invoke<{ success: boolean; short_sha: string; size_mb: number; message: string }>(
            'pull_single_jar',
            { serviceName: svc.service }
          );
          if (pull.success) {
            this.notification.success(`${svc.service} updated to ${pull.short_sha} (${pull.size_mb.toFixed(1)} MB)`);
          } else {
            this.notification.warning(pull.message || `Failed to update ${svc.service}`);
          }
        } else {
          const result = await this.tauri.invoke<{ success: boolean; message: string }>(
            'update_native_service',
            { serviceName: svc.service }
          );
          if (result.success) {
            this.notification.success(result.message);
          } else {
            this.notification.warning(result.message);
          }
        }
        await this.checkServices();
        await this.loadHistory();
        return;
      }

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
