/**
 * Startup (design §6): read the settings → apply language and theme → render → show the
 * window, which the backend created hidden so the first frame already has the right theme.
 *
 * The window is shown whatever happens: if the settings cannot be read the error screen is
 * rendered instead of the app, and a failure anywhere still ends in `showMainWindow()` (the
 * backend's 5 s fallback is not relied on).
 */
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { I18nextProvider } from 'react-i18next';

import { App } from '@/app/app';
import { AppErrorBoundary } from '@/app/error-boundary';
import { SettingsReadError } from '@/app/fatal-error';
import { ReadySignal } from '@/app/ready-signal';
import { useRunningStore } from '@/features/profiles/running-store';
import { useSettingsStore } from '@/features/settings/store';
import { DEFAULT_LOCALE, i18n, initI18n } from '@/i18n';
import type { Settings } from '@/lib/bindings';
import { getSettings, showMainWindow } from '@/lib/bridge';
import { applyTheme, resolveTheme, systemPrefersDark } from '@/lib/theme';

/** Longest wait for the first commit before the window is shown anyway. */
const RENDER_TIMEOUT_MS = 3000;

async function start(container: HTMLElement): Promise<void> {
  let settings: Settings | null = null;
  let failure: unknown = null;
  try {
    settings = await getSettings();
  } catch (error) {
    failure = error;
    console.error('cannot read the settings', error);
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
  await Promise.race([
    rendered,
    new Promise<void>((resolve) => setTimeout(resolve, RENDER_TIMEOUT_MS)),
  ]);
}

export async function bootstrap(container: HTMLElement): Promise<void> {
  try {
    await start(container);
  } catch (error) {
    // Nothing could render (i18n or React failed): at least show why.
    console.error('startup failed', error);
    container.textContent = error instanceof Error ? (error.stack ?? error.message) : String(error);
  } finally {
    try {
      await showMainWindow();
    } catch (error) {
      console.error('cannot show the main window', error);
    }
    // Screenshot scripts wait for this.
    document.documentElement.dataset.appReady = 'true';
  }
}
