import { defineConfig } from 'astro/config'

// Static output only: the site must deploy to any file host.
export default defineConfig({
  site: 'https://cubebot.io',
  output: 'static',
  build: { inlineStylesheets: 'always' },
})
