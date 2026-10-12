# fingerprint-chromium GUI 启动器

## Goal

为 [adryfish/fingerprint-chromium](https://github.com/adryfish/fingerprint-chromium) 构建一个跨平台桌面 GUI 启动器：参照 [Simprint](https://github.com/Simprint/simprint) 的外观骨架，让用户以「浏览器配置文件（profile）」为单位管理 fingerprint-chromium 的指纹 / 代理 / 语言 / 时区等命令行参数，并一键启动、停止、批量开窗。

启动器自身**不修改浏览器内核**，它只做三件事：持久化配置、把配置翻译成命令行参数、管理 chrome 子进程的生命周期。

## 技术决策（已确认）

| 项 | 决定 | 备注 |
|---|---|---|
| 外壳 | Tauri 2 | 与 Simprint 同构 |
| 前端 | React 19 + TypeScript | 原定 React 18，已改为 19 与参考仓库对齐 |
| 样式 | Tailwind 4 + shadcn/ui（new-york, baseColor=neutral, 蓝色主色） | 复刻 Simprint 主题变量 |
| 数据 | SQLite | 经 Rust 侧访问 |
| i18n | react-i18next，中英双语，默认 zh-CN | 按 feature 拆 resources |
| 目标平台 | Linux 桌面 / Windows / macOS | 三平台都要可用 |
| 参照程度 | 完整复刻 Simprint 外观骨架 | titlebar + 可折叠图标侧栏 + 密集小字表格 + 右侧摘要栏 + canvas 背景 |
| 复刻方式 | **视觉复刻，代码自写** | Simprint 为 AGPLv3；不拷贝其源码与资源，本项目不承接 AGPL |
| 许可证 | MIT（默认，用户可改） | |

## 功能需求

### R1 配置文件管理

- R1.1 profile 的增删改查，字段至少含：名称、备注、指纹种子、平台、品牌、语言、时区、代理、user-data-dir、启动 URL、所属分组、标签、绑定内核版本。
- R1.2 列表页复刻 Simprint 环境列表：分段视图切换（全部 / 运行中 / 回收站）、搜索（Ctrl+K）、分页、行内快捷操作（启动 / 停止 / 编辑 / 复制 / 删除）。
- R1.3 软删除进回收站，支持恢复与永久删除。
- R1.4 复制 profile 时生成新的 user-data-dir 与新指纹种子，不复用原目录。
- R1.5 导入 / 导出 profile 为 JSON，用于备份和跨机迁移。

### R2 指纹配置表单

- R2.1 表单覆盖 fingerprint-chromium 的全部启动参数：

  | 参数 | 类型 | 说明 |
  |---|---|---|
  | `--fingerprint` | 32 位整数 | 指纹种子，启用后大部分指纹功能生效；支持随机生成 |
  | `--fingerprint-platform` | `windows` / `linux` / `macos` | |
  | `--fingerprint-platform-version` | 字符串 | 留空用默认 |
  | `--fingerprint-brand` | `Chrome` / `Edge` / `Opera` / `Vivaldi` / 自定义 | 留空默认 Chromium |
  | `--fingerprint-brand-version` | 字符串 | |
  | `--fingerprint-hardware-concurrency` | 整数 | 留空由种子随机生成 |
  | `--disable-non-proxied-udp` | 开关 | WebRTC 策略，默认开启 |
  | `--lang` | 语言代码 | 如 `zh-CN` |
  | `--accept-lang` | 语言代码列表 | 如 `zh-CN,en-US` |
  | `--timezone` | IANA 时区 | 如 `Asia/Shanghai` |
  | `--proxy-server` | http / socks URL | |
  | `--disable-spoofing` | 多选 `font,audio,canvas,clientrects,gpu` | 仅 Chrome 144+ |
  | `--fingerprint-gpu-vendor` | 字符串 | 仅 Chrome 139–143，144 已移除 |
  | `--fingerprint-gpu-renderer` | 字符串 | 仅 Chrome 139–143，144 已移除 |
  | `--disable-gpu-fingerprint` | 开关 | 仅 Chrome 142–143，144 已移除 |
  | `--user-data-dir` | 路径 | 每个 profile 独立目录 |
  | `--headless` | 开关 | 只改 UA 中的 HeadlessChrome，其他 headless 特征未处理，UI 需标注风险 |

- R2.2 **参数集随内核版本变化**：profile 绑定一个内核大版本号，表单据此显示/隐藏并校验参数（144+ 显示 `--disable-spoofing` 且隐藏三个 GPU 参数；139–143 反之）。切换版本时对失效字段给出明确提示，不静默丢数据。
- R2.3 支持追加用户自定义的额外命令行参数（自由文本），拼接在生成参数之后。
- R2.4 右侧摘要栏实时预览最终命令行，可一键复制。
- R2.5 「随机指纹」可一次性重掷种子及由种子派生的字段。

### R3 启动与进程管理

- R3.1 单个 profile 一键启动，启动后状态变为「运行中」并记录 PID 与启动时间。
- R3.2 停止运行中的 profile，需真正终止进程树（不留孤儿进程）。
- R3.3 用户手动关闭浏览器窗口后，启动器需在数秒内感知并把状态改回「未运行」。
- R3.4 同一 profile 不允许重复启动（user-data-dir 冲突），已运行时启动按钮置为停止。
- R3.5 启动器退出时，询问并清理其拉起的所有 chrome 进程，不残留。
- R3.6 启动失败（内核路径不存在、不可执行、端口占用等）给出可读的错误原因，而非静默失败。

### R4 批量操作

- R4.1 列表支持多选（含全选当前页），底部出现批量操作栏。
- R4.2 批量启动，带并发上限与启动间隔，避免瞬间拉起几十个进程打满机器。
- R4.3 批量停止、批量移动分组、批量打标签、批量删除。

### R5 分组与标签

- R5.1 分组为单选归属，支持增删改查；侧栏可按分组筛选。
- R5.2 标签为多选，带颜色，支持增删改查与按标签筛选。

### R6 代理管理

- R6.1 代理可独立维护成列表（协议 / 主机 / 端口 / 备注），profile 引用代理条目，而非每个 profile 各写一份。
- R6.2 支持 http 与 socks5。**fingerprint-chromium 的 `--proxy-server` 不支持带密码验证的代理**，UI 必须明确说明这一限制，不提供会静默失效的用户名密码字段。
- R6.3 提供代理连通性测试（出口 IP 与地理位置回显）。

### R7 设置

- R7.1 内核路径设置：手动指定可执行文件（文件选择器），并在常见安装位置自动探测候选项供用户确认。支持配置多个内核路径并标注版本号。
- R7.2 user-data-dir 根目录设置，profile 目录在其下按 profile id 创建。
- R7.3 界面语言（中 / 英）与主题（浅色 / 深色 / 跟随系统）。
- R7.4 批量启动的并发上限与间隔。

### R8 UI 复刻

- R8.1 外观骨架完整复刻 Simprint：`h-12` titlebar（品牌 / 导航 / 状态 / 窗口控制四槽位）、可折叠图标侧栏（`w-16` ↔ `w-58`，300ms 过渡，折叠态 tooltip）、主区 `border-l`、canvas 网格 + 点阵 + 噪点 + 蓝色角落光晕背景。
- R8.2 复刻其交互细节：活动导航项 `bg-primary/15 text-primary` 配右边缘 primary 竖条、顶部渐变 CTA 按钮、`bg-secondary rounded-xl` 分段 tab、`h-8 px-3 text-xs` 工具条按钮、6px 细滚动条。
- R8.3 复刻其排版密度：`text-xs` / `text-[11px]` / `text-[10px]`、`uppercase tracking-wider` 的 muted 标签、`font-mono` 数值。
- R8.4 表单页复刻其创建环境页结构：顶部工具条、内容区 `w-5/6 mx-auto`、中间分段滚动表单 + 分段跳转、右侧 `w-[420px] border-l` 摘要栏、底部 `backdrop-blur-2xl` 悬浮操作栏。
- R8.5 复刻其主题变量（primary `#3b82f6`、radius `0.375rem`、浅色 `#ffffff` / 深色 `#171717`、card 深色 `#262626`），浅色与深色都要完整可用。

### R9 国际化

- R9.1 全部界面文案走 react-i18next，按 feature 拆 resources，默认 zh-CN，可切 en，无硬编码裸文案。

### R10 跨平台

- R10.1 Linux / Windows / macOS 三平台的内核路径形态差异（`./chrome`、`*.AppImage`、`chrome.exe`、`Chromium.app/Contents/MacOS/Chromium`）都要能正确识别与启动。
- R10.2 进程树终止在三平台都要生效（Windows Job Object、Unix 进程组）。
- R10.3 三平台的打包产物都能构建（CI 矩阵）。

## 非目标（本任务不做）

- **内核版本下载管理**——探测已安装版本、从 GitHub Release 下载 AppImage/ZIP/dmg、按 profile 绑定版本。已确认拆为后续子任务；本任务只做「手动指定内核路径 + 自动探测候选」。
- RPA / 自动化流程编排、MCP 服务、Local API。
- 多窗口同步器（Syncer）。
- 团队协作、云同步、账号体系、计费。
- 浏览器扩展管理、Cookie 导入导出。
- 带密码验证代理的本地中继（需要内置 mihomo 一类的转发器）。

## 验收标准

- [ ] A1 `npm run tauri build` 在 Linux 上产出可运行的安装包；Windows / macOS 的构建配置就位并在 CI 中通过。
- [ ] A2 新建一个 profile，填写指纹种子 / 平台 / 品牌 / 语言 / 时区 / 代理后保存，重启应用数据仍在（SQLite 持久化生效）。
- [ ] A3 摘要栏预览出的命令行与 R2.1 表格逐项对应；对照 fingerprint-chromium README 的两个官方示例，能一字不差地复现其参数组合。
- [ ] A4 profile 绑定 Chrome 144 时表单出现 `--disable-spoofing` 且无 GPU 三参数；绑定 Chrome 142 时相反；版本切换对失效字段有提示。
- [ ] A5 启动一个 profile 后状态为「运行中」并显示 PID；手动关闭浏览器窗口后状态在 5 秒内回到「未运行」。
- [ ] A6 停止一个 profile 后，系统中不存在该 profile 的残留 chrome 进程。
- [ ] A7 关闭启动器后，其拉起的所有 chrome 进程都已退出。
- [ ] A8 选中 10 个 profile 批量启动，按设定的并发上限与间隔依次拉起，不一次性全开。
- [ ] A9 两个 profile 使用各自独立的 user-data-dir，互不共享登录态。
- [ ] A10 界面在中英之间切换，无残留硬编码文案；浅色与深色主题都完整可用。
- [ ] A11 UI 与 Simprint 并排对比，R8 所列的骨架、交互细节、排版密度、主题变量均一致。
- [ ] A12 内核路径填错（不存在 / 不可执行）时，启动给出可读错误，应用不崩溃。
- [ ] A13 `npm run lint`、`tsc --noEmit`、`cargo clippy`、`cargo fmt --check` 全部通过。
- [ ] A14 仓库中不含 Simprint 的源文件与资源（logo、字体等），不打包任何字体文件。

## 约束与已知风险

- **当前开发机是无头云 VM**（无 DISPLAY，仅有 Xvfb），且没有 fingerprint-chromium 二进制。本机能验证的范围（详见 design §10）：
  - A3 / A4：参数构建纯函数测试
  - A5 / A6 / A7 / A9 / A12：用假内核脚本 + 本机自带的 Chrome for Testing 151（普通 Chromium，会忽略 `--fingerprint*` 开关）测进程生命周期
  - A10 / A11 的大部分：前端在 mock 模式下用 Chromium 逐页截图核对；ESLint 规则机检硬编码文案
  - A13、A14
- **GitHub Actions（权威 CI）能验证的范围**：三平台安装包构建（A1）；Windows / macOS 原生 runner 上的 Rust 编译与单元测试，包括 Windows Job Object 整树终止、macOS 进程组行为（R10.1 / R10.2 的可自动化部分）。
- **仍需在真机上验收**：fingerprint-chromium 的指纹参数是否真实生效；WebKitGTK 与 Chromium 的渲染差异；Windows / macOS 平台特有的进程管理、版本探测、标题栏与安装包。
- `--headless` 只改写 UA 中的 `HeadlessChrome` 字样，其余 headless 特征未处理，UI 必须标注此风险。
- WebGL 元数据伪装（GPU vendor / renderer）按上游说明目前仅 Linux 生效，UI 需标注。
- 代理不支持密码验证是上游内核的限制，不是本启动器可以绕过的。

## Notes

- 环境（2026-10-12 就绪）：
  - Tauri 的 Linux 系统依赖已由用户安装（webkit2gtk-4.1 2.54.0 / libsoup-3.0 3.6.5 / gtk3 3.24.49）
  - Rust stable 1.99.0 + clippy + rustfmt，经 USTC 镜像安装；`~/.cargo/bin` 不在默认 PATH，命令前需 `. ~/.cargo/env`
  - 本机连不上 `static.rust-lang.org` 与 crates.io（Fastly 节点不可达），已在 `~/.cargo/config.toml` 配置 USTC sparse 镜像
  - Node 26.8.1 / npm 11.19.0（npm registry 可达）
- 参考仓库临时克隆在 `/home/a/projects/simprint`，版本与重新拉取方法见 `research/simprint-ui-reference.md` §0。本任务不采用它的 slotkit 插件架构，只以其外观与布局为视觉参照。
- 后续任务（已确认，本任务不做）：内核版本下载管理。
- fingerprint-chromium 文档仓库在 `/home/a/projects/fingerprint-chromium`（仅文档，无二进制），参数表以 `README-ZH.md` 为准。
