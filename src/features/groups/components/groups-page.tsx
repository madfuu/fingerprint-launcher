import { FolderTree } from 'lucide-react';
import { useTranslation } from 'react-i18next';

import { PagePlaceholder } from '@/components/page-placeholder';

/** P5 placeholder of the groups and tags page (P9). */
export function GroupsPage() {
  const { t } = useTranslation('groups');
  return <PagePlaceholder icon={FolderTree} title={t('title')} description={t('description')} />;
}
