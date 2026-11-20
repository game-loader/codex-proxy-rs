import type { Period } from '../../api/modules/ranking'

export function periodStart(period: Period, date: string): string {
  if (period === 'day')
    return date
  if (period === 'month')
    return `${date.slice(0, 7)}-01`

  // 使用 UTC 处理日历日期，避免浏览器时区及夏令时改变周界限
  const day = new Date(`${date}T00:00:00Z`)
  const daysSinceMonday = (day.getUTCDay() + 6) % 7
  day.setUTCDate(day.getUTCDate() - daysSinceMonday)
  return day.toISOString().slice(0, 10)
}

export function shiftPeriod(period: Period, start: string, direction: -1 | 1): string | undefined {
  const day = new Date(`${start}T00:00:00Z`)
  if (period === 'month') {
    day.setUTCMonth(day.getUTCMonth() + direction, 1)
  }
  else {
    day.setUTCDate(day.getUTCDate() + direction * (period === 'week'
      ? 7
      : 1))
  }
  if (day.getUTCFullYear() >= 1 && day.getUTCFullYear() <= 9999)
    return day.toISOString().slice(0, 10)
}
