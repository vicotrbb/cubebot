import js from '@eslint/js'
import { defineConfig, globalIgnores } from 'eslint/config'
import reactHooks from 'eslint-plugin-react-hooks'
import tseslint from 'typescript-eslint'

export default defineConfig([
  globalIgnores([
    '**/node_modules/**',
    '**/.output/**',
    '**/.tanstack/**',
    '**/dist/**',
    'target/**',
    '**/.venv/**',
    '.tools/**',
    'apps/web/src/routeTree.gen.ts',
  ]),
  js.configs.recommended,
  ...tseslint.configs.strict,
  {
    files: ['**/*.{js,mjs,cjs,ts,tsx,mts,cts}'],
    rules: {
      complexity: ['error', { max: 15, variant: 'classic' }],
      'no-warning-comments': [
        'error',
        { terms: ['eslint-disable'], location: 'anywhere' },
      ],
    },
    linterOptions: {
      noInlineConfig: true,
      reportUnusedDisableDirectives: 'error',
    },
  },
  {
    files: ['**/*.{ts,tsx}'],
    plugins: { 'react-hooks': reactHooks },
    rules: reactHooks.configs.recommended.rules,
  },
])
