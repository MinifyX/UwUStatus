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
