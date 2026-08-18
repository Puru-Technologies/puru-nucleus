import { Injectable, inject } from '@angular/core';
import { HttpClient, HttpHeaders, HttpParams } from '@angular/common/http';
import { firstValueFrom } from 'rxjs';
import { ConnectionService } from './connection.service';

/**
 * Describes how a Tauri command name maps onto a daemon REST call.
 *
 * - `pathParams`: argName -> token in `path` (e.g. { containerName: 'name' } fills `:name`)
 * - `query`:      arg names sent as ?query=
 * - `body`:       'rest' = all unused args as JSON; string[] = pick those keys;
 *                 Record = rename argName -> bodyKey; 'raw:<arg>' = send that arg as the whole body
 * - `unwrap`:     pull the real value out of the JSON envelope (e.g. {logs} -> string)
 * - `notFoundNull`: map HTTP 404 to null (mirrors Option<T> commands like get_license)
 */
export interface HttpMapping {
  method: 'GET' | 'POST' | 'PUT';
  path: string;
  pathParams?: Record<string, string>;
  query?: string[];
  body?: 'rest' | string[] | Record<string, string> | `raw:${string}`;
  unwrap?: (json: any) => any;
  notFoundNull?: boolean;
}

/**
 * Tauri-command → daemon-REST mapping. Only commands present here work in remote
 * mode; anything else throws a clear "not available in remote mode" error.
 * Phase 1 = monitoring/control (existing REST routes). Phase 2 appends setup.
 */
export const COMMAND_MAP: Record<string, HttpMapping> = {
  // ── Services ───────────────────────────────────────────────────────────────
  get_services: { method: 'GET', path: '/api/services' },
  start_service: { method: 'POST', path: '/api/services/:name/start', pathParams: { name: 'name' } },
  stop_service: { method: 'POST', path: '/api/services/:name/stop', pathParams: { name: 'name' } },
  restart_service: { method: 'POST', path: '/api/services/:name/restart', pathParams: { name: 'name' } },
  get_container_logs: {
    method: 'GET',
    path: '/api/services/:name/logs',
    pathParams: { containerName: 'name' },
    query: ['tail', 'since', 'until'],
    unwrap: (j) => j?.logs ?? '',
  },

  // ── System / config / license ──────────────────────────────────────────────
  get_telemetry_snapshot: { method: 'GET', path: '/api/system' },
  get_config: { method: 'GET', path: '/api/config' },
  get_performance_plan: { method: 'GET', path: '/api/performance' },
  save_performance_config: {
    method: 'PUT',
    path: '/api/performance',
    body: 'raw:performance',
  },
  get_license: { method: 'GET', path: '/api/license', notFoundNull: true },
  pull_settings: { method: 'POST', path: '/api/pull' },

  // ── Backups ────────────────────────────────────────────────────────────────
  start_backup: { method: 'POST', path: '/api/backup', body: { type: 'backup_type' } },
  get_backup_history: { method: 'GET', path: '/api/backup/list' },
  restore_backup: { method: 'POST', path: '/api/restore', body: { backupId: 'backup_id' } },
  restore_pitr: {
    method: 'POST',
    path: '/api/restore/pitr',
    body: { backupId: 'backup_id', stopDatetime: 'stop_datetime' },
  },
  ship_binlogs_lan: { method: 'POST', path: '/api/lan/binlog/ship' },

  // ── Updates (docker + native JARs) ─────────────────────────────────────────
  check_service_updates: { method: 'GET', path: '/api/updates/check' },
  update_docker_service: {
    method: 'POST',
    path: '/api/updates/:service',
    pathParams: { serviceName: 'service' },
    body: { newTag: 'tag' },
  },
  rollback_docker_service: { method: 'POST', path: '/api/rollback/:service', pathParams: { serviceName: 'service' } },
  pull_jars: { method: 'POST', path: '/api/jars/pull', body: ['services'] },
  check_jar_updates: { method: 'GET', path: '/api/jars/updates' },
  update_native_service: { method: 'POST', path: '/api/jars/update/:service', pathParams: { serviceName: 'service' } },
  rollback_native_service: { method: 'POST', path: '/api/jars/rollback/:service', pathParams: { serviceName: 'service' } },

  // ── Shell ──────────────────────────────────────────────────────────────────
  execute_shell_command: { method: 'POST', path: '/api/exec', body: ['command'] },
  get_shell_audit_log: { method: 'GET', path: '/api/exec/history' },
  get_allowed_shell_commands: { method: 'GET', path: '/api/exec/allowed' },

  // ── Alerts ─────────────────────────────────────────────────────────────────
  get_alerts: { method: 'GET', path: '/api/alerts' },
  acknowledge_alert: { method: 'POST', path: '/api/alerts/:id/ack', pathParams: { alertId: 'id' } },

  // ── Network ────────────────────────────────────────────────────────────────
  check_network: { method: 'GET', path: '/api/network' },
  run_speed_test: { method: 'POST', path: '/api/network/speedtest' },

  // ── Logs ───────────────────────────────────────────────────────────────────
  get_log_sources: { method: 'GET', path: '/api/logs/sources' },
  list_log_files: { method: 'GET', path: '/api/logs/files', query: ['path'] },
  read_log_file: { method: 'GET', path: '/api/logs/file', query: ['path', 'tail', 'offset', 'limit'] },

  // ── Seed (already REST-exposed) ────────────────────────────────────────────
  seed_data: { method: 'POST', path: '/api/seed', body: ['db', 'queues', 'templates'] },

  // ── Phase 2: setup pipeline, detection, templates, config, modules ─────────
  setup_check_prerequisites: { method: 'POST', path: '/api/setup/prerequisites/check' },
  setup_create_databases: { method: 'POST', path: '/api/setup/databases' },
  setup_configure_rabbitmq: { method: 'POST', path: '/api/setup/rabbitmq' },
  setup_generate_config: { method: 'POST', path: '/api/setup/compose' },
  setup_pull_images: { method: 'POST', path: '/api/setup/images' },
  setup_start_services: { method: 'POST', path: '/api/setup/services/start' },
  setup_generate_env_files: { method: 'POST', path: '/api/setup/env-files' },
  setup_pull_jars: { method: 'POST', path: '/api/setup/jars' },
  setup_start_native_services: { method: 'POST', path: '/api/setup/native-services/start' },
  setup_seed_queues: { method: 'POST', path: '/api/setup/seed-queues' },
  setup_seed_database: { method: 'POST', path: '/api/setup/seed-database' },
  setup_health_check: { method: 'POST', path: '/api/setup/health-check' },
  setup_configure_backups: { method: 'POST', path: '/api/setup/backups' },
  setup_tls: { method: 'POST', path: '/api/setup/tls' },
  setup_reset: { method: 'POST', path: '/api/setup/reset' },
  detect_existing_setup: { method: 'GET', path: '/api/detect' },
  check_prerequisites: { method: 'GET', path: '/api/prerequisites' },
  get_deployment_mode: { method: 'GET', path: '/api/deployment-mode' },
  get_service_modules: { method: 'GET', path: '/api/modules' },
  save_config: { method: 'PUT', path: '/api/config', body: 'raw:config' },
  finalise_templates: { method: 'POST', path: '/api/templates/finalise' },
  check_template_updates: { method: 'GET', path: '/api/templates/updates' },
  apply_template_updates: { method: 'POST', path: '/api/templates/apply' },
};

/**
 * Executes a Tauri command against the active remote daemon over HTTP, mirroring
 * the local `invoke()` contract: resolves with the command's value, rejects with
 * a string message.
 */
@Injectable({ providedIn: 'root' })
export class RemoteTransportService {
  private http = inject(HttpClient);
  private conn = inject(ConnectionService);

  /** True if a command can be served remotely. */
  canHandle(command: string): boolean {
    return command in COMMAND_MAP;
  }

  async execute<T>(command: string, args: Record<string, unknown> = {}): Promise<T> {
    const m = COMMAND_MAP[command];
    if (!m) {
      throw `"${command}" is not available in remote mode`;
    }

    const used = new Set<string>();

    // Path params
    let path = m.path;
    if (m.pathParams) {
      for (const [arg, token] of Object.entries(m.pathParams)) {
        path = path.replace(`:${token}`, encodeURIComponent(String(args[arg] ?? '')));
        used.add(arg);
      }
    }

    // Query params
    let params = new HttpParams();
    for (const q of m.query ?? []) {
      const v = args[q];
      if (v !== undefined && v !== null && v !== '') params = params.set(q, String(v));
      used.add(q);
    }

    // Body
    let body: unknown = undefined;
    if (typeof m.body === 'string' && m.body.startsWith('raw:')) {
      body = args[m.body.slice(4)];
    } else if (m.body === 'rest') {
      const rest: Record<string, unknown> = {};
      for (const [k, v] of Object.entries(args)) if (!used.has(k)) rest[k] = v;
      body = rest;
    } else if (Array.isArray(m.body)) {
      const picked: Record<string, unknown> = {};
      for (const k of m.body) {
        picked[k] = args[k];
        used.add(k);
      }
      body = picked;
    } else if (m.body && typeof m.body === 'object') {
      const renamed: Record<string, unknown> = {};
      for (const [arg, key] of Object.entries(m.body)) {
        renamed[key] = args[arg];
        used.add(arg);
      }
      body = renamed;
    }

    const url = this.conn.baseUrl() + path;
    const headers = new HttpHeaders(this.conn.headers());

    try {
      const req$ =
        m.method === 'GET'
          ? this.http.get<any>(url, { headers, params })
          : m.method === 'PUT'
            ? this.http.put<any>(url, body ?? {}, { headers, params })
            : this.http.post<any>(url, body ?? {}, { headers, params });
      const res = await firstValueFrom(req$);
      return (m.unwrap ? m.unwrap(res) : res) as T;
    } catch (e: any) {
      if (m.notFoundNull && e?.status === 404) return null as T;
      throw this.normalizeError(e);
    }
  }

  /** Daemon errors arrive as plain-text bodies; mirror Tauri's string rejection. */
  private normalizeError(e: any): string {
    if (typeof e?.error === 'string' && e.error) return e.error;
    if (e?.error?.message) return e.error.message;
    if (e?.status === 0) return `Cannot reach ${this.conn.baseUrl()} — is the daemon running?`;
    if (e?.status) return `HTTP ${e.status}: ${e.statusText ?? 'error'}`;
    return e?.message ?? 'Remote request failed';
  }
}
