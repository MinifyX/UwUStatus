import { afterEach, expect, it, vi } from 'vitest';
import { startPolling } from './polling';

afterEach(() => vi.useRealTimers());

it('polls while visible, pauses while hidden, refetches on return', () => {
  vi.useFakeTimers();
  const listeners = new Set<() => void>();
  const doc = {
    hidden: false,
    addEventListener: (_: string, l: () => void) => listeners.add(l),
    removeEventListener: (_: string, l: () => void) => listeners.delete(l),
  } as unknown as Document;
  const fire = (hidden: boolean) => {
    (doc as { hidden: boolean }).hidden = hidden;
    listeners.forEach((l) => l());
  };
  const tick = vi.fn();
  const stop = startPolling(tick, 10_000, doc);
  expect(tick).toHaveBeenCalledTimes(1);
  vi.advanceTimersByTime(25_000);
  expect(tick).toHaveBeenCalledTimes(3);
  fire(true);
  vi.advanceTimersByTime(60_000);
  expect(tick).toHaveBeenCalledTimes(3);
  fire(false);
  expect(tick).toHaveBeenCalledTimes(4);
  vi.advanceTimersByTime(10_000);
  expect(tick).toHaveBeenCalledTimes(5);
  stop();
  vi.advanceTimersByTime(60_000);
  expect(tick).toHaveBeenCalledTimes(5);
  expect(listeners.size).toBe(0);
});
