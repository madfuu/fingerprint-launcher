import js from '@eslint/js';
import prettier from 'eslint-config-prettier';
import i18next from 'eslint-plugin-i18next';
import reactHooks from 'eslint-plugin-react-hooks';
import reactRefresh from 'eslint-plugin-react-refresh';
import { defineConfig, globalIgnores } from 'eslint/config';
import globals from 'globals';
import tseslint from 'typescript-eslint';

const TAURI_TRANSPORT = 'src/lib/bridge.tauri.ts';
const BRIDGE_INTERNALS = ['src/lib/bridge.ts', 'src/lib/bridge.mock.ts', 'src/lib/mock/**'];
const NO_TAURI = {
  regex: '^@tauri-apps/',
  message: 'Native calls go through @/lib/bridge (design decision 4).',
};
const NO_MOCK = {
  // `@/lib/bridge.mock`, `./bridge.mock`, `@/lib/mock/...`, `./mock/...`, `../mock/...`
  regex: '(^|/)bridge\\.mock$|(^|/)lib/mock(/|$)|^(\\.\\.?/)+mock(/|$)',
  message: 'The mock backend is dev-only; only bridge.ts loads it (dynamic import).',
};

export default defineConfig([
  globalIgnores(['dist', 'coverage', '.screenshots', 'src-tauri']),
  {
    files: ['**/*.{ts,tsx}'],
    extends: [
      js.configs.recommended,
      tseslint.configs.recommended,
      reactHooks.configs.flat.recommended,
      reactRefresh.configs.vite,
    ],
    languageOptions: {
      ecmaVersion: 2023,
      globals: globals.browser,
    },
  },
  {
    // shadcn/ui primitives export variant helpers next to components by design.
    files: ['src/components/ui/**/*.tsx'],
    rules: {
      'react-refresh/only-export-components': 'off',
    },
  },
  {
    // A10: every user-visible string inside JSX must come from i18next.
    files: ['src/**/*.{ts,tsx}'],
    ignores: ['**/*.test.*', 'src/lib/bridge.mock.ts'],
    extends: [i18next.configs['flat/recommended']],
    rules: {
      'i18next/no-literal-string': [
        'error',
        {
          mode: 'jsx-only',
          'jsx-attributes': {
            // Plugin defaults plus props that carry identifiers, not copy.
            exclude: [
              'className',
              'styleName',
              'style',
              'type',
              'key',
              'id',
              'width',
              'height',
              'data-.*',
              'variant',
              'size',
              'side',
              'align',
              'to',
            ],
          },
        },
      ],
    },
  },
  // Design decision 4: native code is reached only through src/lib/bridge.ts. `@tauri-apps/*`
  // may be imported by the Tauri transport alone, and the mock (dev-only, loaded by bridge.ts
  // through a dynamic import) by nothing but itself and tests, so it cannot end up in a
  // production bundle through a static import.
  {
    files: ['src/**/*.{ts,tsx}'],
    ignores: [...BRIDGE_INTERNALS, TAURI_TRANSPORT, '**/*.test.*'],
    rules: {
      'no-restricted-imports': ['error', { patterns: [NO_TAURI, NO_MOCK] }],
    },
  },
  {
    files: [...BRIDGE_INTERNALS, '**/*.test.*'],
    rules: {
      'no-restricted-imports': ['error', { patterns: [NO_TAURI] }],
    },
  },
  {
    files: [TAURI_TRANSPORT],
    rules: {
      'no-restricted-imports': ['error', { patterns: [NO_MOCK] }],
    },
  },
  {
    // Node-side tooling: config files and scripts.
    files: ['**/*.{js,mjs,cjs}'],
    extends: [js.configs.recommended],
    languageOptions: {
      globals: globals.node,
    },
  },
  {
    files: ['vite.config.ts'],
    languageOptions: {
      globals: globals.node,
    },
  },
  prettier,
]);
