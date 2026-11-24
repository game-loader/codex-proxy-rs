<script setup lang="ts">
import type { KeyModelCosts, ModelCost } from '@/api'
import { defineTableColumns, ZCard, ZEmpty, ZTable } from '@codex-proxy/ui'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

defineProps<{
  result?: KeyModelCosts | null
  loading?: boolean
  error?: string
  idleText?: string
  idle?: boolean
}>()

const { t } = useI18n({ useScope: 'global' })
const columns = computed(() => defineTableColumns<ModelCost>([
  {
    key: 'model',
    label: t('usage.fields.model'),
    size: 'lg',
  },
  {
    key: 'costUsd',
    label: t('usage.fields.cost'),
    kind: 'numeric',
    size: 'lg',
  },
]))
const usd = new Intl.NumberFormat('en-US', {
  style: 'currency',
  currency: 'USD',
  maximumFractionDigits: 0,
})

function amountDisplay(amount: string) {
  const [dollars = '0', fraction = ''] = amount.split('.')
  return `${usd.format(BigInt(dollars))}.${fraction.padEnd(2, '0')}`
}
</script>

<template>
  <ZCard as="section" :title="t('usage.modelCosts.title')">
    <template v-if="$slots.actions" #actions>
      <slot name="actions" />
    </template>
    <template #body>
      <div v-if="result?.items.length" class="max-h-80 min-w-0">
        <ZTable :columns="columns" :data="result.items" :row-key="row => row.model ?? ''" size="small">
          <template #model="{ row }">
            {{ row.model ?? t('common.values.unknown') }}
          </template>
          <template #costUsd="{ row }">
            <span v-if="row.costUsd === null" class="text-z-text-secondary">{{ t('common.values.unknown') }}</span>
            <template v-else>
              <span class="wrap-anywhere">{{ amountDisplay(row.costUsd) }}</span>
              <span v-if="row.costIncomplete" class="ml-2 text-z-text-secondary">{{ t('usage.modelCosts.partial') }}</span>
            </template>
          </template>
        </ZTable>
      </div>
      <ZEmpty
        v-else-if="!idle || error || loading || idleText"
        size="small"
        surface="none"
        :title="error || (loading ? t('usage.cost.loading') : idleText || t('usage.modelCosts.empty'))"
        role="status"
      />
      <p v-if="result?.truncated" class="mt-2 mb-0 text-z-sm text-z-text-secondary">
        {{ t('usage.modelCosts.truncated') }}
      </p>
    </template>
  </ZCard>
</template>
