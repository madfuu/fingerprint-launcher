import { Navigate, Route, Routes } from 'react-router';

import { AppLayout } from '@/app/layout/app-layout';
import { paths } from '@/app/paths';
import { GroupsPage } from '@/features/groups/components/groups-page';
import { ProfileEditorPage } from '@/features/profile-editor/components/profile-editor-page';
import { ProfilesPage } from '@/features/profiles/components/profiles-page';
import { ProxiesPage } from '@/features/proxies/components/proxies-page';
import { SettingsPage } from '@/features/settings/components/settings-page';

/** Routes of design §6. */
export function AppRoutes() {
  return (
    <Routes>
      <Route element={<AppLayout />}>
        <Route index element={<ProfilesPage />} />
        <Route path={paths.newProfile} element={<ProfileEditorPage />} />
        <Route path={paths.editProfilePattern} element={<ProfileEditorPage />} />
        <Route path={paths.groups} element={<GroupsPage />} />
        <Route path={paths.proxies} element={<ProxiesPage />} />
        <Route path={paths.settings} element={<SettingsPage />} />
        {/* The WebView starts at /index.html; anything unknown goes to the list. */}
        <Route path="*" element={<Navigate to={paths.profiles} replace />} />
      </Route>
    </Routes>
  );
}
