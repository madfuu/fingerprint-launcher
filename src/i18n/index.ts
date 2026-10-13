/**
 * i18n setup (R9): react-i18next with every namespace of `registry.ts`, zh-CN by default.
 * The UI language is the `locale` setting; {@link initI18n} runs once at startup and
 * {@link changeLocale} follows later changes.
 */
import i18next from 'i18next';
import { initReactI18next } from 'react-i18next';

import type { Locale } from '@/lib/bindings';

import { buildResources, locales, namespaces } from './registry';

/** Default UI language, also used when the settings cannot be read. */
export const DEFAULT_LOCALE: Locale = 'zh-CN';

export const i18n = i18next.createInstance();

function setDocumentLanguage(locale: Locale): void {
  if (typeof document !== 'undefined') {
    document.documentElement.lang = locale;
  }
}

export async function initI18n(locale: Locale = DEFAULT_LOCALE): Promise<void> {
  if (i18n.isInitialized) {
    await changeLocale(locale);
    return;
  }
  await i18n.use(initReactI18next).init({
    resources: buildResources(),
    lng: locale,
    fallbackLng: DEFAULT_LOCALE,
    supportedLngs: [...locales],
    // Resources are bundled per exact locale; never look up `zh` for `zh-CN`.
    load: 'currentOnly',
    ns: namespaces,
    defaultNS: 'common',
    // React escapes text itself.
    interpolation: { escapeValue: false },
  });
  setDocumentLanguage(locale);
}

export async function changeLocale(locale: Locale): Promise<void> {
  if (!i18n.isInitialized) {
    await initI18n(locale);
    return;
  }
  await i18n.changeLanguage(locale);
  setDocumentLanguage(locale);
}
