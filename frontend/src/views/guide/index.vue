<script setup lang="ts">
import { ZButton, ZPageHeader } from '@codex-proxy/ui'
import { KeyRound } from '@lucide/vue'
import { useI18n } from 'vue-i18n'
import UsageGuide from '@/components/usage-guide/index.vue'
import { resolveServiceRootUrl } from '@/utils/client'

const { t } = useI18n({ useScope: 'global' })

const apiBaseUrl = `${resolveServiceRootUrl()}/v1`
</script>

<template>
  <div class="flex w-full flex-col gap-5">
    <ZPageHeader :title="t('navigation.guide')" :description="t('guide.description')" />
    <UsageGuide :api-base-url="apiBaseUrl">
      <template #stepsActions>
        <!-- 管理端没有当前 Key，配置需在 API 密钥列表中按具体 Key 生成 -->
        <ZButton @click="$router.push({ name: 'keys' })">
          <KeyRound class="size-4" />
          {{ t('guide.steps.openKeys') }}
        </ZButton>
      </template>
    </UsageGuide>
  </div>
</template>
