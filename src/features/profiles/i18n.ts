import { defineMessages } from '@/i18n/define';

export const profilesMessages = defineMessages(
  {
    title: '配置列表',
    description: '以配置文件为单位管理 fingerprint-chromium：启动、停止与批量操作。',
    summary: '共 {{total}} 个配置，{{running}} 个运行中',
  },
  {
    title: 'Profiles',
    description: 'Manage fingerprint-chromium by profile: launch, stop and batch operations.',
    summary: '{{total}} profiles, {{running}} running',
  }
);
