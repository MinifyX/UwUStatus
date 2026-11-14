// Overview: counters per state, search and tag filter, monitors grouped, polled every 10 s.

import { useMemo, useState } from 'react';
import { CountUp, Icon, ICONS, Pill, TextInput } from '@uwusuite/design';
import { api } from '../api/client';
import type { MonState, OverviewItem } from '../api/types';
import { Heartbeat, STATE, StateTag } from '../components/status';
import { ButtonLink, Empty, ErrorBox, Link, Loading, PageHeader } from '../components/ui';
import { formatAgo, formatLatency, formatUptime } from '../lib/format';
import { useNow, useResource } from '../lib/hooks';
import { useT } from '../lib/i18n';
import { filterMonitors, groupMonitors } from '../lib/overview';
import { usePolling } from '../lib/polling';
import { hrefOf } from '../lib/router';

const COUNTED: MonState[] = ['up', 'degraded', 'down', 'maintenance', 'paused'];

export function Dashboard() {
  const { t } = useT();
  const overview = useResource(api.overview, []);
  usePolling(overview.reload);
  const [query, setQuery] = useState('');
  const [state, setState] = useState<MonState | null>(null);
  const [tags, setTags] = useState<string[]>([]);

  const items = overview.data;
  const allTags = useMemo(
    () => [...new Set((items ?? []).flatMap((m) => m.tags))].sort((a, b) => a.localeCompare(b)),
    [items],
  );
  const shown = useMemo(
    () => groupMonitors(filterMonitors(items ?? [], query, state, tags)),
    [items, query, state, tags],
  );

  const header = (
    <PageHeader
      title={t('nav.overview')}
      actions={
        <ButtonLink href={hrefOf({ name: 'monitorNew' })} variant="primary" icon={ICONS.add}>
          {t('monitor.new')}
        </ButtonLink>
      }
    />
  );
  if (!items) {
    return (
      <>
        {header}
        {overview.error ? <ErrorBox error={overview.error} retry={overview.reload} /> : <Loading />}
      </>
    );
  }
  if (items.length === 0) {
    return (
      <>
        {header}
        <Empty title={t('overview.empty')} body={t('overview.emptyBody')} mood="sleepy" />
      </>
    );
  }

  const count = (s: MonState) => items.filter((m) => m.state === s).length;
  return (
    <>
      {header}
      {overview.error && (
        <div className="mb-4">
          <ErrorBox error={overview.error} retry={overview.reload} />
        </div>
      )}
      <section
        aria-label={t('overview.counts')}
        className="uwu-stagger mb-6 grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-5"
      >
        {COUNTED.map((s) => {
          const active = state === s;
          return (
            <button
              key={s}
              type="button"
              aria-pressed={active}
              onClick={() => setState(active ? null : s)}
              className={`flex min-h-16 items-center gap-3 rounded-card border bg-surface px-4 py-3 text-left focus-visible:shadow-focus focus-visible:outline-none ${
                active ? 'border-pink bg-pink-tint' : 'border-hairline hover:bg-pink-tint/45'
              }`}
            >
              <Icon icon={STATE[s].icon} size="md" className={iconTone(s)} />
              <span className="flex flex-col">
                <CountUp value={count(s)} className="num text-title leading-none font-bold" />
                <span className="text-meta text-muted">{t(STATE[s].label)}</span>
              </span>
            </button>
          );
        })}
      </section>

      <div className="mb-4 flex flex-col gap-3">
        <label className="relative block max-w-md">
          <span className="sr-only">{t('overview.search')}</span>
          <Icon
            icon={ICONS.search}
            size="sm"
            className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-muted"
          />
          <TextInput
            type="search"
            className="pl-9"
            placeholder={t('overview.searchPlaceholder')}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            autoComplete="off"
            spellCheck={false}
          />
        </label>
        {allTags.length > 0 && (
          <div role="group" aria-label={t('overview.tags')} className="flex flex-wrap gap-2">
            {allTags.map((tag) => (
              <Pill
                key={tag}
                active={tags.includes(tag)}
                onClick={() =>
                  setTags(tags.includes(tag) ? tags.filter((x) => x !== tag) : [...tags, tag])
                }
              >
                {tag}
              </Pill>
            ))}
          </div>
        )}
      </div>

      {shown.length === 0 ? (
        <Empty title={t('overview.noMatch')} mood="puzzled" body={t('overview.noMatchBody')} />
      ) : (
        <div className="flex flex-col gap-6">
          {shown.map(([group, monitors]) => (
            <section key={group} aria-labelledby={`g-${group}`}>
              <h2
                id={`g-${group}`}
                className="m-0 mb-2 px-1 text-caption font-semibold tracking-wide text-muted uppercase"
              >
                {group || t('overview.noGroup')} <span className="num">({monitors.length})</span>
              </h2>
              <ul className="m-0 flex list-none flex-col gap-1 rounded-card border border-hairline bg-surface p-1.5">
                {monitors.map((m) => (
                  <MonitorRow key={m.id} m={m} />
                ))}
              </ul>
            </section>
          ))}
        </div>
      )}
    </>
  );
}

export const iconTone = (s: MonState) =>
  s === 'up'
    ? 'text-success-ink'
    : s === 'degraded'
      ? 'text-warning-ink'
      : s === 'down'
        ? 'text-danger-ink'
        : 'text-muted';

function MonitorRow({ m }: { m: OverviewItem }) {
  const { t, lang } = useT();
  const now = useNow(30_000);
  const detail = m.unreachable_via_parent ? t('overview.viaParent') : m.message || m.reason;
  return (
    <li>
      <Link
        href={hrefOf({ name: 'monitor', id: m.id })}
        className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 gap-y-1 rounded-xl px-3 py-2.5 text-ink no-underline hover:bg-pink-tint/45 focus-visible:shadow-focus focus-visible:outline-none md:grid-cols-[minmax(0,1fr)_auto_7rem_9rem_180px]"
      >
        <span className="min-w-0">
          <span className="flex items-center gap-2">
            <span className="truncate font-semibold">{m.name}</span>
            {m.muted && (
              <Icon
                icon={ICONS.notificationsOff}
                size="xs"
                label={t('monitor.muted')}
                className="text-muted"
              />
            )}
          </span>
          <span className="block truncate text-meta text-muted">
            {detail || (m.last_check ? formatAgo(m.last_check, now, lang) : t('state.pending'))}
          </span>
        </span>
        <StateTag state={m.state} />
        <span className="num hidden text-meta md:block">
          <span className="sr-only">{t('monitor.latency')}: </span>
          {formatLatency(m.latency_ms, lang)}
        </span>
        <span className="num col-span-2 whitespace-nowrap text-meta text-muted md:col-span-1 md:text-ink">
          <abbr title={t('monitor.uptime24h')} className="no-underline">
            24h
          </abbr>{' '}
          {formatUptime(m.uptime_24h, lang)}
          {' · '}
          <abbr title={t('monitor.uptime30d')} className="no-underline">
            30d
          </abbr>{' '}
          {formatUptime(m.uptime_30d, lang)}
        </span>
        <span className="col-span-2 md:col-span-1">
          <Heartbeat beats={m.recent} />
        </span>
      </Link>
    </li>
  );
}
