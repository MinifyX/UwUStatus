// The monitor form as plain strings, its validation (the server's rules, so errors show at the
// field before a round trip) and the conversion to the server's MonitorConfig.

import type { Check, CheckType, MailTls, Monitor, MonitorConfig, Quorum } from '../api/types';
import type { Key } from './i18n';

export type FormType = CheckType | 'push';
export const FORM_TYPES: FormType[] = [
  'http',
  'tcp',
  'ping',
  'dns',
  'smtp',
  'imap',
  'docker',
  'healthz',
  'push',
];

export interface MonitorDraft {
  type: FormType;
  name: string;
  description: string;
  group: string;
  tags: string;
  // Check fields; only the ones of `type` are used.
  url: string;
  method: string;
  accepted: string;
  keyword: string;
  keywordAbsent: boolean;
  jsonPath: string;
  jsonValue: string;
  followRedirects: boolean;
  ignoreTls: boolean;
  host: string;
  port: string;
  dnsName: string;
  recordType: string;
  resolver: string;
  expected: string;
  tls: MailTls;
  container: string;
  // Timing and behaviour.
  interval: string;
  timeout: string;
  retries: string;
  retryInterval: string;
  reminder: string;
  certWarn: string;
  parentId: string;
  locations: string[];
  quorum: Quorum;
  muted: boolean;
  paused: boolean;
  /** The check as loaded, so fields the form does not show (headers, body) survive an edit. */
  base: Check | null;
}

export type DraftField = keyof MonitorDraft;
export interface FieldError {
  key: Key;
  vars?: Record<string, string | number>;
}
export type Errors = Partial<Record<DraftField, FieldError>>;

const DEFAULT_PORTS: Partial<Record<FormType, string>> = { tcp: '443', smtp: '587', imap: '993' };

export function emptyDraft(type: FormType = 'http'): MonitorDraft {
  return {
    type,
    name: '',
    description: '',
    group: '',
    tags: '',
    url: '',
    method: 'GET',
    accepted: '',
    keyword: '',
    keywordAbsent: false,
    jsonPath: '',
    jsonValue: '',
    followRedirects: true,
    ignoreTls: false,
    host: '',
    port: DEFAULT_PORTS[type] ?? '',
    dnsName: '',
    recordType: 'A',
    resolver: '',
    expected: '',
    tls: type === 'smtp' ? 'start_tls' : type === 'imap' ? 'tls' : 'none',
    container: '',
    interval: '0',
    timeout: '10',
    retries: '0',
    retryInterval: '0',
    reminder: '0',
    certWarn: '14',
    parentId: '',
    locations: ['local'],
    quorum: 'any',
    muted: false,
    paused: false,
    base: null,
  };
}

/** Switching the type keeps what fits and resets what does not (ports, TLS). */
export function withType(d: MonitorDraft, type: FormType): MonitorDraft {
  const fresh = emptyDraft(type);
  return {
    ...d,
    type,
    port: DEFAULT_PORTS[d.type] === d.port || !d.port ? fresh.port : d.port,
    tls: fresh.tls,
    locations: type === 'push' ? ['local'] : d.locations,
  };
}

export function draftFrom(m: Monitor): MonitorDraft {
  const d: MonitorDraft = {
    ...emptyDraft(m.kind === 'push' ? 'push' : (m.check?.type ?? 'http')),
    name: m.name,
    description: m.description,
    group: m.group,
    tags: m.tags.join(', '),
    interval: String(m.interval_s),
    timeout: String(m.timeout_s),
    retries: String(m.retries),
    retryInterval: String(m.retry_interval_s),
    reminder: String(m.reminder_min),
    certWarn: String(m.cert_warn_days),
    parentId: m.parent_id === null ? '' : String(m.parent_id),
    locations: m.locations,
    quorum: m.quorum,
    muted: m.muted,
    paused: m.paused,
  };
  const c = m.kind === 'check' ? m.check : null;
  if (!c) return d;
  // timeout_ms is derived from timeout_s by the server.
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  const { timeout_ms, ...base } = c;
  d.base = base as Check;
  switch (c.type) {
    case 'http':
      return {
        ...d,
        url: c.url,
        method: c.method ?? 'GET',
        accepted: (c.accepted_status ?? []).join(', '),
        keyword: c.keyword ?? '',
        keywordAbsent: c.keyword_absent ?? false,
        jsonPath: c.json_path?.path ?? '',
        jsonValue: c.json_path?.value ?? '',
        followRedirects: c.follow_redirects ?? true,
        ignoreTls: c.ignore_tls_errors ?? false,
      };
    case 'tcp':
      return { ...d, host: c.host, port: String(c.port) };
    case 'ping':
      return { ...d, host: c.host };
    case 'dns':
      return {
        ...d,
        dnsName: c.name,
        recordType: c.record_type ?? 'A',
        resolver: c.resolver ?? '',
        expected: (c.expected ?? []).join(', '),
      };
    case 'smtp':
    case 'imap':
      return {
        ...d,
        host: c.host,
        port: String(c.port),
        tls: c.tls ?? 'none',
        ignoreTls: c.ignore_tls_errors ?? false,
      };
    case 'docker':
      return { ...d, url: c.url, container: c.container };
    case 'healthz':
      return { ...d, url: c.url };
  }
}

export const list = (text: string) =>
  text
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);

const HTTP_URL = /^https?:\/\/[^\s/?#]+/i;
const STATUS_RANGE = /^[1-5]\d\d(-[1-5]\d\d)?$/;

/** Checks the draft like the server does; `config` is set when there are no errors. */
export function validate(d: MonitorDraft): { errors: Errors; config?: MonitorConfig } {
  const errors: Errors = {};
  const req = (field: DraftField, value: string) => {
    if (!value.trim()) errors[field] = { key: 'form.err.required' };
  };
  const max = (field: DraftField, value: string, n: number) => {
    if (value.trim().length > n) errors[field] = { key: 'form.err.tooLong', vars: { max: n } };
  };
  const num = (field: DraftField, value: string, min: number, maxN: number, zeroOk = false) => {
    const v = value.trim();
    const n = Number(v);
    if (!/^\d+$/.test(v) || (!(zeroOk && n === 0) && (n < min || n > maxN))) {
      errors[field] = zeroOk
        ? { key: 'form.err.rangeOrZero', vars: { min, max: maxN } }
        : { key: 'form.err.range', vars: { min, max: maxN } };
      return 0;
    }
    return n;
  };

  req('name', d.name);
  max('name', d.name, 100);
  max('description', d.description, 2000);
  max('group', d.group, 100);
  const tags = list(d.tags);
  if (tags.length > 20) errors.tags = { key: 'form.err.tooMany', vars: { max: 20 } };
  else if (tags.some((t) => t.length > 50)) errors.tags = { key: 'form.err.tagLong' };

  const push = d.type === 'push';
  const interval = num('interval', d.interval, 10, 86_400, true);
  const timeout = push ? num('timeout', d.timeout, 0, 86_400) : num('timeout', d.timeout, 1, 300);
  if (!push && !errors.timeout && interval && timeout > interval)
    errors.timeout = { key: 'form.err.timeoutInterval' };
  const retries = num('retries', d.retries, 0, 10);
  const retryInterval = num('retryInterval', d.retryInterval, 10, 86_400, true);
  const reminder = num('reminder', d.reminder, 0, 10_080);
  const certWarn = num('certWarn', d.certWarn, 0, 365);

  if (d.locations.length === 0) errors.locations = { key: 'form.err.location' };
  else if (push && (d.locations.length !== 1 || d.locations[0] !== 'local'))
    errors.locations = { key: 'form.err.pushLocal' };

  let check: Check | null = null;
  const port = () => num('port', d.port, 1, 65_535);
  const url = () => {
    if (!HTTP_URL.test(d.url.trim())) errors.url = { key: 'form.err.url' };
    return d.url.trim();
  };
  switch (d.type) {
    case 'http': {
      const accepted = list(d.accepted);
      if (accepted.some((a) => !STATUS_RANGE.test(a))) errors.accepted = { key: 'form.err.status' };
      if (d.jsonPath.trim() && !d.jsonValue.trim()) errors.jsonValue = { key: 'form.err.required' };
      const base = d.base?.type === 'http' ? d.base : null;
      check = {
        ...base,
        type: 'http',
        url: url(),
        method: d.method,
        accepted_status: accepted,
        keyword: d.keyword.trim() || null,
        keyword_absent: d.keywordAbsent,
        json_path: d.jsonPath.trim()
          ? { path: d.jsonPath.trim(), value: d.jsonValue.trim() }
          : null,
        follow_redirects: d.followRedirects,
        ignore_tls_errors: d.ignoreTls,
      };
      break;
    }
    case 'tcp':
      req('host', d.host);
      check = { type: 'tcp', host: d.host.trim(), port: port() };
      break;
    case 'ping':
      req('host', d.host);
      check = { type: 'ping', host: d.host.trim() };
      break;
    case 'dns':
      req('dnsName', d.dnsName);
      check = {
        type: 'dns',
        name: d.dnsName.trim(),
        record_type: d.recordType,
        resolver: d.resolver.trim() || null,
        expected: list(d.expected),
      };
      break;
    case 'smtp':
    case 'imap':
      req('host', d.host);
      check = {
        type: d.type,
        host: d.host.trim(),
        port: port(),
        tls: d.tls,
        ignore_tls_errors: d.ignoreTls,
      };
      break;
    case 'docker':
      req('container', d.container);
      max('container', d.container, 200);
      check = { type: 'docker', url: url(), container: d.container.trim() };
      break;
    case 'healthz':
      check = { type: 'healthz', url: url() };
      break;
    case 'push':
      break;
  }

  if (Object.keys(errors).length) return { errors };
  return {
    errors,
    config: {
      name: d.name.trim(),
      description: d.description.trim(),
      group: d.group.trim(),
      tags,
      kind: push ? 'push' : 'check',
      check,
      interval_s: interval,
      timeout_s: timeout,
      retries,
      retry_interval_s: retryInterval,
      reminder_min: reminder,
      muted: d.muted,
      paused: d.paused,
      parent_id: d.parentId ? Number(d.parentId) : null,
      locations: d.locations,
      quorum: d.quorum,
      cert_warn_days: certWarn,
    },
  };
}

/** The URL a push monitor's service calls. */
export function pushUrl(origin: string, token: string): string {
  return `${origin}/api/push/${token}`;
}

export function pushCurl(origin: string, token: string): string {
  return `curl -fsS "${pushUrl(origin, token)}?status=up&msg=OK&ping=42"`;
}
