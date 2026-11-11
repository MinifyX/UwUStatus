import { expect, it } from 'vitest';
import { emptyMaintenance, maintenanceDraftFrom, validateMaintenance } from './maintenanceForm';

const NOW = 1_767_225_600;

it('validates once and weekly windows', () => {
  const empty = validateMaintenance(emptyMaintenance(NOW));
  expect(empty.errors.title?.key).toBe('form.err.required');
  expect(empty.errors.monitors?.key).toBe('maint.err.target');

  const base = { ...emptyMaintenance(NOW), title: 'Update', monitors: [1] };
  expect(validateMaintenance(base).input?.schedule.type).toBe('once');
  expect(validateMaintenance({ ...base, end: base.start }).errors.end?.key).toBe('maint.err.end');

  const weekly = { ...base, type: 'weekly' as const, timezone: 'Europe/Berlin' };
  expect(validateMaintenance(weekly).errors.weekdays?.key).toBe('maint.err.weekdays');
  const bad = validateMaintenance({
    ...weekly,
    weekdays: [1],
    time: '25:00',
    timezone: 'Mars/Base',
  });
  expect([bad.errors.time?.key, bad.errors.timezone?.key]).toEqual([
    'maint.err.time',
    'maint.err.zone',
  ]);
  const ok = validateMaintenance({ ...weekly, weekdays: [7, 1] });
  expect(ok.input?.schedule).toEqual({
    type: 'weekly',
    weekdays: [1, 7],
    time: '03:00',
    duration_min: 60,
    timezone: 'Europe/Berlin',
  });
});

it('round-trips a stored window', () => {
  const stored = {
    id: 1,
    title: 'Nacht',
    description: '',
    schedule: { type: 'once' as const, start: NOW, end: NOW + 7200 },
    monitors: [2],
    groups: [],
    active: true,
  };
  expect(validateMaintenance(maintenanceDraftFrom(stored, NOW)).input?.schedule).toEqual(
    stored.schedule,
  );
});
