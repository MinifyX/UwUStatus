// Numbers and times for the UI. Times from the server are unix seconds.

import type { Lang } from './i18n';

const locale = (lang: Lang) => (lang === 'de' ? 'de-DE' : 'en-GB');

export function formatLatency(ms: number | null | undefined, lang: Lang): string {
  if (ms === null || ms === undefined) return '–';
  if (ms < 1000) return `${Math.round(ms)} ms`;
  return `${new Intl.NumberFormat(locale(lang), { maximumFractionDigits: 1 }).format(ms / 1000)} s`;
}

/** 99.95 → "99,95 %"; never rounds up to 100 unless it is 100. */
export function formatUptime(percent: number | null | undefined, lang: Lang): string {
  if (percent === null || percent === undefined) return '–';
  const floored = Math.floor(percent * 100) / 100;
  return `${new Intl.NumberFormat(locale(lang), { maximumFractionDigits: 2 }).format(floored)} %`;
}

/** "vor 3 Minuten" / "3 minutes ago". */
export function formatAgo(atSeconds: number, nowMs: number, lang: Lang): string {
  const diff = Math.max(0, Math.round(nowMs / 1000 - atSeconds));
  const rtf = new Intl.RelativeTimeFormat(locale(lang), { numeric: 'auto', style: 'long' });
  if (diff < 45) return rtf.format(0, 'second');
  if (diff < 3600) return rtf.format(-Math.round(diff / 60), 'minute');
  if (diff < 86_400) return rtf.format(-Math.round(diff / 3600), 'hour');
  return rtf.format(-Math.round(diff / 86_400), 'day');
}

/** A span in seconds, two biggest units: "2 Std. 5 Min." / "2 h 5 min". */
export function formatDuration(seconds: number, lang: Lang): string {
  const s = Math.max(0, Math.round(seconds));
  const units: [number, string, string][] = [
    [86_400, 'T.', 'd'],
    [3600, 'Std.', 'h'],
    [60, 'Min.', 'min'],
    [1, 'Sek.', 's'],
  ];
  const parts: string[] = [];
  let rest = s;
  for (const [size, de, en] of units) {
    const n = Math.floor(rest / size);
    if (n > 0 || (size === 1 && parts.length === 0)) {
      parts.push(`${n} ${lang === 'de' ? de : en}`);
      rest -= n * size;
    }
    if (parts.length === 2) break;
  }
  return parts.join(' ');
}

export function formatDateTime(atSeconds: number, lang: Lang): string {
  return new Intl.DateTimeFormat(locale(lang), { dateStyle: 'medium', timeStyle: 'short' }).format(
    atSeconds * 1000,
  );
}

/** Axis label: time for short ranges, date for long ones. */
export function formatTick(atSeconds: number, spanSeconds: number, lang: Lang): string {
  const opts: Intl.DateTimeFormatOptions =
    spanSeconds <= 2 * 86_400
      ? { hour: '2-digit', minute: '2-digit' }
      : { day: 'numeric', month: 'short' };
  return new Intl.DateTimeFormat(locale(lang), opts).format(atSeconds * 1000);
}
