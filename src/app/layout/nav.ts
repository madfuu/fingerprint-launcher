/**
 * Navigation of the app frame: the sidebar's entries (research/simprint-ui-reference.md §6,
 * "本项目导航") and the page the title bar's breadcrumb names. Labels are `common:nav.<key>`.
 */
import {
  FolderTree,
  LayoutList,
  Network,
  Settings,
  SquarePen,
  SquarePlus,
  type LucideIcon,
} from 'lucide-react';
import { matchPath } from 'react-router';

import { paths } from '@/app/paths';

/** A key of `common:nav`. */
export type NavKey = 'profiles' | 'newProfile' | 'editProfile' | 'groups' | 'proxies' | 'settings';

export interface NavEntry {
  key: NavKey;
  to: string;
  icon: LucideIcon;
}

export const NAV = {
  profiles: { key: 'profiles', to: paths.profiles, icon: LayoutList },
  newProfile: { key: 'newProfile', to: paths.newProfile, icon: SquarePlus },
  groups: { key: 'groups', to: paths.groups, icon: FolderTree },
  proxies: { key: 'proxies', to: paths.proxies, icon: Network },
  settings: { key: 'settings', to: paths.settings, icon: Settings },
} satisfies Record<string, NavEntry>;

/** A titled block of the sidebar; `title` is a key of `layout:sidebar`. */
export interface NavSection {
  title: 'resources';
  entries: NavEntry[];
}

/** The sidebar from top to bottom: under the "new profile" button, sections, then the foot. */
export const SIDEBAR: { top: NavEntry[]; sections: NavSection[]; bottom: NavEntry[] } = {
  top: [NAV.profiles, NAV.groups],
  sections: [{ title: 'resources', entries: [NAV.proxies] }],
  bottom: [NAV.settings],
};

/** Whether `entry` is the current page. The profile editor belongs to the profile list. */
export function isNavActive(entry: NavEntry, pathname: string): boolean {
  if (entry.to === paths.profiles) {
    return pathname === paths.profiles || pathname.startsWith('/profiles/');
  }
  return pathname === entry.to || pathname.startsWith(`${entry.to}/`);
}

interface Page {
  pattern: string;
  key: NavKey;
  icon: LucideIcon;
}

/** Every route of `routes.tsx`, most specific first. */
const PAGES: Page[] = [
  { pattern: paths.newProfile, key: 'newProfile', icon: SquarePlus },
  { pattern: paths.editProfilePattern, key: 'editProfile', icon: SquarePen },
  { pattern: paths.groups, key: 'groups', icon: FolderTree },
  { pattern: paths.proxies, key: 'proxies', icon: Network },
  { pattern: paths.settings, key: 'settings', icon: Settings },
  { pattern: paths.profiles, key: 'profiles', icon: LayoutList },
];

/** The page at `pathname`, for the breadcrumb; `null` for an unknown path. */
export function currentPage(pathname: string): Page | null {
  return PAGES.find((page) => matchPath(page.pattern, pathname) !== null) ?? null;
}
