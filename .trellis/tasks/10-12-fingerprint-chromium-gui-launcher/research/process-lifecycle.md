# chrome 子进程生命周期：三平台差异与陷阱

服务于 PRD R3 / R10.2 与 design §4。标「待真机核实」的条目在当前无头 VM 上无法验证。

## 1. Simprint 的做法（参照，不照抄）

文件：`/home/a/projects/simprint/src-tauri/crates/runtime/src/services/environment/kernel/`

- `launcher.rs`：`tokio::process::Command` 拉起，`stdin(Stdio::null())`；spawn 后 `tokio::spawn` 一个协程 `browser.wait().await`，退出时更新状态。另外用 `--remote-debugging-port` + 轮询 `/json/version` 判定就绪。
- `job.rs`：Windows 用 `CreateJobObjectW` + `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` + `AssignProcessToJobObject`；**非 Windows 是 no-op**（`create_and_assign` / `remove` 都是空实现）。

我们保留「waiter 协程驱动状态」与「Windows Job Object」，但：

- **Unix 不能 no-op**：用进程组补齐，否则 A6「无残留进程」在 Linux 上过不了。
- **不加 `--remote-debugging-port`**：它让本机任何进程都能接管浏览器，而且 fingerprint-chromium 特意做了 CDP 检测规避，默认开端口与此相悖。就绪判定改用「spawn 成功 + 早退检测」（§4）。如有自动化需求，可作为后续任务以 profile 级选项提供。

## 2. 进程树终止

chrome 会派生 GPU、网络、渲染器等大量子进程，只杀主进程会留孤儿。

### Unix（Linux / macOS）

- spawn 时 `tokio::process::Command::process_group(0)`（tokio 在 unix 上直接提供；std 对应 `CommandExt::process_group`，Rust 1.64 起稳定），chrome 成为新进程组组长，pgid = pid。
- 终止：用 `libc::killpg` 或 `nix::sys::signal::killpg`（需新增依赖其一）。

### Windows

- `CreateJobObjectW` + `SetInformationJobObject(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)`，spawn 后 `AssignProcessToJobObject`。
- 好处：启动器进程退出（含崩溃）时句柄关闭，整树连带终止。
- `windows` crate（0.61）features：`Win32_Foundation`、`Win32_Security`、`Win32_System_JobObjects`、`Win32_System_Threading`；版本探测另需 `Win32_Storage_FileSystem`（`GetFileVersionInfoW` / `VerQueryValueW`），`WM_CLOSE` 需 `Win32_UI_WindowsAndMessaging`。
- **`Win32_Security` 必须开**：`CreateJobObjectW` 的参数是 `SECURITY_ATTRIBUTES`，缺这个 feature 时函数不会生成，报 `E0425 cannot find function CreateJobObjectW`（2026-10-12 本机交叉检查实测）。
- 本机可交叉检查 Windows 分支：`x86_64-pc-windows-gnu` target + mingw-w64 已就绪；整棵 tauri + rusqlite(bundled) + windows 依赖树 check / clippy / 链接 exe 均通过（冷 check 77 s，完整链接 65 s）。

## 3. 先温和、后强制

**直接硬杀会丢登录态。** Chrome 的 Cookie 存在 SQLite 中、周期性落盘；硬杀会丢掉最后一段写入，用户感知为「刚登录的号又掉了」——对配置文件管理器是用户可见的缺陷。

| 平台 | 温和 | 宽限 | 强制 |
|---|---|---|---|
| Unix | `kill(pid, SIGTERM)`，**只发给主进程** | 5 s | `killpg(pgid, SIGKILL)` |
| Windows | 向主进程的顶层窗口投递 `WM_CLOSE`（`EnumWindows` + `GetWindowThreadProcessId` + `PostMessageW`），或 `taskkill /PID <pid> /T`（不带 `/F`） | 5 s | `TerminateJobObject` |

Chrome 在 Linux 上对 SIGTERM 做优雅关闭；Windows 上对 `WM_CLOSE` 同理。（待真机核实具体耗时，5 s 宽限是保守值，做成常量。）

SIGTERM 只发主进程而不发整组：主进程优雅退出时会自行收拾子进程；整组发送会让渲染器先于主进程死掉，徒增噪声。强制阶段才对整组下手。

## 4. 启动失败的可读化（R3.6）

### 4.1 stderr 必须持续读空

捕获 stderr（`Stdio::piped()`）后**必须**起协程持续读，否则 chrome 写满管道缓冲区（Linux 默认 64 KiB）后会阻塞——表现为浏览器"卡死"。

做法：协程逐行读 stderr，保留最后 50 行的环形缓冲区，进程整个生命周期都在读。可选地同时写入 `<app_log_dir>/profiles/<profile_id>.log`。

### 4.2 早退检测

进程在 spawn 后 **10 s 内退出且退出码非 0**，判定为启动失败：状态置 `Error`，错误信息带上环形缓冲区末尾若干行。

常见原因（待真机核实）：

- Linux 沙箱不可用：`No usable sandbox!`。Ubuntu 23.10+ 通过 AppArmor 限制非特权 user namespace，tar.xz 版 chromium 常见。用户可在 profile 的额外参数里加 `--no-sandbox`（R2.3）。错误信息里识别这个关键字，给出这条建议。
- AppImage 缺 FUSE：`dlopen(): error loading libfuse.so.2`。提示安装 libfuse2 或使用 tar.xz 版。
- 可执行位缺失：spawn 本身失败（`PermissionDenied`），在 spawn 前就能检查。

## 5. 单实例转交陷阱（R3.4）

Chromium 对同一个 `--user-data-dir` 只允许一个浏览器实例。第二次启动时，新进程会把命令行**转交给已在运行的实例然后立刻以 0 退出**。

后果：如果该 profile 已在启动器之外运行（用户手动开的、或启动器崩溃后留下的孤儿），启动器 spawn 的进程会秒退，waiter 把状态标成「已停止」，而浏览器实际还开着——状态完全错乱。

**对策：spawn 前检查锁。**

| 平台 | 锁 | 判定 |
|---|---|---|
| Linux / macOS | `<udd>/SingletonLock`，一个指向 `<hostname>-<pid>` 的符号链接 | 读链接目标，解析 pid，检查该 pid 是否存活（`kill(pid, 0)`）。主机名不同或 pid 已死 = 陈旧锁，可忽略 |
| Windows | `<udd>/lockfile`，运行中的实例以独占方式打开 | 尝试以 `share_mode(0)` 打开；失败（共享冲突）= 有实例在运行 |

检测到时拒绝启动，返回可读错误「该配置正在启动器外部运行」。不做「接管外部实例」，那是后续需求。（锁文件细节待真机核实。）

注意：Windows 有 Job Object kill-on-close，启动器崩溃不会留孤儿；Unix 没有等价机制，崩溃后会留孤儿，上面的锁检查正是这种情况下的兜底。

## 6. 应用退出（R3.5）

Tauri 2 有两条退出路径，都要拦：

1. **主窗口关闭**：自定义标题栏的关闭按钮调 `getCurrentWindow().close()`，触发 `WindowEvent::CloseRequested { api, .. }`。若有运行中的 profile：`api.prevent_close()` → emit `app://close-requested` → 前端弹确认框 → 用户确认后调 `quit_app` 命令：停止全部（§3 流程）→ `app.exit(0)`。
2. **应用级退出**（macOS Cmd+Q、托盘退出等）：`RunEvent::ExitRequested { api, .. }` → `api.prevent_exit()`，走同一确认流程。

`quit_app` 内部要防重入：确认流程进行中再次触发关闭时不重复弹框。

## 7. Tauri 2 命令线程模型

非 async 的 `#[tauri::command]` **在主线程执行**，会卡住 UI。所有涉及数据库、文件系统、进程的命令都声明为 `async`（或 `#[tauri::command(async)]`）。

SQLite：桌面单用户场景，`Mutex<rusqlite::Connection>` 足够，开 WAL。阻塞调用包在 `tokio::task::spawn_blocking` 里，不在 async 上下文直接持锁做 IO。不需要连接池。
