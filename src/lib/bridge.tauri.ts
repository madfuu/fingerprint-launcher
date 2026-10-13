/**
 * The Tauri transport of the bridge: the one module that imports `@tauri-apps/*` (ESLint
 * enforces it). Everything else reaches native code through `@/lib/bridge`.
 */
import { invoke, type InvokeArgs } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import { open, save } from '@tauri-apps/plugin-dialog';

import type { CommandArgs, CommandName, CommandResult, Transport } from '@/lib/bridge';

export const tauriTransport: Transport = {
  kind: 'tauri',

  invoke<K extends CommandName>(command: K, args: CommandArgs<K>): Promise<CommandResult<K>> {
    return invoke<CommandResult<K>>(command, args as InvokeArgs);
  },

  listen(event, handler) {
    return listen(event, (message) => handler(message.payload as Parameters<typeof handler>[0]));
  },

  async openFile({ title, filters, defaultPath, directory = false }) {
    const picked = await open({ title, filters, defaultPath, directory, multiple: false });
    return typeof picked === 'string' ? picked : null;
  },

  saveFile({ title, filters, defaultPath }) {
    return save({ title, filters, defaultPath });
  },

  writeClipboard(text) {
    return writeText(text);
  },
};
