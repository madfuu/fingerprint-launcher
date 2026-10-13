import { useLayoutEffect, type ReactNode } from 'react';

import { useResolvedTheme } from '@/hooks/use-resolved-theme';
import { applyTheme } from '@/lib/theme';

/**
 * Keeps `<html>` on the theme of the `theme` setting (R7.3), following OS changes while it is
 * `system`. The bootstrap applies the first theme before anything renders, so there is no
 * flash of the wrong one.
 */
export function ThemeProvider({ children }: { children: ReactNode }) {
  const resolved = useResolvedTheme();
  useLayoutEffect(() => {
    applyTheme(resolved);
  }, [resolved]);
  return children;
}
