// Calls for status pages, incidents, API tokens and badges, plus the editor's pure helpers.

import { errorFrom, request, withQuery } from './client';
import type {
  ApiToken,
  Badge,
  Incident,
  IncidentStatus,
  Impact,
  MonitorChoice,
  NewIncident,
  PageInput,
  Scope,
  StatusPage,
} from './pages-types';

export const LOGO_TYPES = ['image/png', 'image/jpeg', 'image/webp', 'image/svg+xml'];
export const LOGO_MAX_BYTES = 60 * 1024;

/** The logo goes up as the raw file, not JSON. */
async function uploadLogo(id: number, file: File): Promise<void> {
  let response: Response;
  try {
    response = await fetch(`/api/pages/${id}/logo`, {
      method: 'PUT',
      credentials: 'same-origin',
      headers: { 'content-type': file.type, accept: 'application/json' },
      body: file,
    });
  } catch {
    throw errorFrom(0, null);
  }
  if (response.ok) return;
  const body: unknown = await response.json().catch(() => undefined);
  throw errorFrom(response.status, body);
}

export const pagesApi = {
  monitors: () => request<MonitorChoice[]>('GET', '/api/monitors'),

  pages: () => request<StatusPage[]>('GET', '/api/pages'),
  page: (id: number) => request<StatusPage>('GET', `/api/pages/${id}`),
  createPage: (input: PageInput) => request<StatusPage>('POST', '/api/pages', input),
  savePage: (id: number, input: PageInput) => request<StatusPage>('PUT', `/api/pages/${id}`, input),
  deletePage: (id: number) => request<void>('DELETE', `/api/pages/${id}`),
  uploadLogo,
  deleteLogo: (id: number) => request<void>('DELETE', `/api/pages/${id}/logo`),

  incidents: (status: 'open' | 'all', lang: string) =>
    request<Incident[]>('GET', withQuery('/api/incidents', { status, lang })),
  incident: (id: number, lang: string) =>
    request<Incident>('GET', withQuery(`/api/incidents/${id}`, { lang })),
  createIncident: (input: NewIncident) => request<Incident>('POST', '/api/incidents', input),
  saveIncident: (id: number, patch: { title?: string; impact?: Impact; monitors?: number[] }) =>
    request<Incident>('PUT', `/api/incidents/${id}`, patch),
  deleteIncident: (id: number) => request<void>('DELETE', `/api/incidents/${id}`),
  addUpdate: (id: number, status: IncidentStatus, body: string) =>
    request<Incident>('POST', `/api/incidents/${id}/updates`, { status, body }),
  editUpdate: (id: number, uid: number, body: string) =>
    request<Incident>('PUT', `/api/incidents/${id}/updates/${uid}`, { body }),

  tokens: () => request<ApiToken[]>('GET', '/api/tokens'),
  createToken: (name: string, scopes: Scope[]) =>
    request<ApiToken & { token: string }>('POST', '/api/tokens', { name, scopes }),
  deleteToken: (id: number) => request<void>('DELETE', `/api/tokens/${id}`),

  badge: (id: number) => request<Badge>('GET', `/api/monitors/${id}/badge`),
  saveBadge: (id: number, badge_public: boolean) =>
    request<Badge>('PUT', `/api/monitors/${id}/badge`, { badge_public }),
};

export const SLUG_RE = /^[a-z0-9-]{1,50}$/;

/** One host name per line or comma, trimmed, lower-cased, no empties or duplicates. */
export function splitDomains(text: string): string[] {
  const hosts = text
    .split(/[\s,]+/)
    .map((h) => h.trim().toLowerCase())
    .filter(Boolean);
  return [...new Set(hosts)];
}

/** A copy of `list` with item `i` moved by `by` places; unchanged when that leaves the list. */
export function moveItem<T>(list: readonly T[], i: number, by: -1 | 1): T[] {
  const j = i + by;
  const copy = [...list];
  if (i < 0 || i >= list.length || j < 0 || j >= list.length) return copy;
  copy.splice(j, 0, ...copy.splice(i, 1));
  return copy;
}

export const emptyPage = (): PageInput => ({
  slug: '',
  title: '',
  description: '',
  domains: [],
  lang: 'de',
  auto_lang: true,
  accent: 'pink',
  nyu: true,
  timezone: 'Europe/Berlin',
  groups: [],
});

/** Unix seconds as a short local date and time. */
export const when = (s: number, lang: string) =>
  new Date(s * 1000).toLocaleString(lang, { dateStyle: 'medium', timeStyle: 'short' });
