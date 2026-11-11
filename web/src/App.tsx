// The frame: sign-in gate (UwUAuth and/or local admin), header with navigation, language and user.

import { useCallback, useEffect, useState, type FormEvent } from 'react';
import {
  Button,
  Card,
  Field,
  ICONS,
  Nyu,
  Segmented,
  TextInput,
  Toaster,
  Wordmark,
  useAppearance,
} from '@uwusuite/design';
import { api, loginUrl, UNAUTHENTICATED_EVENT } from './api/client';
import type { Me, Methods, Settings } from './api/types';
import { ButtonLink, Empty, ErrorBox, Link, Loading, PageHeader } from './components/ui';
import { useStoredAppearance } from './lib/appearance';
import { NotificationsPage } from './pages/Notifications';
import { toasts, useAction, useResource } from './lib/hooks';
import { useT, type Key, type Lang } from './lib/i18n';
import { currentPath, hrefOf, navigate, useRoute, type Route } from './lib/router';

type Auth =
  | { state: 'loading' }
  | { state: 'signedOut' }
  | { state: 'error'; error: Error }
  | { state: 'in'; me: Me };

const NAV: { route: Route; label: Key }[] = [
  { route: { name: 'overview' }, label: 'nav.overview' },
  { route: { name: 'notifications' }, label: 'nav.notifications' },
  { route: { name: 'settings' }, label: 'nav.settings' },
];

export function App() {
  useAppearance(useStoredAppearance());
  const { t } = useT();
  const route = useRoute();
  const [auth, setAuth] = useState<Auth>({ state: 'loading' });

  const loadMe = useCallback(() => {
    api.me().then(
      (me) => setAuth({ state: 'in', me }),
      (error: unknown) =>
        setAuth(
          error instanceof Error && 'status' in error && error.status === 401
            ? { state: 'signedOut' }
            : { state: 'error', error: error instanceof Error ? error : new Error(String(error)) },
        ),
    );
  }, []);

  useEffect(() => {
    loadMe();
    const signedOut = () => setAuth({ state: 'signedOut' });
    window.addEventListener(UNAUTHENTICATED_EVENT, signedOut);
    return () => window.removeEventListener(UNAUTHENTICATED_EVENT, signedOut);
  }, [loadMe]);

  useEffect(() => {
    window.scrollTo({ top: 0 });
    const page = NAV.find((n) => n.route.name === route.name)?.label;
    document.title = page ? `${t(page)} – UwUStatus` : 'UwUStatus';
  }, [route, t]);

  if (auth.state === 'loading') return <Loading label={t('auth.checking')} />;
  if (auth.state === 'signedOut') return <SignIn onLocal={loadMe} />;
  if (auth.state === 'error')
    return (
      <main className="mx-auto max-w-lg px-4 py-16">
        <ErrorBox error={auth.error} retry={loadMe} />
      </main>
    );

  const logout = async () => {
    await api.logout().catch(() => undefined);
    setAuth({ state: 'signedOut' });
  };

  return (
    <>
      <a
        href="#main"
        className="sr-only focus:not-sr-only focus:fixed focus:top-2 focus:left-2 focus:z-50 focus:rounded-full focus:bg-surface focus:px-4 focus:py-2"
      >
        {t('app.skip')}
      </a>
      <header className="sticky top-0 z-30 border-b border-hairline bg-surface">
        <div className="mx-auto flex max-w-[1280px] flex-wrap items-center gap-x-6 gap-y-2 px-4 py-2 lg:px-8">
          <Link href="/" className="no-underline" aria-label={t('nav.home')}>
            <Wordmark product="Status" shell="box" />
          </Link>
          <nav
            aria-label={t('nav.main')}
            className="order-3 -mx-1 flex w-full gap-1 overflow-x-auto px-1 md:order-none md:w-auto"
          >
            {NAV.map((item) => {
              const active = item.route.name === route.name;
              return (
                <Link
                  key={item.label}
                  href={hrefOf(item.route)}
                  aria-current={active ? 'page' : undefined}
                  className={`flex h-9 shrink-0 items-center rounded-xl px-3 text-[13.5px] no-underline ${
                    active
                      ? 'bg-pink-tint font-semibold text-pink-ink'
                      : 'text-ink hover:bg-pink-tint/45'
                  }`}
                >
                  {t(item.label)}
                </Link>
              );
            })}
          </nav>
          <div className="ml-auto flex items-center gap-2">
            <LanguageSwitch />
            <span
              className="hidden text-meta text-muted sm:inline"
              title={auth.me.email ?? undefined}
            >
              {auth.me.name}
            </span>
            <Button size="sm" variant="ghost" icon={ICONS.signOut} onClick={() => void logout()}>
              {t('auth.logout')}
            </Button>
          </div>
        </div>
      </header>
      <main id="main" className="mx-auto w-full max-w-[1280px] min-w-0 px-4 py-6 lg:px-8">
        <Page route={route} />
      </main>
      <Toaster store={toasts} />
    </>
  );
}

function LanguageSwitch() {
  const { t, lang, setLang } = useT();
  return (
    <Segmented<Lang>
      label={t('language.label')}
      value={lang}
      onChange={setLang}
      options={[
        { value: 'de', label: 'DE' },
        { value: 'en', label: 'EN' },
      ]}
    />
  );
}

function SignIn({ onLocal }: { onLocal: () => void }) {
  const { t } = useT();
  const methods = useResource<Methods>(api.methods, []);
  const { busy, run } = useAction();
  const [user, setUser] = useState('admin');
  const [password, setPassword] = useState('');
  const [totp, setTotp] = useState('');

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const done = await run('local', async () => {
      await api.localLogin(user, password, totp);
      return true;
    });
    setTotp('');
    if (done) {
      setPassword('');
      onLocal();
    }
  };

  const m = methods.data;
  return (
    <main className="flex min-h-screen items-center justify-center px-4 py-12">
      <div className="flex w-full max-w-md flex-col items-center gap-4 rounded-card border border-hairline bg-surface p-8 text-center">
        <Nyu shell="box" mood="happy" size={96} title="" />
        <Wordmark product="Status" />
        <h1 className="m-0 text-section font-bold">{t('auth.title')}</h1>
        <p className="m-0 text-muted">{t('auth.intro')}</p>
        {methods.error && <ErrorBox error={methods.error} retry={methods.reload} />}
        {methods.loading && !m && <Loading />}
        {m?.oidc && (
          <ButtonLink external href={loginUrl(currentPath())} variant="primary" icon={ICONS.signIn}>
            {t('auth.oidc')}
          </ButtonLink>
        )}
        {m?.oidc && m.local && <p className="m-0 text-meta text-muted">{t('auth.or')}</p>}
        {m?.local && (
          <form className="flex w-full flex-col gap-3 text-left" onSubmit={(e) => void submit(e)}>
            <Field label={t('auth.user')}>
              {(id) => (
                <TextInput
                  id={id}
                  autoComplete="username"
                  value={user}
                  onChange={(e) => setUser(e.target.value)}
                  required
                />
              )}
            </Field>
            <Field label={t('auth.password')}>
              {(id) => (
                <TextInput
                  id={id}
                  type="password"
                  autoComplete="current-password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  required
                />
              )}
            </Field>
            {m.local_totp && (
              <Field label={t('auth.totp')}>
                {(id) => (
                  <TextInput
                    id={id}
                    inputMode="numeric"
                    autoComplete="one-time-code"
                    pattern="[0-9]{6}"
                    maxLength={6}
                    value={totp}
                    onChange={(e) => setTotp(e.target.value.replace(/\D/g, ''))}
                    required
                  />
                )}
              </Field>
            )}
            <Button
              type="submit"
              variant={m.oidc ? 'secondary' : 'primary'}
              disabled={busy !== null}
            >
              {t('auth.submit')}
            </Button>
          </form>
        )}
        {m && !m.oidc && !m.local && <p className="m-0 text-danger-ink">{t('auth.none')}</p>}
        <LanguageSwitch />
      </div>
      <Toaster store={toasts} />
    </main>
  );
}

function SettingsPage() {
  const { t } = useT();
  const settings = useResource(api.settings, []);
  const { busy, run } = useAction();
  const [draft, setDraft] = useState<Settings>();
  useEffect(() => setDraft(settings.data), [settings.data]);

  if (settings.error) return <ErrorBox error={settings.error} retry={settings.reload} />;
  if (!draft) return <Loading />;

  const fields: { key: keyof Settings; label: Key }[] = [
    { key: 'retention_raw_days', label: 'settings.raw' },
    { key: 'retention_hourly_days', label: 'settings.hourly' },
    { key: 'retention_daily_days', label: 'settings.daily' },
  ];
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const saved = await run('save', () => api.saveSettings(draft), t('settings.saved'));
    if (saved) settings.set(saved);
  };
  return (
    <>
      <PageHeader title={t('nav.settings')} />
      <Card title={t('settings.retention')} icon={ICONS.settings}>
        <form className="flex flex-col gap-4" onSubmit={(e) => void submit(e)}>
          {fields.map(({ key, label }) => (
            <Field key={key} label={t(label)}>
              {(id) => (
                <TextInput
                  id={id}
                  inputMode="numeric"
                  className="w-32"
                  value={String(draft[key])}
                  onChange={(e) =>
                    setDraft({ ...draft, [key]: Number(e.target.value.replace(/\D/g, '')) })
                  }
                />
              )}
            </Field>
          ))}
          <div>
            <Button type="submit" variant="primary" disabled={busy !== null}>
              {t('settings.save')}
            </Button>
          </div>
        </form>
      </Card>
    </>
  );
}

function Page({ route }: { route: Route }) {
  const { t } = useT();
  switch (route.name) {
    case 'overview':
      return (
        <>
          <PageHeader title={t('nav.overview')} />
          <Empty title={t('overview.empty')} body={t('overview.emptyBody')} />
        </>
      );
    case 'settings':
      return <SettingsPage />;
    case 'notifications':
      return <NotificationsPage />;
    case 'notFound':
      return (
        <Empty
          title={t('common.notFound')}
          mood="puzzled"
          body={
            <Link href="/" onClick={() => navigate('/')}>
              {t('common.toOverview')}
            </Link>
          }
        />
      );
  }
}
