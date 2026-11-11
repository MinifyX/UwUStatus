// In-memory server for `pnpm dev:mock` (VITE_MOCK=1): UI work and screenshots without the backend.
// Only reserved names (RFC 2606) and documentation IPs (RFC 5737).
import type {
  Channel,
  History,
  HistoryRange,
  Maintenance,
  Monitor,
  MonitorConfig,
  MonitorEvent,
  OverviewItem,
  RawPoint,
  ResultStatus,
  RollupPoint,
  Settings,
} from './types';

const NOW = Math.floor(Date.now() / 1000);
const BASE: Omit<Monitor, 'id' | 'name' | 'group' | 'check' | 'kind'> = {
  description: '',
  tags: [],
  interval_s: 60,
  timeout_s: 10,
  retries: 1,
  retry_interval_s: 0,
  reminder_min: 0,
  muted: false,
  paused: false,
  parent_id: null,
  locations: ['local'],
  quorum: 'any',
  cert_warn_days: 14,
  push_token: null,
  created: NOW - 40 * 86400,
  updated: NOW - 86400,
};

let monitors: Monitor[] = [
  {
    ...BASE,
    id: 1,
    name: 'Website',
    group: 'Öffentlich',
    kind: 'check',
    tags: ['web'],
    check: { type: 'http', url: 'https://www.example.com/' },
    locations: ['local', 'sat-1'],
    quorum: 'majority',
  },
  {
    ...BASE,
    id: 2,
    name: 'Mail SMTP',
    group: 'Mail',
    kind: 'check',
    tags: ['mail'],
    check: { type: 'smtp', host: 'mail.example.com', port: 465, tls: 'tls' },
  },
  {
    ...BASE,
    id: 3,
    name: 'Mail IMAP',
    group: 'Mail',
    kind: 'check',
    tags: ['mail'],
    check: { type: 'imap', host: 'mail.example.com', port: 993, tls: 'tls' },
  },
  {
    ...BASE,
    id: 4,
    name: 'UwUAuth',
    group: 'Suite',
    kind: 'check',
    tags: ['suite'],
    check: { type: 'healthz', url: 'https://auth.example.com/healthz' },
  },
  {
    ...BASE,
    id: 5,
    name: 'Router',
    group: 'Netz',
    kind: 'check',
    check: { type: 'ping', host: '192.0.2.1' },
  },
  {
    ...BASE,
    id: 6,
    name: 'NAS',
    group: 'Netz',
    kind: 'check',
    parent_id: 5,
    check: { type: 'tcp', host: '192.0.2.20', port: 5001 },
  },
  {
    ...BASE,
    id: 7,
    name: 'Backup-Job',
    group: '',
    kind: 'push',
    check: null,
    interval_s: 86400,
    push_token: 'mockpushtoken0000000000000000000',
  },
  {
    ...BASE,
    id: 8,
    name: 'DNS',
    group: 'Netz',
    kind: 'check',
    paused: true,
    check: { type: 'dns', name: 'example.com', record_type: 'A', expected: ['203.0.113.7'] },
  },
  {
    ...BASE,
    id: 9,
    name: 'Immich',
    group: '',
    kind: 'check',
    muted: true,
    tags: ['foto'],
    check: { type: 'docker', url: 'http://socket-proxy.test:2375', container: 'immich' },
  },
];

const STATE: Record<number, OverviewItem['state']> = {
  1: 'up',
  2: 'up',
  3: 'degraded',
  4: 'up',
  5: 'down',
  6: 'down',
  7: 'up',
  8: 'paused',
  9: 'maintenance',
};

// Deterministic noise, so screenshots stay the same.
const noise = (i: number) => (((Math.sin(i * 12.9898) * 43758.5453) % 1) + 1) % 1;
const statusAt = (id: number, i: number): ResultStatus =>
  STATE[id] === 'down' && i > 55 ? 'down' : noise(id * 1000 + i) > 0.97 ? 'degraded' : 'up';

function overview(m: Monitor): OverviewItem {
  const state = STATE[m.id] ?? 'pending';
  const recent = Array.from({ length: 60 }, (_, i) => ({
    at: NOW - (60 - i) * 60,
    status: statusAt(m.id, i),
  }));
  const down = state === 'down';
  return {
    id: m.id,
    name: m.name,
    group: m.group,
    tags: m.tags,
    kind: m.kind,
    paused: m.paused,
    muted: m.muted,
    parent_id: m.parent_id,
    state,
    reason: down ? 'connection refused' : '',
    since: NOW - (down ? 420 : 9 * 86400),
    unreachable_via_parent: m.id === 6,
    last_check: state === 'paused' ? NOW - 3 * 86400 : NOW - 25,
    latency_ms: down || state === 'paused' ? null : Math.round(40 + noise(m.id) * 200),
    message: down ? 'connection refused' : 'OK',
    uptime_24h: down ? 97.12 : 99.98,
    uptime_30d: down ? 99.21 : 99.995,
    version: m.check?.type === 'healthz' ? '0.5.0-beta.4' : null,
    cert_days_left: m.check?.type === 'http' || m.check?.type === 'smtp' ? 47 : null,
    recent: state === 'paused' ? [] : recent,
    locations: m.locations.map((location, i) => ({
      location,
      state,
      failures: down ? 3 : 0,
      at: NOW - 25 - i * 3,
      message: down ? 'connection refused' : 'OK',
    })),
  };
}

function history(m: Monitor, range: HistoryRange): History {
  const span = { '24h': 86400, '7d': 7 * 86400, '30d': 30 * 86400, '90d': 90 * 86400 }[range];
  const from = NOW - span;
  if (range === '24h') {
    const points: RawPoint[] = [];
    for (let i = 0; i < 288; i++) {
      for (const location of m.locations) {
        const status = statusAt(m.id, i % 60);
        const base = location === 'local' ? 60 : 110;
        points.push({
          at: from + i * 300,
          location,
          status,
          latency_ms: Math.round(
            base + noise(i + location.length) * 80 + (i > 200 && i < 215 ? 300 : 0),
          ),
          message: status === 'up' ? 'OK' : 'slow',
        });
      }
    }
    if (m.id === 1)
      points.splice(
        400,
        6,
        ...points.slice(400, 406).map((p) => ({ ...p, status: 'down' as const, latency_ms: 0 })),
      );
    return { from, to: NOW, uptime: 99.79, level: 'raw', points };
  }
  const step = range === '7d' ? 3600 : 86400;
  const points: RollupPoint[] = [];
  for (let start = from; start < NOW; start += step) {
    const i = (start - from) / step;
    const down = noise(m.id + i) > 0.95 ? 2 : 0;
    const avg = 70 + noise(i) * 60;
    points.push({
      start,
      up: 60 - down,
      degraded: 0,
      down,
      latency_avg: avg,
      latency_p95: avg * 1.6,
      uptime: ((60 - down) * 100) / 60,
    });
  }
  return { from, to: NOW, uptime: 99.91, level: range === '7d' ? 'hour' : 'day', points };
}

function events(id: number, before?: number): MonitorEvent[] {
  const all: MonitorEvent[] = Array.from({ length: 12 }, (_, i) => ({
    id: 100 - i,
    at: NOW - 420 - i * 86400 * 2,
    kind: i === 3 ? 'version' : i === 5 ? 'cert' : 'state',
    from: i === 3 ? '0.5.0-beta.3' : i === 5 ? '3' : i % 2 ? 'down' : 'up',
    to: i === 3 ? '0.5.0-beta.4' : i === 5 ? '90' : i % 2 ? 'up' : 'down',
    reason: i % 2 || i === 3 || i === 5 ? '' : 'connection refused',
  }));
  return id === 7 ? [] : all.filter((e) => before === undefined || e.id < before).slice(0, 50);
}

let maintenance: Maintenance[] = [
  {
    id: 1,
    title: 'Updates am Sonntag',
    description: 'Container-Updates',
    active: true,
    monitors: [],
    groups: ['Mail'],
    schedule: {
      type: 'weekly',
      weekdays: [7],
      time: '03:00',
      duration_min: 60,
      timezone: 'Europe/Berlin',
    },
  },
  {
    id: 2,
    title: 'Router-Tausch',
    description: '',
    active: true,
    monitors: [5, 6],
    groups: [],
    schedule: { type: 'once', start: NOW + 3 * 86400, end: NOW + 3 * 86400 + 7200 },
  },
];
let settings: Settings = {
  retention_raw_days: 30,
  retention_hourly_days: 180,
  retention_daily_days: 730,
  default_interval_s: 60,
};
const EVENTS = {
  down: true,
  degraded: false,
  up: true,
  reminder: true,
  cert: true,
  version: false,
};
const STAMP = { created: NOW - 9 * 86400, updated: NOW - 86400 };
const channels: Channel[] = [
  {
    id: 1,
    name: 'Mail an Admin',
    enabled: true,
    is_default: true,
    ...STAMP,
    lang: 'de',
    events: EVENTS,
    secrets: [],
    channel: { type: 'email', to: 'admin@example.com' },
  },
  {
    id: 2,
    name: 'ntfy',
    enabled: true,
    is_default: false,
    ...STAMP,
    lang: 'de',
    events: EVENTS,
    secrets: [],
    channel: { type: 'ntfy', url: 'https://ntfy.example.com', topic: 'status' },
  },
  {
    id: 3,
    name: 'Discord',
    enabled: false,
    is_default: false,
    ...STAMP,
    lang: 'en',
    events: EVENTS,
    secrets: ['webhook_url'],
    channel: { type: 'discord' },
  },
];
const monitorChannels: Record<number, number[]> = { 1: [1, 2] };
let nextId = 100;

type Reply = { status: number; body?: unknown };
const ok = (body?: unknown): Reply => ({ status: body === undefined ? 204 : 200, body });
const missing: Reply = { status: 404, body: { error: 'not_found', message: 'Nicht gefunden' } };

export async function mockRequest(method: string, path: string, body?: unknown): Promise<Reply> {
  await new Promise((resolve) => setTimeout(resolve, 120));
  const url = new URL(path, 'http://mock.test');
  const p = url.pathname;
  const id = Number(p.split('/')[3]);
  const monitor = monitors.find((m) => m.id === id);

  if (p === '/api/me') return ok({ name: 'Admin', email: 'admin@example.com', method: 'oidc' });
  if (p === '/api/auth/methods') return ok({ oidc: true, local: true, local_totp: true });
  if (p === '/api/auth/logout') return ok();
  if (p === '/healthz') return ok({ status: 'ok', version: '0.1.0-mock' });
  if (p === '/api/settings') {
    if (method === 'PUT') settings = { ...settings, ...(body as Partial<Settings>) };
    return ok(settings);
  }
  if (p === '/api/overview') return ok(monitors.map(overview));
  if (p === '/api/satellites')
    return ok([{ id: 'sat-1', name: 'VPS Frankfurt', region: 'EU', status: 'paired' }]);
  if (p === '/api/channels') return ok(channels);
  if (p.startsWith('/api/channels/') && p.endsWith('/log')) return ok([]);

  if (p === '/api/monitors') {
    if (method === 'POST') {
      const m: Monitor = {
        ...(body as MonitorConfig),
        id: nextId++,
        created: NOW,
        updated: NOW,
        push_token:
          (body as MonitorConfig).kind === 'push' ? 'mocknewtoken000000000000000000' : null,
      };
      monitors.push(m);
      return ok(m);
    }
    return ok(monitors);
  }
  if (p.startsWith('/api/monitors/')) {
    if (!monitor) return missing;
    if (p.endsWith('/history'))
      return ok(history(monitor, (url.searchParams.get('range') ?? '24h') as HistoryRange));
    if (p.endsWith('/events')) {
      const before = url.searchParams.get('before');
      return ok(events(id, before ? Number(before) : undefined));
    }
    if (p.endsWith('/channels')) {
      if (method === 'PUT') monitorChannels[id] = body as number[];
      return ok(monitorChannels[id] ?? []);
    }
    if (method === 'PUT') {
      Object.assign(monitor, body as MonitorConfig, { updated: NOW });
      if (monitor.paused) STATE[id] = 'paused';
      else if (STATE[id] === 'paused') STATE[id] = 'up';
      return ok(monitor);
    }
    if (method === 'DELETE') {
      monitors = monitors.filter((m) => m.id !== id);
      return ok();
    }
    return ok(monitor);
  }

  if (p === '/api/maintenance') {
    if (method === 'POST') {
      const m = { ...(body as Maintenance), id: nextId++ };
      maintenance.push(m);
      return ok(m);
    }
    return ok(maintenance);
  }
  if (p.startsWith('/api/maintenance/')) {
    const m = maintenance.find((x) => x.id === id);
    if (!m) return missing;
    if (method === 'PUT') Object.assign(m, body);
    if (method === 'DELETE') {
      maintenance = maintenance.filter((x) => x.id !== id);
      return ok();
    }
    return ok(m);
  }
  return missing;
}
