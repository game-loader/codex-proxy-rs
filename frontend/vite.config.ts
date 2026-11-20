import CodexProxyUI from '@codex-proxy/ui/vite'
import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vite'

export default defineConfig({
  base: './',
  plugins: [
    vue(),
    CodexProxyUI(),
    {
      name: 'plugin-classic-script',
      apply: 'build',
      // 宿主隔离页仅接受经典脚本，静态资源仍由 Vite 生成
      transformIndexHtml: {
        order: 'post',
        handler: html => html.replace('type="module" crossorigin', 'defer'),
      },
    },
  ],
  build: {
    modulePreload: false,
    cssCodeSplit: false,
    rolldownOptions: {
      output: {
        format: 'iife',
        codeSplitting: false,
        entryFileNames: 'app.js',
        assetFileNames: 'app.[ext]',
      },
    },
  },
})
