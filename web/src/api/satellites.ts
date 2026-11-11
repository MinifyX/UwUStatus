// Satellites (pairing by code, live tunnel state) and the dead man's switch.

import { request } from './client';

export interface Satellite {
  /** The location id monitors use. */
  id: string;
  name: string;
  region: string;
  status: 'pending' | 'paired';
  created: number;
  last_seen: number | null;
  version: string | null;
  fingerprint: string | null;
  live: {
    connected: boolean;
    since: number | null;
    error: string | null;
    last_result: number | null;
  };
}

export interface DeadMan {
  /** 0 = off. */
  after_min: number;
  channel_ids: number[];
  text_de: string;
  text_en: string;
  page_enabled: boolean;
  page_title: string;
  page_domain: string;
  page_monitors: number[];
}

export const satellitesApi = {
  list: () => request<Satellite[]>('GET', '/api/satellites'),
  create: (code: string, name: string, region: string) =>
    request<Satellite>('POST', '/api/satellites', { code, name, region }),
  save: (id: string, name: string, region: string) =>
    request<Satellite>('PUT', `/api/satellites/${encodeURIComponent(id)}`, { name, region }),
  remove: (id: string) => request<void>('DELETE', `/api/satellites/${encodeURIComponent(id)}`),
  deadMan: () => request<DeadMan>('GET', '/api/satellites/dead-man'),
  saveDeadMan: (d: DeadMan) => request<DeadMan>('PUT', '/api/satellites/dead-man', d),
};

/** online / connecting (paired, tunnel down) / offline (never paired yet) / error. */
export function tunnelState(s: Satellite): 'online' | 'connecting' | 'offline' | 'error' {
  if (s.live.connected) return 'online';
  if (s.live.error) return 'error';
  return s.status === 'paired' ? 'connecting' : 'offline';
}
