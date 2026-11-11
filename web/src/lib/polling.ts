// Polls every `ms` while the tab is visible; a hidden tab stops, coming back refetches at once.

import { useEffect, useRef } from 'react';

type Doc = Pick<Document, 'hidden' | 'addEventListener' | 'removeEventListener'>;

/** Calls `tick` now and every `ms` while `doc` is visible. Returns the stop function. */
export function startPolling(tick: () => void, ms: number, doc: Doc): () => void {
  let timer: ReturnType<typeof setInterval> | undefined;
  const run = () => {
    clearInterval(timer);
    timer = undefined;
    if (doc.hidden) return;
    tick();
    timer = setInterval(tick, ms);
  };
  doc.addEventListener('visibilitychange', run);
  run();
  return () => {
    clearInterval(timer);
    doc.removeEventListener('visibilitychange', run);
  };
}

/** `tick` is read fresh on every call, so callers need not memoise it. */
export function usePolling(tick: () => void, ms = 10_000, enabled = true): void {
  const ref = useRef(tick);
  ref.current = tick;
  useEffect(() => {
    if (!enabled) return;
    // The first load is the caller's; polling only refreshes.
    let first = true;
    return startPolling(
      () => {
        if (first) first = false;
        else ref.current();
      },
      ms,
      document,
    );
  }, [ms, enabled]);
}
