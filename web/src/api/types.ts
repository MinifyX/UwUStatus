// Shapes of the server's JSON.

export interface Me {
  name: string;
  email: string | null;
  method: 'oidc' | 'local';
}

export interface Methods {
  oidc: boolean;
  local: boolean;
  local_totp: boolean;
}

export interface Settings {
  retention_raw_days: number;
  retention_hourly_days: number;
  retention_daily_days: number;
  default_interval_s: number;
}

export type ChannelType =
  'email' | 'webhook' | 'ntfy' | 'gotify' | 'pushover' | 'discord' | 'telegram';

/** `channel` of a notification channel: the type plus its fields (see `lib/channels.ts`). */
export type ChannelSpec = { type: ChannelType } & Record<string, unknown>;

export interface ChannelEvents {
  down: boolean;
  degraded: boolean;
  up: boolean;
  reminder: boolean;
  cert: boolean;
  version: boolean;
}

export interface ChannelInput {
  name: string;
  enabled: boolean;
  lang: 'de' | 'en';
  events: ChannelEvents;
  is_default: boolean;
  channel: ChannelSpec;
  /** Name → new value; '' deletes, a missing name keeps the stored one. */
  secrets?: Record<string, string>;
}

export interface Channel extends Omit<ChannelInput, 'secrets'> {
  id: number;
  /** Names of the secrets that are set; values never come back. */
  secrets: string[];
  created: number;
  updated: number;
}

// --- Monitors, history, maintenance (phase 7) ---

export type MonState = 'pending' | 'up' | 'degraded' | 'down' | 'maintenance' | 'paused';
export type ResultStatus = 'up' | 'degraded' | 'down';
export type Quorum = 'any' | 'majority' | 'all';
export type MailTls = 'none' | 'start_tls' | 'tls';

export type Check =
  | {
      type: 'http';
      url: string;
      method?: string;
      headers?: Record<string, string>;
      body?: string | null;
      accepted_status?: string[];
      follow_redirects?: boolean;
      ignore_tls_errors?: boolean;
      keyword?: string | null;
      keyword_absent?: boolean;
      json_path?: { path: string; value: string } | null;
    }
  | { type: 'tcp'; host: string; port: number }
  | { type: 'ping'; host: string }
  | {
      type: 'dns';
      name: string;
      record_type?: string;
      resolver?: string | null;
      expected?: string[];
    }
  | {
      type: 'smtp' | 'imap';
      host: string;
      port: number;
      tls?: MailTls;
      ignore_tls_errors?: boolean;
    }
  | { type: 'docker'; url: string; container: string }
  | { type: 'healthz'; url: string };

export type CheckType = Check['type'];

/** What the admin edits (`PUT /api/monitors/{id}` takes exactly these fields). */
export interface MonitorConfig {
  name: string;
  description: string;
  group: string;
  tags: string[];
  kind: 'check' | 'push';
  check: (Check & { timeout_ms?: number }) | null;
  interval_s: number;
  timeout_s: number;
  retries: number;
  retry_interval_s: number;
  reminder_min: number;
  muted: boolean;
  paused: boolean;
  parent_id: number | null;
  locations: string[];
  quorum: Quorum;
  cert_warn_days: number;
}

export interface Monitor extends MonitorConfig {
  id: number;
  push_token: string | null;
  created: number;
  updated: number;
}

export interface DeliveryLog {
  id: number;
  at: number;
  monitor_id: number | null;
  event: string;
  title: string;
  ok: boolean;
  attempts: number;
  error: string;
}

export interface SmtpSettings {
  host: string;
  port: number;
  security: 'starttls' | 'tls' | 'none';
  user: string;
  from: string;
  password_set: boolean;
  configured: boolean;
}

export interface SmtpInput extends Omit<SmtpSettings, 'password_set' | 'configured'> {
  /** Missing: keep; '': delete. */
  password?: string;
}

export interface LocationState {
  location: string;
  state: string;
  failures: number;
  at: number;
  message: string;
}

export interface OverviewItem {
  id: number;
  name: string;
  group: string;
  tags: string[];
  kind: 'check' | 'push';
  paused: boolean;
  muted: boolean;
  parent_id: number | null;
  state: MonState;
  reason: string;
  since: number | null;
  unreachable_via_parent: boolean;
  last_check: number | null;
  latency_ms: number | null;
  message: string | null;
  uptime_24h: number | null;
  uptime_30d: number | null;
  version: string | null;
  cert_days_left: number | null;
  recent: { at: number; status: ResultStatus }[];
  locations: LocationState[];
}

export type HistoryRange = '24h' | '7d' | '30d' | '90d';

export interface RawPoint {
  at: number;
  location: string;
  status: ResultStatus;
  latency_ms: number;
  message: string;
}

export interface RollupPoint {
  start: number;
  up: number;
  degraded: number;
  down: number;
  latency_avg: number | null;
  latency_p95: number | null;
  uptime: number | null;
}

export type History = {
  from: number;
  to: number;
  uptime: number | null;
} & ({ level: 'raw'; points: RawPoint[] } | { level: 'hour' | 'day'; points: RollupPoint[] });

export interface MonitorEvent {
  id: number;
  at: number;
  kind: 'state' | 'version' | 'cert';
  from: string;
  to: string;
  reason: string;
}

export type Schedule =
  | { type: 'once'; start: number; end: number }
  | { type: 'weekly'; weekdays: number[]; time: string; duration_min: number; timezone: string };

export interface MaintenanceInput {
  title: string;
  description: string;
  schedule: Schedule;
  monitors: number[];
  groups: string[];
  active: boolean;
}

export interface Maintenance extends MaintenanceInput {
  id: number;
}

/** What the monitor form needs from `GET /api/satellites`. */
export interface SatelliteRef {
  id: string;
  name: string;
  status?: string;
}

export interface Healthz {
  status?: string;
  version?: string;
}
