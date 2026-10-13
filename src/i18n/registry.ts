/**
 * Every i18n namespace (design §6): `common` plus one per feature. A feature adds its catalog
 * here; `index.ts` builds the i18next resources from this table and `i18next.d.ts` types `t()`
 * keys from the zh-CN catalogs.
 */
import { groupsMessages } from '@/features/groups/i18n';
import { profileEditorMessages } from '@/features/profile-editor/i18n';
import { profilesMessages } from '@/features/profiles/i18n';
import { proxiesMessages } from '@/features/proxies/i18n';
import { settingsMessages } from '@/features/settings/i18n';
import type { Locale } from '@/lib/bindings';

import { common } from './common';

export const registry = {
  common,
  profiles: profilesMessages,
  profileEditor: profileEditorMessages,
  groups: groupsMessages,
  proxies: proxiesMessages,
  settings: settingsMessages,
};

export type Namespace = keyof typeof registry;

/** Typed `t()` keys come from the zh-CN catalogs (the reference language). */
export type Resources = { [N in Namespace]: (typeof registry)[N]['zh-CN'] };

export const namespaces = Object.keys(registry) as Namespace[];

export const locales: readonly Locale[] = ['zh-CN', 'en'];

/** i18next `resources`: language → namespace → catalog. */
export function buildResources(): Record<Locale, Record<Namespace, object>> {
  const forLocale = (locale: Locale) =>
    Object.fromEntries(namespaces.map((ns) => [ns, registry[ns][locale]])) as Record<
      Namespace,
      object
    >;
  return { 'zh-CN': forLocale('zh-CN'), en: forLocale('en') };
}
