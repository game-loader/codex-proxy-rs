const storageKey = 'cpr-feishu-status-return'
const flowLifetimeMs = 10 * 60 * 1000

// 飞书回调固定返回用量页，仅在当前标签页暂存返回服务状态的意图
export function rememberFeishuStatusReturn() {
  try {
    sessionStorage.setItem(storageKey, String(Date.now() + flowLifetimeMs))
  }
  catch {
    // 无法使用存储时仍可完成登录
  }
}

export function consumeFeishuStatusReturn() {
  try {
    const expiresAt = sessionStorage.getItem(storageKey)
    sessionStorage.removeItem(storageKey)
    return Number(expiresAt) > Date.now()
  }
  catch {
    return false
  }
}
