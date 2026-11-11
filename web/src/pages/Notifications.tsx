// Notification channels: list, create/edit with type-specific fields, test, delivery log, SMTP.

import { useEffect, useState, type FormEvent } from 'react';
import {
  Button,
  Card,
  Dialog,
  Field,
  Hint,
  ICONS,
  Segmented,
  Select,
  Tag,
  TextArea,
  TextInput,
  Toggle,
} from '@uwusuite/design';
import { api } from '../api/client';
import type { Channel, ChannelInput, ChannelType, SmtpInput } from '../api/types';
import { Empty, ErrorBox, Loading, PageHeader } from '../components/ui';
import {
  CHANNEL_TYPES,
  DEFAULT_EVENTS,
  EVENT_KEYS,
  FIELDS,
  SECRETS,
  specFrom,
  valuesFrom,
} from '../lib/channels';
import { useAction, useResource } from '../lib/hooks';
import { useT, type Key } from '../lib/i18n';

type Editing = { channel?: Channel } | null;

export function NotificationsPage() {
  const { t } = useT();
  const channels = useResource(api.channels, []);
  const { busy, run } = useAction();
  const [editing, setEditing] = useState<Editing>(null);
  const [deleting, setDeleting] = useState<Channel>();
  const [logOf, setLogOf] = useState<Channel>();

  const remove = async (channel: Channel) => {
    await run('delete', () => api.deleteChannel(channel.id), t('notify.deleted'));
    setDeleting(undefined);
    channels.reload();
  };

  return (
    <>
      <PageHeader
        title={t('nav.notifications')}
        subtitle={t('notify.intro')}
        actions={
          <Button variant="primary" icon={ICONS.add} onClick={() => setEditing({})}>
            {t('notify.add')}
          </Button>
        }
      />
      <div className="flex flex-col gap-6">
        <Card title={t('notify.channels')} icon={ICONS.notifications}>
          {channels.error && <ErrorBox error={channels.error} retry={channels.reload} />}
          {!channels.data && !channels.error && <Loading />}
          {channels.data?.length === 0 && (
            <Empty title={t('notify.empty')} body={t('notify.emptyBody')} />
          )}
          {channels.data && channels.data.length > 0 && (
            <ul className="m-0 flex list-none flex-col divide-y divide-hairline p-0">
              {channels.data.map((c) => (
                <li key={c.id} className="flex flex-wrap items-center gap-x-3 gap-y-2 py-3">
                  <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
                    <span className="font-semibold break-words">{c.name}</span>
                    <Tag>{t(`notify.t.${c.channel.type}` as Key)}</Tag>
                    {c.is_default && <Tag tone="pink">{t('notify.default')}</Tag>}
                    {!c.enabled && <Tag tone="warning">{t('notify.off')}</Tag>}
                  </div>
                  <div className="flex flex-wrap gap-1">
                    <Button
                      size="sm"
                      variant="secondary"
                      icon={ICONS.send}
                      disabled={busy !== null}
                      onClick={() =>
                        void run(`test-${c.id}`, () => api.testChannel(c.id), t('notify.tested'))
                      }
                    >
                      {t('notify.test')}
                    </Button>
                    <Button size="sm" variant="ghost" onClick={() => setLogOf(c)}>
                      {t('notify.log')}
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      icon={ICONS.edit}
                      onClick={() => setEditing({ channel: c })}
                    >
                      {t('notify.edit')}
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      icon={ICONS.delete}
                      onClick={() => setDeleting(c)}
                    >
                      {t('notify.delete')}
                    </Button>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </Card>
        <SmtpCard />
      </div>
      {editing && (
        <ChannelDialog
          channel={editing.channel}
          onClose={() => setEditing(null)}
          onSaved={() => {
            setEditing(null);
            channels.reload();
          }}
        />
      )}
      <Dialog
        open={deleting !== undefined}
        onClose={() => setDeleting(undefined)}
        title={t('notify.deleteTitle', { name: deleting?.name ?? '' })}
        tone="warning"
        width="sm"
        footer={
          <>
            <Button variant="ghost" onClick={() => setDeleting(undefined)}>
              {t('notify.cancel')}
            </Button>
            <Button
              variant="danger"
              disabled={busy !== null}
              onClick={() => deleting && void remove(deleting)}
            >
              {t('notify.delete')}
            </Button>
          </>
        }
      >
        <p className="m-0">{t('notify.deleteBody')}</p>
      </Dialog>
      {logOf && <LogDialog channel={logOf} onClose={() => setLogOf(undefined)} />}
    </>
  );
}

function ChannelDialog({
  channel,
  onClose,
  onSaved,
}: {
  channel?: Channel;
  onClose: () => void;
  onSaved: () => void;
}) {
  const { t, lang } = useT();
  const { busy, run } = useAction();
  const [name, setName] = useState(channel?.name ?? '');
  const [type, setType] = useState<ChannelType>(channel?.channel.type ?? 'webhook');
  const [values, setValues] = useState(() => (channel ? valuesFrom(channel.channel) : {}));
  const [secrets, setSecrets] = useState<Record<string, string>>({});
  const [msgLang, setMsgLang] = useState(channel?.lang ?? lang);
  const [events, setEvents] = useState(channel?.events ?? DEFAULT_EVENTS);
  const [enabled, setEnabled] = useState(channel?.enabled ?? true);
  const [isDefault, setIsDefault] = useState(channel?.is_default ?? false);
  // Stored secrets only count while the type is unchanged (the server drops them otherwise).
  const stored = channel && channel.channel.type === type ? channel.secrets : [];

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const input: ChannelInput = {
      name,
      enabled,
      lang: msgLang,
      events,
      is_default: isDefault,
      channel: specFrom(type, values),
      // Empty inputs keep what is stored.
      // shortcut: an optional secret cannot be cleared from the form; the API takes "" for that.
      secrets: Object.fromEntries(
        SECRETS[type].map((s) => [s.key, secrets[s.key] ?? '']).filter(([, v]) => v !== ''),
      ),
    };
    const saved = await run(
      'save',
      () => (channel ? api.saveChannel(channel.id, input) : api.createChannel(input)),
      t('notify.saved'),
    );
    if (saved) onSaved();
  };

  return (
    <Dialog
      open
      onClose={onClose}
      width="md"
      closeOnOutsideClick={false}
      title={t(channel ? 'notify.editTitle' : 'notify.newTitle')}
    >
      <form className="flex flex-col gap-4" onSubmit={(e) => void submit(e)}>
        <Field label={t('notify.name')}>
          {(id) => (
            <TextInput
              id={id}
              value={name}
              maxLength={100}
              required
              onChange={(e) => setName(e.target.value)}
            />
          )}
        </Field>
        <Field label={t('notify.type')}>
          {(id) => (
            <Select id={id} value={type} onChange={(e) => setType(e.target.value as ChannelType)}>
              {CHANNEL_TYPES.map((k) => (
                <option key={k} value={k}>
                  {t(`notify.t.${k}` as Key)}
                </option>
              ))}
            </Select>
          )}
        </Field>
        {type === 'email' && <Hint tone="info">{t('notify.emailHint')}</Hint>}
        {type === 'webhook' && <Hint tone="info">{t('notify.webhookHint')}</Hint>}
        {FIELDS[type].map((f) => {
          const label =
            f.kind === 'number'
              ? t('notify.f.priority', { min: f.min ?? 0, max: f.max ?? 0 })
              : t(`notify.f.${f.key}` as Key);
          const value = values[f.key] ?? (f.initial !== undefined ? String(f.initial) : '');
          const set = (v: string) => setValues({ ...values, [f.key]: v });
          return (
            <Field key={`${type}-${f.key}`} label={label}>
              {(id) =>
                f.kind === 'lines' ? (
                  <TextArea
                    id={id}
                    rows={3}
                    required
                    placeholder={f.placeholder}
                    value={value}
                    onChange={(e) => set(e.target.value)}
                  />
                ) : (
                  <TextInput
                    id={id}
                    type={f.kind === 'number' ? 'number' : f.kind === 'url' ? 'url' : 'text'}
                    min={f.min}
                    max={f.max}
                    required
                    className={f.kind === 'number' ? 'w-32' : undefined}
                    placeholder={f.placeholder}
                    value={value}
                    onChange={(e) => set(e.target.value)}
                  />
                )
              }
            </Field>
          );
        })}
        {SECRETS[type].map((s) => {
          const has = stored.includes(s.key);
          const label = t(`notify.s.${s.key}` as Key);
          return (
            <Field
              key={`${type}-${s.key}`}
              label={s.required ? label : t('notify.optional', { label })}
              hint={has ? t('notify.secretKeep') : undefined}
            >
              {(id, describedBy) => (
                <TextInput
                  id={id}
                  type="password"
                  autoComplete="off"
                  aria-describedby={describedBy}
                  required={s.required && !has}
                  placeholder={has ? '••••••••' : undefined}
                  value={secrets[s.key] ?? ''}
                  onChange={(e) => setSecrets({ ...secrets, [s.key]: e.target.value })}
                />
              )}
            </Field>
          );
        })}
        <fieldset className="m-0 flex flex-col gap-2 border-0 p-0">
          <legend className="mb-1 font-semibold">{t('notify.events')}</legend>
          {EVENT_KEYS.map((k) => (
            <Toggle
              key={k}
              label={t(`notify.ev.${k}` as Key)}
              checked={events[k]}
              onChange={(v) => setEvents({ ...events, [k]: v })}
            />
          ))}
        </fieldset>
        <div className="flex flex-wrap items-center justify-between gap-3">
          <span aria-hidden className="font-semibold">
            {t('notify.lang')}
          </span>
          <Segmented<'de' | 'en'>
            label={t('notify.lang')}
            value={msgLang}
            onChange={setMsgLang}
            options={[
              { value: 'de', label: 'Deutsch' },
              { value: 'en', label: 'English' },
            ]}
          />
        </div>
        <Toggle label={t('notify.enabled')} checked={enabled} onChange={setEnabled} />
        <Toggle
          label={t('notify.isDefault')}
          description={t('notify.isDefaultHint')}
          checked={isDefault}
          onChange={setIsDefault}
        />
        <div className="flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={onClose}>
            {t('notify.cancel')}
          </Button>
          <Button type="submit" variant="primary" disabled={busy !== null}>
            {t('notify.save')}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

function LogDialog({ channel, onClose }: { channel: Channel; onClose: () => void }) {
  const { t, lang } = useT();
  const log = useResource(() => api.channelLog(channel.id), [channel.id]);
  const time = new Intl.DateTimeFormat(lang, { dateStyle: 'short', timeStyle: 'medium' });
  return (
    <Dialog open onClose={onClose} width="lg" title={t('notify.logTitle', { name: channel.name })}>
      {log.error && <ErrorBox error={log.error} retry={log.reload} />}
      {!log.data && !log.error && <Loading />}
      {log.data?.length === 0 && <p className="m-0 text-muted">{t('notify.logEmpty')}</p>}
      {log.data && log.data.length > 0 && (
        <div className="overflow-x-auto">
          <table className="w-full border-collapse text-left text-meta">
            <thead>
              <tr className="border-b border-hairline text-muted">
                <th className="py-2 pr-3 font-semibold">{t('notify.when')}</th>
                <th className="py-2 pr-3 font-semibold">{t('notify.event')}</th>
                <th className="py-2 font-semibold">{t('notify.result')}</th>
              </tr>
            </thead>
            <tbody>
              {log.data.map((row) => (
                <tr key={row.id} className="border-b border-hairline align-top">
                  <td className="py-2 pr-3 whitespace-nowrap">{time.format(row.at * 1000)}</td>
                  <td className="py-2 pr-3">
                    <div>{t(`notify.ev.${row.event}` as Key)}</div>
                    <div className="text-muted break-words">{row.title}</div>
                  </td>
                  <td className="py-2">
                    <Tag tone={row.ok ? 'success' : 'danger'}>
                      {t(row.ok ? 'notify.logOk' : 'notify.logFailed')}
                    </Tag>
                    {row.attempts > 1 && (
                      <span className="ml-2 text-muted">
                        {t('notify.attempts', { n: row.attempts })}
                      </span>
                    )}
                    {row.error && (
                      <div className="mt-1 break-words text-danger-ink">{row.error}</div>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Dialog>
  );
}

function SmtpCard() {
  const { t } = useT();
  const smtp = useResource(api.smtp, []);
  const { busy, run } = useAction();
  const [draft, setDraft] = useState<SmtpInput>();
  useEffect(() => {
    const s = smtp.data;
    if (s)
      setDraft({ host: s.host, port: s.port, security: s.security, user: s.user, from: s.from });
  }, [smtp.data]);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!draft) return;
    const saved = await run('smtp', () => api.saveSmtp(draft), t('notify.saved'));
    if (saved) smtp.set(saved);
  };
  const text = (key: 'host' | 'user' | 'from', label: Key, type = 'text') => (
    <Field label={t(label)}>
      {(id) => (
        <TextInput
          id={id}
          type={type}
          value={draft?.[key] ?? ''}
          onChange={(e) => draft && setDraft({ ...draft, [key]: e.target.value })}
        />
      )}
    </Field>
  );

  return (
    <Card title={t('smtp.title')} subtitle={t('smtp.hint')} icon={ICONS.mail}>
      {smtp.error && <ErrorBox error={smtp.error} retry={smtp.reload} />}
      {!draft && !smtp.error && <Loading />}
      {draft && (
        <form className="grid gap-4 sm:grid-cols-2" onSubmit={(e) => void submit(e)}>
          {text('host', 'smtp.host')}
          <Field label={t('smtp.port')}>
            {(id) => (
              <TextInput
                id={id}
                type="number"
                min={1}
                max={65535}
                className="w-32"
                value={String(draft.port)}
                onChange={(e) => setDraft({ ...draft, port: Number(e.target.value) })}
              />
            )}
          </Field>
          <Field label={t('smtp.security')}>
            {(id) => (
              <Select
                id={id}
                value={draft.security}
                onChange={(e) =>
                  setDraft({ ...draft, security: e.target.value as SmtpInput['security'] })
                }
              >
                <option value="starttls">{t('smtp.starttls')}</option>
                <option value="tls">{t('smtp.tls')}</option>
                <option value="none">{t('smtp.none')}</option>
              </Select>
            )}
          </Field>
          {text('from', 'smtp.from')}
          {text('user', 'smtp.user')}
          <Field
            label={t('smtp.password')}
            hint={smtp.data?.password_set ? t('notify.secretKeep') : undefined}
          >
            {(id, describedBy) => (
              <TextInput
                id={id}
                type="password"
                autoComplete="off"
                aria-describedby={describedBy}
                placeholder={smtp.data?.password_set ? '••••••••' : undefined}
                value={draft.password ?? ''}
                onChange={(e) =>
                  setDraft({
                    ...draft,
                    password: e.target.value === '' ? undefined : e.target.value,
                  })
                }
              />
            )}
          </Field>
          <div className="sm:col-span-2">
            <Button type="submit" variant="secondary" disabled={busy !== null}>
              {t('notify.save')}
            </Button>
          </div>
        </form>
      )}
    </Card>
  );
}
