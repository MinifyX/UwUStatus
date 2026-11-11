import { expect, it } from 'vitest';
import { errorFrom, withQuery } from './client';

it('builds queries and reads error bodies', () => {
  expect(withQuery('/api/auth/login', { next: '/settings', x: undefined })).toBe(
    '/api/auth/login?next=%2Fsettings',
  );
  const e = errorFrom(429, { error: 'rate_limited', message: 'Zu viele.' });
  expect([e.status, e.code, e.message]).toEqual([429, 'rate_limited', 'Zu viele.']);
  expect(errorFrom(500, null).message).toBe('Fehler 500');
});
