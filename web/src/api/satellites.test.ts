import { expect, it } from 'vitest';
import { tunnelState, type Satellite } from './satellites';

const sat = (
  live: Partial<Satellite['live']>,
  status: Satellite['status'] = 'paired',
): Satellite => ({
  id: 'fra',
  name: 'Fra',
  region: '',
  status,
  created: 0,
  last_seen: null,
  version: null,
  fingerprint: null,
  live: { connected: false, since: null, error: null, last_result: null, ...live },
});

it('derives the tunnel state', () => {
  expect(tunnelState(sat({ connected: true, error: 'old' }))).toBe('online');
  expect(tunnelState(sat({ error: 'refused' }))).toBe('error');
  expect(tunnelState(sat({}))).toBe('connecting');
  expect(tunnelState(sat({}, 'pending'))).toBe('offline');
});
