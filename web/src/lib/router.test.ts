import { expect, it } from 'vitest';
import { hrefOf, parseRoute } from './router';

it('parses and builds routes', () => {
  expect(parseRoute('/')).toEqual({ name: 'overview' });
  expect(parseRoute('/settings')).toEqual({ name: 'settings' });
  expect(parseRoute('/settings/x')).toEqual({ name: 'notFound' });
  expect(hrefOf({ name: 'settings' })).toBe('/settings');
});
