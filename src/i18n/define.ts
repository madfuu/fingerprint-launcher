/**
 * Helpers for message catalogs. Each namespace is written once per language; the types make
 * every language carry exactly the keys of the zh-CN catalog (R9.1: en is complete).
 */
import type { Locale } from '@/lib/bindings';

export interface Messages {
  readonly [key: string]: string | Messages;
}

/** The keys of `T`, with any text. */
export type Translation<T> = {
  readonly [K in keyof T]: T[K] extends string ? string : Translation<T[K]>;
};

/** One catalog per `Locale`: zh-CN is the reference, the others mirror its keys. */
export type Catalog<T extends Messages> = {
  readonly [L in Locale]: L extends 'zh-CN' ? T : Translation<T>;
};

/** One namespace in every UI language. zh-CN is the reference catalog and the default. */
export function defineMessages<T extends Messages>(zhCN: T, en: Translation<T>): Catalog<T> {
  return { 'zh-CN': zhCN, en };
}
