<script setup lang="ts">
import type { KeyModelCosts } from '@/api'
import { ZSelect } from '@codex-proxy/ui'
import { onMounted, shallowRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { getAdminKeyModelCosts, getApiKeys } from '@/api'
import UsageModelCosts from '@/components/usage/UsageModelCosts.vue'
import { useRequestState } from '@/composables/useRequestState'

const props = defineProps<{
  range: { startTime: string, endTime: string }
  provider: string
}>()

const { t } = useI18n({ useScope: 'global' })
const selected = shallowRef<string>()
const options = shallowRef<{ value: string, label: string }[]>([])
const result = shallowRef<KeyModelCosts>()
const keys = useRequestState()
const costs = useRequestState()

onMounted(async () => {
  const id = keys.start()
  const signal = keys.signal
  const choices: typeof options.value = []
  try {
    let cursor: string | undefined
    do {
      const page = await getApiKeys({ limit: 200, cursor }, { signal })
      choices.push(...page.items.map(item => ({ value: item.id, label: `${item.name} · ${item.prefix}` })))
      cursor = page.nextCursor ?? undefined
    } while (cursor && keys.isCurrent(id))
    if (keys.isCurrent(id))
      options.value = choices
  }
  catch (cause) {
    keys.fail(id, cause)
  }
  finally {
    keys.finish(id)
  }
})

watch([
  selected,
  () => props.range,
  () => props.provider,
], async ([clientApiKeyId, range, provider]) => {
  result.value = undefined
  if (!clientApiKeyId) {
    costs.invalidate()
    return
  }
  const id = costs.start()
  try {
    const data = await getAdminKeyModelCosts({ ...range, clientApiKeyId, provider: provider || undefined }, { signal: costs.signal })
    if (costs.isCurrent(id))
      result.value = data
  }
  catch (cause) {
    costs.fail(id, cause)
  }
  finally {
    costs.finish(id)
  }
})
</script>

<template>
  <UsageModelCosts
    :result="result"
    :loading="keys.loading.value || costs.loading.value"
    :error="keys.error.value || (selected ? costs.error.value : '')"
    :idle="!selected"
    :idle-text="!selected && !options.length ? t('usage.modelCosts.noKeys') : undefined"
  >
    <template #actions>
      <ZSelect
        v-model="selected"
        :options="options"
        :aria-label="t('usage.modelCosts.selectKey')"
        :placeholder="t('usage.modelCosts.selectKey')"
        :loading="keys.loading.value"
        :disabled="!options.length"
        class="w-full min-w-0 sm:w-64"
        filterable
        clearable
      />
    </template>
  </UsageModelCosts>
</template>
