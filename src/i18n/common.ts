/** Shared copy: navigation, actions, statuses, error codes and generated UI primitives. */
import type { ErrorCode, FailureHint, Locale, ProfileStatus, Theme } from '@/lib/bindings';

import { defineMessages } from './define';

export const common = defineMessages(
  {
    nav: {
      profiles: '配置列表',
      newProfile: '新建配置',
      editProfile: '编辑配置',
      groups: '分组与标签',
      proxies: '代理',
      settings: '设置',
    },
    actions: {
      save: '保存',
      cancel: '取消',
      confirm: '确定',
      close: '关闭',
      retry: '重试',
      reload: '重新加载',
      copy: '复制',
    },
    status: {
      stopped: '未运行',
      starting: '启动中…',
      running: '运行中',
      stopping: '关闭中…',
      error: '启动失败',
    } satisfies Record<ProfileStatus, string>,
    errors: {
      not_found: '记录不存在或已被删除。',
      conflict: '与现有数据冲突：名称、路径或浏览器数据目录已被占用。',
      invalid_input: '输入或请求无效。',
      db: '数据库错误。',
      io: '文件读写失败。',
      no_kernel: '尚未配置内核：请先在设置中添加 fingerprint-chromium 内核。',
      kernel_missing: '内核可执行文件不存在，请检查内核路径。',
      kernel_not_executable: '内核路径不是可执行文件（Linux / macOS 上可能缺少执行权限）。',
      profile_running: '该配置正在运行，请先停止。',
      profile_running_externally: '该配置的浏览器数据目录正被启动器之外的浏览器使用。',
      spawn_failed: '系统拒绝启动内核进程。',
      startup_failed: '浏览器在启动过程中退出。',
      shutting_down: '启动器正在退出，无法启动新的浏览器。',
      kernel_version_unknown: '无法识别内核版本，请手动填写主版本号。',
      proxy_test_failed: '代理测试失败：两个出口 IP 查询服务都无法通过该代理访问。',
    } satisfies Record<ErrorCode, string>,
    failureHints: {
      no_sandbox:
        '系统禁止了 Chromium 沙箱（Ubuntu 23.10+ 等发行版限制了非特权用户命名空间）：在该配置的额外参数中加入 --no-sandbox 后重试。',
      missing_fuse: 'AppImage 需要 FUSE 2：安装 libfuse2，或改用 tar.xz 版内核。',
    } satisfies Record<FailureHint, string>,
    theme: {
      light: '浅色',
      dark: '深色',
      system: '跟随系统',
    } satisfies Record<Theme, string>,
    language: {
      'zh-CN': '简体中文',
      en: 'English',
    } satisfies Record<Locale, string>,
    ui: {
      close: '关闭',
      pagination: '分页',
      previousPage: '上一页',
      nextPage: '下一页',
      previous: '上一页',
      next: '下一页',
      morePages: '更多页',
      commandPalette: '命令面板',
      commandPaletteDescription: '搜索要执行的命令…',
    },
    fatal: {
      title: '启动器无法启动',
      settingsFailed: '无法读取应用设置。',
      renderFailed: '界面发生错误。',
      detail: '错误详情',
    },
    placeholder: {
      comingSoon: '即将推出',
      note: '此页面将在后续阶段实现。',
    },
  },
  {
    nav: {
      profiles: 'Profiles',
      newProfile: 'New profile',
      editProfile: 'Edit profile',
      groups: 'Groups & tags',
      proxies: 'Proxies',
      settings: 'Settings',
    },
    actions: {
      save: 'Save',
      cancel: 'Cancel',
      confirm: 'OK',
      close: 'Close',
      retry: 'Retry',
      reload: 'Reload',
      copy: 'Copy',
    },
    status: {
      stopped: 'Stopped',
      starting: 'Starting…',
      running: 'Running',
      stopping: 'Stopping…',
      error: 'Failed to start',
    },
    errors: {
      not_found: 'The record does not exist or has been deleted.',
      conflict:
        'Conflicts with existing data: the name, path or browser data directory is already in use.',
      invalid_input: 'Invalid input or request.',
      db: 'Database error.',
      io: 'Could not read or write a file.',
      no_kernel: 'No kernel is configured: add a fingerprint-chromium kernel in Settings first.',
      kernel_missing: 'The kernel executable does not exist; check the kernel path.',
      kernel_not_executable:
        'The kernel path is not an executable file (on Linux / macOS it may lack the execute permission).',
      profile_running: 'This profile is running; stop it first.',
      profile_running_externally:
        "A browser started outside the launcher is using this profile's data directory.",
      spawn_failed: 'The system refused to start the kernel process.',
      startup_failed: 'The browser exited during startup.',
      shutting_down: 'The launcher is quitting; no new browsers can be started.',
      kernel_version_unknown: 'The kernel version could not be detected; enter its major version.',
      proxy_test_failed:
        'Proxy test failed: neither exit IP lookup service could be reached through this proxy.',
    },
    failureHints: {
      no_sandbox:
        'The system does not allow the Chromium sandbox (Ubuntu 23.10+ and similar distributions restrict unprivileged user namespaces): add --no-sandbox to the extra arguments of this profile and try again.',
      missing_fuse: 'The AppImage needs FUSE 2: install libfuse2, or use the tar.xz build.',
    },
    theme: {
      light: 'Light',
      dark: 'Dark',
      system: 'System',
    },
    language: {
      'zh-CN': '简体中文',
      en: 'English',
    },
    ui: {
      close: 'Close',
      pagination: 'Pagination',
      previousPage: 'Go to previous page',
      nextPage: 'Go to next page',
      previous: 'Previous',
      next: 'Next',
      morePages: 'More pages',
      commandPalette: 'Command palette',
      commandPaletteDescription: 'Search for a command to run…',
    },
    fatal: {
      title: 'The launcher could not start',
      settingsFailed: 'The app settings could not be read.',
      renderFailed: 'The interface ran into an error.',
      detail: 'Details',
    },
    placeholder: {
      comingSoon: 'Coming soon',
      note: 'This page is built in a later phase.',
    },
  }
);
