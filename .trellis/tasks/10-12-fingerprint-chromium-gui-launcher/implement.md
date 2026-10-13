# Implement — fingerprint-chromium GUI 启动器

> 执行计划。需求见 `prd.md`，设计见 `design.md`，研究依据见 `research/`。
> 每个阶段：实现 → 本机开发循环检查全绿 → trellis-check 审查 → 本地提交。**只在里程碑推送**，以该次 GitHub Actions 结果为准（见「推送与 CI」）。

## 通用约定

**环境前置**（每个 shell 都要）：

```bash
. "$HOME/.cargo/env"            # ~/.cargo/bin 不在默认 PATH
cd /home/a/projects/js/fingerprint-launcher
```

- crates 经 `~/.cargo/config.toml` 的 USTC 镜像拉取；**不要**删改该文件
- 需要 rustup 安装新组件/target 时带上：`RUSTUP_DIST_SERVER=https://mirrors.ustc.edu.cn/rust-static RUSTUP_UPDATE_ROOT=https://mirrors.ustc.edu.cn/rust-static/rustup`（官方源不可达）
- 包管理只用 **npm**；所有脚手架命令用非交互参数（`--yes` / `--ci` / `-y`）
- Simprint 参考仓库在 `/home/a/projects/simprint`，**只读**；不存在时按 `research/simprint-ui-reference.md` §0 重新拉取。**不得把其中任何文件或代码片段拷进本仓库**（design 决策 5）

### 本机与 CI 的分工（用户决定，2026-10-12）

| 在哪 | 做什么 |
|---|---|
| **本机：开发循环** | 写代码必需的快速反馈：typecheck、lint、vitest、vite build、Linux 上的 `cargo fmt` / `clippy` / `test`、真 Chromium 生命周期测试（`--ignored`）、截图核对 |
| **GitHub Actions：CI（权威）** | 三平台原生的 clippy + test、`tauri build` 打包、Linux xvfb 冒烟、产物上传 |
| **本机不再做** | Windows 交叉检查（`x86_64-pc-windows-gnu`）、`tauri build`、xvfb 冒烟 |

**本机开发循环检查**（每阶段末尾）：

```bash
npm run check     # = typecheck + lint + test(vitest) + build(vite)
(cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test)
```

注意：`tauri::generate_context!()` 在编译期要求 `frontendDist`（`../dist`）存在，所以 `cargo clippy` / `cargo test` 之前必须已执行过 `npm run build`（`npm run check` 已包含）。CI 中同理。

只适用于 Unix 的测试与夹具（如 `fake-chrome.sh`）放在 `#[cfg(unix)]` 下；Windows 专属测试放在 `#[cfg(windows)]` 下——CI 的 Windows runner 会**原生**运行 `cargo test`，因此 Windows 分支不仅被编译，还能被测试。

本机实测（2026-10-12）：8 vCPU / 15 GiB；Tauri 依赖树冷编译 2 min、增量 1 s；每个 target 目录约 3 GB。

**截图验证**（P6 起）：`npm run screenshots` 产出 `.screenshots/<route>-<variant>-<theme>-<lang>.png`（变体：`collapsed` / `expanded` / `macos` / `dialog`），由主会话读图对照 `research/simprint-ui-reference.md` 核对；`npm run ui-check` 跑截图看不出的交互断言。两者留在本机做，因为 Actions 的产物需要鉴权才能下载，本机没有 GitHub API 令牌。

**推送与 CI**：

- 远端：`git@github.com:madfuu/fingerprint-launcher.git`（公开仓库，用户已创建并添加 Deploy key）
- 凭据：仓库级 Deploy key `~/.ssh/fingerprint_launcher_deploy`，已通过本仓库 `core.sshCommand` 指定；**不要**改 `~/.ssh/config`
- **控制 Actions 用量（用户要求，2026-10-13）**：频繁触发 Actions 可能被 GitHub 判定滥用、封禁账号。因此：
  - **只在里程碑推送**：P3 完成后（第一批平台相关代码，需要 Windows / macOS 原生验证）、P6 完成后（交给用户看外观之前）、P10 之后、P11 发布前。其余阶段只做本地提交
  - CI 失败时先在本地复现、尽量一次修完再推送；同一问题的修复推送间隔不短于上一次 run 完成后的分析时间，连续失败 3 次则停止推送，记录问题后继续后续阶段（用户决定）
  - `bundle`（三平台打包，最耗时）只在推 `v*` tag、手动触发、或 head commit 信息含 `[bundle]` 时运行；需要验证打包时才在提交信息里加 `[bundle]`
  - `.trellis/`、`.claude/`、`*.md`、`LICENSE` 的改动不触发 CI（`paths-ignore`）
- 结果查询：公开仓库可匿名调用 `GET https://api.github.com/repos/madfuu/fingerprint-launcher/actions/runs?head_sha=<sha>` 取 run 状态与各 job / step 的结论（匿名限额 60 次/小时，轮询间隔 ≥ 60 s）；封装为 `~/.cache/fpl-tools/ci-status.sh <sha> [--annotations]`。CI 用带 problem matcher 的 Rust 工具链 action，让编译错误以 annotation 形式出现，可经 check-runs annotations API 匿名读取。拿不到完整日志时请用户贴出

**回滚**：每阶段一个提交。阶段内失败就地修；需要回退已推送的提交时用 `git revert`，不改写远端历史；本地 `git reset --hard` 先征得用户同意。

---

## P0 脚手架

- [x] `git init`（本目录当前不是 git 仓库）；`.gitignore`：`node_modules/`、`dist/`、`src-tauri/target/`、`src-tauri/gen/schemas/`、`.screenshots/`、`*.log`
- [x] 在临时目录用 `npm create vite@latest -- --template react-ts` 生成，再把文件移入本目录根（根目录已有 `AGENTS.md`、`.claude/`、`.trellis/`、`.gitattributes`，不得覆盖）
- [x] 依赖：React 19、react-router 7、zustand 5、i18next + react-i18next、react-hook-form + zod + @hookform/resolvers、sonner、lucide-react、class-variance-authority + `cn`（shadcn 4.x 以 `cn` 包取代 clsx + tailwind-merge）、`@tauri-apps/api`、`@tauri-apps/plugin-dialog`、`@tauri-apps/plugin-clipboard-manager`
- [x] Tailwind 4：`tailwindcss` + `@tailwindcss/vite` + `tw-animate-css`
- [x] `vite.config.ts`：别名 `@` → `./src`；`server.strictPort`、忽略 `src-tauri/**`；`build.target` 按 `TAURI_ENV_PLATFORM` 取 `chrome105` / `safari13`（design §8）
- [x] `tsconfig`：strict、`noUnusedLocals`、`noUnusedParameters`、`noFallthroughCasesInSwitch`、`verbatimModuleSyntax`，`paths` 配 `@/*`
- [x] shadcn 初始化：`components.json` 为 style `new-york`、baseColor `neutral`、`cssVariables: true`、iconLibrary `lucide`，别名 `@/components`、`@/components/ui`、`@/lib`、`@/hooks`、`@/lib/utils`
- [x] Tauri 2：`npm i -D @tauri-apps/cli@^2` → `npx tauri init --ci`；productName `Fingerprint Launcher`，identifier `io.github.madfuu.fingerprint-launcher`，devUrl `http://localhost:5173`，frontendDist `../dist`，`app.windows: []`（窗口在 P4 由代码创建）。Rust edition 2024
- [x] ESLint 9 flat config：typescript-eslint、react-hooks、react-refresh、`eslint-plugin-i18next`（`no-literal-string`，`mode: 'jsx-only'`，排除 `*.test.*` 与 `bridge.mock.ts`）；Prettier `{ singleQuote: true, semi: true, printWidth: 100, trailingComma: 'es5' }`
- [x] vitest + jsdom + @testing-library/react
- [x] npm scripts：`dev`、`build`、`typecheck`、`lint`、`format`、`test`、`check`、`tauri`
- [x] `LICENSE`（MIT）；`README.md` 骨架（项目说明、注明视觉设计参考 Simprint、fingerprint-chromium 链接）
- [x] 自绘 SVG logo → `npx tauri icon` 生成图标

**验证**（本机开发循环）：

```bash
npm run check
(cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings)
```

`tauri build` 与 xvfb 冒烟移到 P0b 的 CI 中。

实际偏差（已接受）：ESLint 10、TypeScript 6.0、Vite 7 有意固定、去掉模板自带的 `tauri-plugin-log` 与 android/ios 图标；`app.windows: []`，主窗口已在 `setup()` 中按 design §7 由代码创建（暂为 `visible(true)`，P5 改为就绪后显示）。

**提交**：`chore: scaffold tauri 2 + react 19 + tailwind 4 + shadcn`

---

## P0b CI 工作流与首次推送

前置：用户已在 GitHub 网页创建空的公开仓库 `madfuu/fingerprint-launcher`，并把 `~/.ssh/fingerprint_launcher_deploy.pub` 加为 Deploy key（勾选 Allow write access）。

- [x] `.github/workflows/ci.yml`，触发：`push`（`main`）、`pull_request`、`workflow_dispatch`
  - `check` job，矩阵 `ubuntu-22.04` / `windows-latest` / `macos-latest`：
    - Node LTS + `npm ci`；`npm run check` 只在 ubuntu 跑（前端与平台无关），其余平台只需 `npm run build` 产出 `dist/`
    - Rust stable + clippy + rustfmt（用带 problem matcher 的 action，如 `actions-rust-lang/setup-rust-toolchain`）+ `Swatinem/rust-cache`
    - ubuntu 先 apt 安装 Tauri 依赖：`libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev build-essential pkg-config`
    - `cargo fmt --check`（仅 ubuntu）、`cargo clippy --all-targets -- -D warnings`、`cargo test`
  - `bundle` job（`needs: check`），同一矩阵：`tauri build`（Linux `deb` + `appimage`，Windows `nsis`，macOS `dmg`），`actions/upload-artifact` 上传安装包；ubuntu 额外对产物二进制做 `xvfb-run` 15 s 冒烟（期望退出码 124）
  - `concurrency` 按分支取消过时的运行
- [x] 添加远端 `origin` = `git@github.com:madfuu/fingerprint-launcher.git`，`git ls-remote origin` 验证 Deploy key 可用后推送 `main`（包含 P0 的提交）
- [x] 轮询 Actions 结果直到三平台 `check` 与 `bundle` 全绿；修复后重推

**验证**：Actions 上该提交的 run 结论为 `success`；三平台安装包产物存在。

结果（2026-10-13）：首次推送 `002dcd9`，run 38277815453 全绿——三平台 `check` 与 `bundle`（含 Linux xvfb 冒烟）均 success。之后的用量控制改动（`e0c8eba`）在下一个里程碑随 P3 推送。

**提交**：`ci: three-platform check and bundle workflow`

---

## P1 Rust 数据层

- [x] 依赖：`rusqlite`（`bundled`）、`uuid`（v4）、`chrono`（serde）、`serde` / `serde_json`、`thiserror`、`tokio`；dev：`tempfile`
- [x] `src-tauri/src/db/`：`open(path)` 每次连接 `PRAGMA foreign_keys = ON` + `journal_mode = WAL`；migration v1 按 design §2.2 原样落地；`meta.schema_version`
- [x] 仓储：kernels / groups / tags / proxies / profiles / profile_tags / settings；profiles 支持 design §5 的 `ProfileQuery`（分页、view、分组、标签、关键字），软删除 / 恢复 / 永久删除
- [x] 错误类型 `AppError { code, message }`，可序列化为前端约定的形状
- [x] DTO 一律 `#[serde(rename_all = "camelCase")]`
- [x] 测试（临时目录 SQLite）：
  - migration 可重复执行
  - 删除分组 → profile.group_id 置 NULL；删除 profile → profile_tags 级联删除
  - 分页边界、`view = trash` 只返回已删除、关键字与标签组合过滤
  - settings 默认值（design §2.2）

**验证**：`cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

结果：51 个测试通过。偏差：`tokio` 推迟到 P3 再加（P1 未用到）；chrono 未开 `serde`（DTO 时间戳为 RFC 3339 字符串）。推迟到 P4：复制 / 导入导出、默认 user-data-dir 解析、`addKernel` 的名称与版本补全、复制时「副本」后缀的本地化。

审查后：嵌套 `in_tx` 改为 SAVEPOINT（外层捕获错误继续执行时不残留半写数据）；代理主机拒绝带端口的写法；分组与标签名称改为大小写不敏感唯一（`COLLATE NOCASE`，design §2.2 已同步）。共 53 个测试。未来 v2 若要重建表，需在事务外临时关闭 `foreign_keys`。

**提交**：`feat(db): sqlite schema v1 and repositories`

---

## P2 参数构建 + 版本门控 ★ 高风险

- [x] `kernel/args.rs`：`ArgSupport::for_major`、`build_launch_plan`、`validate_profile_for_kernel`，严格按 design §3.2 / §3.3 的区间与顺序
- [x] `shell_line` 转义（design §3.4），`HostPlatform` 显式传入
- [x] `extra_args` 用自写的可移植规则切分（P2 审查改定，见 design §3.3 第 9 步）；切分失败（引号不配对）返回校验错误而非 panic；同时在 `ProfileDraft` 保存时（`profiles` 的 normalized 校验）就校验，保存即报错，而不是等到启动才失败
- [x] `build_launch_plan` 接收的可能是**未保存的编辑器草稿**：空白字符串字段一律视为未设置（不生成 `--x=`）；未保存的 profile 没有 id，预览时 user-data-dir 由调用方传入占位路径
- [x] 代理主机为 IPv6 时，`--proxy-server` 中用方括号包裹（`socks5://[::1]:1080`）；P1 的校验只接受不带方括号的裸 IPv6
- [x] 测试（design §3.5 全部用例）：
  - README 两个官方示例：argv 与 `shell_line` 精确断言（argv 中无引号）
  - major ∈ {130, 131, 138, 139, 141, 142, 143, 144, 150}：参数集断言
  - seed 为 NULL 时不生成任何 `--fingerprint*` 参数
  - 切换版本时 `validate_profile_for_kernel` 对失效字段产出警告（A4）
  - POSIX / Windows 转义：空格、单引号、双引号、空字符串

**验证**：同 P1。

**审查关卡**：主会话逐条对照 `research/fingerprint-chromium-cli.md` §1–§3 审阅测试用例，确认没有遗漏参数、区间端点正确。这是 A3 / A4 的唯一证据来源。

结果：主会话已逐条对照 research §1–§3 审阅测试用例（门控区间、生成顺序、README 两例的参数集合）。81 个测试通过。偏差：README 示例只能按参数集合比对，顺序以 design §3.3 为准；`validate_profile_for_kernel` 接收大版本号而非 `&Kernel`（无内核时用 `effective_major`）。

审查后（trellis-check）：启动 URL 也禁止设置 `--user-data-dir`（Chromium 在任意位置识别开关）；cmd.exe 引号状态失衡时的 `^` 转义；zsh `=word` 加引号；Unicode 空白分词；拒绝 NUL；主会话追加：拒绝直引号外的弯引号。共 84 个测试。

**提交**：`feat(kernel): launch argument builder with version gating`

---

## P3 进程管理 ★ 高风险

- [x] 依赖：Unix `nix`（`signal`、`process`）；Windows `windows`（features 见 design §4.4）
- [x] `kernel/process/{mod,unix,windows}.rs`：spawn（Unix `process_group(0)`；Windows Job Object）、温和停止、强制停止（design §4.3）
- [x] `kernel/lock.rs`：单实例锁检测（design §4.2 第 2 步）
- [x] `kernel/probe.rs`：分平台版本探测（design §5）；Linux 两种形态都实现，Windows / macOS 实现在 `cfg` 分支内
- [x] `services/launcher.rs`：`AppState.running`、waiter 状态机（design §4.2 表格）、stderr 环形缓冲 + 日志文件、错误识别（sandbox / libfuse）、批量启动信号量（design §4.6）、`StatusSink` 与 `LauncherConfig` 注入（design §4.1）
- [x] `tests/fixtures/fake-chrome.sh`：按环境变量切换行为——正常常驻并派生一个子进程；忽略 SIGTERM；启动即带 stderr 非 0 退出；写 `SingletonLock` 指向自身；把收到的 argv 写入文件（实际由 `tests/launcher/fake.rs` 实现，见下方偏差）
- [x] 集成测试（Unix，用假内核，`LauncherConfig` 时间常量调短）：
  - 启动 → `Starting` → `Running`；argv 与 `build_launch_plan` 一致
  - 温和停止 → `Stopped`，且进程组内无存活进程（`killpg(pgid, 0)` 返回 ESRCH）（A6）
  - 忽略 SIGTERM 的内核 → 宽限期后被强制终止，进程组清空
  - 外部 kill 主进程 → waiter 检测到并转 `Stopped`（A5）
  - 早退 → `Error`，消息含 stderr 末行（A12）
  - 内核路径不存在 / 不可执行 → 可读错误（A12）
  - 重复启动同一 profile → 拒绝（R3.4）
  - 存活的外部 `SingletonLock` → 拒绝；陈旧锁（pid 已死）→ 放行
  - 批量启动：同时 `Starting` 数不超过上限（A8）
  - 停止全部（`quitApp` 的服务层部分）后无残留（A7）
- [x] 真 Chromium 测试（`#[ignore]`，`FPL_TEST_CHROME` 指向 `~/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome`，参数加 `--headless=new`）：启动、温和停止后无残留、两个 profile 目录互不干扰（A9）、真实 `SingletonLock` 被识别
- [x] Windows 分支：Job Object、`WM_CLOSE`、PE 版本、`lockfile` 的 `cfg(windows)` 代码由 CI 的 `windows-latest` 原生 clippy + test 验证。为此写 `#[cfg(windows)]` 测试：用一个跨平台的 Rust 假内核（`examples/fake_chrome.rs`：常驻、派生子进程、按参数决定是否早退），验证 Job Object 整树终止与早退检测；Unix 侧的同类测试照常用 `fake-chrome.sh`。macOS runner 原生跑 Unix 测试，覆盖 macOS 的进程组行为（实际三平台共用 `tests/launcher` 的同一个假内核；Windows 专属断言见 `cases.rs` 中的 `cfg(windows)` 分支与 `kernel/lock.rs`、`kernel/probe/pe.rs` 的 Windows 单元测试）

**验证**：

```bash
cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
FPL_TEST_CHROME=$HOME/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome cargo test -- --ignored
```

**审查关卡**：主会话审阅 waiter 状态机与停止流程，确认状态变更只有一条路径、无竞态（停止与自然退出同时发生、批量中途停止）。

结果（实现 agent，2026-10-13）：本机 lib 108 个单元测试 + 16 个假内核生命周期测试 + 3 个真 Chromium 测试（`--ignored`，沙箱可用，无需 `--no-sandbox`）全部通过；假内核测试连续 5 次、3 路并发、8 核满载下各跑通。Windows / macOS 分支本机未编译，首次验证在 CI。

偏差（均为减少本机无法编译的平台代码）：

- 假内核不用 `fake-chrome.sh` / `examples/fake_chrome.rs`：集成测试 `tests/launcher/`（`harness = false`）的二进制自身兼任假内核（带 `--user-data-dir=` 启动即扮演浏览器，行为由 `<udd>/fake.cfg` 配置），三平台同一份代码；自带一个兼容 `--ignored` / 过滤参数的小型测试运行器（带 300 s 看门狗：挂起的测试使运行失败并列出名称，而不是占住 CI runner 直到作业超时）。
- Windows 依赖用 `windows-sys` 0.61（已在依赖树中，FFI 签名逐项核对源码），不用 `windows` crate；温和停止用 `taskkill /PID`（不带 `/F`、`/T`，即投递 `WM_CLOSE`），不手写 `EnumWindows` + `PostMessageW`；因此不需要 `Win32_UI_WindowsAndMessaging` / `Win32_Storage_FileSystem`。
- 版本探测按文件类型而非宿主 OS 选择方法，全部为可移植代码：PE 版本资源自写解析（`pelite` 依赖陈旧）；macOS 用 `plist` crate（tauri-utils 已依赖）；夹具为 mingw 生成的 4 KB PE（`tests/fixtures/pe/`）。
- `nix` features 为 `signal` + `hostname`（`signal` 已隐含 `process`）。
- 运行表用 `std::sync::Mutex`（临界区内无 await），由 `services::launcher::Launcher` 持有，而非 design §4.1 的 `AppState.running: tokio::sync::Mutex`；`AppState` 与 `exiting` 标志留给 P4。
- `StatusSink` 为两个方法（状态 / 批量进度）；`batch://progress` 增加 `skipped`。
- 每次主进程退出后都对整棵进程树补杀一次（Unix `killpg`，Windows `TerminateJobObject`），回收外部 kill 主进程或崩溃后遗留的子进程。

里程碑推送（2026-10-13）：P1–P3 合并为一次推送 `c464c86`，run 38307455004 一次通过——ubuntu / windows / macos 的 `check`（原生 clippy + 全部测试，含 Windows Job Object 与 macOS 进程组用例）均 success，`bundle` 按规则跳过。

**提交**：`feat(launcher): process lifecycle, singleton lock, version probe`

---

## P4 命令层、窗口与退出

- [x] `commands/*.rs`：design §5 全部命令，全部 `async`；错误统一 `{ code, message }`
- [x] **TS 类型由 Rust 生成**（主会话决定，2026-10-13）：用 `ts-rs` 给 bridge 用到的全部 DTO / 事件载荷 / 错误类型派生 TS，导出到 `src/lib/bindings/`（提交进仓库）；一个 `cargo test` 负责导出，CI 在 `cargo test` 之后执行 `git diff --exit-code src/lib/bindings` 防止漂移。P5 的 `bridge.ts` 只引用生成的类型，不手写 DTO
- [x] 插件：`tauri-plugin-dialog`、`tauri-plugin-clipboard-manager`
- [x] `capabilities/default.json`：窗口最小化 / 切换最大化 / 关闭 / 拖动 / 显示、事件监听、`dialog:allow-open`、`dialog:allow-save`、`clipboard-manager:allow-write-text`——**只开用到的**
- [x] 用显式权限替换脚手架的 `core:default`；`tauri.conf.json` 的 `app.security.csp` 从 `null` 改为严格策略（`default-src 'self'`，放行 Tauri IPC 所需的 `ipc:` / `http://ipc.localhost`，样式允许 `'unsafe-inline'` 以兼容 Radix 的内联样式），并在 CI 冒烟中确认页面仍能加载（策略已设；加载验证随 P5 的冒烟升级）
- [x] 主窗口代码创建（design §7：尺寸、`visible(false)`、`decorations(false)`；macOS 用 Overlay 标题栏）
- [x] 退出拦截：`CloseRequested` + `ExitRequested` → `app://close-requested`；`quitApp` 防重入（design §4.5）
- [x] `testProxy`：`reqwest`（`rustls-tls`、`socks`），ipinfo → ip-api 回退，10 s 超时
- [x] `detectKernels`：`research/fingerprint-chromium-cli.md` §8 的候选位置
- [x] 导入 / 导出（design §5 细则）
- [x] `StatusSink` 的 Tauri 实现。**约束（P3 审查）**：sink 在启动器的运行表锁内被调用——只能向前端 emit，不得访问数据库（会与命令层的锁顺序相反），也不得注册会回调启动器的 Rust 侧 `listen`（Tauri 在 `emit` 内同步执行它们，std `Mutex` 不可重入）
- [x] 保存时的「两个 profile 共用同一自定义 user-data-dir」检查：路径先规范化（Windows 下大小写折叠）再比较；P3 的运行时检查只覆盖已在运行的 profile
- [x] `addKernel` 选中 `.app` 目录时先用 `kernel::probe::bundle_executable` 解析出内部可执行文件，再做可执行检查与版本探测
- [ ] （可选，未做）单次运行中 profile 日志无上限增长（只在启动时截断到 5 MB 以内）：如有需要改为运行中按大小轮转
- [x] 测试：导入导出往返；`purgeProfile(dropUserData)` 不删除 `user_data_root` 之外的目录
- [x] `previewCommand` 在没有任何内核时：用占位程序名 `chrome` 构造临时 `Kernel`，参数集按 `effective_major(None)`（最新）；`validateProfileForKernel` 同理
- [x] 命令层策略（P1 审查遗留）：删除（软删除 / 永久删除）运行中的 profile 时拒绝，错误码 `profile_running`；两个 profile 指向同一个自定义 user-data-dir 时保存即报 `conflict`；`testProxy` 的 URL 同样要给 IPv6 主机加方括号
- [ ] 设置页 / 文件对话框：Windows「复制为路径」带来的首尾引号在前端去掉再提交（前端事项，移交 P5 / P9）

**验证**：同 P1；推送后 CI 的 Linux 冒烟确认主窗口按代码创建后应用仍能正常启动。

结果（实现 agent，2026-10-13）：本机 lib 143 个单元测试 + 17 个假内核生命周期测试（新增 `running_profiles_cannot_be_deleted`）+ 3 个真 Chromium 测试（`--ignored`）通过；`npm run check`、`cargo fmt --check`、`cargo clippy --all-targets -D warnings`（`RUSTFLAGS=-D warnings`）全绿；actionlint 通过。Windows / macOS 分支本机未编译（新增的平台差异只有 `cfg!` 表达式与一个 `cfg(windows)` 测试），首次验证在 CI。

结构：`commands/{profiles,launch,kernels,library,app}.rs` 只做参数转发；业务在 `services/{profiles,transfer,kernels,preview,proxy_check}.rs`（不依赖 Tauri，单测直接调用）；`state.rs`（`AppState`：DB + `Launcher` + `AppPaths`，`db()` 走 `spawn_blocking`）、`paths.rs`（目录布局与路径比较）、`events.rs`（事件名、`CloseRequested`、`TauriSink`）、`shell.rs`（窗口、显示兜底、退出流程）、`bindings.rs`（ts-rs 导出，仅测试构建）。

偏差：

- 命令签名（design §5 已同步）：`previewCommand(draft, profileId)`、`validateProfileForKernel(draft)` 一律取 `draft.kernelId`；`purgeProfile` 返回 `PurgeResult`；`batchLaunch` 返回最终 `BatchProgress`；新增 `getAppInfo`；新错误码 `kernel_version_unknown`、`proxy_test_failed`。
- 退出（design §4.5 已同步）：不拦截 `RunEvent::ExitRequested`——tao 0.37 只在最后一个窗口销毁后才发它，已无处弹框；macOS Cmd+Q 不经过任何可拦截事件。改为 `CloseRequested` 为唯一确认点，`RunEvent::Exit` 中同步 `launcher.shutdown()` 兜底所有路径（含 Cmd+Q）。前端未就绪（从未调用 `showMainWindow`）时关闭不弹框，直接走退出流程。
- user-data-dir 检查从「相同」扩为「相同或互为包含」（含回收站中的 profile、他人的默认目录），macOS 也忽略大小写；purge 比较 canonical 路径，目录为符号链接 / 等于根目录 / 与他人重叠时保留。
- 导出不含内核绑定（机器相关）；复制时原 profile 无种子则副本仍无种子（不擅自开启指纹）。
- `detectKernels` 的 Linux AppImage 候选要求文件名含 `chromium`（探测会执行它）。
- reqwest 0.13 的 TLS 用 `rustls-no-provider` + `rustls/ring`（默认的 aws-lc-rs 在 Windows 上构建需额外工具）；SOCKS5 测试用 `socks5h`。
- ts-rs 为 dev-dependency，类型以 `cfg_attr(test, derive(TS))` 派生；生成目录不经 Prettier，参与 tsc / ESLint。CI 用 `git status --porcelain` 而非 `git diff --exit-code` 检查（后者看不到新增文件）。ts-rs 编译时会打印一条 `deserialize_with` 无法解析的提示（`SettingsPatch.userDataRoot`，预期内，其 TS 类型由 `ts(optional_fields)` 给出）。
- CSP 加 `dangerousDisableAssetCspModification: ["style-src"]`，否则 Tauri 注入的 nonce 会使 `'unsafe-inline'` 失效。CSP 的实际加载验证待 P5 的冒烟升级（`FPL_SMOKE_EXIT_ON_READY` 的 Rust 侧已就位，ci.yml 冒烟判定未改）。
- 未做：日志按大小轮转（可选项）；Windows「复制为路径」引号处理属前端。

审查后（trellis-check）：导入在解析时限制最多 10 000 条 profile，并去掉中间的 `serde_json::Value` 整树（32 MiB 的 `[{},{},…]` 原本会膨胀到数 GB 而 OOM），按上限读取文件而不信任事先的 `metadata` 大小；`probeKernel` 要求绝对路径（裸名会经 `PATH` 查找并以 `--version` 执行）；自定义 user-data-dir 不得是数据根目录本身或其上级（原先只在已有默认目录 profile 时才拒绝）；`testProxy` 的响应体最多读 64 KiB、不跟随重定向（回退服务是经用户代理的明文 HTTP），并新增经本地假代理的离线端到端测试（回退与超限）。共 149 个单元测试。退出流程逐条对照 tauri 2.12.3 / tao 0.37.1 源码核对无误（`RunEvent::Exit` 中 `block_on` 不会死锁：sink 的 `emit` 在工作线程上只投递到事件循环代理，不等待主线程；`tracing` feature 未开启，`eval_script` 不阻塞）。移交前端：重复的 `app://close-requested` 由前端去重（P10）；Tauri 自身的参数 / ACL 错误以纯字符串拒绝，`bridge.ts` 统一包装（P5）。

**提交**：`feat(commands): tauri command layer, window, exit handling`

---

## P5 前端地基

- [x] `src/index.css`：用本项目主题**替换** shadcn init 写入的 neutral 默认主题；按 `research/simprint-ui-reference.md` §1 **自行编写**主题变量、`@theme inline` 映射、基础层、滚动条、keyframes（§1.4）、对话框遮罩 `top: 48px`；字体按名称引用系统字体（§1.3），不打包字体文件
- [x] `lib/bridge.ts`：design §5 全部命令与事件的类型化封装；DTO 类型与 Rust 保持 camelCase 一致。Tauri 自身的拒绝（参数反序列化失败、ACL 拒绝）是纯字符串而非 `{ code, message }`，在 bridge 内统一包装成 `AppError`（design §5）
- [x] `lib/bridge.mock.ts`：内存实现 + 种子数据（约 25 个 profile，覆盖运行中 / 已删除 / 多分组 / 多标签 / 有无代理；3 个内核：142、144、150），模拟启动 → 运行 → 停止的状态事件；仅 `import.meta.env.DEV` 且无 `__TAURI_INTERNALS__` 时加载
- [x] mock 模式下支持 `?theme=light|dark&lang=zh-CN|en` 覆盖，供截图使用
- [x] i18n：`i18n/index.ts` + `registry.ts`；各 feature 自带 `i18n.ts`（zh-CN + en）；`common` 命名空间放通用文案
- [x] 主题：设置表驱动的 ThemeProvider（`light` / `dark` / `system`，监听系统主题变化）
- [x] 启动引导：`getSettings()` → 初始化 i18n 与主题 → 渲染 → `showMainWindow()`
- [x] **CI 冒烟升级**（主会话决定）：Rust 侧在环境变量 `FPL_SMOKE_EXIT_ON_READY=1` 时，收到 `showMainWindow` 调用即 `app.exit(0)`；ci.yml 的 xvfb 冒烟改为「设置该变量，60 s 内必须以 0 退出」——这样 CI 验证的是：WebView 加载前端、在正式 CSP 下完成一次 IPC 往返。原「15 s 仍存活」的判定作废。（Rust 侧已在 P4 完成：`shell::SMOKE_EXIT_ON_READY_ENV`；本阶段只需前端调用 `showMainWindow` 并改 ci.yml）
- [x] 运行态 store：`listRunning()` + 订阅 `profile://status-changed`
- [x] 路由骨架（design §6 表格），页面先放占位
- [x] shadcn 组件按需 `npx shadcn@latest add`。部分组件（如 dialog、sheet、pagination）自带硬编码读屏文案（`<span className="sr-only">Close</span>` 等），会触发 `i18next/no-literal-string`：生成后改为 `t()` 文案，不要为此放宽 lint 规则。组件列表：（button、input、label、select、checkbox、switch、tooltip、dropdown-menu、dialog、alert-dialog、popover、scroll-area、separator、badge、pagination、textarea、command、sonner、form 等）
- [x] `scripts/screenshot.mjs`：`playwright-core`，可执行文件取 `FPL_SCREENSHOT_CHROME`（默认 Chrome for Testing 路径），启动 vite dev，按「路由 × 主题 × 语言」在 1438×828 截图到 `.screenshots/`；npm script `screenshots`
- [x] vitest：mock bridge 状态流转、运行态 store 增量更新

**验证**：`npm run check`；`npm run screenshots` 能产出图片（此时为占位页）。

结果（实现 agent，2026-10-13）：`npm run check` 全绿（typecheck、lint 零告警、vitest 6 个文件 51 个测试、vite build）；`npm run screenshots` 产出 6 路由 × 浅/深 × 中/英共 24 张图，人工抽看浅色 / 深色、中 / 英均正常渲染；actionlint + shellcheck 通过。生产构建 `dist/` 中检索 mock 特征串（`沙箱失败示例`、`createMockTransport`、`__FPL_MOCK__`、种子代理主机名等）均为 0 次、只有一个 JS chunk；对照构建（`NODE_ENV=development vite build --mode development`）中这些串出现在独立的 `bridge.mock-*.js` chunk 里，证明检索有效。未改 Rust，未跑 cargo。

结构与决定（design §5 / §6 / §8 已同步）：

- bridge：`bridge.ts`（类型化命令表 `Commands`、46 个命令函数、事件订阅、对话框 / 剪贴板封装、`BridgeError` + `toAppError` 错误归一）+ `bridge.tauri.ts`（唯一 import `@tauri-apps/*` 的文件）+ `bridge.mock.ts` / `lib/mock/`（dev 且非 Tauri 时动态 import）。ESLint `no-restricted-imports` 强制这两条边界，已用探针文件验证规则命中
- 非 `AppError` 的拒绝一律归为 `invalid_input`（`ErrorCode` 无「内部错误」类；未知 code 保留在 message 中），映射表见 design §5
- 运行态 store：先订阅、后快照，订阅后收到过事件的 profile 以事件为准（论证见 `running-store.ts` 头注释与 design §6），测试覆盖快照前 / 后到达的事件、失败重试
- 启动：设置读取失败渲染错误页、渲染异常由 error boundary 接住，`showMainWindow()` 始终在 `finally` 中调用；`<html data-app-ready="true">` 为截图就绪信号
- 截图脚本在本进程内运行 Vite（随机空闲端口、排除 5173），因此不存在需要按 PID 回收的子进程；浏览器由 playwright-core 关闭。已验证：过滤参数错误退出 2；Vite 启动后浏览器启动失败退出 1 且无残留；跑完后无 vite / chrome / esbuild 进程、端口已释放

偏差：

- shadcn 组件中硬编码的读屏 / 按钮文案（dialog、pagination、command 的默认标题）改为 `t()`；`sonner.tsx` 改用 `useResolvedTheme`，移除 shadcn 自动安装的 `next-themes`。lint 配置未加任何豁免：`position` / `reason` / `path` 这类标识属性改为常量或拆分组件（`SettingsReadError` / `RenderError`、`paths.editProfilePattern`）
- mock 额外支持 `?os=` 覆盖与 `window.__FPL_MOCK__`（供 P6 截 macOS 标题栏、P10 触发退出确认事件）
- 窗口控制（最小化 / 最大化 / 关闭 / 拖动）尚未进 bridge：P6 实现标题栏时加到 `Transport`（`@tauri-apps/*` 只能在 `bridge.tauri.ts` 中使用）
- P4 移交的「Windows『复制为路径』首尾引号」处理推迟到 P9：P5 尚无路径输入框
- 本机限制：截图只验证 Chromium 渲染；本机未装 PingFang / 微软雅黑 / Noto Sans CJK，中文以 WenQuanYi 等回退字体显示
- 仓库既有问题（非 P5 引入）：`npx prettier --check .` 报 `src-tauri/tests/fixtures/pe/README.md`（`c464c86` 提交时表格未格式化）；本阶段已顺带把该表格按 Prettier 格式化（仅空白变化）

审查后（trellis-check）：逐个核对 46 个命令封装与 `src-tauri/src/commands/**` 及 `lib.rs` 的注册表（命令名、参数键——tauri-macros 2.7.2 默认 `ArgumentCase::Camel`——与返回类型）全部一致，并新增 `src/lib/bridge.contract.test.ts`：以 Vite `?raw` 读取 Rust 源码，解析已注册命令及其非注入参数，断言每个封装恰好以该命令名与 camelCase 参数键调用（已用改坏一个参数键的变异验证会失败），此后契约漂移在 vitest 中即被发现。mock 对齐 Rust：draft 按 serde 读取（多余字段如 `id` / `deletedAt` / 行的 `group` 被忽略，缺失字段取 `ProfileDraft::default`——此前把整个 `Profile` 传给 `updateProfile` 会覆盖其 id 与删除时间）；列表同时间戳时按插入倒序（`rowid DESC`）；关键字与标签排序只折叠 ASCII 大小写（SQLite `LIKE` / `NOCASE`）；`batchSetGroup` / `batchAddTags` / `setProfileTags` 全有或全无并更新 `updatedAt`，未知分组为 `invalid_input`（外键）而非 `not_found`；种子中运行中 profile 的 `lastLaunchedAt` 与其启动时间一致。运行态 store：`dispose()` 后仍在进行中的 `init()` 不再写入 store，迟到的订阅会被立即退订。截图脚本：未知命令行选项与其他参数错误一样退出 2，SIGTERM 退出 143。共 7 个测试文件 56 个测试。

**提交**：`feat(ui): theme, i18n, bridge, app shell foundation`

---

## P6 布局复刻

依据 `research/simprint-ui-reference.md` §2–§6，**自行实现**：

- [x] `app-background.tsx`：双层 canvas（网格 / 点阵 / 噪点 + 模糊光晕），按 DPR 缩放，主题切换重绘，resize debounce 150 ms
- [x] `app-titlebar.tsx`：四槽位；品牌（logo + 名称）、面包屑、状态区（主题切换、语言切换）、窗口控制（拖拽把手 + 三按钮，关闭 hover 为 destructive）；macOS 隐藏窗口控制、品牌区左留白。最大化 / 还原图标随窗口状态切换，需要在 capabilities 中加 `core:window:allow-is-maximized`（P4 审查）
- [x] 对话框打开时标题栏仍可用（P5 审查）：遮罩已从 48px 起（`index.css`），但 Radix 模态对话框会给 `body` 设 `pointer-events: none`（`@radix-ui/react-dismissable-layer`），并在外部 pointerdown 时关闭 Dialog。标题栏根元素须显式 `pointer-events-auto`，拖动 / 最小化 / 关闭才可用；并决定点标题栏是否应关闭已打开的 Dialog（必要时在 `onPointerDownOutside` 中对标题栏区域 `preventDefault`）。截图核对外再用 Playwright 在 mock 下点一次验证
- [x] `app-sidebar.tsx`：`w-16 ↔ w-58`、毛玻璃、渐变 CTA「新建配置」、导航项（激活竖条、tooltip、分组标题）、折叠按钮动画；折叠状态存 `settings.sidebar_collapsed`
- [x] **冒烟判定收紧**（P5 审查，主会话决定）：`showMainWindow` 增加参数 `bootOk: boolean`（Rust 命令 + bindings 重新生成 + bridge + mock）。正常运行时无论真假都显示窗口；`FPL_SMOKE_EXIT_ON_READY=1` 时 `bootOk` 为真才 `exit(0)`，为假则 `exit(1)` 并打印原因。前端在设置加载失败 / 渲染进入错误边界时传 `false`
- [x] 窗口控制（最小化 / 最大化切换 / 关闭 / 是否最大化 / 拖动）加入 bridge 的 `Transport`，mock 中为空操作
- [x] `app-layout.tsx`：§3 的骨架

**验证**：`npm run check`；`npm run screenshots`。

**审查关卡**：主会话读截图——侧栏折叠 / 展开 × 浅色 / 深色，与参考文档逐项核对（尺寸、激活态、CTA 渐变、背景网格与光晕、标题栏分隔线）。

结果（实现 agent，2026-10-13）：`npm run check` 全绿（typecheck、lint 零告警、vitest 7 个文件 58 个测试、vite build）；`cargo fmt --check`、`RUSTFLAGS=-D warnings cargo clippy --all-targets -D warnings`、`cargo test`（149 单元 + 17 假内核）全绿，`src/lib/bindings` 无改动（新参数都是原始类型，不产生新 DTO）；`npx prettier --check .` 通过。`npm run screenshots` 产出 31 张（6 路由 × 折叠 + 配置列表的展开 / macOS / 对话框变体，浅深 × 中英）；`npm run ui-check` 18 项全过。

实现要点与偏差：

- 文件：`app/layout/{app-layout,app-titlebar,app-sidebar,app-background}.tsx` + `background-paint.ts`（两层画法的纯函数）、`nav.ts`（侧栏导航表 + 面包屑路由表）、`i18n.ts`（`layout` 命名空间）；`app/app-info.ts`（`getAppInfo` store，宿主 OS）、`app/boot-status.ts`（启动失败原因汇总）；`hooks/use-window-maximized.ts`、`lib/titlebar.ts`
- 标题栏整条是 `data-tauri-drag-region="deep"`（Tauri 2.12 的 drag 脚本会跳过 button / a 等可点元素），因此不必逐块标注拖动区；双击最大化走 `internal_toggle_maximize`
- 点标题栏**不关闭**对话框：`DialogContent` 的 `onInteractOutside` 对 `[data-app-titlebar]` 内的交互 `preventDefault`（拖窗口不是对对话框的回答）；点其他地方照常关闭。两条都在 `ui-check` 中断言
- 错误页（设置读取失败 / 渲染异常）也渲染一条无导航的 `TitlebarFrame`，否则无边框窗口在启动失败时无法移动或关闭
- `showMainWindow(bootOk, failure)`：多传一个原因字符串，Rust 在 `bootOk` 为假时打到 stderr（CI 冒烟的 `::error::` 分支据此区分「没加载」与「加载了但渲染的是错误页」）。前端的失败来源有三处：`getSettings` / `getAppInfo` 失败、首帧 3 s 未提交、错误边界捕获
- `getAppInfo` 失败不再阻断启动（只记一条失败原因）：标题栏退回按 user agent 判断宿主 OS
- macOS 红绿灯用 `traffic_light_position(18, 18)` 对齐 48px 标题栏中线（`WebviewWindowBuilder` 的 macOS 专有方法）
- 截图脚本重构为 `scripts/lib/dev-preview.mjs`（Vite + Chrome 外壳）+ `screenshot.mjs`（加 `--variant`，文件名变为 `<route>-<variant>-<theme>-<lang>.png`）+ 新增 `ui-check.mjs`；对话框状态由 `src/dev/`（仅脚本经 dev server 动态加载，ESLint 禁止应用 import，`dist/` 中已确认无痕迹）提供
- dev server 需预热 `src/dev/*.tsx`：探针首次引入 `cmdk` 会触发 Vite 重新预构建，页面因此拿到第二份 React（实测报 "Invalid hook call"）
- 侧栏折叠按钮带 `pulse-gentle` 无限动画，Playwright 的稳定性检查永远不通过，`ui-check` 对它用 `force: true`；宽度断言等 Web Animations 的 `finished`

审查后（trellis-check）：

- **冒烟判定补洞**（已修）：窗口被销毁（WebView 崩溃、未拦截的关闭）时 tao 的事件循环本身以 0 退出（`tauri-runtime-wry` 的 `TaoWindowEvent::Destroyed` → `ControlFlow::Exit`），此前这会被冒烟当成「通过」，而冒烟是整个 CI 里唯一的端到端证据。现 `shell::on_run_event` 在冒烟模式且 `frontend_ready` 为假时打一行 stderr 并退 3，ci.yml 多一个分支说明这种情况。`showMainWindow` 的成功/失败语义不变（0 / 1）
- ci.yml：退 1 的 `::error::` 文案补上「或 Xvfb 没能启动」——`xvfb-run` 自身在 Xvfb 启动失败时也退 1（Debian 的 `/usr/bin/xvfb-run`），两者无法用退出码区分，日志里有没有 `frontend startup failed` 才是判据。`status=0; cmd || status=$?` 在 `-eo pipefail` 下不会提前退出，124 / 1 / 3 / 其他四条分支齐全，actionlint + shellcheck 通过
- `ui-check` 里一条恒为 `true` 的断言改为真正读 `data-maximized`（原先靠前一句 `waitFor()` 兜着，但 18/18 的计数里混了一条空断言）
- 已核对、无问题：`show_main_window(boot_ok, failure)` ↔ `bridge.showMainWindow` ↔ `bridge.contract.test.ts`（参数键 `bootOk` / `failure`）；`showMainWindow` 在所有路径上都会被调用（`finally`），正常运行两种 `bootOk` 都显示窗口，`app.exit(0)` 只有「冒烟 + bootOk」与 `quitApp` 两处；能力清单对照 `@tauri-apps/api` 2.12.3 实际 invoke 的命令（`plugin:window|{minimize,toggle_maximize,close,is_maximized}`、`plugin:event|{listen,unlisten}`，拖动与双击走注入脚本的 `start_dragging` / `internal_toggle_maximize`，`getCurrentWindow()` 带 `skip: true` 不建窗）逐条齐全，且都存在于 `gen/schemas/desktop-schema.json`
- `data-tauri-drag-region="deep"`（tauri 2.12.3 `src/window/scripts/drag.js`）：`isDragRegion` 自事件目标向上走，遇到可点元素（`BUTTON` / `A` / 有 `tabindex` / 交互 `role`）且其自身无该属性即返回 false——标题栏里的按钮照常可点、双击不会最大化（图标 svg 不是 `HTMLElement`，会被跳过，`[&_svg]:pointer-events-none` 之外的触发器也安全）；macOS 预留的 78px 在 deep 区域内，仍可拖动；下拉菜单内容都在 Portal 里，不在拖动区
- 背景 canvas：DPR 用 `setTransform` + `max(1, round(w*dpr))`，0×0 与极小窗口不会让 `getImageData` 抛错；resize 监听与 debounce 定时器在卸载和主题切换时都清掉（StrictMode 双挂载不漏）；`getImageData/putImageData` 每次重绘一次，1438×828@dpr1 约 71k 次采样，受 150 ms debounce 约束
- A14：`app-background` / `app-sidebar` / 标题栏与 `/home/a/projects/simprint` 对应文件逐段比对，结构、命名、画法都不同（我们是纯函数 + 常量表，Simprint 是单个 `useEffect` 内联），一致的只有视觉参数（6px 网格、0.012 点阵透明度、色值），这正是「视觉复刻」的目标；仓库内无任何字体文件，`public/` 只有自绘 `logo.svg`
- `src/dev` 围栏：用探针文件验证 `@/dev/...` 与 `../dev/...` 的静态 import 会报错；`npm run build` 后 `dist/` 中检索 `dialogProbe` / `openDialogProbe` / `mount-dialog-probe` / `__FPL_MOCK__` / `createMockTransport` 均为 0

判断留给用户（未改）：

- `core:window:allow-show` 自 P4 起就在能力清单里，但前端从不调 `appWindow.show()`（显示窗口是 Rust 侧 `showMainWindow` 做的），属多余权限；要删需同步 design §7
- 侧栏折叠时 `resizing`（内容淡出 300 ms）从点击那一刻开始，而宽度要等 `updateSettings` 回来才变；本机 SQLite 往返很快，但保存慢于 300 ms 时会先淡出再变宽，保存失败则白淡一次
- 网格线没有 Simprint 的 +0.5px 偏移（`lineWidth 0.5` 落在整数坐标上会跨像素，线更淡），属视觉取向
- macOS `traffic_light_position(18, 18)`：tao 把 y 当作按钮组上方的留白（`title_bar_frame_height = 按钮高 + y`），按钮高约 15px 时组中线落在 25.5px，48px 标题栏的中线是 24px——差 1~2px，需真机核对
- ESLint 的 `no-restricted-imports` 只管静态 import，动态 `import()`（bridge 加载 mock 正是靠它）绕得过去，最终仍靠 `dist/` 检索兜底
- Tailwind 会扫描 `src/dev/*.tsx`，其中用到的工具类会进生产 CSS（目前探针只用应用自身组件的类，没有多余产出）

**提交**：`feat(ui): app layout — titlebar, sidebar, background`

---

## P7 配置列表页

- [ ] 头部：分段 tab（全部 / 运行中 / 回收站）+ 工具按钮（搜索 Ctrl/Cmd+K、导入、导出）
- [ ] `components/data-table/`：粘性选择列 / 序号列 / 操作列、表头样式、骨架行、空状态、全选 indeterminate
- [ ] 列：`☐ / # / 指纹（平台图标 + 种子 mono）/ 名称（备注 tooltip）/ 代理 / 时区 / 分组 / 标签 / 状态 / 操作`
- [ ] 启动 / 停止按钮四态（未运行 / 启动中 / 运行中 / 停止中）、运行中行底色、启动 / 停止进度遮罩（用主题无关颜色，参考文档 §7.2）；操作下拉：编辑、复制、导出、删除
- [ ] 分页条；批量浮条（普通 / 回收站两套）
- [ ] 分组筛选需要「未分组」选项：`ProfileQuery.groupId` 目前无法表达，届时在 P1 的查询上加一个显式的 ungrouped 条件
- [ ] 对话框：删除确认、永久删除（含「同时删除浏览器数据目录」复选；说明文字与后端规则一致：只删除位于数据根目录内的目录，根目录外的自定义目录永不删除，与其他配置共用的目录也会保留——P4 审查后定稿）、移动分组、设置标签、搜索
- [ ] 导入 / 导出接 dialog 插件的打开 / 保存对话框

**验证**：`npm run check`；`npm run screenshots`（列表有数据、运行中、多选浮条、回收站、空状态，浅 / 深 × 中 / 英）。

**审查关卡**：主会话读截图核对参考文档 §7。

**提交**：`feat(profiles): list page with batch actions`

---

## P8 编辑器页

- [ ] 结构：顶部工具条 + `w-5/6 mx-auto` 主区 + 分段导航滚动 + 右侧 420px 摘要栏 + 底部悬浮操作栏（参考文档 §8）
- [ ] 分段：基本信息（名称、备注、内核、分组、标签）/ 指纹（种子 + 随机、平台、平台版本、品牌、品牌版本、CPU 核心数、GPU 段）/ 网络与语言（代理选择、语言、接受语言、时区、WebRTC UDP）/ 浏览器行为（启动 URL、headless + 风险提示）/ 高级（额外参数、user-data-dir 覆盖）
- [ ] 字段显隐由 `getArgSupport(kernelId)` 驱动；切换内核时展示 `validateProfileForKernel` 的警告，**不清空**失效字段的值
- [ ] 指纹种子为空时，指纹分段的其余字段都不会生效（构建器整段跳过）：新建 profile 默认填入随机种子；种子被清空时在指纹分段顶部显示提示，并把其余指纹字段显示为未生效状态。后端不为此产生警告（P2 决定）
- [ ] 额外参数输入框的帮助文案说明拆分规则：空格分隔、引号分组、反斜杠按字面保留、不支持 `--user-data-dir`（用专门字段）
- [ ] 上游限制文案：代理不支持密码验证、headless 风险、GPU 伪装仅 Linux 生效（`research/fingerprint-chromium-cli.md` §4）
- [ ] 时区下拉：`Intl.supportedValuesOf('timeZone')`，不可用时回退内置列表；语言下拉用内置常用语言列表
- [ ] 摘要栏：指纹摘要卡片（水印图标、随机按钮）+ 命令行预览卡片（debounce 300 ms 调 `previewCommand`，等宽，复制按钮走 clipboard 插件）
- [ ] 新建 / 编辑两种模式；离开页面时若有未保存修改则确认
- [ ] vitest：表单 zod 校验（种子范围 1..=2147483647、端口、语言代码格式）

**验证**：`npm run check`；`npm run screenshots`（新建、编辑、内核 142 与 144 的字段差异、切换内核的警告，浅 / 深）。

**审查关卡**：主会话读截图；核对 142 / 144 字段集与 design §3.2 一致（A4 的 UI 部分）。

**提交**：`feat(editor): profile editor with live command preview`

---

## P9 分组与标签、代理中心、设置

- [ ] 分组与标签页：增删改，颜色，删除前提示受影响的 profile 数
- [ ] 代理中心：列表 + 增删改对话框（仅协议 / 主机 / 端口 / 名称 / 备注，**无用户名密码字段**，附说明）+ 连通性测试（出口 IP / 国家 / 城市 / 时区）
- [ ] 设置页：
  - 内核管理：列表（名称、路径、版本、默认）；添加（文件对话框 → `probeKernel` → 探测失败时要求手填大版本号）；「自动探测」对话框列出候选，用户勾选确认后才加入；设默认；删除
  - 浏览器数据根目录（目录对话框）。修改时若新根目录与现有自定义 user-data-dir 相互嵌套，给出警告（P4 审查遗留）
  - 语言、主题
  - 批量启动并发上限与间隔

**验证**：`npm run check`；`npm run screenshots`（三个页面，浅 / 深）。

**提交**：`feat(ui): groups, tags, proxies, settings`

---

## P10 批量、退出确认、收尾

- [ ] 批量启动 / 停止接通，`batch://progress` 用 sonner 显示进度
- [ ] `app://close-requested` → 确认框（显示运行中数量）→ `quitApp()`；确认框已打开时再收到该事件只更新数量，不叠加第二个（标题栏不被遮罩覆盖，关闭按钮仍可点，design §4.5）
- [ ] en 文案补全；`npm run lint` 中 `i18next/no-literal-string` 零告警（A10）
- [ ] 错误码 → i18n 文案映射全覆盖（design §5 错误约定）
- [ ] 启动时数据库打开失败（损坏、无权限、版本过新）目前在 `setup()` 中 panic 且无任何提示：改为弹出原生错误对话框（说明数据库路径与原因，提供「退出」），不得静默崩溃（P4 审查遗留，主会话决定）

**验证**：本机开发循环检查；真 Chromium `--ignored` 测试；`npm run screenshots`（退出确认框、批量进度）。

**提交**：`feat: batch operations and exit confirmation`

---

## P11 跨平台收尾与发布

CI 已在 P0b 建立，此阶段只做收尾。

- [ ] 复查所有 `cfg(target_os)` 分支：Windows（Job Object、`WM_CLOSE`、PE 版本、`lockfile`）、macOS（Overlay 标题栏、plist 探测），确认 CI 三平台的 test 覆盖了它们
- [ ] `.github/workflows/release.yml`：推送 `v*` tag 时三平台 `tauri build`，产物附到 GitHub Release（草稿）
- [ ] macOS 签名：CI 产物未签名，下载后可能被提示「已损坏」。至少配置 ad-hoc 签名（`bundle.macOS.signingIdentity: "-"`）；`macos-latest` 为 arm64，dmg 仅支持 Apple 芯片，是否加 x86_64 / universal 由用户决定
- [ ] 若 Actions 缓存超过 10 GB 上限导致频繁冷编译：`setup-rust-toolchain` 加 `cache-save-if: ${{ github.ref == 'refs/heads/main' }}`
- [ ] macOS 最低版本：Tailwind 4 需要 Safari 16.4+，即 macOS 13.3+。在 `tauri.conf.json` 设 `bundle.macOS.minimumSystemVersion: "13.3"`，README 注明（P5 审查）
- [ ] README：安装、配置内核路径、上游限制（代理无密码验证、headless、GPU 仅 Linux）、构建方法、视觉参考致谢

**验证**：

```bash
npm run check
(cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test)
grep -rIn -i "simprint" --exclude-dir={node_modules,target,.git,.trellis} .   # A14：逐条人工确认，只允许致谢/说明性提及
find . -path ./node_modules -prune -o \( -name "*.ttf" -o -name "*.otf" -o -name "*.woff*" \) -print   # A14：应无输出
```

推送后 CI 全绿，三平台安装包产物齐全（A1）。release 工作流只做语法与触发条件审查，是否实际打 tag 发布由用户决定。

**提交**：`chore: release workflow and docs`

---

## 最终关卡

- [ ] trellis-check 全量审查（对照 prd / design / implement）
- [ ] 逐条列出 A1–A14 的证据（测试名 / 截图文件 / 命令输出 / CI run 链接），并单列「需真机验收」的项目（prd 约束一节）
