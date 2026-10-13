/**
 * Seed data of the mock backend: a realistic, deterministic library for UI work and
 * screenshots (same ids and values on every load). Never imported outside the mock.
 */
import type {
  FailureHint,
  FpPlatform,
  Group,
  Kernel,
  Profile,
  ProfileDraft,
  Proxy,
  ProxyCheck,
  SpoofingFeature,
  Tag,
} from '@/lib/bindings';

/** Deterministic PRNG (mulberry32): numbers in [0, 1). */
export function createRng(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** A v4-shaped UUID from `rng`. */
export function uuidFrom(rng: () => number): string {
  const hex = Array.from({ length: 32 }, () => Math.floor(rng() * 16).toString(16));
  hex[12] = '4';
  hex[16] = ((Math.floor(rng() * 4) & 0x3) | 0x8).toString(16);
  const s = hex.join('');
  return `${s.slice(0, 8)}-${s.slice(8, 12)}-${s.slice(12, 16)}-${s.slice(16, 20)}-${s.slice(20)}`;
}

/** Largest `--fingerprint` seed (positive signed 32-bit). */
export const SEED_MAX = 2_147_483_647;

export function randomSeed(rng: () => number): number {
  return 1 + Math.floor(rng() * SEED_MAX);
}

/** Every seeded timestamp is relative to this instant. */
const BASE_TIME = Date.parse('2026-09-01T08:00:00.000Z');

function at(hours: number): string {
  return new Date(BASE_TIME + hours * 3_600_000).toISOString();
}

export const EMPTY_DRAFT: ProfileDraft = {
  name: '',
  note: null,
  groupId: null,
  kernelId: null,
  proxyId: null,
  tagIds: [],
  fingerprintSeed: null,
  fpPlatform: null,
  fpPlatformVersion: null,
  fpBrand: null,
  fpBrandVersion: null,
  fpHardwareConcurrency: null,
  fpGpuVendor: null,
  fpGpuRenderer: null,
  disableGpuFingerprint: false,
  disableSpoofing: [],
  disableNonProxiedUdp: true,
  lang: null,
  acceptLang: null,
  timezone: null,
  headless: false,
  userDataDir: null,
  startUrls: [],
  extraArgs: null,
};

/** How a seeded profile fails at startup. */
export interface MockFailure {
  hint: FailureHint | null;
  exitCode: number;
  stderrTail: string[];
}

export interface MockSeed {
  kernels: Kernel[];
  groups: Group[];
  tags: Tag[];
  proxies: Proxy[];
  profiles: Profile[];
  /** Running when the mock starts, with their pids. */
  running: { profileId: string; pid: number; minutesAgo: number }[];
  /** Profiles whose browser exits during startup. */
  failures: Record<string, MockFailure>;
  /** `testProxy` results; `null` = both lookup services fail. */
  proxyChecks: Record<string, ProxyCheck | null>;
}

type KernelKey = 'k142' | 'k144' | 'k150';
type GroupKey = 'ecommerce' | 'social' | 'ads' | 'testing' | 'research';
type TagKey = 'vip' | 'us' | 'eu' | 'sea' | 'new' | 'pending' | 'warmup';
type ProxyKey = 'la' | 'fra' | 'tokyo' | 'office' | 'sg' | 'v6';

interface ProfileSpec {
  name: string;
  note?: string;
  group?: GroupKey;
  tags?: TagKey[];
  proxy?: ProxyKey;
  kernel?: KernelKey;
  /** `undefined` = random, `null` = no `--fingerprint`. */
  seed?: number | null;
  platform?: FpPlatform;
  platformVersion?: string;
  brand?: string;
  brandVersion?: string;
  cores?: number;
  gpuVendor?: string;
  gpuRenderer?: string;
  disableGpuFingerprint?: boolean;
  disableSpoofing?: SpoofingFeature[];
  udp?: boolean;
  lang?: string;
  acceptLang?: string;
  timezone?: string;
  headless?: boolean;
  userDataDir?: string;
  startUrls?: string[];
  extraArgs?: string;
  launchedHoursAgo?: number;
  trashedHoursAgo?: number;
  running?: { pid: number; minutesAgo: number };
  fails?: MockFailure;
}

const KERNELS: Record<KernelKey, Omit<Kernel, 'id' | 'createdAt'> & { hours: number }> = {
  k142: {
    name: 'Chromium 142 (GPU)',
    execPath: '/home/user/Downloads/ungoogled-chromium-142.0.7444.175-1_linux/chrome',
    majorVersion: 142,
    fullVersion: '142.0.7444.175',
    isDefault: false,
    hours: -240,
  },
  k144: {
    name: 'Chromium 144',
    execPath: '/opt/ungoogled-chromium-144/chrome',
    majorVersion: 144,
    fullVersion: '144.0.7559.132',
    isDefault: false,
    hours: -200,
  },
  k150: {
    name: 'Chromium 150',
    execPath: '/home/user/Applications/ungoogled-chromium-150.0.7871.186-1-x86_64.AppImage',
    majorVersion: 150,
    fullVersion: '150.0.7871.186',
    isDefault: true,
    hours: -160,
  },
};

const GROUPS: Record<GroupKey, { name: string; color: string }> = {
  ecommerce: { name: '电商运营', color: '#3b82f6' },
  social: { name: '社媒矩阵', color: '#8b5cf6' },
  ads: { name: 'Ads', color: '#f97316' },
  testing: { name: '测试', color: '#10b981' },
  research: { name: 'Research', color: '#64748b' },
};

const TAGS: Record<TagKey, { name: string; color: string }> = {
  vip: { name: 'VIP', color: '#ef4444' },
  us: { name: '美国', color: '#3b82f6' },
  eu: { name: '欧洲', color: '#6366f1' },
  sea: { name: '东南亚', color: '#14b8a6' },
  new: { name: '新号', color: '#22c55e' },
  pending: { name: '待验证', color: '#f59e0b' },
  warmup: { name: 'Warm-up', color: '#a855f7' },
};

const PROXIES: Record<
  ProxyKey,
  Pick<Proxy, 'name' | 'scheme' | 'host' | 'port' | 'note'> & { check: ProxyCheck | null }
> = {
  la: {
    name: '洛杉矶住宅 IP',
    scheme: 'http',
    host: 'la.proxy.example.net',
    port: 8080,
    note: '按月续费',
    check: {
      ip: '104.28.12.77',
      country: 'US',
      city: 'Los Angeles',
      timezone: 'America/Los_Angeles',
      service: 'ipinfo',
      latencyMs: 182,
    },
  },
  fra: {
    name: 'Frankfurt DC',
    scheme: 'socks5',
    host: '45.86.12.33',
    port: 1080,
    note: null,
    check: {
      ip: '45.86.12.33',
      country: 'DE',
      city: 'Frankfurt am Main',
      timezone: 'Europe/Berlin',
      service: 'ipinfo',
      latencyMs: 96,
    },
  },
  tokyo: {
    name: 'Tokyo',
    scheme: 'socks5',
    host: 'jp-01.example.org',
    port: 1080,
    note: null,
    check: {
      ip: '139.162.88.10',
      country: 'JP',
      city: 'Tokyo',
      timezone: 'Asia/Tokyo',
      service: 'ip_api',
      latencyMs: 241,
    },
  },
  office: {
    name: null,
    scheme: 'http',
    host: '192.168.1.20',
    port: 3128,
    note: '办公室出口',
    check: {
      ip: '58.33.120.5',
      country: 'CN',
      city: 'Shanghai',
      timezone: 'Asia/Shanghai',
      service: 'ipinfo',
      latencyMs: 35,
    },
  },
  sg: {
    name: 'Singapore',
    scheme: 'http',
    host: 'sg.example.com',
    port: 8000,
    note: null,
    check: {
      ip: '128.199.77.4',
      country: 'SG',
      city: 'Singapore',
      timezone: 'Asia/Singapore',
      service: 'ipinfo',
      latencyMs: 120,
    },
  },
  v6: {
    name: 'IPv6 test',
    scheme: 'socks5',
    host: '2001:db8::25',
    port: 1080,
    note: 'documentation range: never answers',
    check: null,
  },
};

const NO_SANDBOX: MockFailure = {
  hint: 'no_sandbox',
  exitCode: 1,
  stderrTail: [
    '[1012/093201.552:FATAL:zygote_host_impl_linux.cc(132)] No usable sandbox! If you are running on Ubuntu 23.10+ or another Linux distro that has disabled unprivileged user namespaces with AppArmor, see https://chromium.googlesource.com/chromium/src/+/main/docs/security/apparmor-userns-restrictions.md.',
    'Trace/breakpoint trap (core dumped)',
  ],
};

const MISSING_FUSE: MockFailure = {
  hint: 'missing_fuse',
  exitCode: 127,
  stderrTail: [
    'dlopen(): error loading libfuse.so.2',
    '',
    'AppImages require FUSE to run.',
    'You might still be able to extract the contents of this AppImage',
    'if you run it with the --appimage-extract option.',
  ],
};

/** Newest first, as the list shows them. */
const PROFILES: ProfileSpec[] = [
  {
    name: 'Shopify 主店铺',
    note: '主力店铺，勿频繁切换代理',
    group: 'ecommerce',
    tags: ['vip', 'us'],
    proxy: 'la',
    seed: 1000,
    platform: 'windows',
    platformVersion: '15.0.0',
    brand: 'Chrome',
    brandVersion: '150.0.7871.186',
    cores: 8,
    lang: 'en-US',
    acceptLang: 'en-US,en',
    timezone: 'America/Los_Angeles',
    startUrls: ['https://admin.shopify.com'],
    launchedHoursAgo: 0.5,
    running: { pid: 48213, minutesAgo: 32 },
  },
  {
    name: 'TikTok 达人号 A',
    group: 'social',
    tags: ['us', 'warmup'],
    proxy: 'la',
    platform: 'macos',
    platformVersion: '15.2.0',
    brand: 'Chrome',
    lang: 'en-US',
    acceptLang: 'en-US,en',
    timezone: 'America/New_York',
    launchedHoursAgo: 0.2,
    running: { pid: 48377, minutesAgo: 11 },
  },
  {
    name: 'Amazon US Seller Central',
    note: 'Two-step verification on the shared phone',
    group: 'ecommerce',
    tags: ['vip', 'us'],
    proxy: 'la',
    platform: 'windows',
    brand: 'Edge',
    cores: 12,
    lang: 'en-US',
    timezone: 'America/Chicago',
    launchedHoursAgo: 20,
  },
  {
    name: 'Facebook Ads 投放 01',
    group: 'ads',
    tags: ['us', 'new'],
    proxy: 'la',
    seed: 2023,
    platform: 'macos',
    platformVersion: '15.2.0',
    brand: 'Edge',
    lang: 'en-US',
    timezone: 'America/Los_Angeles',
    launchedHoursAgo: 30,
  },
  {
    name: 'Instagram Brand',
    group: 'social',
    tags: ['eu'],
    proxy: 'fra',
    platform: 'macos',
    brand: 'Chrome',
    lang: 'de-DE',
    acceptLang: 'de-DE,de,en',
    timezone: 'Europe/Berlin',
    launchedHoursAgo: 0.1,
    running: { pid: 49102, minutesAgo: 4 },
  },
  {
    name: '小红书运营',
    note: '内容号，每天发帖',
    group: 'social',
    tags: ['new'],
    platform: 'macos',
    lang: 'zh-CN',
    acceptLang: 'zh-CN,zh',
    timezone: 'Asia/Shanghai',
    launchedHoursAgo: 50,
  },
  {
    name: 'Etsy Handmade',
    group: 'ecommerce',
    tags: ['us'],
    proxy: 'la',
    platform: 'macos',
    brand: 'Vivaldi',
    lang: 'en-US',
    timezone: 'America/Denver',
  },
  {
    name: 'eBay UK',
    group: 'ecommerce',
    tags: ['eu'],
    proxy: 'fra',
    platform: 'windows',
    brand: 'Chrome',
    lang: 'en-GB',
    acceptLang: 'en-GB,en',
    timezone: 'Europe/London',
    launchedHoursAgo: 72,
  },
  {
    name: 'Google Ads 测试',
    note: 'GPU 参数仅在 139–143 内核生效',
    group: 'ads',
    tags: ['pending'],
    kernel: 'k142',
    platform: 'linux',
    gpuVendor: 'NVIDIA Corporation',
    gpuRenderer: 'NVIDIA GeForce RTX 3060/PCIe/SSE2',
    lang: 'en-US',
    timezone: 'UTC',
    launchedHoursAgo: 100,
  },
  {
    name: 'LinkedIn Recruiting',
    group: 'research',
    platform: 'windows',
    brand: 'Edge',
    timezone: 'Europe/London',
  },
  {
    name: 'Twitter 矩阵 #3',
    group: 'social',
    tags: ['warmup'],
    proxy: 'tokyo',
    platform: 'windows',
    lang: 'ja-JP',
    acceptLang: 'ja-JP,ja,en',
    timezone: 'Asia/Tokyo',
    launchedHoursAgo: 8,
  },
  {
    name: '抖音直播间',
    group: 'social',
    tags: ['vip'],
    platform: 'windows',
    brand: 'Chrome',
    lang: 'zh-CN',
    timezone: 'Asia/Shanghai',
    startUrls: ['https://live.douyin.com'],
  },
  {
    name: 'Shopee 台湾',
    group: 'ecommerce',
    tags: ['sea'],
    proxy: 'sg',
    platform: 'windows',
    lang: 'zh-TW',
    acceptLang: 'zh-TW,zh',
    timezone: 'Asia/Taipei',
  },
  {
    name: 'Lazada 东南亚',
    group: 'ecommerce',
    tags: ['sea', 'new'],
    proxy: 'sg',
    kernel: 'k144',
    platform: 'linux',
    disableSpoofing: ['font', 'audio'],
    lang: 'en-SG',
    timezone: 'Asia/Singapore',
    launchedHoursAgo: 140,
  },
  {
    name: 'Pinterest Pins',
    group: 'social',
    tags: ['us'],
    platform: 'windows',
    brand: 'Opera',
    timezone: 'America/New_York',
  },
  {
    name: 'Reddit Research',
    group: 'research',
    proxy: 'office',
    platform: 'linux',
    headless: true,
    extraArgs: '--window-size=1280,800',
  },
  {
    name: '测试环境（无指纹）',
    note: '不传 --fingerprint，行为同原版 Chromium',
    group: 'testing',
    seed: null,
  },
  {
    name: '沙箱失败示例',
    note: '启动即失败：演示 --no-sandbox 提示',
    group: 'testing',
    tags: ['pending'],
    platform: 'linux',
    fails: NO_SANDBOX,
  },
  {
    name: 'AppImage FUSE demo',
    note: 'Fails at startup: shows the libfuse2 hint',
    group: 'testing',
    platform: 'linux',
    udp: false,
    fails: MISSING_FUSE,
  },
  {
    name: 'Walmart Marketplace',
    group: 'ecommerce',
    tags: ['us'],
    proxy: 'la',
    platform: 'windows',
    brand: 'Chrome',
    userDataDir: '/data/browser-profiles/walmart',
    timezone: 'America/Chicago',
  },
  {
    name: 'Mercado Libre',
    group: 'ecommerce',
    proxy: 'v6',
    platform: 'windows',
    lang: 'pt-BR',
    timezone: 'America/Sao_Paulo',
  },
  {
    name: 'Discord 社区',
    platform: 'windows',
    timezone: 'Asia/Shanghai',
  },
  {
    name: '旧 Facebook 账号',
    note: '已封禁',
    group: 'ads',
    proxy: 'la',
    platform: 'windows',
    trashedHoursAgo: 30,
  },
  {
    name: 'Temu 卖家后台',
    group: 'ecommerce',
    tags: ['new'],
    platform: 'windows',
    trashedHoursAgo: 6,
  },
  {
    name: 'Notion Workspace',
    platform: 'macos',
    trashedHoursAgo: 90,
  },
];

/** Hours after `BASE_TIME` that count as "now" for the seed's relative times. */
const SEED_NOW_HOURS = 24 * 30;

export function createSeed(): MockSeed {
  const rng = createRng(20261013);
  const id = () => uuidFrom(rng);

  const kernelIds = {} as Record<KernelKey, string>;
  const kernels = (Object.keys(KERNELS) as KernelKey[]).map((key): Kernel => {
    const { hours, ...kernel } = KERNELS[key];
    kernelIds[key] = id();
    return { id: kernelIds[key], ...kernel, createdAt: at(hours) };
  });

  const groupIds = {} as Record<GroupKey, string>;
  const groups = (Object.keys(GROUPS) as GroupKey[]).map((key, index): Group => {
    groupIds[key] = id();
    return { id: groupIds[key], ...GROUPS[key], sortOrder: index, createdAt: at(-300 + index) };
  });

  const tagIds = {} as Record<TagKey, string>;
  const tags = (Object.keys(TAGS) as TagKey[]).map((key, index): Tag => {
    tagIds[key] = id();
    return { id: tagIds[key], ...TAGS[key], createdAt: at(-290 + index) };
  });

  const proxyIds = {} as Record<ProxyKey, string>;
  const proxyChecks: Record<string, ProxyCheck | null> = {};
  const proxies = (Object.keys(PROXIES) as ProxyKey[]).map((key, index): Proxy => {
    const { check, ...proxy } = PROXIES[key];
    proxyIds[key] = id();
    proxyChecks[proxyIds[key]] = check;
    const createdAt = at(-280 + index);
    return { id: proxyIds[key], ...proxy, createdAt, updatedAt: createdAt };
  });

  const running: MockSeed['running'] = [];
  const failures: Record<string, MockFailure> = {};
  const profiles = PROFILES.map((spec, index): Profile => {
    const profileId = id();
    // Newest first: the first spec was created last.
    const createdHours = SEED_NOW_HOURS - 26 - index * 19;
    if (spec.running) {
      running.push({ profileId, ...spec.running });
    }
    if (spec.fails) {
      failures[profileId] = spec.fails;
    }
    return {
      ...EMPTY_DRAFT,
      id: profileId,
      name: spec.name,
      note: spec.note ?? null,
      groupId: spec.group ? groupIds[spec.group] : null,
      kernelId: spec.kernel ? kernelIds[spec.kernel] : null,
      proxyId: spec.proxy ? proxyIds[spec.proxy] : null,
      tagIds: (spec.tags ?? []).map((tag) => tagIds[tag]).sort(),
      fingerprintSeed: spec.seed === undefined ? randomSeed(rng) : spec.seed,
      fpPlatform: spec.platform ?? null,
      fpPlatformVersion: spec.platformVersion ?? null,
      fpBrand: spec.brand ?? null,
      fpBrandVersion: spec.brandVersion ?? null,
      fpHardwareConcurrency: spec.cores ?? null,
      fpGpuVendor: spec.gpuVendor ?? null,
      fpGpuRenderer: spec.gpuRenderer ?? null,
      disableGpuFingerprint: spec.disableGpuFingerprint ?? false,
      disableSpoofing: spec.disableSpoofing ?? [],
      disableNonProxiedUdp: spec.udp ?? true,
      lang: spec.lang ?? null,
      acceptLang: spec.acceptLang ?? null,
      timezone: spec.timezone ?? null,
      headless: spec.headless ?? false,
      userDataDir: spec.userDataDir ?? null,
      startUrls: spec.startUrls ?? [],
      extraArgs: spec.extraArgs ?? null,
      lastLaunchedAt:
        spec.launchedHoursAgo === undefined ? null : at(SEED_NOW_HOURS - spec.launchedHoursAgo),
      deletedAt:
        spec.trashedHoursAgo === undefined ? null : at(SEED_NOW_HOURS - spec.trashedHoursAgo),
      createdAt: at(createdHours),
      updatedAt: at(createdHours + 2),
    };
  });

  return { kernels, groups, tags, proxies, profiles, running, failures, proxyChecks };
}
