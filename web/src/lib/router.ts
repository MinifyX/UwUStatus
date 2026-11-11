// Path routing with the history API. The server hands index.html to every non-API path, so the
// OIDC login can return to the exact page (`next`).

import { useEffect, useState, type MouseEvent } from 'react';

export type Route =
  { name: 'overview' } | { name: 'settings' } | { name: 'notifications' } | { name: 'notFound' };

export function parseRoute(pathname: string): Route {
  const parts = pathname.split('/').filter(Boolean);
  if (parts.length === 0) return { name: 'overview' };
  if (parts.length === 1 && parts[0] === 'settings') return { name: 'settings' };
  if (parts.length === 1 && parts[0] === 'notifications') return { name: 'notifications' };
  return { name: 'notFound' };
}

export function hrefOf(route: Route): string {
  switch (route.name) {
    case 'overview':
    case 'notFound':
      return '/';
    case 'settings':
      return '/settings';
    case 'notifications':
      return '/notifications';
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
