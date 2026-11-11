// Filtering and grouping of the overview list.

import type { MonState, OverviewItem } from '../api/types';

/** Filters by search text (name, group, tag, message), state and tags (all must match). */
export function filterMonitors(
  items: OverviewItem[],
  query: string,
  state: MonState | null,
  tags: string[],
): OverviewItem[] {
  const q = query.trim().toLowerCase();
  return items.filter(
    (m) =>
      (!state || m.state === state) &&
      tags.every((t) => m.tags.includes(t)) &&
      (!q ||
        [m.name, m.group, m.message ?? '', ...m.tags].some((s) => s.toLowerCase().includes(q))),
  );
}

/** Groups in name order, monitors without a group last. */
export function groupMonitors(items: OverviewItem[]): [string, OverviewItem[]][] {
  const groups = new Map<string, OverviewItem[]>();
  for (const m of [...items].sort((a, b) => a.name.localeCompare(b.name)))
    groups.set(m.group, [...(groups.get(m.group) ?? []), m]);
  return [...groups].sort(([a], [b]) => (!a ? 1 : !b ? -1 : a.localeCompare(b)));
}
