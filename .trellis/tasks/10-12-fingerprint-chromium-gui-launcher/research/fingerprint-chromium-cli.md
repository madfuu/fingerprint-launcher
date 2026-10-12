# fingerprint-chromium 命令行参数与内核形态

来源：`/home/a/projects/fingerprint-chromium/README-ZH.md`（上游 `adryfish/fingerprint-chromium`，最新提交 `698a1002 Update to Chromium 150.0.7871.186`）。
上游仓库只含文档，不含二进制；本机也没有任何 fingerprint-chromium 二进制。

## 1. 参数全表

| 参数 | 值 | 适用版本 | 说明 |
|---|---|---|---|
| `--fingerprint` | 32 位整数 | 全部 | 指纹种子。**启用后大部分指纹功能才生效**；不传则行为同原版 Chromium |
| `--fingerprint-platform` | `windows` / `linux` / `macos` | 全部 | |
| `--fingerprint-platform-version` | 字符串，如 `15.2.0` | 全部 | 不填用默认 |
| `--fingerprint-brand` | `Chrome` / `Edge` / `Opera` / `Vivaldi` / 自定义 | 131+ | 不填默认 Chromium |
| `--fingerprint-brand-version` | 字符串 | 131+ | 四个内置品牌各有默认版本 |
| `--fingerprint-hardware-concurrency` | 整数 | 全部 | 不填由种子派生 |
| `--disable-non-proxied-udp` | 开关 | 全部 | WebRTC 策略，**建议默认开** |
| `--lang` | 语言代码，如 `zh-CN` | 全部 | |
| `--accept-lang` | 逗号分隔，如 `zh-CN,en-US` | 全部 | |
| `--timezone` | IANA，如 `Asia/Shanghai`、`UTC` | 全部 | |
| `--proxy-server` | `http://h:p` / `socks5://h:p` | 全部 | **不支持密码验证** |
| `--fingerprint-gpu-vendor` | 字符串 | **139–143** | 144 移除 |
| `--fingerprint-gpu-renderer` | 字符串 | **139–143** | 144 移除 |
| `--disable-gpu-fingerprint` | 开关 | **142–143** | 144 移除，改用 `--disable-spoofing=gpu` |
| `--disable-spoofing` | CSV：`font,audio,canvas,clientrects,gpu` | **144+** | 选择性关闭伪装项 |
| `--user-data-dir` | 路径 | Chromium 原生 | 每个 profile 独立 |
| `--headless` | 开关 | Chromium 原生 | 见 §4 风险 |

## 2. 版本门控（design §3.2 的依据）

时间线：

- **131**：新增 `--fingerprint-brand` / `--fingerprint-brand-version`
- **133**：去除插件伪装，返回固定插件列表（无参数变化）
- **139**：`--fingerprint` 自动启用 GPU 指纹；新增 `--fingerprint-gpu-vendor` / `-renderer`
- **142**：新增 `--disable-gpu-fingerprint`；UA / 音频 / Canvas 实现更新
- **144**：**移除** `--fingerprint-gpu-vendor`、`--fingerprint-gpu-renderer`、`--disable-gpu-fingerprint`；**新增** `--disable-spoofing`

上游发布过的大版本：132 / 133 / 134 / 135 / 136 / 138 / 139 / 142 / 144 / 148 / 150。
低于 131 的版本不在本启动器支持范围内（品牌参数不存在）；遇到时按 131 以下处理：不生成品牌参数并在表单提示。

## 3. 官方示例（单测 fixture）

```bash
# 基本使用
./chrome --fingerprint=1000 --user-data-dir=/tmp/chromium/1000 \
  --timezone="America/Los_Angeles" --proxy-server="你的代理服务器地址"

# 定制 User-Agent
chrome.exe --fingerprint=2023 --fingerprint-platform=macos \
  --fingerprint-platform-version="15.2.0" --fingerprint-brand="Edge" \
  --user-data-dir=%TEMP%\chromium
```

注意：示例里的引号是 shell 层面的。`tokio::process::Command::arg` 直接传 argv，**不经 shell**，因此生成的参数里不能带引号：
`--timezone=America/Los_Angeles`，而不是 `--timezone="America/Los_Angeles"`。
摘要栏的「可复制命令行」才需要按目标 shell 加引号（POSIX 用单引号；Windows 用双引号），这是展示层的转义，和实际 spawn 的 argv 是两回事。

## 4. 上游明示的限制（UI 必须标注）

| 限制 | 原文要点 | UI 处理 |
|---|---|---|
| 代理无密码验证 | `http`, `socks` 代理（不支持密码验证） | 代理表单不提供用户名/密码字段，加说明文字 |
| Headless 不完整 | 只是将 UA 的 `HeadlessChrome` 改成 `Chrome`，其他 Headless 特征没有更改，谨慎使用 | 开关旁加风险提示 |
| WebGL 元数据 | 修改 GPU 供应商和显卡型号（暂时只支持 Linux） | GPU 字段旁标注「仅 Linux 生效」 |
| BrowserScan | GPU 指纹可能存在问题 | 不处理，`--disable-spoofing=gpu` 已可关闭 |

## 5. 指纹种子

- README 只说「32 位整数」。示例值为 1000、2023。
- 随机生成取 `1..=2147483647`（正的有符号 32 位），避免负数和无符号溢出在不同解析器上的歧义。
- 存 SQLite `INTEGER`。

## 6. 各平台的内核形态

下载地址形态（以 150 为例，均来自 README 下载表）：

| 平台 | 产物 | 可执行文件 |
|---|---|---|
| Linux | `ungoogled-chromium-150.0.7871.186-1-x86_64.AppImage` | AppImage 本身 |
| Linux | `ungoogled-chromium-150.0.7871.186-1-x86_64_linux.tar.xz` | 解压目录下的 `chrome` |
| Windows | `ungoogled-chromium_150.0.7871.186-1.1_installer_x64.exe` | 安装后的 `chrome.exe` |
| Windows | `ungoogled-chromium_150.0.7871.186-1.1_windows_x64.zip` | 解压目录下的 `chrome.exe` |
| macOS | `ungoogled-chromium_148.0.7778.215-1.1_macos.dmg` | `Chromium.app/Contents/MacOS/Chromium` |

macOS 并非每个版本都发布（150 / 144 / 139 / 138 / 134 / 133 没有 dmg）。

## 7. 版本探测（design §5 `probeKernel`）

**不能只靠 `--version`**：

| 平台 | 可靠手段 | 备注 |
|---|---|---|
| Windows | 读 `chrome.exe` 的 PE VERSIONINFO（`GetFileVersionInfoW` + `VerQueryValueW`） | Windows 上 `chrome.exe --version` **不向控制台输出任何内容**（GUI 子系统程序），这是 Chromium 的长期行为 |
| macOS | 读 `Chromium.app/Contents/Info.plist` 的 `CFBundleShortVersionString`；或执行二进制 `--version` | 两者都可用，读 plist 不需要启动进程 |
| Linux tar.xz | 执行 `chrome --version`，输出形如 `Chromium 150.0.7871.186 ...` | |
| Linux AppImage | 先从文件名解析 `ungoogled-chromium-<ver>-...AppImage`；失败再执行 `--version` | AppImage 运行需要宿主有 FUSE（libfuse2）；没有时可设 `APPIMAGE_EXTRACT_AND_RUN=1`，但每次都会解包，慢 |

全部失败时：要求用户手填大版本号，**不猜**。版本号只取主版本（`150`）用于门控，完整版本串仅展示。

## 8. 自动探测候选位置（R7.1，需在真机核实）

只列候选、让用户确认，不自动采用——用户机器上可能同时装有普通 Chrome / Chromium，无法可靠区分是否为 fingerprint-chromium。

- Linux：`~/Applications/*.AppImage`、`~/Downloads/*ungoogled-chromium*.AppImage`、`~/Downloads/ungoogled-chromium-*_linux/chrome`、`/opt/ungoogled-chromium*/chrome`
- Windows：`%LOCALAPPDATA%\Chromium\Application\chrome.exe`（ungoogled-chromium 安装器按用户安装的默认位置，待核实）、`%PROGRAMFILES%\Chromium\Application\chrome.exe`
- macOS：`/Applications/Chromium.app/Contents/MacOS/Chromium`、`~/Applications/Chromium.app/Contents/MacOS/Chromium`
