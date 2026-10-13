/**
 * Painting of the window background (research/simprint-ui-reference.md §4): a base layer of
 * fill, fine grid, sparse dots and pixel noise, and a glow layer with two soft blue lights in
 * the bottom corners (the glow canvas is blurred by CSS).
 *
 * Both layers are sized in CSS pixels and drawn at the device pixel ratio, so lines stay crisp
 * on HiDPI screens.
 */
import type { ResolvedTheme } from '@/lib/theme';

export interface BackgroundPalette {
  base: string;
  grid: string;
  dot: string;
  /** `r, g, b` of the corner glows. */
  glow: string;
  /** Peak opacity of a glow, at its centre. */
  glowOpacity: number;
}

export const BACKGROUND_PALETTES: Record<ResolvedTheme, BackgroundPalette> = {
  light: {
    base: '#ffffff',
    grid: 'rgba(0, 0, 0, 0.03)',
    dot: 'rgba(0, 0, 0, 0.012)',
    glow: '37, 99, 235', // blue-600
    glowOpacity: 0.09,
  },
  dark: {
    base: '#0a0a0b',
    grid: 'rgba(255, 255, 255, 0.06)',
    dot: 'rgba(255, 255, 255, 0.012)',
    glow: '147, 197, 253', // sky-300
    glowOpacity: 0.15,
  },
};

/** CSS px between grid lines. */
const GRID_STEP = 6;
const GRID_LINE_WIDTH = 0.5;
/** CSS px between dots, and their radius. */
const DOT_STEP = 24;
const DOT_RADIUS = 0.8;
/** Share of device pixels nudged by noise, and the largest nudge per channel. */
const NOISE_SHARE = 0.06;
const NOISE_AMPLITUDE = 1.25;
/** Glow centres as fractions of the width and height. */
const GLOW_CENTRES: readonly (readonly [number, number])[] = [
  [0.1, 0.9],
  [0.9, 0.9],
];
/** Glow radius as a fraction of the shorter side. */
const GLOW_RADIUS = 0.4;
/** Falloff from the centre: [position along the radius, share of the peak opacity]. */
const GLOW_FALLOFF: readonly (readonly [number, number])[] = [
  [0, 1],
  [0.2, 0.7],
  [0.5, 0.4],
  [0.8, 0.15],
  [1, 0],
];

export interface Viewport {
  width: number;
  height: number;
  dpr: number;
}

/** Resizes `canvas` to the viewport and returns a context that draws in CSS pixels. */
function prepare(canvas: HTMLCanvasElement, { width, height, dpr }: Viewport) {
  canvas.width = Math.max(1, Math.round(width * dpr));
  canvas.height = Math.max(1, Math.round(height * dpr));
  const ctx = canvas.getContext('2d');
  ctx?.setTransform(dpr, 0, 0, dpr, 0, 0);
  return ctx;
}

export function paintBase(
  canvas: HTMLCanvasElement,
  viewport: Viewport,
  palette: BackgroundPalette,
  random: () => number = Math.random
): void {
  const ctx = prepare(canvas, viewport);
  if (!ctx) {
    return;
  }
  const { width, height } = viewport;

  ctx.fillStyle = palette.base;
  ctx.fillRect(0, 0, width, height);

  ctx.beginPath();
  for (let x = 0; x <= width; x += GRID_STEP) {
    ctx.moveTo(x, 0);
    ctx.lineTo(x, height);
  }
  for (let y = 0; y <= height; y += GRID_STEP) {
    ctx.moveTo(0, y);
    ctx.lineTo(width, y);
  }
  ctx.lineWidth = GRID_LINE_WIDTH;
  ctx.strokeStyle = palette.grid;
  ctx.stroke();

  ctx.beginPath();
  for (let x = DOT_STEP; x < width; x += DOT_STEP) {
    for (let y = DOT_STEP; y < height; y += DOT_STEP) {
      ctx.moveTo(x + DOT_RADIUS, y);
      ctx.arc(x, y, DOT_RADIUS, 0, Math.PI * 2);
    }
  }
  ctx.fillStyle = palette.dot;
  ctx.fill();

  addNoise(ctx, canvas.width, canvas.height, random);
}

/** Nudges the RGB of a random share of the device pixels up or down by a hair. */
function addNoise(
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  random: () => number
): void {
  const image = ctx.getImageData(0, 0, width, height);
  const pixels = width * height;
  const data = image.data; // Uint8ClampedArray: writes round and clamp to 0–255
  for (let n = Math.floor(pixels * NOISE_SHARE); n > 0; n -= 1) {
    const offset = Math.floor(random() * pixels) * 4;
    const delta = (random() * 2 - 1) * NOISE_AMPLITUDE;
    data[offset] += delta;
    data[offset + 1] += delta;
    data[offset + 2] += delta;
  }
  ctx.putImageData(image, 0, 0);
}

export function paintGlow(
  canvas: HTMLCanvasElement,
  viewport: Viewport,
  palette: BackgroundPalette
): void {
  const ctx = prepare(canvas, viewport);
  if (!ctx) {
    return;
  }
  const { width, height } = viewport;
  ctx.clearRect(0, 0, width, height);
  const radius = Math.min(width, height) * GLOW_RADIUS;
  for (const [fx, fy] of GLOW_CENTRES) {
    const [cx, cy] = [width * fx, height * fy];
    const gradient = ctx.createRadialGradient(cx, cy, 0, cx, cy, radius);
    for (const [at, share] of GLOW_FALLOFF) {
      gradient.addColorStop(at, `rgba(${palette.glow}, ${palette.glowOpacity * share})`);
    }
    ctx.fillStyle = gradient;
    ctx.fillRect(cx - radius, cy - radius, radius * 2, radius * 2);
  }
}
