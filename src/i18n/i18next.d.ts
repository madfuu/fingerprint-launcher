/** Types `t()` keys and namespaces from the catalogs in `registry.ts`. */
import 'i18next';

import type { Resources } from './registry';

declare module 'i18next' {
  interface CustomTypeOptions {
    defaultNS: 'common';
    resources: Resources;
  }
}
