// Incidents: the list (open/all), reporting one by hand, and the detail with its timeline.

import { useEffect, useState, type FormEvent } from 'react';
import {
  Button,
  Card,
  Dialog,
  Field,
  ICONS,
  Segmented,
  Select,
  Tag,
  TextArea,
  TextInput,
  type TagTone,
} from '@uwusuite/design';
import { pagesApi, when } from '../api/pages';
import {
  IMPACTS,
  INCIDENT_STATUSES,
  type Impact,
  type Incident,
  type IncidentStatus,
  type IncidentUpdate,
  type MonitorChoice,
} from '../api/pages-types';
import { Empty, ErrorBox, Link, Loading, PageHeader } from '../components/ui';
import { useAction, useResource } from '../lib/hooks';
import { useT, type Key } from '../lib/i18n';
import { hrefOf, navigate } from '../lib/router';

const STATUS_TONE: Record<IncidentStatus, TagTone> = {
  investigating: 'danger',
  identified: 'warning',
  monitoring: 'pink',
  resolved: 'success',
};
const IMPACT_TONE: Record<Impact, TagTone> = {
  down: 'danger',
  degraded: 'warning',
  none: 'neutral',
};

function StatusTag({ status }: { status: IncidentStatus }) {
  const { t } = useT();
  return <Tag tone={STATUS_TONE[status]}>{t(`incident.status.${status}` as Key)}</Tag>;
}
function ImpactTag({ impact }: { impact: Impact }) {
  const { t } = useT();
  return <Tag tone={IMPACT_TONE[impact]}>{t(`incident.impact.${impact}` as Key)}</Tag>;
}

export function Incidents({ id }: { id?: number }) {
  return id === undefined ? <IncidentList /> : <IncidentDetail key={id} id={id} />;
}

function IncidentList() {
  const { t, lang } = useT();
  const [filter, setFilter] = useState<'open' | 'all'>('open');
  const [reporting, setReporting] = useState(false);
  const list = useResource(() => pagesApi.incidents(filter, lang), [filter, lang]);
  return (
    <>
      <PageHeader
        title={t('nav.incidents')}
        actions={
          <Button variant="primary" icon={ICONS.add} onClick={() => setReporting(true)}>
            {t('incident.new')}
          </Button>
        }
      />
      <Segmented<'open' | 'all'>
        className="mb-4"
        label={t('incident.filter')}
        value={filter}
        onChange={setFilter}
        options={[
          { value: 'open', label: t('incident.filterOpen') },
          { value: 'all', label: t('incident.filterAll') },
        ]}
      />
      {list.error && <ErrorBox error={list.error} retry={list.reload} />}
      {list.loading && !list.data && <Loading />}
      {list.data?.length === 0 && (
        <Empty
          mood="happy"
          title={filter === 'open' ? t('incident.noneOpen') : t('incident.none')}
        />
      )}
      {!!list.data?.length && (
        <ul className="m-0 flex list-none flex-col gap-3 p-0">
          {list.data.map((incident) => (
            <li key={incident.id}>
              <Card
                title={
                  <Link href={hrefOf({ name: 'incidents', id: incident.id })}>
                    {incident.display_title}
                  </Link>
                }
                subtitle={`${when(incident.started, lang)} · ${incident.monitors.map((m) => m.name).join(', ')}`}
                aside={
                  <div className="flex flex-wrap gap-1">
                    <StatusTag status={incident.status} />
                    <ImpactTag impact={incident.impact} />
                    {incident.auto && <Tag>{t('incident.auto')}</Tag>}
                  </div>
                }
              />
            </li>
          ))}
        </ul>
      )}
      <NewIncidentDialog
        open={reporting}
        onClose={() => setReporting(false)}
        onCreated={(incident) => navigate(hrefOf({ name: 'incidents', id: incident.id }))}
      />
    </>
  );
}

/** Checkboxes for the monitors an incident is about. */
function MonitorChecks({
  choices,
  value,
  onChange,
}: {
  choices: MonitorChoice[];
  value: number[];
  onChange: (ids: number[]) => void;
}) {
  const { t } = useT();
  return (
    <fieldset className="m-0 border-0 p-0">
      <legend className="mb-2 text-meta font-semibold">{t('incident.monitors')}</legend>
      {choices.length === 0 && <p className="m-0 text-muted">{t('incident.noMonitors')}</p>}
      <div className="flex max-h-56 flex-col gap-1 overflow-y-auto">
        {choices.map((m) => (
          <label key={m.id} className="flex items-center gap-2">
            <input
              type="checkbox"
              className="size-4 accent-pink"
              checked={value.includes(m.id)}
              onChange={(e) =>
                onChange(e.target.checked ? [...value, m.id] : value.filter((x) => x !== m.id))
              }
            />
            {m.name}
            {m.group && <span className="text-meta text-muted">· {m.group}</span>}
          </label>
        ))}
      </div>
    </fieldset>
  );
}

function StatusSelect({
  value,
  onChange,
}: {
  value: IncidentStatus;
  onChange: (s: IncidentStatus) => void;
}) {
  const { t } = useT();
  return (
    <Field label={t('incident.status')}>
      {(fid) => (
        <Select id={fid} value={value} onChange={(e) => onChange(e.target.value as IncidentStatus)}>
          {INCIDENT_STATUSES.map((s) => (
            <option key={s} value={s}>
              {t(`incident.status.${s}` as Key)}
            </option>
          ))}
        </Select>
      )}
    </Field>
  );
}

function ImpactSelect({ value, onChange }: { value: Impact; onChange: (i: Impact) => void }) {
  const { t } = useT();
  return (
    <Field label={t('incident.impact')}>
      {(fid) => (
        <Select id={fid} value={value} onChange={(e) => onChange(e.target.value as Impact)}>
          {IMPACTS.map((i) => (
            <option key={i} value={i}>
              {t(`incident.impact.${i}` as Key)}
            </option>
          ))}
        </Select>
      )}
    </Field>
  );
}

function NewIncidentDialog({
  open,
  onClose,
  onCreated,
}: {
  open: boolean;
  onClose: () => void;
  onCreated: (incident: Incident) => void;
}) {
  const { t } = useT();
  const monitors = useResource(pagesApi.monitors, []);
  const { busy, run } = useAction();
  const [title, setTitle] = useState('');
  const [body, setBody] = useState('');
  const [status, setStatus] = useState<IncidentStatus>('investigating');
  const [impact, setImpact] = useState<Impact>('degraded');
  const [ids, setIds] = useState<number[]>([]);
  const dirty = title !== '' || body !== '' || ids.length > 0;

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (ids.length === 0) return;
    const created = await run(
      'create',
      () => pagesApi.createIncident({ title, body, status, impact, monitors: ids }),
      t('incident.created'),
    );
    if (created) onCreated(created);
  };

  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={t('incident.new')}
      width="md"
      closeOnOutsideClick={!dirty}
    >
      <form className="flex flex-col gap-4" onSubmit={(e) => void submit(e)}>
        <Field label={t('incident.title')}>
          {(fid) => (
            <TextInput id={fid} value={title} onChange={(e) => setTitle(e.target.value)} required />
          )}
        </Field>
        <div className="grid gap-4 sm:grid-cols-2">
          <StatusSelect value={status} onChange={setStatus} />
          <ImpactSelect value={impact} onChange={setImpact} />
        </div>
        <Field label={t('incident.body')}>
          {(fid) => (
            <TextArea id={fid} rows={4} value={body} onChange={(e) => setBody(e.target.value)} />
          )}
        </Field>
        {monitors.error && <ErrorBox error={monitors.error} retry={monitors.reload} />}
        <MonitorChecks choices={monitors.data ?? []} value={ids} onChange={setIds} />
        {ids.length === 0 && (
          <p className="m-0 text-meta text-muted" role="status">
            {t('incident.pickOne')}
          </p>
        )}
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose}>
            {t('common.cancel')}
          </Button>
          <Button
            type="submit"
            variant="primary"
            busy={busy === 'create'}
            disabled={busy !== null || ids.length === 0}
          >
            {t('incident.create')}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

function IncidentDetail({ id }: { id: number }) {
  const { t, lang } = useT();
  const incident = useResource(() => pagesApi.incident(id, lang), [id, lang]);
  const monitors = useResource(pagesApi.monitors, []);
  const { busy, run } = useAction();
  const [title, setTitle] = useState('');
  const [impact, setImpact] = useState<Impact>('none');
  const [ids, setIds] = useState<number[]>([]);
  const [status, setStatus] = useState<IncidentStatus>('investigating');
  const [body, setBody] = useState('');
  const [confirmDelete, setConfirmDelete] = useState(false);

  const data = incident.data;
  useEffect(() => {
    if (!data) return;
    setTitle(data.title);
    setImpact(data.impact);
    setIds(data.monitors.map((m) => m.id));
    setStatus(data.status);
  }, [data]);

  if (incident.error) return <ErrorBox error={incident.error} retry={incident.reload} />;
  if (!data) return <Loading />;

  // The incident's own monitors stay pickable even if the list does not have them (yet).
  const choices = [
    ...(monitors.data ?? []),
    ...data.monitors
      .filter((m) => !monitors.data?.some((c) => c.id === m.id))
      .map((m) => ({ ...m, group: '' })),
  ];

  const save = async (event: FormEvent) => {
    event.preventDefault();
    if (ids.length === 0) return;
    const saved = await run(
      'save',
      () => pagesApi.saveIncident(id, { title, impact, monitors: ids }),
      t('incident.saved'),
    );
    if (saved) incident.set(saved);
  };
  const addUpdate = async (event: FormEvent) => {
    event.preventDefault();
    const saved = await run(
      'update',
      () => pagesApi.addUpdate(id, status, body),
      t('incident.updateAdded'),
    );
    if (saved) {
      incident.set(saved);
      setBody('');
    }
  };
  const remove = async () => {
    const done = await run('delete', async () => {
      await pagesApi.deleteIncident(id);
      return true;
    });
    if (done) navigate('/incidents');
  };

  return (
    <>
      <PageHeader
        title={data.display_title}
        back={{ href: '/incidents', label: t('nav.incidents') }}
        subtitle={
          <span className="flex flex-wrap items-center gap-2">
            <StatusTag status={data.status} />
            <ImpactTag impact={data.impact} />
            {data.auto && <Tag>{t('incident.auto')}</Tag>}
            <span>
              {t('incident.since', { time: when(data.started, lang) })}
              {data.resolved !== null &&
                ` · ${t('incident.resolvedAt', { time: when(data.resolved, lang) })}`}
            </span>
          </span>
        }
        actions={
          <Button variant="danger" icon={ICONS.delete} onClick={() => setConfirmDelete(true)}>
            {t('common.delete')}
          </Button>
        }
      />
      <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(0,1.3fr)]">
        <Card title={t('incident.details')} icon={ICONS.edit}>
          <form className="flex flex-col gap-4" onSubmit={(e) => void save(e)}>
            <Field label={t('incident.title')} hint={t('incident.titleHint')}>
              {(fid, describedBy) => (
                <TextInput
                  id={fid}
                  aria-describedby={describedBy}
                  placeholder={data.display_title}
                  value={title}
                  onChange={(e) => setTitle(e.target.value)}
                />
              )}
            </Field>
            <ImpactSelect value={impact} onChange={setImpact} />
            <MonitorChecks choices={choices} value={ids} onChange={setIds} />
            <div>
              <Button
                type="submit"
                variant="primary"
                busy={busy === 'save'}
                disabled={busy !== null || ids.length === 0}
              >
                {t('common.save')}
              </Button>
            </div>
          </form>
        </Card>

        <Card title={t('incident.timeline')} icon={ICONS.history}>
          <ol className="m-0 flex list-none flex-col gap-3 p-0">
            {data.updates.map((u) => (
              <UpdateItem
                key={u.id}
                incident={id}
                update={u}
                onSaved={(saved) => incident.set(saved)}
              />
            ))}
          </ol>
          <form
            className="mt-4 flex flex-col gap-3 border-t border-hairline pt-4"
            onSubmit={(e) => void addUpdate(e)}
          >
            <h3 className="m-0 text-body font-semibold">{t('incident.newUpdate')}</h3>
            <StatusSelect value={status} onChange={setStatus} />
            <Field label={t('incident.body')}>
              {(fid) => (
                <TextArea
                  id={fid}
                  rows={3}
                  value={body}
                  onChange={(e) => setBody(e.target.value)}
                  required
                />
              )}
            </Field>
            <div>
              <Button
                type="submit"
                icon={ICONS.send}
                busy={busy === 'update'}
                disabled={busy !== null}
              >
                {t('incident.post')}
              </Button>
            </div>
          </form>
        </Card>
      </div>

      <Dialog
        open={confirmDelete}
        onClose={() => setConfirmDelete(false)}
        title={t('incident.deleteTitle')}
        tone="warning"
        width="sm"
        footer={
          <>
            <Button variant="ghost" onClick={() => setConfirmDelete(false)}>
              {t('common.cancel')}
            </Button>
            <Button variant="danger" busy={busy === 'delete'} onClick={() => void remove()}>
              {t('common.delete')}
            </Button>
          </>
        }
      >
        <p className="m-0">{t('incident.deleteBody')}</p>
      </Dialog>
    </>
  );
}

function UpdateItem({
  incident,
  update,
  onSaved,
}: {
  incident: number;
  update: IncidentUpdate;
  onSaved: (incident: Incident) => void;
}) {
  const { t, lang } = useT();
  const { busy, run } = useAction();
  const [editing, setEditing] = useState(false);
  const [body, setBody] = useState(update.body);

  const save = async (event: FormEvent) => {
    event.preventDefault();
    const saved = await run('save', () => pagesApi.editUpdate(incident, update.id, body));
    if (saved) {
      onSaved(saved);
      setEditing(false);
    }
  };

  return (
    <li className="border-l-2 border-hairline pl-3">
      <div className="flex flex-wrap items-center gap-2 text-meta text-muted">
        <StatusTag status={update.status} />
        <time dateTime={new Date(update.at * 1000).toISOString()}>{when(update.at, lang)}</time>
        {!editing && (
          <Button
            size="sm"
            variant="ghost"
            icon={ICONS.edit}
            onClick={() => {
              setBody(update.body);
              setEditing(true);
            }}
          >
            {t('incident.editUpdate')}
          </Button>
        )}
      </div>
      {editing ? (
        <form className="mt-2 flex flex-col gap-2" onSubmit={(e) => void save(e)}>
          <Field label={t('incident.body')} hint={t('incident.bodyHint')}>
            {(fid, describedBy) => (
              <TextArea
                id={fid}
                aria-describedby={describedBy}
                rows={3}
                placeholder={update.display_body}
                value={body}
                onChange={(e) => setBody(e.target.value)}
              />
            )}
          </Field>
          <div className="flex gap-2">
            <Button type="submit" size="sm" variant="primary" busy={busy === 'save'}>
              {t('common.save')}
            </Button>
            <Button size="sm" variant="ghost" onClick={() => setEditing(false)}>
              {t('common.cancel')}
            </Button>
          </div>
        </form>
      ) : (
        <p className="m-0 mt-1 whitespace-pre-wrap">{update.display_body}</p>
      )}
    </li>
  );
}
