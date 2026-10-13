import { afterEach, describe, expect, it, vi } from 'vitest';

import type { EventPayloads, StatusEvent } from '@/lib/bindings';
import {
  BridgeError,
  closeWindow,
  FOREIGN_ERROR_CODE,
  getArgSupport,
  isErrorCode,
  isWindowMaximized,
  launchProfile,
  listProfiles,
  minimizeWindow,
  onProfileStatusChanged,
  onWindowResized,
  openFileDialog,
  previewCommand,
  setTransport,
  toAppError,
  toggleMaximizeWindow,
  type Transport,
} from '@/lib/bridge';
import { EMPTY_DRAFT } from '@/lib/mock/seed';

/** A transport whose `invoke` and `listen` are spies. */
function fakeTransport(invoke: (command: string, args: unknown) => Promise<unknown>) {
  const handlers = new Map<string, (payload: unknown) => void>();
  const unlisten = vi.fn();
  const transport: Transport = {
    kind: 'tauri',
    invoke: vi.fn(invoke) as Transport['invoke'],
    listen: vi.fn(async (event: string, handler: (payload: never) => void) => {
      handlers.set(event, handler as (payload: unknown) => void);
      return unlisten;
    }) as Transport['listen'],
    openFile: vi.fn(async () => {
      throw 'dialog.open not allowed. Permissions associated with this command: dialog:allow-open';
    }),
    saveFile: vi.fn(async () => null),
    writeClipboard: vi.fn(async () => {}),
    appWindow: {
      minimize: vi.fn(async () => {
        throw 'window.minimize not allowed. Permissions associated with this command: core:window:allow-minimize';
      }),
      toggleMaximize: vi.fn(async () => {}),
      close: vi.fn(async () => {}),
      isMaximized: vi.fn(async () => true),
      onResized: vi.fn(async () => unlisten),
    },
  };
  const fire = <E extends keyof EventPayloads>(event: E, payload: EventPayloads[E]) =>
    handlers.get(event)?.(payload);
  return { transport, unlisten, fire };
}

afterEach(() => {
  setTransport(null);
});

describe('toAppError', () => {
  it('keeps a backend AppError as it is', () => {
    expect(toAppError({ code: 'kernel_missing', message: 'kernel not found: /x' })).toEqual({
      code: 'kernel_missing',
      message: 'kernel not found: /x',
    });
  });

  it("wraps Tauri's plain-string rejections (argument / ACL errors)", () => {
    const text = 'invalid args `query` for command `list_profiles`: missing field `page`';
    expect(toAppError(text)).toEqual({ code: 'invalid_input', message: text });
    expect(FOREIGN_ERROR_CODE).toBe('invalid_input');
  });

  it('wraps exceptions and message-only objects', () => {
    expect(toAppError(new TypeError('window.__TAURI_INTERNALS__ is undefined'))).toEqual({
      code: 'invalid_input',
      message: 'window.__TAURI_INTERNALS__ is undefined',
    });
    expect(toAppError(new RangeError(''))).toEqual({
      code: 'invalid_input',
      message: 'RangeError',
    });
    expect(toAppError({ message: 'boom' })).toEqual({ code: 'invalid_input', message: 'boom' });
  });

  it('keeps an unknown code in the message', () => {
    expect(toAppError({ code: 'brand_new', message: 'from a newer backend' })).toEqual({
      code: 'invalid_input',
      message: 'brand_new: from a newer backend',
    });
  });

  it('describes anything else', () => {
    expect(toAppError(undefined).message).toBe('unknown error');
    expect(toAppError(null).message).toBe('null');
    expect(toAppError(42).message).toBe('42');
    expect(toAppError({ code: 'db' }).message).toBe('{"code":"db"}');
  });

  it('recognizes every generated error code and nothing else', () => {
    expect(isErrorCode('proxy_test_failed')).toBe(true);
    expect(isErrorCode('toString')).toBe(false);
    expect(isErrorCode(undefined)).toBe(false);
  });
});

describe('commands', () => {
  it('invoke the snake_case command with camelCase arguments', async () => {
    const { transport } = fakeTransport(async () => null);
    setTransport(transport);

    await previewCommand(EMPTY_DRAFT, null);
    await getArgSupport('k1');
    await launchProfile('p1');
    expect(vi.mocked(transport.invoke).mock.calls).toEqual([
      ['preview_command', { draft: EMPTY_DRAFT, profileId: null }],
      ['get_arg_support', { kernelId: 'k1' }],
      ['launch_profile', { id: 'p1' }],
    ]);
  });

  it('reject with a BridgeError whatever the transport rejected with', async () => {
    const rejections: unknown[] = [
      { code: 'profile_running', message: 'profile p1 is already running' },
      'invalid args `id` for command `launch_profile`: invalid type: null, expected a string',
      new Error('IPC is gone'),
    ];
    for (const rejection of rejections) {
      setTransport(fakeTransport(() => Promise.reject(rejection)).transport);
      const error = await launchProfile('p1').catch((e: unknown) => e);
      expect(error).toBeInstanceOf(BridgeError);
      expect(error).toBeInstanceOf(Error);
      expect({
        code: (error as BridgeError).code,
        message: (error as BridgeError).message,
      }).toEqual(toAppError(rejection));
      expect((error as BridgeError).cause).toBe(rejection);
    }
  });

  it('pass results through', async () => {
    const page = { items: [], total: 0, page: 1, pageSize: 20 };
    setTransport(fakeTransport(async () => page).transport);
    await expect(
      listProfiles({ page: 1, pageSize: 20, view: 'all', groupId: null, tagIds: [], keyword: null })
    ).resolves.toEqual(page);
  });
});

describe('events and native helpers', () => {
  it('deliver the payload and return the unlisten function', async () => {
    const { transport, unlisten, fire } = fakeTransport(async () => null);
    setTransport(transport);
    const seen: StatusEvent[] = [];
    const stop = await onProfileStatusChanged((event) => seen.push(event));

    const event: StatusEvent = { profileId: 'p1', status: 'running', pid: 7, error: null };
    fire('profile://status-changed', event);
    expect(seen).toEqual([event]);
    expect(transport.listen).toHaveBeenCalledWith('profile://status-changed', expect.any(Function));

    stop();
    expect(unlisten).toHaveBeenCalledOnce();
  });

  it('normalize plugin rejections too', async () => {
    setTransport(fakeTransport(async () => null).transport);
    await expect(openFileDialog()).rejects.toMatchObject({
      name: 'BridgeError',
      code: 'invalid_input',
      message: expect.stringContaining('dialog:allow-open'),
    });
  });

  it('route window controls to the transport and normalize their rejections', async () => {
    const { transport, unlisten } = fakeTransport(async () => null);
    setTransport(transport);
    await expect(minimizeWindow()).rejects.toMatchObject({
      name: 'BridgeError',
      code: 'invalid_input',
      message: expect.stringContaining('core:window:allow-minimize'),
    });
    await expect(isWindowMaximized()).resolves.toBe(true);
    await toggleMaximizeWindow();
    await closeWindow();
    expect(transport.appWindow.toggleMaximize).toHaveBeenCalledOnce();
    expect(transport.appWindow.close).toHaveBeenCalledOnce();
    const handler = vi.fn();
    await expect(onWindowResized(handler)).resolves.toBe(unlisten);
    expect(transport.appWindow.onResized).toHaveBeenCalledWith(handler);
  });
});
