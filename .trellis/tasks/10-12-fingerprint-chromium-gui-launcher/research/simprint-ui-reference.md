# Simprint UI 复刻参考

服务于 PRD R8 与 design §7。

## 0. 参考仓库

- 本地：`/home/a/projects/simprint`（用户临时拉取，**可能被删除**）
- 远端：`https://github.com/Simprint/simprint`
- 版本：`d2dc939bfae8efda8e4e4d791587d7a364ca4dd8`（2026-08-24，`refactor: keep only production Cargo feature`）
- 本地目录不存在时重新拉取并检出该提交：
  ```bash
  git clone https://github.com/Simprint/simprint.git /home/a/projects/simprint
  git -C /home/a/projects/simprint checkout d2dc939bfae8efda8e4e4d791587d7a364ca4dd8
  ```
- 技术栈：Tauri 2.9.5 + React 19.2 + Vite 7 + Tailwind 4.1 + shadcn/ui（new-york / neutral / lucide）+ radix-ui + zustand 5 + react-router 7 + i18next 25 + sonner
- **许可：AGPLv3**。见 §9。

## 1. 全局主题 — `src/index.css`

### 1.1 设计变量

| 变量 | 浅色 | 深色 |
|---|---|---|
| `--background` | `#ffffff` | `#171717` |
| `--foreground` | `#333333` | `#e5e5e5` |
| `--card` / `--popover` | `#ffffff` | `#262626` |
| `--primary` | `#3b82f6` | `#3b82f6` |
| `--primary-foreground` | `#ffffff` | `#ffffff` |
| `--secondary` | `#f3f4f6` | `#262626` |
| `--secondary-foreground` | `#4b5563` | `#e5e5e5` |
| `--muted` | `#f9fafb` | `#1f1f1f` |
| `--muted-foreground` | `#6b7280` | `#a3a3a3` |
| `--accent` | `#e0f2fe` | `#1e3a8a` |
| `--accent-foreground` | `#1e3a8a` | `#bfdbfe` |
| `--destructive` | `#ef4444` | `#ef4444` |
| `--border` / `--input` | `#e5e7eb` | `#404040` |
| `--ring` | `#3b82f6` | `#3b82f6` |
| `--sidebar` | `#f9fafb` | `#171717` |
| `--sidebar-accent` | `#e0f2fe` | `#1e3a8a` |
| `--sidebar-border` | `#e5e7eb` | `#404040` |
| `--radius` | `0.375rem` | 同 |

`--chart-1..5` 为蓝色阶；阴影变量 `--shadow-2xs..2xl` 两套主题相同（`hsl(0 0% 0% / 0.05~0.25)`）。
`@theme inline` 把上述变量映射为 `--color-*`，`--radius-sm/md/lg/xl` = `radius - 4px / - 2px / radius / + 4px`。
深色通过 `.dark` 类切换（`@custom-variant dark (&:is(.dark *))`）。

### 1.2 基础层

- `* { @apply border-border outline-ring/50 }`，`body { @apply bg-background text-foreground }`
- 滚动条：宽高 `6px`，track 透明，thumb `var(--border)` 圆角 `9999px`，hover 时 `var(--muted-foreground)`
- 工具类 `.scrollbar-hidden`

### 1.3 字体 — ⚠️ 不要照抄

Simprint 在 `public/fonts/` 打包了 `PingFang-SC-Regular.ttf` 与 `AlimamaShuHeiTi.ttf`。**PingFang SC 是 Apple 专有字体，不允许再分发。** 本项目不打包任何字体文件，只按名称引用系统字体：

```css
--font-sans: "PingFang SC", "Microsoft YaHei", "Noto Sans CJK SC", Inter, system-ui, sans-serif;
--font-mono: "JetBrains Mono", ui-monospace, "SFMono-Regular", Menlo, Consolas, monospace;
```

### 1.4 自定义动画 — `plugins/layouts/app-layout/src/styles.css`

侧栏依赖这些 keyframes（Tailwind 不内置），通过任意值类 `animate-[name_dur_ease_iter]` 使用：

| 名称 | 效果 | 用处 |
|---|---|---|
| `fadeIn` | 透明度 0→1 | 侧栏占位 |
| `pulse-gentle` | 透明度 0.7↔1 + scale 1↔1.05 | 折叠按钮 |
| `slide-hint` | translateX 0↔6px | 折叠按钮图标 |
| `slideInRight` | translateX(-100%)→0，opacity 0.9→1 | 侧栏展开/折叠后内容入场 |
| `shake`（environment-manager） | 左右 ±4px 抖动 0.5s | 行操作被拒（如重复启动） |

同文件还有：`[data-slot="dialog-overlay"] { top: 48px !important }`——**对话框遮罩不盖住 titlebar**，这样弹框时仍可拖动、最小化窗口。要复刻。

## 2. 窗口 — `src-tauri/src/services/window/mod.rs:272`

```rust
WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
    .inner_size(1438.0, 828.0)
    .min_inner_size(1038.0, 688.0)
    .resizable(true).maximizable(true).center()
    .decorations(false)       // 自绘标题栏
    .visible(false)           // 前端就绪后再 show，避免白屏闪烁
    .drag_and_drop(false)
```

`tauri.conf.json` 里 `"windows": []`，窗口由代码创建。

**macOS 注意**：Simprint 只发布 Windows（bundle target 仅 `nsis`），它的窗口控制按钮是 Windows 风格。`decorations(false)` 在 macOS 上会连红绿灯一起去掉。本项目在 macOS 上改用 `title_bar_style(TitleBarStyle::Overlay)` + `hidden_title(true)` 保留原生红绿灯，隐藏自绘的三个按钮，并给品牌区左侧留出约 78px。

## 3. 布局骨架 — `plugins/layouts/app-layout/src/index.tsx`

```
div.relative.flex.flex-col.h-screen.w-full.min-w-0.overflow-hidden
├─ AppBackground                      fixed inset-0, z-index:-10, pointer-events-none
└─ div.relative.z-0.flex.flex-col.h-full.w-full.min-w-0.overflow-hidden
   ├─ AppTitlebar                     h-12
   └─ div.flex.flex-1.min-h-0.overflow-hidden.min-w-0
      ├─ AppSidebar
      └─ main.flex-1.flex.flex-col.overflow-hidden.min-w-0.border-l.border-sidebar-border/80
         └─ div.flex-1.overflow-hidden.min-w-0 → <Outlet/>
```

## 4. 背景 — `plugins/layouts/app-layout/src/components/app-background.tsx`

两层 canvas，按 `devicePixelRatio` 缩放，监听 `resize` 和系统主题变化重绘：

1. **底层**：纯色（深 `#0a0a0b` / 浅 `#ffffff`）→ 6px 网格线（`lineWidth 0.5`，深 `rgba(255,255,255,0.06)` / 浅 `rgba(0,0,0,0.03)`）→ 24px 点阵（半径 0.8，alpha 0.012）→ 随机 6% 像素加 ±1.25 噪点
2. **光晕层**（`filter: blur(40px)`）：左下 `(0.1w, 0.9h)` 与右下 `(0.9w, 0.9h)` 两个径向渐变，半径 `0.4 × min(w,h)`，不透明度深 0.15 / 浅 0.09；颜色深 `147,197,253`（sky-300）/ 浅 `37,99,235`（blue-600）；色标 0 / 0.2 / 0.5 / 0.8 / 1 对应 1 / 0.7 / 0.4 / 0.15 / 0

缺陷：每次 `resize` 都对全屏做 `getImageData`/`putImageData` 噪点处理，大窗口拖拽缩放时会卡。实现时给 resize 加 debounce（~150ms）。

## 5. 标题栏 — `.../components/titlebar/`

容器：`data-tauri-drag-region h-12 bg-background border-b border-border/80 flex items-center justify-between select-none shrink-0`

| 槽位 | 样式要点 |
|---|---|
| 品牌 | `flex items-center gap-3 px-5 border-r border-border/80 h-full`；logo `w-7 h-7`；名称 `text-xs font-semibold tracking-tight` |
| 导航（面包屑） | `flex items-center gap-2.5 px-5 text-xs text-muted-foreground`：`Home` 图标 → `ChevronRight`（opacity-50）→ 当前页图标 → 当前页名；图标都是 `w-3 h-3 opacity-70` |
| 状态 | 本项目放主题切换、语言切换（Simprint 这里是工作区/消息/用户菜单，不要） |
| 窗口控制 | 拖拽把手 `w-12 h-full border-l border-border/80 cursor-grab`（`GripVertical`）；最小化/最大化/关闭各 `w-12 h-full`，`text-muted-foreground/80 hover:bg-accent/80 hover:text-foreground`；**关闭按钮 hover 为 `hover:bg-destructive/90 hover:text-destructive-foreground`**；图标 `Minus` / `Square(w-3.5)` / `X` |

窗口控制调用 `getCurrentWindow().minimize() / toggleMaximize() / close()`。

## 6. 侧栏 — `.../components/app-sidebar.tsx`

- 宽度：折叠 `w-16`，展开 `w-58`；`transition-[width] duration-300 ease-out px-1`
- 毛玻璃：`before:absolute before:inset-0 before:bg-background/10 before:backdrop-blur-2xl before:-z-10`
- 切换时内容先 `opacity-0`，300ms 后以 `animate-[slideInRight_300ms_ease-in-out]` 入场
- 默认折叠（来自设置 `defaultCollapseSidebar`）→ 本项目存 `settings.sidebar_collapsed`

**结构**（自上而下）：

1. 顶部 CTA「新建配置」：`rounded-lg mb-2 bg-linear-to-r from-blue-500 via-blue-600 to-indigo-600 hover:from-blue-600 hover:via-blue-700 hover:to-indigo-700 text-white font-semibold shadow-sm hover:shadow-md`；展开时 `flex-row gap-3 px-3 py-4`，折叠时 `flex-col items-center py-3.5`；展开时右上角有装饰大图标 `absolute -right-2 -top-2 h-16 w-16 text-sidebar-primary-foreground/10`（Simprint 用 react-icons `TfiWorld`，可用 lucide `Globe` 代替；主图标 `VscGitPullRequestCreate` 可用 lucide `GitPullRequestCreate` / `SquarePlus` 代替）
2. 顶部导航项（列表、分组）
3. 折叠态分隔线 `mx-3 border-b border-sidebar-border`
4. 分组导航（`ScrollArea flex-1 m-1`）：分组标题 `px-3 py-1 text-[10px] font-semibold text-sidebar-foreground/50 uppercase tracking-wider`（仅展开态）
5. 底部导航项（设置）
6. 折叠按钮：`absolute bottom-0 -right-6`，图标 `-rotate-45`，`animate-[pulse-gentle_2s_ease-in-out_infinite]` + 图标 `animate-[slide-hint_2s_ease-in-out_infinite]`

**导航项**：

- 容器 `group relative`，折叠 `w-11` / 展开 `w-full`
- 激活指示条：`absolute -right-1 top-1/2 h-6 w-1 -translate-y-1/2 rounded-2xl bg-primary`
- 激活：`bg-primary/15 text-primary font-semibold`；未激活：`text-sidebar-foreground/80 hover:bg-accent/60 hover:text-foreground`
- 行：`rounded-lg h-11 text-sm`，`Link` 展开 `pl-3 pr-3 gap-3` / 折叠 `justify-center px-3 w-11`
- 图标 `h-4 w-4`（未激活 `opacity-80`），外包 `Tooltip side="right"`
- 文字 `text-xs h-4 font-semibold`，折叠时 `max-w-0 hidden`

本项目导航：新建配置（CTA）→ 配置列表 `/` → 分组与标签 `/groups` → 代理 `/proxies` → 设置 `/settings`（底部）。

## 7. 列表页 — `plugins/pages/environment-manager/src/components/`

页面容器：`flex flex-col h-[calc(100vh-50px)]`。⚠️ titlebar 实际是 48px（`h-12`），这个 50 是魔数；本项目用 flex 撑满，不写死。

### 7.1 头部 — `environment-header.tsx`

`header.border-b.border-border.flex.items-center.justify-between.px-4.py-2.bg-background/10.backdrop-blur-2xl`

- 左：分段 tab 容器 `flex items-center gap-2 p-1 bg-secondary rounded-xl`；tab `px-3 py-1.5 text-xs rounded-xl transition-all duration-200`，激活 `bg-primary text-primary-foreground`，否则 `text-muted-foreground hover:text-foreground hover:bg-muted`
- 右：工具按钮 `h-8 px-3 flex items-center gap-2 rounded-md border border-border bg-background hover:bg-muted transition-colors text-muted-foreground hover:text-foreground text-xs font-medium`
- 搜索按钮：`Search` 图标 + 竖分隔 `h-3 w-px bg-border` + `text-[10px] font-medium opacity-60` 的「Ctrl+K」；全局监听 Ctrl/Cmd+K 打开搜索对话框

### 7.2 表格 — `src/components/data-table/`

- 外框：`flex-1 flex flex-col bg-background/10 backdrop-blur-2xl m-4 border border-border min-h-0 rounded-lg overflow-auto`；`table.w-full.border-collapse.table-auto`，`minWidth: max-content`
- 表头单元格：`sticky top-0 bg-muted z-5 text-left px-4 py-3 text-[11px] font-bold uppercase tracking-wider text-muted-foreground border-b`
- 粘性列：选择列（48px）与序号列粘左，操作列粘右；右粘列带左侧阴影线 `before:w-px before:bg-border before:shadow-[1px_0_2px_rgba(0,0,0,0.05)]`
- 全选框支持 indeterminate
- 加载时渲染骨架行（8 行）；空数据 `px-4 py-8 text-center text-muted-foreground`
- 单元格：序号 `text-xs text-muted-foreground font-mono`；名称 `font-medium`（有备注时 hover 出 tooltip）；时间 `text-xs text-muted-foreground font-mono`
- 启动按钮：`px-3 py-1 text-[11px] font-bold rounded-md border flex items-center gap-1.5`；未运行 `bg-primary text-primary-foreground border-primary hover:bg-primary/90`；运行中 `bg-destructive ...`；忙碌 `bg-muted text-muted-foreground border-muted cursor-not-allowed`；文案「启动 / 停止 / 启动中… / 关闭中…」
- 启动进度遮罩：从左向右覆盖整行 `absolute inset-y-0 left-0 bg-blue-100/40 transition-[width] duration-[1500ms]`；停止进度从右向左 `bg-orange-100/40 duration-[800ms]`

**不要照抄的缺陷**：

- 运行中行底色 `bg-green-50/50` 写死浅色，深色主题下突兀 → 改用主题无关的 `bg-emerald-500/5`
- 进度遮罩 `bg-blue-100/40`、`bg-orange-100/40` 同理 → 改 `bg-primary/10`、`bg-orange-500/10`
- 按钮图标用 `<i className="fa-solid fa-play">`，但 **Font Awesome 从未加载**，实际不显示 → 用 lucide `Play` / `Square`
- 混有硬编码中文（「恢复」「永久删除」「拖拽移动窗口」「最小化」）→ 全部走 i18n（PRD R9）

本项目列：`☐ / # / 指纹 / 名称 / 代理 / 时区 / 分组 / 标签 / 状态 / 操作`。

### 7.3 分页 — `environment-pagination.tsx`

`flex items-center justify-between px-6 py-2 border-t border-border bg-background/10 backdrop-blur-2xl`；左侧「共 N 条」+ 每页条数 `Select`（`h-7 w-[70px] text-xs`），右侧 shadcn `Pagination`。

### 7.4 批量操作浮条 — `environment-batch-actions.tsx`

`absolute bottom-6 left-1/2 -translate-x-1/2 bg-background/95 backdrop-blur-xl px-3 py-2 flex items-center gap-1.5 shadow-lg border border-border/50 z-100 rounded-lg animate-in fade-in slide-in-from-bottom-4 duration-300`

- 按钮：`text-xs font-semibold hover:text-primary hover:bg-primary/10 flex items-center gap-2 px-3 py-1.5 rounded-md`；危险操作 `text-destructive hover:bg-destructive/15`
- 分隔 `w-px h-5 bg-border/50 mx-1`；末尾关闭按钮 `h-7 w-7`
- 回收站视图下换成「批量恢复 / 批量永久删除」

## 8. 编辑器页 — `plugins/pages/create-window/src/components/`

- 顶部工具条 + 主区 `flex flex-1 overflow-hidden w-5/6 mx-auto`
- 中间：`ScrollArea flex-1`，内层 `py-4 space-y-4 pb-24 pr-4`，每个分段一个带 `id` 的 div，配合分段导航滚动跳转（`use-scroll-navigation.ts`）
- **分段卡片**：`space-y-4 p-4 border border-border rounded-lg`，标题 `h3.text-sm.font-semibold`
- **字段行**：两列网格 `grid grid-cols-2 gap-x-12 gap-y-4`，每格 `flex items-center justify-between gap-4`，`Label.text-xs.shrink-0`
- **药丸分段选择**：`inline-flex rounded-full bg-muted p-0.5`
- 右侧摘要：`w-[420px] border-l border-border overflow-y-auto` → 内层 `p-4 space-y-4` → 卡片 `relative space-y-4 p-4 border border-border rounded-lg overflow-hidden`，右下角水印 `Fingerprint` 图标 `absolute -bottom-12 -right-12 w-48 h-48 text-muted-foreground/5`；标题行 `text-sm font-semibold` + 右侧随机按钮（`Shuffle`，`h-7 w-7`）；分组小标题 `text-xs font-medium text-muted-foreground`；键值行 `flex justify-between gap-2`，键 `text-muted-foreground shrink-0`，值 `truncate text-right`（数值加 `font-mono`）
- 底部悬浮栏：`absolute bottom-0 inset-x-0 bg-background/10 border-t border-border py-3 z-10 backdrop-blur-2xl`，内层 `w-5/6 mx-auto flex items-center justify-between`；左侧分组/标签按钮（`variant="outline" size="sm"`），右侧主按钮「保存」

本项目编辑器分段：基本信息 / 指纹 / 网络与语言 / 浏览器行为 / 高级（额外参数）。摘要栏在指纹摘要卡片下方加「命令行预览」卡片（等宽字体、可复制）。

## 9. 许可

Simprint 为 **AGPLv3**。直接移植其源文件会让本项目承接 AGPLv3。本项目的处理方式见 design §9（待用户决定）。

无论哪种方式：

- shadcn/ui 组件用 shadcn CLI 重新生成（MIT），不从 Simprint 拷贝
- 不打包 Simprint 的 logo、字体等资源

## 10. 不移植的部分

slotkit 插件架构与扩展点注册、工作区/团队/消息/用户菜单、mihomo、会话锁、自动更新、MSW、Syncer、RPA、MCP、Local API、账号中心、审计页。
