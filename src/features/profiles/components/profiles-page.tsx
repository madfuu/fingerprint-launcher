import { LayoutList } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { PagePlaceholder } from '@/components/page-placeholder';
import { useActiveCount } from '@/features/profiles/running-store';
import { listProfiles } from '@/lib/bridge';

/** P5 placeholder of the profile list (P7). Shows that the bridge and running store work. */
export function ProfilesPage() {
  const { t } = useTranslation('profiles');
  const running = useActiveCount();
  const [total, setTotal] = useState<number | null>(null);

  useEffect(() => {
    let live = true;
    listProfiles({ page: 1, pageSize: 1, view: 'all', groupId: null, tagIds: [], keyword: null })
      .then((page) => live && setTotal(page.total))
      .catch(() => live && setTotal(null));
    return () => {
      live = false;
    };
  }, []);

  return (
    <PagePlaceholder icon={LayoutList} title={t('title')} description={t('description')}>
      {total !== null && (
        <p className="text-xs text-foreground tabular-nums">{t('summary', { total, running })}</p>
      )}
    </PagePlaceholder>
  );
}
