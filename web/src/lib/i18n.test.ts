import { describe, expect, it } from 'vitest';
import { detectLang, LANGS, translate } from './i18n';

describe('i18n', () => {
  it('has the same keys in every language, none empty', () => {
    const keys = Object.keys(LANGS.de).sort();
    for (const dict of Object.values(LANGS)) {
      expect(Object.keys(dict).sort()).toEqual(keys);
      expect(Object.values(dict).every((v) => v.trim() !== '')).toBe(true);
    }
  });

  it('picks the stored, then the browser language, then German', () => {
    expect(detectLang('en', ['de-DE'])).toBe('en');
    expect(detectLang(null, ['fr-FR', 'en-GB'])).toBe('en');
    expect(detectLang('xx', ['fr'])).toBe('de');
  });

  it('translates', () => {
    expect(translate('en', 'auth.submit')).toBe('Sign in');
    expect(translate('de', 'auth.submit')).toBe('Anmelden');
  });
});
