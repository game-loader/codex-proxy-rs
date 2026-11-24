<script setup lang="ts">
import type { RankingRow } from '../../api/modules/ranking'
import { ZButton, ZCard, ZDatePicker, ZEmpty, ZSegmented } from '@codex-proxy/ui'
import { zhHans } from '@codex-proxy/ui/locale'
import { computed, nextTick, useTemplateRef } from 'vue'
import RankingTable from './RankingTable.vue'
import { useRanking } from './useRanking'

const periods = [
  { value: 'day', label: '日' },
  { value: 'week', label: '周' },
  { value: 'month', label: '月' },
]

const {
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
} = useRanking()
const results = useTemplateRef('results')
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
  if (!range.value)
    return ''
  const { startDate, endDate, timezone } = range.value
  const dates = startDate === endDate
    ? startDate
    : `${startDate} 至 ${endDate}`
  return `${dates} · ${timezone}`
})

async function updateKey(key?: Pick<RankingRow, 'keyId' | 'keyName'>) {
  selectKey(key)
  await nextTick()
  results.value?.focus()
}

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

    <section ref="results" class="ranking-results" tabindex="-1" :aria-label="selectedKey ? '模型用量' : '排行结果'">
      <div v-if="selectedKey" class="ranking-scope">
        <ZButton size="small" @click="updateKey()">
          返回排行
        </ZButton>
        <h2 class="ranking-scope-title">
          {{ selectedKey.keyName || selectedKey.keyId }} · 模型用量
        </h2>
        <span v-if="selectedKey.keyName && selectedKey.keyName !== selectedKey.keyId" class="ranking-scope-id">
          {{ selectedKey.keyId }}
        </span>
      </div>
      <ZCard padding="none">
        <p v-if="state.status === 'loading'" class="ranking-status" role="status">
          {{ selectedKey ? '正在查询模型用量' : '正在查询用量排行' }}
        </p>
        <div v-else-if="state.status === 'error'" class="ranking-status">
          <p role="alert">
            {{ state.message }}
          </p>
          <ZButton @click="retry">
            重试
          </ZButton>
        </div>
        <div v-else-if="state.status === 'unavailable'" role="status">
          <ZEmpty title="所选时段无法按模型查看" :description="state.message" surface="none" />
        </div>
        <template v-else-if="state.status === 'ready'">
          <ZEmpty v-if="'models' in state.data && !state.data.models.length" title="该 Key 在所选时段没有模型用量" surface="none" />
          <ZEmpty v-else-if="'rows' in state.data && !state.data.rows.length" title="该时段没有用量记录" surface="none" />
          <RankingTable v-else :report="state.data" @select-key="updateKey" />
        </template>
      </ZCard>
    </section>
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

.ranking-results {
  min-inline-size: 0;
}

.ranking-results:focus-visible {
  outline: 2px solid var(--z-color-primary);
  outline-offset: 2px;
}

.ranking-scope {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 0.5rem;
  margin-block-end: 0.5rem;
  overflow-wrap: anywhere;
}

.ranking-scope-title {
  margin: 0;
  font-size: inherit;
  font-weight: 600;
}

.ranking-scope-id {
  color: var(--z-color-text-secondary);
  font-size: var(--z-font-size-sm, 12px);
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
