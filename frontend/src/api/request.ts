interface ManagementReply {
  status: number
  contentType: string
  body: ArrayBuffer
}

interface PluginHost {
  readonly version: 2
  readonly theme: 'light' | 'dark'
  request: (input: { method: 'GET', path: string, query: string }) => Promise<ManagementReply>
}

declare global {
  interface Window {
    readonly codexProxyPlugin?: PluginHost
  }
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

export async function request({ path, query }: { path: string, query: URLSearchParams }): Promise<unknown> {
  const host = window.codexProxyPlugin
  if (host?.version !== 2)
    throw new Error('请从网关的插件管理页面打开')

  let reply: ManagementReply
  try {
    reply = await host.request({
      method: 'GET',
      path,
      query: query.toString(),
    })
  }
  catch {
    throw new Error('无法查询用量排行')
  }

  const failed = reply.status < 200 || reply.status >= 300
  const statusMessage = `用量排行查询失败（HTTP ${reply.status}）`
  if (reply.contentType.split(';', 1)[0]?.trim().toLowerCase() !== 'application/json') {
    if (failed)
      throw new Error(statusMessage)
    throw new Error('用量排行返回了不支持的内容类型')
  }

  let value: unknown
  try {
    value = JSON.parse(new TextDecoder().decode(reply.body))
  }
  catch {
    throw new Error(failed ? statusMessage : '用量排行返回了无效 JSON')
  }

  if (failed) {
    if (isRecord(value) && isRecord(value.error) && typeof value.error.message === 'string' && value.error.message)
      throw new Error(value.error.message)
    throw new Error(statusMessage)
  }
  return value
}
