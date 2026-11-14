// The app: sign-in gate (UwUAuth and/or local admin), then the shell with one page per route.

import { useCallback, useEffect, useState, type FormEvent } from 'react';
import {
  Button,
  Field,
  ICONS,
  Nyu,
  revealFrom,
  TextInput,
  Toaster,
  Wordmark,
  useAppearance,
} from '@uwusuite/design';
import { api, loginUrl, UNAUTHENTICATED_EVENT } from './api/client';
import type { Me, Methods } from './api/types';
import { LanguageSwitch, navFor, Shell } from './components/Shell';
import { ButtonLink, Empty, ErrorBox, Link, Loading } from './components/ui';
import { Dashboard } from './pages/Dashboard';
import { MaintenanceForm, MaintenanceList } from './pages/Maintenance';
import { MonitorDetail } from './pages/MonitorDetail';
import { MonitorForm } from './pages/MonitorForm';
import { SettingsPage } from './pages/Settings';
import { useStoredAppearance } from './lib/appearance';
import { NotificationsPage } from './pages/Notifications';
import { toasts, useAction, useResource } from './lib/hooks';
import { useT } from './lib/i18n';
import { currentPath, useRoute, type Route } from './lib/router';
import { ApiTokens } from './pages/ApiTokens';
import { Incidents } from './pages/Incidents';
import { StatusPages } from './pages/StatusPages';
import { Satellites } from './pages/Satellites';

type Auth =
  | { state: 'loading' }
  | { state: 'signedOut' }
  | { state: 'error'; error: Error }
  | { state: 'in'; me: Me };

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

  // Once per sign-in (cold start counts, OIDC comes back as one): uncover the app from the visible logo.
  const signedIn = auth.state === 'in';
  useEffect(() => {
    const root = document.getElementById('root');
    if (!signedIn || !root) return;
    const logos = [...document.querySelectorAll('[data-uwu-reveal-origin]')];
    revealFrom(
      root,
      logos.find((el) => el.getClientRects().length > 0),
    );
  }, [signedIn]);

  useEffect(() => {
    window.scrollTo({ top: 0 });
    const page = navFor(route)?.label;
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
    <Shell me={auth.me} route={route} onLogout={() => void logout()}>
      <div key={route.name} className="uwu-page">
        <Page route={route} me={auth.me} />
      </div>
    </Shell>
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

/**
 * One case per page. Other phases add theirs here (plus a route in lib/router.ts and a NAV
 * line in components/Shell.tsx).
 */
function Page({ route, me }: { route: Route; me: Me }) {
  const { t } = useT();
  switch (route.name) {
    case 'overview':
      return <Dashboard />;
    case 'monitor':
      return <MonitorDetail key={route.id} id={route.id} />;
    case 'monitorNew':
      return <MonitorForm />;
    case 'monitorEdit':
      return <MonitorForm key={route.id} id={route.id} />;
    case 'maintenance':
      return <MaintenanceList />;
    case 'maintenanceNew':
      return <MaintenanceForm />;
    case 'maintenanceEdit':
      return <MaintenanceForm key={route.id} id={route.id} />;
    case 'settings':
      return <SettingsPage me={me} />;
    case 'notifications':
      return <NotificationsPage />;
    case 'pages':
      return <StatusPages id={route.id} />;
    case 'incidents':
      return <Incidents id={route.id} />;
    case 'apiTokens':
      return <ApiTokens />;
    case 'satellites':
      return <Satellites />;
    case 'notFound':
      return (
        <Empty
          title={t('common.notFound')}
          mood="puzzled"
          body={<Link href="/">{t('common.toOverview')}</Link>}
        />
      );
  }
}
