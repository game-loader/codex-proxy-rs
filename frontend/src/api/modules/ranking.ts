import { isRecord, request } from '../request'

export type Period = 'day' | 'week' | 'month'

export interface RankingQuery {
  period: Period
  date?: string
}

export interface RankingRow {
  keyId: string
  keyName: string
  usd: string | null
  requests: string
  tokens: string | null
  unknownCosts: string
}

export interface Ranking {
  period: Period
  startDate: string
  endDate: string
  timezone: string
  rows: RankingRow[]
}

const integer = /^\d+$/u
const decimal = /^-?\d+(?:\.\d+)?$/u
const date = /^\d{4}-\d{2}-\d{2}$/u

function isNumericString(value: unknown, pattern: RegExp): value is string {
  return typeof value === 'string' && pattern.test(value)
}

function isRankingRow(value: unknown): value is RankingRow {
  return isRecord(value)
    && typeof value.keyId === 'string'
    && typeof value.keyName === 'string'
    && (value.usd === null || isNumericString(value.usd, decimal))
    && isNumericString(value.requests, integer)
    && (value.tokens === null || isNumericString(value.tokens, integer))
    && isNumericString(value.unknownCosts, integer)
}

export async function getRanking(selection: RankingQuery): Promise<Ranking> {
  const query = new URLSearchParams({ period: selection.period })
  if (selection.date)
    query.set('date', selection.date)

  const value = await request({ path: 'ranking', query })
  if (!isRecord(value)
    || value.period !== selection.period
    || !isNumericString(value.startDate, date)
    || !isNumericString(value.endDate, date)
    || value.endDate < value.startDate
    || typeof value.timezone !== 'string'
    || !Array.isArray(value.rows)
    || !value.rows.every(isRankingRow)) {
    throw new Error('用量排行数据格式不正确')
  }

  return {
    period: selection.period,
    startDate: value.startDate,
    endDate: value.endDate,
    timezone: value.timezone,
    rows: value.rows,
  }
}
