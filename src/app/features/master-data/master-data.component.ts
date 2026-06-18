import { Component, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { TauriService, SeedReport } from '../../core/services/tauri.service';
import { NotificationService } from '../../core/services/notification.service';

@Component({
  selector: 'app-master-data',
  standalone: true,
  imports: [CommonModule],
  template: `
    <div class="master-data-page p-4">
      <div class="header">
        <h1>Master Data</h1>
        <p class="subtitle">
          User-triggered seeding of curated catalogues. Never runs as part of
          first-install setup. Existing rows are never overwritten — duplicate
          (name, s_class, type) tuples are skipped.
        </p>
      </div>

      <!-- Radiology Services -->
      <div class="card card-pad">
        <div class="card-header">
          <span class="material-icons card-avatar">radiology</span>
          <div class="card-titles">
            <div class="card-title">Radiology Services</div>
            <div class="card-subtitle">
              CT Scan / MRI / X Ray / Sonography USG / Echo — normalised from
              real-world production data
            </div>
          </div>
        </div>

        <div class="card-content">
          @if (loading) {
            <div class="loading-row">
              <span class="spinner"></span>
              <span>Seeding radiology services…</span>
            </div>
          } @else if (report) {
            <div class="result-block">
              @for (section of report.sections; track section.name) {
                <div class="section-row">
                  <div class="section-name">{{ section.name }}</div>
                  <div class="counts">
                    <span class="chip created">{{ section.created }} created</span>
                    <span class="chip skipped">{{ section.skipped }} skipped</span>
                    @if (section.errors.length > 0) {
                      <span class="chip errors">{{ section.errors.length }} errors</span>
                    }
                  </div>
                </div>
                @if (section.errors.length > 0) {
                  <ul class="error-list">
                    @for (e of section.errors; track e) {
                      <li>{{ e }}</li>
                    }
                  </ul>
                }
              }
            </div>
          } @else {
            <p class="hint">
              Seeds the canonical radiology service rows into
              <code>puru_has.service</code> with <code>type = RADIO (2)</code>.
              Safe to run repeatedly.
            </p>
          }

          <div class="actions">
            <button
              class="btn btn-primary"
              (click)="seedRadiology()"
              [disabled]="loading">
              @if (loading) {
                <span class="spinner sm"></span>
              } @else {
                <span class="material-icons">cloud_upload</span>
              }
              Seed Radiology Services
            </button>
          </div>
        </div>
      </div>
    </div>
  `,
  styles: [`
    .master-data-page { display: flex; flex-direction: column; gap: 20px; }
    .header h1 { margin: 0 0 4px; }
    .subtitle { color: var(--mat-sys-on-surface-variant, #666); margin: 0; max-width: 720px; }
    .card { background: var(--mat-sys-surface-container-low, #fafafa); border-radius: 12px; }
    .card-pad { padding: 20px; }
    .card-header { display: flex; align-items: center; gap: 12px; margin-bottom: 16px; }
    .card-avatar { font-size: 28px; color: var(--mat-sys-primary, #1976d2); }
    .card-title { font-weight: 600; font-size: 16px; }
    .card-subtitle { font-size: 13px; color: var(--mat-sys-on-surface-variant, #666); }
    .card-content { display: flex; flex-direction: column; gap: 16px; }
    .hint { margin: 0; color: var(--mat-sys-on-surface-variant, #666); }
    .hint code { background: rgba(0,0,0,0.06); padding: 1px 6px; border-radius: 4px; font-size: 12px; }
    .actions { display: flex; gap: 8px; }
    .btn { display: inline-flex; align-items: center; gap: 6px; padding: 8px 16px; border-radius: 8px; cursor: pointer; font-weight: 500; border: 1px solid transparent; }
    .btn-primary { background: var(--mat-sys-primary, #1976d2); color: white; }
    .btn-primary:disabled { opacity: 0.6; cursor: not-allowed; }
    .spinner { width: 18px; height: 18px; border: 2px solid currentColor; border-top-color: transparent; border-radius: 50%; animation: spin 1s linear infinite; display: inline-block; }
    .spinner.sm { width: 16px; height: 16px; border-width: 2px; }
    @keyframes spin { to { transform: rotate(360deg); } }
    .loading-row { display: flex; align-items: center; gap: 12px; padding: 12px 0; }
    .result-block { display: flex; flex-direction: column; gap: 8px; }
    .section-row { display: flex; justify-content: space-between; align-items: center; padding: 8px 12px; background: rgba(0,0,0,0.03); border-radius: 8px; }
    .section-name { font-weight: 500; font-size: 14px; }
    .counts { display: flex; gap: 6px; }
    .chip { padding: 2px 10px; border-radius: 12px; font-size: 12px; font-weight: 500; }
    .chip.created { background: #c8e6c9; color: #1b5e20; }
    .chip.skipped { background: #e0e0e0; color: #424242; }
    .chip.errors { background: #ffcdd2; color: #b71c1c; }
    .error-list { margin: 4px 0 8px 16px; color: #b71c1c; font-size: 12px; }
  `]
})
export class MasterDataComponent {
  private tauri = inject(TauriService);
  private notify = inject(NotificationService);

  loading = false;
  report: SeedReport | null = null;

  async seedRadiology(): Promise<void> {
    this.loading = true;
    this.report = null;
    try {
      this.report = await this.tauri.invoke<SeedReport>('seed_master_data', {
        radiology: true,
      });
      const created = this.report.sections.reduce((s, x) => s + x.created, 0);
      const skipped = this.report.sections.reduce((s, x) => s + x.skipped, 0);
      const errors = this.report.sections.reduce((s, x) => s + x.errors.length, 0);
      if (errors > 0) {
        this.notify.error(`Seeded with ${errors} error(s) — see details below.`);
      } else {
        this.notify.success(`Radiology services seeded: ${created} created, ${skipped} skipped.`);
      }
    } catch (e: any) {
      this.notify.error(typeof e === 'string' ? e : 'Master-data seed failed.');
    } finally {
      this.loading = false;
    }
  }
}
