// Maintenance windows: the list and the form (once or weekly, monitors and groups).

import { useEffect, useRef, useState, type FormEvent } from 'react';
import {
  Button,
  Card,
  Dialog,
  Field,
  Hint,
  ICONS,
  IconButton,
  Segmented,
  Tag,
  TextArea,
  TextInput,
  Toggle,
} from '@uwusuite/design';
import { api } from '../api/client';
import type { Maintenance, Schedule } from '../api/types';
import { ButtonLink, Empty, ErrorBox, Link, Loading, PageHeader } from '../components/ui';
import { formatDateTime, formatDuration } from '../lib/format';
import { toasts, useAction, useResource } from '../lib/hooks';
import { useT, type Lang } from '../lib/i18n';
import {
  emptyMaintenance,
  maintenanceDraftFrom,
  validateMaintenance,
  type MaintenanceDraft,
  type MaintenanceErrors,
} from '../lib/maintenanceForm';
import { hrefOf, navigate } from '../lib/router';

const WEEKDAYS = [1, 2, 3, 4, 5, 6, 7];

/** 1 = Monday … 7 = Sunday, as the server counts. 2024-01-01 was a Monday. */
export const weekdayName = (n: number, lang: Lang, width: 'short' | 'long' = 'short') =>
  new Date(2024, 0, n).toLocaleDateString(lang, { weekday: width });

export function scheduleText(s: Schedule, lang: Lang): string {
  if (s.type === 'once') return `${formatDateTime(s.start, lang)} – ${formatDateTime(s.end, lang)}`;
  const days = s.weekdays.map((n) => weekdayName(n, lang)).join(', ');
  return `${days} · ${s.time} · ${formatDuration(s.duration_min * 60, lang)} · ${s.timezone}`;
}

export function MaintenanceList() {
  const { t, lang } = useT();
  const list = useResource(api.maintenance, []);
  const monitors = useResource(api.monitors, []);
  const { busy, run } = useAction();
  const [doomed, setDoomed] = useState<Maintenance | null>(null);
  const nameOf = (id: number) => monitors.data?.find((m) => m.id === id)?.name ?? `#${id}`;

  const remove = async (m: Maintenance) => {
    const done = await run('delete', async () => {
      await api.deleteMaintenance(m.id);
      return true;
    });
    setDoomed(null);
    if (done) {
      toasts.show(t('maint.deleted', { name: m.title }), { tone: 'success' });
      list.reload();
    }
  };

  return (
    <>
      <PageHeader
        title={t('nav.maintenance')}
        subtitle={t('maint.intro')}
        actions={
          <ButtonLink href={hrefOf({ name: 'maintenanceNew' })} variant="primary" icon={ICONS.add}>
            {t('maint.new')}
          </ButtonLink>
        }
      />
      {list.error ? (
        <ErrorBox error={list.error} retry={list.reload} />
      ) : !list.data ? (
        <Loading />
      ) : list.data.length === 0 ? (
        <Empty title={t('maint.empty')} body={t('maint.emptyBody')} mood="sleepy" />
      ) : (
        <ul className="m-0 flex list-none flex-col gap-3 p-0">
          {list.data.map((m) => (
            <li
              key={m.id}
              className="flex flex-wrap items-start justify-between gap-3 rounded-card border border-hairline bg-surface px-4 py-3"
            >
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  <Link
                    href={hrefOf({ name: 'maintenanceEdit', id: m.id })}
                    className="font-semibold text-ink"
                  >
                    {m.title}
                  </Link>
                  <Tag tone={m.active ? 'pink' : 'neutral'}>
                    {t(m.schedule.type === 'once' ? 'maint.once' : 'maint.weekly')}
                  </Tag>
                  {!m.active && <Tag>{t('maint.inactive')}</Tag>}
                </div>
                <div className="num mt-0.5 text-meta text-muted">
                  {scheduleText(m.schedule, lang)}
                </div>
                <div className="mt-0.5 text-meta text-muted">
                  {[
                    ...m.monitors.map(nameOf),
                    ...m.groups.map((g) => t('maint.groupNamed', { name: g })),
                  ].join(', ')}
                </div>
              </div>
              <div className="flex gap-1">
                <ButtonLink
                  href={hrefOf({ name: 'maintenanceEdit', id: m.id })}
                  size="sm"
                  icon={ICONS.edit}
                >
                  {t('common.edit')}
                </ButtonLink>
                <IconButton
                  icon={ICONS.delete}
                  label={t('maint.deleteNamed', { name: m.title })}
                  onClick={() => setDoomed(m)}
                />
              </div>
            </li>
          ))}
        </ul>
      )}
      <Dialog
        open={doomed !== null}
        onClose={() => setDoomed(null)}
        title={t('maint.deleteTitle', { name: doomed?.title ?? '' })}
        width="sm"
        tone="warning"
        footer={
          <>
            <Button variant="secondary" onClick={() => setDoomed(null)}>
              {t('common.cancel')}
            </Button>
            <Button
              variant="danger"
              busy={busy === 'delete'}
              onClick={() => doomed && void remove(doomed)}
            >
              {t('common.delete')}
            </Button>
          </>
        }
      >
        <p className="m-0">{t('maint.deleteBody')}</p>
      </Dialog>
    </>
  );
}

export function MaintenanceForm({ id }: { id?: number }) {
  const { t } = useT();
  const existing = useResource(() => (id ? api.maintenanceOne(id) : Promise.resolve(null)), [id]);
  const back = { href: hrefOf({ name: 'maintenance' }), label: t('nav.maintenance') };
  if (existing.error)
    return (
      <>
        <PageHeader title={t('nav.maintenance')} back={back} />
        <ErrorBox error={existing.error} retry={existing.reload} />
      </>
    );
  if (existing.loading && !existing.data) return <Loading />;
  const m = existing.data ?? null;
  return (
    <>
      <PageHeader
        title={m ? t('maint.editTitle', { name: m.title }) : t('maint.new')}
        back={back}
      />
      <MaintenanceBody key={m?.id ?? 'new'} existing={m} />
    </>
  );
}

function MaintenanceBody({ existing }: { existing: Maintenance | null }) {
  const { t, lang } = useT();
  const [d, setD] = useState<MaintenanceDraft>(() => {
    const now = Date.now() / 1000;
    return existing ? maintenanceDraftFrom(existing, now) : emptyMaintenance(now);
  });
  const [errors, setErrors] = useState<MaintenanceErrors>({});
  const [tries, setTries] = useState(0);
  const form = useRef<HTMLFormElement>(null);
  const monitors = useResource(api.monitors, []);
  const { busy, run } = useAction();
  const groups = [...new Set((monitors.data ?? []).map((m) => m.group).filter(Boolean))].sort();

  useEffect(() => {
    if (tries) form.current?.querySelector<HTMLElement>('[aria-invalid="true"]')?.focus();
  }, [tries]);

  const set = <K extends keyof MaintenanceDraft>(key: K, value: MaintenanceDraft[K]) => {
    setD((prev) => ({ ...prev, [key]: value }));
    if (errors[key]) setErrors((prev) => ({ ...prev, [key]: undefined }));
  };
  const err = (key: keyof MaintenanceDraft) => {
    const e = errors[key];
    return e ? t(e.key, e.vars) : undefined;
  };
  const toggleIn = <T,>(items: T[], item: T, on: boolean) =>
    on ? [...items, item] : items.filter((x) => x !== item);

  const input = (
    key: 'title' | 'start' | 'end' | 'time' | 'duration' | 'timezone',
    label: string,
    type = 'text',
    hint?: string,
  ) => (
    <Field label={label} error={err(key)} hint={hint}>
      {(fid, describedBy) => (
        <TextInput
          id={fid}
          type={type}
          aria-describedby={describedBy}
          aria-invalid={errors[key] ? true : undefined}
          inputMode={key === 'duration' ? 'numeric' : undefined}
          value={d[key]}
          autoComplete="off"
          onChange={(e) => set(key, e.target.value)}
        />
      )}
    </Field>
  );

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const { errors: found, input: body } = validateMaintenance(d);
    setErrors(found);
    if (!body) {
      setTries((n) => n + 1);
      return;
    }
    const saved = await run('save', () =>
      existing ? api.saveMaintenance(existing.id, body) : api.createMaintenance(body),
    );
    if (saved) {
      toasts.show(t('form.saved'), { tone: 'success' });
      navigate(hrefOf({ name: 'maintenance' }));
    }
  };

  return (
    <form
      ref={form}
      noValidate
      className="flex max-w-3xl flex-col gap-6"
      onSubmit={(e) => void submit(e)}
    >
      <Card title={t('form.basics')} icon={ICONS.edit}>
        <div className="flex flex-col gap-4">
          {input('title', t('maint.title'))}
          <Field
            label={t('form.description')}
            error={err('description')}
            hint={t('maint.descriptionHint')}
          >
            {(fid, describedBy) => (
              <TextArea
                id={fid}
                rows={3}
                aria-describedby={describedBy}
                aria-invalid={errors.description ? true : undefined}
                value={d.description}
                onChange={(e) => set('description', e.target.value)}
              />
            )}
          </Field>
          <Toggle
            label={t('maint.active')}
            description={t('maint.activeHint')}
            checked={d.active}
            onChange={(v) => set('active', v)}
          />
        </div>
      </Card>

      <Card
        title={t('maint.when')}
        icon={ICONS.calendar}
        aside={
          <Segmented<MaintenanceDraft['type']>
            label={t('maint.when')}
            value={d.type}
            onChange={(v) => set('type', v)}
            options={[
              { value: 'once', label: t('maint.once') },
              { value: 'weekly', label: t('maint.weekly') },
            ]}
          />
        }
      >
        {d.type === 'once' ? (
          <div className="grid gap-4 sm:grid-cols-2">
            {input('start', t('maint.start'), 'datetime-local')}
            {input('end', t('maint.end'), 'datetime-local')}
          </div>
        ) : (
          <div className="flex flex-col gap-4">
            <fieldset
              className="m-0 border-0 p-0"
              aria-describedby={errors.weekdays ? 'wd-err' : undefined}
            >
              <legend className="mb-2 text-meta font-semibold">{t('maint.weekdays')}</legend>
              <div className="flex flex-wrap gap-2">
                {WEEKDAYS.map((n) => (
                  <label
                    key={n}
                    className="flex min-h-9 items-center gap-2 rounded-full border border-hairline px-3 has-checked:border-pink has-checked:bg-pink-tint"
                  >
                    <input
                      type="checkbox"
                      className="size-4 accent-pink"
                      aria-invalid={errors.weekdays ? true : undefined}
                      checked={d.weekdays.includes(n)}
                      onChange={(e) => set('weekdays', toggleIn(d.weekdays, n, e.target.checked))}
                    />
                    <abbr title={weekdayName(n, lang, 'long')} className="no-underline">
                      {weekdayName(n, lang)}
                    </abbr>
                  </label>
                ))}
              </div>
              {errors.weekdays && (
                <p id="wd-err" className="m-0 mt-1 text-meta text-danger-ink">
                  {err('weekdays')}
                </p>
              )}
            </fieldset>
            <div className="grid gap-4 sm:grid-cols-3">
              {input('time', t('maint.time'), 'time')}
              {input(
                'duration',
                t('maint.duration'),
                'text',
                Number(d.duration) > 0 ? formatDuration(Number(d.duration) * 60, lang) : undefined,
              )}
              {input('timezone', t('maint.zone'), 'text', t('maint.zoneHint'))}
            </div>
          </div>
        )}
      </Card>

      <Card title={t('maint.targets')} icon={ICONS.overview} subtitle={t('maint.targetsHint')}>
        {monitors.error && <ErrorBox error={monitors.error} retry={monitors.reload} />}
        <div className="grid gap-6 sm:grid-cols-2">
          <fieldset
            className="m-0 flex flex-col gap-1 border-0 p-0"
            aria-describedby={errors.monitors ? 'tg-err' : undefined}
          >
            <legend className="mb-1 text-meta font-semibold">{t('maint.monitors')}</legend>
            {(monitors.data ?? []).map((m) => (
              <label key={m.id} className="flex min-h-9 items-center gap-2.5">
                <input
                  type="checkbox"
                  className="size-4 accent-pink"
                  aria-invalid={errors.monitors ? true : undefined}
                  checked={d.monitors.includes(m.id)}
                  onChange={(e) => set('monitors', toggleIn(d.monitors, m.id, e.target.checked))}
                />
                <span className="min-w-0 truncate">{m.name}</span>
              </label>
            ))}
            {monitors.data?.length === 0 && <p className="m-0 text-muted">{t('overview.empty')}</p>}
          </fieldset>
          <fieldset className="m-0 flex flex-col gap-1 border-0 p-0">
            <legend className="mb-1 text-meta font-semibold">{t('maint.groups')}</legend>
            {groups.map((g) => (
              <label key={g} className="flex min-h-9 items-center gap-2.5">
                <input
                  type="checkbox"
                  className="size-4 accent-pink"
                  checked={d.groups.includes(g)}
                  onChange={(e) => {
                    set('groups', toggleIn(d.groups, g, e.target.checked));
                    if (errors.monitors) setErrors((prev) => ({ ...prev, monitors: undefined }));
                  }}
                />
                {g}
              </label>
            ))}
            {groups.length === 0 && (
              <p className="m-0 text-meta text-muted">{t('maint.noGroups')}</p>
            )}
          </fieldset>
        </div>
        {errors.monitors && (
          <p id="tg-err" className="m-0 mt-2 text-meta text-danger-ink">
            {err('monitors')}
          </p>
        )}
      </Card>

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
        <ButtonLink variant="ghost" href={hrefOf({ name: 'maintenance' })}>
          {t('common.cancel')}
        </ButtonLink>
      </div>
    </form>
  );
}
