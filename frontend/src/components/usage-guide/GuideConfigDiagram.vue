<script setup lang="ts">
import { ZSegmented } from '@codex-proxy/ui'
import { Apple, FileCode, FileKey, Folder, Monitor } from '@lucide/vue'
import { computed, shallowRef } from 'vue'
import { useI18n } from 'vue-i18n'

const props = defineProps<{ apiBaseUrl: string }>()

const { t } = useI18n({ useScope: 'global' })

const platform = shallowRef('unix')
const platformOptions = [
  { label: 'macOS / Linux', value: 'unix', icon: Apple },
  { label: 'Windows', value: 'windows', icon: Monitor },
]
const configDir = computed(() => platform.value === 'windows'
  ? '%USERPROFILE%\\.codex\\'
  : '~/.codex/')

// 只展示接入必需的字段，完整模板仍由配置弹窗按当前 Key 生成
const snippet = computed(() => {
  const baseUrl = props.apiBaseUrl.replace(/\/+$/, '')
  return [
    { text: 'model_provider = "OpenAI"', mark: 1 },
    { text: '' },
    { text: '[model_providers.OpenAI]', mark: 1 },
    { text: `base_url = ${JSON.stringify(baseUrl)}`, mark: 2 },
    { text: `model_catalog_url = ${JSON.stringify(`${baseUrl}/models`)}`, mark: 2 },
    { text: 'wire_api = "responses"' },
    { text: 'supports_websockets = false', mark: 3 },
    { text: 'requires_openai_auth = false', mark: 4 },
    { text: 'experimental_bearer_token = "<API Key>"', mark: 5 },
  ]
})
const markClass = 'inline-flex size-4.5 shrink-0 items-center justify-center rounded-full bg-z-primary font-sans text-[0.6875rem] leading-none text-z-text-light-solid'
const fields = computed(() => [
  {
    mark: 1,
    name: 'model_provider',
    detail: t('guide.config.fields.provider'),
  },
  {
    mark: 2,
    name: 'base_url',
    detail: t('guide.config.fields.baseUrl'),
  },
  {
    mark: 3,
    name: 'supports_websockets',
    detail: t('guide.config.fields.websocket'),
  },
  {
    mark: 4,
    name: 'requires_openai_auth',
    detail: t('guide.config.fields.auth'),
  },
  {
    mark: 5,
    name: 'experimental_bearer_token',
    detail: t('guide.config.fields.token'),
  },
])
</script>

<template>
  <div class="@container flex flex-col gap-4">
    <ZSegmented v-model="platform" class="self-start" :aria-label="t('keys.configuration.platform')" :options="platformOptions" />
    <div class="grid min-w-0 grid-cols-1 gap-4 @4xl:grid-cols-[minmax(0,15rem)_minmax(0,1fr)_minmax(0,18rem)]">
      <div class="flex min-w-0 flex-col gap-2 rounded-z-card bg-z-fill-quaternary p-4" :aria-label="t('guide.config.location')">
        <span class="text-z-sm font-emphasis text-z-text-secondary">{{ t('guide.config.location') }}</span>
        <ul class="m-0 flex list-none flex-col gap-1 p-0 font-mono text-z-sm">
          <li class="flex min-w-0 items-center gap-2 text-z-text">
            <Folder class="size-4 shrink-0 text-z-warning" aria-hidden="true" />
            <span class="truncate" :title="configDir">{{ configDir }}</span>
          </li>
          <li class="ml-3 flex min-w-0 items-center gap-2 rounded-z-sm bg-z-primary-container px-2 py-1 text-z-primary-on-container">
            <FileCode class="size-4 shrink-0" aria-hidden="true" />
            <span class="min-w-0 flex-1 truncate">config.toml</span>
            <span class="shrink-0 font-sans text-z-xs">{{ t('guide.config.writeHere') }}</span>
          </li>
          <li class="ml-3 flex min-w-0 items-center gap-2 px-2 py-1 text-z-text-tertiary">
            <FileKey class="size-4 shrink-0" aria-hidden="true" />
            <span class="min-w-0 flex-1 truncate">auth.json</span>
            <span class="shrink-0 font-sans text-z-xs">{{ t('guide.config.authKept') }}</span>
          </li>
        </ul>
        <span class="mt-1 text-z-xs text-z-text-tertiary">{{ t('guide.config.codexHome') }}</span>
      </div>

      <div class="min-w-0 overflow-hidden rounded-z-card bg-z-fill-quaternary">
        <div class="px-4 py-2.5 font-mono text-z-sm font-emphasis text-z-text-secondary">
          config.toml
        </div>
        <pre tabindex="0" :aria-label="t('guide.config.snippet')" class="mx-3 mb-3 overflow-x-auto rounded-z bg-z-bg-container px-3.5 py-3 font-mono text-z-sm leading-[1.65] text-z-text shadow-z-tertiary"><span v-for="(line, index) in snippet" :key="index" class="flex min-h-[1.65em] w-max items-center gap-2"><span class="whitespace-pre">{{ line.text }}</span><span v-if="line.mark" :class="markClass" aria-hidden="true">{{ line.mark }}</span></span></pre>
      </div>

      <ol class="m-0 flex list-none flex-col gap-2 p-0">
        <li v-for="field in fields" :key="field.mark" class="flex min-w-0 items-start gap-2.5">
          <span class="mt-0.5" :class="[markClass]" aria-hidden="true">{{ field.mark }}</span>
          <span class="flex min-w-0 flex-col gap-0.5">
            <code class="truncate font-mono text-z-sm text-z-text">{{ field.name }}</code>
            <span class="text-z-xs text-z-text-tertiary">{{ field.detail }}</span>
          </span>
        </li>
      </ol>
    </div>
  </div>
</template>
