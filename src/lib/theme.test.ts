import { afterEach, describe, expect, it, vi } from 'vitest';

import { applyTheme, resolveTheme, systemPrefersDark, watchSystemTheme } from '@/lib/theme';

afterEach(() => {
  vi.unstubAllGlobals();
  document.documentElement.className = '';
  document.documentElement.removeAttribute('style');
});

/** A matchMedia stub whose dark preference can be flipped. */
function stubMatchMedia(initiallyDark: boolean) {
  const listeners = new Set<() => void>();
  const query = {
    matches: initiallyDark,
    addEventListener: (_: string, listener: () => void) => listeners.add(listener),
    removeEventListener: (_: string, listener: () => void) => listeners.delete(listener),
  };
  vi.stubGlobal('matchMedia', () => query);
  return {
    listeners,
    flip(dark: boolean) {
      query.matches = dark;
      listeners.forEach((listener) => listener());
    },
  };
}

describe('resolveTheme', () => {
  it.each([
    ['light', false, 'light'],
    ['light', true, 'light'],
    ['dark', false, 'dark'],
    ['dark', true, 'dark'],
    ['system', false, 'light'],
    ['system', true, 'dark'],
  ] as const)('%s with a dark OS = %s → %s', (theme, systemDark, expected) => {
    expect(resolveTheme(theme, systemDark)).toBe(expected);
  });
});

describe('system theme', () => {
  it('reads and follows the OS preference', () => {
    const media = stubMatchMedia(true);
    expect(systemPrefersDark()).toBe(true);
    const onChange = vi.fn();
    const stop = watchSystemTheme(onChange);
    media.flip(false);
    expect(onChange).toHaveBeenCalledOnce();
    expect(systemPrefersDark()).toBe(false);
    stop();
    expect(media.listeners.size).toBe(0);
  });

  it('defaults to light without matchMedia', () => {
    vi.stubGlobal('matchMedia', undefined);
    expect(systemPrefersDark()).toBe(false);
    expect(() => watchSystemTheme(() => {})()).not.toThrow();
  });
});

describe('applyTheme', () => {
  it('toggles .dark and color-scheme on <html>', () => {
    const root = document.documentElement;
    applyTheme('dark');
    expect(root.classList.contains('dark')).toBe(true);
    expect(root.style.colorScheme).toBe('dark');
    applyTheme('light');
    expect(root.classList.contains('dark')).toBe(false);
    expect(root.style.colorScheme).toBe('light');
  });
});
