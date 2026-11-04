import type { Plugin } from 'vite'
import { Buffer } from 'node:buffer'
import { readFileSync } from 'node:fs'
import { renderCodexConfig } from '../../src/utils/codex-config.ts'

const SCRIPT_DIRECTORY = 'codex'
const SHELL_SCRIPT = 'install.sh'
const POWERSHELL_SCRIPT = 'install.ps1'

function readTemplate(name: string) {
  return readFileSync(new URL(name, import.meta.url), 'utf8')
}

const SHELL_VARIABLES = [
  'base_url_toml',
  'model_catalog_url_toml',
  'api_key_toml',
  'websocket_enabled',
] as const

// heredoc 会展开 $、反引号和反斜杠，只允许下方插入的变量引用
function renderShellScript() {
  const [baseUrl, modelCatalogUrl, apiKey, websocketEnabled] = SHELL_VARIABLES.map(name => `$\{${name}}`)
  const config = renderCodexConfig({
    baseUrl,
    modelCatalogUrl,
    apiKey,
    websocketEnabled,
  })
  const unexpanded = SHELL_VARIABLES.reduce((text, name) => text.replaceAll(`$\{${name}}`, ''), config)
  if (/[$`\\]/.test(unexpanded))
    throw new Error('Codex 配置模板含有 heredoc 会展开的字符')
  return readTemplate(SHELL_SCRIPT).replace('@@CODEX_CONFIG@@', () => config)
}

// Windows PowerShell 按下载时推断的编码读取脚本，去掉中文注释后产物只含 ASCII
function renderPowerShellScript() {
  const config = renderCodexConfig({
    baseUrl: '@@BASE_URL@@',
    modelCatalogUrl: '@@MODEL_CATALOG_URL@@',
    apiKey: '@@API_KEY@@',
    websocketEnabled: '@@WEBSOCKET_ENABLED@@',
  })
  const script = readTemplate(POWERSHELL_SCRIPT)
    .split('\n')
    .filter(line => !/^\s*#/.test(line))
    .join('\n')
    .replace('@@CODEX_CONFIG_BASE64@@', () => Buffer.from(config, 'utf8').toString('base64'))
  if (/[^\t\n\r\x20-\x7E]/.test(script))
    throw new Error('PowerShell 安装脚本必须只含 ASCII 字符')
  return script
}

const scripts: Record<string, () => string> = {
  [SHELL_SCRIPT]: renderShellScript,
  [POWERSHELL_SCRIPT]: renderPowerShellScript,
}

// 一键配置脚本随静态资源发布在 /codex/ 下，开发服务器按同一路径实时生成
export function codexInstallScripts(): Plugin {
  return {
    name: 'codex-install-scripts',
    buildStart() {
      for (const name of Object.keys(scripts))
        this.addWatchFile(new URL(name, import.meta.url).pathname)
    },
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        const path = request.url?.split('?')[0] ?? ''
        const render = path.startsWith(`/${SCRIPT_DIRECTORY}/`)
          ? scripts[path.slice(SCRIPT_DIRECTORY.length + 2)]
          : undefined
        if (!render)
          return next()
        response.setHeader('Content-Type', 'text/plain; charset=utf-8')
        response.setHeader('Cache-Control', 'no-cache')
        response.end(render())
      })
    },
    generateBundle() {
      for (const [name, render] of Object.entries(scripts)) {
        this.emitFile({
          type: 'asset',
          fileName: `${SCRIPT_DIRECTORY}/${name}`,
          source: render(),
        })
      }
    },
  }
}
