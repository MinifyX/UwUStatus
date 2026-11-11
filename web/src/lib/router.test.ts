import { expect, it } from 'vitest';
import { hrefOf, parseRoute } from './router';

it('parses and builds routes', () => {
  expect(parseRoute('/')).toEqual({ name: 'overview' });
  expect(parseRoute('/settings')).toEqual({ name: 'settings' });
  expect(parseRoute('/settings/x')).toEqual({ name: 'notFound' });
  expect(hrefOf({ name: 'settings' })).toBe('/settings');
  expect(parseRoute('/pages')).toEqual({ name: 'pages' });
  expect(parseRoute('/pages/new')).toEqual({ name: 'pages', id: 'new' });
  expect(parseRoute('/pages/12')).toEqual({ name: 'pages', id: 12 });
  expect(parseRoute('/pages/0')).toEqual({ name: 'notFound' });
  expect(parseRoute('/incidents/abc')).toEqual({ name: 'notFound' });
  expect(parseRoute('/incidents/3')).toEqual({ name: 'incidents', id: 3 });
  expect(parseRoute('/api-tokens')).toEqual({ name: 'apiTokens' });
  expect(hrefOf({ name: 'pages', id: 'new' })).toBe('/pages/new');
  expect(hrefOf({ name: 'incidents', id: 3 })).toBe('/incidents/3');
});
