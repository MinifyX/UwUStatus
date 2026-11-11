import { expect, it } from 'vitest';
import type { Monitor } from '../api/types';
import { draftFrom, emptyDraft, pushCurl, validate, withType } from './monitorForm';

it('requires the fields of the chosen type', () => {
  const { errors, config } = validate(emptyDraft('http'));
  expect(config).toBeUndefined();
  expect(errors.name?.key).toBe('form.err.required');
  expect(errors.url?.key).toBe('form.err.url');
  const tcp = validate({ ...emptyDraft('tcp'), name: 'x', host: '', port: '70000' }).errors;
  expect([tcp.host?.key, tcp.port?.key]).toEqual(['form.err.required', 'form.err.range']);
});

it('checks ranges like the server', () => {
  const base = { ...emptyDraft('ping'), name: 'Router', host: '192.0.2.1' };
  expect(validate({ ...base, interval: '5' }).errors.interval?.key).toBe('form.err.rangeOrZero');
  expect(validate({ ...base, interval: '0' }).config?.interval_s).toBe(0);
  expect(validate({ ...base, interval: '30', timeout: '60' }).errors.timeout?.key).toBe(
    'form.err.timeoutInterval',
  );
  expect(validate({ ...base, retries: '11' }).errors.retries).toBeDefined();
  expect(validate({ ...base, tags: Array(21).fill('a').join(',') }).errors.tags?.key).toBe(
    'form.err.tooMany',
  );
  expect(validate({ ...base, locations: [] }).errors.locations?.key).toBe('form.err.location');
  const push = { ...emptyDraft('push'), name: 'Backup', locations: ['local', 'sat1'] };
  expect(validate(push).errors.locations?.key).toBe('form.err.pushLocal');
  expect(validate(withType(push, 'push')).config?.kind).toBe('push');
});

it('builds the config and keeps hidden check fields on edit', () => {
  const m: Monitor = {
    id: 1,
    push_token: null,
    created: 0,
    updated: 0,
    name: 'Web',
    description: '',
    group: 'Mail',
    tags: ['prod'],
    kind: 'check',
    check: {
      type: 'http',
      url: 'https://mail.example.com/',
      headers: { 'x-probe': '1' },
      accepted_status: ['200-299'],
      timeout_ms: 10_000,
    },
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
  };
  const { config, errors } = validate({ ...draftFrom(m), keyword: 'UwU' });
  expect(errors).toEqual({});
  expect(config?.check).toMatchObject({
    type: 'http',
    url: 'https://mail.example.com/',
    headers: { 'x-probe': '1' },
    keyword: 'UwU',
  });
  expect(config?.check).not.toHaveProperty('timeout_ms');
  expect(config).not.toHaveProperty('id');
  expect(validate({ ...draftFrom(m), accepted: '200-299, 3xx' }).errors.accepted).toBeDefined();
  expect(pushCurl('https://status.example.com', 'abc')).toContain(
    'https://status.example.com/api/push/abc?status=up',
  );
});
