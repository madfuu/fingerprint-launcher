/**
 * In-memory mock of the Rust backend, so the frontend runs in a plain browser (`npm run dev`
 * outside Tauri) for UI work and screenshots (design §6 / §10).
 *
 * It mirrors the Rust semantics closely enough for the UI: the `ProfileQuery` paging and
 * filters, the version gates of `getArgSupport` / `validateProfileForKernel`, the command line
 * order of `previewCommand`, and the launcher's status events (`starting` → `running`,
 * `stopping` → `stopped`, startup failures with a hint). Rejections are plain
 * `{ code, message }` objects, as Tauri delivers them, so the bridge's normalization runs the
 * same way as in the app.
 *
 * Loaded only by `bridge.ts`, through a dynamic import behind `import.meta.env.DEV`:
 * production bundles contain none of this.
 *
 * URL overrides (screenshots): `?theme=light|dark|system`, `?lang=zh-CN|en`,
 * `?os=linux|windows|macos`, `?sidebar=collapsed|expanded`.
 *
 * Window controls do nothing to the browser window; they are recorded in
 * `MockBackend.windowCalls`, and closing with browsers running emits `app://close-requested`
 * like the Rust shell does.
 */
import type {
  AppError,
  AppInfo,
  BatchProgress,
  EventPayloads,
  Group,
  GroupInput,
  HostOs,
  Kernel,
  KernelCandidate,
  KernelInput,
  KernelProbe,
  LaunchFailure,
  Locale,
  Profile,
  ProfileDraft,
  ProfileRow,
  ProfileStatus,
  Proxy,
  ProxyCheck,
  ProxyInput,
  RunningInfo,
  Settings,
  SettingsPatch,
  SpoofingFeature,
  Tag,
  TagInput,
  Theme,
} from '@/lib/bindings';
import type {
  CommandArgs,
  CommandName,
  CommandResult,
  EventName,
  OpenFileOptions,
  SaveFileOptions,
  Transport,
  Unlisten,
  WindowControls,
} from '@/lib/bridge';
import {
  argSupportFor,
  buildPlan,
  cleanStartUrls,
  fieldWarnings,
  NEWEST_MAJOR,
  rejectUserDataDir,
  splitExtraArgs,
} from '@/lib/mock/command-line';
import { mockError } from '@/lib/mock/errors';
import {
  createRng,
  createSeed,
  EMPTY_DRAFT,
  randomSeed,
  SEED_MAX,
  uuidFrom,
  type MockFailure,
} from '@/lib/mock/seed';

/** Delays in ms. Shorter than the real launcher's, long enough to see every state. */
export interface MockTiming {
  /** Before every command resolves. */
  latency: number;
  /** Reservation → process exists (`starting`). */
  spawn: number;
  /** `starting` → `running` (real: 3 s). */
  startup: number;
  /** `starting` → `error` for the profiles seeded to fail. */
  failure: number;
  /** `stopping` → `stopped`. */
  stop: number;
  /** `testProxy`. */
  proxyCheck: number;
}

export const DEFAULT_TIMING: MockTiming = {
  latency: 40,
  spawn: 150,
  startup: 1500,
  failure: 1200,
  stop: 700,
  proxyCheck: 600,
};

export interface MockOptions {
  /** `location.search`, for the `theme` / `lang` / `os` / `sidebar` overrides. */
  search?: string;
  timing?: Partial<MockTiming>;
}

/** What `window.__FPL_MOCK__` offers to the console and screenshot scripts. */
export interface MockControls {
  emit<E extends EventName>(event: E, payload: EventPayloads[E]): void;
  backend: MockBackend;
}

const MAX_PAGE_SIZE = 500;
const STDERR_IN_MESSAGE = 5;
const SPOOFING_FEATURES: SpoofingFeature[] = ['font', 'audio', 'canvas', 'clientrects', 'gpu'];

const THEMES: Theme[] = ['light', 'dark', 'system'];
const LOCALES: Locale[] = ['zh-CN', 'en'];
const HOST_OSES: HostOs[] = ['linux', 'windows', 'macos'];
const SIDEBAR_STATES = ['collapsed', 'expanded'] as const;

/** A window control the UI used, in `MockBackend.windowCalls`. */
export type WindowCall = 'minimize' | 'toggleMaximize' | 'close';

/** What the frontend reported through `showMainWindow`. */
export interface BootReport {
  bootOk: boolean;
  failure: string | null;
}

function pick<T extends string>(value: string | null, allowed: readonly T[]): T | undefined {
  return allowed.find((candidate) => candidate === value);
}

function sleep(ms: number): Promise<void> {
  return ms > 0 ? new Promise((resolve) => setTimeout(resolve, ms)) : Promise.resolve();
}

function now(): string {
  return new Date().toISOString();
}

function clone<T>(value: T): T {
  return structuredClone(value);
}

function isAbsolute(path: string): boolean {
  return path.startsWith('/') || /^[A-Za-z]:[\\/]/.test(path) || path.startsWith('\\\\');
}

function requireAbsolute(path: string, field: string): void {
  if (!isAbsolute(path)) {
    throw mockError('invalid_input', `${field} must be an absolute path: ${path}`);
  }
}

function requiredText(value: string, field: string): string {
  const trimmed = value.trim();
  if (trimmed === '') {
    throw mockError('invalid_input', `${field} must not be empty`);
  }
  return trimmed;
}

function cleanOpt(value: string | null): string | null {
  const trimmed = value?.trim() ?? '';
  return trimmed === '' ? null : trimmed;
}

/** Lowercases ASCII letters only, like SQLite's `LIKE` and `NOCASE`. */
function asciiLower(text: string): string {
  return text.replace(/[A-Z]/g, (c) => c.toLowerCase());
}

/** SQLite `ORDER BY name` on a `COLLATE NOCASE` column. */
function compareNoCase(a: string, b: string): number {
  const [x, y] = [asciiLower(a), asciiLower(b)];
  return x < y ? -1 : x > y ? 1 : 0;
}

const DRAFT_KEYS = Object.keys(EMPTY_DRAFT) as (keyof ProfileDraft)[];

/**
 * The `ProfileDraft` fields of `input`, as serde reads it (`#[serde(default)]`, unknown fields
 * ignored): a missing field takes its default, extra fields (`id`, `deletedAt`, a row's `group`
 * …, when the UI passes a whole `Profile`) are dropped.
 */
function pickDraft(input: ProfileDraft): ProfileDraft {
  const source = input as Partial<Record<keyof ProfileDraft, unknown>>;
  return Object.fromEntries(
    DRAFT_KEYS.map((key) => [
      key,
      clone(source[key] === undefined ? EMPTY_DRAFT[key] : source[key]),
    ])
  ) as ProfileDraft;
}

function validateColor(color: string): void {
  if (!/^#([0-9a-f]{3}|[0-9a-f]{6})$/i.test(color)) {
    throw mockError('invalid_input', `color must be #rgb or #rrggbb: ${color}`);
  }
}

/** `a` and `b` are the same directory or one contains the other. */
function dirsOverlap(a: string, b: string, sep: string): boolean {
  const norm = (dir: string) => dir.replace(/[\\/]+$/, '');
  const [x, y] = [norm(a), norm(b)];
  return x === y || x.startsWith(y + sep) || y.startsWith(x + sep);
}

/** Counting semaphore whose release functions are idempotent. */
class Semaphore {
  private available: number;
  private waiters: (() => void)[] = [];

  constructor(permits: number) {
    this.available = permits;
  }

  async acquire(): Promise<() => void> {
    if (this.available > 0) {
      this.available -= 1;
    } else {
      await new Promise<void>((resolve) => this.waiters.push(resolve));
    }
    let released = false;
    return () => {
      if (released) {
        return;
      }
      released = true;
      const next = this.waiters.shift();
      if (next) {
        next();
      } else {
        this.available += 1;
      }
    };
  }
}

interface RunEntry {
  /** `launching`: reserved, no process yet (not listed by `listRunning`). */
  phase: 'launching' | 'process';
  status: ProfileStatus;
  pid: number;
  startedAt: string;
  kernelId: string;
  userDataDir: string;
  timer: ReturnType<typeof setTimeout> | null;
  stopRequested: boolean;
  /** Batch launch permit, released when the profile leaves `starting`. */
  release: (() => void) | null;
  done: Promise<void>;
  resolveDone: () => void;
}

/** Same naming as the Rust `copy_name`. */
function copyName(name: string, locale: Locale): string {
  return locale === 'zh-CN' ? `${name} 副本` : `${name} (copy)`;
}

function appInfoFor(os: HostOs): AppInfo {
  const identifier = 'io.github.madfuu.fingerprint-launcher';
  const dataDir = {
    linux: `/home/user/.local/share/${identifier}`,
    macos: `/Users/user/Library/Application Support/${identifier}`,
    windows: `C:\\Users\\user\\AppData\\Roaming\\${identifier}`,
  }[os];
  const sep = os === 'windows' ? '\\' : '/';
  return {
    version: '0.1.0',
    os,
    dataDir,
    defaultUserDataRoot: `${dataDir}${sep}profiles`,
    logDir: `${dataDir}${sep}logs${sep}profiles`,
  };
}

export class MockBackend {
  readonly timing: MockTiming;
  settings: Settings;
  readonly appInfo: AppInfo;
  kernels: Kernel[];
  groups: Group[];
  tags: Tag[];
  proxies: Proxy[];
  readonly profiles = new Map<string, Profile>();
  /** Last value written by `copyToClipboard`. */
  clipboard: string | null = null;
  /** Every window control used, in order. */
  readonly windowCalls: WindowCall[] = [];
  /** Last `showMainWindow` call; `null` before the frontend called it. */
  bootReport: BootReport | null = null;
  private maximized = false;
  private readonly resizeListeners = new Set<() => void>();

  private readonly failures: Record<string, MockFailure>;
  private readonly proxyChecks: Record<string, ProxyCheck | null>;
  private readonly table = new Map<string, RunEntry>();
  private readonly listeners = new Map<string, Set<(payload: unknown) => void>>();
  private readonly rng = createRng(7_150_144);
  private closed = false;
  private nextPid = 51_200;

  constructor(options: MockOptions = {}) {
    this.timing = { ...DEFAULT_TIMING, ...options.timing };
    const params = new URLSearchParams(options.search ?? '');
    this.settings = {
      userDataRoot: null,
      locale: pick(params.get('lang'), LOCALES) ?? 'zh-CN',
      theme: pick(params.get('theme'), THEMES) ?? 'system',
      batchConcurrency: 3,
      batchIntervalMs: 800,
      sidebarCollapsed: pick(params.get('sidebar'), SIDEBAR_STATES) !== 'expanded',
    };
    this.appInfo = appInfoFor(pick(params.get('os'), HOST_OSES) ?? 'linux');

    const seed = createSeed();
    this.kernels = seed.kernels;
    this.groups = seed.groups;
    this.tags = seed.tags;
    this.proxies = seed.proxies;
    for (const profile of seed.profiles) {
      this.profiles.set(profile.id, profile);
    }
    this.failures = seed.failures;
    this.proxyChecks = seed.proxyChecks;
    for (const { profileId, pid, minutesAgo } of seed.running) {
      const profile = this.profiles.get(profileId);
      if (!profile) {
        continue;
      }
      const entry = this.newEntry();
      Object.assign(entry, {
        phase: 'process',
        status: 'running',
        pid,
        startedAt: new Date(Date.now() - minutesAgo * 60_000).toISOString(),
        kernelId: this.kernelFor(profile.kernelId)?.id ?? '',
        userDataDir: this.userDataDirOf(profile),
      } satisfies Partial<RunEntry>);
      // The launch that is still running is the last one.
      profile.lastLaunchedAt = entry.startedAt;
      this.table.set(profileId, entry);
    }
  }

  // ------------------------------------------------------------------------------------------
  // Transport plumbing
  // ------------------------------------------------------------------------------------------

  async invoke<K extends CommandName>(command: K, args: CommandArgs<K>): Promise<CommandResult<K>> {
    await sleep(this.timing.latency);
    const handler = this.handlers[command] as (
      args: CommandArgs<K>
    ) => CommandResult<K> | Promise<CommandResult<K>>;
    if (!handler) {
      // What Tauri answers for an unregistered command: a plain string.
      throw `command ${command} not found`;
    }
    return clone(await handler(args));
  }

  listen<E extends EventName>(event: E, handler: (payload: EventPayloads[E]) => void): Unlisten {
    const set = this.listeners.get(event) ?? new Set();
    this.listeners.set(event, set);
    const listener = handler as (payload: unknown) => void;
    set.add(listener);
    return () => set.delete(listener);
  }

  /** Delivered asynchronously, like Tauri events. */
  emit<E extends EventName>(event: E, payload: EventPayloads[E]): void {
    const set = this.listeners.get(event);
    if (!set) {
      return;
    }
    const delivered = clone(payload);
    for (const listener of [...set]) {
      queueMicrotask(() => listener(delivered));
    }
  }

  private emitStatus(
    profileId: string,
    status: ProfileStatus,
    pid: number | null,
    error: LaunchFailure | null
  ): void {
    this.emit('profile://status-changed', { profileId, status, pid, error });
  }

  // ------------------------------------------------------------------------------------------
  // Lookups
  // ------------------------------------------------------------------------------------------

  private get sep(): string {
    return this.appInfo.os === 'windows' ? '\\' : '/';
  }

  private userDataRoot(): string {
    return this.settings.userDataRoot ?? this.appInfo.defaultUserDataRoot;
  }

  private dirFor(custom: string | null, id: string): string {
    return custom ?? `${this.userDataRoot()}${this.sep}${id}`;
  }

  private userDataDirOf(profile: Profile): string {
    return this.dirFor(profile.userDataDir, profile.id);
  }

  private profile(id: string): Profile {
    const profile = this.profiles.get(id);
    if (!profile) {
      throw mockError('not_found', `profile ${id} not found`);
    }
    return profile;
  }

  private kernel(id: string): Kernel {
    const kernel = this.kernels.find((k) => k.id === id);
    if (!kernel) {
      throw mockError('not_found', `kernel ${id} not found`);
    }
    return kernel;
  }

  /** The kernel with this id (`not_found` if gone), or the default kernel. */
  private kernelFor(id: string | null): Kernel | null {
    const kernelId = cleanOpt(id);
    return kernelId ? this.kernel(kernelId) : (this.kernels.find((k) => k.isDefault) ?? null);
  }

  private proxy(id: string): Proxy {
    const proxy = this.proxies.find((p) => p.id === id);
    if (!proxy) {
      throw mockError('not_found', `proxy ${id} not found`);
    }
    return proxy;
  }

  private proxyUrl(proxy: Proxy): string {
    const host = proxy.host.includes(':') ? `[${proxy.host}]` : proxy.host;
    return `${proxy.scheme}://${host}:${proxy.port}`;
  }

  private tagName(id: string): string {
    return this.tags.find((tag) => tag.id === id)?.name ?? '';
  }

  /** Tag ids ordered by tag name, as the Rust repository reads them back. */
  private withOrderedTags(profile: Profile): Profile {
    const tagIds = [...profile.tagIds].sort((a, b) =>
      compareNoCase(this.tagName(a), this.tagName(b))
    );
    return { ...profile, tagIds };
  }

  private toRow(profile: Profile): ProfileRow {
    const ordered = this.withOrderedTags(profile);
    return {
      ...ordered,
      group: this.groups.find((g) => g.id === profile.groupId) ?? null,
      proxy: this.proxies.find((p) => p.id === profile.proxyId) ?? null,
      tags: ordered.tagIds
        .map((id) => this.tags.find((tag) => tag.id === id))
        .filter((tag): tag is Tag => tag !== undefined),
    };
  }

  private newId(): string {
    return uuidFrom(this.rng);
  }

  // ------------------------------------------------------------------------------------------
  // Profiles
  // ------------------------------------------------------------------------------------------

  /** Mirrors `normalize_ids` plus the `profile_tags` foreign key. */
  private tagIds(ids: string[]): string[] {
    const normalized = [...new Set(ids.map((id) => id.trim()).filter((id) => id !== ''))].sort();
    if (normalized.some((id) => !this.tags.some((t) => t.id === id))) {
      throw mockError('invalid_input', 'FOREIGN KEY constraint failed (tag)');
    }
    return normalized;
  }

  /** Mirrors `ProfileDraft::normalized` plus the foreign keys SQLite would check. */
  private normalize(input: ProfileDraft): ProfileDraft {
    const draft = pickDraft(input);
    const d: ProfileDraft = { ...draft, name: requiredText(draft.name, 'profile name') };
    for (const key of [
      'note',
      'groupId',
      'kernelId',
      'proxyId',
      'fpPlatformVersion',
      'fpBrand',
      'fpBrandVersion',
      'fpGpuVendor',
      'fpGpuRenderer',
      'lang',
      'acceptLang',
      'timezone',
      'userDataDir',
      'extraArgs',
    ] as const) {
      d[key] = cleanOpt(d[key]);
    }
    if (d.fingerprintSeed !== null) {
      if (
        !Number.isInteger(d.fingerprintSeed) ||
        d.fingerprintSeed < 1 ||
        d.fingerprintSeed > SEED_MAX
      ) {
        throw mockError('invalid_input', `fingerprint seed must be 1-${SEED_MAX}`);
      }
    }
    if (d.fpHardwareConcurrency !== null && !(d.fpHardwareConcurrency >= 1)) {
      throw mockError('invalid_input', 'hardware concurrency must be positive');
    }
    if (d.userDataDir !== null) {
      requireAbsolute(d.userDataDir, 'user data directory');
    }
    if (d.extraArgs !== null) {
      splitExtraArgs(d.extraArgs);
    }
    d.disableSpoofing = SPOOFING_FEATURES.filter((f) => d.disableSpoofing.includes(f));
    d.startUrls = cleanStartUrls(d.startUrls);
    rejectUserDataDir(d.startUrls, 'start URLs');

    const dangling = (what: string) =>
      mockError('invalid_input', `FOREIGN KEY constraint failed (${what})`);
    if (d.groupId !== null && !this.groups.some((g) => g.id === d.groupId)) throw dangling('group');
    if (d.kernelId !== null && !this.kernels.some((k) => k.id === d.kernelId))
      throw dangling('kernel');
    if (d.proxyId !== null && !this.proxies.some((p) => p.id === d.proxyId))
      throw dangling('proxy');
    d.tagIds = this.tagIds(d.tagIds);
    return d;
  }

  /** Approximates the Rust check that a profile's directory is its own (design §5). */
  private checkDirIsFree(draft: ProfileDraft, id: string, selfId: string | null): void {
    const dir = this.dirFor(draft.userDataDir, id);
    const root = this.userDataRoot();
    if (
      draft.userDataDir !== null &&
      dirsOverlap(dir, root, this.sep) &&
      !dir.startsWith(root + this.sep)
    ) {
      throw mockError('conflict', `user data directory ${dir} contains the user data root ${root}`);
    }
    for (const other of this.profiles.values()) {
      if (other.id !== selfId && dirsOverlap(dir, this.userDataDirOf(other), this.sep)) {
        throw mockError(
          'conflict',
          `user data directory ${dir} overlaps the one of profile "${other.name}"`
        );
      }
    }
  }

  private createProfile(draft: ProfileDraft): Profile {
    const normalized = this.normalize(draft);
    const id = this.newId();
    this.checkDirIsFree(normalized, id, null);
    const at = now();
    const profile: Profile = {
      ...normalized,
      id,
      lastLaunchedAt: null,
      deletedAt: null,
      createdAt: at,
      updatedAt: at,
    };
    this.profiles.set(id, profile);
    return this.withOrderedTags(profile);
  }

  private ensureNotRunning(id: string): void {
    if (this.table.has(id)) {
      throw mockError('profile_running', `profile ${id} is running; stop it first`);
    }
  }

  private listProfiles({ query }: CommandArgs<'list_profiles'>) {
    if (query.page < 1) {
      throw mockError('invalid_input', 'page is 1-based');
    }
    if (query.pageSize < 1 || query.pageSize > MAX_PAGE_SIZE) {
      throw mockError('invalid_input', `page size must be 1-${MAX_PAGE_SIZE}`);
    }
    const groupId = cleanOpt(query.groupId);
    const tagIds = new Set(query.tagIds.map((id) => id.trim()).filter((id) => id !== ''));
    // `LIKE` with every wildcard escaped: a literal, ASCII-case-insensitive substring.
    const keyword = asciiLower(cleanOpt(query.keyword) ?? '') || null;
    // Map order is insertion order, which stands in for SQLite's rowid.
    const matches = [...this.profiles.values()].filter((p) => {
      const inView =
        query.view === 'trash'
          ? p.deletedAt !== null
          : p.deletedAt === null && (query.view === 'all' || this.table.has(p.id));
      return (
        inView &&
        (groupId === null || p.groupId === groupId) &&
        (tagIds.size === 0 || p.tagIds.some((id) => tagIds.has(id))) &&
        (keyword === null ||
          [p.name, p.note ?? '', p.fingerprintSeed?.toString() ?? ''].some((text) =>
            asciiLower(text).includes(keyword)
          ))
      );
    });
    // `ORDER BY created_at DESC, rowid DESC` (trash: `deleted_at DESC, rowid DESC`).
    const key = (p: Profile) => (query.view === 'trash' ? (p.deletedAt ?? '') : p.createdAt);
    matches.reverse();
    matches.sort((a, b) => (key(a) < key(b) ? 1 : key(a) > key(b) ? -1 : 0));
    const start = (query.page - 1) * query.pageSize;
    return {
      items: matches.slice(start, start + query.pageSize).map((p) => this.toRow(p)),
      total: matches.length,
      page: query.page,
      pageSize: query.pageSize,
    };
  }

  // ------------------------------------------------------------------------------------------
  // Launcher
  // ------------------------------------------------------------------------------------------

  private newEntry(): RunEntry {
    let resolveDone = () => {};
    const done = new Promise<void>((resolve) => {
      resolveDone = resolve;
    });
    return {
      phase: 'launching',
      status: 'starting',
      pid: 0,
      startedAt: '',
      kernelId: '',
      userDataDir: '',
      timer: null,
      stopRequested: false,
      release: null,
      done,
      resolveDone,
    };
  }

  private leaveStarting(entry: RunEntry): void {
    entry.release?.();
    entry.release = null;
  }

  /** Mirrors `Launcher::launch`: resolves once the process exists. */
  async launch(id: string, release: (() => void) | null = null): Promise<null> {
    if (this.closed) {
      throw mockError('shutting_down', 'the launcher is shutting down');
    }
    if (this.table.has(id)) {
      throw mockError('profile_running', `profile ${id} is already running`);
    }
    const entry = this.newEntry();
    this.table.set(id, entry);

    let profile: Profile;
    try {
      profile = this.profile(id);
      if (profile.deletedAt !== null) {
        throw mockError('invalid_input', `profile ${id} is in the trash`);
      }
      const kernel = this.kernelFor(profile.kernelId);
      if (!kernel) {
        throw mockError('no_kernel', 'no kernel is configured');
      }
      const dir = this.userDataDirOf(profile);
      for (const [otherId, other] of this.table) {
        if (otherId !== id && other.phase === 'process' && other.userDataDir === dir) {
          throw mockError(
            'conflict',
            `user data directory ${dir} is in use by running profile ${otherId}`
          );
        }
      }
      entry.kernelId = kernel.id;
      entry.userDataDir = dir;
    } catch (error) {
      this.table.delete(id);
      const err = error as AppError;
      this.emitStatus(id, 'error', null, {
        code: err.code,
        message: err.message,
        hint: null,
        exitCode: null,
        stderrTail: [],
      });
      throw error;
    }

    await sleep(this.timing.spawn);
    entry.phase = 'process';
    entry.pid = this.nextPid;
    this.nextPid += 1 + Math.floor(this.rng() * 40);
    entry.startedAt = now();
    entry.release = release;
    this.emitStatus(id, 'starting', entry.pid, null);
    profile.lastLaunchedAt = entry.startedAt;

    const failure = this.failures[id];
    if (failure) {
      entry.timer = setTimeout(() => {
        const quoted = failure.stderrTail.slice(-STDERR_IN_MESSAGE).join('\n');
        this.finish(id, 'error', {
          code: 'startup_failed',
          message: `the browser exited during startup (exit status: ${failure.exitCode}):\n${quoted}`,
          hint: failure.hint,
          exitCode: failure.exitCode,
          stderrTail: failure.stderrTail,
        });
      }, this.timing.failure);
    } else {
      entry.timer = setTimeout(() => {
        entry.status = 'running';
        this.emitStatus(id, 'running', entry.pid, null);
        this.leaveStarting(entry);
      }, this.timing.startup);
    }
    if (entry.stopRequested) {
      this.requestStop(id, entry);
    }
    return null;
  }

  private requestStop(id: string, entry: RunEntry): void {
    if (entry.phase === 'launching') {
      entry.stopRequested = true;
      return;
    }
    if (entry.status === 'stopping') {
      return;
    }
    if (entry.timer !== null) {
      clearTimeout(entry.timer);
    }
    entry.status = 'stopping';
    this.emitStatus(id, 'stopping', entry.pid, null);
    this.leaveStarting(entry);
    entry.timer = setTimeout(() => this.finish(id, 'stopped', null), this.timing.stop);
  }

  private finish(id: string, status: ProfileStatus, error: LaunchFailure | null): void {
    const entry = this.table.get(id);
    if (!entry) {
      return;
    }
    if (entry.timer !== null) {
      clearTimeout(entry.timer);
    }
    this.table.delete(id);
    this.emitStatus(id, status, null, error);
    this.leaveStarting(entry);
    entry.resolveDone();
  }

  /** Mirrors `Launcher::stop`: waits unless the profile is still being spawned. */
  async stop(id: string): Promise<null> {
    const entry = this.table.get(id);
    if (!entry) {
      return null;
    }
    this.requestStop(id, entry);
    if (entry.phase === 'process') {
      await entry.done;
    }
    return null;
  }

  async stopMany(ids: string[]): Promise<null> {
    await Promise.all(ids.map((id) => this.stop(id)));
    return null;
  }

  /** Mirrors `Launcher::batch_launch` (design §4.6). */
  async batchLaunch(ids: string[]): Promise<BatchProgress> {
    const slots = new Semaphore(Math.max(1, this.settings.batchConcurrency));
    const progress: BatchProgress = { total: ids.length, launched: 0, failed: 0, skipped: 0 };
    for (const [index, id] of ids.entries()) {
      const release = await slots.acquire();
      await sleep(this.settings.batchIntervalMs);
      try {
        await this.launch(id, release);
        progress.launched += 1;
      } catch (error) {
        release();
        const code = (error as AppError).code;
        if (code === 'profile_running') {
          progress.skipped += 1;
        } else if (code === 'shutting_down') {
          progress.skipped += ids.length - index;
          this.emit('batch://progress', { ...progress });
          break;
        } else {
          progress.failed += 1;
        }
      }
      this.emit('batch://progress', { ...progress });
    }
    return progress;
  }

  listRunning(): RunningInfo[] {
    return [...this.table.entries()]
      .filter(([, entry]) => entry.phase === 'process')
      .map(([profileId, entry]) => ({
        profileId,
        status: entry.status,
        pid: entry.pid,
        startedAt: entry.startedAt,
        kernelId: entry.kernelId,
        userDataDir: entry.userDataDir,
      }))
      .sort((a, b) =>
        a.startedAt === b.startedAt
          ? a.profileId.localeCompare(b.profileId)
          : a.startedAt.localeCompare(b.startedAt)
      );
  }

  /** Mirrors `Launcher::shutdown`: refuses new launches, stops everything. */
  async shutdown(): Promise<null> {
    this.closed = true;
    while (this.table.size > 0) {
      await this.stopMany([...this.table.keys()]);
      if (this.table.size > 0) {
        await sleep(20);
      }
    }
    return null;
  }

  // ------------------------------------------------------------------------------------------
  // Kernels
  // ------------------------------------------------------------------------------------------

  probe(path: string): KernelProbe {
    const match = /(\d{2,3})\.(\d+)\.(\d+)\.(\d+)/.exec(path);
    if (!match?.[1]) {
      return { majorVersion: null, fullVersion: null, method: null };
    }
    return {
      majorVersion: Number(match[1]),
      fullVersion: match[0],
      method: path.endsWith('.AppImage') ? 'file_name' : 'version_flag',
    };
  }

  private addKernel({ path, name, majorVersion }: CommandArgs<'add_kernel'>): Kernel {
    const exec = path.trim();
    requireAbsolute(exec, 'kernel path');
    if (/missing/i.test(exec)) {
      throw mockError('kernel_missing', `kernel not found: ${exec}`);
    }
    const probe = this.probe(exec);
    const major = majorVersion ?? probe.majorVersion;
    if (major === null) {
      throw mockError(
        'kernel_version_unknown',
        `cannot detect the version of ${exec}; enter its major version`
      );
    }
    if (this.kernels.some((k) => k.execPath === exec)) {
      throw mockError('conflict', 'UNIQUE constraint failed: kernels.exec_path');
    }
    const fullVersion = probe.majorVersion === major ? probe.fullVersion : null;
    const kernel: Kernel = {
      id: this.newId(),
      name: cleanOpt(name) ?? `Chromium ${fullVersion ?? major}`,
      execPath: exec,
      majorVersion: major,
      fullVersion,
      isDefault: !this.kernels.some((k) => k.isDefault),
      createdAt: now(),
    };
    this.kernels.push(kernel);
    return kernel;
  }

  private updateKernel({ id, input }: { id: string; input: KernelInput }): Kernel {
    const kernel = this.kernel(id);
    const execPath = input.execPath.trim();
    requireAbsolute(execPath, 'kernel path');
    if (this.kernels.some((k) => k.id !== id && k.execPath === execPath)) {
      throw mockError('conflict', 'UNIQUE constraint failed: kernels.exec_path');
    }
    if (!(input.majorVersion >= 1)) {
      throw mockError('invalid_input', 'major version must be positive');
    }
    Object.assign(kernel, {
      name: requiredText(input.name, 'kernel name'),
      execPath,
      majorVersion: input.majorVersion,
      fullVersion: cleanOpt(input.fullVersion),
    });
    return kernel;
  }

  private removeKernel(id: string): null {
    const kernel = this.kernel(id);
    this.kernels = this.kernels.filter((k) => k.id !== id);
    if (kernel.isDefault && this.kernels[0]) {
      this.kernels[0].isDefault = true;
    }
    for (const profile of this.profiles.values()) {
      if (profile.kernelId === id) {
        profile.kernelId = null;
      }
    }
    return null;
  }

  private detectKernels(): KernelCandidate[] {
    const paths = [
      ...this.kernels.map((k) => k.execPath),
      '/home/user/Applications/ungoogled-chromium-148.0.7778.215-1-x86_64.AppImage',
      '/opt/ungoogled-chromium/chrome',
    ];
    return paths.map((path) => ({
      path,
      ...this.probe(path),
      registered: this.kernels.some((k) => k.execPath === path),
    }));
  }

  // ------------------------------------------------------------------------------------------
  // Proxies, groups, tags
  // ------------------------------------------------------------------------------------------

  private proxyInput(input: ProxyInput): ProxyInput {
    const host = requiredText(input.host, 'proxy host');
    if (host.includes('@')) {
      throw mockError(
        'invalid_input',
        'fingerprint-chromium does not support proxy authentication; remove the credentials'
      );
    }
    if (/[/\s]/.test(host) || (host.includes(':') && !/^[0-9a-f:.]+$/i.test(host))) {
      throw mockError(
        'invalid_input',
        `proxy host must be a bare host name or IP address: ${host}`
      );
    }
    if (!Number.isInteger(input.port) || input.port < 1 || input.port > 65_535) {
      throw mockError('invalid_input', 'proxy port must be 1-65535');
    }
    return { ...input, host, name: cleanOpt(input.name), note: cleanOpt(input.note) };
  }

  private async testProxy(id: string): Promise<ProxyCheck> {
    const proxy = this.proxy(id);
    await sleep(this.timing.proxyCheck);
    const check = Object.hasOwn(this.proxyChecks, id)
      ? this.proxyChecks[id]
      : {
          ip: '203.0.113.42',
          country: 'US',
          city: 'San Jose',
          timezone: 'America/Los_Angeles',
          service: 'ipinfo' as const,
          latencyMs: 210,
        };
    if (!check) {
      throw mockError(
        'proxy_test_failed',
        `ipinfo: connection through ${this.proxyUrl(proxy)} timed out after 10s; ip-api: connection through ${this.proxyUrl(proxy)} timed out after 10s`
      );
    }
    return check;
  }

  private groupInput(input: GroupInput, selfId: string | null): GroupInput {
    const name = requiredText(input.name, 'group name');
    const color = cleanOpt(input.color);
    if (color !== null) {
      validateColor(color);
    }
    if (this.groups.some((g) => g.id !== selfId && g.name.toLowerCase() === name.toLowerCase())) {
      throw mockError('conflict', 'UNIQUE constraint failed: groups.name');
    }
    return { name, color, sortOrder: input.sortOrder };
  }

  private tagInput(input: TagInput, selfId: string | null): TagInput {
    const name = requiredText(input.name, 'tag name');
    const color = input.color.trim();
    validateColor(color);
    if (this.tags.some((t) => t.id !== selfId && t.name.toLowerCase() === name.toLowerCase())) {
      throw mockError('conflict', 'UNIQUE constraint failed: tags.name');
    }
    return { name, color };
  }

  private find<T extends { id: string }>(list: T[], id: string, what: string): T {
    const item = list.find((entry) => entry.id === id);
    if (!item) {
      throw mockError('not_found', `${what} ${id} not found`);
    }
    return item;
  }

  // ------------------------------------------------------------------------------------------
  // Settings
  // ------------------------------------------------------------------------------------------

  private updateSettings(patch: SettingsPatch): Settings {
    const next = { ...this.settings };
    if (Object.hasOwn(patch, 'userDataRoot')) {
      const root = cleanOpt(patch.userDataRoot ?? null);
      if (root !== null) {
        requireAbsolute(root, 'user data root');
      }
      next.userDataRoot = root;
    }
    if (patch.batchConcurrency !== undefined) {
      if (patch.batchConcurrency < 1 || patch.batchConcurrency > 32) {
        throw mockError('invalid_input', 'batch concurrency must be 1-32');
      }
      next.batchConcurrency = patch.batchConcurrency;
    }
    if (patch.batchIntervalMs !== undefined) {
      if (patch.batchIntervalMs < 0 || patch.batchIntervalMs > 60_000) {
        throw mockError('invalid_input', 'batch interval must be 0-60000 ms');
      }
      next.batchIntervalMs = patch.batchIntervalMs;
    }
    next.locale = patch.locale ?? next.locale;
    next.theme = patch.theme ?? next.theme;
    next.sidebarCollapsed = patch.sidebarCollapsed ?? next.sidebarCollapsed;
    this.settings = next;
    return next;
  }

  // ------------------------------------------------------------------------------------------
  // Command table
  // ------------------------------------------------------------------------------------------

  private readonly handlers: {
    [K in CommandName]: (args: CommandArgs<K>) => CommandResult<K> | Promise<CommandResult<K>>;
  } = {
    list_profiles: (args) => this.listProfiles(args),
    get_profile: ({ id }) => this.withOrderedTags(this.profile(id)),
    create_profile: ({ draft }) => this.createProfile(draft),
    update_profile: ({ id, draft }) => {
      const profile = this.profile(id);
      const normalized = this.normalize(draft);
      this.checkDirIsFree(normalized, id, id);
      Object.assign(profile, normalized, { updatedAt: now() });
      return this.withOrderedTags(profile);
    },
    delete_profile: ({ id }) => {
      this.ensureNotRunning(id);
      const profile = this.profile(id);
      profile.deletedAt ??= now();
      return null;
    },
    restore_profile: ({ id }) => {
      this.profile(id).deletedAt = null;
      return null;
    },
    purge_profile: ({ id, dropUserData }) => {
      this.ensureNotRunning(id);
      const profile = this.profile(id);
      const userDataDir = this.userDataDirOf(profile);
      const insideRoot = userDataDir.startsWith(this.userDataRoot() + this.sep);
      this.profiles.delete(id);
      return {
        userDataDir,
        dataDir: !dropUserData ? 'not_requested' : insideRoot ? 'removed' : 'outside_root',
      };
    },
    duplicate_profile: ({ id }) => {
      const original = this.profile(id);
      return this.createProfile({
        ...original,
        name: copyName(original.name, this.settings.locale),
        fingerprintSeed: original.fingerprintSeed === null ? null : randomSeed(this.rng),
        userDataDir: null,
      });
    },
    export_profiles: ({ ids, path }) => {
      requireAbsolute(path, 'export file');
      for (const id of ids) {
        this.profile(id);
      }
      return ids.length;
    },
    import_profiles: ({ path }) => {
      requireAbsolute(path, 'import file');
      // Stands in for the file's content: two new profiles named after the file.
      const base = path.split(/[\\/]/).pop() ?? path;
      for (const n of [1, 2]) {
        this.createProfile({
          ...EMPTY_DRAFT,
          name: `${base} #${n}`,
          fingerprintSeed: randomSeed(this.rng),
          fpPlatform: 'windows',
        });
      }
      return { imported: 2, groupsCreated: 0, tagsCreated: 0, proxiesCreated: 0 };
    },
    set_profile_tags: ({ profileId, tagIds }) => {
      const profile = this.profile(profileId);
      profile.tagIds = this.tagIds(tagIds);
      profile.updatedAt = now();
      return null;
    },
    // Both batch actions are all-or-nothing, like the Rust transactions: validate, then apply.
    batch_set_group: ({ ids, groupId }) => {
      const profiles = ids.map((id) => {
        const profile = this.profile(id);
        // The FK is only checked on a row that is actually updated.
        if (groupId !== null && !this.groups.some((g) => g.id === groupId)) {
          throw mockError('invalid_input', 'FOREIGN KEY constraint failed (group)');
        }
        return profile;
      });
      const at = now();
      for (const profile of profiles) {
        Object.assign(profile, { groupId, updatedAt: at });
      }
      return null;
    },
    batch_add_tags: ({ ids, tagIds }) => {
      const profiles = ids.map((id) => this.profile(id));
      const added = this.tagIds(tagIds);
      const at = now();
      for (const profile of profiles) {
        profile.tagIds = [...new Set([...profile.tagIds, ...added])].sort();
        profile.updatedAt = at;
      }
      return null;
    },

    preview_command: ({ draft, profileId }) => {
      const kernel = this.kernelFor(draft.kernelId);
      const proxyId = cleanOpt(draft.proxyId);
      const proxy = proxyId ? this.proxy(proxyId) : null;
      return buildPlan({
        draft,
        program: kernel?.execPath ?? 'chrome',
        major: kernel?.majorVersion ?? NEWEST_MAJOR,
        proxyUrl: proxy ? this.proxyUrl(proxy) : null,
        userDataDir: this.dirFor(cleanOpt(draft.userDataDir), cleanOpt(profileId) ?? 'new-profile'),
        host: this.appInfo.os,
      });
    },
    get_arg_support: ({ kernelId }) =>
      argSupportFor(this.kernelFor(kernelId)?.majorVersion ?? NEWEST_MAJOR),
    validate_profile_for_kernel: ({ draft }) =>
      fieldWarnings(draft, this.kernelFor(draft.kernelId)?.majorVersion ?? NEWEST_MAJOR),
    launch_profile: ({ id }) => this.launch(id),
    stop_profile: ({ id }) => this.stop(id),
    list_running: () => this.listRunning(),
    batch_launch: ({ ids }) => this.batchLaunch(ids),
    batch_stop: ({ ids }) => this.stopMany(ids),

    list_kernels: () => this.kernels,
    add_kernel: (args) => this.addKernel(args),
    update_kernel: (args) => this.updateKernel(args),
    remove_kernel: ({ id }) => this.removeKernel(id),
    set_default_kernel: ({ id }) => {
      this.kernel(id);
      for (const kernel of this.kernels) {
        kernel.isDefault = kernel.id === id;
      }
      return null;
    },
    detect_kernels: () => this.detectKernels(),
    probe_kernel: ({ path }) => {
      requireAbsolute(path.trim(), 'kernel path');
      return this.probe(path.trim());
    },

    list_proxies: () => this.proxies,
    create_proxy: ({ input }) => {
      const at = now();
      const proxy: Proxy = {
        id: this.newId(),
        ...this.proxyInput(input),
        createdAt: at,
        updatedAt: at,
      };
      this.proxies.push(proxy);
      return proxy;
    },
    update_proxy: ({ id, input }) =>
      Object.assign(this.proxy(id), this.proxyInput(input), { updatedAt: now() }),
    delete_proxy: ({ id }) => {
      this.proxy(id);
      this.proxies = this.proxies.filter((p) => p.id !== id);
      for (const profile of this.profiles.values()) {
        if (profile.proxyId === id) {
          profile.proxyId = null;
        }
      }
      return null;
    },
    test_proxy: ({ id }) => this.testProxy(id),

    list_groups: () =>
      [...this.groups].sort((a, b) =>
        a.sortOrder !== b.sortOrder
          ? a.sortOrder - b.sortOrder
          : a.createdAt.localeCompare(b.createdAt)
      ),
    create_group: ({ input }) => {
      const group: Group = { id: this.newId(), ...this.groupInput(input, null), createdAt: now() };
      this.groups.push(group);
      return group;
    },
    update_group: ({ id, input }) =>
      Object.assign(this.find(this.groups, id, 'group'), this.groupInput(input, id)),
    delete_group: ({ id }) => {
      this.find(this.groups, id, 'group');
      this.groups = this.groups.filter((g) => g.id !== id);
      for (const profile of this.profiles.values()) {
        if (profile.groupId === id) {
          profile.groupId = null;
        }
      }
      return null;
    },

    list_tags: () => [...this.tags].sort((a, b) => compareNoCase(a.name, b.name)),
    create_tag: ({ input }) => {
      const tag: Tag = { id: this.newId(), ...this.tagInput(input, null), createdAt: now() };
      this.tags.push(tag);
      return tag;
    },
    update_tag: ({ id, input }) =>
      Object.assign(this.find(this.tags, id, 'tag'), this.tagInput(input, id)),
    delete_tag: ({ id }) => {
      this.find(this.tags, id, 'tag');
      this.tags = this.tags.filter((t) => t.id !== id);
      for (const profile of this.profiles.values()) {
        profile.tagIds = profile.tagIds.filter((tagId) => tagId !== id);
      }
      return null;
    },

    get_settings: () => this.settings,
    update_settings: ({ patch }) => this.updateSettings(patch),
    get_app_info: () => this.appInfo,
    show_main_window: ({ bootOk, failure }) => {
      this.bootReport = { bootOk, failure };
      if (!bootOk) {
        console.warn(`[mock] showMainWindow: the frontend reported a failed boot: ${failure}`);
      }
      return null;
    },
    quit_app: async () => {
      await this.shutdown();
      console.info('[mock] quitApp: every browser stopped; the app would exit now');
      return null;
    },
  };

  // ------------------------------------------------------------------------------------------
  // Dialogs and clipboard
  // ------------------------------------------------------------------------------------------

  openFile(options: OpenFileOptions): string | null {
    if (options.directory) {
      return '/home/user/browser-profiles';
    }
    if (options.filters?.some((filter) => filter.extensions.includes('json'))) {
      return '/home/user/Downloads/profiles-export.json';
    }
    return '/home/user/Applications/ungoogled-chromium-148.0.7778.215-1-x86_64.AppImage';
  }

  saveFile(options: SaveFileOptions): string | null {
    const name = options.defaultPath?.split(/[\\/]/).pop() || 'profiles.json';
    return `/home/user/Downloads/${name}`;
  }

  // ------------------------------------------------------------------------------------------
  // Window controls
  // ------------------------------------------------------------------------------------------

  readonly appWindow: WindowControls = {
    minimize: async () => {
      this.windowCalls.push('minimize');
    },
    toggleMaximize: async () => {
      this.windowCalls.push('toggleMaximize');
      this.maximized = !this.maximized;
      for (const listener of [...this.resizeListeners]) {
        queueMicrotask(listener);
      }
    },
    close: async () => {
      this.windowCalls.push('close');
      // Like `shell::on_window_event`: with browsers running (or starting), ask first.
      if (this.table.size > 0) {
        this.emit('app://close-requested', { runningCount: this.table.size });
      } else {
        console.info('[mock] close: nothing is running; the app would exit now');
      }
    },
    isMaximized: async () => this.maximized,
    onResized: async (handler) => {
      const listener = () => handler();
      this.resizeListeners.add(listener);
      return () => this.resizeListeners.delete(listener);
    },
  };
}

/** The mock as a bridge transport. */
export function createMockTransport(options: MockOptions = {}): Transport & {
  backend: MockBackend;
} {
  const backend = new MockBackend(options);
  const transport = {
    kind: 'mock' as const,
    backend,
    invoke: <K extends CommandName>(command: K, args: CommandArgs<K>) =>
      backend.invoke(command, args),
    listen: async <E extends EventName>(event: E, handler: (payload: EventPayloads[E]) => void) =>
      backend.listen(event, handler),
    openFile: async (o: OpenFileOptions) => backend.openFile(o),
    saveFile: async (o: SaveFileOptions) => backend.saveFile(o),
    writeClipboard: async (text: string) => {
      backend.clipboard = text;
      await navigator.clipboard?.writeText(text).catch(() => undefined);
    },
    appWindow: backend.appWindow,
  };
  if (typeof window !== 'undefined') {
    const controls: MockControls = {
      emit: (event, payload) => backend.emit(event, payload),
      backend,
    };
    Object.assign(window, { __FPL_MOCK__: controls });
  }
  return transport;
}
