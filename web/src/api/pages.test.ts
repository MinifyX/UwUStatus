import { expect, it } from 'vitest';
import { moveItem, SLUG_RE, splitDomains } from './pages';

it('splits domains, moves items, checks slugs', () => {
  expect(splitDomains(' Status.Example.com,\nstatus.example.com  b.example.org ')).toEqual([
    'status.example.com',
    'b.example.org',
  ]);
  expect(moveItem(['a', 'b', 'c'], 0, 1)).toEqual(['b', 'a', 'c']);
  expect(moveItem(['a', 'b'], 0, -1)).toEqual(['a', 'b']);
  expect(SLUG_RE.test('my-page-1')).toBe(true);
  expect(SLUG_RE.test('My Page')).toBe(false);
});
