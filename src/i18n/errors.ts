/** Error text for the UI: the message of the error code (design §5) plus the raw detail. */
import type { TFunction } from 'i18next';

import type { AppError } from '@/lib/bindings';
import { toAppError } from '@/lib/bridge';

export interface LocalizedError {
  code: AppError['code'];
  /** Translated message of the error code. */
  message: string;
  /** The backend's raw message, untranslated (paths, stderr lines, ...). */
  detail: string;
}

export function localizeError(t: TFunction<'common'>, error: unknown): LocalizedError {
  const { code, message } = toAppError(error);
  return { code, message: t(`errors.${code}`), detail: message };
}
