/**
 * Shared harness for the Playwright-driven scripts (`screenshot.mjs`, `ui-check.mjs`): the Vite
 * dev server with the mock bridge, and Chrome for Testing to drive it.
 *
 * The browser is FPL_SCREENSHOT_CHROME (default: the Chrome for Testing that Playwright keeps
 * in ~/.cache/ms-playwright); playwright-core never downloads one. Vite runs inside the
 * script's process on a random free port (never 5173, which belongs to `tauri dev`), so it
 * stops with the script; the browser and server are closed on success, failure and Ctrl+C
 * alike.
 */
import { existsSync } from 'node:fs';
import { createServer as createNetServer } from 'node:net';
import { homedir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { chromium } from 'playwright-core';
import { createServer } from 'vite';

export const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
/** The app's default window size (design §7). */
export const VIEWPORT = { width: 1438, height: 828 };

const TAURI_DEV_PORT = 5173;
const READY_TIMEOUT_MS = 30_000;
/** After the app is ready: lets data load and enter animations finish. */
const SETTLE_MS = 600;

const DEFAULT_CHROME = path.join(
  homedir(),
  '.cache/ms-playwright/chromium-1234/chrome-linux64/chrome'
);

/** Exit code 2: the script was called wrong or its browser is missing. */
export function fail(message) {
  console.error(`${path.basename(process.argv[1] ?? 'script')}: ${message}`);
  process.exit(2);
}

/** `--thing a,b` → the matching entries of `all`. */
export function select(all, value, kind, key = (entry) => entry) {
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

/**
 * Runs `body({ origin, browser, problems })` against a fresh dev server and browser, then
 * closes both. Errors reported by the pages land in `problems`.
 *
 * Returns the process exit code: 0 when `body` resolved and no page reported an error, 1
 * otherwise.
 */
export async function withPreview(body) {
  const executablePath = process.env.FPL_SCREENSHOT_CHROME || DEFAULT_CHROME;
  if (!existsSync(executablePath)) {
    fail(`browser not found: ${executablePath} (set FPL_SCREENSHOT_CHROME)`);
  }

  let server = null;
  let browser = null;
  const cleanup = async () => {
    const closing = [browser?.close(), server?.close()];
    browser = null;
    server = null;
    await Promise.allSettled(closing);
  };

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
      server: {
        host: '127.0.0.1',
        port,
        strictPort: true,
        hmr: false,
        // Transform the dev-only modules before the browser connects: a dependency first
        // seen afterwards (the probe's `cmdk`) would re-run the optimizer and hand the page
        // a second copy of React.
        warmup: { clientFiles: ['./src/dev/*.tsx'] },
      },
    });
    await server.listen();
    const origin = `http://127.0.0.1:${port}`;
    console.log(`vite (mock bridge) on ${origin}, pid ${process.pid}`);

    browser = await chromium.launch({ executablePath, headless: true });
    await body({ origin, browser, problems });
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
  return exitCode;
}

/** A page at the app's window size, in `theme` / `lang`, reporting its errors into `problems`. */
export async function newPage(browser, { theme, lang, problems }) {
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
  return page;
}

/** `?theme=&lang=` plus the variant's own overrides, which the mock bridge reads. */
export function appUrl(origin, route, { theme, lang, params = {} }) {
  const query = new URLSearchParams({ theme, lang, ...params });
  return `${origin}${route}?${query.toString()}`;
}

/** Navigates to `url` and waits until the app has rendered and settled. */
export async function openApp(page, url) {
  await page.goto(url);
  await page.waitForSelector('html[data-app-ready="true"]', { timeout: READY_TIMEOUT_MS });
  await page.waitForLoadState('networkidle');
  await page.evaluate(() => globalThis.document.fonts.ready);
  await page.waitForTimeout(SETTLE_MS);
}

/**
 * Opens the app's command dialog over the current page (`src/dev/`, which only these scripts
 * load, through the dev server), and waits for it.
 */
export async function openDialog(page) {
  await page.evaluate(async () => {
    const probe = await import('/src/dev/mount-dialog-probe.tsx');
    probe.openDialogProbe();
  });
  await page.waitForSelector('[data-slot="dialog-content"][data-state="open"]');
  await page.waitForTimeout(300);
}

/** The first seeded profile, for the routes that need an id. */
export function firstProfileId(page) {
  // Runs in the page: the mock exposes itself as `window.__FPL_MOCK__`.
  return page.evaluate(() => [...globalThis.__FPL_MOCK__.backend.profiles.keys()][0]);
}
