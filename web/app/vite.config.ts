import { defineConfig } from 'vite'
import preact from '@preact/preset-vite'

const apiTarget = process.env.DALI2RUST_API_TARGET ?? 'http://127.0.0.1:8080'

export default defineConfig({
  plugins: [preact()],
  server: {
    proxy: {
      '/api': { target: apiTarget, changeOrigin: true, ws: true, rewriteWsOrigin: true },
    },
  },
  build: {
    rollupOptions: {
      output: {
        entryFileNames: 'assets/app.js',
        chunkFileNames: 'assets/[name].js',
        assetFileNames: 'assets/app[extname]',
      },
    },
  },
})
