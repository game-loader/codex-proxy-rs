import type { ModelRanking, Period, Ranking, RankingQuery, RankingRow, RankingWindow } from '../../api/modules/ranking'
import { computed, shallowRef, watch } from 'vue'
import { getRanking } from '../../api/modules/ranking'
import { ManagementError } from '../../api/request'
import { periodStart, shiftPeriod } from './calendar'

type QueryState
  = | { status: 'loading' }
    | { status: 'ready', data: Ranking | ModelRanking }
    | { status: 'error' | 'unavailable', message: string }

export function useRanking() {
  const selection = shallowRef<RankingQuery>({ period: 'day' })
  const selectedKey = shallowRef<Pick<RankingRow, 'keyId' | 'keyName'>>()
  const attempt = shallowRef(0)
  const state = shallowRef<QueryState>({ status: 'loading' })
  const range = shallowRef<RankingWindow>()

  watch([selection, selectedKey, attempt], async ([query, key], [previousQuery], onCleanup) => {
    let active = true
    onCleanup(() => {
      active = false
    })
    if (query !== previousQuery)
      range.value = undefined
    state.value = { status: 'loading' }
    try {
      const data = await getRanking(query, key?.keyId)
      if (!active)
        return
      range.value = data
      state.value = { status: 'ready', data }
    }
    catch (error) {
      if (active) {
        const historyUnavailable = key && error instanceof ManagementError
          && error.code === 'history_unavailable'
        state.value = {
          status: historyUnavailable ? 'unavailable' : 'error',
          message: error instanceof Error
            ? error.message
            : '无法查询用量排行',
        }
      }
    }
  }, { immediate: true, flush: 'sync' })

  const period = computed(() => selection.value.period)
  const start = computed(() => range.value?.startDate ?? selection.value.date)
  const isCurrent = computed(() => selection.value.date === undefined)
  const previous = computed(() => start.value && shiftPeriod(period.value, start.value, -1))
  const next = computed(() => start.value && shiftPeriod(period.value, start.value, 1))

  function setPeriod(value: Period) {
    if (period.value !== value)
      selection.value = { period: value, date: selection.value.date && periodStart(value, selection.value.date) }
  }

  function setDate(value: string) {
    const anchor = periodStart(period.value, value)
    if (start.value !== anchor)
      selection.value = { period: period.value, date: anchor }
  }

  function goToCurrent() {
    if (!isCurrent.value)
      selection.value = { period: period.value }
  }

  function retry() {
    attempt.value++
  }

  function selectKey(key?: Pick<RankingRow, 'keyId' | 'keyName'>) {
    selectedKey.value = key
  }

  return {
    state,
    range,
    period,
    selectedKey,
    selectKey,
    start,
    isCurrent,
    previous,
    next,
    setPeriod,
    setDate,
    goToCurrent,
    retry,
  }
}
