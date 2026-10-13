/**
 * Startup (design §6): read the settings and app info → apply language and theme → render →
 * show the window, which the backend created hidden so the first frame already has the right
 * theme.
 *
 * The window is shown whatever happens: if the settings cannot be read the error screen is
 * rendered instead of the app, and a failure anywhere still ends in `showMainWindow()` (the
 * backend's 5 s fallback is not relied on). What went wrong is passed along with it (see
 * `boot-status.ts`).
 */
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nextProvider } from 'react-i18next';

import { App } from '@/app/app';
import { useAppInfoStore } from '@/app/app-info';
import { bootFailure, reportBootFailure } from '@/app/boot-status';
import { AppErrorBoundary } from '@/app/error-boundary';
import { SettingsReadError } from '@/app/fatal-error';
import { ReadySignal } from '@/app/ready-signal';
import { useRunningStore } from '@/features/profiles/running-store';
import { useSettingsStore } from '@/features/settings/store';
import { DEFAULT_LOCALE, i18n, initI18n } from '@/i18n';
import type { Settings } from '@/lib/bindings';
import { getAppInfo, getSettings, showMainWindow, toAppError } from '@/lib/bridge';
import { applyTheme, resolveTheme, systemPrefersDark } from '@/lib/theme';

/** Longest wait for the first commit before the window is shown anyway. */
const RENDER_TIMEOUT_MS = 3000;

function describe(error: unknown): string {
  const { code, message } = toAppError(error);
  return `${code}: ${message}`;
}

async function start(container: HTMLElement): Promise<void> {
  const [settingsRead, infoRead] = await Promise.allSettled([getSettings(), getAppInfo()]);
  let settings: Settings | null = null;
  let failure: unknown = null;
  if (settingsRead.status === 'fulfilled') {
    settings = settingsRead.value;
  } else {
    failure = settingsRead.reason;
    console.error('cannot read the settings', failure);
    reportBootFailure(`getSettings: ${describe(failure)}`);
  }
  if (infoRead.status === 'fulfilled') {
    useAppInfoStore.getState().load(infoRead.value);
  } else {
    // The title bar falls back to the user agent; the app still works.
    console.error('cannot read the app info', infoRead.reason);
    reportBootFailure(`getAppInfo: ${describe(infoRead.reason)}`);
  }

  applyTheme(resolveTheme(settings?.theme ?? 'system', systemPrefersDark()));
  await initI18n(settings?.locale ?? DEFAULT_LOCALE);
  if (settings) {
    useSettingsStore.getState().load(settings);
    void useRunningStore.getState().init();
  }

  let signalReady = () => {};
  const rendered = new Promise<void>((resolve) => {
    signalReady = resolve;
  });
  createRoot(container).render(
    <StrictMode>
      <I18nextProvider i18n={i18n}>
        {settings ? (
          <AppErrorBoundary>
            <App />
          </AppErrorBoundary>
        ) : (
          <SettingsReadError error={failure} />
        )}
        <ReadySignal onReady={signalReady} />
      </I18nextProvider>
    </StrictMode>
  );
  const committed = await Promise.race([
    rendered.then(() => true),
    new Promise<boolean>((resolve) => setTimeout(() => resolve(false), RENDER_TIMEOUT_MS)),
  ]);
  if (!committed) {
    reportBootFailure(`the first render did not commit within ${RENDER_TIMEOUT_MS} ms`);
  }
}

export async function bootstrap(container: HTMLElement): Promise<void> {
  try {
    await start(container);
  } catch (error) {
    // Nothing could render (i18n or React failed): at least show why.
    console.error('startup failed', error);
    reportBootFailure(`startup: ${describe(error)}`);
    container.textContent = error instanceof Error ? (error.stack ?? error.message) : String(error);
  } finally {
    const failure = bootFailure();
    try {
      await showMainWindow(failure === null, failure);
    } catch (error) {
      console.error('cannot show the main window', error);
    }
    // Screenshot scripts wait for this.
    document.documentElement.dataset.appReady = 'true';
  }
}
