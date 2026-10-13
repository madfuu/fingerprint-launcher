import { Network } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { PagePlaceholder } from '@/components/page-placeholder';

/** P5 placeholder of the proxy center (P9). */
export function ProxiesPage() {
  const { t } = useTranslation('proxies');
  return <PagePlaceholder icon={Network} title={t('title')} description={t('description')} />;
}
