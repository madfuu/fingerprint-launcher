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

**截图验证**（P6 起）：`npm run screenshots` 产出 `.screenshots/<route>-<theme>-<lang>.png`，由主会话读图对照 `research/simprint-ui-reference.md` 核对。截图留在本机做，因为 Actions 的产物需要鉴权才能下载，本机没有 GitHub API 令牌。

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
- [ ] 添加远端 `origin` = `git@github.com:madfuu/fingerprint-launcher.git`，`git ls-remote origin` 验证 Deploy key 可用后推送 `main`（包含 P0 的提交）
- [ ] 轮询 Actions 结果直到三平台 `check` 与 `bundle` 全绿；修复后重推

**验证**：Actions 上该提交的 run 结论为 `success`；三平台安装包产物存在。

**提交**：`ci: three-platform check and bundle workflow`

---

## P1 Rust 数据层

- [ ] 依赖：`rusqlite`（`bundled`）、`uuid`（v4）、`chrono`（serde）、`serde` / `serde_json`、`thiserror`、`tokio`；dev：`tempfile`
- [ ] `src-tauri/src/db/`：`open(path)` 每次连接 `PRAGMA foreign_keys = ON` + `journal_mode = WAL`；migration v1 按 design §2.2 原样落地；`meta.schema_version`
- [ ] 仓储：kernels / groups / tags / proxies / profiles / profile_tags / settings；profiles 支持 design §5 的 `ProfileQuery`（分页、view、分组、标签、关键字），软删除 / 恢复 / 永久删除
- [ ] 错误类型 `AppError { code, message }`，可序列化为前端约定的形状
- [ ] DTO 一律 `#[serde(rename_all = "camelCase")]`
- [ ] 测试（临时目录 SQLite）：
  - migration 可重复执行
  - 删除分组 → profile.group_id 置 NULL；删除 profile → profile_tags 级联删除
  - 分页边界、`view = trash` 只返回已删除、关键字与标签组合过滤
  - settings 默认值（design §2.2）

**验证**：`cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

**提交**：`feat(db): sqlite schema v1 and repositories`

---

## P2 参数构建 + 版本门控 ★ 高风险

- [ ] `kernel/args.rs`：`ArgSupport::for_major`、`build_launch_plan`、`validate_profile_for_kernel`，严格按 design §3.2 / §3.3 的区间与顺序
- [ ] `shell_line` 转义（design §3.4），`HostPlatform` 显式传入
- [ ] `extra_args` 用 `shell-words` 切分；切分失败（引号不配对）返回校验错误而非 panic
- [ ] 测试（design §3.5 全部用例）：
  - README 两个官方示例：argv 与 `shell_line` 精确断言（argv 中无引号）
  - major ∈ {130, 131, 138, 139, 141, 142, 143, 144, 150}：参数集断言
  - seed 为 NULL 时不生成任何 `--fingerprint*` 参数
  - 切换版本时 `validate_profile_for_kernel` 对失效字段产出警告（A4）
  - POSIX / Windows 转义：空格、单引号、双引号、空字符串

**验证**：同 P1。

**审查关卡**：主会话逐条对照 `research/fingerprint-chromium-cli.md` §1–§3 审阅测试用例，确认没有遗漏参数、区间端点正确。这是 A3 / A4 的唯一证据来源。

**提交**：`feat(kernel): launch argument builder with version gating`

---

## P3 进程管理 ★ 高风险

- [ ] 依赖：Unix `nix`（`signal`、`process`）；Windows `windows`（features 见 design §4.4）
- [ ] `kernel/process/{mod,unix,windows}.rs`：spawn（Unix `process_group(0)`；Windows Job Object）、温和停止、强制停止（design §4.3）
- [ ] `kernel/lock.rs`：单实例锁检测（design §4.2 第 2 步）
- [ ] `kernel/probe.rs`：分平台版本探测（design §5）；Linux 两种形态都实现，Windows / macOS 实现在 `cfg` 分支内
- [ ] `services/launcher.rs`：`AppState.running`、waiter 状态机（design §4.2 表格）、stderr 环形缓冲 + 日志文件、错误识别（sandbox / libfuse）、批量启动信号量（design §4.6）、`StatusSink` 与 `LauncherConfig` 注入（design §4.1）
- [ ] `tests/fixtures/fake-chrome.sh`：按环境变量切换行为——正常常驻并派生一个子进程；忽略 SIGTERM；启动即带 stderr 非 0 退出；写 `SingletonLock` 指向自身；把收到的 argv 写入文件
- [ ] 集成测试（Unix，用假内核，`LauncherConfig` 时间常量调短）：
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
- [ ] 真 Chromium 测试（`#[ignore]`，`FPL_TEST_CHROME` 指向 `~/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome`，参数加 `--headless=new`）：启动、温和停止后无残留、两个 profile 目录互不干扰（A9）、真实 `SingletonLock` 被识别
- [ ] Windows 分支：Job Object、`WM_CLOSE`、PE 版本、`lockfile` 的 `cfg(windows)` 代码由 CI 的 `windows-latest` 原生 clippy + test 验证。为此写 `#[cfg(windows)]` 测试：用一个跨平台的 Rust 假内核（`examples/fake_chrome.rs`：常驻、派生子进程、按参数决定是否早退），验证 Job Object 整树终止与早退检测；Unix 侧的同类测试照常用 `fake-chrome.sh`。macOS runner 原生跑 Unix 测试，覆盖 macOS 的进程组行为

**验证**：

```bash
cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
FPL_TEST_CHROME=$HOME/.cache/ms-playwright/chromium-1234/chrome-linux64/chrome cargo test -- --ignored
```

**审查关卡**：主会话审阅 waiter 状态机与停止流程，确认状态变更只有一条路径、无竞态（停止与自然退出同时发生、批量中途停止）。

**提交**：`feat(launcher): process lifecycle, singleton lock, version probe`

---

## P4 命令层、窗口与退出

- [ ] `commands/*.rs`：design §5 全部命令，全部 `async`；错误统一 `{ code, message }`
- [ ] 插件：`tauri-plugin-dialog`、`tauri-plugin-clipboard-manager`
- [ ] `capabilities/default.json`：窗口最小化 / 切换最大化 / 关闭 / 拖动 / 显示、事件监听、`dialog:allow-open`、`dialog:allow-save`、`clipboard-manager:allow-write-text`——**只开用到的**
- [ ] 用显式权限替换脚手架的 `core:default`；`tauri.conf.json` 的 `app.security.csp` 从 `null` 改为严格策略（`default-src 'self'`，放行 Tauri IPC 所需的 `ipc:` / `http://ipc.localhost`，样式允许 `'unsafe-inline'` 以兼容 Radix 的内联样式），并在 CI 冒烟中确认页面仍能加载
- [ ] 主窗口代码创建（design §7：尺寸、`visible(false)`、`decorations(false)`；macOS 用 Overlay 标题栏）
- [ ] 退出拦截：`CloseRequested` + `ExitRequested` → `app://close-requested`；`quitApp` 防重入（design §4.5）
- [ ] `testProxy`：`reqwest`（`rustls-tls`、`socks`），ipinfo → ip-api 回退，10 s 超时
- [ ] `detectKernels`：`research/fingerprint-chromium-cli.md` §8 的候选位置
- [ ] 导入 / 导出（design §5 细则）
- [ ] `StatusSink` 的 Tauri 实现
- [ ] 测试：导入导出往返；`purgeProfile(dropUserData)` 不删除 `user_data_root` 之外的目录

**验证**：同 P1；推送后 CI 的 Linux 冒烟确认主窗口按代码创建后应用仍能正常启动。

**提交**：`feat(commands): tauri command layer, window, exit handling`

---

## P5 前端地基

- [ ] `src/index.css`：用本项目主题**替换** shadcn init 写入的 neutral 默认主题；按 `research/simprint-ui-reference.md` §1 **自行编写**主题变量、`@theme inline` 映射、基础层、滚动条、keyframes（§1.4）、对话框遮罩 `top: 48px`；字体按名称引用系统字体（§1.3），不打包字体文件
- [ ] `lib/bridge.ts`：design §5 全部命令与事件的类型化封装；DTO 类型与 Rust 保持 camelCase 一致
- [ ] `lib/bridge.mock.ts`：内存实现 + 种子数据（约 25 个 profile，覆盖运行中 / 已删除 / 多分组 / 多标签 / 有无代理；3 个内核：142、144、150），模拟启动 → 运行 → 停止的状态事件；仅 `import.meta.env.DEV` 且无 `__TAURI_INTERNALS__` 时加载
- [ ] mock 模式下支持 `?theme=light|dark&lang=zh-CN|en` 覆盖，供截图使用
- [ ] i18n：`i18n/index.ts` + `registry.ts`；各 feature 自带 `i18n.ts`（zh-CN + en）；`common` 命名空间放通用文案
- [ ] 主题：设置表驱动的 ThemeProvider（`light` / `dark` / `system`，监听系统主题变化）
- [ ] 启动引导：`getSettings()` → 初始化 i18n 与主题 → 渲染 → `showMainWindow()`
- [ ] 运行态 store：`listRunning()` + 订阅 `profile://status-changed`
- [ ] 路由骨架（design §6 表格），页面先放占位
- [ ] shadcn 组件按需 `npx shadcn@latest add`。部分组件（如 dialog、sheet、pagination）自带硬编码读屏文案（`<span className="sr-only">Close</span>` 等），会触发 `i18next/no-literal-string`：生成后改为 `t()` 文案，不要为此放宽 lint 规则。组件列表：（button、input、label、select、checkbox、switch、tooltip、dropdown-menu、dialog、alert-dialog、popover、scroll-area、separator、badge、pagination、textarea、command、sonner、form 等）
- [ ] `scripts/screenshot.mjs`：`playwright-core`，可执行文件取 `FPL_SCREENSHOT_CHROME`（默认 Chrome for Testing 路径），启动 vite dev，按「路由 × 主题 × 语言」在 1438×828 截图到 `.screenshots/`；npm script `screenshots`
- [ ] vitest：mock bridge 状态流转、运行态 store 增量更新

**验证**：`npm run check`；`npm run screenshots` 能产出图片（此时为占位页）。

**提交**：`feat(ui): theme, i18n, bridge, app shell foundation`

---

## P6 布局复刻

依据 `research/simprint-ui-reference.md` §2–§6，**自行实现**：

- [ ] `app-background.tsx`：双层 canvas（网格 / 点阵 / 噪点 + 模糊光晕），按 DPR 缩放，主题切换重绘，resize debounce 150 ms
- [ ] `app-titlebar.tsx`：四槽位；品牌（logo + 名称）、面包屑、状态区（主题切换、语言切换）、窗口控制（拖拽把手 + 三按钮，关闭 hover 为 destructive）；macOS 隐藏窗口控制、品牌区左留白
- [ ] `app-sidebar.tsx`：`w-16 ↔ w-58`、毛玻璃、渐变 CTA「新建配置」、导航项（激活竖条、tooltip、分组标题）、折叠按钮动画；折叠状态存 `settings.sidebar_collapsed`
- [ ] `app-layout.tsx`：§3 的骨架

**验证**：`npm run check`；`npm run screenshots`。

**审查关卡**：主会话读截图——侧栏折叠 / 展开 × 浅色 / 深色，与参考文档逐项核对（尺寸、激活态、CTA 渐变、背景网格与光晕、标题栏分隔线）。

**提交**：`feat(ui): app layout — titlebar, sidebar, background`

---

## P7 配置列表页

- [ ] 头部：分段 tab（全部 / 运行中 / 回收站）+ 工具按钮（搜索 Ctrl/Cmd+K、导入、导出）
- [ ] `components/data-table/`：粘性选择列 / 序号列 / 操作列、表头样式、骨架行、空状态、全选 indeterminate
- [ ] 列：`☐ / # / 指纹（平台图标 + 种子 mono）/ 名称（备注 tooltip）/ 代理 / 时区 / 分组 / 标签 / 状态 / 操作`
- [ ] 启动 / 停止按钮四态（未运行 / 启动中 / 运行中 / 停止中）、运行中行底色、启动 / 停止进度遮罩（用主题无关颜色，参考文档 §7.2）；操作下拉：编辑、复制、导出、删除
- [ ] 分页条；批量浮条（普通 / 回收站两套）
- [ ] 对话框：删除确认、永久删除（含「同时删除浏览器数据目录」复选，说明仅删默认目录）、移动分组、设置标签、搜索
- [ ] 导入 / 导出接 dialog 插件的打开 / 保存对话框

**验证**：`npm run check`；`npm run screenshots`（列表有数据、运行中、多选浮条、回收站、空状态，浅 / 深 × 中 / 英）。

**审查关卡**：主会话读截图核对参考文档 §7。

**提交**：`feat(profiles): list page with batch actions`

---

## P8 编辑器页

- [ ] 结构：顶部工具条 + `w-5/6 mx-auto` 主区 + 分段导航滚动 + 右侧 420px 摘要栏 + 底部悬浮操作栏（参考文档 §8）
- [ ] 分段：基本信息（名称、备注、内核、分组、标签）/ 指纹（种子 + 随机、平台、平台版本、品牌、品牌版本、CPU 核心数、GPU 段）/ 网络与语言（代理选择、语言、接受语言、时区、WebRTC UDP）/ 浏览器行为（启动 URL、headless + 风险提示）/ 高级（额外参数、user-data-dir 覆盖）
- [ ] 字段显隐由 `getArgSupport(kernelId)` 驱动；切换内核时展示 `validateProfileForKernel` 的警告，**不清空**失效字段的值
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
  - 浏览器数据根目录（目录对话框）
  - 语言、主题
  - 批量启动并发上限与间隔

**验证**：`npm run check`；`npm run screenshots`（三个页面，浅 / 深）。

**提交**：`feat(ui): groups, tags, proxies, settings`

---

## P10 批量、退出确认、收尾

- [ ] 批量启动 / 停止接通，`batch://progress` 用 sonner 显示进度
- [ ] `app://close-requested` → 确认框（显示运行中数量）→ `quitApp()`
- [ ] en 文案补全；`npm run lint` 中 `i18next/no-literal-string` 零告警（A10）
- [ ] 错误码 → i18n 文案映射全覆盖（design §5 错误约定）

**验证**：本机开发循环检查；真 Chromium `--ignored` 测试；`npm run screenshots`（退出确认框、批量进度）。

**提交**：`feat: batch operations and exit confirmation`

---

## P11 跨平台收尾与发布

CI 已在 P0b 建立，此阶段只做收尾。

- [ ] 复查所有 `cfg(target_os)` 分支：Windows（Job Object、`WM_CLOSE`、PE 版本、`lockfile`）、macOS（Overlay 标题栏、plist 探测），确认 CI 三平台的 test 覆盖了它们
- [ ] `.github/workflows/release.yml`：推送 `v*` tag 时三平台 `tauri build`，产物附到 GitHub Release（草稿）
- [ ] macOS 签名：CI 产物未签名，下载后可能被提示「已损坏」。至少配置 ad-hoc 签名（`bundle.macOS.signingIdentity: "-"`）；`macos-latest` 为 arm64，dmg 仅支持 Apple 芯片，是否加 x86_64 / universal 由用户决定
- [ ] 若 Actions 缓存超过 10 GB 上限导致频繁冷编译：`setup-rust-toolchain` 加 `cache-save-if: ${{ github.ref == 'refs/heads/main' }}`
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
