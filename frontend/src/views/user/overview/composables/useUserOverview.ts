import type { UserBillingSummary, UserClientKey, UserUsageRecord, UserUsageSummary } from '@/api'
import { computed, onBeforeUnmount, onMounted, shallowRef, watch } from 'vue'
import { getApiKeys, getUserBilling, getUserUsageRecords, getUserUsageSummary } from '@/api'
import { useUsageTimeRange } from '@/composables/useUsageTimeRange'

export function useUserOverview() {
  const { timeRange, timeRangeLabel, timeRangeParams, refreshTimeRangeEnd } = useUsageTimeRange()
  const billing = shallowRef<UserBillingSummary | null>(null)
  const usage = shallowRef<UserUsageSummary | null>(null)
  const keys = shallowRef<UserClientKey[]>([])
  const recent = shallowRef<UserUsageRecord[]>([])
  const selectedKeyId = shallowRef('')
  const catalogLoading = shallowRef(true)
  const scopeLoading = shallowRef(true)
  const error = shallowRef('')
  let catalogController: AbortController | undefined
  let scopeController: AbortController | undefined

  const selectedKey = computed(() => keys.value.find(key => key.id === selectedKeyId.value) ?? null)
  const keyOptions = computed(() => [
    { label: '全部密钥', value: '' },
    ...keys.value.map(key => ({ label: key.name, value: key.id })),
  ])
  const loading = computed(() => catalogLoading.value || scopeLoading.value)

  async function loadScope() {
    scopeController?.abort()
    const controller = new AbortController()
    scopeController = controller
    scopeLoading.value = true
    error.value = ''
    usage.value = null
    recent.value = []
    const scope = { ...timeRangeParams.value, clientApiKeyId: selectedKeyId.value || undefined }
    const options = { signal: controller.signal, silent: true }
    try {
      const [summary, records] = await Promise.all([
        getUserUsageSummary(scope, options),
        getUserUsageRecords({ ...scope, currentPage: 1, pageSize: 5 }, options),
      ])
      if (controller.signal.aborted)
        return
      usage.value = summary
      recent.value = records.items
    }
    catch {
      if (!controller.signal.aborted)
        error.value = '用量加载失败'
    }
    finally {
      if (scopeController === controller)
        scopeLoading.value = false
    }
  }

  async function load() {
    refreshTimeRangeEnd()
    catalogController?.abort()
    scopeController?.abort()
    const controller = new AbortController()
    catalogController = controller
    catalogLoading.value = true
    error.value = ''
    try {
      const options = { signal: controller.signal, silent: true }
      const [nextBilling, nextKeys] = await Promise.all([
        getUserBilling(options),
        getApiKeys({ limit: 100, sortBy: 'name', sortDirection: 'asc' }, options),
      ])
      if (controller.signal.aborted)
        return
      billing.value = nextBilling
      keys.value = nextKeys.items
      if (selectedKeyId.value && !keys.value.some(key => key.id === selectedKeyId.value))
        selectedKeyId.value = ''
      await loadScope()
    }
    catch {
      if (!controller.signal.aborted)
        error.value = '账户数据加载失败'
    }
    finally {
      if (catalogController === controller) {
        catalogLoading.value = false
        if (error.value)
          scopeLoading.value = false
      }
    }
  }

  watch([selectedKeyId, timeRange], () => {
    refreshTimeRangeEnd()
    if (!catalogLoading.value)
      void loadScope()
  })
  onMounted(load)
  onBeforeUnmount(() => {
    catalogController?.abort()
    scopeController?.abort()
  })
  return { timeRange, timeRangeLabel, billing, usage, keys, recent, selectedKeyId, selectedKey, keyOptions, loading, catalogLoading, scopeLoading, error, load }
}
