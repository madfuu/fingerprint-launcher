/** Errors the mock rejects with: the same plain `{ code, message }` objects Tauri delivers. */
import type { AppError, ErrorCode } from '@/lib/bindings';

export function mockError(code: ErrorCode, message: string): AppError {
  return { code, message };
}
