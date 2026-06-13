import { Injectable, computed, signal } from '@angular/core';

/** A saved remote Nucleus daemon the GUI can connect to. */
export interface SavedServer {
  id: string;
  name: string;
  host: string;
  port: number;
  apiKey: string;
}

/** Shape of GET /api/health. */
export interface HealthResponse {
  status: string;
  version: string;
  uptime_seconds: number;
  docker_connected: boolean;
}

export interface ConnectionHealth {
  reachable: boolean;
  version?: string;
  dockerConnected?: boolean;
  checkedAt?: string;
}

const LS_MODE = 'nucleus.connection.mode';
const LS_ACTIVE = 'nucleus.connection.activeServerId';
const LS_SERVERS = 'nucleus.servers';

/**
 * Holds the GUI's connection mode (local Tauri IPC vs a remote daemon's REST
 * API) and the list of saved servers. Persisted in localStorage — this is the
 * admin workstation's preference, NOT deployment config (nucleus.toml).
 *
 * Security: remote mode is only honoured for a server with a non-empty API key
 * (plain-HTTP-on-LAN posture). A keyless server may only be health-tested.
 */
@Injectable({ providedIn: 'root' })
export class ConnectionService {
  readonly mode = signal<'local' | 'remote'>('local');
  readonly servers = signal<SavedServer[]>([]);
  readonly activeServerId = signal<string | null>(null);
  readonly health = signal<ConnectionHealth | null>(null);

  readonly activeServer = computed<SavedServer | null>(
    () => this.servers().find((s) => s.id === this.activeServerId()) ?? null,
  );

  /** Notifies listeners (e.g. the license guard) when the connection changes. */
  private changeListeners: Array<() => void> = [];

  constructor() {
    this.load();
  }

  // ── Mode / connection state ────────────────────────────────────────────────

  /** True only when remote mode is active AND the active server has an API key. */
  isRemote(): boolean {
    const s = this.activeServer();
    return this.mode() === 'remote' && !!s && !!s.apiKey;
  }

  baseUrl(server?: SavedServer): string {
    const s = server ?? this.activeServer();
    return s ? `http://${s.host}:${s.port}` : '';
  }

  headers(server?: SavedServer): Record<string, string> {
    const s = server ?? this.activeServer();
    return s?.apiKey ? { 'X-API-Key': s.apiKey } : {};
  }

  onChange(fn: () => void): void {
    this.changeListeners.push(fn);
  }

  private emitChange(): void {
    for (const fn of this.changeListeners) {
      try {
        fn();
      } catch {
        /* ignore */
      }
    }
  }

  // ── Server list management ─────────────────────────────────────────────────

  newId(): string {
    return (globalThis.crypto?.randomUUID?.() ?? 'srv-' + Date.now().toString(36));
  }

  upsertServer(server: SavedServer): void {
    const list = [...this.servers()];
    const idx = list.findIndex((s) => s.id === server.id);
    if (idx >= 0) list[idx] = server;
    else list.push(server);
    this.servers.set(list);
    this.persist();
  }

  removeServer(id: string): void {
    this.servers.set(this.servers().filter((s) => s.id !== id));
    if (this.activeServerId() === id) {
      this.activeServerId.set(null);
      this.mode.set('local');
    }
    this.persist();
    this.emitChange();
  }

  /** Switch to remote mode pointed at a saved server (must have an API key). */
  connect(id: string): void {
    const s = this.servers().find((x) => x.id === id);
    if (!s || !s.apiKey) return;
    this.activeServerId.set(id);
    this.mode.set('remote');
    this.health.set(null);
    this.persist();
    this.emitChange();
  }

  /** Return to local (this-machine) mode. */
  goLocal(): void {
    this.mode.set('local');
    this.health.set(null);
    this.persist();
    this.emitChange();
  }

  setHealth(h: ConnectionHealth | null): void {
    this.health.set(h);
  }

  // ── Health probe (used by the Connect screen + status chip) ────────────────

  async testConnection(server: SavedServer): Promise<HealthResponse> {
    const res = await fetch(`http://${server.host}:${server.port}/api/health`, {
      method: 'GET',
      headers: server.apiKey ? { 'X-API-Key': server.apiKey } : {},
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    return (await res.json()) as HealthResponse;
  }

  // ── Persistence ────────────────────────────────────────────────────────────

  private load(): void {
    try {
      const mode = localStorage.getItem(LS_MODE);
      if (mode === 'remote' || mode === 'local') this.mode.set(mode);
      const servers = localStorage.getItem(LS_SERVERS);
      if (servers) this.servers.set(JSON.parse(servers) as SavedServer[]);
      this.activeServerId.set(localStorage.getItem(LS_ACTIVE));
      // Remote mode requires a valid, keyed active server; otherwise fall back.
      if (this.mode() === 'remote' && !this.isRemote()) {
        this.mode.set('local');
      }
    } catch {
      /* corrupt storage — start clean in local mode */
    }
  }

  private persist(): void {
    localStorage.setItem(LS_MODE, this.mode());
    localStorage.setItem(LS_SERVERS, JSON.stringify(this.servers()));
    const id = this.activeServerId();
    if (id) localStorage.setItem(LS_ACTIVE, id);
    else localStorage.removeItem(LS_ACTIVE);
  }
}
