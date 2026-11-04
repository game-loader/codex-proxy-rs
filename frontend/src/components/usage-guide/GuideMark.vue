<script setup lang="ts">
// 配置示例与字段说明共用编号，便于对照同一行配置
defineProps<{ value: number }>()
</script>

<template>
  <span class="inline-flex size-4.5 shrink-0 items-center justify-center rounded-full bg-z-primary font-sans text-[0.6875rem] leading-none text-z-text-light-solid" aria-hidden="true">{{ value }}</span>
</template>
