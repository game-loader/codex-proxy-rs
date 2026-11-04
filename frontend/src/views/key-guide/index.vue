<script setup lang="ts">
import { ZButton, ZIconButton, ZPageHeader, ZScrollbar } from '@codex-proxy/ui'
import { ArrowLeft, Moon, Sun, Terminal } from '@lucide/vue'
import { useI18n } from 'vue-i18n'
import ApiKeyConfigModal from '@/components/ApiKeyConfigModal.vue'
import AppLanguageMenu from '@/components/AppLanguageMenu.vue'
import UsageGuide from '@/components/usage-guide/index.vue'
import { useThemeStore } from '@/stores/modules/theme'
import { useKeyConfig } from '@/views/key-usage/composables/useKeyConfig'

const { t } = useI18n({ useScope: 'global' })

const theme = useThemeStore()
const { showConfig, configKey, configuring, apiBaseUrl, openConfig, copyConfig } = useKeyConfig()
</script>

<template>
  <main class="h-dvh overflow-hidden bg-z-bg-layout text-z-text">
    <ZScrollbar>
      <div class="flex min-h-full w-full flex-col gap-5 p-4 min-[961px]:p-6">
        <ZPageHeader :title="t('navigation.guide')" :description="t('guide.description')">
          <template #actions>
            <div class="key-guide-actions flex max-w-[calc(100vw-32px)] flex-wrap items-center justify-end gap-2">
              <ZButton @click="$router.push({ name: 'key-usage' })">
                <ArrowLeft class="size-4" />
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
        <UsageGuide :api-base-url="apiBaseUrl">
          <template #stepsActions>
            <ZButton type="primary" :loading="configuring" @click="openConfig">
              <Terminal class="size-4" />
              {{ t('keys.actions.configure') }}
            </ZButton>
          </template>
        </UsageGuide>
      </div>
    </ZScrollbar>
    <ApiKeyConfigModal v-model="showConfig" :title="t('usage.header.keyConfiguration')" :api-key="configKey" :api-base-url="apiBaseUrl" @copy="copyConfig" @after-leave="configKey = null" />
  </main>
</template>

<style scoped>
/* 与用量页头部一致，页面背景上的次级按钮使用按钮底色区分层级 */
.key-guide-actions {
  --z-icon-button-secondary-bg: var(--z-button-secondary-bg);
  --z-icon-button-secondary-hover-bg: var(--z-button-secondary-hover-bg);
  --z-icon-button-secondary-active-bg: var(--z-button-secondary-active-bg);
}
</style>
