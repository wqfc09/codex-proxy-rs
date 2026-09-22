<script setup lang="ts">
import type { Component } from 'vue'
import BaseCard from '@/components/base/BaseCard.vue'
import BaseMotionIcon from '@/components/base/BaseMotionIcon.vue'

defineProps<{
  label: string
  value: string
  caption: string
  icon: Component
  tone?: 'normal' | 'info' | 'success'
}>()
</script>

<template>
  <BaseCard as="article" padding="compact" class="min-w-0">
    <div class="flex min-w-0 items-center gap-2.5">
      <BaseMotionIcon
        class="inline-flex size-8 shrink-0 items-center justify-center rounded-cp"
        :class="tone === 'success' ? 'bg-cp-success-container text-cp-success-on-container' : tone === 'info' ? 'bg-cp-info-container text-cp-info-on-container' : 'bg-cp-fill-tertiary text-cp-text-secondary'"
      >
        <component :is="icon" class="size-4" />
      </BaseMotionIcon>
      <h2 class="m-0 min-w-0 text-cp font-emphasis text-cp-text-secondary">
        {{ label }}
      </h2>
    </div>
    <p class="mt-3 mb-0 truncate font-mono text-cp-xl leading-tight font-heavy tabular-nums text-cp-text" :title="value">
      {{ value }}
    </p>
    <p class="mt-1.5 mb-0 truncate text-cp-xs text-cp-text-tertiary" :title="caption">
      {{ caption }}
    </p>
  </BaseCard>
</template>
