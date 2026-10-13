/**
 * Light / dark theme resolution (R7.3). The `theme` setting may follow the OS; the result is
 * applied as the `.dark` class on `<html>` (the `dark` variant in `index.css`).
 */
import type { Theme } from '@/lib/bindings';

export type ResolvedTheme = 'light' | 'dark';

const DARK_QUERY = '(prefers-color-scheme: dark)';

export function resolveTheme(theme: Theme, systemPrefersDark: boolean): ResolvedTheme {
  switch (theme) {
    case 'light':
    case 'dark':
      return theme;
    case 'system':
      return systemPrefersDark ? 'dark' : 'light';
  }
}

function darkQuery(): MediaQueryList | null {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function'
    ? window.matchMedia(DARK_QUERY)
    : null;
}

export function systemPrefersDark(): boolean {
  return darkQuery()?.matches ?? false;
}

/** Calls `onChange` whenever the OS switches between light and dark; returns the unsubscriber. */
export function watchSystemTheme(onChange: () => void): () => void {
  const query = darkQuery();
  if (!query) {
    return () => {};
  }
  query.addEventListener('change', onChange);
  return () => query.removeEventListener('change', onChange);
}

/** Puts the theme on `<html>`: the `.dark` class plus `color-scheme` for native controls. */
export function applyTheme(resolved: ResolvedTheme, root: HTMLElement = document.documentElement) {
  root.classList.toggle('dark', resolved === 'dark');
  root.style.colorScheme = resolved;
}
