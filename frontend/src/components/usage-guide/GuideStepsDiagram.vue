<script setup lang="ts">
import { ZTag } from '@codex-proxy/ui'
import { ChartNoAxesColumn, ChevronRight, FileCode, RotateCcw, Terminal, Upload } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

const { t } = useI18n({ useScope: 'global' })

const steps = computed(() => [
  {
    icon: Terminal,
    title: t('guide.steps.getConfig'),
    detail: t('guide.steps.getConfigDetail'),
  },
  {
    icon: FileCode,
    title: t('guide.steps.write'),
    detail: t('guide.steps.writeDetail'),
  },
  {
    icon: RotateCcw,
    title: t('guide.steps.restart'),
    detail: t('guide.steps.restartDetail'),
  },
  {
    icon: ChartNoAxesColumn,
    title: t('guide.steps.verify'),
    detail: t('guide.steps.verifyDetail'),
  },
])
const writeModes = computed(() => [
  {
    icon: FileCode,
    title: t('guide.steps.manual'),
    detail: t('guide.steps.manualDetail'),
    recommended: true,
  },
  {
    icon: Upload,
    title: t('guide.steps.ccSwitch'),
    detail: t('guide.steps.ccSwitchDetail'),
    recommended: false,
  },
])
</script>

<template>
  <div class="@container flex flex-col gap-4">
    <ol class="m-0 grid list-none grid-cols-1 gap-3 p-0 @xl:grid-cols-2 @4xl:grid-cols-4">
      <li v-for="(step, index) in steps" :key="step.title" class="relative flex min-w-0 gap-3 rounded-z-card bg-z-fill-quaternary p-4">
        <span class="relative flex size-10 shrink-0 items-center justify-center rounded-z bg-z-bg-container text-z-primary-text shadow-z-tertiary">
          <component :is="step.icon" class="size-5" aria-hidden="true" />
          <span class="absolute -top-1.5 -right-1.5 flex size-5 items-center justify-center rounded-full bg-z-primary text-z-xs leading-none font-emphasis text-z-text-light-solid">{{ index + 1 }}</span>
        </span>
        <span class="flex min-w-0 flex-col gap-1">
          <span class="font-emphasis text-z-text">{{ step.title }}</span>
          <span class="text-z-sm text-z-text-tertiary">{{ step.detail }}</span>
        </span>
        <!-- 只在四列横排时提示顺序，换行后编号已能表达先后 -->
        <ChevronRight v-if="index < steps.length - 1" class="absolute top-1/2 -right-3 z-1 hidden size-4 -translate-y-1/2 text-z-text-quaternary @4xl:block" aria-hidden="true" />
      </li>
    </ol>

    <div class="flex flex-col gap-2">
      <span class="text-z-sm font-emphasis text-z-text-secondary">{{ t('guide.steps.writeModes') }}</span>
      <div class="grid grid-cols-1 gap-3 @xl:grid-cols-2">
        <div v-for="mode in writeModes" :key="mode.title" class="flex min-w-0 items-start gap-3 rounded-z-card bg-z-fill-quaternary p-4">
          <component :is="mode.icon" class="mt-0.5 size-4.5 shrink-0 text-z-text-secondary" aria-hidden="true" />
          <span class="flex min-w-0 flex-col gap-1">
            <span class="flex flex-wrap items-center gap-2 font-emphasis text-z-text">
              {{ mode.title }}
              <ZTag v-if="mode.recommended" type="success" size="small">{{ t('guide.steps.recommended') }}</ZTag>
            </span>
            <span class="text-z-sm text-z-text-tertiary">{{ mode.detail }}</span>
          </span>
        </div>
      </div>
    </div>
  </div>
</template>
