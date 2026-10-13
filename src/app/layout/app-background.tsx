import { useEffect, useRef } from 'react';

import {
  BACKGROUND_PALETTES,
  paintBase,
  paintGlow,
  type Viewport,
} from '@/app/layout/background-paint';
import { useResolvedTheme } from '@/hooks/use-resolved-theme';

/** Repaint delay after the last resize: painting adds noise to every pixel, so not per frame. */
const RESIZE_DEBOUNCE_MS = 150;

function viewport(): Viewport {
  return {
    width: window.innerWidth,
    height: window.innerHeight,
    dpr: window.devicePixelRatio || 1,
  };
}

/**
 * Window background behind the frame (research §4): grid, dots and noise, plus blurred blue
 * glows in the bottom corners. Repainted on theme changes and, debounced, on resize; in
 * between, the canvases stretch with the window.
 */
export function AppBackground() {
  const theme = useResolvedTheme();
  const baseRef = useRef<HTMLCanvasElement>(null);
  const glowRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const palette = BACKGROUND_PALETTES[theme];
    const paint = () => {
      const size = viewport();
      if (baseRef.current) {
        paintBase(baseRef.current, size, palette);
      }
      if (glowRef.current) {
        paintGlow(glowRef.current, size, palette);
      }
    };
    paint();

    let timer: ReturnType<typeof setTimeout> | undefined;
    const onResize = () => {
      clearTimeout(timer);
      timer = setTimeout(paint, RESIZE_DEBOUNCE_MS);
    };
    window.addEventListener('resize', onResize);
    return () => {
      window.removeEventListener('resize', onResize);
      clearTimeout(timer);
    };
  }, [theme]);

  return (
    <div aria-hidden className="pointer-events-none fixed inset-0 -z-10">
      <canvas ref={baseRef} className="absolute inset-0 size-full" />
      <canvas ref={glowRef} className="absolute inset-0 size-full blur-[40px]" />
    </div>
  );
}
