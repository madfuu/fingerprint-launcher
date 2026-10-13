import { useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { toast } from 'sonner';

import { useSettingsStore } from '@/features/settings/store';
import { localizeError } from '@/i18n/errors';
import type { SettingsPatch } from '@/lib/bindings';

/**
 * Saves a settings change from a control that applies at once (theme, language, sidebar): the
 * UI follows when the backend has stored it; a failure leaves everything as it was and says so
 * in a toast.
 */
export function useUpdateSettings(): (patch: SettingsPatch) => void {
  const { t } = useTranslation('settings');
  const { t: tc } = useTranslation();
  return useCallback(
    (patch: SettingsPatch) => {
      useSettingsStore
        .getState()
        .update(patch)
        .catch((error: unknown) => {
          const { message, detail } = localizeError(tc, error);
          toast.error(t('saveFailed'), { description: `${message} ${detail}` });
        });
    },
    [t, tc]
  );
}
