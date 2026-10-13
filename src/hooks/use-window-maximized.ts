import { useEffect, useState } from 'react';

import { isWindowMaximized, onWindowResized, type Unlisten } from '@/lib/bridge';

/** Resize events arrive per frame while the user drags an edge; ask once it settles. */
const RESIZE_SETTLE_MS = 100;

/**
 * Whether the window is maximized, for the title bar's maximize / restore button. Follows
 * maximizing and restoring from anywhere (double-clicking the title bar, the window manager,
 * keyboard shortcuts), since each of them resizes the window.
 */
export function useWindowMaximized(): boolean {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let live = true;
    let unlisten: Unlisten | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const refresh = () => {
      isWindowMaximized()
        .then((value) => live && setMaximized(value))
        .catch((error: unknown) => console.error('cannot read the window state', error));
    };
    refresh();
    onWindowResized(() => {
      clearTimeout(timer);
      timer = setTimeout(refresh, RESIZE_SETTLE_MS);
    })
      .then((stop) => {
        if (live) {
          unlisten = stop;
        } else {
          stop();
        }
      })
      .catch((error: unknown) => console.error('cannot follow window resizes', error));

    return () => {
      live = false;
      clearTimeout(timer);
      unlisten?.();
    };
  }, []);

  return maximized;
}
