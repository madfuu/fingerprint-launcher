import { describe, expect, it, vi } from 'vitest';

import {
  applySnapshot,
  applyStatusEvent,
  createRunningStore,
  type RunningSource,
} from '@/features/profiles/running-store';
import type { RunningInfo, StatusEvent } from '@/lib/bindings';

const T0 = '2026-10-13T08:00:00.000Z';
const NOW = '2026-10-13T09:00:00.000Z';

function info(profileId: string, pid: number, status: RunningInfo['status'] = 'running') {
  return { profileId, status, pid, startedAt: T0, kernelId: 'k', userDataDir: `/u/${profileId}` };
}

function event(
  profileId: string,
  status: StatusEvent['status'],
  pid: number | null = null
): StatusEvent {
  return { profileId, status, pid, error: null };
}

/** A source whose snapshot resolves only when the test says so. */
function controlledSource() {
  let emit: (event: StatusEvent) => void = () => {
    throw new Error('not subscribed');
  };
  let resolveSnapshot: (list: RunningInfo[]) => void = () => {};
  let rejectSnapshot: (error: unknown) => void = () => {};
  const unlisten = vi.fn();
  const calls: string[] = [];
  const source: RunningSource = {
    onStatusChanged: vi.fn(async (handler) => {
      calls.push('subscribe');
      emit = handler;
      return unlisten;
    }),
    listRunning: vi.fn(() => {
      calls.push('snapshot');
      return new Promise<RunningInfo[]>((resolve, reject) => {
        resolveSnapshot = resolve;
        rejectSnapshot = reject;
      });
    }),
    now: () => NOW,
  };
  return {
    source,
    calls,
    unlisten,
    emit: (e: StatusEvent) => emit(e),
    resolveSnapshot: (list: RunningInfo[]) => resolveSnapshot(list),
    rejectSnapshot: (error: unknown) => rejectSnapshot(error),
  };
}

/** Lets the store's pending promise continuations run. */
const flush = () => new Promise<void>((resolve) => setTimeout(resolve, 0));

describe('applyStatusEvent', () => {
  it('tracks a launch until it stops', () => {
    let byId = applyStatusEvent({}, event('a', 'starting', 7), NOW);
    expect(byId).toEqual({ a: { status: 'starting', pid: 7, startedAt: NOW, error: null } });
    byId = applyStatusEvent(byId, event('a', 'running', 7), '2026-10-13T09:00:03.000Z');
    expect(byId.a).toEqual({ status: 'running', pid: 7, startedAt: NOW, error: null });
    byId = applyStatusEvent(byId, event('a', 'stopping', 7), NOW);
    expect(byId.a?.status).toBe('stopping');
    expect(applyStatusEvent(byId, event('a', 'stopped'), NOW)).toEqual({});
  });

  it('keeps a failure until the next launch', () => {
    const failure = {
      code: 'startup_failed' as const,
      message: 'exited',
      hint: 'no_sandbox' as const,
      exitCode: 1,
      stderrTail: ['No usable sandbox!'],
    };
    let byId = applyStatusEvent({}, { ...event('a', 'error'), error: failure }, NOW);
    expect(byId.a).toEqual({ status: 'error', pid: null, startedAt: null, error: failure });
    byId = applyStatusEvent(byId, event('a', 'starting', 9), NOW);
    expect(byId.a).toEqual({ status: 'starting', pid: 9, startedAt: NOW, error: null });
  });
});

describe('applySnapshot', () => {
  it('lets events seen since the subscription win over the snapshot', () => {
    // b stopped after the subscription, c started; d had no event.
    const byId = applyStatusEvent({}, event('c', 'starting', 3), NOW);
    const merged = applySnapshot(
      byId,
      [info('b', 2), info('c', 3, 'starting'), info('d', 4)],
      new Set(['b', 'c'])
    );
    expect(merged).toEqual({
      // the snapshot's real start time fills in the same process
      c: { status: 'starting', pid: 3, startedAt: T0, error: null },
      d: { status: 'running', pid: 4, startedAt: T0, error: null },
    });
  });
});

describe('running store', () => {
  it('subscribes before it takes the snapshot', async () => {
    const ctl = controlledSource();
    const store = createRunningStore(ctl.source);
    void store.getState().init();
    await flush();
    expect(ctl.calls).toEqual(['subscribe', 'snapshot']);
  });

  it('does not lose a stop that arrives before the snapshot', async () => {
    const ctl = controlledSource();
    const store = createRunningStore(ctl.source);
    const done = store.getState().init();
    await flush();

    // The snapshot was taken while `a` still ran, but its `stopped` event arrives first.
    ctl.emit(event('a', 'stopped'));
    ctl.resolveSnapshot([info('a', 1), info('b', 2)]);
    await done;

    expect(store.getState().ready).toBe(true);
    expect(Object.keys(store.getState().byId)).toEqual(['b']);
  });

  it('keeps a launch that arrives before the snapshot and is missing from it', async () => {
    const ctl = controlledSource();
    const store = createRunningStore(ctl.source);
    const done = store.getState().init();
    await flush();

    ctl.emit(event('n', 'starting', 5));
    ctl.resolveSnapshot([]); // taken just before `n` was spawned
    await done;
    expect(store.getState().byId.n).toEqual({
      status: 'starting',
      pid: 5,
      startedAt: NOW,
      error: null,
    });
  });

  it('applies events that arrive after the snapshot', async () => {
    const ctl = controlledSource();
    const store = createRunningStore(ctl.source);
    const done = store.getState().init();
    await flush();
    ctl.resolveSnapshot([info('a', 1, 'starting')]);
    await done;
    expect(store.getState().byId.a).toMatchObject({ status: 'starting', startedAt: T0 });

    ctl.emit(event('a', 'running', 1));
    expect(store.getState().byId.a).toEqual({
      status: 'running',
      pid: 1,
      startedAt: T0,
      error: null,
    });
    ctl.emit(event('a', 'stopped'));
    expect(store.getState().byId).toEqual({});
  });

  it('settles an old event delivered after the snapshot to the latest state', async () => {
    const ctl = controlledSource();
    const store = createRunningStore(ctl.source);
    const done = store.getState().init();
    await flush();
    // Both events were emitted before the snapshot was taken but are delivered after it.
    ctl.resolveSnapshot([info('a', 1)]);
    await done;
    ctl.emit(event('a', 'starting', 1));
    ctl.emit(event('a', 'running', 1));
    expect(store.getState().byId.a).toMatchObject({ status: 'running', pid: 1 });
  });

  it('runs once, and reports a failed snapshot while events keep flowing', async () => {
    const ctl = controlledSource();
    const store = createRunningStore(ctl.source);
    const first = store.getState().init();
    expect(store.getState().init()).toBe(first);
    await flush();
    ctl.rejectSnapshot({ code: 'io', message: 'boom' });
    await first;
    expect(store.getState()).toMatchObject({ ready: true, error: { code: 'io', message: 'boom' } });
    ctl.emit(event('a', 'starting', 1));
    expect(store.getState().byId.a?.status).toBe('starting');
    expect(ctl.source.onStatusChanged).toHaveBeenCalledOnce();

    store.getState().dispose();
    expect(ctl.unlisten).toHaveBeenCalledOnce();
    expect(store.getState().byId).toEqual({});
  });

  it('can retry after the subscription failed', async () => {
    const ctl = controlledSource();
    vi.mocked(ctl.source.onStatusChanged).mockRejectedValueOnce('event.listen not allowed by ACL');
    const store = createRunningStore(ctl.source);
    await store.getState().init();
    expect(store.getState().error).toEqual({
      code: 'invalid_input',
      message: 'event.listen not allowed by ACL',
    });
    expect(ctl.source.listRunning).not.toHaveBeenCalled();

    const retry = store.getState().init();
    await flush();
    ctl.resolveSnapshot([info('a', 1)]);
    await retry;
    expect(store.getState()).toMatchObject({ error: null, byId: { a: { status: 'running' } } });
  });

  it('dismisses a shown failure', async () => {
    const ctl = controlledSource();
    const store = createRunningStore(ctl.source);
    const done = store.getState().init();
    await flush();
    ctl.resolveSnapshot([]);
    await done;
    ctl.emit(event('a', 'error'));
    store.getState().dismissError('a');
    expect(store.getState().byId).toEqual({});
  });

  it('ignores an init still in flight when disposed', async () => {
    const ctl = controlledSource();
    let subscribed: (unlisten: () => void) => void = () => {};
    const lateUnlisten = vi.fn();
    vi.mocked(ctl.source.onStatusChanged).mockImplementationOnce(
      () => new Promise((resolve) => (subscribed = resolve))
    );
    const store = createRunningStore(ctl.source);
    const first = store.getState().init();
    store.getState().dispose();
    subscribed(lateUnlisten);
    await first;
    // The late subscription is released and nothing reached the store.
    expect(lateUnlisten).toHaveBeenCalledOnce();
    expect(ctl.source.listRunning).not.toHaveBeenCalled();
    expect(store.getState()).toMatchObject({ ready: false, byId: {} });

    // Disposed while the snapshot is pending: its late result is dropped too.
    const second = store.getState().init();
    await flush();
    store.getState().dispose();
    expect(ctl.unlisten).toHaveBeenCalledOnce();
    ctl.resolveSnapshot([info('a', 1)]);
    await second;
    expect(store.getState()).toMatchObject({ ready: false, byId: {} });
  });
});
