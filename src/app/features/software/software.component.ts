import { Component, OnDestroy, OnInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import {
  InstallProgress,
  InstallResult,
  ManualDownloadProgress,
  ManualDownloadResult,
  TauriService,
} from '../../core/services/tauri.service';
import { NotificationService } from '../../core/services/notification.service';

interface SoftwareItem {
  /** Manifest component id — matches oxygen's INFRA_COMPONENTS. */
  id: string;
  /** Display name — also matches the `software` field on InstallProgress events. */
  label: string;
  description: string;
  icon: string;
  /** True when this is a fix for a known error (surfaced prominently). */
  troubleshoot?: boolean;
}

@Component({
  selector: 'app-software',
  standalone: true,
  imports: [CommonModule],
  template: `
    <div class="page p-4">
      <div class="header">
        <div>
          <h1>Software</h1>
          <p class="sub">
            Optional helpers and one-click fixes for the hospital box. Files come
            from the same cloud bucket that hosts the main installers.
          </p>
        </div>
      </div>

      <div class="grid">
        @for (item of items; track item.id) {
          <div class="card card-pad soft-card" [class.warn]="item.troubleshoot">
            <div class="soft-head">
              <span class="material-icons">{{ item.icon }}</span>
              <div class="soft-title">
                <div class="name">{{ item.label }}</div>
                @if (item.troubleshoot) {
                  <span class="chip warn-chip">Fix for MySQL error 1603</span>
                }
              </div>
            </div>
            <p class="desc">{{ item.description }}</p>

            @if (installing === item.id) {
              <div class="progress">
                <div class="bar"><div class="fill" [style.width.%]="installProgress?.percent || 0"></div></div>
                <div class="msg">
                  {{ installProgress?.stage || 'starting' }} — {{ installProgress?.message }}
                </div>
              </div>
            }

            @if (downloading === item.id) {
              <div class="progress">
                <div class="bar"><div class="fill" [style.width.%]="downloadProgress?.percent || 0"></div></div>
                <div class="msg">
                  Downloading {{ downloadProgress?.file }}…
                  @if (downloadProgress?.stage === 'completed') {
                    <span class="ok"> — saved to {{ downloadProgress?.path }}</span>
                  }
                </div>
              </div>
            }

            @if (lastResult[item.id]) {
              <div class="result" [class.ok]="lastResult[item.id]!.success" [class.err]="!lastResult[item.id]!.success">
                <span class="material-icons">
                  {{ lastResult[item.id]!.success ? 'check_circle' : 'error' }}
                </span>
                <span>
                  {{ lastResult[item.id]!.success ? ('Installed' + (lastResult[item.id]!.version ? ' — v' + lastResult[item.id]!.version : '')) : lastResult[item.id]!.error }}
                </span>
              </div>
            }

            <div class="actions">
              <button
                class="btn btn-primary"
                [disabled]="isBusy()"
                (click)="install(item)">
                <span class="material-icons">download</span>
                Install
              </button>
              <button
                class="btn btn-stroked"
                [disabled]="isBusy()"
                (click)="downloadToDownloads(item)"
                title="Save the installer to your Downloads folder and run it by hand">
                <span class="material-icons">folder_open</span>
                Download only
              </button>
            </div>
          </div>
        }
      </div>
    </div>
  `,
  styles: [`
    .page { max-width: 1100px; margin: 0 auto; }
    .header { display: flex; justify-content: space-between; align-items: flex-start; margin-bottom: 20px; }
    h1 { margin: 0 0 4px 0; font-size: 1.5rem; }
    .sub { color: var(--text-secondary); font-size: 0.875rem; margin: 0; max-width: 640px; }

    .grid {
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(340px, 1fr));
      gap: 16px;
    }
    .soft-card { display: flex; flex-direction: column; gap: 12px; }
    .soft-card.warn { border-color: #fed7aa; background: #fffaf5; }

    .soft-head { display: flex; align-items: flex-start; gap: 12px; }
    .soft-head .material-icons { font-size: 28px; color: var(--brand-blue, #009efb); }
    .soft-card.warn .soft-head .material-icons { color: #ea580c; }
    .soft-title .name { font-weight: 600; font-size: 1.05rem; color: var(--text-primary); }
    .chip { display: inline-block; margin-top: 4px; padding: 2px 8px; border-radius: 999px; font-size: 11px; font-weight: 600; }
    .warn-chip { background: #fed7aa; color: #9a3412; }

    .desc { color: var(--text-secondary); font-size: 0.85rem; margin: 0; }

    .progress { display: flex; flex-direction: column; gap: 6px; }
    .bar { height: 6px; border-radius: 3px; background: var(--border-light, #e2e8f0); overflow: hidden; }
    .fill { height: 100%; background: var(--brand-blue, #009efb); transition: width 0.2s ease; }
    .msg { font-size: 0.8rem; color: var(--text-secondary); }
    .msg .ok { color: #059669; }

    .result { display: flex; align-items: center; gap: 8px; padding: 8px 10px; border-radius: 8px; font-size: 0.85rem; }
    .result.ok { background: #ecfdf5; color: #065f46; }
    .result.err { background: #fef2f2; color: #991b1b; }
    .result .material-icons { font-size: 18px; }

    .actions { display: flex; gap: 8px; margin-top: auto; }
    .actions .btn { flex: 1; justify-content: center; }
  `]
})
export class SoftwareComponent implements OnInit, OnDestroy {
  private tauri = inject(TauriService);
  private notification = inject(NotificationService);

  items: SoftwareItem[] = [
    {
      id: 'vc-redist',
      label: 'VC++ Redistributable (x64)',
      icon: 'build_circle',
      description:
        'Microsoft Visual C++ 2015-2022 x64 runtime. Install this before ' +
        'MySQL if you hit "installer failed with error 1603" — MySQL\'s MSI ' +
        'links against VCRUNTIME140.dll and fails opaquely without it.',
      troubleshoot: true,
    },
    {
      id: 'mysql-workbench',
      label: 'MySQL Workbench',
      icon: 'storage',
      description:
        'Official MySQL GUI client for schema browsing, ad-hoc queries and ' +
        'DBA support tasks. Optional — nothing on the box depends on it.',
    },
  ];

  /** Currently-running install (matches `items[].id`), null if idle. */
  installing: string | null = null;
  installProgress: InstallProgress | null = null;

  /** Currently-running download-to-Downloads (matches `items[].id`), null if idle. */
  downloading: string | null = null;
  downloadProgress: ManualDownloadProgress | null = null;

  /** Last install result per item id — shown until the next attempt. */
  lastResult: Record<string, InstallResult | null> = {};

  private unlistenInstall: UnlistenFn | null = null;
  private unlistenDownload: UnlistenFn | null = null;

  isBusy(): boolean {
    return this.installing !== null || this.downloading !== null;
  }

  async ngOnInit(): Promise<void> {
    this.unlistenInstall = await listen<InstallProgress>(
      'install-progress',
      (event) => {
        // Only track events for whichever install we started from this page.
        const p = event.payload;
        const active = this.items.find(i => i.id === this.installing);
        if (active && p.software && p.software.toLowerCase().includes(active.label.split(' ')[0].toLowerCase())) {
          this.installProgress = p;
        } else if (this.installing) {
          this.installProgress = p;
        }
      }
    );
    this.unlistenDownload = await listen<ManualDownloadProgress>(
      'manual-infra-download-progress',
      (event) => {
        this.downloadProgress = event.payload;
      }
    );
  }

  ngOnDestroy(): void {
    this.unlistenInstall?.();
    this.unlistenDownload?.();
  }

  async install(item: SoftwareItem): Promise<void> {
    if (this.isBusy()) return;

    // Match setup.component.ts: silent installs write to Program Files and
    // register services — they need elevation. Prompt for it up front.
    try {
      const elevated = await this.tauri.invoke<boolean>('is_elevated');
      if (!elevated) {
        this.notification.warning('Installing needs administrator — restarting as admin…');
        await this.tauri.invoke('restart_as_admin');
        return;
      }
    } catch {
      // Couldn't determine elevation — fall through; the backend gate catches it.
    }

    this.installing = item.id;
    this.installProgress = null;
    this.lastResult[item.id] = null;

    try {
      const results = await this.tauri.invoke<InstallResult[]>('install_prerequisites', {
        software: [item.id],
      });
      // Pick the result that matches this component — install_prerequisites
      // can return several (e.g. VC redist pulled in as a MySQL dep).
      const primary = results.find(r =>
        r.software.toLowerCase().includes(item.label.split(' ')[0].toLowerCase())
      ) || results[results.length - 1] || null;
      this.lastResult[item.id] = primary;
      if (primary?.success) {
        this.notification.success(`${item.label} installed`);
      } else {
        this.notification.error(primary?.error || `${item.label} install failed`);
      }
    } catch (err: any) {
      const msg = String(err?.message || err || '');
      if (msg.includes('ELEVATION_REQUIRED')) {
        this.notification.warning('Administrator required — restarting as admin…');
        try { await this.tauri.invoke('restart_as_admin'); } catch {}
      } else {
        this.notification.error(msg || `${item.label} install failed`);
      }
      this.lastResult[item.id] = {
        software: item.label,
        success: false,
        error: msg,
      };
    } finally {
      this.installing = null;
    }
  }

  async downloadToDownloads(item: SoftwareItem): Promise<void> {
    if (this.isBusy()) return;
    this.downloading = item.id;
    this.downloadProgress = null;

    try {
      const results = await this.tauri.invoke<ManualDownloadResult[]>(
        'download_prerequisites_to_downloads',
        { software: [item.id] }
      );
      const primary = results[0];
      if (primary?.success) {
        this.notification.success(`Saved to ${primary.path}`);
      } else {
        this.notification.error(primary?.error || 'Download failed');
      }
    } catch (err: any) {
      this.notification.error(String(err?.message || err || 'Download failed'));
    } finally {
      this.downloading = null;
    }
  }
}
