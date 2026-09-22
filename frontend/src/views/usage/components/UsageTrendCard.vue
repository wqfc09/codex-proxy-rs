<script setup lang="ts">
import type { EChartsOption } from 'echarts'
import type { UserUsageSummary } from '@/api'
import { computed, shallowRef } from 'vue'
import BaseCard from '@/components/base/BaseCard.vue'
import BaseEmpty from '@/components/base/BaseEmpty.vue'
import BaseSegmented from '@/components/base/BaseSegmented.vue'
import BaseSkeleton from '@/components/base/BaseSkeleton.vue'
import BaseChart from '@/components/charts/BaseChart.vue'
import { useChartPalette } from '@/composables/useChartPalette'

const props = withDefaults(defineProps<{ summary: UserUsageSummary | null, loading?: boolean, title?: string, embedded?: boolean }>(), { loading: false, title: '用量趋势', embedded: false })
const metric = shallowRef('requests')
const options = [{ label: '请求', value: 'requests' }, { label: 'Token', value: 'tokens' }, { label: '已计费', value: 'billed' }]
const { palette } = useChartPalette()
const points = computed(() => {
  const summary = props.summary
  if (!summary)
    return []
  const interval = summary.trendGranularity === '1h' ? 3600000 : 86400000
  const offset = 8 * 3600000
  const start = Math.floor((Date.parse(summary.startTime) + offset) / interval) * interval - offset
  const end = Date.parse(summary.endTime)
  const recorded = new Map((summary.trend ?? summary.daily).map(point => [Date.parse(point.bucketStart), point]))
  const result = []
  // 没有请求的桶补零；存在请求但计费未知的桶保留 null，不伪造零费用。
  for (let time = start; time < end && result.length < 400; time += interval) {
    const point = recorded.get(time)
    result.push({
      time,
      requests: point?.requestCount ?? 0,
      failures: point?.failureCount ?? 0,
      tokens: point?.totalTokens ?? 0,
      billed: point ? point.billedUsd === null ? null : Number(point.billedUsd) : 0,
    })
  }
  return result
})
const option = computed<EChartsOption>(() => {
  const colors = palette.value
  const name = metric.value === 'tokens' ? 'Token' : metric.value === 'billed' ? '已知计费 USD' : '请求'
  const data = points.value.map(point => metric.value === 'tokens' ? point.tokens : metric.value === 'billed' ? point.billed : point.requests)
  return {
    animationDuration: 180,
    grid: { left: 4, right: 12, top: 32, bottom: 4, containLabel: true },
    legend: { top: 0, right: 0, textStyle: { color: colors.textSecondary } },
    tooltip: { trigger: 'axis', renderMode: 'richText', backgroundColor: colors.surface, borderColor: colors.border, textStyle: { color: colors.textPrimary } },
    xAxis: {
      type: 'category',
      boundaryGap: false,
      data: points.value.map(point => new Date(point.time).toLocaleString('zh-CN', props.summary?.trendGranularity === '1h' ? { hour: '2-digit', minute: '2-digit', timeZone: 'Asia/Shanghai', hour12: false } : { month: '2-digit', day: '2-digit', timeZone: 'Asia/Shanghai' })),
      axisTick: { show: false },
      axisLine: { lineStyle: { color: colors.divider } },
      axisLabel: { color: colors.textSecondary, hideOverlap: true },
    },
    yAxis: { type: 'value', minInterval: metric.value === 'billed' ? undefined : 1, axisLabel: { color: colors.textSecondary }, splitLine: { lineStyle: { color: colors.grid, type: 'dashed' } } },
    series: [
      { name, type: 'line', showSymbol: false, data, lineStyle: { color: colors.info, width: 2 }, itemStyle: { color: colors.info }, areaStyle: { color: colors.info, opacity: 0.07 } },
      ...(metric.value === 'requests' ? [{ name: '未成功', type: 'line' as const, showSymbol: false, data: points.value.map(point => point.failures), lineStyle: { color: colors.danger, width: 1.5 }, itemStyle: { color: colors.danger } }] : []),
    ],
  }
})
</script>

<template>
  <BaseCard
    :title="embedded ? undefined : title"
    :padding="embedded ? 'none' : 'compact'"
    class="min-w-0"
    :class="embedded ? 'h-full min-h-0 overflow-visible! rounded-none! bg-transparent! shadow-none!' : 'min-h-[22rem]'"
  >
    <template #actions>
      <div class="flex flex-wrap items-center justify-end gap-2">
        <slot name="actions" />
        <BaseSegmented v-model="metric" label="趋势指标" :options="options" size="sm" />
      </div>
    </template>
    <BaseSkeleton v-if="loading" :class="embedded ? 'min-h-40 flex-1' : 'h-70'" />
    <BaseChart
      v-else-if="summary?.requestCount"
      :option="option"
      :height="embedded ? '100%' : 280"
      role="img"
      :aria-label="title"
    />
    <BaseEmpty
      v-else
      title="当前范围暂无请求"
      surface="none"
      size="sm"
      class="content-center"
      :class="embedded ? 'min-h-40 flex-1' : 'min-h-70'"
    />
  </BaseCard>
</template>
