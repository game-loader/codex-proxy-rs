<script setup lang="ts">
import { ZButton, ZCard, ZDatePicker, ZEmpty, ZSegmented } from '@codex-proxy/ui'
import { zhHans } from '@codex-proxy/ui/locale'
import { computed } from 'vue'
import RankingTable from './RankingTable.vue'
import { useRanking } from './useRanking'

const periods = [
  { value: 'day', label: '日' },
  { value: 'week', label: '周' },
  { value: 'month', label: '月' },
]

const { state, period, data, start, isCurrent, previous, next, setPeriod, setDate, goToCurrent, retry } = useRanking()
const periodLabel = computed(() => ({ day: '日', week: '周', month: '月' })[period.value])
const currentLabel = computed(() => ({ day: '回到今天', week: '回到本周', month: '回到本月' })[period.value])
const pickerLabel = computed(() => period.value === 'month'
  ? '选择月份'
  : period.value === 'week'
    ? '选择周内日期'
    : '选择日期')
const pickerFormat = computed(() => period.value === 'month'
  ? 'YYYY-MM'
  : 'YYYY-MM-DD')
const pickerValue = computed(() => period.value === 'month'
  ? start.value?.slice(0, 7)
  : start.value)
const rangeLabel = computed(() => {
  if (!data.value)
    return ''
  const { startDate, endDate, timezone } = data.value
  const range = startDate === endDate
    ? startDate
    : `${startDate} 至 ${endDate}`
  return `${range} · ${timezone}`
})

function updatePeriod(value: string) {
  if (value === 'day' || value === 'week' || value === 'month')
    setPeriod(value)
}

function updateDate(value: unknown) {
  if (typeof value === 'string') {
    setDate(period.value === 'month'
      ? `${value}-01`
      : value)
  }
}
</script>

<template>
  <main class="ranking-page">
    <div class="ranking-toolbar">
      <ZSegmented
        :model-value="period"
        :options="periods"
        aria-label="统计周期"
        @update:model-value="updatePeriod"
      />
      <div class="ranking-navigation" role="group" aria-label="日期导航">
        <ZButton :disabled="!previous" @click="previous && setDate(previous)">
          上{{ periodLabel }}
        </ZButton>
        <ZDatePicker
          :key="period"
          class="ranking-picker"
          :model-value="pickerValue"
          :type="period === 'month' ? 'month' : 'date'"
          :aria-label="pickerLabel"
          :placeholder="pickerLabel"
          :format="pickerFormat"
          :value-format="pickerFormat"
          :locale="zhHans"
          :clearable="false"
          :editable="false"
          @update:model-value="updateDate"
        />
        <ZButton :disabled="!next" @click="next && setDate(next)">
          下{{ periodLabel }}
        </ZButton>
        <ZButton :disabled="isCurrent" @click="goToCurrent">
          {{ currentLabel }}
        </ZButton>
      </div>
      <p v-if="rangeLabel" class="ranking-range">
        {{ rangeLabel }}
      </p>
    </div>

    <ZCard padding="none" as="section" aria-label="排行结果">
      <p v-if="state.status === 'loading'" class="ranking-status" role="status">
        正在查询用量排行
      </p>
      <div v-else-if="state.status === 'error'" class="ranking-status">
        <p role="alert">
          {{ state.message }}
        </p>
        <ZButton @click="retry">
          重试
        </ZButton>
      </div>
      <ZEmpty v-else-if="!state.data.rows.length" title="该时段没有用量记录" surface="none" />
      <RankingTable v-else :rows="state.data.rows" />
    </ZCard>
  </main>
</template>

<style scoped>
.ranking-page {
  display: grid;
  gap: 1rem;
  color: var(--z-color-text);
  font-family: var(--z-font-family, system-ui, sans-serif);
  font-size: var(--z-font-size, 14px);
}

.ranking-toolbar,
.ranking-navigation {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.5rem;
}

.ranking-toolbar {
  column-gap: 1rem;
}

.ranking-picker {
  width: 9.25rem;
  max-width: 100%;
}

.ranking-range {
  margin-block: 0;
  margin-inline-start: auto;
  color: var(--z-color-text-secondary);
  font-variant-numeric: tabular-nums;
}

.ranking-status {
  padding: 1.5rem 1rem;
  margin: 0;
  color: var(--z-color-text-secondary);
  text-align: center;
}

.ranking-status p {
  margin: 0 0 0.75rem;
  color: var(--z-color-error-text);
}

@media (max-width: 48rem) {
  .ranking-range {
    margin-inline-start: 0;
  }
}
</style>
