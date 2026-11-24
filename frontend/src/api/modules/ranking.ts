import { isRecord, request } from '../request'

export type Period = 'day' | 'week' | 'month'

export interface RankingQuery {
  period: Period
  date?: string
}

export interface UsageMetrics {
  usd: string | null
  requests: string
  tokens: string | null
  unknownCosts: string
}

export interface RankingRow extends UsageMetrics {
  keyId: string
  keyName: string
}

export interface ModelRow extends UsageMetrics {
  model: string | null
}

export interface RankingWindow {
  period: Period
  startDate: string
  endDate: string
  timezone: string
}

export interface Ranking extends RankingWindow {
  rows: RankingRow[]
}

export interface ModelRanking extends RankingWindow {
  models: ModelRow[]
}

const integer = /^\d+$/u
const decimal = /^-?\d+(?:\.\d+)?$/u
const date = /^\d{4}-\d{2}-\d{2}$/u

function isNumericString(value: unknown, pattern: RegExp): value is string {
  return typeof value === 'string' && pattern.test(value)
}

function isUsageMetrics(value: unknown): value is UsageMetrics & Record<string, unknown> {
  return isRecord(value)
    && (value.usd === null || isNumericString(value.usd, decimal))
    && isNumericString(value.requests, integer)
    && (value.tokens === null || isNumericString(value.tokens, integer))
    && isNumericString(value.unknownCosts, integer)
}

function isRankingRow(value: unknown): value is RankingRow {
  return isUsageMetrics(value) && typeof value.keyId === 'string' && typeof value.keyName === 'string'
}

function isModelRow(value: unknown): value is ModelRow {
  return isUsageMetrics(value) && (value.model === null || typeof value.model === 'string')
}

export async function getRanking(selection: RankingQuery, keyId?: string): Promise<Ranking | ModelRanking> {
  const query = new URLSearchParams({ period: selection.period })
  if (selection.date)
    query.set('date', selection.date)
  if (keyId !== undefined)
    query.set('keyId', keyId)

  const value = await request({ path: 'ranking', query })
  if (!isRecord(value)
    || value.period !== selection.period
    || !isNumericString(value.startDate, date)
    || !isNumericString(value.endDate, date)
    || value.endDate < value.startDate
    || typeof value.timezone !== 'string') {
    throw new Error('用量排行数据格式不正确')
  }

  const window = {
    period: selection.period,
    startDate: value.startDate,
    endDate: value.endDate,
    timezone: value.timezone,
  }
  if (keyId !== undefined) {
    if (Array.isArray(value.models) && value.models.every(isModelRow))
      return { ...window, models: value.models }
    throw new Error('模型用量数据格式不正确')
  }
  if (Array.isArray(value.rows) && value.rows.every(isRankingRow))
    return { ...window, rows: value.rows }
  throw new Error('用量排行数据格式不正确')
}
