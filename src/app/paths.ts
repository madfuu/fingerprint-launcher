/** Route paths (design §6), for links and navigation. */
export const paths = {
  profiles: '/',
  newProfile: '/profiles/new',
  editProfile: (id: string) => `/profiles/${encodeURIComponent(id)}/edit`,
  /** Route pattern of {@link paths.editProfile}. */
  editProfilePattern: '/profiles/:id/edit',
  groups: '/groups',
  proxies: '/proxies',
  settings: '/settings',
} as const;
