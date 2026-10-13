import { useEffect } from 'react';

/**
 * Calls `onReady` once the tree has committed. Rendered after the app, so its effect runs
 * after the effects of everything before it.
 */
export function ReadySignal({ onReady }: { onReady: () => void }) {
  useEffect(() => {
    onReady();
  }, [onReady]);
  return null;
}
