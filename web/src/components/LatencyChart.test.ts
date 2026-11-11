import { expect, it } from 'vitest';
import type { History } from '../api/types';
import { pathOf, seriesOf } from './LatencyChart';

it('splits raw points per location and breaks lines where down', () => {
  const h: History = {
    level: 'raw',
    from: 0,
    to: 100,
    uptime: 50,
    points: [
      { at: 10, location: 'local', status: 'up', latency_ms: 20, message: '' },
      { at: 20, location: 'sat', status: 'up', latency_ms: 40, message: '' },
      { at: 30, location: 'local', status: 'down', latency_ms: 0, message: '' },
      { at: 40, location: 'local', status: 'up', latency_ms: 30, message: '' },
    ],
  };
  const { series, downs, values } = seriesOf(h);
  expect(series.map((s) => s.name)).toEqual(['local', 'sat']);
  expect(downs).toEqual([30]);
  expect(values).toEqual([20, 40, 30]);
  expect(
    pathOf(
      series[0]!.points,
      (v) => v,
      (v) => v,
    ),
  ).toBe('M10.0 20.0M40.0 30.0');
});
