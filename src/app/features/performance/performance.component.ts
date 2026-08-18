import { Component, OnInit, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import {
  TauriService,
  MemoryPlan,
  PerformanceConfig,
  ServicePlan,
  ReserveConfig,
} from '../../core/services/tauri.service';
import { NotificationService } from '../../core/services/notification.service';

/**
 * Performance — the JVM memory budget for native-mode services.
 *
 * Nucleus runs each service as a bare `java -jar` child process, and an untuned
 * JVM sizes its own max heap at 25% of the box's RAM. A dozen of them each
 * believing they may take a quarter of the machine is what produces the slow
 * creep into swap that surfaces as an unexplained low-memory alert.
 *
 * This screen shows the budget: what is reserved for the OS, MySQL, RabbitMQ and
 * Nucleus, what is left for the JVMs, and how that remainder is split. Nucleus
 * computes the whole thing from the box's RAM; editing any value freezes the
 * computed plan into explicit numbers so nothing stays implicit.
 */
@Component({
  selector: 'app-performance',
  standalone: true,
  imports: [CommonModule, FormsModule],
  template: `
    <div class="performance-page p-4">
      <div class="header flex justify-between items-center">
        <div>
          <h1>Performance</h1>
          <p class="subtitle">JVM memory budget for native services</p>
        </div>
        <div class="actions">
          @if (editing) {
            <button class="btn btn-primary" (click)="save()" [disabled]="saving">
              <span class="material-icons">save</span> Save
            </button>
            <button class="btn btn-stroked" (click)="cancelEdit()" [disabled]="saving">Cancel</button>
          } @else {
            <button class="btn btn-stroked" (click)="load()" [disabled]="loading">
              <span class="material-icons">refresh</span> Refresh
            </button>
            @if (plan && !plan.auto_tune) {
              <button class="btn btn-stroked" (click)="resetToAuto()" [disabled]="saving">
                <span class="material-icons">auto_fix_high</span> Reset to auto
              </button>
            }
            <button class="btn btn-stroked" (click)="startEdit()" [disabled]="!plan || saving">
              <span class="material-icons">edit</span> Edit
            </button>
          }
        </div>
      </div>

      @if (loading && !plan) {
        <div class="loading-container"><span class="spinner spinner-lg"></span></div>
      } @else if (plan && draft) {
        @if (deploymentMode === 'docker') {
          <div class="card card-pad note note-info">
            <span class="material-icons">info</span>
            <span>
              This deployment runs in Docker mode, where container limits govern memory.
              These settings apply only to native (JAR) mode.
            </span>
          </div>
        }

        @for (warning of plan.warnings; track warning) {
          <div class="card card-pad note" [class.note-danger]="!plan.fits">
            <span class="material-icons">{{ plan.fits ? 'info' : 'error' }}</span>
            <span>{{ warning }}</span>
          </div>
        }

        <!-- ── Budget ─────────────────────────────────────────────────────── -->
        <div class="card card-pad budget-card">
          <div class="card-head">
            <span class="material-icons card-avatar">memory</span>
            <h2 class="card-title">Budget</h2>
            <span class="ram-total">{{ gb(plan.total_ram_mb) }} GB installed</span>
          </div>

          <div class="budget-bar">
            <div class="seg seg-reserved" [style.width.%]="pct(plan.reserved_total_mb)"
                 title="Reserved for OS, MySQL, RabbitMQ, Nucleus"></div>
            <div class="seg seg-allocated" [style.width.%]="pct(plan.allocated_mb)"
                 title="Planned for Java services"></div>
            <div class="seg seg-free" [style.width.%]="pct(freeMb)" title="Unallocated"></div>
          </div>

          <div class="budget-legend">
            <span class="legend"><i class="swatch reserved"></i> Reserved
              <b>{{ gb(plan.reserved_total_mb) }} GB</b></span>
            <span class="legend"><i class="swatch allocated"></i> Services
              <b>{{ gb(plan.allocated_mb) }} GB</b></span>
            <span class="legend" [class.negative]="!plan.fits">
              <i class="swatch free"></i> {{ plan.fits ? 'Headroom' : 'Over budget' }}
              <b>{{ gb(abs(plan.headroom_mb)) }} GB</b>
            </span>
          </div>

          <p class="budget-note">
            A service costs the box more than its heap: roughly <b>-Xmx plus 280 MB</b> of
            metaspace, code cache, thread stacks and buffers. Every figure here budgets that tail.
          </p>
        </div>

        <div class="two-col">
          <!-- ── Mode ─────────────────────────────────────────────────────── -->
          <div class="card card-pad">
            <div class="card-head">
              <span class="material-icons card-avatar">tune</span>
              <h2 class="card-title">Mode</h2>
            </div>
            <div class="card-body">
              <label class="check">
                <input type="checkbox" [(ngModel)]="draft.enabled" [disabled]="!editing">
                <span>Apply JVM memory limits</span>
              </label>
              <p class="toggle-description">
                Off restores the untuned launch, where each JVM sizes its own heap at 25% of RAM.
              </p>

              <div class="divider"></div>

              <div class="mode-display">
                <span class="mode-label">Sizing:</span>
                <span class="mode-value" [class.native]="draft.auto_tune">
                  {{ draft.auto_tune ? 'Automatic (from this box\\'s RAM)' : 'Manual' }}
                </span>
              </div>
              <p class="toggle-description">
                @if (draft.auto_tune) {
                  Nucleus recalculates the plan on every service start. Editing any value below
                  freezes these numbers and switches to manual.
                } @else {
                  Values below are stored in nucleus.toml. Use <em>Reset to auto</em> to hand
                  sizing back to Nucleus.
                }
              </p>

              <div class="divider"></div>

              <label class="check">
                <input type="checkbox" [(ngModel)]="draft.exit_on_oom" [disabled]="!editing">
                <span>Stop a service that exhausts its heap</span>
              </label>
              <p class="toggle-description">
                Adds <code>-XX:+ExitOnOutOfMemoryError</code>, so the watchdog restarts the service
                instead of leaving it thrashing. An improvement for a spike, not for a leak — a
                leaking service will restart in a loop.
              </p>
            </div>
          </div>

          <!-- ── Reserves ─────────────────────────────────────────────────── -->
          <div class="card card-pad">
            <div class="card-head">
              <span class="material-icons card-avatar">shield</span>
              <h2 class="card-title">Reserved for other software</h2>
            </div>
            <div class="card-body">
              <div class="reserve-grid">
                <div class="field">
                  <label>Operating system</label>
                  <input class="input" type="number" min="0" step="64"
                         [(ngModel)]="draft.reserves!.os_mb" [disabled]="!editing">
                </div>
                <div class="field">
                  <label>Nucleus</label>
                  <input class="input" type="number" min="0" step="64"
                         [(ngModel)]="draft.reserves!.nucleus_mb" [disabled]="!editing">
                </div>
                <div class="field">
                  <label>MySQL</label>
                  <input class="input" type="number" min="0" step="64"
                         [(ngModel)]="draft.reserves!.mysql_mb" [disabled]="!editing">
                  <span class="hint">Buffer pool + overhead. InnoDB's own default pool is 128 MB — too small here.</span>
                </div>
                <div class="field">
                  <label>RabbitMQ</label>
                  <input class="input" type="number" min="0" step="64"
                         [(ngModel)]="draft.reserves!.rabbitmq_mb" [disabled]="!editing">
                  <span class="hint">Pin the broker's vm_memory_high_watermark to match.</span>
                </div>
                <div class="field">
                  <label>nginx + backup headroom</label>
                  <input class="input" type="number" min="0" step="64"
                         [(ngModel)]="draft.reserves!.other_mb" [disabled]="!editing">
                </div>
                <div class="field reserve-total">
                  <label>Total reserved</label>
                  <div class="reserve-total-value">{{ gb(reservesTotal) }} GB</div>
                </div>
              </div>
            </div>
          </div>
        </div>

        <!-- ── Per-service ────────────────────────────────────────────────── -->
        <div class="card table-card">
          <table class="data-table perf-table">
            <thead>
              <tr>
                <th>Service</th>
                <th>Tier</th>
                <th class="num">Min heap</th>
                <th class="num">Max heap</th>
                <th class="num">Metaspace</th>
                <th>GC</th>
                <th class="num">Planned cost</th>
                <th class="num">Measured</th>
              </tr>
            </thead>
            <tbody>
              @for (row of plan.services; track row.service) {
                <tr [class.not-installed]="!row.installed">
                  <td class="svc">
                    <span class="svc-name">{{ row.service }}</span>
                    @if (!row.installed) { <span class="chip">not installed</span> }
                    <div class="flags" [title]="row.jvm_args.join(' ')">
                      {{ row.jvm_args.length ? row.jvm_args.join(' ') : 'no JVM options' }}
                    </div>
                  </td>
                  <td><span class="tier tier-{{ row.tier }}">{{ row.tier }}</span></td>
                  <td class="num">
                    <input class="input input-num" type="number" min="32" step="32"
                           [(ngModel)]="draft.services[row.service].min_mb" [disabled]="!editing">
                  </td>
                  <td class="num">
                    <input class="input input-num" type="number" min="64" step="64"
                           [(ngModel)]="draft.services[row.service].max_mb" [disabled]="!editing">
                  </td>
                  <td class="num">
                    <input class="input input-num" type="number" min="64" step="64"
                           [(ngModel)]="draft.services[row.service].metaspace_mb" [disabled]="!editing">
                  </td>
                  <td>
                    <select class="select select-sm" [(ngModel)]="draft.services[row.service].gc"
                            [disabled]="!editing">
                      <option value="default">JVM default</option>
                      <option value="serial">Serial</option>
                      <option value="g1">G1</option>
                    </select>
                  </td>
                  <td class="num">{{ row.estimated_rss_mb }} MB</td>
                  <td class="num">
                    @if (row.measured_rss_mb !== null) {
                      <span [class.over]="row.measured_rss_mb > row.estimated_rss_mb">
                        {{ row.measured_rss_mb }} MB
                      </span>
                    } @else {
                      <span class="muted">—</span>
                    }
                  </td>
                </tr>
              }
            </tbody>
          </table>
          <div class="table-foot">
            Measured values come from the running processes. Where a service consistently sits well
            under its planned cost, lower its max heap and give the difference to one that runs close
            to the line.
          </div>
        </div>

        <div class="card card-pad note note-info">
          <span class="material-icons">restart_alt</span>
          <span>
            A JVM reads these options only at launch. Changes take effect the next time each service
            is restarted.
          </span>
        </div>
      }
    </div>
  `,
  styles: [`
    .performance-page { max-width: 1180px; }
    .header { margin-bottom: 16px; }
    .header h1 { margin: 0; }
    .subtitle { margin: 2px 0 0; font-size: 0.8rem; color: var(--text-muted); }
    .actions { display: flex; gap: 8px; }

    .note {
      display: flex; align-items: flex-start; gap: 10px;
      margin-bottom: 12px; font-size: 0.82rem; line-height: 1.5;
      .material-icons { font-size: 18px; flex-shrink: 0; color: var(--text-muted); }
    }
    .note-info .material-icons { color: #0284c7; }
    .note-danger {
      border-color: #dc2626;
      .material-icons { color: #dc2626; }
    }

    .budget-card { margin-bottom: 16px; }
    .card-head { display: flex; align-items: center; gap: 8px; margin-bottom: 14px; }
    .card-title { margin: 0; font-size: 0.95rem; }
    .card-avatar { color: #ea580c; }
    .ram-total { margin-left: auto; font-size: 0.8rem; color: var(--text-muted); }

    .budget-bar {
      display: flex; height: 22px; border-radius: 6px; overflow: hidden;
      background: var(--surface-2, #f1f5f9);
    }
    .seg { height: 100%; transition: width 0.3s ease; }
    .seg-reserved { background: #94a3b8; }
    .seg-allocated { background: #ea580c; }
    .seg-free { background: #22c55e33; }

    .budget-legend {
      display: flex; gap: 20px; flex-wrap: wrap;
      margin-top: 10px; font-size: 0.78rem; color: var(--text-muted);
      b { color: var(--text); margin-left: 4px; }
      .negative b { color: #dc2626; }
    }
    .swatch {
      display: inline-block; width: 10px; height: 10px;
      border-radius: 2px; margin-right: 5px;
      &.reserved { background: #94a3b8; }
      &.allocated { background: #ea580c; }
      &.free { background: #22c55e; }
    }
    .budget-note {
      margin: 12px 0 0; font-size: 0.78rem; line-height: 1.5; color: var(--text-muted);
    }

    .two-col {
      display: grid; grid-template-columns: repeat(auto-fit, minmax(340px, 1fr));
      gap: 16px; margin-bottom: 16px;
    }
    .card-body { display: flex; flex-direction: column; gap: 6px; }
    .divider { height: 1px; background: var(--border); margin: 10px 0; }
    .toggle-description {
      margin: 0; font-size: 0.75rem; line-height: 1.5; color: var(--text-muted);
      code { font-size: 0.72rem; }
    }
    .mode-display { display: flex; align-items: center; gap: 8px; font-size: 0.85rem; }
    .mode-label { color: var(--text-muted); }
    .mode-value { font-weight: 600; }
    .mode-value.native { color: #ea580c; }

    .reserve-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 10px 12px; }
    .reserve-total-value { font-size: 1.05rem; font-weight: 600; padding-top: 4px; }
    .field .hint { font-size: 0.68rem; line-height: 1.4; }

    .table-card { margin-bottom: 16px; overflow-x: auto; }
    .perf-table { width: 100%; font-size: 0.8rem; }
    .perf-table th.num, .perf-table td.num { text-align: right; }
    .perf-table .svc-name { font-weight: 600; }
    .perf-table .flags {
      margin-top: 3px; font-family: ui-monospace, monospace; font-size: 0.68rem;
      color: var(--text-muted); max-width: 340px;
      overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    }
    .not-installed { opacity: 0.55; }
    .input-num { width: 84px; text-align: right; }
    .select-sm { padding: 3px 6px; font-size: 0.75rem; }
    .muted { color: var(--text-muted); }
    .over { color: #dc2626; font-weight: 600; }

    .tier {
      display: inline-block; padding: 1px 7px; border-radius: 10px;
      font-size: 0.68rem; text-transform: uppercase; letter-spacing: 0.03em;
    }
    .tier-hot { background: #fee2e2; color: #b91c1c; }
    .tier-mid { background: #ffedd5; color: #c2410c; }
    .tier-light { background: #e0f2fe; color: #0369a1; }

    .table-foot {
      padding: 10px 14px; font-size: 0.75rem; line-height: 1.5;
      color: var(--text-muted); border-top: 1px solid var(--border);
    }
  `]
})
export class PerformanceComponent implements OnInit {
  private tauri = inject(TauriService);
  private notify = inject(NotificationService);

  plan: MemoryPlan | null = null;
  /** Editable copy. Also drives the read-only view, so both render one model. */
  draft: PerformanceConfig | null = null;
  deploymentMode: 'docker' | 'native' = 'native';

  loading = false;
  saving = false;
  editing = false;
  /** Serialised sizing at the moment Edit was pressed, for change detection. */
  private baseline = '';

  async ngOnInit(): Promise<void> {
    await this.load();
    try {
      const config = await this.tauri.invokeSilent<{ deployment_mode: 'docker' | 'native' }>('get_config');
      this.deploymentMode = config.deployment_mode;
    } catch {
      // Non-fatal: the banner is context, not a requirement.
    }
  }

  async load(): Promise<void> {
    this.loading = true;
    try {
      this.plan = await this.tauri.invoke<MemoryPlan>('get_performance_plan');
      this.draft = this.toDraft(this.plan);
      this.editing = false;
    } finally {
      this.loading = false;
    }
  }

  startEdit(): void {
    // Snapshot so save() can tell whether any number actually moved.
    this.baseline = JSON.stringify(this.sizingOf(this.draft!));
    this.editing = true;
  }

  cancelEdit(): void {
    if (this.plan) {
      this.draft = this.toDraft(this.plan);
    }
    this.editing = false;
  }

  async save(): Promise<void> {
    if (!this.draft) return;

    const invalid = Object.entries(this.draft.services).find(
      ([, m]) => m.min_mb > m.max_mb
    );
    if (invalid) {
      this.notify.warning(`${invalid[0]}: minimum heap cannot exceed maximum heap.`);
      return;
    }

    // The draft always carries every service, so persisting it after a real
    // edit both freezes the plan and switches sizing to manual — otherwise the
    // next service start would recompute straight over the operator's numbers.
    // Toggling a switch without touching a size leaves auto-tuning alone.
    const changed = JSON.stringify(this.sizingOf(this.draft)) !== this.baseline;
    const performance: PerformanceConfig = {
      ...this.draft,
      auto_tune: changed ? false : this.draft.auto_tune,
    };

    this.saving = true;
    try {
      this.plan = await this.tauri.invoke<MemoryPlan>('save_performance_config', { performance });
      this.draft = this.toDraft(this.plan);
      this.editing = false;
      this.notify.success('Memory plan saved. Restart each service to apply it.', 5000);
    } finally {
      this.saving = false;
    }
  }

  /** Drop every stored override and hand sizing back to Nucleus. */
  async resetToAuto(): Promise<void> {
    const performance: PerformanceConfig = {
      enabled: this.draft?.enabled ?? true,
      auto_tune: true,
      exit_on_oom: this.draft?.exit_on_oom ?? false,
      reserves: null,
      services: {},
    };

    this.saving = true;
    try {
      this.plan = await this.tauri.invoke<MemoryPlan>('save_performance_config', { performance });
      this.draft = this.toDraft(this.plan);
      this.editing = false;
      this.notify.success('Sizing handed back to Nucleus.', 4000);
    } finally {
      this.saving = false;
    }
  }

  // ── View helpers ───────────────────────────────────────────────────────────

  get freeMb(): number {
    return this.plan && this.plan.headroom_mb > 0 ? this.plan.headroom_mb : 0;
  }

  get reservesTotal(): number {
    const r = this.draft?.reserves;
    if (!r) return 0;
    return (
      Number(r.os_mb) + Number(r.nucleus_mb) + Number(r.mysql_mb) +
      Number(r.rabbitmq_mb) + Number(r.other_mb)
    );
  }

  gb(mb: number): string {
    return (mb / 1024).toFixed(1);
  }

  abs(n: number): number {
    return Math.abs(n);
  }

  /** Share of total RAM, as a bar width. */
  pct(mb: number): number {
    if (!this.plan || this.plan.total_ram_mb === 0) return 0;
    return Math.max(0, Math.min(100, (mb / this.plan.total_ram_mb) * 100));
  }

  /** Just the parts that constitute a sizing decision — toggles excluded. */
  private sizingOf(draft: PerformanceConfig): unknown {
    return { reserves: draft.reserves, services: draft.services };
  }

  /**
   * Flatten a plan into an editable config. Every service the plan knows about
   * gets an entry, so the table can bind to `draft.services[name]` without
   * null checks.
   */
  private toDraft(plan: MemoryPlan): PerformanceConfig {
    const services: PerformanceConfig['services'] = {};
    for (const row of plan.services as ServicePlan[]) {
      services[row.service] = { ...row.memory };
    }
    const reserves: ReserveConfig = { ...plan.reserves };
    return {
      enabled: plan.enabled,
      auto_tune: plan.auto_tune,
      exit_on_oom: plan.exit_on_oom,
      reserves,
      services,
    };
  }
}
