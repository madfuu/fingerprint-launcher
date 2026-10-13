#!/usr/bin/env node
/**
 * Screenshots for visual review (design §10): every route × theme × language, rendered by the
 * Vite dev server with the mock bridge, in Chrome for Testing at the app's default window size.
 *
 *   npm run screenshots
 *   npm run screenshots -- --route profiles,settings --theme dark --lang en
 *
 * Output: .screenshots/<route>-<theme>-<lang>.png
 *
 * The browser is FPL_SCREENSHOT_CHROME (default: the Chrome for Testing that Playwright keeps
 * in ~/.cache/ms-playwright); playwright-core never downloads one. Vite runs inside this
 * process on a random free port (never 5173, which belongs to `tauri dev`), so it stops with
 * the script; the browser and server are closed on success, failure and Ctrl+C alike.
 */
import { existsSync, mkdirSync } from 'node:fs';
import { createServer as createNetServer } from 'node:net';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

import { chromium } from 'playwright-core';
import { createServer } from 'vite';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const OUT_DIR = path.join(ROOT, '.screenshots');
const VIEWPORT = { width: 1438, height: 828 };
const TAURI_DEV_PORT = 5173;
const READY_TIMEOUT_MS = 30_000;
/** After the app is ready: lets data load and enter animations finish. */
const SETTLE_MS = 600;

/** `path` may contain `:profileId`, replaced with the first seeded profile. */
const ROUTES = [
  { name: 'profiles', path: '/' },
  { name: 'profile-new', path: '/profiles/new' },
  { name: 'profile-edit', path: '/profiles/:profileId/edit' },
  { name: 'groups', path: '/groups' },
  { name: 'proxies', path: '/proxies' },
  { name: 'settings', path: '/settings' },
];
const THEMES = ['light', 'dark'];
const LANGS = ['zh-CN', 'en'];

const DEFAULT_CHROME = path.join(
  homedir(),
  '.cache/ms-playwright/chromium-1234/chrome-linux64/chrome'
);

function fail(message) {
  console.error(`screenshots: ${message}`);
  process.exit(2);
}

/** `--route a,b` → the matching entries of `all`. */
function select(all, value, kind, key = (entry) => entry) {
  if (value === undefined) {
    return all;
  }
  const wanted = value.split(',').map((item) => item.trim());
  const unknown = wanted.filter((item) => !all.some((entry) => key(entry) === item));
  if (unknown.length > 0) {
    fail(`unknown ${kind} ${unknown.join(', ')}; expected one of ${all.map(key).join(', ')}`);
  }
  return all.filter((entry) => wanted.includes(key(entry)));
}

/** A free TCP port above 1024 chosen by the OS, never Tauri's dev port. */
async function freePort() {
  for (;;) {
    const port = await new Promise((resolve, reject) => {
      const probe = createNetServer();
      probe.unref();
      probe.on('error', reject);
      probe.listen(0, '127.0.0.1', () => {
        const { port: chosen } = probe.address();
        probe.close(() => resolve(chosen));
      });
    });
    if (port > 1024 && port !== TAURI_DEV_PORT) {
      return port;
    }
  }
}

function parseOptions() {
  try {
    return parseArgs({
      options: {
        route: { type: 'string' },
        theme: { type: 'string' },
        lang: { type: 'string' },
      },
    }).values;
  } catch (error) {
    return fail(error.message);
  }
}

const options = parseOptions();
const routes = select(ROUTES, options.route, 'route', (route) => route.name);
const themes = select(THEMES, options.theme, 'theme');
const langs = select(LANGS, options.lang, 'lang');

const executablePath = process.env.FPL_SCREENSHOT_CHROME || DEFAULT_CHROME;
if (!existsSync(executablePath)) {
  fail(`browser not found: ${executablePath} (set FPL_SCREENSHOT_CHROME)`);
}

let server = null;
let browser = null;

async function cleanup() {
  const closing = [browser?.close(), server?.close()];
  browser = null;
  server = null;
  await Promise.allSettled(closing);
}

// 128 + signal number, as a shell reports a process killed by it.
for (const [signal, code] of [
  ['SIGINT', 130],
  ['SIGTERM', 143],
]) {
  process.once(signal, () => {
    void cleanup().finally(() => process.exit(code));
  });
}

const problems = [];
let exitCode = 0;
try {
  const port = await freePort();
  server = await createServer({
    root: ROOT,
    configFile: path.join(ROOT, 'vite.config.ts'),
    logLevel: 'warn',
    clearScreen: false,
    server: { host: '127.0.0.1', port, strictPort: true, hmr: false },
  });
  await server.listen();
  const origin = `http://127.0.0.1:${port}`;
  console.log(`vite (mock bridge) on ${origin}, pid ${process.pid}`);

  browser = await chromium.launch({ executablePath, headless: true });
  mkdirSync(OUT_DIR, { recursive: true });

  let profileId = null;
  for (const theme of themes) {
    for (const lang of langs) {
      const context = await browser.newContext({
        viewport: VIEWPORT,
        deviceScaleFactor: 1,
        colorScheme: theme,
        locale: lang,
      });
      const page = await context.newPage();
      page.on('pageerror', (error) => problems.push(`${page.url()}: ${error.message}`));
      page.on('console', (message) => {
        if (message.type() === 'error') {
          problems.push(`${page.url()}: console: ${message.text()}`);
        }
      });

      for (const route of routes) {
        if (route.path.includes(':profileId') && profileId === null) {
          await page.goto(`${origin}/?theme=${theme}&lang=${lang}`);
          await page.waitForSelector('html[data-app-ready="true"]', { timeout: READY_TIMEOUT_MS });
          profileId = await page.evaluate(
            // Runs in the page: the mock exposes itself as `window.__FPL_MOCK__`.
            () => [...globalThis.__FPL_MOCK__.backend.profiles.keys()][0]
          );
        }
        const target = route.path.replace(':profileId', encodeURIComponent(profileId ?? ''));
        await page.goto(`${origin}${target}?theme=${theme}&lang=${lang}`);
        await page.waitForSelector('html[data-app-ready="true"]', { timeout: READY_TIMEOUT_MS });
        await page.waitForLoadState('networkidle');
        await page.evaluate(() => globalThis.document.fonts.ready);
        await page.waitForTimeout(SETTLE_MS);

        const file = path.join(OUT_DIR, `${route.name}-${theme}-${lang}.png`);
        await page.screenshot({ path: file });
        console.log(`  ${path.relative(ROOT, file)}`);
      }
      await context.close();
    }
  }
} catch (error) {
  console.error(error);
  exitCode = 1;
} finally {
  await cleanup();
}

if (problems.length > 0) {
  console.error(`\n${problems.length} page error(s):`);
  for (const problem of problems) {
    console.error(`  ${problem}`);
  }
  exitCode = 1;
}
process.exit(exitCode);
