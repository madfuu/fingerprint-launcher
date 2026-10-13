#!/usr/bin/env node
/**
 * Interaction checks that a screenshot cannot show, run against the Vite dev server with the
 * mock bridge (design §10):
 *
 *   npm run ui-check
 *
 * 1. The title bar stays usable while a modal dialog is open (design §4.5 / §7, P6). Radix
 *    turns pointer events off on `<body>` and dismisses the dialog on an outside pointerdown;
 *    the title bar opts out of both, so the window can still be moved, minimized or closed
 *    while a dialog asks something.
 * 2. Clicking anywhere else outside still dismisses the dialog: the opt-out is the title bar's
 *    alone.
 * 3. The window controls work: the maximize button's icon and label follow the window state
 *    (`core:window:allow-is-maximized` plus the resize subscription), and the close button is
 *    the only one that turns destructive on hover.
 * 4. The sidebar's width follows the `sidebarCollapsed` setting, toggling it persists the
 *    change through `updateSettings` (the mock's store, which the real backend writes to
 *    SQLite), and the collapsed entries name themselves in a tooltip.
 */
import { appUrl, newPage, openApp, openDialog, withPreview } from './lib/dev-preview.mjs';

const TITLEBAR = '[data-app-titlebar]';
const DIALOG = '[data-slot="dialog-content"][data-state="open"]';
const MINIMIZE = '[data-window-control="minimize"]';
const MAXIMIZE = '[data-window-control="toggle-maximize"]';
const CLOSE = '[data-window-control="close"]';

/** Sidebar widths in CSS pixels: Tailwind `w-16` and `w-58`. */
const COLLAPSED_PX = 64;
const EXPANDED_PX = 232;

const results = [];

function check(name, ok, detail = '') {
  results.push({ name, ok, detail });
  console.log(`  ${ok ? 'ok  ' : 'FAIL'}  ${name}${detail ? ` — ${detail}` : ''}`);
}

/**
 * Whether a computed colour is a red. Chromium reports theme colours as `oklab(L a b / α)`
 * (Tailwind 4 keeps them in that space); other engines may answer `rgb(...)`.
 */
function isRed(color) {
  const [first, second, third] = [...color.matchAll(/-?\d*\.?\d+/g)].map((match) =>
    Number(match[0])
  );
  if (color.startsWith('oklab')) {
    // `a` towards red, `b` not towards blue.
    return second > 0.05 && third > -0.02;
  }
  return first > 1.5 * second && first > 1.5 * third;
}

/** The sidebar's width once its width transition has finished (Web Animations API). */
function settledWidth(page) {
  return page.locator('aside[data-collapsed]').evaluate(async (aside) => {
    await Promise.all(aside.getAnimations().map((animation) => animation.finished));
    return aside.clientWidth;
  });
}

/** Window controls the mock recorded, in order. */
function windowCalls(page) {
  return page.evaluate(() => [...globalThis.__FPL_MOCK__.backend.windowCalls]);
}

async function titlebarDuringDialog(origin, browser, problems) {
  const page = await newPage(browser, { theme: 'light', lang: 'zh-CN', problems });
  await openApp(page, appUrl(origin, '/', { theme: 'light', lang: 'zh-CN' }));
  await openDialog(page);

  check('a dialog is open', (await page.locator(DIALOG).count()) === 1);
  const pointerEvents = await page
    .locator(TITLEBAR)
    .evaluate((element) => globalThis.getComputedStyle(element).pointerEvents);
  check('the title bar takes pointer events', pointerEvents === 'auto', pointerEvents);

  // The overlay must not cover the title bar either (index.css: it starts at 48px).
  const overlayTop = await page
    .locator('[data-slot="dialog-overlay"]')
    .evaluate((element) => element.getBoundingClientRect().top);
  check('the overlay starts below the title bar', overlayTop === 48, `top ${overlayTop}px`);

  await page.locator(MINIMIZE).click();
  await page.waitForTimeout(150);
  const calls = await windowCalls(page);
  check('the minimize button ran', calls.includes('minimize'), `calls: ${calls.join(', ') || '-'}`);
  check('the dialog is still open', (await page.locator(DIALOG).count()) === 1);

  // Dragging the title bar is Tauri's injected script, which the mock bridge has no part in;
  // what we can check here is that the bar is marked as a drag region for it.
  const dragRegion = await page.locator(TITLEBAR).getAttribute('data-tauri-drag-region');
  check('the title bar is a drag region', dragRegion === 'deep', String(dragRegion));

  // Clicking outside the dialog anywhere else still closes it (bottom-right corner: past the
  // centred dialog, below the title bar).
  await page.mouse.click(1380, 780);
  await page.waitForTimeout(300);
  check('clicking the page closes the dialog', (await page.locator(DIALOG).count()) === 0);

  await page.context().close();
}

async function windowControls(origin, browser, problems) {
  const page = await newPage(browser, { theme: 'light', lang: 'zh-CN', problems });
  await openApp(page, appUrl(origin, '/', { theme: 'light', lang: 'zh-CN' }));

  const maximize = page.locator(MAXIMIZE);
  check('the window starts restored', (await maximize.getAttribute('data-maximized')) === 'false');
  const restoreLabel = await maximize.getAttribute('aria-label');

  await maximize.click();
  await page.locator(`${MAXIMIZE}[data-maximized="true"]`).waitFor();
  const maximizeLabel = await maximize.getAttribute('aria-label');
  check('maximizing flips the button', (await maximize.getAttribute('data-maximized')) === 'true');
  check('the label changed with it', maximizeLabel !== restoreLabel, `now "${maximizeLabel}"`);

  await maximize.click();
  await page.locator(`${MAXIMIZE}[data-maximized="false"]`).waitFor();
  const calls = await windowCalls(page);
  check(
    'restoring flips it back',
    calls.filter((call) => call === 'toggleMaximize').length === 2,
    `calls: ${calls.join(', ')}`
  );

  // Only the close button turns destructive (#ef4444) on hover; the others take the accent.
  const background = (locator) =>
    locator.evaluate((element) => globalThis.getComputedStyle(element).backgroundColor);
  await page.locator(CLOSE).hover();
  await page.waitForTimeout(250);
  const closeHover = await background(page.locator(CLOSE));
  check('close hovers destructive red', isRed(closeHover), closeHover);
  await page.locator(MINIMIZE).hover();
  await page.waitForTimeout(250);
  const minimizeHover = await background(page.locator(MINIMIZE));
  check('minimize hovers accent, not red', !isRed(minimizeHover), minimizeHover);

  await page.context().close();
}

async function sidebarState(origin, browser, problems) {
  const page = await newPage(browser, { theme: 'light', lang: 'zh-CN', problems });
  await openApp(page, appUrl(origin, '/', { theme: 'light', lang: 'zh-CN' }));

  const collapsedWidth = await settledWidth(page);
  check(
    'the sidebar starts collapsed (w-16)',
    collapsedWidth === COLLAPSED_PX,
    `${collapsedWidth}px`
  );

  // Collapsed entries show their name on hover (no labels are visible then).
  const settings = page.locator('aside a[href="/settings"]');
  await settings.hover();
  const tooltip = page.locator('[data-slot="tooltip-content"]');
  await tooltip.first().waitFor();
  check(
    'a collapsed entry names itself in a tooltip',
    (await tooltip.first().innerText()) === '设置'
  );
  // Dismiss it (Escape) and move away, so it cannot take the toggle's click below.
  await page.keyboard.press('Escape');
  await page.mouse.move(700, 400);
  await tooltip.first().waitFor({ state: 'detached' });

  // The collapse toggle, found by its state rather than its copy. `force`: it pulses for
  // attention (research §6), so it never passes Playwright's stability check.
  await page.locator('aside button[aria-expanded="false"]').click({ force: true });
  // The setting is saved first, then the width animates over 300 ms.
  await page.locator('aside[data-collapsed="false"]').waitFor();
  const expandedWidth = await settledWidth(page);
  check('it expands to w-58', expandedWidth === EXPANDED_PX, `${expandedWidth}px`);

  const stored = await page.evaluate(
    () => globalThis.__FPL_MOCK__.backend.settings.sidebarCollapsed
  );
  check('the change went to the backend', stored === false, `sidebarCollapsed: ${String(stored)}`);

  // The initial width comes from the settings, not from a default in the component: the mock's
  // `?sidebar=` override only changes what `getSettings` answers.
  const fresh = await newPage(browser, { theme: 'light', lang: 'zh-CN', problems });
  await openApp(
    fresh,
    appUrl(origin, '/', { theme: 'light', lang: 'zh-CN', params: { sidebar: 'expanded' } })
  );
  const seeded = await fresh
    .locator('aside[data-collapsed="false"]')
    .evaluate((element) => element.clientWidth);
  check('a stored expanded state renders expanded', seeded === EXPANDED_PX, `${seeded}px`);

  await fresh.context().close();
  await page.context().close();
}

const exitCode = await withPreview(async ({ origin, browser, problems }) => {
  console.log('title bar during a modal dialog');
  await titlebarDuringDialog(origin, browser, problems);
  console.log('window controls');
  await windowControls(origin, browser, problems);
  console.log('sidebar state');
  await sidebarState(origin, browser, problems);

  const failed = results.filter((result) => !result.ok);
  console.log(`\n${results.length - failed.length}/${results.length} checks passed`);
  if (failed.length > 0) {
    throw new Error(`failed: ${failed.map((result) => result.name).join('; ')}`);
  }
});

process.exit(exitCode);
