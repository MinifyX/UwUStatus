// Data hooks: load a resource, reload it, run an action with a busy flag and an error toast.

import { useCallback, useEffect, useRef, useState } from 'react';
import { createToasts } from '@uwusuite/design';

export const toasts = createToasts();

export function toastError(error: unknown): void {
  toasts.show(error instanceof Error ? error.message : String(error), { tone: 'error' });
}

export interface Resource<T> {
  data: T | undefined;
  error: Error | undefined;
  loading: boolean;
  reload: () => void;
  set: (value: T) => void;
}

/** Loads `load()` and again whenever `deps` change; `reload()` refetches without clearing. */
export function useResource<T>(load: () => Promise<T>, deps: readonly unknown[]): Resource<T> {
  const [data, setData] = useState<T>();
  const [error, setError] = useState<Error>();
  const [loading, setLoading] = useState(true);
  const [tick, setTick] = useState(0);
  const loadRef = useRef(load);
  loadRef.current = load;

  useEffect(() => {
    let active = true;
    setLoading(true);
    loadRef.current().then(
      (value) => {
        if (!active) return;
        setData(value);
        setError(undefined);
        setLoading(false);
      },
      (reason: unknown) => {
        if (!active) return;
        setError(reason instanceof Error ? reason : new Error(String(reason)));
        setLoading(false);
      },
    );
    return () => {
      active = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...deps, tick]);

  const reload = useCallback(() => setTick((t) => t + 1), []);
  return { data, error, loading, reload, set: setData };
}

/** Re-renders every `ms` (for "vor 3 Min." and the elevation clock). */
export function useNow(ms = 30_000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), ms);
    return () => clearInterval(timer);
  }, [ms]);
  return now;
}

/** Runs an action with a busy key and an error toast. */
export function useAction() {
  const [busy, setBusy] = useState<string | null>(null);
  const run = useCallback(
    async <T>(key: string, action: () => Promise<T>, success?: string): Promise<T | undefined> => {
      setBusy(key);
      try {
        const result = await action();
        if (success) toasts.show(success, { tone: 'success' });
        return result;
      } catch (error) {
        toastError(error);
        return undefined;
      } finally {
        setBusy(null);
      }
    },
    [],
  );
  return { busy, run };
}
