<script setup lang="ts">
import { ChartNoAxesColumn, KeyRound, Laptop, Route, Server, ShieldCheck } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import GuideConnector from './GuideConnector.vue'

const props = defineProps<{ apiBaseUrl: string }>()

const { t } = useI18n({ useScope: 'global' })

const gatewayFeatures = computed(() => [
  {
    icon: ShieldCheck,
    label: t('guide.flow.keyAuth'),
  },
  {
    icon: Route,
    label: t('guide.flow.routing'),
  },
  {
    icon: ChartNoAxesColumn,
    label: t('guide.flow.metering'),
  },
])
const upstreamAccounts = computed(() => [
  t('guide.flow.upstreamOAuth'),
  t('guide.flow.upstreamApiKey'),
])
</script>

<template>
  <figure class="@container m-0" :aria-label="t('guide.flow.title')">
    <div class="flex flex-col items-stretch @3xl:flex-row @3xl:items-center">
      <div class="flex min-w-0 flex-1 items-center gap-3 rounded-z-card bg-z-fill-quaternary p-4">
        <span class="flex size-10 shrink-0 items-center justify-center rounded-z bg-z-bg-container text-z-text-secondary shadow-z-tertiary">
          <Laptop class="size-5" aria-hidden="true" />
        </span>
        <span class="flex min-w-0 flex-col gap-1">
          <span class="font-emphasis text-z-text">{{ t('guide.flow.client') }}</span>
          <span class="text-z-xs text-z-text-tertiary">{{ t('guide.flow.clientDetail') }}</span>
          <span class="inline-flex w-fit items-center gap-1 rounded-z-sm bg-z-bg-container px-1.5 py-0.5 text-z-xs text-z-text-secondary">
            <KeyRound class="size-3" aria-hidden="true" />
            <code class="font-mono">config.toml</code>
          </span>
        </span>
      </div>

      <GuideConnector :label="t('guide.flow.request')" :return-label="t('guide.flow.response')" />

      <div class="flex min-w-0 flex-[1.3] flex-col gap-3 rounded-z-card bg-z-primary-container p-4">
        <div class="flex items-center gap-3">
          <span class="flex size-10 shrink-0 items-center justify-center rounded-z bg-z-primary text-z-text-light-solid">
            <Server class="size-5" aria-hidden="true" />
          </span>
          <span class="flex min-w-0 flex-col gap-1">
            <span class="font-emphasis text-z-primary-on-container">{{ t('guide.flow.gateway') }}</span>
            <code class="truncate font-mono text-z-xs text-z-primary-text" :title="props.apiBaseUrl">{{ props.apiBaseUrl }}</code>
          </span>
        </div>
        <ul class="m-0 flex list-none flex-wrap gap-1.5 p-0">
          <li v-for="feature in gatewayFeatures" :key="feature.label" class="inline-flex items-center gap-1 rounded-z-sm bg-z-bg-container px-2 py-1 text-z-xs text-z-text-secondary">
            <component :is="feature.icon" class="size-3.5 text-z-primary-text" aria-hidden="true" />
            {{ feature.label }}
          </li>
        </ul>
      </div>

      <GuideConnector :label="t('guide.flow.credential')" />

      <div class="flex min-w-0 flex-1 flex-col gap-2 rounded-z-card bg-z-fill-quaternary p-4">
        <span class="font-emphasis text-z-text">{{ t('guide.flow.upstream') }}</span>
        <ul class="m-0 flex list-none flex-col gap-1.5 p-0">
          <li v-for="account in upstreamAccounts" :key="account" class="flex items-center gap-2 rounded-z-sm bg-z-bg-container px-2.5 py-1.5 text-z-xs text-z-text-secondary">
            <span class="size-1.5 shrink-0 rounded-full bg-z-success" aria-hidden="true" />
            {{ account }}
          </li>
        </ul>
      </div>
    </div>
    <figcaption class="mt-3 text-z-sm text-z-text-tertiary">
      {{ t('guide.flow.caption') }}
    </figcaption>
  </figure>
</template>
