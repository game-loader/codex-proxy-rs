import type { Account, ApiKeyConfiguration } from '@/api'

export function isOpenAiApiKeyAccount(account: Pick<Account, 'provider' | 'authenticationKind'> | null | undefined): boolean {
  return account?.provider === 'openai' && account.authenticationKind === 'api_key'
}

/** 只有地址和密钥的中转账号；连接只走 HTTP/SSE */
export function isRelayApiKeyAccount(account: Pick<Account, 'provider' | 'authenticationKind'> | null | undefined): boolean {
  return account?.provider === 'newapi' && account.authenticationKind === 'api_key'
}

/** 可在编辑弹窗修改地址与密钥的账号 */
export function isEditableApiKeyAccount(account: Pick<Account, 'provider' | 'authenticationKind'> | null | undefined): boolean {
  return isOpenAiApiKeyAccount(account) || isRelayApiKeyAccount(account)
}

export function isOpenAiOAuthAccount(account: Pick<Account, 'provider' | 'authenticationKind'> | null | undefined): boolean {
  return account?.provider === 'openai' && account.authenticationKind === 'oauth'
}

export interface ApiKeyAccountForm extends Omit<ApiKeyConfiguration, 'transport'> {
  apiKey: string
  transport: ApiKeyConfiguration['transport'] | ''
}

export function emptyApiKeyAccountForm(): ApiKeyAccountForm {
  return {
    base_url: '',
    apiKey: '',
    transport: 'http',
  }
}

export function parseApiKeyConfiguration(value: Record<string, unknown> | undefined): ApiKeyConfiguration | undefined {
  if (!value || typeof value.base_url !== 'string' || (value.transport !== 'http' && value.transport !== 'prefer_websocket'))
    return undefined
  return { base_url: value.base_url, transport: value.transport }
}
