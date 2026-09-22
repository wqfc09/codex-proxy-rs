<script setup lang="ts">
import type { EChartsOption } from 'echarts'
import type { UserUsageBreakdown } from '@/api'
import { computed, shallowRef } from 'vue'
import BaseCard from '@/components/base/BaseCard.vue'
import BaseEmpty from '@/components/base/BaseEmpty.vue'
import BaseSegmented from '@/components/base/BaseSegmented.vue'
import BaseChart from '@/components/charts/BaseChart.vue'
import { useChartPalette } from '@/composables/useChartPalette'

const props = defineProps<{ title: string, items: UserUsageBreakdown[] }>()
const metric = shallowRef('requestCount')
const { palette } = useChartPalette()
const options = [{ label: '请求', value: 'requestCount' }, { label: 'Token', value: 'totalTokens' }]
const number = new Intl.NumberFormat('en', { notation: 'compact', maximumFractionDigits: 1 })
const colors = computed(() => [palette.value.info, palette.value.success, palette.value.reasoning, palette.value.warning, palette.value.normal, palette.value.textMuted])
const rows = computed(() => {
  const key = metric.value === 'totalTokens' ? 'totalTokens' : 'requestCount'
  const sorted = props.items.filter(item => !item.isOther).sort((a, b) => b[key] - a[key])
  const leading = sorted.slice(0, 5)
  const rest = [...sorted.slice(5), ...props.items.filter(item => item.isOther)]
  if (rest.length) {
    leading.push({
      id: null,
      name: '其他',
      isOther: true,
      requestCount: rest.reduce((sum, item) => sum + item.requestCount, 0),
      totalTokens: rest.reduce((sum, item) => sum + item.totalTokens, 0),
    })
  }
  return leading.map((item, index) => ({ ...item, value: item[key], color: colors.value[index % colors.value.length] }))
})
const total = computed(() => rows.value.reduce((sum, item) => sum + item.value, 0))
const option = computed<EChartsOption>(() => ({
  animationDuration: 220,
  tooltip: { trigger: 'item', renderMode: 'richText', backgroundColor: palette.value.surface, borderColor: palette.value.border, textStyle: { color: palette.value.textPrimary } },
  series: [{
    type: 'pie',
    radius: ['70%', '88%'],
    center: ['50%', '50%'],
    label: { show: false },
    labelLine: { show: false },
    emphasis: { scaleSize: 3 },
    data: rows.value.filter(item => item.value > 0).map(item => ({ name: item.name, value: item.value, itemStyle: { color: item.color } })),
  }],
}))
</script>

<template>
  <BaseCard :title="title" padding="compact" class="usage-distribution min-h-[20rem] min-w-0">
    <template #actions>
      <BaseSegmented v-model="metric" :label="`${title}统计方式`" :options="options" size="sm" />
    </template>
    <div v-if="total > 0" class="distribution-content grid min-w-0 items-center gap-3">
      <div class="relative mx-auto w-full max-w-40 min-w-0" aria-hidden="true">
        <BaseChart :option="option" :height="190" style="height: 11.875rem" />
        <div class="pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-1">
          <strong class="font-mono text-cp-xl text-cp-text">{{ number.format(total) }}</strong>
          <span class="text-cp-xs text-cp-text-tertiary">{{ metric === 'totalTokens' ? 'Token' : '请求' }}</span>
        </div>
      </div>
      <ul class="m-0 grid min-w-0 list-none gap-2.5 p-0">
        <li v-for="row in rows" :key="row.isOther ? 'other' : row.id ?? 'unknown'" class="flex min-w-0 items-center gap-2 text-cp-xs">
          <span class="size-1.5 shrink-0 rounded-full" :style="{ backgroundColor: row.color }" />
          <span class="min-w-0 flex-1 truncate text-cp-text" :title="row.name">{{ row.name }}</span>
          <span class="shrink-0 font-mono tabular-nums text-cp-text-secondary" :title="String(row.value)">{{ number.format(row.value) }}</span>
          <span class="w-10 shrink-0 text-right font-mono tabular-nums text-cp-text-tertiary">{{ (row.value / total * 100).toFixed(0) }}%</span>
        </li>
      </ul>
    </div>
    <BaseEmpty v-else size="sm" surface="none" :title="metric === 'totalTokens' && items.length ? '暂无 Token 记录' : '暂无使用数据'" class="min-h-60 content-center" />
  </BaseCard>
</template>

<style scoped>
.usage-distribution {
  container: distribution / inline-size;
}
.distribution-content {
  grid-template-columns: minmax(0, 1fr);
}
@container distribution (min-width: 22rem) {
  .distribution-content {
    grid-template-columns: 9rem minmax(0, 1fr);
  }
}
</style>
