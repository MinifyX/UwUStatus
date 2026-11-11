// Create or edit a monitor: type-specific check fields, timing, locations + quorum, channels.

import { useEffect, useRef, useState, type FormEvent, type ReactNode } from 'react';
import {
  Button,
  Card,
  Field,
  Hint,
  ICONS,
  Select,
  TextArea,
  TextInput,
  Toggle,
} from '@uwusuite/design';
import { api } from '../api/client';
import type { Monitor, Quorum } from '../api/types';
import { ButtonLink, ErrorBox, Loading, PageHeader } from '../components/ui';
import { formatDuration } from '../lib/format';
import { toasts, useAction, useResource } from '../lib/hooks';
import { useT } from '../lib/i18n';
import {
  draftFrom,
  emptyDraft,
  FORM_TYPES,
  pushCurl,
  pushUrl,
  validate,
  withType,
  type DraftField,
  type Errors,
  type FormType,
  type MonitorDraft,
} from '../lib/monitorForm';
import { hrefOf, navigate } from '../lib/router';
import { useLocationName } from './MonitorDetail';

const QUORUMS: Quorum[] = ['any', 'majority', 'all'];
const RECORD_TYPES = ['A', 'AAAA', 'CNAME', 'MX', 'TXT', 'NS', 'SRV', 'CAA'];
const METHODS = ['GET', 'HEAD', 'POST', 'PUT', 'OPTIONS'];

export function MonitorForm({ id }: { id?: number }) {
  const { t } = useT();
  const existing = useResource(() => (id ? api.monitor(id) : Promise.resolve(null)), [id]);
  if (existing.error) return <ErrorBox error={existing.error} retry={existing.reload} />;
  if (existing.loading && !existing.data) return <Loading />;
  const m = existing.data ?? null;
  return (
    <>
      <PageHeader
        title={m ? t('form.editTitle', { name: m.name }) : t('monitor.new')}
        back={
          m
            ? { href: hrefOf({ name: 'monitor', id: m.id }), label: m.name }
            : { href: '/', label: t('nav.overview') }
        }
      />
      <FormBody key={m?.id ?? 'new'} monitor={m} />
    </>
  );
}

function FormBody({ monitor }: { monitor: Monitor | null }) {
  const { t, lang } = useT();
  const [d, setD] = useState<MonitorDraft>(() => (monitor ? draftFrom(monitor) : emptyDraft()));
  const [errors, setErrors] = useState<Errors>({});
  const [focusError, setFocusError] = useState(0);
  const form = useRef<HTMLFormElement>(null);
  const { busy, run } = useAction();
  const locationName = useLocationName();

  const monitors = useResource(api.monitors, []);
  const settings = useResource(api.settings, []);
  const satellites = useResource(api.satellites, []);
  const channels = useResource(api.channels, []);
  const assigned = useResource(
    () => (monitor ? api.monitorChannels(monitor.id) : Promise.resolve(null)),
    [monitor?.id],
  );
  const [picked, setPicked] = useState<number[] | null>(null);
  const chosen =
    picked ??
    (monitor ? assigned.data : channels.data?.filter((c) => c.is_default).map((c) => c.id)) ??
    [];

  useEffect(() => {
    if (focusError) form.current?.querySelector<HTMLElement>('[aria-invalid="true"]')?.focus();
  }, [focusError]);

  const set = <K extends DraftField>(key: K, value: MonitorDraft[K]) => {
    setD((prev) => ({ ...prev, [key]: value }));
    if (errors[key]) setErrors((prev) => ({ ...prev, [key]: undefined }));
  };
  const err = (key: DraftField) => {
    const e = errors[key];
    return e ? t(e.key, e.vars) : undefined;
  };

  /** A labelled text input bound to one draft field. */
  const text = (
    key: DraftField,
    label: string,
    opts: {
      hint?: ReactNode;
      placeholder?: string;
      numeric?: boolean;
      mono?: boolean;
      className?: string;
    } = {},
  ) => (
    <Field label={label} hint={opts.hint} error={err(key)} className={opts.className}>
      {(fid, describedBy) => (
        <TextInput
          id={fid}
          aria-describedby={describedBy}
          aria-invalid={errors[key] ? true : undefined}
          value={String(d[key] ?? '')}
          placeholder={opts.placeholder}
          inputMode={opts.numeric ? 'numeric' : undefined}
          spellCheck={false}
          autoComplete="off"
          className={opts.mono ? 'font-mono' : undefined}
          onChange={(e) => set(key, e.target.value as never)}
        />
      )}
    </Field>
  );
  const seconds = (key: DraftField, label: string, hint?: string) =>
    text(key, label, {
      numeric: true,
      hint: hint ?? (Number(d[key]) > 0 ? formatDuration(Number(d[key]), lang) : undefined),
    });

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const { errors: found, config } = validate(d);
    setErrors(found);
    if (!config) {
      setFocusError((n) => n + 1);
      return;
    }
    const saved = await run('save', async () => {
      const m = monitor
        ? await api.saveMonitor(monitor.id, config)
        : await api.createMonitor(config);
      if (channels.data && (picked !== null || !monitor))
        await api.saveMonitorChannels(m.id, chosen);
      return m;
    });
    if (saved) {
      toasts.show(t('form.saved'), { tone: 'success' });
      navigate(hrefOf({ name: 'monitor', id: saved.id }));
    }
  };

  const push = d.type === 'push';
  const http = d.type === 'http';
  const mail = d.type === 'smtp' || d.type === 'imap';
  const sats = satellites.data;
  const locationIds = ['local', ...(sats ?? []).map((s) => String(s.id))];
  const parents = (monitors.data ?? []).filter((x) => x.id !== monitor?.id);
  const groups = [...new Set((monitors.data ?? []).map((x) => x.group).filter(Boolean))];
  const defaultInterval = settings.data?.default_interval_s;

  return (
    <form
      ref={form}
      noValidate
      className="flex max-w-3xl flex-col gap-6"
      onSubmit={(e) => void submit(e)}
    >
      <Card title={t('form.basics')} icon={ICONS.edit}>
        <div className="grid gap-4 sm:grid-cols-2">
          <Field label={t('form.type')} className="sm:col-span-2">
            {(fid) => (
              <Select
                id={fid}
                value={d.type}
                onChange={(e) => setD(withType(d, e.target.value as FormType))}
              >
                {FORM_TYPES.map((type) => (
                  <option key={type} value={type}>
                    {t(`type.${type}`)}
                  </option>
                ))}
              </Select>
            )}
          </Field>
          {text('name', t('form.name'), { className: 'sm:col-span-2' })}
          <Field label={t('form.description')} error={err('description')} className="sm:col-span-2">
            {(fid, describedBy) => (
              <TextArea
                id={fid}
                aria-describedby={describedBy}
                aria-invalid={errors.description ? true : undefined}
                rows={2}
                value={d.description}
                onChange={(e) => set('description', e.target.value)}
              />
            )}
          </Field>
          <Field label={t('form.group')} error={err('group')} hint={t('form.groupHint')}>
            {(fid, describedBy) => (
              <>
                <TextInput
                  id={fid}
                  list={`${fid}-groups`}
                  aria-describedby={describedBy}
                  aria-invalid={errors.group ? true : undefined}
                  value={d.group}
                  autoComplete="off"
                  onChange={(e) => set('group', e.target.value)}
                />
                <datalist id={`${fid}-groups`}>
                  {groups.map((g) => (
                    <option key={g} value={g} />
                  ))}
                </datalist>
              </>
            )}
          </Field>
          {text('tags', t('form.tags'), { hint: t('form.tagsHint') })}
        </div>
      </Card>

      <Card title={t('form.check')} icon={ICONS.securityCheck} subtitle={t(`type.${d.type}.help`)}>
        <div className="grid gap-4 sm:grid-cols-2">
          {(http || d.type === 'healthz' || d.type === 'docker') &&
            text('url', t(d.type === 'docker' ? 'form.dockerUrl' : 'form.url'), {
              className: 'sm:col-span-2',
              mono: true,
              placeholder:
                d.type === 'docker'
                  ? 'http://socket-proxy.test:2375'
                  : d.type === 'healthz'
                    ? 'https://app.example.com/healthz'
                    : 'https://www.example.com/',
              hint: d.type === 'docker' ? t('form.dockerUrlHint') : undefined,
            })}
          {http && (
            <>
              <Field label={t('form.method')}>
                {(fid) => (
                  <Select id={fid} value={d.method} onChange={(e) => set('method', e.target.value)}>
                    {METHODS.map((x) => (
                      <option key={x}>{x}</option>
                    ))}
                  </Select>
                )}
              </Field>
              {text('accepted', t('form.accepted'), {
                placeholder: '200-299',
                hint: t('form.acceptedHint'),
              })}
              {text('keyword', t('form.keyword'), { hint: t('form.keywordHint') })}
              <div className="self-end pb-1">
                <Toggle
                  label={t('form.keywordAbsent')}
                  checked={d.keywordAbsent}
                  disabled={!d.keyword.trim()}
                  onChange={(v) => set('keywordAbsent', v)}
                />
              </div>
              {text('jsonPath', t('form.jsonPath'), {
                mono: true,
                placeholder: '$.status',
                hint: t('form.jsonPathHint'),
              })}
              {text('jsonValue', t('form.jsonValue'), { mono: true, placeholder: 'ok' })}
              <Toggle
                label={t('form.follow')}
                checked={d.followRedirects}
                onChange={(v) => set('followRedirects', v)}
              />
            </>
          )}
          {(d.type === 'tcp' || d.type === 'ping' || mail) &&
            text('host', t('form.host'), {
              mono: true,
              placeholder: mail ? 'mail.example.com' : '192.0.2.10',
              className: d.type === 'ping' ? 'sm:col-span-2' : undefined,
            })}
          {(d.type === 'tcp' || mail) && text('port', t('form.port'), { numeric: true })}
          {mail && (
            <Field label={t('form.tls')}>
              {(fid) => (
                <Select
                  id={fid}
                  value={d.tls}
                  onChange={(e) => set('tls', e.target.value as MonitorDraft['tls'])}
                >
                  <option value="none">{t('form.tls.none')}</option>
                  <option value="start_tls">STARTTLS</option>
                  <option value="tls">{t('form.tls.tls')}</option>
                </Select>
              )}
            </Field>
          )}
          {(http || mail) && (
            <div className="self-end pb-1">
              <Toggle
                label={t('form.ignoreTls')}
                description={t('form.ignoreTlsHint')}
                checked={d.ignoreTls}
                onChange={(v) => set('ignoreTls', v)}
              />
            </div>
          )}
          {d.type === 'dns' && (
            <>
              {text('dnsName', t('form.dnsName'), { mono: true, placeholder: 'www.example.com' })}
              <Field label={t('form.recordType')}>
                {(fid) => (
                  <Select
                    id={fid}
                    value={d.recordType}
                    onChange={(e) => set('recordType', e.target.value)}
                  >
                    {RECORD_TYPES.map((x) => (
                      <option key={x}>{x}</option>
                    ))}
                  </Select>
                )}
              </Field>
              {text('resolver', t('form.resolver'), {
                mono: true,
                placeholder: '192.0.2.53',
                hint: t('form.resolverHint'),
              })}
              {text('expected', t('form.expected'), {
                mono: true,
                placeholder: '203.0.113.7',
                hint: t('form.expectedHint'),
              })}
            </>
          )}
          {d.type === 'docker' &&
            text('container', t('form.container'), { mono: true, placeholder: 'webmail' })}
          {push && (
            <div className="sm:col-span-2">
              {monitor?.push_token ? (
                <PushInfo token={monitor.push_token} />
              ) : (
                <Hint tone="info">{t('form.pushAfterSave')}</Hint>
              )}
            </div>
          )}
        </div>
      </Card>

      <Card title={t('form.timing')} icon={ICONS.time}>
        <div className="grid gap-4 sm:grid-cols-2">
          {seconds(
            'interval',
            t(push ? 'form.pushInterval' : 'form.interval'),
            Number(d.interval) === 0 && defaultInterval
              ? t('form.intervalDefault', { value: formatDuration(defaultInterval, lang) })
              : undefined,
          )}
          {seconds('timeout', t(push ? 'form.pushGrace' : 'form.timeout'))}
          {text('retries', t('form.retries'), { numeric: true, hint: t('form.retriesHint') })}
          {seconds(
            'retryInterval',
            t('form.retryInterval'),
            Number(d.retryInterval) === 0 ? t('form.retryIntervalHint') : undefined,
          )}
          {text('reminder', t('form.reminder'), {
            numeric: true,
            hint:
              Number(d.reminder) > 0
                ? formatDuration(Number(d.reminder) * 60, lang)
                : t('form.reminderOff'),
          })}
          {(http || mail) &&
            text('certWarn', t('form.certWarn'), { numeric: true, hint: t('form.certWarnHint') })}
          <Field label={t('form.parent')} hint={t('form.parentHint')} className="sm:col-span-2">
            {(fid, describedBy) => (
              <Select
                id={fid}
                aria-describedby={describedBy}
                value={d.parentId}
                onChange={(e) => set('parentId', e.target.value)}
              >
                <option value="">{t('form.noParent')}</option>
                {parents.map((p) => (
                  <option key={p.id} value={p.id}>
                    {p.name}
                  </option>
                ))}
              </Select>
            )}
          </Field>
          <Toggle
            label={t('form.muted')}
            description={t('form.mutedHint')}
            checked={d.muted}
            onChange={(v) => set('muted', v)}
          />
          <Toggle label={t('form.paused')} checked={d.paused} onChange={(v) => set('paused', v)} />
        </div>
      </Card>

      <Card title={t('monitor.locations')} icon={ICONS.location}>
        <fieldset
          className="m-0 flex flex-col gap-2 border-0 p-0"
          aria-describedby={errors.locations ? 'loc-err' : undefined}
        >
          <legend className="sr-only">{t('monitor.locations')}</legend>
          {locationIds.map((loc) => (
            <label key={loc} className="flex min-h-9 items-center gap-2.5">
              <input
                type="checkbox"
                className="size-4 accent-pink"
                checked={d.locations.includes(loc)}
                disabled={push && loc !== 'local'}
                aria-invalid={errors.locations ? true : undefined}
                onChange={(e) =>
                  set(
                    'locations',
                    e.target.checked ? [...d.locations, loc] : d.locations.filter((x) => x !== loc),
                  )
                }
              />
              {locationName(loc)}
            </label>
          ))}
          {errors.locations && (
            <p id="loc-err" className="m-0 text-meta text-danger-ink">
              {err('locations')}
            </p>
          )}
          {sats === null && <p className="m-0 text-meta text-muted">{t('form.noSatellites')}</p>}
          {push && <p className="m-0 text-meta text-muted">{t('form.pushLocalOnly')}</p>}
        </fieldset>
        {d.locations.length > 1 && (
          <fieldset className="m-0 mt-4 flex flex-col gap-2 border-0 p-0">
            <legend className="mb-1 font-semibold">{t('form.quorum')}</legend>
            {QUORUMS.map((q) => (
              <label key={q} className="flex items-start gap-2.5">
                <input
                  type="radio"
                  name="quorum"
                  className="mt-1 size-4 accent-pink"
                  checked={d.quorum === q}
                  onChange={() => set('quorum', q)}
                />
                <span>
                  <span className="font-semibold">{t(`quorum.${q}`)}</span>
                  <span className="block text-meta text-muted">{t(`quorum.${q}.help`)}</span>
                </span>
              </label>
            ))}
          </fieldset>
        )}
      </Card>

      {channels.data && (
        <Card
          title={t('form.channels')}
          icon={ICONS.notifications}
          subtitle={t('form.channelsHint')}
        >
          {channels.data.length === 0 ? (
            <p className="m-0 text-muted">{t('form.noChannels')}</p>
          ) : (
            <fieldset className="m-0 flex flex-col gap-2 border-0 p-0">
              <legend className="sr-only">{t('form.channels')}</legend>
              {channels.data.map((c) => (
                <label key={c.id} className="flex min-h-9 items-center gap-2.5">
                  <input
                    type="checkbox"
                    className="size-4 accent-pink"
                    checked={chosen.includes(c.id)}
                    onChange={(e) =>
                      setPicked(
                        e.target.checked ? [...chosen, c.id] : chosen.filter((x) => x !== c.id),
                      )
                    }
                  />
                  <span>
                    {c.name}
                    {!c.enabled && (
                      <span className="text-meta text-muted"> · {t('form.channelOff')}</span>
                    )}
                  </span>
                </label>
              ))}
            </fieldset>
          )}
        </Card>
      )}

      {Object.values(errors).some(Boolean) && (
        <Hint tone="danger" role="alert">
          {t('form.fix')}
        </Hint>
      )}
      <div className="flex flex-wrap gap-2">
        <Button
          type="submit"
          variant="primary"
          icon={ICONS.save}
          busy={busy === 'save'}
          disabled={busy !== null}
        >
          {t('common.save')}
        </Button>
        <ButtonLink
          variant="ghost"
          href={monitor ? hrefOf({ name: 'monitor', id: monitor.id }) : '/'}
        >
          {t('common.cancel')}
        </ButtonLink>
      </div>
    </form>
  );
}

/** Push URL and a curl line, each with a copy button. */
export function PushInfo({ token }: { token: string }) {
  const { t } = useT();
  const origin = window.location.origin;
  const rows = [
    { label: t('form.pushUrl'), value: pushUrl(origin, token) },
    { label: t('form.pushCurl'), value: pushCurl(origin, token) },
  ];
  const copy = (value: string) =>
    navigator.clipboard.writeText(value).then(
      () => toasts.show(t('common.copied'), { tone: 'success' }),
      () => toasts.show(t('common.copyFailed'), { tone: 'error' }),
    );
  return (
    <div className="flex flex-col gap-3">
      <p className="m-0 text-meta text-muted">{t('form.pushHelp')}</p>
      {rows.map((r) => (
        <div key={r.label} className="flex flex-col gap-1">
          <span className="text-meta font-semibold">{r.label}</span>
          <div className="flex items-start gap-2">
            <code className="min-w-0 flex-1 rounded-control bg-canvas px-3 py-2 font-mono text-meta break-all">
              {r.value}
            </code>
            <Button
              type="button"
              size="sm"
              variant="secondary"
              icon={ICONS.copy}
              onClick={() => void copy(r.value)}
            >
              {t('common.copy')}
            </Button>
          </div>
        </div>
      ))}
    </div>
  );
}
