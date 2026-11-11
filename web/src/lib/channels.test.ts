import { expect, it } from 'vitest';
import { specFrom, valuesFrom } from './channels';

it('builds channel specs with only the fields of the type', () => {
  const values = { to: 'a@example.com\n b@example.com, ', url: 'x', priority: '' };
  expect(specFrom('email', values)).toEqual({
    type: 'email',
    to: ['a@example.com', 'b@example.com'],
  });
  expect(specFrom('ntfy', { server: ' https://ntfy.example.com ', topic: 'ops' })).toEqual({
    type: 'ntfy',
    server: 'https://ntfy.example.com',
    topic: 'ops',
    priority: 4,
  });
  expect(specFrom('discord', values)).toEqual({ type: 'discord' });
  const spec = specFrom('gotify', { url: 'https://gotify.example.com', priority: '9' });
  expect(specFrom('gotify', valuesFrom(spec))).toEqual(spec);
});
