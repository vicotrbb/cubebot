import { tanstackStart } from '@tanstack/react-start/plugin/vite'
import react from '@vitejs/plugin-react'
import { nitro } from 'nitro/vite'
import { defineConfig } from 'vite'

export default defineConfig(({ command }) => ({
  plugins: [
    tanstackStart(),
    nitro({
      preset: 'bun',
      // Nitro handles document/API requests before Vite's server.proxy middleware.
      routeRules:
        command === 'serve'
          ? { '/api/**': { proxy: 'http://127.0.0.1:8080/api/**' } }
          : {},
    }),
    react(),
  ],
}))
