// Status pages: the list and the editor (fields, logo, groups of monitors).

import { useEffect, useMemo, useState, type FormEvent } from 'react';
import {
  Button,
  Card,
  Dialog,
  Field,
  IconButton,
  ICONS,
  Select,
  Switch,
  Tag,
  TextArea,
  TextInput,
  Toggle,
} from '@uwusuite/design';
import {
  emptyPage,
  LOGO_MAX_BYTES,
  LOGO_TYPES,
  moveItem,
  pagesApi,
  SLUG_RE,
  splitDomains,
} from '../api/pages';
import {
  ACCENTS,
  type MonitorChoice,
  type PageGroup,
  type PageInput,
  type StatusPage,
} from '../api/pages-types';
import { ButtonLink, Empty, ErrorBox, Link, Loading, PageHeader } from '../components/ui';
import { toasts, useAction, useResource } from '../lib/hooks';
import { useT, type Key } from '../lib/i18n';
import { hrefOf, navigate } from '../lib/router';

export function StatusPages({ id }: { id?: number | 'new' }) {
  return id === undefined ? <PageList /> : <PageEditor key={id} id={id} />;
}

function PageList() {
  const { t } = useT();
  const pages = useResource(pagesApi.pages, []);
  return (
    <>
      <PageHeader
        title={t('nav.pages')}
        actions={
          <ButtonLink href="/pages/new" variant="primary" icon={ICONS.add}>
            {t('pages.new')}
          </ButtonLink>
        }
      />
      {pages.error && <ErrorBox error={pages.error} retry={pages.reload} />}
      {pages.loading && !pages.data && <Loading />}
      {pages.data?.length === 0 && <Empty title={t('pages.empty')} body={t('pages.emptyBody')} />}
      {!!pages.data?.length && (
        <ul className="m-0 flex list-none flex-col gap-3 p-0">
          {pages.data.map((page) => (
            <li key={page.id}>
              <Card
                title={<Link href={hrefOf({ name: 'pages', id: page.id })}>{page.title}</Link>}
                subtitle={[page.preview_url, ...page.domains].join(' · ')}
                aside={<PreviewLink page={page} />}
              />
            </li>
          ))}
        </ul>
      )}
    </>
  );
}

function PreviewLink({ page }: { page: StatusPage }) {
  const { t } = useT();
  return (
    <ButtonLink
      external
      size="sm"
      href={page.preview_url}
      target="_blank"
      rel="noopener"
      icon={ICONS.openExternal}
    >
      {t('pages.preview')}
      <span className="sr-only"> {t('common.newTab')}</span>
    </ButtonLink>
  );
}

function toInput(page: StatusPage): PageInput {
  const { slug, title, description, domains, lang, auto_lang, accent, nyu, timezone, groups } =
    page;
  return { slug, title, description, domains, lang, auto_lang, accent, nyu, timezone, groups };
}

// Whole class names, so Tailwind finds them.
const SWATCH: Record<(typeof ACCENTS)[number], string> = {
  pink: 'bg-account-pink',
  violet: 'bg-account-violet',
  sky: 'bg-account-sky',
  mint: 'bg-account-mint',
  amber: 'bg-account-amber',
  coral: 'bg-account-coral',
};

const TIMEZONES = (() => {
  try {
    return Intl.supportedValuesOf('timeZone');
  } catch {
    return [];
  }
})();

function PageEditor({ id }: { id: number | 'new' }) {
  const { t } = useT();
  const page = useResource(
    () => (id === 'new' ? Promise.resolve(undefined) : pagesApi.page(id)),
    [id],
  );
  const monitors = useResource(pagesApi.monitors, []);
  const { busy, run } = useAction();
  const [draft, setDraft] = useState<PageInput>();
  const [domains, setDomains] = useState('');
  const [confirmDelete, setConfirmDelete] = useState(false);

  useEffect(() => {
    if (page.loading) return;
    const input = page.data ? toInput(page.data) : emptyPage();
    setDraft(input);
    setDomains(input.domains.join('\n'));
  }, [page.data, page.loading]);

  if (page.error) return <ErrorBox error={page.error} retry={page.reload} />;
  if (!draft) return <Loading />;

  const set = (patch: Partial<PageInput>) => setDraft({ ...draft, ...patch });
  const slugBad = draft.slug !== '' && !SLUG_RE.test(draft.slug);
  const setGroups = (groups: PageGroup[]) => set({ groups });

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (!SLUG_RE.test(draft.slug)) return;
    const input = { ...draft, domains: splitDomains(domains) };
    const saved = await run(
      'save',
      () => (id === 'new' ? pagesApi.createPage(input) : pagesApi.savePage(id, input)),
      t('pages.saved'),
    );
    if (!saved) return;
    if (id === 'new') navigate(hrefOf({ name: 'pages', id: saved.id }), true);
    else page.set(saved);
  };

  const remove = async () => {
    if (id === 'new') return;
    const done = await run('delete', async () => {
      await pagesApi.deletePage(id);
      return true;
    });
    if (done) navigate('/pages');
  };

  return (
    <>
      <PageHeader
        title={id === 'new' ? t('pages.new') : page.data?.title || t('nav.pages')}
        back={{ href: '/pages', label: t('nav.pages') }}
        actions={
          page.data && (
            <>
              <PreviewLink page={page.data} />
              <Button
                variant="danger"
                icon={ICONS.delete}
                onClick={() => setConfirmDelete(true)}
                disabled={busy !== null}
              >
                {t('common.delete')}
              </Button>
            </>
          )
        }
      />
      <form className="flex flex-col gap-4" onSubmit={(e) => void submit(e)}>
        <Card title={t('pages.basics')} icon={ICONS.website}>
          <div className="grid gap-4 md:grid-cols-2">
            <Field label={t('pages.title')}>
              {(fid) => (
                <TextInput
                  id={fid}
                  value={draft.title}
                  onChange={(e) => set({ title: e.target.value })}
                  required
                />
              )}
            </Field>
            <Field
              label={t('pages.slug')}
              hint={t('pages.slugHint')}
              error={slugBad ? t('pages.slugBad') : undefined}
            >
              {(fid, describedBy) => (
                <TextInput
                  id={fid}
                  aria-describedby={describedBy}
                  aria-invalid={slugBad || undefined}
                  value={draft.slug}
                  maxLength={50}
                  autoCapitalize="none"
                  spellCheck={false}
                  onChange={(e) => set({ slug: e.target.value.toLowerCase() })}
                  required
                />
              )}
            </Field>
            <Field label={t('pages.description')} className="md:col-span-2">
              {(fid) => (
                <TextArea
                  id={fid}
                  rows={3}
                  value={draft.description}
                  onChange={(e) => set({ description: e.target.value })}
                />
              )}
            </Field>
            <Field label={t('pages.domains')} hint={t('pages.domainsHint')}>
              {(fid, describedBy) => (
                <TextArea
                  id={fid}
                  aria-describedby={describedBy}
                  rows={3}
                  spellCheck={false}
                  placeholder="status.example.com"
                  value={domains}
                  onChange={(e) => setDomains(e.target.value)}
                />
              )}
            </Field>
            <Field label={t('pages.timezone')}>
              {(fid) => (
                <>
                  <TextInput
                    id={fid}
                    list="uwus-timezones"
                    value={draft.timezone}
                    onChange={(e) => set({ timezone: e.target.value })}
                    required
                  />
                  <datalist id="uwus-timezones">
                    {TIMEZONES.map((z) => (
                      <option key={z} value={z} />
                    ))}
                  </datalist>
                </>
              )}
            </Field>
            <Field label={t('pages.lang')}>
              {(fid) => (
                <Select
                  id={fid}
                  value={draft.lang}
                  onChange={(e) => set({ lang: e.target.value as PageInput['lang'] })}
                >
                  <option value="de">Deutsch</option>
                  <option value="en">English</option>
                </Select>
              )}
            </Field>
            <div className="flex flex-col gap-3">
              <Toggle
                label={t('pages.autoLang')}
                description={t('pages.autoLangHint')}
                checked={draft.auto_lang}
                onChange={(auto_lang) => set({ auto_lang })}
              />
              <Toggle label={t('pages.nyu')} checked={draft.nyu} onChange={(nyu) => set({ nyu })} />
            </div>
            <fieldset className="m-0 border-0 p-0 md:col-span-2">
              <legend className="mb-2 text-meta font-semibold">{t('pages.accent')}</legend>
              <div className="flex flex-wrap gap-2">
                {ACCENTS.map((accent) => (
                  <label
                    key={accent}
                    className={`flex h-10 cursor-pointer items-center gap-2 rounded-full border px-3 text-[13.5px] has-focus-visible:shadow-focus ${
                      draft.accent === accent
                        ? 'border-pink bg-pink-tint font-semibold text-pink-ink'
                        : 'border-control'
                    }`}
                  >
                    <input
                      type="radio"
                      name="accent"
                      value={accent}
                      checked={draft.accent === accent}
                      onChange={() => set({ accent })}
                      className="sr-only"
                    />
                    <span
                      aria-hidden="true"
                      className={`size-4 rounded-full border border-hairline ${SWATCH[accent]}`}
                    />
                    {t(`accent.${accent}` as Key)}
                    {draft.accent === accent && <span aria-hidden="true">✓</span>}
                  </label>
                ))}
              </div>
            </fieldset>
          </div>
        </Card>

        <Card
          title={t('pages.groups')}
          icon={ICONS.collection}
          aside={
            <Button
              size="sm"
              icon={ICONS.add}
              onClick={() =>
                setGroups([
                  ...draft.groups,
                  { name: t('pages.groupDefault', { n: draft.groups.length + 1 }), monitors: [] },
                ])
              }
            >
              {t('pages.addGroup')}
            </Button>
          }
        >
          {monitors.error && <ErrorBox error={monitors.error} retry={monitors.reload} />}
          {draft.groups.length === 0 && <p className="m-0 text-muted">{t('pages.noGroups')}</p>}
          <ol className="m-0 flex list-none flex-col gap-4 p-0">
            {draft.groups.map((group, i) => (
              <GroupEditor
                key={i}
                group={group}
                index={i}
                count={draft.groups.length}
                choices={monitors.data ?? []}
                onChange={(g) => setGroups(draft.groups.map((old, j) => (j === i ? g : old)))}
                onMove={(by) => setGroups(moveItem(draft.groups, i, by))}
                onRemove={() => setGroups(draft.groups.filter((_, j) => j !== i))}
              />
            ))}
          </ol>
        </Card>

        <div>
          <Button type="submit" variant="primary" busy={busy === 'save'} disabled={busy !== null}>
            {id === 'new' ? t('pages.create') : t('common.save')}
          </Button>
        </div>
      </form>

      {page.data && <LogoCard page={page.data} onChange={page.reload} />}

      <Dialog
        open={confirmDelete}
        onClose={() => setConfirmDelete(false)}
        title={t('pages.deleteTitle')}
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
        <p className="m-0">{t('pages.deleteBody', { title: page.data?.title ?? '' })}</p>
      </Dialog>
    </>
  );
}

function GroupEditor({
  group,
  index,
  count,
  choices,
  onChange,
  onMove,
  onRemove,
}: {
  group: PageGroup;
  index: number;
  count: number;
  choices: MonitorChoice[];
  onChange: (group: PageGroup) => void;
  onMove: (by: -1 | 1) => void;
  onRemove: () => void;
}) {
  const { t } = useT();
  const [pick, setPick] = useState('');
  const byId = useMemo(() => new Map(choices.map((m) => [m.id, m])), [choices]);
  const free = choices.filter((m) => !group.monitors.some((g) => g.id === m.id));
  const byGroup = new Map<string, MonitorChoice[]>();
  for (const m of free) byGroup.set(m.group, [...(byGroup.get(m.group) ?? []), m]);
  const label = group.name || t('pages.groupN', { n: index + 1 });
  const monitors = group.monitors;
  const setMonitors = (list: PageGroup['monitors']) => onChange({ ...group, monitors: list });

  return (
    <li className="rounded-card border border-hairline p-4">
      <div className="flex flex-wrap items-end gap-2">
        <Field label={t('pages.groupName')} className="min-w-48 flex-1">
          {(fid) => (
            <TextInput
              id={fid}
              value={group.name}
              onChange={(e) => onChange({ ...group, name: e.target.value })}
              required
            />
          )}
        </Field>
        <IconButton
          icon={ICONS.moveUp}
          label={t('pages.groupUp', { name: label })}
          disabled={index === 0}
          onClick={() => onMove(-1)}
        />
        <IconButton
          icon={ICONS.moveDown}
          label={t('pages.groupDown', { name: label })}
          disabled={index === count - 1}
          onClick={() => onMove(1)}
        />
        <IconButton
          icon={ICONS.delete}
          label={t('pages.groupRemove', { name: label })}
          onClick={onRemove}
        />
      </div>

      {monitors.length > 0 && (
        <ol className="m-0 mt-3 flex list-none flex-col gap-2 p-0">
          {monitors.map((m, i) => {
            const real = byId.get(m.id)?.name ?? `#${m.id}`;
            return (
              <li
                key={m.id}
                className="flex flex-wrap items-center gap-2 rounded-control bg-canvas px-3 py-2"
              >
                <span className="min-w-32 flex-1 font-semibold break-words">{real}</span>
                <TextInput
                  aria-label={t('pages.displayName', { name: real })}
                  placeholder={real}
                  className="w-56 max-w-full"
                  value={m.name}
                  onChange={(e) =>
                    setMonitors(
                      monitors.map((o, j) => (j === i ? { ...o, name: e.target.value } : o)),
                    )
                  }
                />
                <label className="flex items-center gap-2 text-meta">
                  <Switch
                    size="sm"
                    checked={m.show_latency}
                    label={t('pages.latencyFor', { name: real })}
                    onChange={(show_latency) =>
                      setMonitors(monitors.map((o, j) => (j === i ? { ...o, show_latency } : o)))
                    }
                  />
                  <span aria-hidden="true">{t('pages.latency')}</span>
                </label>
                <IconButton
                  size="sm"
                  icon={ICONS.moveUp}
                  label={t('pages.monitorUp', { name: real })}
                  disabled={i === 0}
                  onClick={() => setMonitors(moveItem(monitors, i, -1))}
                />
                <IconButton
                  size="sm"
                  icon={ICONS.moveDown}
                  label={t('pages.monitorDown', { name: real })}
                  disabled={i === monitors.length - 1}
                  onClick={() => setMonitors(moveItem(monitors, i, 1))}
                />
                <IconButton
                  size="sm"
                  icon={ICONS.close}
                  label={t('pages.monitorRemove', { name: real })}
                  onClick={() => setMonitors(monitors.filter((_, j) => j !== i))}
                />
              </li>
            );
          })}
        </ol>
      )}

      <div className="mt-3 flex flex-wrap items-end gap-2">
        <Field label={t('pages.addMonitor')} className="min-w-48 flex-1">
          {(fid) => (
            <Select
              id={fid}
              value={pick}
              onChange={(e) => setPick(e.target.value)}
              disabled={free.length === 0}
            >
              <option value="">
                {free.length ? t('pages.pickMonitor') : t('pages.noMoreMonitors')}
              </option>
              {[...byGroup].map(([name, list]) => (
                <optgroup key={name} label={name || t('pages.ungrouped')}>
                  {list.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.name}
                    </option>
                  ))}
                </optgroup>
              ))}
            </Select>
          )}
        </Field>
        <Button
          icon={ICONS.add}
          disabled={!pick}
          onClick={() => {
            setMonitors([...monitors, { id: Number(pick), name: '', show_latency: false }]);
            setPick('');
          }}
        >
          {t('pages.addMonitorButton')}
        </Button>
      </div>
    </li>
  );
}

function LogoCard({ page, onChange }: { page: StatusPage; onChange: () => void }) {
  const { t } = useT();
  const { busy, run } = useAction();
  const [version, setVersion] = useState(0);
  const [problem, setProblem] = useState('');

  const upload = async (file: File | undefined) => {
    setProblem('');
    if (!file) return;
    if (!LOGO_TYPES.includes(file.type)) return setProblem(t('pages.logoType'));
    if (file.size > LOGO_MAX_BYTES) return setProblem(t('pages.logoSize'));
    const done = await run(
      'logo',
      async () => {
        await pagesApi.uploadLogo(page.id, file);
        return true;
      },
      t('pages.logoSaved'),
    );
    if (done) {
      setVersion((v) => v + 1);
      onChange();
    }
  };
  const remove = async () => {
    const done = await run('logoDelete', async () => {
      await pagesApi.deleteLogo(page.id);
      return true;
    });
    if (done) {
      toasts.show(t('pages.logoRemoved'), { tone: 'success' });
      onChange();
    }
  };

  return (
    <Card title={t('pages.logo')} icon={ICONS.image} className="mt-4">
      <div className="flex flex-wrap items-center gap-4">
        {page.has_logo ? (
          <img
            src={`${page.preview_url}/logo?v=${version}`}
            alt={t('pages.logoAlt', { title: page.title })}
            className="h-16 max-w-48 rounded-control border border-hairline bg-surface object-contain p-1"
          />
        ) : (
          <Tag>{t('pages.noLogo')}</Tag>
        )}
        <Field
          label={t('pages.logoPick')}
          hint={t('pages.logoHint')}
          error={problem || undefined}
          className="min-w-56 flex-1"
        >
          {(fid, describedBy) => (
            <input
              id={fid}
              type="file"
              aria-describedby={describedBy}
              accept={LOGO_TYPES.join(',')}
              disabled={busy !== null}
              className="text-meta file:mr-3 file:rounded-full file:border file:border-control file:bg-surface file:px-3 file:py-1.5 file:font-semibold"
              onChange={(e) => {
                void upload(e.target.files?.[0]);
                e.target.value = '';
              }}
            />
          )}
        </Field>
        {page.has_logo && (
          <Button
            variant="danger"
            size="sm"
            icon={ICONS.delete}
            busy={busy === 'logoDelete'}
            onClick={() => void remove()}
          >
            {t('pages.logoDelete')}
          </Button>
        )}
      </div>
    </Card>
  );
}
