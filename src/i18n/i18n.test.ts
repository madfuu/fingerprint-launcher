import { afterEach, beforeAll, describe, expect, it } from 'vitest';

import { changeLocale, i18n, initI18n } from '@/i18n';
import { localizeError } from '@/i18n/errors';
import { registry } from '@/i18n/registry';

type Tree = { [key: string]: string | Tree };

/** `a.b.c` paths of every leaf. */
function leaves(tree: Tree, prefix = ''): string[] {
  return Object.entries(tree).flatMap(([key, value]) =>
    typeof value === 'string' ? [`${prefix}${key}`] : leaves(value, `${prefix}${key}.`)
  );
}

beforeAll(async () => {
  await initI18n('zh-CN');
});

afterEach(async () => {
  await changeLocale('zh-CN');
});

describe('catalogs', () => {
  it.each(Object.entries(registry))('%s: en has the zh-CN keys, no empty text', (_, catalog) => {
    const zh = leaves(catalog['zh-CN'] as Tree);
    expect(leaves(catalog.en as Tree)).toEqual(zh);
    for (const locale of ['zh-CN', 'en'] as const) {
      const flat = leaves(catalog[locale] as Tree);
      expect(flat.length).toBeGreaterThan(0);
      expect(JSON.stringify(catalog[locale])).not.toContain('""');
    }
  });
});

describe('i18n', () => {
  it('switches language and the document language with it', async () => {
    await initI18n('en'); // already initialized: changes the language
    expect(i18n.t('nav.settings')).toBe('Settings');
    expect(i18n.t('title', { ns: 'proxies' })).toBe('Proxies');
    expect(document.documentElement.lang).toBe('en');

    await changeLocale('zh-CN');
    expect(i18n.t('nav.settings')).toBe('设置');
    expect(document.documentElement.lang).toBe('zh-CN');
  });

  it('localizes bridge errors by code and keeps the raw detail', async () => {
    await changeLocale('en');
    expect(
      localizeError(i18n.t, { code: 'kernel_missing', message: 'kernel not found: /x' })
    ).toEqual({
      code: 'kernel_missing',
      message: 'The kernel executable does not exist; check the kernel path.',
      detail: 'kernel not found: /x',
    });
    expect(localizeError(i18n.t, 'command x not allowed by ACL')).toMatchObject({
      code: 'invalid_input',
      detail: 'command x not allowed by ACL',
    });
  });
});
