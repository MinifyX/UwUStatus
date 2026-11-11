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
  'nav.notifications': 'Benachrichtigungen',
  'notify.intro':
    'Kanäle sagen dir Bescheid, wenn ein Monitor ausfällt, wieder da ist oder ein Zertifikat abläuft.',
  'notify.add': 'Kanal anlegen',
  'notify.channels': 'Kanäle',
  'notify.empty': 'Noch keine Kanäle',
  'notify.emptyBody':
    'Lege einen Kanal an: E-Mail, Webhook, ntfy, Gotify, Pushover, Discord oder Telegram.',
  'notify.default': 'Standard',
  'notify.off': 'Aus',
  'notify.test': 'Testen',
  'notify.tested': 'Testnachricht gesendet.',
  'notify.edit': 'Bearbeiten',
  'notify.delete': 'Löschen',
  'notify.deleteTitle': 'Kanal „{name}“ löschen?',
  'notify.deleteBody':
    'Monitore verlieren diesen Kanal, das Protokoll geht mit. Das lässt sich nicht rückgängig machen.',
  'notify.deleted': 'Kanal gelöscht.',
  'notify.log': 'Protokoll',
  'notify.logTitle': 'Zustellungen an „{name}“',
  'notify.logEmpty': 'Noch nichts gesendet.',
  'notify.logOk': 'Zugestellt',
  'notify.logFailed': 'Fehlgeschlagen',
  'notify.attempts': '{n} Versuche',
  'notify.when': 'Zeit',
  'notify.event': 'Ereignis',
  'notify.result': 'Ergebnis',
  'notify.newTitle': 'Neuer Kanal',
  'notify.editTitle': 'Kanal bearbeiten',
  'notify.name': 'Name',
  'notify.type': 'Art',
  'notify.lang': 'Sprache der Nachrichten',
  'notify.enabled': 'Aktiv',
  'notify.isDefault': 'Für neue Monitore',
  'notify.isDefaultHint': 'Neue Monitore bekommen diesen Kanal automatisch.',
  'notify.events': 'Ereignisse',
  'notify.ev.down': 'Ausgefallen',
  'notify.ev.degraded': 'Eingeschränkt',
  'notify.ev.up': 'Wieder erreichbar',
  'notify.ev.reminder': 'Erinnerung bei anhaltendem Ausfall',
  'notify.ev.cert': 'Zertifikat läuft ab',
  'notify.ev.version': 'Neue Version',
  'notify.ev.test': 'Test',
  'notify.save': 'Speichern',
  'notify.saved': 'Gespeichert.',
  'notify.cancel': 'Abbrechen',
  'notify.optional': '{label} (optional)',
  'notify.secretKeep': 'Gespeichert. Leer lassen, um es zu behalten.',
  'notify.t.email': 'E-Mail',
  'notify.t.webhook': 'Webhook',
  'notify.t.ntfy': 'ntfy',
  'notify.t.gotify': 'Gotify',
  'notify.t.pushover': 'Pushover',
  'notify.t.discord': 'Discord',
  'notify.t.telegram': 'Telegram',
  'notify.f.to': 'Empfänger, eine Adresse pro Zeile',
  'notify.f.url': 'URL',
  'notify.f.server': 'Server',
  'notify.f.topic': 'Thema (Topic)',
  'notify.f.priority': 'Priorität bei Ausfällen ({min} bis {max})',
  'notify.f.chat_id': 'Chat-ID',
  'notify.s.hmac_secret': 'Schlüssel für die HMAC-Signatur',
  'notify.s.token': 'Token',
  'notify.s.user_key': 'User-Key',
  'notify.s.webhook_url': 'Webhook-URL',
  'notify.s.bot_token': 'Bot-Token',
  'notify.webhookHint':
    'POST mit JSON {title, body, severity, link, event}. Mit Schlüssel steht im Header X-UwUStatus-Signature „sha256=…“ (HMAC über den Body).',
  'notify.emailHint': 'Braucht die SMTP-Einstellungen auf dieser Seite.',
  'smtp.title': 'SMTP für E-Mail',
  'smtp.hint': 'Ein leerer Server entfernt die Einstellungen.',
  'smtp.host': 'Server',
  'smtp.port': 'Port',
  'smtp.security': 'Verschlüsselung',
  'smtp.starttls': 'STARTTLS',
  'smtp.tls': 'TLS',
  'smtp.none': 'Keine',
  'smtp.user': 'Benutzer',
  'smtp.password': 'Passwort',
  'smtp.from': 'Absender',
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
  'nav.notifications': 'Notifications',
  'notify.intro':
    'Channels tell you when a monitor goes down, comes back or a certificate expires.',
  'notify.add': 'Add channel',
  'notify.channels': 'Channels',
  'notify.empty': 'No channels yet',
  'notify.emptyBody': 'Add a channel: email, webhook, ntfy, Gotify, Pushover, Discord or Telegram.',
  'notify.default': 'Default',
  'notify.off': 'Off',
  'notify.test': 'Test',
  'notify.tested': 'Test message sent.',
  'notify.edit': 'Edit',
  'notify.delete': 'Delete',
  'notify.deleteTitle': 'Delete channel “{name}”?',
  'notify.deleteBody':
    'Monitors lose this channel and its log goes with it. This cannot be undone.',
  'notify.deleted': 'Channel deleted.',
  'notify.log': 'Log',
  'notify.logTitle': 'Deliveries to “{name}”',
  'notify.logEmpty': 'Nothing sent yet.',
  'notify.logOk': 'Delivered',
  'notify.logFailed': 'Failed',
  'notify.attempts': '{n} attempts',
  'notify.when': 'Time',
  'notify.event': 'Event',
  'notify.result': 'Result',
  'notify.newTitle': 'New channel',
  'notify.editTitle': 'Edit channel',
  'notify.name': 'Name',
  'notify.type': 'Type',
  'notify.lang': 'Message language',
  'notify.enabled': 'Active',
  'notify.isDefault': 'For new monitors',
  'notify.isDefaultHint': 'New monitors get this channel automatically.',
  'notify.events': 'Events',
  'notify.ev.down': 'Down',
  'notify.ev.degraded': 'Degraded',
  'notify.ev.up': 'Back up',
  'notify.ev.reminder': 'Reminder while still down',
  'notify.ev.cert': 'Certificate expiring',
  'notify.ev.version': 'New version',
  'notify.ev.test': 'Test',
  'notify.save': 'Save',
  'notify.saved': 'Saved.',
  'notify.cancel': 'Cancel',
  'notify.optional': '{label} (optional)',
  'notify.secretKeep': 'Stored. Leave empty to keep it.',
  'notify.t.email': 'Email',
  'notify.t.webhook': 'Webhook',
  'notify.t.ntfy': 'ntfy',
  'notify.t.gotify': 'Gotify',
  'notify.t.pushover': 'Pushover',
  'notify.t.discord': 'Discord',
  'notify.t.telegram': 'Telegram',
  'notify.f.to': 'Recipients, one address per line',
  'notify.f.url': 'URL',
  'notify.f.server': 'Server',
  'notify.f.topic': 'Topic',
  'notify.f.priority': 'Priority for outages ({min} to {max})',
  'notify.f.chat_id': 'Chat ID',
  'notify.s.hmac_secret': 'Key for the HMAC signature',
  'notify.s.token': 'Token',
  'notify.s.user_key': 'User key',
  'notify.s.webhook_url': 'Webhook URL',
  'notify.s.bot_token': 'Bot token',
  'notify.webhookHint':
    'POST with JSON {title, body, severity, link, event}. With a key, the X-UwUStatus-Signature header carries “sha256=…” (HMAC over the body).',
  'notify.emailHint': 'Needs the SMTP settings on this page.',
  'smtp.title': 'SMTP for email',
  'smtp.hint': 'An empty server removes the settings.',
  'smtp.host': 'Server',
  'smtp.port': 'Port',
  'smtp.security': 'Encryption',
  'smtp.starttls': 'STARTTLS',
  'smtp.tls': 'TLS',
  'smtp.none': 'None',
  'smtp.user': 'User',
  'smtp.password': 'Password',
  'smtp.from': 'Sender',
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
