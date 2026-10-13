/**
 * Mock-only approximation of the Rust argument builder (`src-tauri/src/kernel/args.rs`), so
 * the editor behaves without a backend. The real contract is proven by the Rust tests; this
 * copy follows the same version gates (design §3.2) and argument order (design §3.3) but is
 * not authoritative. Never imported outside the mock.
 */
import type {
  ArgSupport,
  FieldWarning,
  GatedField,
  HostOs,
  MajorRange,
  ProfileDraft,
  SpoofingFeature,
} from '@/lib/bindings';

import { mockError } from './errors';

/** The newest argument set: what a profile gets when no kernel is configured. */
export const NEWEST_MAJOR = Number.MAX_SAFE_INTEGER;

const BRAND: MajorRange = { min: 131, max: null };
const GPU_VENDOR_RENDERER: MajorRange = { min: 139, max: 143 };
const DISABLE_GPU_FINGERPRINT: MajorRange = { min: 142, max: 143 };
const DISABLE_SPOOFING: MajorRange = { min: 144, max: null };

const SPOOFING_ORDER: SpoofingFeature[] = ['font', 'audio', 'canvas', 'clientrects', 'gpu'];

function supports(range: MajorRange, major: number): boolean {
  return major >= range.min && (range.max === null || major <= range.max);
}

export function argSupportFor(major: number): ArgSupport {
  return {
    brand: supports(BRAND, major),
    gpuVendorRenderer: supports(GPU_VENDOR_RENDERER, major),
    disableGpuFingerprint: supports(DISABLE_GPU_FINGERPRINT, major),
    disableSpoofing: supports(DISABLE_SPOOFING, major),
  };
}

function nonBlank(value: string | null): string | null {
  const trimmed = value?.trim() ?? '';
  return trimmed === '' ? null : trimmed;
}

export function spoofingCsv(features: SpoofingFeature[]): string | null {
  const set = new Set(features);
  const ordered = SPOOFING_ORDER.filter((feature) => set.has(feature));
  return ordered.length > 0 ? ordered.join(',') : null;
}

export function fieldWarnings(draft: ProfileDraft, major: number): FieldWarning[] {
  const gated: [GatedField, MajorRange, boolean][] = [
    ['fpBrand', BRAND, nonBlank(draft.fpBrand) !== null],
    ['fpBrandVersion', BRAND, nonBlank(draft.fpBrandVersion) !== null],
    ['fpGpuVendor', GPU_VENDOR_RENDERER, nonBlank(draft.fpGpuVendor) !== null],
    ['fpGpuRenderer', GPU_VENDOR_RENDERER, nonBlank(draft.fpGpuRenderer) !== null],
    ['disableGpuFingerprint', DISABLE_GPU_FINGERPRINT, draft.disableGpuFingerprint],
    ['disableSpoofing', DISABLE_SPOOFING, draft.disableSpoofing.length > 0],
  ];
  return gated
    .filter(([, range, isSet]) => isSet && !supports(range, major))
    .map(([field, range]) => ({
      field,
      supported: range,
      reason: major < range.min ? 'requires_newer_kernel' : 'removed_in_kernel',
    }));
}

const TYPOGRAPHIC_QUOTES = new Set(['“', '”', '‘', '’', '＂', '＇']);

/** The splitting rules of the Rust `split_extra_args` (design §3.3 step 9). */
export function splitExtraArgs(text: string): string[] {
  if (text.includes('\0')) {
    throw mockError('invalid_input', 'extra arguments must not contain NUL characters');
  }
  const words: string[] = [];
  let word = '';
  let inWord = false;
  let quote: string | null = null;
  for (const c of text) {
    if (quote !== null) {
      if (c === quote) {
        quote = null;
      } else {
        word += c;
      }
    } else if (/\s/u.test(c)) {
      if (inWord) {
        words.push(word);
        word = '';
        inWord = false;
      }
    } else if (c === '"' || c === "'") {
      quote = c;
      inWord = true;
    } else if (TYPOGRAPHIC_QUOTES.has(c)) {
      throw mockError(
        'invalid_input',
        `extra arguments: ${c} is not a quote here; use straight quotes " or '`
      );
    } else {
      word += c;
      inWord = true;
    }
  }
  if (quote !== null) {
    throw mockError('invalid_input', `extra arguments: missing closing ${quote}`);
  }
  if (inWord) {
    words.push(word);
  }
  rejectUserDataDir(words, 'extra arguments');
  return words;
}

function setsUserDataDir(word: string): boolean {
  const name = (word.split('=', 1)[0] ?? '').toLowerCase();
  return ['--', '-', '/'].some((prefix) => name === `${prefix}user-data-dir`);
}

export function rejectUserDataDir(words: string[], field: string): void {
  const word = words.find(setsUserDataDir);
  if (word !== undefined) {
    throw mockError(
      'invalid_input',
      `${field} must not set the user data directory (${word}); use the profile's user data directory field`
    );
  }
}

/** Trimmed, non-empty start URLs. */
export function cleanStartUrls(urls: string[]): string[] {
  return urls.map((url) => url.trim()).filter((url) => url !== '');
}

export interface PlanInput {
  draft: ProfileDraft;
  program: string;
  major: number;
  proxyUrl: string | null;
  userDataDir: string;
  host: HostOs;
}

/** argv in the order of design §3.3, plus a display command line for `host`. */
export function buildPlan({ draft, program, major, proxyUrl, userDataDir, host }: PlanInput) {
  const support = argSupportFor(major);
  const args = [`--user-data-dir=${userDataDir}`];
  const value = (name: string, v: string | number | null) => {
    if (v !== null) {
      args.push(`--${name}=${v}`);
    }
  };
  const flag = (name: string, on: boolean) => {
    if (on) {
      args.push(`--${name}`);
    }
  };

  if (draft.fingerprintSeed !== null) {
    value('fingerprint', draft.fingerprintSeed);
    value('fingerprint-platform', draft.fpPlatform);
    value('fingerprint-platform-version', nonBlank(draft.fpPlatformVersion));
    if (support.brand) {
      value('fingerprint-brand', nonBlank(draft.fpBrand));
      value('fingerprint-brand-version', nonBlank(draft.fpBrandVersion));
    }
    value('fingerprint-hardware-concurrency', draft.fpHardwareConcurrency);
    if (support.gpuVendorRenderer) {
      value('fingerprint-gpu-vendor', nonBlank(draft.fpGpuVendor));
      value('fingerprint-gpu-renderer', nonBlank(draft.fpGpuRenderer));
    }
    flag('disable-gpu-fingerprint', support.disableGpuFingerprint && draft.disableGpuFingerprint);
    if (support.disableSpoofing) {
      value('disable-spoofing', spoofingCsv(draft.disableSpoofing));
    }
  }
  flag('disable-non-proxied-udp', draft.disableNonProxiedUdp);
  value('lang', nonBlank(draft.lang));
  value('accept-lang', nonBlank(draft.acceptLang));
  value('timezone', nonBlank(draft.timezone));
  value('proxy-server', proxyUrl);
  flag('headless', draft.headless);
  const extra = nonBlank(draft.extraArgs);
  if (extra !== null) {
    args.push(...splitExtraArgs(extra));
  }
  const urls = cleanStartUrls(draft.startUrls);
  rejectUserDataDir(urls, 'start URLs');
  args.push(...urls);

  const quote = host === 'windows' ? windowsQuote : posixQuote;
  return {
    program,
    args,
    shellLine: [program, ...args].map(quote).join(' '),
  };
}

function posixQuote(word: string): string {
  if (/^[A-Za-z0-9_@%+=:,./-]+$/.test(word) && !word.startsWith('=')) {
    return word;
  }
  return `'${word.replaceAll("'", "'\\''")}'`;
}

function windowsQuote(word: string): string {
  if (word !== '' && !/[\s"]/.test(word)) {
    return word;
  }
  return `"${word.replaceAll('"', '\\"')}"`;
}
