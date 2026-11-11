import { expect, it } from 'vitest';
import { formatAgo, formatDuration, formatLatency, formatUptime } from './format';

it('formats latency and uptime', () => {
  expect(formatLatency(null, 'de')).toBe('–');
  expect(formatLatency(42.4, 'de')).toBe('42 ms');
  expect(formatLatency(1234, 'de')).toBe('1,2 s');
  expect(formatLatency(1234, 'en')).toBe('1.2 s');
  expect(formatUptime(99.999, 'de')).toBe('99,99 %');
  expect(formatUptime(100, 'en')).toBe('100 %');
  expect(formatUptime(null, 'en')).toBe('–');
});

it('formats relative times and spans', () => {
  const now = 1_767_225_600_000;
  const at = now / 1000;
  expect(formatAgo(at - 10, now, 'en')).toBe('now');
  expect(formatAgo(at - 180, now, 'en')).toBe('3 minutes ago');
  expect(formatAgo(at - 2 * 3600, now, 'de')).toBe('vor 2 Stunden');
  expect(formatAgo(at + 30, now, 'en')).toBe('now');
  expect(formatDuration(0, 'de')).toBe('0 Sek.');
  expect(formatDuration(3 * 3600 + 5 * 60 + 9, 'de')).toBe('3 Std. 5 Min.');
  expect(formatDuration(86_400 + 30, 'en')).toBe('1 d 30 s');
});
