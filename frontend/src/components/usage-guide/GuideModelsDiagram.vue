<script setup lang="ts">
import { FileCode, ListChecks, Terminal } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

const { t } = useI18n({ useScope: 'global' })

// 模型列表由 model_catalog_url 从网关拉取，示例模型只用于说明写法
const exampleModel = 'kimi-k3.1'

const methods = computed(() => [
  {
    icon: ListChecks,
    title: t('guide.models.pick'),
    code: '/model',
    detail: t('guide.models.pickDetail'),
  },
  {
    icon: FileCode,
    title: t('guide.models.default'),
    code: `model = "${exampleModel}"`,
    detail: t('guide.models.defaultDetail'),
  },
  {
    icon: Terminal,
    title: t('guide.models.once'),
    code: `codex -m ${exampleModel}`,
    detail: t('guide.models.onceDetail'),
  },
])
const snippet = computed(() => [
  `model = "${exampleModel}"`,
  'model_provider = "OpenAI"',
  '',
  '[model_providers.OpenAI]',
  `# ${t('guide.models.keepProvider')}`,
])
</script>

<template>
  <div class="@container">
    <div class="grid min-w-0 grid-cols-1 gap-4 @4xl:grid-cols-[minmax(0,1fr)_minmax(0,24rem)]">
      <ol class="m-0 grid list-none grid-cols-1 gap-3 p-0 @2xl:grid-cols-3 @4xl:grid-cols-1">
        <li v-for="item in methods" :key="item.code" class="flex min-w-0 items-start gap-3 rounded-z-card bg-z-fill-quaternary p-4">
          <component :is="item.icon" class="mt-0.5 size-4.5 shrink-0 text-z-primary" aria-hidden="true" />
          <span class="flex min-w-0 flex-col gap-1">
            <span class="text-z-sm font-emphasis text-z-text">{{ item.title }}</span>
            <code class="font-mono text-z-sm wrap-break-word text-z-text-secondary">{{ item.code }}</code>
            <span class="text-z-sm text-z-text-tertiary">{{ item.detail }}</span>
          </span>
        </li>
      </ol>

      <div class="min-w-0 self-start overflow-hidden rounded-z-card bg-z-fill-quaternary">
        <div class="px-4 py-2.5 font-mono text-z-sm font-emphasis text-z-text-secondary">
          config.toml
        </div>
        <pre tabindex="0" :aria-label="t('guide.models.snippet')" class="mx-3 mb-3 overflow-x-auto rounded-z bg-z-bg-container px-3.5 py-3 font-mono text-z-sm leading-[1.65] text-z-text shadow-z-tertiary"><span v-for="(line, index) in snippet" :key="index" class="block min-h-[1.65em] w-max whitespace-pre" :class="{ 'text-z-text-tertiary': line.startsWith('#') }">{{ line }}</span></pre>
      </div>
    </div>
  </div>
</template>
