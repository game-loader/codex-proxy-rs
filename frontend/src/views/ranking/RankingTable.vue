<script setup lang="ts">
import type { ModelRanking, Ranking, RankingRow } from '../../api/modules/ranking'
import { ZButton } from '@codex-proxy/ui'
import { computed } from 'vue'
import { presentRankingRow, presentUsageMetrics } from './presenter'

const props = defineProps<{ report: Ranking | ModelRanking }>()
const emit = defineEmits<{ selectKey: [key: Pick<RankingRow, 'keyId' | 'keyName'>] }>()
const unknownModelKey = Symbol('unknown model')
const byModel = computed(() => 'models' in props.report)
const label = computed(() => byModel.value ? '模型用量' : 'API Key 用量排行')
const displayRows = computed(() => 'models' in props.report
  ? props.report.models.map(presentUsageMetrics)
  : props.report.rows.map(presentRankingRow))
</script>

<template>
  <div class="ranking-scroll" tabindex="0" role="region" :aria-label="`${label}表`">
    <table class="ranking-table" :class="{ 'ranking-table-models': byModel }" :aria-label="label">
      <thead>
        <tr>
          <th scope="col" class="ranking-position">
            排名
          </th>
          <th scope="col" class="ranking-name">
            {{ byModel ? '模型' : 'API Key' }}
          </th>
          <th scope="col" class="ranking-number" aria-sort="descending">
            已知费用（USD） ↓
          </th>
          <th scope="col" class="ranking-number">
            请求数
          </th>
          <th scope="col" class="ranking-number">
            Token
          </th>
          <th v-if="!byModel" scope="col" class="ranking-number">
            Token / 请求
          </th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="(row, index) in displayRows" :key="'keyId' in row ? row.keyId : (row.model ?? unknownModelKey)">
          <td class="ranking-position">
            {{ index + 1 }}
          </td>
          <th scope="row" class="ranking-name">
            <div v-if="'keyId' in row" class="ranking-key">
              <span>
                {{ row.keyName || row.keyId }}
                <span v-if="row.keyName && row.keyName !== row.keyId" class="ranking-detail">{{ row.keyId }}</span>
              </span>
              <ZButton
                size="small"
                variant="link"
                :aria-label="`查看模型：${row.keyName || row.keyId}`"
                @click="emit('selectKey', { keyId: row.keyId, keyName: row.keyName })"
              >
                查看模型
              </ZButton>
            </div>
            <template v-else>
              {{ row.model ?? '未知模型' }}
            </template>
          </th>
          <td class="ranking-number" :class="{ 'ranking-unknown': row.usd === null }">
            {{ row.costLabel }}
            <span v-if="row.unknownCostLabel" class="ranking-detail">{{ row.unknownCostLabel }}</span>
          </td>
          <td class="ranking-number">
            {{ row.requestsLabel }}
          </td>
          <td class="ranking-number" :class="{ 'ranking-unknown': row.tokens === null }">
            {{ row.tokensLabel }}
          </td>
          <td
            v-if="'tokensPerRequestLabel' in row"
            class="ranking-number"
            :class="{ 'ranking-unknown': row.tokensPerRequestLabel === null }"
          >
            <span v-if="row.tokensPerRequestLabel === null" aria-label="无法计算">—</span>
            <template v-else>
              {{ row.tokensPerRequestLabel }}
            </template>
          </td>
        </tr>
      </tbody>
    </table>
  </div>
</template>

<style scoped>
.ranking-scroll {
  overflow-x: auto;
}

.ranking-scroll:focus-visible {
  outline: 2px solid var(--z-color-primary);
  outline-offset: -2px;
}

.ranking-table {
  width: 100%;
  min-width: 39rem;
  border-collapse: collapse;
  text-align: start;
  font-variant-numeric: tabular-nums;
}

.ranking-table-models {
  min-width: 30rem;
}

.ranking-table th,
.ranking-table td {
  padding: 0.625rem 1rem;
  vertical-align: top;
}

.ranking-table thead {
  color: var(--z-color-text-secondary);
  background: var(--z-table-header-bg);
  white-space: nowrap;
}

.ranking-table tbody tr:nth-child(even) {
  background: var(--z-table-row-stripe-bg);
}

.ranking-table tbody tr:hover {
  background: var(--z-table-row-hover-bg);
}

.ranking-name {
  width: 100%;
  min-width: 8rem;
  font-weight: 600;
  text-align: start;
  overflow-wrap: anywhere;
}

.ranking-key {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 0.25rem 0.5rem;
}

.ranking-position {
  width: 1%;
  color: var(--z-color-text-secondary);
  text-align: center;
  white-space: nowrap;
}

.ranking-number {
  white-space: nowrap;
  text-align: end;
}

.ranking-detail,
.ranking-unknown {
  color: var(--z-color-text-secondary);
}

.ranking-detail {
  display: block;
  margin-top: 0.125rem;
  font-size: var(--z-font-size-sm, 12px);
  font-weight: 400;
}
</style>
