/**
 * Which profiles this launcher is running (design §6): the `listRunning()` snapshot plus the
 * `profile://status-changed` events after it.
 *
 * No event may be lost between the snapshot and the subscription, so the order is:
 *
 * 1. Subscribe. Events are applied as they arrive.
 * 2. Take the snapshot.
 * 3. Merge: a profile that received an event since step 1 keeps the event's state; the
 *    snapshot only fills in profiles without one.
 *
 * Why this is enough: the backend emits an event for every status change, in order, under the
 * same lock that guards the table the snapshot reads. So the last event received for a
 * profile is never older than any snapshot taken before it was emitted, and an event emitted
 * before the snapshot but delivered after it is followed by every later event of that
 * profile — the final state is the latest one either way. A profile without events since
 * step 1 did not change since then, so the snapshot is current for it.
 */
import { create } from 'zustand';

import type {
  AppError,
  LaunchFailure,
  ProfileStatus,
  RunningInfo,
  StatusEvent,
} from '@/lib/bindings';
import { listRunning, onProfileStatusChanged, toAppError, type Unlisten } from '@/lib/bridge';

export interface RunState {
  /** Never `stopped`: a stopped profile has no entry. */
  status: ProfileStatus;
  /** Set while the process exists (`starting`, `running`, `stopping`). */
  pid: number | null;
  /** RFC 3339; when a `starting` event arrives first, the time it arrived. */
  startedAt: string | null;
  /** Set with `error`, until the next launch or {@link RunningState.dismissError}. */
  error: LaunchFailure | null;
}

/** Statuses with a live process. */
export function isActive(status: ProfileStatus): boolean {
  return status === 'starting' || status === 'running' || status === 'stopping';
}

/** Applies one status event. Pure. */
export function applyStatusEvent(
  byId: Readonly<Record<string, RunState>>,
  event: StatusEvent,
  receivedAt: string
): Record<string, RunState> {
  const next = { ...byId };
  const previous = byId[event.profileId];
  switch (event.status) {
    case 'stopped':
      delete next[event.profileId];
      break;
    case 'error':
      next[event.profileId] = { status: 'error', pid: null, startedAt: null, error: event.error };
      break;
    case 'starting':
    case 'running':
    case 'stopping': {
      const sameProcess = previous !== undefined && previous.pid === event.pid;
      next[event.profileId] = {
        status: event.status,
        pid: event.pid,
        startedAt: sameProcess ? previous.startedAt : receivedAt,
        error: null,
      };
      break;
    }
  }
  return next;
}

/**
 * Merges the `listRunning()` snapshot: profiles in `seen` (they received an event since the
 * subscription) keep their state; every other profile takes the snapshot's, or is dropped when
 * the snapshot does not list it. Pure.
 */
export function applySnapshot(
  byId: Readonly<Record<string, RunState>>,
  snapshot: readonly RunningInfo[],
  seen: ReadonlySet<string>
): Record<string, RunState> {
  const next: Record<string, RunState> = {};
  for (const [id, state] of Object.entries(byId)) {
    if (seen.has(id)) {
      next[id] = state;
    }
  }
  for (const info of snapshot) {
    const current = next[info.profileId];
    if (!seen.has(info.profileId)) {
      next[info.profileId] = {
        status: info.status,
        pid: info.pid,
        startedAt: info.startedAt,
        error: null,
      };
    } else if (current && current.pid === info.pid) {
      // Same process as the event described: the snapshot knows the real start time.
      next[info.profileId] = { ...current, startedAt: info.startedAt };
    }
  }
  return next;
}

export interface RunningState {
  byId: Record<string, RunState>;
  /** The snapshot has been merged (or could not be taken). */
  ready: boolean;
  /** Why the subscription or the snapshot failed. */
  error: AppError | null;
  /** Subscribes and takes the snapshot, once; later calls return the same promise. */
  init(): Promise<void>;
  /** Forgets the failure shown for a profile. */
  dismissError(profileId: string): void;
  /** Unsubscribes and clears everything (tests, hot reload). */
  dispose(): void;
}

export interface RunningSource {
  listRunning(): Promise<RunningInfo[]>;
  onStatusChanged(handler: (event: StatusEvent) => void): Promise<Unlisten>;
  now(): string;
}

export const bridgeSource: RunningSource = {
  listRunning,
  onStatusChanged: onProfileStatusChanged,
  now: () => new Date().toISOString(),
};

export function createRunningStore(source: RunningSource = bridgeSource) {
  let started: Promise<void> | null = null;
  let unlisten: Unlisten | null = null;
  /** Bumped by `dispose()`: an `init()` still in flight from before must not touch the store. */
  let generation = 0;

  return create<RunningState>()((set, get) => ({
    byId: {},
    ready: false,
    error: null,

    init() {
      started ??= (async () => {
        const mine = generation;
        const current = () => mine === generation;
        /** Profiles with an event before the snapshot was merged. */
        const seen = new Set<string>();
        let merged = false;
        let stop: Unlisten;
        try {
          stop = await source.onStatusChanged((event) => {
            if (!current()) {
              return;
            }
            if (!merged) {
              seen.add(event.profileId);
            }
            set({ byId: applyStatusEvent(get().byId, event, source.now()) });
          });
        } catch (error) {
          if (current()) {
            started = null; // allow a retry
            set({ ready: true, error: toAppError(error) });
          }
          return;
        }
        if (!current()) {
          stop();
          return;
        }
        unlisten = stop;
        try {
          const snapshot = await source.listRunning();
          if (!current()) {
            return;
          }
          set({ byId: applySnapshot(get().byId, snapshot, seen), error: null });
        } catch (error) {
          if (!current()) {
            return;
          }
          // Events keep flowing; only the profiles that ran before the app loaded are unknown.
          set({ error: toAppError(error) });
        }
        merged = true;
        set({ ready: true });
      })();
      return started;
    },

    dismissError(profileId) {
      const current = get().byId[profileId];
      if (current?.status === 'error') {
        const next = { ...get().byId };
        delete next[profileId];
        set({ byId: next });
      }
    },

    dispose() {
      generation += 1;
      unlisten?.();
      unlisten = null;
      started = null;
      set({ byId: {}, ready: false, error: null });
    },
  }));
}

export const useRunningStore = createRunningStore();

/** The run state of one profile; `undefined` when stopped. */
export function useRunState(profileId: string): RunState | undefined {
  return useRunningStore((state) => state.byId[profileId]);
}

/** Number of profiles with a live process. */
export function useActiveCount(): number {
  return useRunningStore(
    (state) => Object.values(state.byId).filter((run) => isActive(run.status)).length
  );
}
