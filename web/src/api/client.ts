// Every call to the server.

import type {
  Channel,
  ChannelInput,
  DeliveryLog,
  Healthz,
  History,
  HistoryRange,
  Maintenance,
  MaintenanceInput,
  Me,
  Methods,
  Monitor,
  MonitorConfig,
  MonitorEvent,
  OverviewItem,
  SatelliteRef,
  Settings,
  SmtpInput,
  SmtpSettings,
} from './types';

export class ApiError extends Error {
  constructor(
    public readonly status: number,
    /** The server's error code (`rate_limited`, `invalid`, …) or '' when there is none. */
    public readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

/** Fired on any 401, so the app can show the sign-in page. */
export const UNAUTHENTICATED_EVENT = 'uwustatus:unauthenticated';

/** `VITE_MOCK=1` (`pnpm dev:mock`): every call goes to `mock.ts`, no server needed. */
export function isMock(): boolean {
  return import.meta.env.VITE_MOCK === '1';
}

/** Builds `path?key=value` from the defined params. */
export function withQuery(
  path: string,
  params: Record<string, string | number | undefined | null>,
): string {
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined && value !== null && value !== '') query.set(key, String(value));
  }
  const text = query.toString();
  return text ? `${path}?${text}` : path;
}

/** The server's text for an error body (`{ error, message }`), or a fallback by status. */
export function errorFrom(status: number, body: unknown): ApiError {
  const record = body && typeof body === 'object' ? (body as Record<string, unknown>) : {};
  const code = typeof record.error === 'string' ? record.error : '';
  let message = typeof record.message === 'string' && record.message ? record.message : '';
  if (!message) {
    if (status === 0) message = 'Server nicht erreichbar.';
    else if (status === 401) message = 'Nicht angemeldet.';
    else message = `Fehler ${status}`;
  }
  return new ApiError(status, code, message);
}

type Method = 'GET' | 'POST' | 'PUT' | 'DELETE';

export async function request<T>(method: Method, path: string, body?: unknown): Promise<T> {
  let status: number;
  let parsed: unknown;
  if (isMock()) {
    const mock = await import('./mock');
    ({ status, body: parsed } = await mock.mockRequest(method, path, body));
  } else {
    let response: Response;
    try {
      response = await fetch(path, {
        method,
        credentials: 'same-origin',
        headers:
          body === undefined
            ? { accept: 'application/json' }
            : { 'content-type': 'application/json', accept: 'application/json' },
        body: body === undefined ? undefined : JSON.stringify(body),
      });
    } catch {
      throw errorFrom(0, null);
    }
    status = response.status;
    const text = await response.text();
    try {
      parsed = text ? JSON.parse(text) : undefined;
    } catch {
      parsed = undefined;
    }
  }
  if (status >= 200 && status < 300) return parsed as T;
  // A wrong local password is a 403, not a lost session.
  if (status === 401) window.dispatchEvent(new Event(UNAUTHENTICATED_EVENT));
  throw errorFrom(status, parsed);
}

/** `null` when the route does not exist (yet): phases that are not merged answer 404. */
/** The editable part of a monitor, as `PUT` wants it (it rejects unknown fields). */
export function configOf(m: Monitor): MonitorConfig {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  const { id, push_token, created, updated, ...config } = m;
  return config;
}

export const api = {
  me: () => request<Me>('GET', '/api/me'),
  methods: () => request<Methods>('GET', '/api/auth/methods'),
  localLogin: (user: string, password: string, totp?: string) =>
    request<void>('POST', '/api/auth/local', { user, password, totp: totp || undefined }),
  logout: () => request<void>('POST', '/api/auth/logout', {}),

  settings: () => request<Settings>('GET', '/api/settings'),
  saveSettings: (patch: Partial<Settings>) => request<Settings>('PUT', '/api/settings', patch),

  channels: () => request<Channel[]>('GET', '/api/channels'),
  createChannel: (input: ChannelInput) => request<Channel>('POST', '/api/channels', input),
  saveChannel: (id: number, input: ChannelInput) =>
    request<Channel>('PUT', `/api/channels/${id}`, input),
  deleteChannel: (id: number) => request<void>('DELETE', `/api/channels/${id}`),
  testChannel: (id: number) => request<void>('POST', `/api/channels/${id}/test`, {}),
  channelLog: (id: number) => request<DeliveryLog[]>('GET', `/api/channels/${id}/log`),
  monitorChannels: (id: number) => request<number[]>('GET', `/api/monitors/${id}/channels`),
  saveMonitorChannels: (id: number, ids: number[]) =>
    request<number[]>('PUT', `/api/monitors/${id}/channels`, ids),
  smtp: () => request<SmtpSettings>('GET', '/api/smtp'),
  saveSmtp: (input: SmtpInput) => request<SmtpSettings>('PUT', '/api/smtp', input),

  healthz: () => request<Healthz>('GET', '/healthz'),

  overview: () => request<OverviewItem[]>('GET', '/api/overview'),
  monitors: () => request<Monitor[]>('GET', '/api/monitors'),
  monitor: (id: number) => request<Monitor>('GET', `/api/monitors/${id}`),
  createMonitor: (config: MonitorConfig) => request<Monitor>('POST', '/api/monitors', config),
  saveMonitor: (id: number, config: MonitorConfig) =>
    request<Monitor>('PUT', `/api/monitors/${id}`, config),
  deleteMonitor: (id: number) => request<void>('DELETE', `/api/monitors/${id}`),
  history: (id: number, range: HistoryRange) =>
    request<History>('GET', withQuery(`/api/monitors/${id}/history`, { range })),
  events: (id: number, before?: number) =>
    request<MonitorEvent[]>('GET', withQuery(`/api/monitors/${id}/events`, { before, limit: 50 })),

  maintenance: () => request<Maintenance[]>('GET', '/api/maintenance'),
  maintenanceOne: (id: number) => request<Maintenance>('GET', `/api/maintenance/${id}`),
  createMaintenance: (input: MaintenanceInput) =>
    request<Maintenance>('POST', '/api/maintenance', input),
  saveMaintenance: (id: number, input: MaintenanceInput) =>
    request<Maintenance>('PUT', `/api/maintenance/${id}`, input),
  deleteMaintenance: (id: number) => request<void>('DELETE', `/api/maintenance/${id}`),

  satellites: () => request<SatelliteRef[]>('GET', '/api/satellites'),
};

/** Where the browser goes to sign in with UwUAuth; `next` is the path to come back to. */
export const loginUrl = (next: string) => withQuery('/api/auth/login', { next });
