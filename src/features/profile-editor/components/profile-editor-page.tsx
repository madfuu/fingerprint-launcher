import { SquarePen, SquarePlus } from 'lucide-react';
import { useParams } from 'react-router';
import { useTranslation } from 'react-i18next';

import { PagePlaceholder } from '@/components/page-placeholder';

/** P5 placeholder of the profile editor (P8): `/profiles/new` and `/profiles/:id/edit`. */
export function ProfileEditorPage() {
  const { t } = useTranslation('profileEditor');
  const { id } = useParams();
  return (
    <PagePlaceholder
      icon={id === undefined ? SquarePlus : SquarePen}
      title={id === undefined ? t('createTitle') : t('editTitle')}
      description={t('description')}
    >
      {id !== undefined && <p className="truncate font-mono text-xs text-muted-foreground">{id}</p>}
    </PagePlaceholder>
  );
}
