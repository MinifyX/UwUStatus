// DE/EN without a framework: one dictionary per language, `t(key, vars)`, the choice in
// localStorage. German is the source; the English dictionary must have every key (typecheck).

import { useSyncExternalStore } from 'react';

const de = {
  'app.skip': 'Zum Inhalt springen',
  'common.loading': 'Lädt …',
  'common.retry': 'Nochmal',
  'common.notFound': 'Seite nicht gefunden',
  'common.toOverview': 'Zur Übersicht',
  'auth.checking': 'Prüfe die Anmeldung …',
  'auth.title': 'Bitte melde dich an',
  'auth.intro': 'UwUStatus überwacht deine Dienste und zeigt ihren Zustand auf Statusseiten.',
  'auth.oidc': 'Mit UwUAuth anmelden',
  'auth.or': 'oder lokal',
  'auth.user': 'Benutzer',
  'auth.password': 'Passwort',
  'auth.totp': 'Code aus der Authenticator-App',
  'auth.submit': 'Anmelden',
  'auth.logout': 'Abmelden',
  'auth.none': 'Keine Anmeldung eingerichtet.',
  'nav.main': 'Hauptnavigation',
  'nav.overview': 'Übersicht',
  'nav.settings': 'Einstellungen',
  'nav.home': 'UwUStatus, zur Übersicht',
  'overview.empty': 'Noch keine Monitore',
  'overview.emptyBody': 'Monitore kommen in einer der nächsten Versionen.',
  'settings.retention': 'Aufbewahrung',
  'settings.raw': 'Rohdaten (Tage)',
  'settings.hourly': 'Stundenwerte (Tage)',
  'settings.daily': 'Tageswerte (Tage)',
  'settings.save': 'Speichern',
  'settings.saved': 'Gespeichert.',
  'language.label': 'Sprache',
};

export type Key = keyof typeof de;

const en: Record<Key, string> = {
  'app.skip': 'Skip to content',
  'common.loading': 'Loading …',
  'common.retry': 'Retry',
  'common.notFound': 'Page not found',
  'common.toOverview': 'Back to overview',
  'auth.checking': 'Checking sign-in …',
  'auth.title': 'Please sign in',
  'auth.intro': 'UwUStatus monitors your services and shows their state on status pages.',
  'auth.oidc': 'Sign in with UwUAuth',
  'auth.or': 'or locally',
  'auth.user': 'User',
  'auth.password': 'Password',
  'auth.totp': 'Code from your authenticator app',
  'auth.submit': 'Sign in',
  'auth.logout': 'Sign out',
  'auth.none': 'No sign-in method is set up.',
  'nav.main': 'Main navigation',
  'nav.overview': 'Overview',
  'nav.settings': 'Settings',
  'nav.home': 'UwUStatus, to the overview',
  'overview.empty': 'No monitors yet',
  'overview.emptyBody': 'Monitors arrive in one of the next versions.',
  'settings.retention': 'Retention',
  'settings.raw': 'Raw results (days)',
  'settings.hourly': 'Hourly values (days)',
  'settings.daily': 'Daily values (days)',
  'settings.save': 'Save',
  'settings.saved': 'Saved.',
  'language.label': 'Language',
};

export const LANGS = { de, en } as const;
export type Lang = keyof typeof LANGS;
const KEY = 'uwustatus.lang';

/** `{name}` placeholders filled from `vars`; unknown ones stay as they are. */
export function translate(lang: Lang, key: Key, vars?: Record<string, string | number>): string {
  const text = LANGS[lang][key];
  return vars ? text.replace(/\{(\w+)\}/g, (all, name: string) => String(vars[name] ?? all)) : text;
}

/** Stored choice, else the browser's first German or English language, else German. */
export function detectLang(stored: string | null, browser: readonly string[]): Lang {
  if (stored === 'de' || stored === 'en') return stored;
  for (const tag of browser) {
    const base = tag.toLowerCase().split('-')[0];
    if (base === 'de' || base === 'en') return base;
  }
  return 'de';
}

function stored(): string | null {
  try {
    return localStorage.getItem(KEY);
  } catch {
    return null;
  }
}

let current: Lang =
  typeof navigator === 'undefined' ? 'de' : detectLang(stored(), navigator.languages ?? []);
const listeners = new Set<() => void>();

export function setLang(lang: Lang): void {
  current = lang;
  try {
    localStorage.setItem(KEY, lang);
  } catch {
    // Private mode: the choice lasts until reload.
  }
  document.documentElement.lang = lang;
  listeners.forEach((l) => l());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useT() {
  const lang = useSyncExternalStore(subscribe, () => current);
  return {
    lang,
    setLang,
    t: (key: Key, vars?: Record<string, string | number>) => translate(lang, key, vars),
  };
}

if (typeof document !== 'undefined') document.documentElement.lang = current;
