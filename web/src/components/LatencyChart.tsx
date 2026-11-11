// Latency over time as a light SVG (no chart package): one line per location, down marks below.
// shortcut: no hover tooltip, the summary line carries the numbers; add one if people ask.

import type { History } from '../api/types';
import { formatLatency, formatTick } from '../lib/format';
import { useT } from '../lib/i18n';

const W = 600;
const H = 160;
const STROKES = [
  'stroke-pink',
  'stroke-account-sky',
  'stroke-account-mint',
  'stroke-account-amber',
];

export interface Series {
  name: string;
  points: { x: number; y: number | null }[];
}

/** Lines per location (raw) or the hourly/daily average, and the times something was down. */
export function seriesOf(h: History): { series: Series[]; downs: number[]; values: number[] } {
  if (h.level === 'raw') {
    const by = new Map<string, Series>();
    // A Set: several locations can fail at the same second.
    const downs = new Set<number>();
    for (const p of h.points) {
      const s = by.get(p.location) ?? { name: p.location, points: [] };
      by.set(p.location, s);
      s.points.push({ x: p.at, y: p.status === 'down' ? null : p.latency_ms });
      if (p.status === 'down') downs.add(p.at);
    }
    for (const s of by.values()) s.points.sort((a, b) => a.x - b.x);
    const values = h.points.filter((p) => p.status !== 'down').map((p) => p.latency_ms);
    return { series: [...by.values()], downs: [...downs].sort((a, b) => a - b), values };
  }
  const points = h.points.map((p) => ({ x: p.start, y: p.latency_avg }));
  return {
    series: [{ name: '', points }],
    downs: h.points.filter((p) => p.down > 0).map((p) => p.start),
    values: points.flatMap((p) => (p.y === null ? [] : [p.y])),
  };
}

/** SVG path with gaps where `y` is null. */
export function pathOf(
  points: Series['points'],
  x: (v: number) => number,
  y: (v: number) => number,
) {
  let d = '';
  let pen = false;
  for (const p of points) {
    if (p.y === null) {
      pen = false;
      continue;
    }
    d += `${pen ? 'L' : 'M'}${x(p.x).toFixed(1)} ${y(p.y).toFixed(1)}`;
    pen = true;
  }
  return d;
}

export function LatencyChart({
  history,
  locationName,
}: {
  history: History;
  locationName: (id: string) => string;
}) {
  const { t, lang } = useT();
  const { series, downs, values } = seriesOf(history);
  const span = Math.max(1, history.to - history.from);
  const top = Math.max(10, ...values) * 1.1;
  const x = (v: number) => ((v - history.from) / span) * W;
  const y = (v: number) => H - 8 - (v / top) * (H - 16);
  const sorted = [...values].sort((a, b) => a - b);
  const avg = values.length ? values.reduce((a, b) => a + b, 0) / values.length : null;
  const p95 = sorted.length
    ? sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * 0.95) - 1)]
    : null;
  const summary = values.length
    ? t('chart.summary', {
        avg: formatLatency(avg, lang),
        p95: formatLatency(p95, lang),
        max: formatLatency(sorted[sorted.length - 1], lang),
        downs: downs.length,
      })
    : t('chart.empty');

  return (
    <figure className="m-0">
      <div className="relative">
        <svg
          viewBox={`0 0 ${W} ${H}`}
          preserveAspectRatio="none"
          className="block h-40 w-full rounded-control bg-canvas"
          role="img"
          aria-label={summary}
        >
          {[0.25, 0.5, 0.75].map((f) => (
            <line
              key={f}
              x1={0}
              x2={W}
              y1={y(top * f)}
              y2={y(top * f)}
              className="stroke-hairline"
              vectorEffect="non-scaling-stroke"
            />
          ))}
          {downs.map((at) => (
            <rect key={at} x={x(at) - 1} y={H - 6} width={3} height={6} className="fill-danger" />
          ))}
          {series.map((s, i) => (
            <path
              key={s.name}
              d={pathOf(s.points, x, y)}
              fill="none"
              strokeWidth={2}
              strokeLinejoin="round"
              strokeLinecap="round"
              vectorEffect="non-scaling-stroke"
              className={STROKES[i % STROKES.length]}
            />
          ))}
        </svg>
        <span
          className="num pointer-events-none absolute top-1 left-2 text-caption text-muted"
          aria-hidden
        >
          {formatLatency(top, lang)}
        </span>
      </div>
      <div className="num mt-1 flex justify-between text-caption text-muted" aria-hidden>
        <span>{formatTick(history.from, span, lang)}</span>
        <span>{formatTick(history.from + span / 2, span, lang)}</span>
        <span>{formatTick(history.to, span, lang)}</span>
      </div>
      <figcaption className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-meta">
        <span className="num">{summary}</span>
        {series.length > 1 &&
          series.map((s, i) => (
            <span key={s.name} className="inline-flex items-center gap-1.5">
              <svg width="16" height="4" aria-hidden className="shrink-0">
                <line
                  x1="0"
                  x2="16"
                  y1="2"
                  y2="2"
                  strokeWidth={3}
                  className={STROKES[i % STROKES.length]}
                />
              </svg>
              {locationName(s.name)}
            </span>
          ))}
        {downs.length > 0 && (
          <span className="inline-flex items-center gap-1.5">
            <svg width="6" height="10" aria-hidden className="shrink-0">
              <rect width="6" height="10" className="fill-danger" />
            </svg>
            {t('chart.down')}
          </span>
        )}
      </figcaption>
    </figure>
  );
}
