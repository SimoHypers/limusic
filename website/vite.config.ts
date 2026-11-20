import path from 'node:path'
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  // Relative, so one build works at limusic.app/ and at the old github.io/limusic/ path.
  base: './',
  plugins: [react(), tailwindcss()],
  // Two pages: the landing page, and the lyrics leaderboard and profiles at /lyrics/.
  build: {
    rolldownOptions: {
      input: { main: 'index.html', lyrics: 'lyrics/index.html' },
    },
  },
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
})
