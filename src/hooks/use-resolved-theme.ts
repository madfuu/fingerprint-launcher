import { useSyncExternalStore } from 'react';

import { useSettingsStore } from '@/features/settings/store';
import { resolveTheme, systemPrefersDark, watchSystemTheme, type ResolvedTheme } from '@/lib/theme';

/** The theme in effect: the `theme` setting, with `system` following the OS live. */
export function useResolvedTheme(): ResolvedTheme {
  const theme = useSettingsStore((state) => state.settings?.theme ?? 'system');
  const systemDark = useSyncExternalStore(watchSystemTheme, systemPrefersDark, () => false);
  return resolveTheme(theme, systemDark);
}
