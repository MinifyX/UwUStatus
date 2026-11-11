// Path routing with the history API. The server hands index.html to every non-API path, so the
// OIDC login can return to the exact page (`next`).

import { useEffect, useState, type MouseEvent } from 'react';

// Other phases add their pages here: one Route variant, one parse line, one href case.
export type Route =
  | { name: 'overview' }
  | { name: 'monitor'; id: number }
  | { name: 'monitorNew' }
  | { name: 'monitorEdit'; id: number }
  | { name: 'maintenance' }
  | { name: 'maintenanceNew' }
  | { name: 'maintenanceEdit'; id: number }
  | { name: 'settings' }
  | { name: 'notifications' }
  | { name: 'pages'; id?: number | 'new' }
  | { name: 'incidents'; id?: number }
  | { name: 'apiTokens' }
  | { name: 'notFound' }
  | { name: 'satellites' };

const ID = /^[1-9]\d{0,15}$/;

export function parseRoute(pathname: string): Route {
  const [a, b, c, ...rest] = pathname.split('/').filter(Boolean);
  if (rest.length) return { name: 'notFound' };
  const id = b && ID.test(b) ? Number(b) : null;
  if (!a) return { name: 'overview' };
  if (a === 'settings' && !b) return { name: 'settings' };
  // Alert links point at {public_url}/monitors/{id}.
  if (a === 'monitors' && b === 'new' && !c) return { name: 'monitorNew' };
  if (a === 'monitors' && id && !c) return { name: 'monitor', id };
  if (a === 'monitors' && id && c === 'edit') return { name: 'monitorEdit', id };
  if (a === 'maintenance' && !b) return { name: 'maintenance' };
  if (a === 'maintenance' && b === 'new' && !c) return { name: 'maintenanceNew' };
  if (a === 'maintenance' && id && !c) return { name: 'maintenanceEdit', id };
  if (a === 'notifications' && !b) return { name: 'notifications' };
  if (a === 'api-tokens' && !b) return { name: 'apiTokens' };
  if (a === 'pages' && !b) return { name: 'pages' };
  if (a === 'pages' && b === 'new' && !c) return { name: 'pages', id: 'new' };
  if (a === 'pages' && id && !c) return { name: 'pages', id };
  if (a === 'incidents' && !b) return { name: 'incidents' };
  if (a === 'incidents' && id && !c) return { name: 'incidents', id };
  if (a === 'satellites' && !b) return { name: 'satellites' };
  return { name: 'notFound' };
}

export function hrefOf(route: Route): string {
  switch (route.name) {
    case 'overview':
    case 'notFound':
      return '/';
    case 'monitor':
      return `/monitors/${route.id}`;
    case 'monitorNew':
      return '/monitors/new';
    case 'monitorEdit':
      return `/monitors/${route.id}/edit`;
    case 'maintenance':
      return '/maintenance';
    case 'maintenanceNew':
      return '/maintenance/new';
    case 'maintenanceEdit':
      return `/maintenance/${route.id}`;
    case 'settings':
      return '/settings';
    case 'notifications':
      return '/notifications';
    case 'pages':
    case 'incidents':
      return route.id === undefined ? `/${route.name}` : `/${route.name}/${route.id}`;
    case 'apiTokens':
      return '/api-tokens';
    case 'satellites':
      return '/satellites';
  }
}

const EVENT = 'uwustatus:navigate';

export function currentPath(): string {
  return location.pathname + location.search;
}

export function navigate(href: string, replace = false): void {
  if (replace) history.replaceState(null, '', href);
  else history.pushState(null, '', href);
  window.dispatchEvent(new Event(EVENT));
}

/** onClick for an <a>: plain left clicks stay in the app, others (new tab …) go to the browser. */
export function linkClick(event: MouseEvent<HTMLAnchorElement>): void {
  if (
    event.defaultPrevented ||
    event.button !== 0 ||
    event.metaKey ||
    event.ctrlKey ||
    event.shiftKey ||
    event.altKey
  )
    return;
  event.preventDefault();
  navigate(event.currentTarget.getAttribute('href') ?? '/');
}

export function useRoute(): Route {
  const [route, setRoute] = useState(() => parseRoute(location.pathname));
  useEffect(() => {
    const onChange = () => setRoute(parseRoute(location.pathname));
    window.addEventListener('popstate', onChange);
    window.addEventListener(EVENT, onChange);
    return () => {
      window.removeEventListener('popstate', onChange);
      window.removeEventListener(EVENT, onChange);
    };
  }, []);
  return route;
}
