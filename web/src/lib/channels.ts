// What each channel type asks for: plain fields (stored in `channel`) and secrets (write-only).
// Mirrors `uwustatus_notify::Channel`; the server rejects unknown fields.

import type { ChannelEvents, ChannelSpec, ChannelType } from '../api/types';

export const CHANNEL_TYPES: ChannelType[] = [
  'email',
  'webhook',
  'ntfy',
  'gotify',
  'pushover',
  'discord',
  'telegram',
];

export interface FieldDef {
  key: 'to' | 'url' | 'server' | 'topic' | 'priority' | 'chat_id';
  kind: 'text' | 'url' | 'number' | 'lines';
  placeholder?: string;
  min?: number;
  max?: number;
  initial?: number;
}

export const FIELDS: Record<ChannelType, FieldDef[]> = {
  email: [{ key: 'to', kind: 'lines', placeholder: 'ops@example.com' }],
  webhook: [{ key: 'url', kind: 'url', placeholder: 'https://hooks.example.com/uwustatus' }],
  ntfy: [
    { key: 'server', kind: 'url', placeholder: 'https://ntfy.example.com' },
    { key: 'topic', kind: 'text', placeholder: 'status' },
    { key: 'priority', kind: 'number', min: 1, max: 5, initial: 4 },
  ],
  gotify: [
    { key: 'url', kind: 'url', placeholder: 'https://gotify.example.com' },
    { key: 'priority', kind: 'number', min: 0, max: 10, initial: 8 },
  ],
  pushover: [{ key: 'priority', kind: 'number', min: -2, max: 1, initial: 0 }],
  discord: [],
  telegram: [{ key: 'chat_id', kind: 'text', placeholder: '-1001234567890' }],
};

export interface SecretDef {
  key: 'hmac_secret' | 'token' | 'user_key' | 'webhook_url' | 'bot_token';
  required: boolean;
}

export const SECRETS: Record<ChannelType, SecretDef[]> = {
  email: [],
  webhook: [{ key: 'hmac_secret', required: false }],
  ntfy: [{ key: 'token', required: false }],
  gotify: [{ key: 'token', required: true }],
  pushover: [
    { key: 'user_key', required: true },
    { key: 'token', required: true },
  ],
  discord: [{ key: 'webhook_url', required: true }],
  telegram: [{ key: 'bot_token', required: true }],
};

export const EVENT_KEYS: (keyof ChannelEvents)[] = [
  'down',
  'degraded',
  'up',
  'reminder',
  'cert',
  'version',
];

export const DEFAULT_EVENTS: ChannelEvents = {
  down: true,
  degraded: true,
  up: true,
  reminder: true,
  cert: true,
  version: false,
};

/** Form values (strings) → the `channel` object for the server, with only this type's fields. */
export function specFrom(type: ChannelType, values: Record<string, string>): ChannelSpec {
  const spec: ChannelSpec = { type };
  for (const field of FIELDS[type]) {
    const value = values[field.key] ?? '';
    if (field.kind === 'number') spec[field.key] = Number(value || field.initial || 0);
    else if (field.kind === 'lines')
      spec[field.key] = value
        .split(/[\n,;]/)
        .map((a) => a.trim())
        .filter(Boolean);
    else spec[field.key] = value.trim();
  }
  return spec;
}

/** The other way, for editing. */
export function valuesFrom(spec: ChannelSpec): Record<string, string> {
  const values: Record<string, string> = {};
  for (const field of FIELDS[spec.type]) {
    const value = spec[field.key];
    values[field.key] = Array.isArray(value) ? value.join('\n') : String(value ?? '');
  }
  return values;
}
