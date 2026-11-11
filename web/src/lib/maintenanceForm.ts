// The maintenance form: once (local date/time inputs) or weekly (weekdays, time, duration, zone).

import type { Maintenance, MaintenanceInput } from '../api/types';
import type { FieldError } from './monitorForm';

export interface MaintenanceDraft {
  title: string;
  description: string;
  type: 'once' | 'weekly';
  /** `datetime-local` values in the browser's zone. */
  start: string;
  end: string;
  weekdays: number[];
  time: string;
  duration: string;
  timezone: string;
  monitors: number[];
  groups: string[];
  active: boolean;
}

export type MaintenanceErrors = Partial<Record<keyof MaintenanceDraft, FieldError>>;

const pad = (n: number) => String(n).padStart(2, '0');

/** Unix seconds → `YYYY-MM-DDTHH:MM` in the browser's zone. */
export function toLocalInput(at: number): string {
  const d = new Date(at * 1000);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function browserZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
}

export function emptyMaintenance(nowSeconds: number): MaintenanceDraft {
  const start = Math.ceil(nowSeconds / 3600) * 3600;
  return {
    title: '',
    description: '',
    type: 'once',
    start: toLocalInput(start),
    end: toLocalInput(start + 3600),
    weekdays: [],
    time: '03:00',
    duration: '60',
    timezone: browserZone(),
    monitors: [],
    groups: [],
    active: true,
  };
}

export function maintenanceDraftFrom(m: Maintenance, nowSeconds: number): MaintenanceDraft {
  const d = { ...emptyMaintenance(nowSeconds), ...m, type: m.schedule.type };
  const s = m.schedule;
  if (s.type === 'once') return { ...d, start: toLocalInput(s.start), end: toLocalInput(s.end) };
  return {
    ...d,
    weekdays: s.weekdays,
    time: s.time,
    duration: String(s.duration_min),
    timezone: s.timezone,
  };
}

function knownZone(zone: string): boolean {
  try {
    new Intl.DateTimeFormat('en', { timeZone: zone });
    return true;
  } catch {
    return false;
  }
}

export function validateMaintenance(d: MaintenanceDraft): {
  errors: MaintenanceErrors;
  input?: MaintenanceInput;
} {
  const errors: MaintenanceErrors = {};
  const title = d.title.trim();
  if (!title) errors.title = { key: 'form.err.required' };
  else if (title.length > 100) errors.title = { key: 'form.err.tooLong', vars: { max: 100 } };
  if (d.description.length > 2000)
    errors.description = { key: 'form.err.tooLong', vars: { max: 2000 } };
  if (d.monitors.length === 0 && d.groups.length === 0)
    errors.monitors = { key: 'maint.err.target' };

  let schedule: MaintenanceInput['schedule'];
  if (d.type === 'once') {
    const start = Math.floor(new Date(d.start).getTime() / 1000);
    const end = Math.floor(new Date(d.end).getTime() / 1000);
    if (!Number.isFinite(start)) errors.start = { key: 'form.err.required' };
    if (!Number.isFinite(end)) errors.end = { key: 'form.err.required' };
    else if (end <= start || end - start > 366 * 86_400) errors.end = { key: 'maint.err.end' };
    schedule = { type: 'once', start, end };
  } else {
    const duration = Number(d.duration);
    if (d.weekdays.length === 0) errors.weekdays = { key: 'maint.err.weekdays' };
    if (!/^([01]\d|2[0-3]):[0-5]\d$/.test(d.time)) errors.time = { key: 'maint.err.time' };
    if (!/^\d+$/.test(d.duration) || duration < 1 || duration > 10_080)
      errors.duration = { key: 'form.err.range', vars: { min: 1, max: 10_080 } };
    if (!knownZone(d.timezone.trim())) errors.timezone = { key: 'maint.err.zone' };
    schedule = {
      type: 'weekly',
      weekdays: [...d.weekdays].sort(),
      time: d.time,
      duration_min: duration,
      timezone: d.timezone.trim(),
    };
  }
  if (Object.keys(errors).length) return { errors };
  return {
    errors,
    input: {
      title,
      description: d.description.trim(),
      schedule,
      monitors: d.monitors,
      groups: d.groups,
      active: d.active,
    },
  };
}
