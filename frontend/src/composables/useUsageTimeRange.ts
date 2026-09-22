import { computed, shallowRef } from 'vue'

export type UsageTimeRange = 'today' | '7d' | '30d'
export interface UsageTimeRangeParams extends Record<string, string> {
  startTime: string
  endTime: string
}
export const usageTimeRangeOptions = [
  { label: '今天', value: 'today' },
  { label: '最近 7 天', value: '7d' },
  { label: '最近 30 天', value: '30d' },
]
const day = 86400000
const beijingOffset = 8 * 3600000

export function buildUsageTimeRange(range: UsageTimeRange, end = new Date()): UsageTimeRangeParams {
  const days = range === '30d' ? 30 : range === '7d' ? 7 : 1
  return usageRangeForDays(days, end)
}

export function usageRangeForDays(days: number, end = new Date()): UsageTimeRangeParams {
  const midnight = Math.floor((end.getTime() + beijingOffset) / day) * day - beijingOffset
  const start = midnight - (Math.max(1, Math.floor(days)) - 1) * day
  return { startTime: new Date(start).toISOString(), endTime: end.toISOString() }
}

export function useUsageTimeRange(initialRange: UsageTimeRange = 'today') {
  const timeRange = shallowRef<UsageTimeRange>(initialRange)
  const rangeEnd = shallowRef(new Date())
  const timeRangeParams = computed(() => buildUsageTimeRange(timeRange.value, rangeEnd.value))
  const timeRangeLabel = computed(() => usageTimeRangeOptions.find(option => option.value === timeRange.value)?.label ?? '今天')
  function refreshTimeRangeEnd() {
    rangeEnd.value = new Date()
  }
  function latestTimeRangeParams() {
    return buildUsageTimeRange(timeRange.value)
  }
  return { timeRange, timeRangeLabel, timeRangeParams, refreshTimeRangeEnd, latestTimeRangeParams }
}
