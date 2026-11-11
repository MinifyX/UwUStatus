// Settings: retention and default interval (server), sign-in info (read only), appearance (browser).

import { useEffect, useState, type FormEvent } from 'react';
import {
  Button,
  Card,
  Field,
  Hint,
  ICONS,
  Segmented,
  SettingRow,
  TextInput,
} from '@uwusuite/design';
import type { Appearance } from '@uwusuite/design';
import { api } from '../api/client';
import type { Me, Settings } from '../api/types';
import { ErrorBox, Loading, PageHeader } from '../components/ui';
import { useStoredAppearance, writeAppearance } from '../lib/appearance';
import { formatDuration } from '../lib/format';
import { useAction, useResource } from '../lib/hooks';
import { useT, type Key } from '../lib/i18n';

type NumKey = keyof Settings;
const FIELDS: { key: NumKey; label: Key; min: number; max: number; days: boolean }[] = [
  { key: 'retention_raw_days', label: 'settings.raw', min: 1, max: 90, days: true },
  { key: 'retention_hourly_days', label: 'settings.hourly', min: 1, max: 730, days: true },
  { key: 'retention_daily_days', label: 'settings.daily', min: 1, max: 3650, days: true },
  { key: 'default_interval_s', label: 'settings.interval', min: 10, max: 86_400, days: false },
];

export function SettingsPage({ me }: { me: Me }) {
  const { t } = useT();
  return (
    <>
      <PageHeader title={t('nav.settings')} />
      <div className="flex max-w-3xl flex-col gap-6">
        <ServerSettings />
        <AppearanceCard />
        <AccessCard me={me} />
      </div>
    </>
  );
}

function ServerSettings() {
  const { t, lang } = useT();
  const settings = useResource(api.settings, []);
  const { busy, run } = useAction();
  const [draft, setDraft] = useState<Record<NumKey, string>>();
  const [errors, setErrors] = useState<Partial<Record<NumKey | 'order', string>>>({});
  useEffect(() => {
    if (settings.data)
      setDraft(
        Object.fromEntries(FIELDS.map((f) => [f.key, String(settings.data![f.key])])) as Record<
          NumKey,
          string
        >,
      );
  }, [settings.data]);

  if (settings.error) return <ErrorBox error={settings.error} retry={settings.reload} />;
  if (!draft) return <Loading />;

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const found: typeof errors = {};
    const values = {} as Settings;
    for (const f of FIELDS) {
      const v = draft[f.key].trim();
      const n = Number(v);
      if (!/^\d+$/.test(v) || n < f.min || n > f.max)
        found[f.key] = t('form.err.range', { min: f.min, max: f.max });
      values[f.key] = n;
    }
    if (
      !found.retention_raw_days &&
      !found.retention_hourly_days &&
      !found.retention_daily_days &&
      !(
        values.retention_raw_days <= values.retention_hourly_days &&
        values.retention_hourly_days <= values.retention_daily_days
      )
    )
      found.order = t('settings.order');
    setErrors(found);
    if (Object.keys(found).length) return;
    const saved = await run('save', () => api.saveSettings(values), t('settings.saved'));
    if (saved) settings.set(saved);
  };

  return (
    <Card
      title={t('settings.retention')}
      icon={ICONS.history}
      subtitle={t('settings.retentionHint')}
    >
      <form noValidate className="flex flex-col gap-4" onSubmit={(e) => void submit(e)}>
        <div className="grid gap-4 sm:grid-cols-2">
          {FIELDS.map((f) => {
            const n = Number(draft[f.key]);
            return (
              <Field
                key={f.key}
                label={t(f.label)}
                error={errors[f.key]}
                hint={
                  !f.days && n > 0
                    ? formatDuration(n, lang)
                    : t('settings.range', { min: f.min, max: f.max })
                }
              >
                {(id, describedBy) => (
                  <TextInput
                    id={id}
                    inputMode="numeric"
                    aria-describedby={describedBy}
                    aria-invalid={errors[f.key] ? true : undefined}
                    value={draft[f.key]}
                    onChange={(e) => setDraft({ ...draft, [f.key]: e.target.value })}
                  />
                )}
              </Field>
            );
          })}
        </div>
        {errors.order && (
          <Hint tone="danger" role="alert">
            {errors.order}
          </Hint>
        )}
        <div>
          <Button
            type="submit"
            variant="primary"
            icon={ICONS.save}
            busy={busy === 'save'}
            disabled={busy !== null}
          >
            {t('common.save')}
          </Button>
        </div>
      </form>
    </Card>
  );
}

export function AppearanceCard() {
  const { t } = useT();
  const a = useStoredAppearance();
  const set = (patch: Partial<Appearance>) => writeAppearance({ ...a, ...patch });
  return (
    <Card title={t('look.title')} icon={ICONS.appearance} subtitle={t('look.hint')}>
      <div className="flex flex-col">
        <SettingRow label={t('look.theme')}>
          <Segmented<NonNullable<Appearance['theme']>>
            label={t('look.theme')}
            value={a.theme}
            onChange={(theme) => set({ theme })}
            options={[
              { value: 'system', label: t('look.system') },
              { value: 'light', label: t('look.light') },
              { value: 'dark', label: t('look.dark') },
            ]}
          />
        </SettingRow>
        <SettingRow label={t('look.contrast')}>
          <Segmented<NonNullable<Appearance['contrast']>>
            label={t('look.contrast')}
            value={a.contrast ?? 'system'}
            onChange={(contrast) => set({ contrast })}
            options={[
              { value: 'system', label: t('look.system') },
              { value: 'normal', label: t('look.normal') },
              { value: 'high', label: t('look.high') },
            ]}
          />
        </SettingRow>
        <SettingRow label={t('look.motion')}>
          <Segmented<NonNullable<Appearance['motion']>>
            label={t('look.motion')}
            value={a.motion ?? 'system'}
            onChange={(motion) => set({ motion })}
            options={[
              { value: 'system', label: t('look.system') },
              { value: 'on', label: t('look.on') },
              { value: 'off', label: t('look.off') },
            ]}
          />
        </SettingRow>
      </div>
    </Card>
  );
}

function AccessCard({ me }: { me: Me }) {
  const { t } = useT();
  const methods = useResource(api.methods, []);
  const health = useResource(api.healthz, []);
  const yes = (v: boolean | undefined) =>
    v === undefined ? '…' : t(v ? 'common.yes' : 'common.no');
  const rows: { label: string; value: string }[] = [
    { label: t('access.you'), value: me.email ? `${me.name} (${me.email})` : me.name },
    {
      label: t('access.method'),
      value: t(me.method === 'oidc' ? 'access.viaOidc' : 'access.viaLocal'),
    },
    { label: t('access.host'), value: window.location.host },
    { label: t('access.oidc'), value: yes(methods.data?.oidc) },
    { label: t('access.local'), value: yes(methods.data?.local) },
    { label: t('access.totp'), value: yes(methods.data?.local_totp) },
    { label: t('access.version'), value: health.data?.version ?? '–' },
  ];
  return (
    <Card title={t('access.title')} icon={ICONS.locked} subtitle={t('access.hint')}>
      <dl className="m-0 grid grid-cols-1 gap-x-6 gap-y-2 sm:grid-cols-[max-content_1fr]">
        {rows.map((r) => (
          <div key={r.label} className="contents">
            <dt className="text-meta text-muted">{r.label}</dt>
            <dd className="num m-0 min-w-0 break-words">{r.value}</dd>
          </div>
        ))}
      </dl>
    </Card>
  );
}
