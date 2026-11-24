import type { RankingRow, UsageMetrics } from '../../api/modules/ranking'

function formatInteger(value: string): string {
  return value.replace(/\B(?=(\d{3})+(?!\d))/gu, ',')
}

function formatUsd(value: string): string {
  const negative = value.startsWith('-')
  const [whole, fraction = ''] = (negative
    ? value.slice(1)
    : value).split('.')
  return `${negative ? '-' : ''}$${formatInteger(whole)}.${fraction.padEnd(2, '0')}`
}

function formatTokensPerRequest(tokens: string | null, requests: string): string | null {
  const count = BigInt(requests)
  if (tokens === null || count === 0n)
    return null

  const total = BigInt(tokens)
  const scaled = (total * 100n + count / 2n) / count
  if (scaled === 0n && total > 0n)
    return '<0.01'
  const [whole, fraction] = [scaled / 100n, scaled % 100n]
  return `${formatInteger(String(whole))}.${String(fraction).padStart(2, '0')}`
}

export function presentUsageMetrics<T extends UsageMetrics>(row: T) {
  return {
    ...row,
    costLabel: row.usd === null
      ? '未知'
      : formatUsd(row.usd),
    requestsLabel: formatInteger(row.requests),
    tokensLabel: row.tokens === null
      ? '未知'
      : formatInteger(row.tokens),
    unknownCostLabel: BigInt(row.unknownCosts) > 0n
      ? `${formatInteger(row.unknownCosts)} 笔费用未知`
      : null,
  }
}

export function presentRankingRow(row: RankingRow) {
  return {
    ...presentUsageMetrics(row),
    tokensPerRequestLabel: formatTokensPerRequest(row.tokens, row.requests),
  }
}
