import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { BatchProgress, ProfileDraft, StatusEvent } from '@/lib/bindings';
import {
  batchAddTags,
  batchLaunch,
  batchSetGroup,
  BridgeError,
  createProfile,
  deleteProfile,
  duplicateProfile,
  getAppInfo,
  getArgSupport,
  getProfile,
  getSettings,
  launchProfile,
  listProfiles,
  listRunning,
  onBatchProgress,
  onProfileStatusChanged,
  previewCommand,
  setTransport,
  stopProfile,
  updateProfile,
  updateSettings,
  validateProfileForKernel,
} from '@/lib/bridge';
import { createMockTransport, type MockBackend } from '@/lib/bridge.mock';
import { EMPTY_DRAFT } from '@/lib/mock/seed';

const TIMING = { latency: 0, spawn: 10, startup: 100, failure: 50, stop: 30, proxyCheck: 0 };

let backend: MockBackend;

function idOf(name: string): string {
  const profile = [...backend.profiles.values()].find((p) => p.name === name);
  if (!profile) {
    throw new Error(`no seeded profile ${name}`);
  }
  return profile.id;
}

function kernelId(major: number): string {
  const kernel = backend.kernels.find((k) => k.majorVersion === major);
  if (!kernel) {
    throw new Error(`no seeded kernel ${major}`);
  }
  return kernel.id;
}

async function recordStatus(): Promise<StatusEvent[]> {
  const events: StatusEvent[] = [];
  await onProfileStatusChanged((event) => events.push(event));
  return events;
}

const query = (overrides: Partial<Parameters<typeof listProfiles>[0]> = {}) => ({
  page: 1,
  pageSize: 20,
  view: 'all' as const,
  groupId: null,
  tagIds: [],
  keyword: null,
  ...overrides,
});

beforeEach(() => {
  vi.useFakeTimers();
  const transport = createMockTransport({ timing: TIMING });
  backend = transport.backend;
  setTransport(transport);
});

afterEach(() => {
  setTransport(null);
  vi.useRealTimers();
});

describe('mock launcher', () => {
  it('walks starting → running → stopping → stopped', async () => {
    const events = await recordStatus();
    const id = idOf('Etsy Handmade');

    const launched = launchProfile(id);
    await vi.advanceTimersByTimeAsync(TIMING.spawn);
    await launched;
    expect(events.map((e) => e.status)).toEqual(['starting']);
    const pid = events[0]?.pid;
    expect(pid).toEqual(expect.any(Number));
    expect((await listRunning()).find((r) => r.profileId === id)).toMatchObject({
      status: 'starting',
      pid,
    });

    await vi.advanceTimersByTimeAsync(TIMING.startup);
    expect(events.at(-1)).toEqual({ profileId: id, status: 'running', pid, error: null });

    const stopped = stopProfile(id);
    await vi.advanceTimersByTimeAsync(0);
    expect(events.at(-1)).toMatchObject({ status: 'stopping', pid });
    await vi.advanceTimersByTimeAsync(TIMING.stop);
    await stopped;
    expect(events.at(-1)).toEqual({ profileId: id, status: 'stopped', pid: null, error: null });
    expect((await listRunning()).some((r) => r.profileId === id)).toBe(false);
  });

  it('reports a startup failure with its hint and stderr', async () => {
    const events = await recordStatus();
    const id = idOf('沙箱失败示例');

    const launched = launchProfile(id);
    await vi.advanceTimersByTimeAsync(TIMING.spawn);
    await launched;
    await vi.advanceTimersByTimeAsync(TIMING.failure);

    expect(events.map((e) => e.status)).toEqual(['starting', 'error']);
    expect(events[1]).toMatchObject({
      pid: null,
      error: { code: 'startup_failed', hint: 'no_sandbox', exitCode: 1 },
    });
    expect(events[1]?.error?.stderrTail.join('\n')).toContain('No usable sandbox');
    expect(await listRunning()).not.toContainEqual(expect.objectContaining({ profileId: id }));
  });

  it('refuses to launch or delete a running profile', async () => {
    const events = await recordStatus();
    const running = await listRunning();
    expect(running).toHaveLength(3);
    const id = idOf('Shopify 主店铺');
    expect(running.map((r) => r.profileId)).toContain(id);

    const launch = await launchProfile(id).catch((e: unknown) => e);
    expect(launch).toBeInstanceOf(BridgeError);
    expect(launch).toMatchObject({ code: 'profile_running' });
    await expect(deleteProfile(id)).rejects.toMatchObject({ code: 'profile_running' });
    await vi.advanceTimersByTimeAsync(0);
    expect(events).toEqual([]);
  });

  it('emits an error event for a launch that fails before the spawn', async () => {
    const events = await recordStatus();
    const id = idOf('旧 Facebook 账号'); // in the trash
    await expect(launchProfile(id)).rejects.toMatchObject({ code: 'invalid_input' });
    await vi.advanceTimersByTimeAsync(0);
    expect(events).toEqual([
      {
        profileId: id,
        status: 'error',
        pid: null,
        error: expect.objectContaining({ code: 'invalid_input', hint: null }),
      },
    ]);
  });

  it('batch launches with at most `batchConcurrency` profiles starting', async () => {
    await updateSettings({ batchConcurrency: 2, batchIntervalMs: 5 });
    const ids = [
      'Etsy Handmade',
      'eBay UK',
      'LinkedIn Recruiting',
      'Pinterest Pins',
      'Discord 社区',
    ];
    const starting = new Set<string>();
    let maxStarting = 0;
    await onProfileStatusChanged((event) => {
      if (event.status === 'starting') {
        starting.add(event.profileId);
      } else {
        starting.delete(event.profileId);
      }
      maxStarting = Math.max(maxStarting, starting.size);
    });
    const progress: BatchProgress[] = [];
    await onBatchProgress((p) => progress.push(p));

    const batch = batchLaunch([...ids.map(idOf), idOf('Shopify 主店铺')]);
    await vi.advanceTimersByTimeAsync(1000);
    const result = await batch;

    expect(maxStarting).toBe(2);
    expect(result).toEqual({ total: 6, launched: 5, failed: 0, skipped: 1 });
    expect(progress.at(-1)).toEqual(result);
    expect(progress).toHaveLength(6);
  });
});

describe('mock queries', () => {
  it('pages and filters like ProfileQuery', async () => {
    const all = await listProfiles(query());
    expect(all.total).toBe(22);
    expect(all.items[0]?.name).toBe('Shopify 主店铺'); // newest first
    expect(all.items[0]?.tags.map((t) => t.name)).toEqual(['VIP', '美国']); // by name

    const lastPage = await listProfiles(query({ page: 3, pageSize: 10 }));
    expect(lastPage.items).toHaveLength(2);

    expect((await listProfiles(query({ view: 'trash' }))).total).toBe(3);
    expect((await listProfiles(query({ view: 'running' }))).total).toBe(3);
    expect(
      (await listProfiles(query({ keyword: 'SHOP' }))).items.map((p) => p.name).sort()
    ).toEqual(['Shopee 台湾', 'Shopify 主店铺']);

    const vip = backend.tags.find((t) => t.name === 'VIP')?.id ?? '';
    const sea = backend.tags.find((t) => t.name === '东南亚')?.id ?? '';
    expect((await listProfiles(query({ tagIds: [vip, sea] }))).total).toBe(5);

    const testing = backend.groups.find((g) => g.name === '测试')?.id ?? null;
    expect((await listProfiles(query({ groupId: testing }))).total).toBe(3);

    await expect(listProfiles(query({ pageSize: 0 }))).rejects.toMatchObject({
      code: 'invalid_input',
    });
  });

  it('follows the version gates of design §3.2', async () => {
    expect(await getArgSupport(kernelId(142))).toEqual({
      brand: true,
      gpuVendorRenderer: true,
      disableGpuFingerprint: true,
      disableSpoofing: false,
    });
    const newest = {
      brand: true,
      gpuVendorRenderer: false,
      disableGpuFingerprint: false,
      disableSpoofing: true,
    };
    expect(await getArgSupport(kernelId(144))).toEqual(newest);
    expect(await getArgSupport(null)).toEqual(newest); // default kernel: 150
    await expect(getArgSupport('gone')).rejects.toMatchObject({ code: 'not_found' });

    const draft: ProfileDraft = {
      ...EMPTY_DRAFT,
      name: 'gpu',
      fingerprintSeed: 1,
      fpGpuVendor: 'NVIDIA',
      disableSpoofing: ['gpu'],
    };
    expect(await validateProfileForKernel({ ...draft, kernelId: kernelId(142) })).toEqual([
      {
        field: 'disableSpoofing',
        reason: 'requires_newer_kernel',
        supported: { min: 144, max: null },
      },
    ]);
    expect(await validateProfileForKernel({ ...draft, kernelId: kernelId(150) })).toEqual([
      { field: 'fpGpuVendor', reason: 'removed_in_kernel', supported: { min: 139, max: 143 } },
    ]);
  });

  it('previews the README "custom User-Agent" example in builder order', async () => {
    const plan = await previewCommand(
      {
        ...EMPTY_DRAFT,
        name: 'readme',
        fingerprintSeed: 2023,
        fpPlatform: 'macos',
        fpPlatformVersion: '15.2.0',
        fpBrand: 'Edge',
        disableNonProxiedUdp: false,
        timezone: 'America/Los_Angeles',
      },
      null
    );
    const root = (await getAppInfo()).defaultUserDataRoot;
    expect(plan.args).toEqual([
      `--user-data-dir=${root}/new-profile`,
      '--fingerprint=2023',
      '--fingerprint-platform=macos',
      '--fingerprint-platform-version=15.2.0',
      '--fingerprint-brand=Edge',
      '--timezone=America/Los_Angeles',
    ]);
    expect(plan.shellLine.endsWith('--fingerprint-brand=Edge --timezone=America/Los_Angeles')).toBe(
      true
    );
  });

  it('names copies in the UI language', async () => {
    const id = idOf('eBay UK');
    expect((await duplicateProfile(id)).name).toBe('eBay UK 副本');
    await updateSettings({ locale: 'en' });
    const copy = await duplicateProfile(id);
    expect(copy.name).toBe('eBay UK (copy)');
    expect(copy.userDataDir).toBeNull();
    expect(copy.fingerprintSeed).not.toBe(backend.profiles.get(id)?.fingerprintSeed);
  });

  it('reads a draft like serde: extra fields ignored, missing fields defaulted', async () => {
    const id = idOf('旧 Facebook 账号'); // in the trash
    const row = (await listProfiles(query({ view: 'trash' }))).items.find((p) => p.id === id);
    if (!row) {
      throw new Error('seeded trash row missing');
    }
    // The editor may hand back a whole row; only its draft fields count.
    const updated = await updateProfile(id, {
      ...row,
      id: 'forged',
      deletedAt: null,
      createdAt: '2000-01-01T00:00:00.000Z',
      name: 'renamed',
    } as ProfileDraft);
    expect(updated).toMatchObject({ id, deletedAt: row.deletedAt, createdAt: row.createdAt });
    expect(updated.name).toBe('renamed');
    expect(Object.keys(await getProfile(id))).not.toContain('group');

    const partial = await createProfile({ name: 'partial' } as ProfileDraft);
    expect(partial).toMatchObject({ ...EMPTY_DRAFT, name: 'partial' });
  });

  it('applies batch group and tag changes all or nothing, with the Rust error codes', async () => {
    const ids = [idOf('Etsy Handmade'), idOf('eBay UK')];
    const before = ids.map((id) => backend.profiles.get(id)?.groupId);
    await expect(batchSetGroup(ids, 'no-such-group')).rejects.toMatchObject({
      code: 'invalid_input',
    });
    await expect(batchSetGroup([...ids, 'no-such-profile'], null)).rejects.toMatchObject({
      code: 'not_found',
    });
    expect(ids.map((id) => backend.profiles.get(id)?.groupId)).toEqual(before);
    await expect(batchSetGroup([], 'no-such-group')).resolves.toBeUndefined();

    const vip = backend.tags.find((t) => t.name === 'VIP')?.id ?? '';
    await expect(batchAddTags([...ids, 'no-such-profile'], [vip])).rejects.toMatchObject({
      code: 'not_found',
    });
    expect(ids.some((id) => backend.profiles.get(id)?.tagIds.includes(vip))).toBe(false);
    await batchAddTags(ids, [vip, vip]);
    expect(ids.every((id) => backend.profiles.get(id)?.tagIds.includes(vip))).toBe(true);
  });
});

describe('mock URL overrides', () => {
  it('apply theme, language and OS; ignore unknown values', async () => {
    setTransport(createMockTransport({ timing: TIMING, search: '?theme=dark&lang=en&os=macos' }));
    expect(await getSettings()).toMatchObject({ theme: 'dark', locale: 'en' });
    expect((await getAppInfo()).os).toBe('macos');

    setTransport(createMockTransport({ timing: TIMING, search: '?theme=pink&lang=fr' }));
    expect(await getSettings()).toMatchObject({ theme: 'system', locale: 'zh-CN' });
  });
});
