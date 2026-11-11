// Theme/contrast/motion, kept per browser in localStorage (the same key the boot script reads).

import { useEffect, useState } from 'react';
import type { Appearance } from '@uwusuite/design';

export const APPEARANCE_KEY = 'uwustatus.appearance';
const EVENT = 'uwustatus:appearance';

export function readAppearance(): Appearance {
  try {
    const raw = localStorage.getItem(APPEARANCE_KEY);
    const parsed = raw ? (JSON.parse(raw) as Partial<Appearance>) : {};
    return {
      theme: parsed.theme ?? 'system',
      contrast: parsed.contrast ?? 'system',
      motion: parsed.motion ?? 'system',
    };
  } catch {
    return { theme: 'system', contrast: 'system', motion: 'system' };
  }
}

export function writeAppearance(next: Appearance): void {
  try {
    localStorage.setItem(APPEARANCE_KEY, JSON.stringify(next));
  } catch {
    // Not stored; still applied for this page.
  }
  window.dispatchEvent(new CustomEvent(EVENT, { detail: next }));
}

export function useStoredAppearance(): Appearance {
  const [value, setValue] = useState(readAppearance);
  useEffect(() => {
    const onChange = (e: Event) => setValue((e as CustomEvent<Appearance>).detail);
    window.addEventListener(EVENT, onChange);
    return () => window.removeEventListener(EVENT, onChange);
  }, []);
  return value;
}
