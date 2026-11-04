<script setup lang="ts">
defineProps<{ label: string, returnLabel?: string }>()
</script>

<template>
  <div class="flex shrink-0 items-center justify-center gap-2 py-1 @3xl:w-36 @3xl:flex-col @3xl:py-0" aria-hidden="true">
    <!-- 窄容器纵向排列，宽容器横向排列，两种箭头按容器宽度切换 -->
    <svg class="h-12 w-6 @3xl:hidden" viewBox="0 0 24 48" fill="none">
      <path class="guide-flow-line stroke-z-primary" d="M8 2 V42" stroke-width="2" stroke-linecap="round" />
      <path class="fill-z-primary" d="M3.5 38 L8 46 L12.5 38 Z" />
      <path v-if="returnLabel" class="guide-flow-line stroke-z-text-quaternary" d="M18 46 V6" stroke-width="2" stroke-linecap="round" />
      <path v-if="returnLabel" class="fill-z-text-quaternary" d="M13.5 10 L18 2 L22.5 10 Z" />
    </svg>
    <svg class="hidden h-6 w-full @3xl:block" viewBox="0 0 144 24" fill="none" preserveAspectRatio="none">
      <path class="guide-flow-line stroke-z-primary" d="M2 8 H136" stroke-width="2" stroke-linecap="round" vector-effect="non-scaling-stroke" />
      <path class="fill-z-primary" d="M134 3.5 L142 8 L134 12.5 Z" />
      <path v-if="returnLabel" class="guide-flow-line stroke-z-text-quaternary" d="M142 18 H8" stroke-width="2" stroke-linecap="round" vector-effect="non-scaling-stroke" />
      <path v-if="returnLabel" class="fill-z-text-quaternary" d="M10 13.5 L2 18 L10 22.5 Z" />
    </svg>
    <span class="flex flex-col gap-0.5 text-z-xs leading-4 @3xl:items-center @3xl:text-center">
      <span class="font-emphasis text-z-primary-text">{{ label }}</span>
      <span v-if="returnLabel" class="text-z-text-tertiary">{{ returnLabel }}</span>
    </span>
  </div>
</template>

<style scoped>
.guide-flow-line {
  stroke-dasharray: 6 5;
  animation: guide-flow 1.2s linear infinite;
}

@keyframes guide-flow {
  to {
    stroke-dashoffset: -11;
  }
}

@media (prefers-reduced-motion: reduce) {
  .guide-flow-line {
    animation: none;
  }
}
</style>
