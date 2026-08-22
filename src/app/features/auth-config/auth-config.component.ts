import { Component, OnInit, computed, inject, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { TauriService } from '../../core/services/tauri.service';
import { NotificationService } from '../../core/services/notification.service';

interface AuthConfigEntry {
  id?: number;
  configKey: string;
  configValue?: string | null;
  configType?: 'STRING' | 'NUMBER' | 'BOOLEAN' | string | null;
  category?: string | null;
  description?: string | null;
}

@Component({
  selector: 'app-auth-config',
  standalone: true,
  imports: [CommonModule, FormsModule],
  template: `
    <div class="page p-4">
      <div class="header">
        <div>
          <h1>Auth Config</h1>
          <p class="sub">
            Direct view/edit of <code>puru_auth.puru_config</code> — the single
            source of truth every backend service reads at boot. Changes hit
            puru-auth's API instantly, but each service still needs a restart
            to re-read the values it cached at boot.
          </p>
        </div>
        <div class="actions">
          <button class="btn btn-stroked" (click)="load()" [disabled]="loading()">
            <span class="material-icons">refresh</span>
            Reload
          </button>
          <button class="btn btn-stroked" (click)="refreshAuthCache()" [disabled]="loading()"
                  title="Clear auth's cached frontend config so hydrogen sees the change">
            <span class="material-icons">restart_alt</span>
            Refresh cache
          </button>
          <button class="btn btn-primary" (click)="startCreate()" [disabled]="loading()">
            <span class="material-icons">add</span>
            New key
          </button>
        </div>
      </div>

      <div class="filter-row">
        <input
          type="text"
          class="input"
          placeholder="Filter by key, value, category…"
          [(ngModel)]="filter"
          (ngModelChange)="filterSignal.set($event)">
        <span class="count">{{ visible().length }} / {{ rows().length }} rows</span>
      </div>

      @if (loading()) {
        <div class="empty"><span class="spinner spinner-lg"></span></div>
      } @else if (error()) {
        <div class="card card-pad err-card">
          <span class="material-icons">error</span>
          <div>
            <div class="err-title">Could not load config</div>
            <div class="err-msg">{{ error() }}</div>
          </div>
        </div>
      } @else if (visible().length === 0) {
        <div class="card card-pad empty-state">
          <span class="material-icons">tune</span>
          <p>No matching config entries</p>
        </div>
      } @else {
        <div class="table-wrap card">
          <table class="cfg-table">
            <thead>
              <tr>
                <th class="key-col">Key</th>
                <th class="val-col">Value</th>
                <th class="type-col">Type</th>
                <th class="cat-col">Category</th>
                <th class="act-col"></th>
              </tr>
            </thead>
            <tbody>
              @for (row of visible(); track row.configKey) {
                <tr [class.editing]="editing()?.configKey === row.configKey">
                  <td class="key-col mono">{{ row.configKey }}</td>
                  <td class="val-col">
                    @if (editing()?.configKey === row.configKey) {
                      <input class="input mono" [(ngModel)]="editValue" />
                    } @else {
                      <span class="mono val-cell" [title]="row.configValue || ''">{{ row.configValue || '—' }}</span>
                    }
                  </td>
                  <td class="type-col">
                    @if (editing()?.configKey === row.configKey) {
                      <select class="input" [(ngModel)]="editType">
                        <option value="STRING">STRING</option>
                        <option value="NUMBER">NUMBER</option>
                        <option value="BOOLEAN">BOOLEAN</option>
                      </select>
                    } @else {
                      <span class="chip">{{ row.configType || 'STRING' }}</span>
                    }
                  </td>
                  <td class="cat-col">{{ row.category || 'shared' }}</td>
                  <td class="act-col">
                    @if (editing()?.configKey === row.configKey) {
                      <button class="btn btn-primary btn-sm" (click)="save()" [disabled]="saving()">
                        <span class="material-icons">check</span> Save
                      </button>
                      <button class="btn btn-stroked btn-sm" (click)="cancel()" [disabled]="saving()">
                        <span class="material-icons">close</span>
                      </button>
                    } @else {
                      <button class="btn-icon" title="Edit" (click)="startEdit(row)">
                        <span class="material-icons">edit</span>
                      </button>
                      <button class="btn-icon danger" title="Delete" (click)="remove(row)">
                        <span class="material-icons">delete</span>
                      </button>
                    }
                  </td>
                </tr>
              }
            </tbody>
          </table>
        </div>
      }

      @if (creating()) {
        <div class="modal-backdrop" (click)="cancelCreate()">
          <div class="modal card card-pad" (click)="$event.stopPropagation()">
            <div class="modal-title">Create config entry</div>
            <label>Key</label>
            <input class="input mono" [(ngModel)]="newKey" placeholder="e.g. feature.myFlag" />
            <label>Value</label>
            <input class="input mono" [(ngModel)]="newValue" placeholder="value" />
            <div class="grid-2">
              <div>
                <label>Type</label>
                <select class="input" [(ngModel)]="newType">
                  <option value="STRING">STRING</option>
                  <option value="NUMBER">NUMBER</option>
                  <option value="BOOLEAN">BOOLEAN</option>
                </select>
              </div>
              <div>
                <label>Category</label>
                <select class="input" [(ngModel)]="newCategory">
                  <option value="shared">shared</option>
                  <option value="frontend">frontend</option>
                  <option value="backend">backend</option>
                </select>
              </div>
            </div>
            <div class="modal-actions">
              <button class="btn btn-stroked" (click)="cancelCreate()">Cancel</button>
              <button class="btn btn-primary" (click)="createEntry()" [disabled]="saving() || !newKey.trim()">
                Create
              </button>
            </div>
          </div>
        </div>
      }
    </div>
  `,
  styles: [`
    .page { max-width: 1200px; margin: 0 auto; }
    .header { display: flex; justify-content: space-between; align-items: flex-start; gap: 20px; margin-bottom: 20px; }
    h1 { margin: 0 0 4px 0; font-size: 1.5rem; }
    .sub { color: var(--text-secondary); font-size: 0.85rem; margin: 0; max-width: 720px; }
    .sub code { background: var(--bg-hover, #f4f7fb); padding: 1px 6px; border-radius: 4px; font-size: 0.85em; }
    .actions { display: flex; gap: 8px; flex-shrink: 0; }

    .filter-row { display: flex; align-items: center; gap: 12px; margin-bottom: 12px; }
    .filter-row .input { flex: 1; max-width: 360px; }
    .filter-row .count { color: var(--text-muted); font-size: 0.8rem; }

    .table-wrap { overflow-x: auto; padding: 0; }
    .cfg-table { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
    .cfg-table th { text-align: left; padding: 10px 14px; background: var(--bg-hover, #f8fafc); border-bottom: 1px solid var(--border); color: var(--text-secondary); font-weight: 600; }
    .cfg-table td { padding: 8px 14px; border-bottom: 1px solid var(--border-light, #eef2f7); vertical-align: middle; }
    .cfg-table tr:last-child td { border-bottom: none; }
    .cfg-table tr.editing { background: #fffbeb; }
    .mono { font-family: 'JetBrains Mono', 'SF Mono', Menlo, monospace; }
    .val-cell { display: inline-block; max-width: 380px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .chip { background: var(--bg-hover, #f4f7fb); padding: 2px 8px; border-radius: 999px; font-size: 11px; font-weight: 600; color: var(--text-secondary); }

    .key-col { min-width: 240px; }
    .val-col { min-width: 320px; }
    .type-col { width: 110px; }
    .cat-col { width: 100px; color: var(--text-secondary); }
    .act-col { width: 160px; text-align: right; white-space: nowrap; }
    .act-col .btn-sm { margin-left: 4px; }
    .btn-icon { background: none; border: none; cursor: pointer; padding: 4px; color: var(--text-secondary); border-radius: 6px; }
    .btn-icon:hover { background: var(--bg-hover, #f4f7fb); color: var(--text-primary); }
    .btn-icon.danger:hover { color: #dc2626; background: #fef2f2; }
    .btn-icon .material-icons { font-size: 18px; }

    .empty { display: flex; justify-content: center; padding: 60px; }
    .empty-state { text-align: center; color: var(--text-muted); padding: 40px; }
    .empty-state .material-icons { font-size: 40px; opacity: 0.4; display: block; margin: 0 auto 10px; }

    .err-card { background: #fef2f2; border-color: #fecaca; display: flex; gap: 12px; align-items: flex-start; }
    .err-card .material-icons { color: #dc2626; font-size: 22px; }
    .err-title { font-weight: 600; color: #991b1b; margin-bottom: 4px; }
    .err-msg { color: #991b1b; font-size: 0.85rem; }

    .modal-backdrop { position: fixed; inset: 0; background: rgba(15, 23, 42, 0.45); display: flex; align-items: center; justify-content: center; z-index: 100; }
    .modal { width: min(520px, 92vw); display: flex; flex-direction: column; gap: 10px; }
    .modal-title { font-weight: 700; font-size: 1.05rem; margin-bottom: 6px; }
    .modal label { font-size: 0.8rem; color: var(--text-secondary); font-weight: 500; }
    .modal .input { width: 100%; }
    .grid-2 { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
    .modal-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 10px; }
  `]
})
export class AuthConfigComponent implements OnInit {
  private tauri = inject(TauriService);
  private notification = inject(NotificationService);

  loading = signal(false);
  saving = signal(false);
  error = signal<string | null>(null);
  rows = signal<AuthConfigEntry[]>([]);

  filter = '';
  filterSignal = signal('');
  visible = computed(() => {
    const q = this.filterSignal().trim().toLowerCase();
    if (!q) return this.rows();
    return this.rows().filter(r =>
      r.configKey.toLowerCase().includes(q)
      || (r.configValue ?? '').toLowerCase().includes(q)
      || (r.category ?? '').toLowerCase().includes(q)
    );
  });

  editing = signal<AuthConfigEntry | null>(null);
  editValue = '';
  editType: 'STRING' | 'NUMBER' | 'BOOLEAN' = 'STRING';

  creating = signal(false);
  newKey = '';
  newValue = '';
  newType: 'STRING' | 'NUMBER' | 'BOOLEAN' = 'STRING';
  newCategory: 'shared' | 'frontend' | 'backend' = 'shared';

  ngOnInit(): void {
    this.load();
  }

  async load(): Promise<void> {
    this.loading.set(true);
    this.error.set(null);
    try {
      const entries = await this.tauri.invoke<AuthConfigEntry[]>('auth_config_list');
      // Sort by key alphabetically so operators can scan predictably.
      entries.sort((a, b) => a.configKey.localeCompare(b.configKey));
      this.rows.set(entries);
    } catch (err: any) {
      this.error.set(String(err?.message || err || 'Failed to load'));
      this.rows.set([]);
    } finally {
      this.loading.set(false);
    }
  }

  startEdit(row: AuthConfigEntry): void {
    this.editing.set(row);
    this.editValue = row.configValue ?? '';
    const t = (row.configType || 'STRING').toUpperCase();
    this.editType = t === 'NUMBER' || t === 'BOOLEAN' ? (t as any) : 'STRING';
  }

  cancel(): void {
    this.editing.set(null);
  }

  async save(): Promise<void> {
    const target = this.editing();
    if (!target) return;
    this.saving.set(true);
    try {
      await this.tauri.invoke('auth_config_update', {
        key: target.configKey,
        value: this.editValue,
        configType: this.editType,
        category: target.category ?? undefined,
      });
      // Optimistic update in place so the operator sees the new value without
      // waiting for a reload; a background reload catches any server rewrites.
      this.rows.update(list => list.map(r =>
        r.configKey === target.configKey
          ? { ...r, configValue: this.editValue, configType: this.editType }
          : r
      ));
      this.notification.success(`${target.configKey} saved`);
      this.editing.set(null);
    } catch (err: any) {
      this.notification.error(String(err?.message || err || 'Save failed'));
    } finally {
      this.saving.set(false);
    }
  }

  async remove(row: AuthConfigEntry): Promise<void> {
    if (!confirm(`Delete "${row.configKey}"? Services that expect this key at boot may fail to start.`)) {
      return;
    }
    try {
      await this.tauri.invoke('auth_config_delete', { key: row.configKey });
      this.rows.update(list => list.filter(r => r.configKey !== row.configKey));
      this.notification.success(`${row.configKey} deleted`);
    } catch (err: any) {
      this.notification.error(String(err?.message || err || 'Delete failed'));
    }
  }

  startCreate(): void {
    this.newKey = '';
    this.newValue = '';
    this.newType = 'STRING';
    this.newCategory = 'shared';
    this.creating.set(true);
  }

  cancelCreate(): void {
    this.creating.set(false);
  }

  async createEntry(): Promise<void> {
    const key = this.newKey.trim();
    if (!key) return;
    this.saving.set(true);
    try {
      // The nucleus/PUT endpoint upserts, so we use it for create too — one
      // less round-trip than POST + GET, and it fits how the auth service
      // already behaves for missing keys.
      await this.tauri.invoke('auth_config_update', {
        key,
        value: this.newValue,
        configType: this.newType,
        category: this.newCategory,
      });
      this.notification.success(`${key} created`);
      this.creating.set(false);
      await this.load();
    } catch (err: any) {
      this.notification.error(String(err?.message || err || 'Create failed'));
    } finally {
      this.saving.set(false);
    }
  }

  async refreshAuthCache(): Promise<void> {
    try {
      await this.tauri.invoke('auth_config_refresh');
      this.notification.success('Auth cache refreshed');
    } catch (err: any) {
      this.notification.error(String(err?.message || err || 'Refresh failed'));
    }
  }
}
