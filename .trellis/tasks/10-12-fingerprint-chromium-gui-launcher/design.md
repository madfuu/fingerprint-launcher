# Design — fingerprint-chromium GUI 启动器

> 技术设计。需求与验收见 `prd.md`；研究依据见 `research/`。

## 1. 架构总览

```
┌─────────────────────────────────────────────────────────────┐
│  React 19 渲染进程（WebView）                                 │
│                                                             │
│  features/profiles      features/profile-editor             │
│  features/groups        features/proxies    features/settings│
│         │                        │                          │
│         └────────► src/lib/bridge.ts ◄───────┘               │
│                     （唯一原生调用收口）                       │
└──────────────────────────┬──────────────────────────────────┘
                   invoke  │  emit（事件）
┌──────────────────────────┴──────────────────────────────────┐
│  Rust 主进程（Tauri 2）                                       │
│                                                             │
│  commands/         命令层：参数校验、DTO ↔ 领域模型             │
│  services/         profiles / groups / tags / proxies /      │
│                    kernels / settings / launcher            │
│  kernel/args.rs    ★ 命令行构建 + 版本门控（纯函数）            │
│  kernel/probe.rs   内核版本探测（分平台）                      │
│  kernel/process/   进程生命周期：unix.rs / windows.rs          │
│  kernel/lock.rs    单实例锁检测                               │
│  db/               rusqlite + 手写 migration                 │
│  state.rs          AppState：DB 连接 + 运行态内存表            │
└──────────────────────────┬──────────────────────────────────┘
                           │ spawn
                    ┌──────┴───────┐
                    │ chrome 子进程 │ ×N
                    └──────────────┘
```

### 1.1 关键边界决策

**决策 1：命令行在 Rust 侧构建，前端经 `previewCommand` 取预览。**

| 做法 | 问题 |
|---|---|
| 前端 TS 构建，启动时把 args 传给 Rust | Rust 失去对参数的控制；版本门控在前端，难测 |
| 两侧各写一份 | 必然漂移，A3/A4 会在两份实现间来回打补丁 |
| **Rust 单一实现 + `previewCommand`** | 多一次本地 IPC，可忽略 |

参数构建是本项目唯一真正复杂的业务逻辑（R2.2 版本门控），做成无 IO 的纯函数后可用 `cargo test` 完整验收。前端表单 debounce 300ms 调 `previewCommand(draft)`。可复制命令行的 shell 转义也在 Rust 侧完成（§3.4），前端只负责显示。

**决策 2：运行态不落库。** `profiles` 表只存配置；运行中的 PID 等存在 `AppState` 内存表。进程状态在应用重启后必然失效，落库只会产生需要清理的脏数据。

**决策 3：不采用 Simprint 的 slotkit 插件架构。** 那是为它的插件生态服务的；本项目用普通 `src/features/*` 目录组织。

**决策 4：所有原生调用收口到 `src/lib/bridge.ts`。** 前端其他地方不得直接 `import { invoke } from '@tauri-apps/api/core'`。好处：(a) 命令契约类型集中一处；(b) 可整体替换为内存 mock，使前端能脱离 Tauri 在普通浏览器中运行——这是 §10 截图验证的前提；(c) 外壳若要更换，只改此文件。

**决策 5：视觉复刻，代码自写（用户已确认）。** Simprint 为 AGPLv3。本项目以其为**视觉参照**，不拷贝其源文件与资源，许可证自选（默认 MIT）。shadcn/ui 组件用 CLI 生成（MIT）。详见 §7、§9。

## 2. 数据层

### 2.1 存储位置

```
<app_data_dir>/
  launcher.db                 # SQLite（WAL）
  profiles/<profile_id>/      # 默认 user-data-dir 根，可在设置中改
  logs/profiles/<id>.log      # chrome stderr 日志（§4.4）
```

`rusqlite` 开 `bundled` feature（自带 SQLite，三平台一致，不依赖系统库）。手写版本化 migration，不引 ORM。

### 2.2 Schema（v1）

```sql
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);  -- schema_version

CREATE TABLE kernels (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  exec_path     TEXT NOT NULL UNIQUE,
  major_version INTEGER NOT NULL,     -- 决定参数集（§3.2）
  full_version  TEXT,                 -- 仅展示，如 150.0.7871.186
  is_default    INTEGER NOT NULL DEFAULT 0,
  created_at    TEXT NOT NULL
);

CREATE TABLE groups (
  id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE COLLATE NOCASE, color TEXT,
  sort_order INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL
);

CREATE TABLE tags (
  id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE COLLATE NOCASE, color TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE proxies (
  id         TEXT PRIMARY KEY,
  name       TEXT,
  scheme     TEXT NOT NULL CHECK (scheme IN ('http','socks5')),
  host       TEXT NOT NULL,
  port       INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
  note       TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE profiles (
  id        TEXT PRIMARY KEY,
  name      TEXT NOT NULL,
  note      TEXT,
  group_id  TEXT REFERENCES groups(id)  ON DELETE SET NULL,
  kernel_id TEXT REFERENCES kernels(id) ON DELETE SET NULL,  -- NULL = 用默认内核
  proxy_id  TEXT REFERENCES proxies(id) ON DELETE SET NULL,

  -- 指纹（NULL = 不传该参数）
  fingerprint_seed        INTEGER,
  fp_platform             TEXT CHECK (fp_platform IN ('windows','linux','macos')),
  fp_platform_version     TEXT,
  fp_brand                TEXT,
  fp_brand_version        TEXT,
  fp_hardware_concurrency INTEGER,

  -- 版本相关（§3.2 门控）
  fp_gpu_vendor           TEXT,     -- 139–143
  fp_gpu_renderer         TEXT,     -- 139–143
  disable_gpu_fingerprint INTEGER NOT NULL DEFAULT 0,  -- 142–143
  disable_spoofing        TEXT,     -- 144+，CSV

  disable_non_proxied_udp INTEGER NOT NULL DEFAULT 1,
  lang          TEXT,
  accept_lang   TEXT,
  timezone      TEXT,
  headless      INTEGER NOT NULL DEFAULT 0,

  user_data_dir TEXT,               -- NULL = <user_data_root>/<id>
  start_urls    TEXT,               -- JSON 数组
  extra_args    TEXT,               -- 原样追加

  last_launched_at TEXT,
  deleted_at       TEXT,            -- 软删除（回收站）
  created_at       TEXT NOT NULL,
  updated_at       TEXT NOT NULL
);
CREATE INDEX idx_profiles_group   ON profiles(group_id);
CREATE INDEX idx_profiles_deleted ON profiles(deleted_at);

CREATE TABLE profile_tags (
  profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
  tag_id     TEXT NOT NULL REFERENCES tags(id)     ON DELETE CASCADE,
  PRIMARY KEY (profile_id, tag_id)
);

CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
```

- `PRAGMA foreign_keys = ON` 每次连接都要开（SQLite 默认关闭，否则 `ON DELETE` 不生效）。
- `settings` 键：`user_data_root`、`locale`（默认 `zh-CN`）、`theme`（默认 `system`）、`batch_concurrency`（默认 3）、`batch_interval_ms`（默认 800）、`sidebar_collapsed`（默认 `true`，同 Simprint）。
- ID 用 UUID v4 字符串；时间戳用 RFC 3339 文本。

## 3. 参数构建（核心）

依据：`research/fingerprint-chromium-cli.md`。

### 3.1 签名

```rust
// src-tauri/src/kernel/args.rs
pub struct LaunchPlan {
    pub program: PathBuf,
    pub args: Vec<String>,   // 直接作为 argv，不含 shell 引号
    pub shell_line: String,  // 仅供展示/复制，按宿主平台的 shell 转义
}

/// 纯函数：无 IO、无全局状态。
pub fn build_launch_plan(
    profile: &ProfileDraft,   // db::profiles::ProfileDraft（P1 定名，与前端 bridge 同名）
    kernel: &Kernel,
    proxy: Option<&Proxy>,
    user_data_dir: &Path,
    host: HostPlatform,      // 显式传入，便于在 Linux 上测试 Windows 转义
) -> AppResult<LaunchPlan>;   // 额外参数非法或路径非 UTF-8 时 invalid_input
```

### 3.2 版本门控

建模成能力集，不散落 `if major >= 144`：

```rust
pub struct ArgSupport {
    pub brand:                   bool,  // >= 131
    pub gpu_vendor_renderer:     bool,  // 139..=143
    pub disable_gpu_fingerprint: bool,  // 142..=143
    pub disable_spoofing:        bool,  // >= 144
}
impl ArgSupport {
    pub fn for_major(major: u32) -> Self { /* 按上述区间 */ }
}
```

- `build_launch_plan` 对当前版本不支持的字段**跳过不生成**。
- `validate_profile_for_kernel` 单独负责产出「这些字段在该版本上不会生效」的警告给 UI。构建与校验分开：构建只管正确，校验只管提示。
- 同一个 `ArgSupport` 经 `getArgSupport(kernelId)` 交给前端驱动字段显隐。前端**不自己比较版本号**。
- 无可用内核（未配置）时，前端按最新版本（`>= 144`）的能力集显示表单。

### 3.3 生成顺序

固定顺序（可复现，便于断言与 diff）：

1. `--user-data-dir=<path>`
2. `--fingerprint=<seed>`；seed 为 NULL 时第 3、4 步整段不生成
3. `--fingerprint-platform` / `-platform-version` / `-brand` / `-brand-version` / `-hardware-concurrency`
4. GPU 段：按 `ArgSupport` 生成 `--fingerprint-gpu-vendor` / `-renderer` / `--disable-gpu-fingerprint` / `--disable-spoofing=<csv>`
5. `--disable-non-proxied-udp`
6. `--lang` / `--accept-lang` / `--timezone`
7. `--proxy-server=<scheme>://<host>:<port>`
8. `--headless`
9. `extra_args`：用自写的可移植拆分规则（各 OS 一致）——空白分隔（含全角空格等 Unicode 空白）；拒绝 NUL；中文输入法的弯引号 `“ ” ‘ ’ ＂ ＇` 在直引号外直接报错并提示改用直引号；单 / 双引号分组并与相邻文本拼接；**反斜杠一律按字面保留**（Windows 路径可直接粘贴）；无注释、无展开；引号不配对 → `invalid_input`。不允许设置 `--user-data-dir`（含 `-user-data-dir`、`/user-data-dir`，大小写不敏感），否则会绕过单实例锁检测。保存与构建都执行同一校验。追加在生成参数之后，用户有最终覆盖权
10. 启动 URL（位置参数，最末）。Chromium 会把任意位置上形似开关的参数当作开关，所以启动 URL 同样不得设置 `--user-data-dir`，保存与构建时都校验

值一律 `--key=value` 形式，**不加引号**（argv 直传，不经 shell）。

### 3.4 展示用命令行

`shell_line` 按 `host` 转义。目标 shell：Unix 为 sh / bash / zsh；Windows 为 **cmd.exe**（PowerShell 不是目标：带引号的程序路径需要 `& `，且 `$` 会展开）。cmd 中引号状态失衡时对 `& | < > ^` 加 `^` 转义；zsh 会展开以 `=` 开头的裸词，此类值也加引号；cmd 在引号内仍会展开 `%name%`，无法转义，文档注明即可：

- POSIX：值只含 `[A-Za-z0-9_@%+=:,./-]` 时原样，否则单引号包裹，内部 `'` 写作 `'\''`
- Windows：含空格或 `"` 时双引号包裹，内部 `"` 转义为 `\"`（`CommandLineToArgvW` 规则）

### 3.5 测试

- README 的两个官方示例做 fixture（argv 断言 + `shell_line` 断言）
- 版本门控边界：major = 130 / 131 / 138 / 139 / 141 / 142 / 143 / 144 / 150，各断言一次参数集
- seed 为 NULL、全字段为空、`extra_args` 含引号与空格、`shell_line` 的 POSIX / Windows 转义

## 4. 进程管理

依据：`research/process-lifecycle.md`。

### 4.1 运行态

```rust
pub enum ProfileStatus { Stopped, Starting, Running, Stopping, Error }

struct RunningProc {
    pid: u32,
    started_at: DateTime<Utc>,
    kernel_id: String,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,  // 最后 50 行
    stop_tx: Option<oneshot::Sender<StopMode>>, // 通知 waiter 进入停止流程
    #[cfg(windows)] job: JobHandle,
}

pub struct AppState {
    db: Arc<Mutex<rusqlite::Connection>>,   // 单用户桌面，无需连接池
    running: Arc<tokio::sync::Mutex<HashMap<ProfileId, RunningProc>>>,
    exiting: AtomicBool,                     // 退出流程防重入
}
```

- 所有命令声明为 `async`：非 async 的 Tauri 命令在主线程执行，会卡 UI。
- DB 调用包在 `tokio::task::spawn_blocking` 内，不在 async 上下文中持锁做 IO。
- 启动器服务不直接依赖 `AppHandle`：状态事件经 `trait StatusSink { fn status_changed(&self, ev: StatusEvent); fn batch_progress(&self, p: BatchProgress) }` 发出（P3 落定；在持有运行表锁时调用，以保证事件顺序与运行表一致，实现必须立即返回、不得回调启动器）。生产实现转发为 Tauri 事件，测试实现写入 channel——这样进程生命周期可以脱离 Tauri 运行时做集成测试（§10）。宽限期等时间常量经 `LauncherConfig` 注入，测试中可调短。

### 4.2 启动流程

1. **校验**：profile 存在且未删除；不在 `running` 表中（R3.4）；内核路径存在且可执行（R3.6）
2. **单实例锁检查**：`kernel/lock.rs` 检查 user-data-dir 是否已被外部实例占用（Unix 读 `SingletonLock` 链接并探活 pid；Windows 尝试独占打开 `lockfile`）。被占用则拒绝，错误码 `ProfileRunningExternally`。不做这一步，Chromium 的单实例转交会让新进程秒退，状态错乱（research §5）
3. 创建 user-data-dir（不存在则 `create_dir_all`）
4. `build_launch_plan` → `tokio::process::Command`：`stdin(null)`、`stdout(null)`、`stderr(piped)`；Unix 加 `process_group(0)`
5. `spawn()`；Windows 立即 `AssignProcessToJobObject`
6. 写入 `running`，状态 `Starting`，更新 `last_launched_at`
7. 起 **stderr 读取协程**：逐行读入环形缓冲区（50 行）并追加到 `logs/profiles/<id>.log`，直到 EOF。必须持续读空，否则管道写满后 chrome 阻塞
8. 起 **waiter 协程**：`select!` 等待 `child.wait()` 或 `stop_rx`

waiter 的状态推进：

| 事件 | 结果 |
|---|---|
| 启动后 `STARTUP_GRACE`（3 s）内未退出 | `Starting → Running` |
| `STARTUP_GRACE` 内退出（任意退出码） | `Error`，消息附 stderr 末尾若干行 |
| `EARLY_EXIT_WINDOW`（10 s）内以非 0 退出 | `Error`，同上 |
| 其他时刻退出 | `Stopped`（用户手动关窗，R3.3） |
| 收到 `stop_rx` | 进入 §4.3 |

退出后从 `running` 移除；每次状态变化 emit `profile://status-changed`。R3.3 由 `wait()` 直接驱动，不轮询。

**错误识别**：stderr 中出现 `No usable sandbox` 时，错误信息追加建议「在额外参数中加入 `--no-sandbox`」；出现 `libfuse.so.2` 时提示安装 libfuse2 或改用 tar.xz 版。

### 4.3 停止流程：先温和、后强制

直接硬杀会让 Chrome 来不及把 Cookie 刷盘，丢登录态。

| 平台 | 温和 | 宽限 | 强制 |
|---|---|---|---|
| Unix | `kill(pid, SIGTERM)`，只发主进程 | `STOP_GRACE` 5 s | `killpg(pgid, SIGKILL)` |
| Windows | 向主进程顶层窗口投递 `WM_CLOSE` | `STOP_GRACE` 5 s | `TerminateJobObject` |

状态 `Running → Stopping → Stopped`。所有状态变更只经 waiter 一条路径。宽限期常量集中定义，便于真机调参。

### 4.4 三平台进程树（R10.2）

| 平台 | 手段 |
|---|---|
| Windows | `CreateJobObjectW` + `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`。启动器退出或崩溃时句柄关闭，整树终止 |
| Linux / macOS | `process_group(0)` 使 chrome 成为新进程组组长；强制阶段 `killpg` |

Simprint 的 Unix 侧是 no-op（`job.rs`），本项目不照搬——否则 A6 在 Linux 上过不了。Unix 下启动器崩溃会留孤儿，由 §4.2 第 2 步的锁检查兜底。

依赖（P3 落定）：Unix `nix`（`signal`、`hostname` features；`signal` 隐含 `process`）；Windows `windows-sys` 0.61（`Win32_Foundation`、`Win32_Security`、`Win32_System_JobObjects`、`Win32_System_Threading`）。`Win32_Security` 不能省：`CreateJobObjectW` 的参数类型 `SECURITY_ATTRIBUTES` 在该 feature 下，缺了它函数根本不会生成；`JOBOBJECT_EXTENDED_LIMIT_INFORMATION` 由 `Win32_System_Threading` 门控。温和停止用 `taskkill /PID <pid>`（不带 `/F`，即向其窗口投递 `WM_CLOSE`，`CREATE_NO_WINDOW` 启动），版本探测为可移植的 PE / plist 解析，故不需要 `Win32_UI_WindowsAndMessaging`、`Win32_Storage_FileSystem`。

### 4.5 应用退出（R3.5 / A7）

P4 落定（`src-tauri/src/shell.rs`，依据 tauri 2.12.3 / tao 0.37.1 源码）：

1. **主窗口关闭**（自绘标题栏的关闭按钮 → `getCurrentWindow().close()`、Alt+F4、窗口管理器）：`WindowEvent::CloseRequested` 中若运行表（含启动中）非空 → `api.prevent_close()` → emit `app://close-requested { runningCount }`。这是**唯一**的确认点。前端尚未调用过 `showMainWindow`（前端坏了、无人应答）时不弹框，直接按「已确认」走 `quitApp` 流程，避免窗口永远关不掉
2. **`RunEvent::ExitRequested`（无退出码）不拦截**：它只在最后一个窗口已销毁之后才触发，此时已无处弹框；拦截只会留下一个无窗口的进程。这样一次关闭最多一个确认框
3. **macOS Cmd+Q / Dock「退出」**：tao 0.37 直接 `terminate:`，既无 `CloseRequested` 也无 `ExitRequested`，只有 `RunEvent::Exit`，无法拦截确认
4. **兜底**：所有路径最终都经过 `RunEvent::Exit`，在其中同步执行 `launcher.shutdown()`（温和关闭 → 宽限 → 整树强杀，上限约为 `STOP_GRACE`），保证 A7。Windows 上 Job Object 的 kill-on-close 另行兜底

前端确认后调 `quitApp()`：置 `exiting` → `launcher.shutdown()`（并发停止全部，§4.3，拒绝新启动）→ `app.exit(0)`（`ExitRequested` 带退出码，放行）。`exiting` 为真时再次关闭窗口被静默阻止、再次调用 `quitApp` 立即返回，防重入。运行表为空时不拦截，直接退出。

确认框打开期间用户再次点关闭（对话框遮罩不盖标题栏，关闭按钮仍可点），Rust 会再发一次 `app://close-requested`：前端只更新已打开确认框中的数量，不叠加第二个确认框（Rust 侧不知道确认框何时被取消，去重放在前端）。

### 4.6 批量启动（R4.2 / A8）

- `tokio::sync::Semaphore(batch_concurrency)` 限制**同时处于 `Starting` 的数量**；一个启动在离开 `Starting`（转 `Running` 或 `Error`）时释放许可
- 每次拿到许可后先 `sleep(batch_interval_ms)` 再 spawn
- 按选中顺序依次进行；单个失败不中断批次
- emit `batch://progress { total, launched, failed }`

## 5. 命令契约

`bridge.ts` 与 Rust `commands/` 一一对应。错误统一为 `{ code: string; message: string }`（TS 类型 `AppError`），`code` 供前端选 i18n 文案，`message` 为原始细节。例外：参数反序列化失败（`InvalidArgs`）与 ACL 拒绝由 Tauri 自身以**纯字符串**拒绝（tauri 2.12 `ipc/mod.rs` `From<crate::Error> for InvokeError`），`bridge.ts` 须把非 `AppError` 的拒绝统一包装成 `AppError`（如 `invalid_input` + 原文），调用方只处理一种形状。

**类型来源（P4 落定）**：下列签名中的所有 DTO、枚举、事件载荷与 `AppError` 都由 Rust 定义经 ts-rs 生成到 `src/lib/bindings/`（`src-tauri/src/bindings.rs`，`cargo test` 时导出；`index.ts` 汇总再导出，`events.ts` 含事件名常量 `EVENTS` 与 `EventPayloads` 映射）。`bridge.ts` 只引用生成的类型，不手写 DTO；CI 校验生成结果与提交一致。命令参数名为 camelCase（Tauri 默认）。

```ts
// profiles
listProfiles(query: ProfileQuery): Promise<Page<ProfileRow>>
getProfile(id): Promise<Profile>
createProfile(draft: ProfileDraft): Promise<Profile>
updateProfile(id, draft: ProfileDraft): Promise<Profile>
deleteProfile(id) / restoreProfile(id): Promise<void>      // 软删除 / 恢复；运行中删除 → profile_running
purgeProfile(id, dropUserData: boolean): Promise<PurgeResult>   // 运行中 → profile_running
duplicateProfile(id): Promise<Profile>   // 新 id、新种子（原无种子则仍无）、默认目录、名称加「副本」/「(copy)」（按界面语言）
exportProfiles(ids, path): Promise<number>      // 写入用户在保存对话框中选的路径（原子替换），返回条数
importProfiles(path): Promise<ImportResult>     // 全部成功或全部回滚

// 启动
previewCommand(draft: ProfileDraft, profileId: string | null): Promise<LaunchPlan>
getArgSupport(kernelId: string | null): Promise<ArgSupport>       // null = 默认内核
validateProfileForKernel(draft: ProfileDraft): Promise<FieldWarning[]>
launchProfile(id): Promise<void>         // 进程已存在（Starting）即返回，后续状态走事件
stopProfile(id): Promise<void>           // 停止完成才返回（≤ 宽限期 + 强杀）
listRunning(): Promise<RunningInfo[]>
batchLaunch(ids): Promise<BatchProgress> // 并发与间隔取设置；整批结束返回最终计数，过程走 batch://progress
batchStop(ids): Promise<void>
quitApp(): Promise<void>

// 内核
listKernels(): Promise<Kernel[]>
addKernel(path, name: string | null, majorVersion: number | null): Promise<Kernel>
updateKernel(id, input: KernelInput): Promise<Kernel>
removeKernel(id) / setDefaultKernel(id): Promise<void>
detectKernels(): Promise<KernelCandidate[]>
probeKernel(path): Promise<KernelProbe>   // { majorVersion, fullVersion, method: 'pe_version'|'info_plist'|'file_name'|'version_flag' }，找不到时全为 null；path 须为绝对路径（裸名会经 PATH 查找并被执行），否则 invalid_input

// 代理 / 分组 / 标签 / 设置 / 应用
listProxies(): Promise<Proxy[]> / createProxy(input: ProxyInput) / updateProxy(id, input) / deleteProxy(id)
testProxy(id): Promise<ProxyCheck>
listGroups(): Promise<Group[]>  / createGroup(input: GroupInput) / updateGroup(id, input) / deleteGroup(id)
listTags(): Promise<Tag[]>      / createTag(input: TagInput)     / updateTag(id, input)   / deleteTag(id)
setProfileTags(profileId, tagIds): Promise<void>
batchSetGroup(ids, groupId: string | null) / batchAddTags(ids, tagIds): Promise<void>
getSettings(): Promise<Settings> / updateSettings(patch: SettingsPatch): Promise<Settings>
getAppInfo(): Promise<AppInfo>           // { version, os: 'windows'|'macos'|'linux', dataDir, defaultUserDataRoot, logDir }
showMainWindow(): Promise<void>          // 前端就绪后显示窗口
```

P4 相对原契约的改动：`previewCommand` 第二参数由 `kernelId` 改为 `profileId`（内核一律取 `draft.kernelId`，为 null 用默认内核；`profileId` 用于算出已存在 profile 的真实默认目录）；`validateProfileForKernel` 去掉 `kernelId`（同上取 `draft.kernelId`）；`purgeProfile` 返回 `PurgeResult`；`batchLaunch` 返回最终 `BatchProgress`；新增 `getAppInfo`（macOS 标题栏判断、设置页显示默认数据根目录）。

事件（Rust → 前端；名称见生成的 `EVENTS`）：

```ts
'profile://status-changed'  StatusEvent    { profileId, status, pid: number | null, error: LaunchFailure | null }
                            // status: stopped|starting|running|stopping|error
                            // error: { code, message, hint: 'no_sandbox'|'missing_fuse'|null, exitCode, stderrTail }
'batch://progress'          BatchProgress  { total, launched, failed, skipped }  // skipped：已在运行，或因退出而未尝试
'app://close-requested'     CloseRequested { runningCount }
```

细则：

- `ProfileQuery`：`page / pageSize / view('all'|'running'|'trash') / groupId? / tagIds? / keyword?`，分页在 SQL 侧做。`view = running` 时以内存运行表的 id 集合过滤。
- P1 落定的查询语义：列表按创建时间倒序，回收站按删除时间倒序；标签过滤为「命中任一所选标签」；关键字匹配名称、备注、种子（`%`、`_`、`\` 按字面匹配）；`view = running` 由调用方传入运行中 id 集合，且排除已删除。
- 错误码（P1 落定）：`not_found`、`conflict`（唯一约束）、`invalid_input`（外键 / CHECK / 非空 / 校验失败）、`db`、`io`。P3/P4 新增的错误码在此追加。
- 错误码（P3 新增）：`no_kernel`（profile 无内核且无默认内核）、`kernel_missing`（可执行文件不存在）、`kernel_not_executable`（不是文件 / Unix 缺执行位）、`profile_running`（已在本启动器中运行或启动中）、`profile_running_externally`（user-data-dir 被启动器外的浏览器持有）、`spawn_failed`（操作系统拒绝启动）、`startup_failed`（启动期内退出，仅出现在状态事件的 `error` 中）、`shutting_down`（退出流程中拒绝新启动）。两个运行中 profile 共用同一 user-data-dir 时启动返回 `conflict`。启动前失败（`profile_running`、`shutting_down` 除外）同时发出 `error` 状态事件。
- `ProfileDraft` 含 `tagIds`，编辑器一次保存即可写入标签；`setProfileTags` 保留给列表页。
- `probeKernel` 按文件类型（而非宿主平台）选择方法（research §7；P3 落定，全部为可移植代码，三平台都能测）：PE 文件读版本资源（`chrome.exe --version` 在 Windows 上不输出，PE 从不执行）；`.app` 包或其 `Contents/MacOS/*` 读 `Info.plist`；`*.AppImage` 先解析文件名，再试 `--version`；其他执行 `--version`（10 s 超时）。全部失败返回 `null`，UI 要求用户手填大版本号，**不猜**。
- 错误码（P4 新增）：`kernel_version_unknown`（`addKernel` 探测不到版本且未手填大版本号，UI 据此要求手填）、`proxy_test_failed`（两个查询服务都失败，`message` 含两者原因）。
- **user-data-dir 归属（P4 落定）**：保存（新建 / 更新 / 复制 / 导入）时，该 profile 的实际目录（自定义目录，或 `<root>/<id>`）不得与其他任何 profile（含回收站中的）的实际目录相同、互为包含；否则 `conflict`。比较前做词法规范化（`.`、`..`、末尾分隔符），Windows 与 macOS 上忽略大小写（NTFS / APFS 默认不区分）。自定义目录也不能是数据根目录本身或其上级（它会包含现在与将来的全部默认目录），即使暂时没有使用默认目录的 profile 也拒绝，同为 `conflict`。修改设置中的数据根目录时不做这项检查（判断留给 P9 的设置页）。
- `purgeProfile(dropUserData=true)` **只删除严格位于 `user_data_root` 之下的目录**：比较的是 `canonicalize` 后的真实路径（`..`、中间的符号链接、macOS `/var`→`/private/var` 都不能绕过），目录本身是符号链接、等于根目录、或与其他 profile 的目录重叠时一律保留。用户自定义到其他位置的目录永不由程序删除。`PurgeResult = { userDataDir, dataDir: 'removed'|'not_requested'|'missing'|'outside_root'|'shared' }`，UI 据此提示保留的路径。先删目录（失败则返回 `io`、记录保留以便重试），再删记录；同时删除该 profile 的 stderr 日志。
- `testProxy`：经该代理请求 `https://ipinfo.io/json`（失败回退 `http://ip-api.com/json`），每次请求 10 s 超时（含读响应体），响应体最多 64 KiB、不跟随重定向（回退服务是经用户代理的明文 HTTP），回显出口 IP / 国家代码（ISO 3166-1）/ 城市 / 时区 / 耗时 / 所用服务。SOCKS5 用 `socks5h`（经代理解析域名，与 Chromium 的 `--proxy-server=socks5://` 一致）；IPv6 主机加方括号。HTTP 客户端为 reqwest（`rustls-no-provider` + ring，不依赖 OpenSSL / aws-lc）。仅用户点击时触发。
- `detectKernels`：research §8 的候选位置，只列出、不自动添加；每个候选并行探测（最多 4 个同时，`--version` 3 s 超时），`registered` 标出已登记的路径。Linux 的 AppImage 候选要求文件名含 `chromium`（探测会执行它，不能误启动无关的 AppImage）。
- `addKernel`：选中 `.app` 时解析为其中的可执行文件；检查存在且可执行；探测版本（手填的大版本号优先，探测到的完整版本号仅在大版本一致时保留）；名称缺省为 `Chromium <版本>`。`updateKernel` 只在路径变化时重新检查路径。
- 导出 JSON：`{ formatVersion: 1, exportedAt, profiles: [...] }`；每个 profile 含 `ProfileDraft` 中除 `groupId / kernelId / proxyId / tagIds / userDataDir` 外的全部字段，外加 `group: { name, color }`、`tags: [{ name, color }]`、`proxy: { scheme, host, port, name, note }`。不含 id、user-data-dir 与内核绑定（机器相关；导入后用默认内核）。导入时按名称（ASCII 大小写不敏感）复用或新建分组/标签，按 `scheme/host/port` 复用或新建代理，profile 一律新建；更高的 `formatVersion` 明确拒绝；文件上限 32 MiB、最多 10 000 条 profile（解析时逐条计数即拒绝，不先构建整棵 JSON 树——`{}` 这样的最短条目解析后会膨胀百倍以上）；整个导入在一个事务内，任一条失败（错误信息带序号与名称）则全部回滚。

## 6. 前端结构

```
src/
  main.tsx
  index.css                      # 主题变量（§7）
  app/
    app.tsx                      # Provider 栈 + 启动引导（读设置 → i18n/主题 → showMainWindow）
    routes.tsx
    layout/
      app-layout.tsx  app-titlebar.tsx  app-sidebar.tsx  app-background.tsx
  components/
    ui/                          # shadcn CLI 生成（new-york, neutral）
    data-table/                  # 密集表格 + 骨架 + 粘性列
    theme-provider.tsx
  features/
    profiles/  profile-editor/  groups/  proxies/  settings/
      ├ api.ts        # 调 bridge
      ├ components/
      ├ hooks/
      ├ store.ts      # zustand：筛选 / 多选 / 对话框
      ├ types.ts
      └ i18n.ts       # 本 feature 的 zh-CN + en
  lib/
    bridge.ts                    # ★ 唯一 invoke 收口
    bridge.mock.ts               # 内存 mock，仅 dev 构建加载
    bindings/                    # Rust 生成的 DTO / 事件类型（ts-rs，§5），勿手改
    utils.ts                     # cn()
  i18n/
    index.ts  registry.ts
```

| 路径 | 页面 |
|---|---|
| `/` | 配置列表（全部 / 运行中 / 回收站） |
| `/profiles/new`、`/profiles/:id/edit` | 编辑器 |
| `/groups` | 分组与标签 |
| `/proxies` | 代理中心 |
| `/settings` | 设置（含内核管理） |

- 路由：react-router 7 `BrowserRouter`
- 表单：react-hook-form + zod（zod 只做格式校验；版本相关的校验以 Rust `validateProfileForKernel` 为准）
- 文件/目录选择、保存对话框：`tauri-plugin-dialog`；复制命令行：`tauri-plugin-clipboard-manager`（WebKitGTK 下 `navigator.clipboard` 不可靠）
- 状态：服务端数据用 `use*` hooks + 手动 `refresh()`；UI 状态用 zustand；运行态单独一个 zustand store，启动时 `listRunning()` 再订阅 `profile://status-changed` 增量更新
- 通知：sonner
- **bridge 运行模式**：检测 `window.__TAURI_INTERNALS__`；不存在且 `import.meta.env.DEV` 时加载 `bridge.mock.ts`（带种子数据，模拟启动/停止状态流转）。生产构建中 mock 被摇树移除。
- **启动不闪烁**：窗口以 `visible(false)` 创建，前端读完设置、初始化 i18n 与主题后调 `showMainWindow()`。主题与语言存 `settings` 表，单一来源。兜底（P4）：窗口创建 5 s 后前端仍未调用，则 Rust 侧自行显示窗口，前端出错时也不会留下不可见的进程。环境变量 `FPL_SMOKE_EXIT_ON_READY=1` 时，`showMainWindow` 被调用即 `app.exit(0)`，供 CI 冒烟验证「WebView 加载前端 + 正式 CSP 下完成一次 IPC」。
- **DTO 类型**：只从 `src/lib/bindings`（Rust 生成，见 §5）导入，不手写。

## 7. 外观复刻（R8）

逐项样式依据：`research/simprint-ui-reference.md`。

**做法：以 Simprint 为视觉参照，代码自写。**

- shadcn/ui 组件用 `npx shadcn@latest add ...` 生成（new-york、neutral、lucide），不从 Simprint 拷贝
- 主题变量按参考文档 §1.1 的色值自行编写 `index.css`
- titlebar / sidebar / background / data-table / 列表页 / 编辑器页按参考文档描述的结构、尺寸、类名效果自行实现
- 图标统一用 lucide（Simprint 用到的 react-icons 换 lucide 等价图标）
- 不打包任何字体文件，按名称引用系统字体（Simprint 打包的 PingFang SC 是 Apple 专有字体，不可再分发）
- 自绘 logo（SVG），经 `tauri icon` 生成各尺寸图标

**改进 Simprint 的已知缺陷**（参考文档 §7.2）：运行中行底色、进度遮罩改用主题无关的半透明色；图标不用未加载的 Font Awesome；文案全部走 i18n；列表页不写死 `calc(100vh-50px)`；背景 canvas 的 resize 重绘加 debounce。

**平台差异**：

- Linux / Windows：`decorations(false)`，自绘最小化 / 最大化 / 关闭
- macOS：`title_bar_style(Overlay)` + `hidden_title(true)` 保留原生红绿灯，隐藏自绘按钮，品牌区左侧留约 78px

窗口：`inner_size(1438, 828)`、`min_inner_size(1038, 688)`、居中、`visible(false)`、`disable_drag_drop_handler()`（Tauri 2.12 中 `drag_and_drop(false)` 仅 Windows 可用，跨平台用前者）；对话框遮罩从 `top: 48px` 起，不盖住标题栏。

安全（P4 落定）：

- 能力（`capabilities/default.json`，仅 `main` 窗口）只开用到的：`core:window:allow-minimize` / `allow-toggle-maximize` / `allow-close` / `allow-start-dragging` / `allow-internal-toggle-maximize`（`data-tauri-drag-region` 双击最大化走这个内部命令）/ `allow-show`，`core:event:allow-listen` / `allow-unlisten`，`dialog:allow-open` / `allow-save`，`clipboard-manager:allow-write-text`。应用自身的命令在未定义 app ACL manifest 时对本地来源默认放行（tauri 2.12 `webview/mod.rs`）
- CSP：`default-src 'self'; connect-src 'self' ipc: http://ipc.localhost; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; object-src 'none'; base-uri 'self'; form-action 'none'`。IPC 在 Linux / macOS 走 `ipc://localhost`，Windows 走 `http://ipc.localhost`。`dangerousDisableAssetCspModification: ["style-src"]`：否则 Tauri 会给 `style-src` 注入 nonce / hash，使 `'unsafe-inline'` 失效，Radix / sonner 运行时插入的 `<style>` 被拦。`script-src` 仍由 Tauri 管理。桌面端 dev 模式直接加载 devUrl，CSP 不生效，故未设 `devCsp`

## 8. 构建与工具链

- Tauri 2（脚手架时取最新 2.x，锁定于 `Cargo.lock` / `package-lock.json`）
- Vite 7（create-vite 已默认 Vite 8，此处有意固定 7）+ `@vitejs/plugin-react` 5.2 + `@tailwindcss/vite`；别名 `@` → `./src`
- 实际解析版本（P0，2026-10-12）：React 19.3、Tailwind 4.3、TypeScript 6.0（typescript-eslint 只支持 < 6.1）、ESLint 10、Tauri 2.12.3、shadcn CLI 4.21
- shadcn 4.x 生成的组件从 npm 包 `cn`（shadcn-ui 官方，MIT，无依赖）导入 `cn()`，取代 `clsx` + `tailwind-merge`；`src/lib/utils.ts` 仅做再导出
- `build.target`：Windows `chrome105`，其他 `safari13`（Linux/macOS 的 WebView 是 WebKit）
- 包管理：**npm**（本机 Node 26.8.1 / npm 11.19.0，未装 pnpm）
- Rust：stable 1.99（rustup 经 USTC 镜像安装）；crates 经 `~/.cargo/config.toml` 的 USTC sparse 镜像拉取（本机连不上 crates.io 官方源）
- 应用标识：productName `Fingerprint Launcher`，identifier `io.github.madfuu.fingerprint-launcher`（首次发布前可改；发布后改会导致数据目录迁移）
- 检查：`npm run lint`（含 `eslint-plugin-i18next` 的 `no-literal-string`，`jsx-only` 模式，属性白名单排除 `className` / `variant` / `size` / `data-*` 等非文案属性，让 A10「无硬编码文案」可机检）、`npm run typecheck`、`cargo fmt --check`、`cargo clippy -D warnings`、`cargo test`、`vitest run`
- CI：GitHub Actions（公开仓库 `madfuu/fingerprint-launcher`），矩阵 `ubuntu-22.04` / `windows-latest` / `macos-latest`，原生跑 clippy + test，再 `tauri build` 打包并上传产物；Linux 额外做 xvfb 冒烟。**CI 是权威结果**；本机只跑开发循环（typecheck / lint / vitest / Linux 上的 cargo 检查与测试），不做打包、冒烟和 Windows 交叉检查（用户决定）
- `tauri::generate_context!()` 编译期要求 `frontendDist` 存在：任何 `cargo clippy` / `cargo test` 之前先 `npm run build`
- 桥接类型（P4）：ts-rs 12（仅 dev-dependency；类型以 `#[cfg_attr(test, derive(ts_rs::TS))]` 派生，发布构建不含）。`cargo test` 写出 `src/lib/bindings/`（只改内容有变的文件、删除过期文件），整数统一映射为 `number`；该目录不经 Prettier（`.prettierignore`），但参与 `tsc` 与 ESLint。CI 的 ubuntu `check` 在 `cargo test` 之后检查该目录无改动（含新增文件）
- Rust 侧其他 P4 依赖：`tauri-plugin-dialog`、`tauri-plugin-clipboard-manager`；`reqwest` 0.13（`default-features = false`，`rustls-no-provider` + `socks`）+ `rustls`（`ring`）

## 9. 许可

- 本项目代码自写，默认 **MIT**（用户可改）
- shadcn/ui 生成的组件为 MIT，保留其声明
- 不包含 Simprint（AGPLv3）的源码与资源；README 注明视觉设计参考了 Simprint
- fingerprint-chromium（BSD-3-Clause）不随本项目分发，仅作为外部可执行文件被调用

## 10. 验证方案

开发机是无头 VM（无显示，有 Xvfb），没有 fingerprint-chromium 二进制。但本机有 **Chrome for Testing 151**（`~/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome`）：它是完整的多进程 Chromium，会忽略不认识的 `--fingerprint*` 开关，足以验证进程生命周期。

| 层 | 手段 | 覆盖验收项 |
|---|---|---|
| 参数构建 | `cargo test`（纯函数） | A3、A4 |
| 数据层 | `cargo test`（临时目录 SQLite） | A2 的持久化部分 |
| 进程生命周期（逻辑） | `cargo test` + 假内核脚本 `tests/fixtures/fake-chrome.sh`：记录 argv、派生子进程、处理 SIGTERM、可选早退并写 stderr、可选写 `SingletonLock` | A5、A6、A7、A12 的逻辑 |
| 进程生命周期（真 Chromium） | `cargo test -- --ignored`，`FPL_TEST_CHROME` 指向 Chrome for Testing，`--headless=new` 运行，验证启动、手动终止检测、进程树无残留、真实 `SingletonLock` | A5、A6、A9 |
| 前端 | `vitest`（store / 工具函数 / mock bridge） | — |
| 外观 | Vite dev + mock bridge，`playwright-core` 指定上述 Chrome 可执行文件，逐页截图（浅色/深色 × 中/英），对照 Simprint 参考文档核对 | A10、A11 的大部分 |
| 文案 | ESLint `i18next/no-literal-string` | A10 |
| 构建与打包 | CI：三平台 `tauri build` + Linux xvfb 冒烟 | A1 |
| Windows / macOS 平台代码 | CI：`windows-latest` / `macos-latest` 原生 clippy + `cargo test`（含 Windows 的 Job Object 测试、macOS 的进程组测试） | R10.1 / R10.2 |

**本机仍无法覆盖、需真机验收**：fingerprint-chromium 真实指纹参数是否生效；WebKitGTK 与 Chromium 的渲染差异（截图走 Chromium）；Windows 的 Job Object / `WM_CLOSE` / PE 版本探测；macOS 的 Overlay 标题栏与 plist 探测；三平台安装包的实际安装运行。

## 11. 兼容性与回滚

- 从零新建，无既有数据或接口需要兼容
- Schema：`meta.schema_version` + 顺序 migration；本任务只产出 v1
- 回滚单位：`implement.md` 每个阶段一个提交，阶段间耦合点只有 bridge 契约（§5）
- 外壳可换：决策 4 保证换外壳只需重写 `bridge.ts` 与主进程
