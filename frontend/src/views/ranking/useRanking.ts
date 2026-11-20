import type { Period, Ranking, RankingQuery } from '../../api/modules/ranking'
import { computed, shallowRef, watch } from 'vue'
import { getRanking } from '../../api/modules/ranking'
import { periodStart, shiftPeriod } from './calendar'

type QueryState = { status: 'loading' } | { status: 'ready', data: Ranking } | { status: 'error', message: string }

export function useRanking() {
  const selection = shallowRef<RankingQuery>({ period: 'day' })
  const state = shallowRef<QueryState>({ status: 'loading' })

  watch(selection, async (query, _, onCleanup) => {
    let active = true
    onCleanup(() => {
      active = false
    })
    state.value = { status: 'loading' }
    try {
      const data = await getRanking(query)
      if (active)
        state.value = { status: 'ready', data }
    }
    catch (error) {
      if (active) {
        state.value = {
          status: 'error',
          message: error instanceof Error
            ? error.message
            : '无法查询用量排行',
        }
      }
    }
  }, { immediate: true })

  const period = computed(() => selection.value.period)
  const data = computed(() => state.value.status === 'ready'
    ? state.value.data
    : undefined)
  const start = computed(() => data.value?.startDate ?? selection.value.date)
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
    selection.value = { ...selection.value }
  }

  return {
    state,
    period,
    data,
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
