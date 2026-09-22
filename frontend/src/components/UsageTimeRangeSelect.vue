<script setup lang="ts">
import type { UsageTimeRange } from '@/composables/useUsageTimeRange'
import BaseSelect from '@/components/base/BaseSelect.vue'
import { usageTimeRangeOptions } from '@/composables/useUsageTimeRange'

const model = defineModel<UsageTimeRange>({ required: true })
function select(value: string) {
  if (value === 'today' || value === '7d' || value === '30d')
    model.value = value
}
</script>

<template>
  <BaseSelect :model-value="model" :options="usageTimeRangeOptions" class="w-38 max-w-full" aria-label="时间范围" title="按北京时间自然日统计；最近 7/30 天均包含今天" @update:model-value="select" />
</template>
