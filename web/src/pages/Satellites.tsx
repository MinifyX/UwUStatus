// Satellites: redeem a pairing code, see each tunnel live, rename or delete, and the dead man's
// switch that all satellites share.

import { useEffect, useState, type FormEvent } from 'react';
import {
  Button,
  Card,
  Dialog,
  Field,
  Hint,
  IconButton,
  ICONS,
  StatusLine,
  Switch,
  TextArea,
  TextInput,
} from '@uwusuite/design';
import { api } from '../api/client';
import { pagesApi, when } from '../api/pages';
import { satellitesApi, tunnelState, type DeadMan, type Satellite } from '../api/satellites';
import { Empty, ErrorBox, Loading, PageHeader } from '../components/ui';
import { useAction, useResource } from '../lib/hooks';
import { useT, type Key } from '../lib/i18n';

export function Satellites() {
  const { t } = useT();
  return (
    <>
      <PageHeader title={t('nav.satellites')} subtitle={t('sat.intro')} />
      <div className="flex flex-col gap-4">
        <List />
        <DeadManCard />
      </div>
    </>
  );
}

function List() {
  const { t } = useT();
  const list = useResource(satellitesApi.list, []);
  const { busy, run } = useAction();
  const [code, setCode] = useState('');
  const [name, setName] = useState('');
  const [region, setRegion] = useState('');
  const [deleting, setDeleting] = useState<Satellite>();
  const { reload } = list;

  // Tunnel state changes on its own; a cheap poll keeps the list honest.
  useEffect(() => {
    const timer = setInterval(reload, 10_000);
    return () => clearInterval(timer);
  }, [reload]);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const added = await run(
      'create',
      () => satellitesApi.create(code.trim(), name.trim(), region.trim()),
      t('sat.added'),
    );
    if (!added) return;
    setCode('');
    setName('');
    setRegion('');
    reload();
  };
  const remove = async (s: Satellite) => {
    const done = await run('delete', async () => {
      await satellitesApi.remove(s.id);
      return true;
    });
    setDeleting(undefined);
    if (done) reload();
  };

  return (
    <Card title={t('sat.title')} icon={ICONS.tunnel}>
      {list.error && <ErrorBox error={list.error} retry={reload} />}
      {list.loading && !list.data && <Loading />}
      {list.data?.length === 0 && <Empty title={t('sat.empty')} body={t('sat.emptyBody')} />}
      {!!list.data?.length && (
        <ul className="m-0 flex list-none flex-col divide-y divide-hairline p-0">
          {list.data.map((s) => (
            <Row key={s.id} sat={s} onDelete={() => setDeleting(s)} onSaved={reload} />
          ))}
        </ul>
      )}

      <form
        className="mt-4 flex flex-col gap-4 border-t border-hairline pt-4"
        onSubmit={(e) => void submit(e)}
      >
        <Field label={t('sat.code')} hint={t('sat.codeHint')}>
          {(fid) => (
            <TextInput
              id={fid}
              value={code}
              spellCheck={false}
              autoComplete="off"
              className="font-mono text-meta"
              onChange={(e) => setCode(e.target.value)}
              required
            />
          )}
        </Field>
        <div className="flex flex-wrap items-end gap-4">
          <Field label={t('sat.name')} hint={t('sat.nameHint')} className="min-w-48 flex-1">
            {(fid) => (
              <TextInput
                id={fid}
                value={name}
                maxLength={50}
                onChange={(e) => setName(e.target.value)}
                required
              />
            )}
          </Field>
          <Field label={t('sat.region')} className="min-w-48 flex-1">
            {(fid) => (
              <TextInput
                id={fid}
                value={region}
                maxLength={50}
                onChange={(e) => setRegion(e.target.value)}
              />
            )}
          </Field>
          <Button
            type="submit"
            variant="primary"
            icon={ICONS.connect}
            busy={busy === 'create'}
            disabled={busy !== null}
          >
            {t('sat.add')}
          </Button>
        </div>
      </form>

      <Dialog
        open={deleting !== undefined}
        onClose={() => setDeleting(undefined)}
        title={t('sat.deleteTitle', { name: deleting?.name ?? '' })}
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
        <p className="m-0">{t('sat.deleteBody')}</p>
      </Dialog>
    </Card>
  );
}

function Row({
  sat,
  onDelete,
  onSaved,
}: {
  sat: Satellite;
  onDelete: () => void;
  onSaved: () => void;
}) {
  const { t, lang } = useT();
  const { busy, run } = useAction();
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(sat.name);
  const [region, setRegion] = useState(sat.region);
  const state = tunnelState(sat);

  const save = async (event: FormEvent) => {
    event.preventDefault();
    const saved = await run('save', () => satellitesApi.save(sat.id, name.trim(), region.trim()));
    if (!saved) return;
    setEditing(false);
    onSaved();
  };

  return (
    <li className="flex flex-col gap-2 py-3">
      <div className="flex flex-wrap items-center gap-3">
        <span className="font-semibold break-words">{sat.name}</span>
        <code className="text-meta text-muted">{sat.id}</code>
        {sat.region && <span className="text-meta text-muted">{sat.region}</span>}
        <span className="ml-auto flex gap-1">
          <IconButton
            size="sm"
            icon={ICONS.edit}
            label={t('sat.edit', { name: sat.name })}
            onClick={() => setEditing(!editing)}
          />
          <IconButton
            size="sm"
            icon={ICONS.delete}
            label={t('sat.delete', { name: sat.name })}
            onClick={onDelete}
          />
        </span>
      </div>
      <StatusLine state={state}>
        {t(`sat.state.${state}` as Key)}
        {sat.live.since !== null && ` · ${when(sat.live.since, lang)}`}
      </StatusLine>
      <p className="m-0 text-meta text-muted">
        {sat.live.last_result !== null
          ? t('sat.lastResult', { time: when(sat.live.last_result, lang) })
          : sat.last_seen !== null
            ? t('sat.lastSeen', { time: when(sat.last_seen, lang) })
            : t('sat.neverSeen')}
        {sat.version && ` · v${sat.version}`}
      </p>
      {sat.live.error && !sat.live.connected && (
        <p className="m-0 text-meta break-words text-danger-ink">{sat.live.error}</p>
      )}
      {editing && (
        <form className="flex flex-wrap items-end gap-3" onSubmit={(e) => void save(e)}>
          <Field label={t('sat.name')} className="min-w-40 flex-1">
            {(fid) => (
              <TextInput
                id={fid}
                value={name}
                maxLength={50}
                onChange={(e) => setName(e.target.value)}
                required
              />
            )}
          </Field>
          <Field label={t('sat.region')} className="min-w-40 flex-1">
            {(fid) => (
              <TextInput
                id={fid}
                value={region}
                maxLength={50}
                onChange={(e) => setRegion(e.target.value)}
              />
            )}
          </Field>
          <Button type="submit" variant="primary" busy={busy === 'save'} disabled={busy !== null}>
            {t('common.save')}
          </Button>
        </form>
      )}
    </li>
  );
}

function DeadManCard() {
  const { t } = useT();
  const loaded = useResource(satellitesApi.deadMan, []);
  const channels = useResource(api.channels, []);
  const monitors = useResource(pagesApi.monitors, []);
  const { busy, run } = useAction();
  const [d, setD] = useState<DeadMan>();
  useEffect(() => setD(loaded.data), [loaded.data]);

  if (loaded.error) return <ErrorBox error={loaded.error} retry={loaded.reload} />;
  if (!d) return <Loading />;

  const toggle = (list: number[], id: number, on: boolean) =>
    on ? [...list, id] : list.filter((x) => x !== id);
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const saved = await run('save', () => satellitesApi.saveDeadMan(d), t('sat.dm.saved'));
    if (saved) loaded.set(saved);
  };

  return (
    <Card title={t('sat.dm.title')} subtitle={t('sat.dm.intro')} icon={ICONS.reminder}>
      <form className="flex flex-col gap-4" onSubmit={(e) => void submit(e)}>
        <Field label={t('sat.dm.after')} hint={t('sat.dm.afterHint')}>
          {(fid) => (
            <TextInput
              id={fid}
              inputMode="numeric"
              className="w-32"
              value={String(d.after_min)}
              onChange={(e) => setD({ ...d, after_min: Number(e.target.value.replace(/\D/g, '')) })}
            />
          )}
        </Field>
        <fieldset className="m-0 border-0 p-0">
          <legend className="mb-2 text-meta font-semibold">{t('sat.dm.channels')}</legend>
          {channels.error && <ErrorBox error={channels.error} retry={channels.reload} />}
          {channels.data?.length === 0 && (
            <p className="m-0 text-meta text-muted">{t('sat.dm.noChannels')}</p>
          )}
          <div className="flex flex-wrap gap-x-5 gap-y-2">
            {channels.data?.map((c) => (
              <label key={c.id} className="flex items-center gap-2">
                <input
                  type="checkbox"
                  className="size-4 accent-pink"
                  checked={d.channel_ids.includes(c.id)}
                  onChange={(e) =>
                    setD({ ...d, channel_ids: toggle(d.channel_ids, c.id, e.target.checked) })
                  }
                />
                {c.name}
              </label>
            ))}
          </div>
        </fieldset>
        <Hint tone="info">{t('sat.dm.secrets')}</Hint>
        <div className="grid gap-4 md:grid-cols-2">
          <Field label={t('sat.dm.textDe')} hint={t('sat.dm.textHint')}>
            {(fid) => (
              <TextArea
                id={fid}
                rows={3}
                maxLength={1000}
                value={d.text_de}
                onChange={(e) => setD({ ...d, text_de: e.target.value })}
              />
            )}
          </Field>
          <Field label={t('sat.dm.textEn')} hint={t('sat.dm.textHint')}>
            {(fid) => (
              <TextArea
                id={fid}
                rows={3}
                maxLength={1000}
                value={d.text_en}
                onChange={(e) => setD({ ...d, text_en: e.target.value })}
              />
            )}
          </Field>
        </div>

        <div className="flex items-center gap-3 border-t border-hairline pt-4">
          <Switch
            checked={d.page_enabled}
            label={t('sat.dm.page')}
            onChange={(on) => setD({ ...d, page_enabled: on })}
          />
          <span aria-hidden="true" className="font-semibold">
            {t('sat.dm.page')}
          </span>
        </div>
        <p className="m-0 text-meta text-muted">{t('sat.dm.pageHint')}</p>
        {d.page_enabled && (
          <>
            <div className="flex flex-wrap gap-4">
              <Field label={t('sat.dm.pageTitle')} className="min-w-48 flex-1">
                {(fid) => (
                  <TextInput
                    id={fid}
                    value={d.page_title}
                    maxLength={100}
                    onChange={(e) => setD({ ...d, page_title: e.target.value })}
                  />
                )}
              </Field>
              <Field
                label={t('sat.dm.pageDomain')}
                hint={t('sat.dm.pageDomainHint')}
                className="min-w-48 flex-1"
              >
                {(fid) => (
                  <TextInput
                    id={fid}
                    value={d.page_domain}
                    maxLength={253}
                    spellCheck={false}
                    placeholder="notfall.example.com"
                    onChange={(e) => setD({ ...d, page_domain: e.target.value })}
                  />
                )}
              </Field>
            </div>
            <fieldset className="m-0 border-0 p-0">
              <legend className="mb-2 text-meta font-semibold">{t('sat.dm.pageMonitors')}</legend>
              {monitors.error && <ErrorBox error={monitors.error} retry={monitors.reload} />}
              <div className="flex flex-wrap gap-x-5 gap-y-2">
                {monitors.data?.map((m) => (
                  <label key={m.id} className="flex items-center gap-2">
                    <input
                      type="checkbox"
                      className="size-4 accent-pink"
                      checked={d.page_monitors.includes(m.id)}
                      onChange={(e) =>
                        setD({
                          ...d,
                          page_monitors: toggle(d.page_monitors, m.id, e.target.checked),
                        })
                      }
                    />
                    {m.name}
                  </label>
                ))}
              </div>
            </fieldset>
          </>
        )}
        <div>
          <Button type="submit" variant="primary" busy={busy === 'save'} disabled={busy !== null}>
            {t('common.save')}
          </Button>
        </div>
      </form>
    </Card>
  );
}
