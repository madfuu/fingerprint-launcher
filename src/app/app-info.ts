/**
 * Facts about the host that the shell needs before anything renders (`getAppInfo`): mainly the
 * OS, which decides the title bar (design §7: native traffic lights on macOS, our own window
 * controls elsewhere). Loaded once by the bootstrap.
 */
import { create } from 'zustand';

import type { AppInfo, HostOs } from '@/lib/bindings';

interface AppInfoState {
  /** `null` until loaded, or when `getAppInfo` failed. */
  info: AppInfo | null;
  load(info: AppInfo): void;
}

export const useAppInfoStore = create<AppInfoState>()((set) => ({
  info: null,
  load(info) {
    set({ info });
  },
}));

/** The OS as the WebView's user agent tells it; only used when `getAppInfo` failed. */
export function guessHostOs(userAgent: string): HostOs {
  if (/Macintosh|Mac OS X/i.test(userAgent)) {
    return 'macos';
  }
  return /Windows/i.test(userAgent) ? 'windows' : 'linux';
}

/** The OS the launcher runs on. */
export function useHostOs(): HostOs {
  const os = useAppInfoStore((state) => state.info?.os);
  return os ?? guessHostOs(navigator.userAgent);
}
