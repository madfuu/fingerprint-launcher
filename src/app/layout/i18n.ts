import { defineMessages } from '@/i18n/define';

/** The app frame: title bar and sidebar. Page names come from `common:nav`. */
export const layoutMessages = defineMessages(
  {
    sidebar: {
      label: '主导航',
      resources: '资源',
      expand: '展开侧栏',
      collapse: '收起侧栏',
    },
    titlebar: {
      breadcrumb: '当前位置',
      theme: '主题',
      language: '界面语言',
      drag: '按住拖动窗口',
      minimize: '最小化',
      maximize: '最大化',
      restore: '向下还原',
      close: '关闭',
    },
  },
  {
    sidebar: {
      label: 'Main navigation',
      resources: 'Resources',
      expand: 'Expand sidebar',
      collapse: 'Collapse sidebar',
    },
    titlebar: {
      breadcrumb: 'Breadcrumb',
      theme: 'Theme',
      language: 'Language',
      drag: 'Drag to move the window',
      minimize: 'Minimize',
      maximize: 'Maximize',
      restore: 'Restore down',
      close: 'Close',
    },
  }
);
