/**
 * The bridge ↔ Rust command contract, checked against the Rust sources: every registered
 * command has a wrapper, and each wrapper invokes it by its exact name with exactly the
 * argument keys Tauri expects. A mismatch would otherwise only fail at runtime in the real app,
 * which CI does not exercise beyond `showMainWindow`.
 *
 * Tauri's `#[tauri::command]` renames arguments to camelCase by default (tauri-macros
 * `ArgumentCase::Camel`); injected parameters (`State`, `AppHandle`) are not arguments.
 */
import { afterEach, describe, expect, it } from 'vitest';

import libRs from '../../src-tauri/src/lib.rs?raw';

import type { KernelInput, ProfileDraft, ProfileQuery, SettingsPatch } from '@/lib/bindings';
import * as bridge from '@/lib/bridge';
import type { Transport } from '@/lib/bridge';

const commandSources = import.meta.glob<string>('../../src-tauri/src/commands/*.rs', {
  query: '?raw',
  import: 'default',
  eager: true,
});

/** Types Tauri injects instead of reading them from the invoke arguments. */
const INJECTED = /^(tauri::)?(State|AppHandle|Window|WebviewWindow|Webview|ipc::Request)\b/;

function camelCase(snake: string): string {
  return snake.replace(/_([a-z0-9])/g, (_, c: string) => c.toUpperCase());
}

/** Splits `a: A<'_, B>, c: C` at the commas outside brackets. */
function splitTopLevel(text: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let current = '';
  for (const c of text) {
    if ('<([{'.includes(c)) depth += 1;
    if ('>)]}'.includes(c)) depth -= 1;
    if (c === ',' && depth === 0) {
      parts.push(current);
      current = '';
    } else {
      current += c;
    }
  }
  parts.push(current);
  return parts.map((part) => part.trim()).filter((part) => part !== '');
}

/** Registered command → its argument keys, as Tauri reads them. */
function rustCommands(): Map<string, string[]> {
  const handler = /generate_handler!\[([\s\S]*?)\]/.exec(libRs)?.[1] ?? '';
  const registered = [...handler.matchAll(/commands::(\w+)::(\w+)/g)];
  const commands = new Map<string, string[]>();
  for (const [, module, name] of registered) {
    const source = commandSources[`../../src-tauri/src/commands/${module}.rs`];
    if (source === undefined || name === undefined) {
      throw new Error(`no source for commands::${module}::${name}`);
    }
    const start = new RegExp(`#\\[tauri::command\\][\\s\\S]*?pub async fn ${name}\\(`).exec(source);
    if (!start) {
      throw new Error(`commands::${module}::${name} is not an async #[tauri::command]`);
    }
    // The parameter list runs to the parenthesis that closes the one in the match.
    let depth = 1;
    let end = start.index + start[0].length;
    for (; depth > 0; end += 1) {
      if (source[end] === '(') depth += 1;
      if (source[end] === ')') depth -= 1;
    }
    const params = splitTopLevel(source.slice(start.index + start[0].length, end - 1));
    const keys = params
      .map((param) => /^(?:mut\s+)?(\w+)\s*:\s*(.+)$/s.exec(param))
      .filter((match): match is RegExpExecArray => match !== null && !INJECTED.test(match[2] ?? ''))
      .map((match) => camelCase(match[1] ?? ''));
    commands.set(name, keys.sort());
  }
  return commands;
}

/** Records what each wrapper invokes. */
function recordingTransport(calls: Map<string, string[]>): Transport {
  return {
    kind: 'tauri',
    invoke: (async (command: string, args: object) => {
      calls.set(command, Object.keys(args).sort());
      return null;
    }) as Transport['invoke'],
    listen: async () => () => {},
    openFile: async () => null,
    saveFile: async () => null,
    writeClipboard: async () => {},
  };
}

const draft = {} as ProfileDraft;
const id = 'id';

/** Every command wrapper, called once (argument values are irrelevant here). */
const WRAPPERS: (() => Promise<unknown>)[] = [
  () => bridge.listProfiles({} as ProfileQuery),
  () => bridge.getProfile(id),
  () => bridge.createProfile(draft),
  () => bridge.updateProfile(id, draft),
  () => bridge.deleteProfile(id),
  () => bridge.restoreProfile(id),
  () => bridge.purgeProfile(id, true),
  () => bridge.duplicateProfile(id),
  () => bridge.exportProfiles([id], '/tmp/x.json'),
  () => bridge.importProfiles('/tmp/x.json'),
  () => bridge.setProfileTags(id, []),
  () => bridge.batchSetGroup([id], null),
  () => bridge.batchAddTags([id], []),
  () => bridge.previewCommand(draft, null),
  () => bridge.getArgSupport(null),
  () => bridge.validateProfileForKernel(draft),
  () => bridge.launchProfile(id),
  () => bridge.stopProfile(id),
  () => bridge.listRunning(),
  () => bridge.batchLaunch([id]),
  () => bridge.batchStop([id]),
  () => bridge.quitApp(),
  () => bridge.listKernels(),
  () => bridge.addKernel('/opt/chrome', null, null),
  () => bridge.updateKernel(id, {} as KernelInput),
  () => bridge.removeKernel(id),
  () => bridge.setDefaultKernel(id),
  () => bridge.detectKernels(),
  () => bridge.probeKernel('/opt/chrome'),
  () => bridge.listProxies(),
  () => bridge.createProxy({ name: null, scheme: 'http', host: 'h', port: 1, note: null }),
  () => bridge.updateProxy(id, { name: null, scheme: 'http', host: 'h', port: 1, note: null }),
  () => bridge.deleteProxy(id),
  () => bridge.testProxy(id),
  () => bridge.listGroups(),
  () => bridge.createGroup({ name: 'g', color: null, sortOrder: 0 }),
  () => bridge.updateGroup(id, { name: 'g', color: null, sortOrder: 0 }),
  () => bridge.deleteGroup(id),
  () => bridge.listTags(),
  () => bridge.createTag({ name: 't', color: '#000' }),
  () => bridge.updateTag(id, { name: 't', color: '#000' }),
  () => bridge.deleteTag(id),
  () => bridge.getSettings(),
  () => bridge.updateSettings({} as SettingsPatch),
  () => bridge.getAppInfo(),
  () => bridge.showMainWindow(),
];

afterEach(() => {
  bridge.setTransport(null);
});

describe('bridge ↔ Rust command contract', () => {
  it('parses the Rust command layer', () => {
    const rust = rustCommands();
    expect(rust.size).toBeGreaterThanOrEqual(46);
    expect(rust.get('purge_profile')).toEqual(['dropUserData', 'id']);
    expect(rust.get('show_main_window')).toEqual([]);
  });

  it('invokes every registered command by name with exactly its camelCase arguments', async () => {
    const invoked = new Map<string, string[]>();
    bridge.setTransport(recordingTransport(invoked));
    for (const call of WRAPPERS) {
      await call();
    }
    expect(invoked.size).toBe(WRAPPERS.length); // one wrapper per command, no duplicates
    expect(Object.fromEntries(invoked)).toEqual(Object.fromEntries(rustCommands()));
  });
});
