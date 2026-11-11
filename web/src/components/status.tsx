// Monitor states: always colour + icon + text, never colour alone.

import { Icon, ICONS, Tag, type TagTone } from '@uwusuite/design';
import type { LucideIcon } from 'lucide-react';
import type { MonState, OverviewItem, ResultStatus } from '../api/types';
import { useT, type Key } from '../lib/i18n';

export const STATES: MonState[] = ['up', 'degraded', 'down', 'maintenance', 'paused', 'pending'];

export const STATE: Record<MonState, { tone: TagTone; icon: LucideIcon; label: Key; dot: string }> =
  {
    up: { tone: 'success', icon: ICONS.success, label: 'state.up', dot: 'bg-success' },
    degraded: { tone: 'warning', icon: ICONS.warning, label: 'state.degraded', dot: 'bg-warning' },
    down: { tone: 'danger', icon: ICONS.error, label: 'state.down', dot: 'bg-danger' },
    maintenance: {
      tone: 'neutral',
      icon: ICONS.calendar,
      label: 'state.maintenance',
      dot: 'bg-account-violet',
    },
    paused: { tone: 'neutral', icon: ICONS.stop, label: 'state.paused', dot: 'bg-offline' },
    pending: { tone: 'neutral', icon: ICONS.pending, label: 'state.pending', dot: 'bg-offline' },
  };

/** `location_state.state` and result statuses use the same words; unknown ones count as pending. */
export const asState = (s: string): MonState => (s in STATE ? (s as MonState) : 'pending');

export function StateTag({ state, className }: { state: MonState; className?: string }) {
  const { t } = useT();
  const s = STATE[state];
  return (
    <Tag tone={s.tone} className={`inline-flex items-center gap-1 ${className ?? ''}`}>
      <Icon icon={s.icon} size="xs" aria-hidden />
      {t(s.label)}
    </Tag>
  );
}

const BEAT: Record<ResultStatus, { fill: string; height: number }> = {
  up: { fill: 'fill-success', height: 20 },
  degraded: { fill: 'fill-warning', height: 14 },
  down: { fill: 'fill-danger', height: 20 },
};

/** The last results as bars; down bars get a gap mark on top so they differ by shape too. */
export function Heartbeat({
  beats,
  slots = 30,
}: {
  beats: OverviewItem['recent'];
  slots?: number;
}) {
  const { t } = useT();
  const shown = beats.slice(-slots);
  const count = (s: ResultStatus) => shown.filter((b) => b.status === s).length;
  const label = shown.length
    ? t('beat.label', {
        n: shown.length,
        up: count('up'),
        degraded: count('degraded'),
        down: count('down'),
      })
    : t('beat.none');
  const pad = slots - shown.length;
  return (
    <svg
      viewBox={`0 0 ${slots * 6} 20`}
      className="block h-5 w-full max-w-[180px]"
      preserveAspectRatio="none"
      role="img"
      aria-label={label}
    >
      <title>{label}</title>
      {Array.from({ length: pad }, (_, i) => (
        <rect key={`e${i}`} x={i * 6} y={8} width={4} height={4} rx={1} className="fill-hairline" />
      ))}
      {shown.map((b, i) => {
        const beat = BEAT[b.status] ?? BEAT.up;
        return (
          <g key={`${b.at}-${i}`}>
            <rect
              x={(pad + i) * 6}
              y={20 - beat.height}
              width={4}
              height={beat.height}
              rx={1}
              className={beat.fill}
            />
            {b.status === 'down' && (
              <rect x={(pad + i) * 6} y={6} width={4} height={2} className="fill-surface" />
            )}
          </g>
        );
      })}
    </svg>
  );
}
