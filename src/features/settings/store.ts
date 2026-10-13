/**
 * App settings (design §2.2 `settings`): read once at startup, then changed only through
 * {@link SettingsState.update}, which persists first and applies what the backend stored. The
 * theme follows through `ThemeProvider`; the UI language is switched here.
 */
import { create } from 'zustand';

import { changeLocale } from '@/i18n';
import type { Settings, SettingsPatch } from '@/lib/bindings';
import { updateSettings } from '@/lib/bridge';

export interface SettingsState {
  /** `null` until the bootstrap loads them. */
  settings: Settings | null;
  load(settings: Settings): void;
  /** Persists `patch`; resolves with the stored settings (rejects with a `BridgeError`). */
  update(patch: SettingsPatch): Promise<Settings>;
}

export const useSettingsStore = create<SettingsState>()((set, get) => ({
  settings: null,

  load(settings) {
    set({ settings });
  },

  async update(patch) {
    const previous = get().settings;
    const stored = await updateSettings(patch);
    set({ settings: stored });
    if (stored.locale !== previous?.locale) {
      await changeLocale(stored.locale);
    }
    return stored;
  },
}));

/** The loaded settings; only for components rendered after the bootstrap. */
export function useSettings(): Settings {
  const settings = useSettingsStore((state) => state.settings);
  if (!settings) {
    throw new Error('settings are read before the app renders');
  }
  return settings;
}
