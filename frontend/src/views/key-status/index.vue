<script setup lang="ts">
import { ZButton, ZCard, ZIconButton, ZPageHeader, ZScrollbar, ZTag } from '@codex-proxy/ui'
import { ArrowLeft, Moon, RefreshCw, Sun } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import AppLanguageMenu from '@/components/AppLanguageMenu.vue'
import LastUsedAtCell from '@/components/LastUsedAtCell.vue'
import { useThemeStore } from '@/stores/modules/theme'
import { numberLocale } from '@/utils/format'
import { useKeyStatus } from './composables/useKeyStatus'

const { t } = useI18n({ useScope: 'global' })
const theme = useThemeStore()
const { snapshot, refresh, loading, error } = useKeyStatus()

const healthyThreshold = 0.85
const degradedThreshold = 0.5
const rate = computed(() => snapshot.value?.successRate ?? null)
const tone = computed(() => {
  if (rate.value === null) {
    return {
      label: t('keyStatus.noData'),
      dot: 'bg-z-text-quaternary',
    }
  }
  if (rate.value >= healthyThreshold) {
    return {
      label: t('keyStatus.healthy'),
      dot: 'bg-z-green-solid',
    }
  }
  if (rate.value >= degradedThreshold) {
    return {
      label: t('keyStatus.degraded'),
      dot: 'bg-yellow-500',
    }
  }
  return {
    label: t('keyStatus.low'),
    dot: 'bg-z-red-solid',
  }
})

const rateFormatter = computed(() => new Intl.NumberFormat(numberLocale(), {
  style: 'percent',
  maximumFractionDigits: 2,
}))

const displayRate = computed(() => {
  const value = rate.value
  if (value === null)
    return ''
  // 不让两位小数的百分比跨越状态阈值
  const upperBound = [degradedThreshold, healthyThreshold, 1].find(threshold => value < threshold)
  return rateFormatter.value.format(upperBound === undefined
    ? value
    : Math.min(value, upperBound - 0.0001))
})
</script>

<template>
  <main class="h-dvh overflow-hidden bg-z-bg-layout text-z-text">
    <ZScrollbar>
      <div class="flex min-h-full w-full flex-col gap-5 p-4 min-[961px]:p-6">
        <ZPageHeader :title="t('keyStatus.title')">
          <template #actions>
            <div class="flex max-w-[calc(100vw-32px)] flex-wrap items-center justify-end gap-2 [--z-icon-button-secondary-active-bg:var(--z-button-secondary-active-bg)] [--z-icon-button-secondary-bg:var(--z-button-secondary-bg)] [--z-icon-button-secondary-hover-bg:var(--z-button-secondary-hover-bg)]">
              <ZButton @click="$router.push({ name: 'key-usage' })">
                <ArrowLeft class="size-4" aria-hidden="true" />
                {{ t('guide.backToUsage') }}
              </ZButton>
              <AppLanguageMenu size="default" variant="solid" />
              <ZIconButton :aria-label="t('usage.header.switchTheme')" variant="solid" @click="theme.toggleTheme($event)">
                <Sun v-if="theme.effectiveTheme === 'dark'" class="size-4" />
                <Moon v-else class="size-4" />
              </ZIconButton>
            </div>
          </template>
        </ZPageHeader>

        <ZCard class="w-full max-w-xl" :title="t('keyStatus.successRate')" :description="t('keyStatus.window')">
          <template #actions>
            <ZButton :loading="loading" @click="refresh">
              <RefreshCw class="size-4" aria-hidden="true" />
              {{ t('common.actions.refresh') }}
            </ZButton>
          </template>
          <div role="status" class="flex min-h-9 flex-wrap items-center gap-x-3 gap-y-2">
            <template v-if="snapshot">
              <span v-if="displayRate" class="font-mono text-2xl font-semibold text-z-text tabular-nums">
                {{ displayRate }}
              </span>
              <ZTag>
                <span class="inline-flex items-center gap-1.5">
                  <span class="size-2 shrink-0 rounded-full" :class="tone.dot" aria-hidden="true" />
                  {{ tone.label }}
                </span>
              </ZTag>
            </template>
            <p v-else-if="error" class="m-0 text-z text-z-error-text">
              {{ t('keyStatus.failed') }}
            </p>
            <p v-else class="m-0 text-z text-z-text-secondary">
              {{ t('keyStatus.loading') }}
            </p>
          </div>
          <p v-if="snapshot" class="mt-1 mb-0 text-z-sm text-z-text-secondary">
            {{ t('keyStatus.observed') }}
            <time :datetime="snapshot.observedAt">
              <LastUsedAtCell :value="snapshot.observedAt" :full-display="null" />
            </time>
          </p>
        </ZCard>
      </div>
    </ZScrollbar>
  </main>
</template>
