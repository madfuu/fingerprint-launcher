# Fingerprint Launcher

[fingerprint-chromium](https://github.com/adryfish/fingerprint-chromium) 的跨平台桌面 GUI 启动器（Linux / Windows / macOS）。

以「浏览器配置文件（profile）」为单位管理指纹种子、平台、品牌、语言、时区、代理等启动参数，并一键启动、停止、批量开窗。启动器不修改浏览器内核，只负责：

- 持久化配置（SQLite）
- 把配置翻译成 fingerprint-chromium 的命令行参数
- 管理 chrome 子进程的生命周期

fingerprint-chromium 本身不随本项目分发，需要自行下载并在设置中指定可执行文件路径。

> 状态：开发中。

## 技术栈

Tauri 2 · React 19 · TypeScript · Tailwind CSS 4 · shadcn/ui · SQLite

## 开发

前置：Node.js 22.22+ / 24.15+ / 26+ 与 npm、Rust 1.90+，以及 [Tauri 的系统依赖](https://tauri.app/start/prerequisites/)。

```bash
npm install
npm run tauri dev        # 启动桌面应用（开发模式）
npm run dev              # 仅前端（浏览器中运行）
npm run check            # typecheck + lint + test + build
npm run tauri build      # 打包

# Rust 检查前须先构建前端：generate_context!() 在编译期读取 dist/（npm run check 已包含构建）
cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

## 致谢

界面的视觉设计参考了 [Simprint](https://github.com/Simprint/simprint)。本项目代码为独立编写，不包含 Simprint 的源码或资源。

## 许可

[MIT](./LICENSE)
