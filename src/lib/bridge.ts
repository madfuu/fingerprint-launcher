/**
 * The frontend's only way to native code (design decision 4): every Tauri command, event,
 * file dialog, clipboard write and window control goes through this module. ESLint forbids
 * `@tauri-apps/*` imports everywhere except the Tauri transport (`bridge.tauri.ts`).
 *
 * - All DTOs come from `@/lib/bindings`, generated from the Rust types (design §5).
 * - Every failure rejects with a {@link BridgeError}, which is an `AppError`
 *   (`{ code, message }`), whatever the transport rejected with; see {@link toAppError}.
 * - Outside Tauri, a development build swaps in the in-memory mock (`bridge.mock.ts`), loaded
 *   through a dynamic import so production bundles contain none of it.
 */
import type {
  AppError,
  AppInfo,
  ArgSupport,
  BatchProgress,
  CloseRequested,
  ErrorCode,
  EventPayloads,
  FieldWarning,
  Group,
  GroupInput,
  ImportResult,
  Kernel,
  KernelCandidate,
  KernelInput,
  KernelProbe,
  LaunchPlan,
  Page,
  Profile,
  ProfileDraft,
  ProfileQuery,
  ProfileRow,
  Proxy,
  ProxyCheck,
  ProxyInput,
  PurgeResult,
  RunningInfo,
  Settings,
  SettingsPatch,
  StatusEvent,
  Tag,
  TagInput,
} from '@/lib/bindings';
import { EVENTS } from '@/lib/bindings';
import { tauriTransport } from '@/lib/bridge.tauri';

// ---------------------------------------------------------------------------------------------
// Command contract
// ---------------------------------------------------------------------------------------------

type Command<Args, Result> = { args: Args; result: Result };
type NoArgs = Record<string, never>;

/**
 * Every Tauri command: snake_case name (the Rust function) → camelCase arguments and result.
 * Rust's `()` arrives as `null`.
 */
export interface Commands {
  // profiles
  list_profiles: Command<{ query: ProfileQuery }, Page<ProfileRow>>;
  get_profile: Command<{ id: string }, Profile>;
  create_profile: Command<{ draft: ProfileDraft }, Profile>;
  update_profile: Command<{ id: string; draft: ProfileDraft }, Profile>;
  delete_profile: Command<{ id: string }, null>;
  restore_profile: Command<{ id: string }, null>;
  purge_profile: Command<{ id: string; dropUserData: boolean }, PurgeResult>;
  duplicate_profile: Command<{ id: string }, Profile>;
  export_profiles: Command<{ ids: string[]; path: string }, number>;
  import_profiles: Command<{ path: string }, ImportResult>;
  set_profile_tags: Command<{ profileId: string; tagIds: string[] }, null>;
  batch_set_group: Command<{ ids: string[]; groupId: string | null }, null>;
  batch_add_tags: Command<{ ids: string[]; tagIds: string[] }, null>;
  // launch
  preview_command: Command<{ draft: ProfileDraft; profileId: string | null }, LaunchPlan>;
  get_arg_support: Command<{ kernelId: string | null }, ArgSupport>;
  validate_profile_for_kernel: Command<{ draft: ProfileDraft }, FieldWarning[]>;
  launch_profile: Command<{ id: string }, null>;
  stop_profile: Command<{ id: string }, null>;
  list_running: Command<NoArgs, RunningInfo[]>;
  batch_launch: Command<{ ids: string[] }, BatchProgress>;
  batch_stop: Command<{ ids: string[] }, null>;
  // kernels
  list_kernels: Command<NoArgs, Kernel[]>;
  add_kernel: Command<{ path: string; name: string | null; majorVersion: number | null }, Kernel>;
  update_kernel: Command<{ id: string; input: KernelInput }, Kernel>;
  remove_kernel: Command<{ id: string }, null>;
  set_default_kernel: Command<{ id: string }, null>;
  detect_kernels: Command<NoArgs, KernelCandidate[]>;
  probe_kernel: Command<{ path: string }, KernelProbe>;
  // proxies, groups, tags
  list_proxies: Command<NoArgs, Proxy[]>;
  create_proxy: Command<{ input: ProxyInput }, Proxy>;
  update_proxy: Command<{ id: string; input: ProxyInput }, Proxy>;
  delete_proxy: Command<{ id: string }, null>;
  test_proxy: Command<{ id: string }, ProxyCheck>;
  list_groups: Command<NoArgs, Group[]>;
  create_group: Command<{ input: GroupInput }, Group>;
  update_group: Command<{ id: string; input: GroupInput }, Group>;
  delete_group: Command<{ id: string }, null>;
  list_tags: Command<NoArgs, Tag[]>;
  create_tag: Command<{ input: TagInput }, Tag>;
  update_tag: Command<{ id: string; input: TagInput }, Tag>;
  delete_tag: Command<{ id: string }, null>;
  // settings, app
  get_settings: Command<NoArgs, Settings>;
  update_settings: Command<{ patch: SettingsPatch }, Settings>;
  get_app_info: Command<NoArgs, AppInfo>;
  show_main_window: Command<{ bootOk: boolean; failure: string | null }, null>;
  quit_app: Command<NoArgs, null>;
}

export type CommandName = keyof Commands;
export type CommandArgs<K extends CommandName> = Commands[K]['args'];
export type CommandResult<K extends CommandName> = Commands[K]['result'];

/** Backend event names (`profile://status-changed`, ...). */
export type EventName = keyof EventPayloads;
export type Unlisten = () => void;

export interface DialogFilter {
  name: string;
  extensions: string[];
}

export interface OpenFileOptions {
  title?: string;
  filters?: DialogFilter[];
  defaultPath?: string;
  /** Pick a directory instead of a file. */
  directory?: boolean;
}

export interface SaveFileOptions {
  title?: string;
  filters?: DialogFilter[];
  defaultPath?: string;
}

/**
 * The window's own controls, for the custom title bar (the window has no native decorations
 * except on macOS). Dragging is not here: Tauri's injected script handles elements marked
 * `data-tauri-drag-region`.
 */
export interface WindowControls {
  minimize(): Promise<void>;
  toggleMaximize(): Promise<void>;
  /** Asks to close; the backend may intercept it (`app://close-requested`, design §4.5). */
  close(): Promise<void>;
  isMaximized(): Promise<boolean>;
  /** Fires on every size change, maximizing and restoring included. */
  onResized(handler: () => void): Promise<Unlisten>;
}

/**
 * What a backend provides: the real one over Tauri IPC, or the in-memory mock. Rejections may
 * have any shape; {@link call} normalizes them.
 */
export interface Transport {
  readonly kind: 'tauri' | 'mock';
  invoke<K extends CommandName>(command: K, args: CommandArgs<K>): Promise<CommandResult<K>>;
  listen<E extends EventName>(
    event: E,
    handler: (payload: EventPayloads[E]) => void
  ): Promise<Unlisten>;
  /** Single file or directory; `null` when cancelled. */
  openFile(options: OpenFileOptions): Promise<string | null>;
  /** `null` when cancelled. */
  saveFile(options: SaveFileOptions): Promise<string | null>;
  writeClipboard(text: string): Promise<void>;
  appWindow: WindowControls;
}

// ---------------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------------

/** Exhaustive by type: adding an `ErrorCode` on the Rust side fails typecheck here. */
const ERROR_CODES: Record<ErrorCode, true> = {
  not_found: true,
  conflict: true,
  invalid_input: true,
  db: true,
  io: true,
  no_kernel: true,
  kernel_missing: true,
  kernel_not_executable: true,
  profile_running: true,
  profile_running_externally: true,
  spawn_failed: true,
  startup_failed: true,
  shutting_down: true,
  kernel_version_unknown: true,
  proxy_test_failed: true,
};

export function isErrorCode(value: unknown): value is ErrorCode {
  return typeof value === 'string' && Object.hasOwn(ERROR_CODES, value);
}

/**
 * Code given to rejections that are not an `AppError`: Tauri's own plain-string rejections
 * (argument deserialization, ACL denial, unknown command), plugin errors, and exceptions
 * thrown by the transport itself. All of them mean the request could not be carried out as
 * sent, so they share `invalid_input`; the original text stays in `message` (design §5).
 */
export const FOREIGN_ERROR_CODE: ErrorCode = 'invalid_input';

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

/**
 * Normalizes any rejection to `{ code, message }`:
 *
 * | Rejected with | Result |
 * |---|---|
 * | `{ code: <known ErrorCode>, message }` (the backend's `AppError`) | unchanged |
 * | `{ code: <unknown string>, message }` (e.g. a newer backend) | `invalid_input`, message `"<code>: <message>"` |
 * | a string (Tauri / plugin rejection) | `invalid_input`, message = the string |
 * | an `Error`, or `{ message }` | `invalid_input`, message = its message |
 * | anything else | `invalid_input`, message = its JSON / string form |
 */
export function toAppError(error: unknown): AppError {
  if (error instanceof BridgeError) {
    return { code: error.code, message: error.message };
  }
  if (isRecord(error) && typeof error.message === 'string') {
    if (isErrorCode(error.code)) {
      return { code: error.code, message: error.message };
    }
    if (typeof error.code === 'string' && error.code !== '') {
      return { code: FOREIGN_ERROR_CODE, message: `${error.code}: ${error.message}` };
    }
    const fallback = error instanceof Error ? error.name : 'unknown error';
    return { code: FOREIGN_ERROR_CODE, message: error.message || fallback };
  }
  if (typeof error === 'string') {
    return { code: FOREIGN_ERROR_CODE, message: error };
  }
  return { code: FOREIGN_ERROR_CODE, message: describe(error) };
}

function describe(value: unknown): string {
  if (value === undefined) {
    return 'unknown error';
  }
  try {
    return JSON.stringify(value) ?? String(value);
  } catch {
    return String(value);
  }
}

/** What every bridge call rejects with: an `Error` (stack, `cause`) that is also an `AppError`. */
export class BridgeError extends Error implements AppError {
  readonly code: ErrorCode;

  constructor(error: AppError, options?: { cause?: unknown }) {
    super(error.message, options);
    this.name = 'BridgeError';
    this.code = error.code;
  }

  static from(error: unknown): BridgeError {
    return error instanceof BridgeError
      ? error
      : new BridgeError(toAppError(error), { cause: error });
  }
}

export function isBridgeError(error: unknown): error is BridgeError {
  return error instanceof BridgeError;
}

// ---------------------------------------------------------------------------------------------
// Transport selection
// ---------------------------------------------------------------------------------------------

/** Whether the page runs inside the Tauri WebView. */
export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

let selected: Promise<Transport> | null = null;

async function selectTransport(): Promise<Transport> {
  // `import.meta.env.DEV` is a build-time constant: production builds drop this branch and the
  // mock chunk with it.
  if (import.meta.env.DEV && !isTauri()) {
    const { createMockTransport } = await import('@/lib/bridge.mock');
    return createMockTransport({ search: window.location.search });
  }
  return tauriTransport;
}

function transport(): Promise<Transport> {
  selected ??= selectTransport();
  return selected;
}

/** Replaces the transport (tests); `null` restores automatic selection. */
export function setTransport(next: Transport | null): void {
  selected = next ? Promise.resolve(next) : null;
}

/** `'mock'` in a development build outside Tauri. */
export async function bridgeKind(): Promise<Transport['kind']> {
  return (await transport()).kind;
}

async function call<K extends CommandName>(
  command: K,
  args: CommandArgs<K>
): Promise<CommandResult<K>> {
  try {
    const backend = await transport();
    return await backend.invoke(command, args);
  } catch (error) {
    throw BridgeError.from(error);
  }
}

async function native<T>(run: (backend: Transport) => Promise<T>): Promise<T> {
  try {
    return await run(await transport());
  } catch (error) {
    throw BridgeError.from(error);
  }
}

const none: NoArgs = {};

// ---------------------------------------------------------------------------------------------
// Commands (design §5)
// ---------------------------------------------------------------------------------------------

// profiles

export function listProfiles(query: ProfileQuery): Promise<Page<ProfileRow>> {
  return call('list_profiles', { query });
}

export function getProfile(id: string): Promise<Profile> {
  return call('get_profile', { id });
}

export function createProfile(draft: ProfileDraft): Promise<Profile> {
  return call('create_profile', { draft });
}

export function updateProfile(id: string, draft: ProfileDraft): Promise<Profile> {
  return call('update_profile', { id, draft });
}

/** Moves to the trash; `profile_running` while it runs. */
export async function deleteProfile(id: string): Promise<void> {
  await call('delete_profile', { id });
}

export async function restoreProfile(id: string): Promise<void> {
  await call('restore_profile', { id });
}

/** Deletes for good; with `dropUserData` also its data directory when safe (design §5). */
export function purgeProfile(id: string, dropUserData: boolean): Promise<PurgeResult> {
  return call('purge_profile', { id, dropUserData });
}

export function duplicateProfile(id: string): Promise<Profile> {
  return call('duplicate_profile', { id });
}

/** Writes the profiles to `path` (from {@link saveFileDialog}); resolves with the count. */
export function exportProfiles(ids: string[], path: string): Promise<number> {
  return call('export_profiles', { ids, path });
}

/** All or nothing. */
export function importProfiles(path: string): Promise<ImportResult> {
  return call('import_profiles', { path });
}

export async function setProfileTags(profileId: string, tagIds: string[]): Promise<void> {
  await call('set_profile_tags', { profileId, tagIds });
}

/** `groupId = null` takes the profiles out of every group. */
export async function batchSetGroup(ids: string[], groupId: string | null): Promise<void> {
  await call('batch_set_group', { ids, groupId });
}

export async function batchAddTags(ids: string[], tagIds: string[]): Promise<void> {
  await call('batch_add_tags', { ids, tagIds });
}

// launch

/** Command line of an editor draft; `profileId` is `null` for a profile not saved yet. */
export function previewCommand(draft: ProfileDraft, profileId: string | null): Promise<LaunchPlan> {
  return call('preview_command', { draft, profileId });
}

/** Arguments the kernel accepts; `null` = the default kernel. */
export function getArgSupport(kernelId: string | null): Promise<ArgSupport> {
  return call('get_arg_support', { kernelId });
}

/** Set fields the draft's kernel would not pass on. */
export function validateProfileForKernel(draft: ProfileDraft): Promise<FieldWarning[]> {
  return call('validate_profile_for_kernel', { draft });
}

/** Resolves once the process exists (`starting`); later states arrive as status events. */
export async function launchProfile(id: string): Promise<void> {
  await call('launch_profile', { id });
}

/** Resolves once the browser has stopped. */
export async function stopProfile(id: string): Promise<void> {
  await call('stop_profile', { id });
}

export function listRunning(): Promise<RunningInfo[]> {
  return call('list_running', none);
}

/** Resolves with the final count; progress arrives as `batch://progress`. */
export function batchLaunch(ids: string[]): Promise<BatchProgress> {
  return call('batch_launch', { ids });
}

export async function batchStop(ids: string[]): Promise<void> {
  await call('batch_stop', { ids });
}

/** Stops every browser and exits (after the user confirmed `app://close-requested`). */
export async function quitApp(): Promise<void> {
  await call('quit_app', none);
}

// kernels

export function listKernels(): Promise<Kernel[]> {
  return call('list_kernels', none);
}

/** `majorVersion` overrides the probed one; required when probing fails (`kernel_version_unknown`). */
export function addKernel(
  path: string,
  name: string | null,
  majorVersion: number | null
): Promise<Kernel> {
  return call('add_kernel', { path, name, majorVersion });
}

export function updateKernel(id: string, input: KernelInput): Promise<Kernel> {
  return call('update_kernel', { id, input });
}

export async function removeKernel(id: string): Promise<void> {
  await call('remove_kernel', { id });
}

export async function setDefaultKernel(id: string): Promise<void> {
  await call('set_default_kernel', { id });
}

/** Candidates only; nothing is added. */
export function detectKernels(): Promise<KernelCandidate[]> {
  return call('detect_kernels', none);
}

/** `path` must be absolute. */
export function probeKernel(path: string): Promise<KernelProbe> {
  return call('probe_kernel', { path });
}

// proxies

export function listProxies(): Promise<Proxy[]> {
  return call('list_proxies', none);
}

export function createProxy(input: ProxyInput): Promise<Proxy> {
  return call('create_proxy', { input });
}

export function updateProxy(id: string, input: ProxyInput): Promise<Proxy> {
  return call('update_proxy', { id, input });
}

export async function deleteProxy(id: string): Promise<void> {
  await call('delete_proxy', { id });
}

/** Looks up the exit IP through the proxy (network access; only on user request). */
export function testProxy(id: string): Promise<ProxyCheck> {
  return call('test_proxy', { id });
}

// groups

export function listGroups(): Promise<Group[]> {
  return call('list_groups', none);
}

export function createGroup(input: GroupInput): Promise<Group> {
  return call('create_group', { input });
}

export function updateGroup(id: string, input: GroupInput): Promise<Group> {
  return call('update_group', { id, input });
}

export async function deleteGroup(id: string): Promise<void> {
  await call('delete_group', { id });
}

// tags

export function listTags(): Promise<Tag[]> {
  return call('list_tags', none);
}

export function createTag(input: TagInput): Promise<Tag> {
  return call('create_tag', { input });
}

export function updateTag(id: string, input: TagInput): Promise<Tag> {
  return call('update_tag', { id, input });
}

export async function deleteTag(id: string): Promise<void> {
  await call('delete_tag', { id });
}

// settings, app

export function getSettings(): Promise<Settings> {
  return call('get_settings', none);
}

/** Applies the fields present in `patch`; resolves with the resulting settings. */
export function updateSettings(patch: SettingsPatch): Promise<Settings> {
  return call('update_settings', { patch });
}

export function getAppInfo(): Promise<AppInfo> {
  return call('get_app_info', none);
}

/**
 * Shows the window created hidden, once the first frame is rendered. `bootOk` is false when
 * that frame is the error screen; `failure` then says why (printed by the backend; a
 * `FPL_SMOKE_EXIT_ON_READY` run exits 1 on it).
 */
export async function showMainWindow(bootOk: boolean, failure: string | null): Promise<void> {
  await call('show_main_window', { bootOk, failure });
}

// ---------------------------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------------------------

/** Subscribes to a backend event; resolves with the function that unsubscribes. */
export function listen<E extends EventName>(
  event: E,
  handler: (payload: EventPayloads[E]) => void
): Promise<Unlisten> {
  return native((backend) => backend.listen(event, handler));
}

export function onProfileStatusChanged(handler: (event: StatusEvent) => void): Promise<Unlisten> {
  return listen(EVENTS.profileStatusChanged, handler);
}

export function onBatchProgress(handler: (progress: BatchProgress) => void): Promise<Unlisten> {
  return listen(EVENTS.batchProgress, handler);
}

export function onCloseRequested(handler: (request: CloseRequested) => void): Promise<Unlisten> {
  return listen(EVENTS.appCloseRequested, handler);
}

// ---------------------------------------------------------------------------------------------
// Dialogs and clipboard
// ---------------------------------------------------------------------------------------------

/** Native open dialog for one file (or directory); `null` when cancelled. */
export function openFileDialog(options: OpenFileOptions = {}): Promise<string | null> {
  return native((backend) => backend.openFile(options));
}

/** Native save dialog; `null` when cancelled. */
export function saveFileDialog(options: SaveFileOptions = {}): Promise<string | null> {
  return native((backend) => backend.saveFile(options));
}

/** Through the clipboard plugin: `navigator.clipboard` is unreliable in WebKitGTK. */
export function copyToClipboard(text: string): Promise<void> {
  return native((backend) => backend.writeClipboard(text));
}

// ---------------------------------------------------------------------------------------------
// Window controls (custom title bar)
// ---------------------------------------------------------------------------------------------

export function minimizeWindow(): Promise<void> {
  return native((backend) => backend.appWindow.minimize());
}

export function toggleMaximizeWindow(): Promise<void> {
  return native((backend) => backend.appWindow.toggleMaximize());
}

/** Same as the window manager's close: with browsers running, the backend asks first. */
export function closeWindow(): Promise<void> {
  return native((backend) => backend.appWindow.close());
}

export function isWindowMaximized(): Promise<boolean> {
  return native((backend) => backend.appWindow.isMaximized());
}

/** Resolves with the function that unsubscribes. */
export function onWindowResized(handler: () => void): Promise<Unlisten> {
  return native((backend) => backend.appWindow.onResized(handler));
}
