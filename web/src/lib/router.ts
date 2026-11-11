// Path routing with the history API. The server hands index.html to every non-API path, so the
// OIDC login can return to the exact page (`next`).

import { useEffect, useState, type MouseEvent } from 'react';

export type Route =
  | { name: 'overview' }
  | { name: 'settings' }
  | { name: 'notFound' }
  | { name: 'pages'; id?: number | 'new' }
  | { name: 'incidents'; id?: number }
  | { name: 'apiTokens' };

const id = (part: string | undefined) => (part && /^[1-9]\d{0,15}$/.test(part) ? Number(part) : 0);

export function parseRoute(pathname: string): Route {
  const parts = pathname.split('/').filter(Boolean);
  if (parts.length === 0) return { name: 'overview' };
  if (parts.length === 1 && parts[0] === 'settings') return { name: 'settings' };
  if (parts.length === 1 && parts[0] === 'api-tokens') return { name: 'apiTokens' };
  if (parts[0] === 'pages' && parts.length === 1) return { name: 'pages' };
  if (parts[0] === 'pages' && parts.length === 2 && parts[1] === 'new')
    return { name: 'pages', id: 'new' };
  if (parts[0] === 'pages' && parts.length === 2 && id(parts[1]))
    return { name: 'pages', id: id(parts[1]) };
  if (parts[0] === 'incidents' && parts.length === 1) return { name: 'incidents' };
  if (parts[0] === 'incidents' && parts.length === 2 && id(parts[1]))
    return { name: 'incidents', id: id(parts[1]) };
  return { name: 'notFound' };
}

export function hrefOf(route: Route): string {
  switch (route.name) {
    case 'overview':
    case 'notFound':
      return '/';
    case 'settings':
      return '/settings';
    case 'pages':
    case 'incidents':
      return route.id === undefined ? `/${route.name}` : `/${route.name}/${route.id}`;
    case 'apiTokens':
      return '/api-tokens';
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
