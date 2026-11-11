// The frame: sidebar from 1024 px, below that a top bar with a drawer (modal, Esc closes it).

import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { Button, Icon, IconButton, ICONS, Segmented, Toaster, Wordmark } from '@uwusuite/design';
import type { LucideIcon } from 'lucide-react';
import type { Me } from '../api/types';
import { toasts } from '../lib/hooks';
import { useT, type Key, type Lang } from '../lib/i18n';
import { hrefOf, type Route } from '../lib/router';
import { Link } from './ui';

export interface NavItem {
  route: Route;
  label: Key;
  icon: LucideIcon;
  /** Other routes that light up this entry (detail and edit pages). */
  also?: Route['name'][];
}

/**
 * The main navigation. Each phase adds its page with ONE line here (plus its route in
 * lib/router.ts and its case in App.tsx `Page`), e.g.
 *   { route: { name: 'channels' }, label: 'nav.channels', icon: ICONS.notifications },
 * Order: Übersicht, Wartung, Benachrichtigungen, Statusseiten, Vorfälle, Satelliten,
 * API-Tokens, Einstellungen (settings stay last).
 */
export const NAV: NavItem[] = [
  {
    route: { name: 'overview' },
    label: 'nav.overview',
    icon: ICONS.overview,
    also: ['monitor', 'monitorNew', 'monitorEdit'],
  },
  {
    route: { name: 'maintenance' },
    label: 'nav.maintenance',
    icon: ICONS.calendar,
    also: ['maintenanceNew', 'maintenanceEdit'],
  },
  { route: { name: 'notifications' }, label: 'nav.notifications', icon: ICONS.notifications },
  { route: { name: 'pages' }, label: 'nav.pages', icon: ICONS.website },
  { route: { name: 'incidents' }, label: 'nav.incidents', icon: ICONS.warning },
  { route: { name: 'apiTokens' }, label: 'nav.apiTokens', icon: ICONS.secret },
  { route: { name: 'settings' }, label: 'nav.settings', icon: ICONS.settings },
];

export const navFor = (route: Route) =>
  NAV.find((n) => n.route.name === route.name || n.also?.includes(route.name));

export function LanguageSwitch() {
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

function NavList({ route, onGo }: { route: Route; onGo?: () => void }) {
  const { t } = useT();
  const here = navFor(route);
  return (
    <ul className="m-0 flex list-none flex-col gap-0.5 p-0">
      {NAV.map((item) => {
        const active = item === here;
        return (
          <li key={item.label}>
            <Link
              href={hrefOf(item.route)}
              onClick={onGo}
              aria-current={active ? 'page' : undefined}
              className={`flex min-h-9 items-center gap-2.5 rounded-xl px-3 text-body no-underline focus-visible:shadow-focus focus-visible:outline-none ${
                active
                  ? 'bg-pink-tint font-semibold text-pink-ink'
                  : 'text-ink hover:bg-pink-tint/45'
              }`}
            >
              <Icon icon={item.icon} size="sm" />
              {t(item.label)}
            </Link>
          </li>
        );
      })}
    </ul>
  );
}

function Foot({ me, onLogout }: { me: Me; onLogout: () => void }) {
  const { t } = useT();
  return (
    <div className="flex flex-col gap-3 border-t border-hairline pt-4">
      <LanguageSwitch />
      <div className="flex items-center justify-between gap-2">
        <span className="min-w-0 truncate text-meta text-muted" title={me.email ?? undefined}>
          {me.name}
        </span>
        <Button size="sm" variant="ghost" icon={ICONS.signOut} onClick={onLogout}>
          {t('auth.logout')}
        </Button>
      </div>
    </div>
  );
}

export function Shell({
  me,
  route,
  onLogout,
  children,
}: {
  me: Me;
  route: Route;
  onLogout: () => void;
  children: ReactNode;
}) {
  const { t } = useT();
  const [open, setOpen] = useState(false);
  const drawerId = useId();
  const button = useRef<HTMLButtonElement>(null);
  const drawer = useRef<HTMLDivElement>(null);

  useEffect(() => setOpen(false), [route]);
  useEffect(() => {
    if (!open) return;
    const opener = button.current;
    drawer.current?.querySelector<HTMLElement>('[aria-current], a, button')?.focus();
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('keydown', onKey);
      opener?.focus();
    };
  }, [open]);

  const brand = (
    <Link href="/" className="no-underline" aria-label={t('nav.home')}>
      <Wordmark product="Status" shell="box" />
    </Link>
  );

  return (
    <>
      <a
        href="#main"
        className="sr-only focus:not-sr-only focus:fixed focus:top-2 focus:left-2 focus:z-50 focus:rounded-full focus:bg-surface focus:px-4 focus:py-2"
      >
        {t('app.skip')}
      </a>
      <div className="lg:grid lg:min-h-screen lg:grid-cols-[252px_minmax(0,1fr)]">
        <aside className="hidden border-r border-hairline bg-surface lg:block">
          <div className="sticky top-0 flex h-screen flex-col gap-6 overflow-y-auto p-4">
            {brand}
            <nav aria-label={t('nav.main')} className="flex-1">
              <NavList route={route} />
            </nav>
            <Foot me={me} onLogout={onLogout} />
          </div>
        </aside>

        <header
          inert={open || undefined}
          className="sticky top-0 z-30 flex items-center gap-2 border-b border-hairline bg-surface px-3 py-2 lg:hidden"
        >
          <IconButton
            ref={button}
            icon={ICONS.menu}
            label={t('nav.menu')}
            aria-expanded={open}
            aria-controls={drawerId}
            onClick={() => setOpen(!open)}
          />
          {brand}
        </header>
        {open && (
          <div className="fixed inset-0 z-40 lg:hidden">
            <div className="absolute inset-0 bg-scrim" aria-hidden onClick={() => setOpen(false)} />
            <div
              ref={drawer}
              id={drawerId}
              role="dialog"
              aria-modal="true"
              aria-label={t('nav.main')}
              className="absolute inset-y-0 left-0 flex w-[264px] max-w-[85vw] flex-col gap-6 overflow-y-auto overscroll-contain bg-surface p-4 shadow-float motion-safe:animate-drawer motion-reduced:animate-none"
            >
              <div className="flex items-center justify-between gap-2">
                {brand}
                <IconButton
                  icon={ICONS.close}
                  label={t('nav.close')}
                  onClick={() => setOpen(false)}
                />
              </div>
              <nav aria-label={t('nav.main')} className="flex-1">
                <NavList route={route} onGo={() => setOpen(false)} />
              </nav>
              <Foot me={me} onLogout={onLogout} />
            </div>
          </div>
        )}

        <main
          id="main"
          inert={open || undefined}
          className="mx-auto w-full max-w-[1200px] min-w-0 px-4 py-6 lg:px-8"
        >
          {children}
        </main>
      </div>
      <Toaster store={toasts} />
    </>
  );
}
