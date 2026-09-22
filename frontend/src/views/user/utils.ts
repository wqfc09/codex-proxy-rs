import { usageRangeForDays } from '@/composables/useUsageTimeRange'

export function userRange(days = 30) {
  return usageRangeForDays(days)
}

export function displayMoney(value: string | null | undefined) {
  if (value === null || value === undefined || value === '')
    return '未知'
  return `$${value}`
}

export function displayLimit(value: string | number | null | undefined, suffix = '') {
  if (value === null)
    return '不限'
  if (value === undefined || value === '')
    return '—'
  return `${value}${suffix}`
}

export function displayDate(value: string | null | undefined) {
  if (!value)
    return '—'
  const date = new Date(value)
  if (Number.isNaN(date.getTime()))
    return value
  return new Intl.DateTimeFormat('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  }).format(date)
}

export function usageOutcomeText(value: string) {
  const names: Record<string, string> = {
    succeeded: '成功',
    failed: '失败',
    cancelled: '已取消',
    incomplete: '未完成',
  }
  return names[value] ?? value
}

export function subscriptionStatusText(value: string | null | undefined) {
  const names: Record<string, string> = {
    none: '未订阅',
    active: '生效中',
    expired: '已到期',
    revoked: '已撤销',
    pending: '未生效',
    scheduled: '未生效',
    disabled: '已停用',
  }
  return value ? names[value] ?? value : '未订阅'
}
