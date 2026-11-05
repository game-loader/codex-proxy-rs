// 页面配置与构建期生成的一键安装脚本共用此模板，模块不能依赖浏览器环境或应用别名
export const CODEX_DEFAULT_MODEL = 'gpt-6.1-sol'
export const CODEX_WEBSOCKET_ENABLED_BY_DEFAULT = false

interface CodexConfigInput {
  apiKey: string
  baseUrl: string
  websocketEnabled?: boolean
}

// 各字段为已编码的 TOML 值，安装脚本借此插入运行时占位符
export interface CodexConfigValues {
  baseUrl: string
  modelCatalogUrl: string
  apiKey: string
  websocketEnabled: string
}

export function renderCodexConfig(values: CodexConfigValues): string {
  return `model_provider = "OpenAI"
model = "${CODEX_DEFAULT_MODEL}"
review_model = "${CODEX_DEFAULT_MODEL}"
model_reasoning_effort = "max"
service_tier = "default"
# 语音通话控制地址
experimental_realtime_ws_base_url = ${values.baseUrl}

[model_providers.OpenAI]
name = "OpenAI"
base_url = ${values.baseUrl}
model_catalog_url = ${values.modelCatalogUrl}
wire_api = "responses"
supports_websockets = ${values.websocketEnabled}
requires_openai_auth = false
# 代理密钥
experimental_bearer_token = ${values.apiKey}

[model_providers.OpenAI.capabilities]
# V2 远程压缩
remote_compaction = "v2"

[model_providers.OpenAI.http_headers]
# 服务端托管认证
X-OpenAI-Actor-Authorization = "proxy-managed"

[features]
image_generation = true
goals = true
# 从网关拉取模型列表
api_key_model_discovery = true`
}

export function buildCodexConfig(input: CodexConfigInput): string {
  const baseUrl = input.baseUrl.replace(/\/+$/, '')
  return renderCodexConfig({
    baseUrl: JSON.stringify(baseUrl),
    modelCatalogUrl: JSON.stringify(`${baseUrl}/models`),
    apiKey: JSON.stringify(input.apiKey),
    websocketEnabled: String(input.websocketEnabled ?? CODEX_WEBSOCKET_ENABLED_BY_DEFAULT),
  })
}
