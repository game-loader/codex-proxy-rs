<script setup lang="ts">
import { ZCard } from '@codex-proxy/ui'
import { CircleAlert } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import GuideConfigDiagram from './GuideConfigDiagram.vue'
import GuideFlowDiagram from './GuideFlowDiagram.vue'
import GuideModelsDiagram from './GuideModelsDiagram.vue'
import GuideStepsDiagram from './GuideStepsDiagram.vue'

defineProps<{ apiBaseUrl: string }>()

// 获取配置的入口随身份不同，Key 页打开当前 Key 的配置，管理端跳转到 API 密钥列表
defineSlots<{ stepsActions?: () => unknown }>()

const { t } = useI18n({ useScope: 'global' })

const troubleshooting = computed(() => [
  {
    title: t('guide.troubleshooting.loginTitle'),
    detail: t('guide.troubleshooting.login'),
  },
  {
    title: '401',
    code: true,
    detail: t('guide.troubleshooting.unauthorized'),
  },
  {
    title: 'client_version_too_old',
    code: true,
    detail: t('guide.troubleshooting.version'),
  },
  {
    title: t('guide.troubleshooting.imageTitle'),
    detail: t('guide.troubleshooting.image'),
  },
  {
    title: '5173/dev/v1',
    code: true,
    detail: t('guide.troubleshooting.devProxy'),
  },
  {
    title: t('guide.troubleshooting.websocketTitle'),
    detail: t('guide.troubleshooting.websocket'),
  },
])
</script>

<template>
  <div class="flex flex-col gap-5">
    <ZCard :title="t('guide.flow.title')" :description="t('guide.flow.description')">
      <GuideFlowDiagram :api-base-url="apiBaseUrl" />
    </ZCard>

    <ZCard :title="t('guide.steps.title')" :description="t('guide.steps.description')">
      <template v-if="$slots.stepsActions" #actions>
        <slot name="stepsActions" />
      </template>
      <GuideStepsDiagram />
    </ZCard>

    <ZCard :title="t('guide.config.title')" :description="t('guide.config.description')">
      <GuideConfigDiagram :api-base-url="apiBaseUrl" />
    </ZCard>

    <ZCard :title="t('guide.models.title')" :description="t('guide.models.description')">
      <GuideModelsDiagram />
    </ZCard>

    <ZCard :title="t('guide.troubleshooting.title')">
      <ul class="m-0 grid list-none grid-cols-1 gap-3 p-0 lg:grid-cols-2">
        <li v-for="item in troubleshooting" :key="item.title" class="flex min-w-0 items-start gap-3 rounded-z-card bg-z-fill-quaternary p-4">
          <CircleAlert class="mt-0.5 size-4.5 shrink-0 text-z-warning" aria-hidden="true" />
          <span class="flex min-w-0 flex-col gap-1">
            <code v-if="item.code" class="font-mono text-z-sm font-emphasis wrap-break-word text-z-text">{{ item.title }}</code>
            <span v-else class="text-z-sm font-emphasis text-z-text">{{ item.title }}</span>
            <span class="text-z-sm text-z-text-tertiary">{{ item.detail }}</span>
          </span>
        </li>
      </ul>
    </ZCard>
  </div>
</template>
