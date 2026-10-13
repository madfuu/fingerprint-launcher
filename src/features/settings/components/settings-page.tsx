import { Settings } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { PagePlaceholder } from '@/components/page-placeholder';

/** P5 placeholder of the settings page (P9). */
export function SettingsPage() {
  const { t } = useTranslation('settings');
  return <PagePlaceholder icon={Settings} title={t('title')} description={t('description')} />;
}
