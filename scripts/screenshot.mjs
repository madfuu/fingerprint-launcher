#!/usr/bin/env node
/**
 * Screenshots for visual review (design §10): every route × variant × theme × language,
 * rendered by the Vite dev server with the mock bridge, in Chrome for Testing at the app's
 * default window size.
 *
 *   npm run screenshots
 *   npm run screenshots -- --route profiles,settings --variant expanded --theme dark --lang en
 *
 * Output: .screenshots/<route>-<variant>-<theme>-<lang>.png
 *
 * Variants cover the states a review needs beyond the plain page: the sidebar collapsed and
 * expanded, the macOS title bar (native traffic lights, no window controls of ours), and a
 * modal dialog over the app.
 */
import { mkdirSync } from 'node:fs';
import path from 'node:path';
import { parseArgs } from 'node:util';

import {
  appUrl,
  firstProfileId,
  newPage,
  openApp,
  openDialog,
  ROOT,
  select,
  withPreview,
  fail,
} from './lib/dev-preview.mjs';

const OUT_DIR = path.join(ROOT, '.screenshots');

/** `path` may contain `:profileId`, replaced with the first seeded profile. */
const ROUTES = [
  { name: 'profiles', path: '/' },
  { name: 'profile-new', path: '/profiles/new' },
  { name: 'profile-edit', path: '/profiles/:profileId/edit' },
  { name: 'groups', path: '/groups' },
  { name: 'proxies', path: '/proxies' },
  { name: 'settings', path: '/settings' },
];

/**
 * `routes` / `themes` / `langs` narrow a variant to what it is meant to show; absent means
 * every selected one.
 */
const VARIANTS = [
  { name: 'collapsed', params: { sidebar: 'collapsed' } },
  { name: 'expanded', params: { sidebar: 'expanded' }, routes: ['profiles'] },
  {
    name: 'macos',
    params: { sidebar: 'expanded', os: 'macos' },
    routes: ['profiles'],
    themes: ['light'],
    langs: ['zh-CN'],
  },
  {
    name: 'dialog',
    params: { sidebar: 'expanded' },
    routes: ['profiles'],
    langs: ['zh-CN'],
    open: 'dialog',
  },
];

const THEMES = ['light', 'dark'];
const LANGS = ['zh-CN', 'en'];

const options = (() => {
  try {
    return parseArgs({
      options: {
        route: { type: 'string' },
        variant: { type: 'string' },
        theme: { type: 'string' },
        lang: { type: 'string' },
      },
    }).values;
  } catch (error) {
    return fail(error.message);
  }
})();

const routes = select(ROUTES, options.route, 'route', (route) => route.name);
const variants = select(VARIANTS, options.variant, 'variant', (variant) => variant.name);
const themes = select(THEMES, options.theme, 'theme');
const langs = select(LANGS, options.lang, 'lang');

/** Keeps only what both the selection and the variant allow. */
function narrow(selected, allowed, key = (entry) => entry) {
  return allowed === undefined
    ? selected
    : selected.filter((entry) => allowed.includes(key(entry)));
}

const exitCode = await withPreview(async ({ origin, browser, problems }) => {
  mkdirSync(OUT_DIR, { recursive: true });
  let profileId = null;

  for (const variant of variants) {
    const variantRoutes = narrow(routes, variant.routes, (route) => route.name);
    for (const theme of narrow(themes, variant.themes)) {
      for (const lang of narrow(langs, variant.langs)) {
        if (variantRoutes.length === 0) {
          continue;
        }
        const page = await newPage(browser, { theme, lang, problems });
        for (const route of variantRoutes) {
          if (route.path.includes(':profileId') && profileId === null) {
            await openApp(page, appUrl(origin, '/', { theme, lang, params: variant.params }));
            profileId = await firstProfileId(page);
          }
          const target = route.path.replace(':profileId', encodeURIComponent(profileId ?? ''));
          await openApp(page, appUrl(origin, target, { theme, lang, params: variant.params }));
          if (variant.open === 'dialog') {
            await openDialog(page);
          }

          const file = path.join(OUT_DIR, `${route.name}-${variant.name}-${theme}-${lang}.png`);
          await page.screenshot({ path: file });
          console.log(`  ${path.relative(ROOT, file)}`);
        }
        await page.context().close();
      }
    }
  }
});

process.exit(exitCode);
