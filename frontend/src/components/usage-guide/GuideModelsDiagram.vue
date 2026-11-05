<script setup lang="ts">
import { FileCode, ListChecks, Terminal } from '@lucide/vue'
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import GuideMark from './GuideMark.vue'

const props = defineProps<{ apiBaseUrl: string }>()

const { t } = useI18n({ useScope: 'global' })

// 可用模型随 Key 与上游账号变化，示例只写占位符，具体名称从 /model 列表获取
const exampleModel = 'model-name'

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
// Codex 用 API Key 认证时只有同时满足这两项才拉取网关目录，否则只显示内置模型
const snippet = computed(() => [
  { text: `model = "${exampleModel}"` },
  { text: 'model_provider = "OpenAI"' },
  { text: '' },
  { text: '[model_providers.OpenAI]' },
  { text: `model_catalog_url = ${JSON.stringify(`${props.apiBaseUrl.replace(/\/+$/, '')}/models`)}`, mark: 1 },
  { text: `# ${t('guide.models.keepProvider')}`, comment: true },
  { text: '' },
  { text: '[features]' },
  { text: 'api_key_model_discovery = true', mark: 2 },
])
const requirements = computed(() => [
  {
    mark: 1,
    name: 'model_catalog_url',
    detail: t('guide.models.catalogUrl'),
  },
  {
    mark: 2,
    name: 'api_key_model_discovery',
    detail: t('guide.models.discovery'),
  },
])
</script>

<template>
  <div class="@container">
    <div class="grid min-w-0 grid-cols-1 gap-4 @4xl:grid-cols-2">
      <ol class="m-0 grid list-none grid-cols-1 content-start gap-3 p-0 @2xl:grid-cols-3 @4xl:grid-cols-1">
        <li v-for="item in methods" :key="item.code" class="flex min-w-0 items-start gap-3 rounded-z-card bg-z-fill-quaternary p-4">
          <component :is="item.icon" class="mt-0.5 size-4.5 shrink-0 text-z-primary" aria-hidden="true" />
          <span class="flex min-w-0 flex-col gap-1">
            <span class="text-z-sm font-emphasis text-z-text">{{ item.title }}</span>
            <code class="font-mono text-z-sm wrap-break-word text-z-text-secondary">{{ item.code }}</code>
            <span class="text-z-sm text-z-text-tertiary">{{ item.detail }}</span>
          </span>
        </li>
      </ol>

      <div class="flex min-w-0 flex-col gap-3">
        <div class="min-w-0 overflow-hidden rounded-z-card bg-z-fill-quaternary">
          <div class="px-4 py-2.5 font-mono text-z-sm font-emphasis text-z-text-secondary">
            config.toml
          </div>
          <pre tabindex="0" :aria-label="t('guide.models.snippet')" class="mx-3 mb-3 overflow-x-auto rounded-z bg-z-bg-container px-3.5 py-3 font-mono text-z-sm leading-[1.65] text-z-text shadow-z-tertiary"><span v-for="(line, index) in snippet" :key="index" class="flex min-h-[1.65em] w-max items-center gap-2" :class="{ 'text-z-text-tertiary': line.comment }"><span class="whitespace-pre">{{ line.text }}</span><GuideMark v-if="line.mark" :value="line.mark" /></span></pre>
        </div>

        <div class="flex min-w-0 flex-col gap-2">
          <span class="text-z-sm font-emphasis text-z-text-secondary">{{ t('guide.models.requirements') }}</span>
          <ol class="m-0 flex list-none flex-col gap-2 p-0">
            <li v-for="item in requirements" :key="item.mark" class="flex min-w-0 items-start gap-2.5">
              <GuideMark class="mt-0.5" :value="item.mark" />
              <span class="flex min-w-0 flex-col gap-0.5">
                <code class="font-mono text-z-sm wrap-break-word text-z-text">{{ item.name }}</code>
                <span class="text-z-xs text-z-text-tertiary">{{ item.detail }}</span>
              </span>
            </li>
          </ol>
        </div>
      </div>
    </div>
  </div>
</template>
