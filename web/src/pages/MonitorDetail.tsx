// One monitor: state per location, latency chart with range switch, events, pause/mute/delete.

import { useState } from 'react';
import { Button, Card, Dialog, Icon, ICONS, Segmented } from '@uwusuite/design';
import { api, configOf } from '../api/client';
import type { HistoryRange, Monitor, MonitorEvent } from '../api/types';
import { LatencyChart } from '../components/LatencyChart';
import { asState, StateTag, STATE } from '../components/status';
import { ButtonLink, Empty, ErrorBox, Loading, PageHeader } from '../components/ui';
import { formatAgo, formatDateTime, formatLatency, formatUptime } from '../lib/format';
import { toasts, useAction, useNow, useResource } from '../lib/hooks';
import { useT } from '../lib/i18n';
import { usePolling } from '../lib/polling';
import { hrefOf, navigate } from '../lib/router';
import { iconTone } from './Dashboard';
import { PushInfo } from './MonitorForm';

const RANGES: HistoryRange[] = ['24h', '7d', '30d', '90d'];

/** `local` or a satellite id → a readable name. */
export function useLocationName() {
  const { t } = useT();
  const satellites = useResource(api.satellites, []);
  return (id: string) =>
    id === 'local'
      ? t('loc.local')
      : (satellites.data?.find((s) => String(s.id) === id)?.name ?? id);
}

export function MonitorDetail({ id }: { id: number }) {
  const { t, lang } = useT();
  const now = useNow();
  const overview = useResource(api.overview, []);
  const monitor = useResource(() => api.monitor(id), [id]);
  const [range, setRange] = useState<HistoryRange>('24h');
  const history = useResource(() => api.history(id, range), [id, range]);
  const locationName = useLocationName();
  const { busy, run } = useAction();
  const [confirm, setConfirm] = useState(false);
  usePolling(() => {
    overview.reload();
    if (range === '24h') history.reload();
  });

  const item = overview.data?.find((m) => m.id === id);
  const m = monitor.data;
  const back = { href: '/', label: t('nav.overview') };

  if (monitor.error && !m) {
    const missing = 'status' in monitor.error && monitor.error.status === 404;
    return (
      <>
        <PageHeader title={t('monitor.title')} back={back} />
        {missing ? (
          <Empty title={t('monitor.missing')} mood="puzzled" />
        ) : (
          <ErrorBox error={monitor.error} retry={monitor.reload} />
        )}
      </>
    );
  }
  if (!m) return <Loading />;

  const toggle = async (field: 'paused' | 'muted') => {
    const next: Monitor | undefined = await run(field, () =>
      api.saveMonitor(id, { ...configOf(m), [field]: !m[field] }),
    );
    if (next) {
      monitor.set(next);
      overview.reload();
      toasts.show(t(`monitor.${field}.${next[field] ? 'on' : 'off'}`), { tone: 'success' });
    }
  };
  const remove = async () => {
    const done = await run('delete', async () => {
      await api.deleteMonitor(id);
      return true;
    });
    if (done) {
      toasts.show(t('monitor.deleted', { name: m.name }), { tone: 'success' });
      navigate('/');
    }
  };

  const state = item?.state ?? 'pending';
  const facts: { label: string; value: string }[] = [
    { label: t('monitor.latency'), value: formatLatency(item?.latency_ms, lang) },
    { label: t('monitor.uptime24h'), value: formatUptime(item?.uptime_24h, lang) },
    { label: t('monitor.uptime30d'), value: formatUptime(item?.uptime_30d, lang) },
    {
      label: t('monitor.lastCheck'),
      value: item?.last_check ? formatAgo(item.last_check, now, lang) : '–',
    },
  ];
  if (item?.cert_days_left != null)
    facts.push({
      label: t('monitor.cert'),
      value: t('monitor.certDays', { n: item.cert_days_left }),
    });
  if (item?.version) facts.push({ label: t('monitor.version'), value: item.version });

  return (
    <>
      <PageHeader
        title={m.name}
        back={back}
        subtitle={
          <span className="inline-flex flex-wrap items-center gap-2">
            <StateTag state={state} />
            <span>{t(`type.${m.kind === 'push' ? 'push' : (m.check?.type ?? 'http')}`)}</span>
            {m.group && <span>· {m.group}</span>}
            {m.muted && (
              <span className="inline-flex items-center gap-1">
                · <Icon icon={ICONS.notificationsOff} size="xs" /> {t('monitor.muted')}
              </span>
            )}
          </span>
        }
        actions={
          <>
            <Button
              variant="secondary"
              icon={m.paused ? ICONS.start : ICONS.stop}
              busy={busy === 'paused'}
              disabled={busy !== null}
              onClick={() => void toggle('paused')}
            >
              {t(m.paused ? 'monitor.resume' : 'monitor.pause')}
            </Button>
            <Button
              variant="secondary"
              icon={m.muted ? ICONS.notifications : ICONS.notificationsOff}
              busy={busy === 'muted'}
              disabled={busy !== null}
              onClick={() => void toggle('muted')}
            >
              {t(m.muted ? 'monitor.unmute' : 'monitor.mute')}
            </Button>
            <ButtonLink href={hrefOf({ name: 'monitorEdit', id })} icon={ICONS.edit}>
              {t('common.edit')}
            </ButtonLink>
            <Button variant="ghost" icon={ICONS.delete} onClick={() => setConfirm(true)}>
              {t('common.delete')}
            </Button>
          </>
        }
      />

      {(item?.message || item?.reason || item?.unreachable_via_parent) && (
        <p className="m-0 mb-4 text-meta text-muted">
          {item.unreachable_via_parent ? t('overview.viaParent') : item.message || item.reason}
          {item.since ? ` · ${t('monitor.since', { at: formatDateTime(item.since, lang) })}` : ''}
        </p>
      )}
      {m.description && <p className="m-0 mb-4 max-w-prose">{m.description}</p>}

      <dl className="m-0 mb-6 grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-6">
        {facts.map((f) => (
          <div key={f.label} className="rounded-card border border-hairline bg-surface px-4 py-3">
            <dt className="text-meta text-muted">{f.label}</dt>
            <dd className="num m-0 mt-0.5 text-section font-semibold break-words">{f.value}</dd>
          </div>
        ))}
      </dl>

      <div className="flex flex-col gap-6">
        <Card
          title={t('monitor.chart')}
          icon={ICONS.time}
          aside={
            <Segmented<HistoryRange>
              label={t('monitor.range')}
              value={range}
              onChange={setRange}
              options={RANGES.map((r) => ({ value: r, label: r }))}
            />
          }
        >
          {history.data ? (
            <>
              <LatencyChart history={history.data} locationName={locationName} />
              <p className="m-0 mt-2 text-meta text-muted">
                {t('monitor.rangeUptime', { value: formatUptime(history.data.uptime, lang) })}
              </p>
            </>
          ) : history.error ? (
            <ErrorBox error={history.error} retry={history.reload} />
          ) : (
            <Loading />
          )}
        </Card>

        {m.kind === 'push' && m.push_token && (
          <Card title={t('form.pushUrl')} icon={ICONS.link}>
            <PushInfo token={m.push_token} />
          </Card>
        )}

        <Card title={t('monitor.locations')} icon={ICONS.location}>
          {item && item.locations.length > 0 ? (
            <ul className="m-0 flex list-none flex-col gap-2 p-0">
              {item.locations.map((l) => {
                const s = asState(l.state);
                return (
                  <li key={l.location} className="flex flex-wrap items-center gap-x-3 gap-y-1">
                    <Icon icon={STATE[s].icon} size="sm" className={iconTone(s)} />
                    <span className="font-semibold">{locationName(l.location)}</span>
                    <StateTag state={s} />
                    <span className="min-w-0 text-meta text-muted">
                      {l.message}
                      {l.failures > 0 && ` · ${t('monitor.failures', { n: l.failures })}`}
                      {l.at > 0 && ` · ${formatAgo(l.at, now, lang)}`}
                    </span>
                  </li>
                );
              })}
            </ul>
          ) : (
            <p className="m-0 text-muted">{t('monitor.noLocations')}</p>
          )}
          {m.locations.length > 1 && (
            <p className="m-0 mt-3 text-meta text-muted">{t(`quorum.${m.quorum}.help`)}</p>
          )}
        </Card>

        <Events id={id} />
      </div>

      <Dialog
        open={confirm}
        onClose={() => setConfirm(false)}
        title={t('monitor.deleteTitle', { name: m.name })}
        width="sm"
        tone="warning"
        footer={
          <>
            <Button variant="secondary" onClick={() => setConfirm(false)}>
              {t('common.cancel')}
            </Button>
            <Button variant="danger" busy={busy === 'delete'} onClick={() => void remove()}>
              {t('common.delete')}
            </Button>
          </>
        }
      >
        <p className="m-0">{t('monitor.deleteBody')}</p>
      </Dialog>
    </>
  );
}

function eventText(e: MonitorEvent, t: ReturnType<typeof useT>['t']): string {
  if (e.kind === 'state')
    return t('event.state', {
      from: t(STATE[asState(e.from)].label),
      to: t(STATE[asState(e.to)].label),
    });
  if (e.kind === 'version') return t('event.version', { from: e.from || '–', to: e.to });
  return t('event.cert', { from: e.from || '–', to: e.to });
}

function Events({ id }: { id: number }) {
  const { t, lang } = useT();
  const first = useResource(() => api.events(id), [id]);
  const [older, setOlder] = useState<MonitorEvent[]>([]);
  const [end, setEnd] = useState(false);
  const { busy, run } = useAction();
  const all = [...(first.data ?? []), ...older];

  const more = async () => {
    const page = await run('more', () => api.events(id, all[all.length - 1]?.id));
    if (page) {
      setOlder([...older, ...page]);
      if (page.length < 50) setEnd(true);
    }
  };

  return (
    <Card title={t('monitor.events')} icon={ICONS.history}>
      {first.error ? (
        <ErrorBox error={first.error} retry={first.reload} />
      ) : !first.data ? (
        <Loading />
      ) : all.length === 0 ? (
        <p className="m-0 text-muted">{t('monitor.noEvents')}</p>
      ) : (
        <>
          <ol className="m-0 flex list-none flex-col p-0">
            {all.map((e) => {
              const s = e.kind === 'state' ? asState(e.to) : null;
              return (
                <li
                  key={e.id}
                  className="flex gap-3 border-b border-hairline py-2.5 last:border-b-0"
                >
                  <Icon
                    icon={s ? STATE[s].icon : e.kind === 'cert' ? ICONS.locked : ICONS.info}
                    size="sm"
                    className={`mt-0.5 shrink-0 ${s ? iconTone(s) : 'text-muted'}`}
                  />
                  <div className="min-w-0">
                    <div className="font-semibold">{eventText(e, t)}</div>
                    <div className="text-meta text-muted">
                      <time dateTime={new Date(e.at * 1000).toISOString()}>
                        {formatDateTime(e.at, lang)}
                      </time>
                      {e.reason && ` · ${e.reason}`}
                    </div>
                  </div>
                </li>
              );
            })}
          </ol>
          {!end && first.data.length === 50 && (
            <Button
              className="mt-3"
              variant="secondary"
              size="sm"
              busy={busy === 'more'}
              onClick={() => void more()}
            >
              {t('monitor.moreEvents')}
            </Button>
          )}
        </>
      )}
    </Card>
  );
}
