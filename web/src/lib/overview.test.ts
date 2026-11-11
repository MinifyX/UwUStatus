import { expect, it } from 'vitest';
import type { OverviewItem } from '../api/types';
import { filterMonitors, groupMonitors } from './overview';

const item = (
  id: number,
  name: string,
  group: string,
  tags: string[],
  state = 'up',
): OverviewItem => ({ id, name, group, tags, state, message: null }) as OverviewItem;

it('filters and groups', () => {
  const items = [
    item(1, 'Web', 'Mail', ['prod']),
    item(2, 'IMAP', 'Mail', ['prod', 'mail'], 'down'),
    item(3, 'Router', '', []),
    item(4, 'DNS', 'Netz', ['prod']),
  ];
  expect(filterMonitors(items, 'imap', null, []).map((m) => m.id)).toEqual([2]);
  expect(filterMonitors(items, '', 'down', []).map((m) => m.id)).toEqual([2]);
  expect(filterMonitors(items, '', null, ['prod', 'mail']).map((m) => m.id)).toEqual([2]);
  expect(filterMonitors(items, 'netz', null, []).map((m) => m.id)).toEqual([4]);
  expect(groupMonitors(items).map(([g, ms]) => [g, ms.map((m) => m.name)])).toEqual([
    ['Mail', ['IMAP', 'Web']],
    ['Netz', ['DNS']],
    ['', ['Router']],
  ]);
});
