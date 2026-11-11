// API tokens (created once, shown once), public badges per monitor and a short API note.

import { useState, type FormEvent } from 'react';
import {
  Button,
  Card,
  Dialog,
  Field,
  Hint,
  IconButton,
  ICONS,
  Switch,
  Tag,
  TextInput,
} from '@uwusuite/design';
import { pagesApi, when } from '../api/pages';
import type { ApiToken, MonitorChoice, Scope } from '../api/pages-types';
import { Empty, ErrorBox, Loading, PageHeader } from '../components/ui';
import { toasts, toastError, useAction, useResource } from '../lib/hooks';
import { useT, type Key } from '../lib/i18n';

const SCOPES: Scope[] = ['read', 'metrics'];

export function ApiTokens() {
  const { t } = useT();
  return (
    <>
      <PageHeader title={t('nav.apiTokens')} />
      <div className="flex flex-col gap-4">
        <Tokens />
        <Badges />
        <Card title={t('api.title')} icon={ICONS.info}>
          <ul className="m-0 flex flex-col gap-2 pl-5">
            <li>
              <code>/metrics</code> – {t('api.metrics')}
            </li>
            <li>
              <code>/api/v1/monitors</code> – {t('api.monitors')}
            </li>
            <li>
              <code>/api/v1/pages/{'{slug}'}/summary</code> – {t('api.summary')}
            </li>
          </ul>
          <p className="m-0 mt-3 text-meta text-muted">
            {t('api.header')} <code>Authorization: Bearer uwus_…</code>
          </p>
        </Card>
      </div>
    </>
  );
}

async function copy(text: string, done: string) {
  try {
    await navigator.clipboard.writeText(text);
    toasts.show(done, { tone: 'success' });
  } catch (error) {
    toastError(error);
  }
}

/** A read-only text with a copy button. */
function CopyField({ label, value }: { label: string; value: string }) {
  const { t } = useT();
  return (
    <Field label={label}>
      {(fid) => (
        <div className="flex gap-2">
          <TextInput
            id={fid}
            readOnly
            value={value}
            spellCheck={false}
            className="font-mono text-meta"
            onFocus={(e) => e.target.select()}
          />
          <IconButton
            icon={ICONS.copy}
            label={t('api.copy', { what: label })}
            onClick={() => void copy(value, t('api.copied'))}
          />
        </div>
      )}
    </Field>
  );
}

function Tokens() {
  const { t, lang } = useT();
  const tokens = useResource(pagesApi.tokens, []);
  const { busy, run } = useAction();
  const [name, setName] = useState('');
  const [scopes, setScopes] = useState<Scope[]>(['read']);
  const [created, setCreated] = useState<string>();
  const [deleting, setDeleting] = useState<ApiToken>();

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (scopes.length === 0) return;
    const token = await run('create', () => pagesApi.createToken(name.trim(), scopes));
    if (!token) return;
    setCreated(token.token);
    setName('');
    tokens.reload();
  };
  const remove = async (token: ApiToken) => {
    const done = await run('delete', async () => {
      await pagesApi.deleteToken(token.id);
      return true;
    });
    setDeleting(undefined);
    if (done) tokens.reload();
  };

  return (
    <Card title={t('tokens.title')} icon={ICONS.secret}>
      {tokens.error && <ErrorBox error={tokens.error} retry={tokens.reload} />}
      {tokens.loading && !tokens.data && <Loading />}
      {tokens.data?.length === 0 && <p className="m-0 text-muted">{t('tokens.none')}</p>}
      {!!tokens.data?.length && (
        <ul className="m-0 flex list-none flex-col divide-y divide-hairline p-0">
          {tokens.data.map((token) => (
            <li key={token.id} className="flex flex-wrap items-center gap-2 py-2">
              <span className="font-semibold break-words">{token.name}</span>
              {token.scopes.map((s) => (
                <Tag key={s} tone="pink">
                  {t(`tokens.scope.${s}` as Key)}
                </Tag>
              ))}
              <span className="ml-auto text-meta text-muted">
                {t('tokens.created', { time: when(token.created, lang) })} ·{' '}
                {token.last_used === null
                  ? t('tokens.neverUsed')
                  : t('tokens.lastUsed', { time: when(token.last_used, lang) })}
              </span>
              <IconButton
                size="sm"
                icon={ICONS.delete}
                label={t('tokens.delete', { name: token.name })}
                onClick={() => setDeleting(token)}
              />
            </li>
          ))}
        </ul>
      )}

      <form
        className="mt-4 flex flex-wrap items-end gap-4 border-t border-hairline pt-4"
        onSubmit={(e) => void submit(e)}
      >
        <Field label={t('tokens.name')} className="min-w-48 flex-1">
          {(fid) => (
            <TextInput
              id={fid}
              value={name}
              maxLength={100}
              onChange={(e) => setName(e.target.value)}
              required
            />
          )}
        </Field>
        <fieldset className="m-0 border-0 p-0">
          <legend className="mb-2 text-meta font-semibold">{t('tokens.scopes')}</legend>
          <div className="flex h-11 items-center gap-4">
            {SCOPES.map((s) => (
              <label key={s} className="flex items-center gap-2">
                <input
                  type="checkbox"
                  className="size-4 accent-pink"
                  checked={scopes.includes(s)}
                  onChange={(e) =>
                    setScopes(e.target.checked ? [...scopes, s] : scopes.filter((x) => x !== s))
                  }
                />
                {t(`tokens.scope.${s}` as Key)}
              </label>
            ))}
          </div>
        </fieldset>
        <Button
          type="submit"
          variant="primary"
          icon={ICONS.add}
          busy={busy === 'create'}
          disabled={busy !== null || scopes.length === 0}
        >
          {t('tokens.create')}
        </Button>
      </form>

      <Dialog
        open={created !== undefined}
        onClose={() => setCreated(undefined)}
        title={t('tokens.createdTitle')}
        width="md"
        closeOnOutsideClick={false}
        footer={
          <Button variant="primary" onClick={() => setCreated(undefined)}>
            {t('common.done')}
          </Button>
        }
      >
        <div className="flex flex-col gap-3">
          <Hint tone="warning">{t('tokens.once')}</Hint>
          <CopyField label={t('tokens.token')} value={created ?? ''} />
        </div>
      </Dialog>

      <Dialog
        open={deleting !== undefined}
        onClose={() => setDeleting(undefined)}
        title={t('tokens.deleteTitle')}
        tone="warning"
        width="sm"
        footer={
          <>
            <Button variant="ghost" onClick={() => setDeleting(undefined)}>
              {t('common.cancel')}
            </Button>
            <Button
              variant="danger"
              busy={busy === 'delete'}
              onClick={() => deleting && void remove(deleting)}
            >
              {t('common.delete')}
            </Button>
          </>
        }
      >
        <p className="m-0">{t('tokens.deleteBody', { name: deleting?.name ?? '' })}</p>
      </Dialog>
    </Card>
  );
}

function Badges() {
  const { t } = useT();
  const monitors = useResource(pagesApi.monitors, []);
  return (
    <Card title={t('badges.title')} subtitle={t('badges.intro')} icon={ICONS.verified}>
      {monitors.error && <ErrorBox error={monitors.error} retry={monitors.reload} />}
      {monitors.loading && !monitors.data && <Loading />}
      {monitors.data?.length === 0 && <Empty title={t('badges.none')} />}
      <ul className="m-0 flex list-none flex-col divide-y divide-hairline p-0">
        {monitors.data?.map((m) => (
          <BadgeRow key={m.id} monitor={m} />
        ))}
      </ul>
    </Card>
  );
}

function BadgeRow({ monitor }: { monitor: MonitorChoice }) {
  const { t } = useT();
  const badge = useResource(() => pagesApi.badge(monitor.id), [monitor.id]);
  const { busy, run } = useAction();
  const b = badge.data;

  const toggle = async (on: boolean) => {
    const saved = await run('save', () => pagesApi.saveBadge(monitor.id, on));
    if (saved) badge.set(saved);
  };
  const shown = b && (b.badge_public || b.on_page);

  return (
    <li className="py-3">
      <div className="flex flex-wrap items-center gap-3">
        <span className="font-semibold break-words">{monitor.name}</span>
        {monitor.group && <span className="text-meta text-muted">{monitor.group}</span>}
        {b?.on_page && <Tag tone="success">{t('badges.onPage')}</Tag>}
        {b && (
          <label className="ml-auto flex items-center gap-2 text-meta">
            <span aria-hidden="true">{t('badges.public')}</span>
            <Switch
              checked={b.badge_public || b.on_page}
              disabled={b.on_page || busy !== null}
              label={t('badges.publicFor', { name: monitor.name })}
              onChange={(on) => void toggle(on)}
            />
          </label>
        )}
      </div>
      {badge.error && <ErrorBox error={badge.error} retry={badge.reload} />}
      {b?.on_page && <p className="m-0 mt-1 text-meta text-muted">{t('badges.onPageHint')}</p>}
      {shown && (
        <div className="mt-3 grid gap-3 md:grid-cols-3">
          {(
            [
              ['badges.status', b.status_url],
              ['badges.uptime', b.uptime_url],
              ['badges.latency', b.latency_url],
            ] as const
          ).map(([label, url]) => (
            <div key={label} className="flex flex-col gap-2">
              <CopyField label={t(label)} value={location.origin + url} />
              <img src={url} alt={t('badges.preview', { what: t(label) })} className="h-5 w-fit" />
            </div>
          ))}
        </div>
      )}
    </li>
  );
}
