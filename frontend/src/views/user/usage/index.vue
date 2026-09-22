<script setup lang="ts">
import type { UserUsageDisplayRecord } from './presentation'
import type { UserClientKey, UserUsageSummary } from '@/api'
import { Eye, RefreshCw } from '@lucide/vue'
import { watchDebounced } from '@vueuse/core'
import { computed, onBeforeUnmount, onMounted, reactive, shallowRef } from 'vue'
import { useRoute } from 'vue-router'
import { getApiKeys, getUserUsageRecords, getUserUsageSummary } from '@/api'
import BaseButton from '@/components/base/BaseButton.vue'
import BaseCard from '@/components/base/BaseCard.vue'
import BaseEmpty from '@/components/base/BaseEmpty.vue'
import BaseIconButton from '@/components/base/BaseIconButton.vue'
import BaseInput from '@/components/base/BaseInput.vue'
import BaseModal from '@/components/base/BaseModal/index.vue'
import BasePageHeader from '@/components/base/BasePageHeader.vue'
import BaseSelect from '@/components/base/BaseSelect.vue'
import BaseTablePagination from '@/components/base/BaseTable/BaseTablePagination.vue'
import { defineTableColumns } from '@/components/base/BaseTable/columns'
import UsageTimeRangeSelect from '@/components/UsageTimeRangeSelect.vue'
import { useUsageTimeRange } from '@/composables/useUsageTimeRange'
import UsageLatencyCell from '@/views/usage/components/UsageLatencyCell.vue'
import UsageModelCell from '@/views/usage/components/UsageModelCell.vue'
import UsageRecordsTable from '@/views/usage/components/UsageRecordsTable.vue'
import UsageSummaryCards from '@/views/usage/components/UsageSummaryCards.vue'
import UsageTokenCell from '@/views/usage/components/UsageTokenCell.vue'
import UserUsageInsightsGrid from '@/views/usage/components/UserUsageInsightsGrid.vue'
import { displayDate, displayMoney, usageOutcomeText } from '../utils'
import { presentUserUsage } from './presentation'

const route = useRoute()
const { timeRange, timeRangeParams, refreshTimeRangeEnd } = useUsageTimeRange()
const filters = reactive({ key: typeof route.query.clientApiKeyId === 'string' ? route.query.clientApiKeyId : '', model: '', outcome: '', statusCode: '' })
const keys = shallowRef<UserClientKey[]>([])
const keysLoading = shallowRef(false)
const keyOptions = computed(() => keys.value.map(key => ({ value: key.id, label: key.name, description: key.prefix })))
const rows = shallowRef<UserUsageDisplayRecord[]>([])
const summary = shallowRef<UserUsageSummary | null>(null)
const loading = shallowRef(true)
const summaryLoading = shallowRef(true)
const error = shallowRef('')
const currentPage = shallowRef(1)
const pageSize = shallowRef(10)
const total = shallowRef(0)
const detail = shallowRef<UserUsageDisplayRecord | null>(null)
const detailOpen = shallowRef(false)
let controller: AbortController | undefined
let keyController: AbortController | undefined
let disposed = false
const pagination = computed(() => ({ currentPage: currentPage.value, pageSize: pageSize.value, total: total.value }))
const outcomeOptions = [
  { label: '全部结果', value: '' },
  { label: '成功', value: 'succeeded' },
  { label: '失败', value: 'failed' },
  { label: '已取消', value: 'cancelled' },
  { label: '未完成', value: 'incomplete' },
]
const columns = defineTableColumns<UserUsageDisplayRecord>([
  { key: 'clientApiKeyName', label: 'API Key', kind: 'identity', size: 'xl' },
  { key: 'model', label: '模型', kind: 'custom', size: 'xl' },
  { key: 'tokenDetails', label: 'Token', kind: 'numeric', size: 'xl' },
  { key: 'downstreamBilledAmount', label: '计费', kind: 'numeric', size: 'lg' },
  { key: 'downstreamRateMultiplier', label: '倍率', kind: 'numeric', emptyText: '未知' },
  { key: 'outcome', label: '结果', kind: 'status', format: value => usageOutcomeText(String(value ?? '')) },
  { key: 'clientStatusCode', label: '状态码', kind: 'numeric', emptyText: '—' },
  { key: 'latency', label: '耗时', kind: 'numeric', size: 'xl' },
  { key: 'createdAtDisplay', label: '时间', kind: 'datetime' },
  { key: 'actions', label: '操作', kind: 'actions', size: 'sm' },
])

function query() {
  const status = filters.statusCode.trim() ? Number(filters.statusCode) : undefined
  if (status !== undefined && (!Number.isInteger(status) || status < 100 || status > 599))
    throw new Error('状态码应为 100–599 的整数')
  return { ...timeRangeParams.value, clientApiKeyId: filters.key || undefined, model: filters.model.trim() || undefined, outcome: filters.outcome || undefined, statusCode: status }
}
async function loadKeys() {
  keyController?.abort()
  const request = new AbortController()
  keyController = request
  keysLoading.value = true
  try {
    const result = await getApiKeys({ limit: 100, sortBy: 'name', sortDirection: 'asc' }, { signal: request.signal, silent: true })
    if (!request.signal.aborted)
      keys.value = result.items
  }
  catch {}
  finally {
    if (keyController === request)
      keysLoading.value = false
  }
}
async function load(includeSummary = true) {
  controller?.abort()
  const request = new AbortController()
  controller = request
  loading.value = true
  summaryLoading.value = includeSummary
  error.value = ''
  rows.value = []
  if (includeSummary)
    summary.value = null
  try {
    const scope = query()
    const options = { signal: request.signal, silent: true }
    const [page, stats] = await Promise.all([
      getUserUsageRecords({ ...scope, currentPage: currentPage.value, pageSize: pageSize.value }, options),
      includeSummary ? getUserUsageSummary(scope, options) : Promise.resolve(null),
    ])
    if (request.signal.aborted)
      return
    rows.value = page.items.map(presentUserUsage)
    currentPage.value = page.currentPage
    pageSize.value = page.pageSize
    total.value = page.total
    if (stats)
      summary.value = stats
  }
  catch (cause) {
    if (!request.signal.aborted)
      error.value = cause instanceof Error && cause.message.startsWith('状态码') ? cause.message : '使用记录加载失败'
  }
  finally {
    if (controller === request) {
      loading.value = false
      summaryLoading.value = false
    }
  }
}
function refresh() {
  currentPage.value = 1
  refreshTimeRangeEnd()
  void load()
}
function pageChanged(page: number) {
  currentPage.value = page
  void load(false)
}
function sizeChanged(size: number) {
  pageSize.value = size
  currentPage.value = 1
  void load(false)
}
watchDebounced([timeRange, () => filters.key, () => filters.model, () => filters.outcome, () => filters.statusCode], () => {
  if (!disposed)
    refresh()
}, { debounce: 250 })
onMounted(() => {
  void loadKeys()
  refresh()
})
onBeforeUnmount(() => {
  disposed = true
  controller?.abort()
  keyController?.abort()
})
</script>

<template>
  <div class="w-full min-w-0">
    <BasePageHeader title="使用记录">
      <template #actions>
        <UsageTimeRangeSelect v-model="timeRange" />
        <BaseButton variant="secondary" :loading="loading" @click="refresh">
          <template #icon>
            <RefreshCw class="size-4" />
          </template>
          刷新
        </BaseButton>
      </template>
    </BasePageHeader>
    <UsageSummaryCards :user-summary="summary" />
    <UserUsageInsightsGrid :summary="summary" :loading="summaryLoading" />
    <BaseCard class="mt-4 min-w-0" title="请求明细" padding="compact">
      <div class="grid min-w-0 gap-3 sm:grid-cols-2 xl:grid-cols-4" aria-label="使用记录筛选">
        <BaseSelect v-model="filters.key" :options="[{ label: '全部 API Key', value: '' }, ...keyOptions]" :disabled="keysLoading" aria-label="API Key" />
        <BaseInput v-model="filters.model" placeholder="模型" aria-label="模型" />
        <BaseSelect v-model="filters.outcome" :options="outcomeOptions" aria-label="结果" />
        <BaseInput v-model="filters.statusCode" type="number" placeholder="状态码" aria-label="状态码" min="100" max="599" />
      </div>
      <BaseEmpty v-if="error" :title="error" surface="none" class="mt-4">
        <template #action>
          <BaseButton @click="refresh">
            重试
          </BaseButton>
        </template>
      </BaseEmpty>
      <template v-else>
        <div class="mt-4 h-100 min-w-0">
          <UsageRecordsTable :columns="columns" :rows="rows" :loading="loading">
            <template #actions="{ row }">
              <BaseIconButton size="sm" label="查看使用记录详情" @click="detail = row; detailOpen = true">
                <Eye class="size-4" />
              </BaseIconButton>
            </template>
          </UsageRecordsTable>
        </div>
        <BaseTablePagination :pagination="pagination" :loading="loading" @page-change="pageChanged" @page-size-change="sizeChanged" />
      </template>
    </BaseCard>
    <BaseModal v-model="detailOpen" title="使用记录详情" tone="info" size="lg">
      <div v-if="detail" class="grid gap-5">
        <div class="flex flex-wrap items-center justify-between gap-4">
          <UsageModelCell :record="detail" />
          <span :class="detail.outcome === 'succeeded' ? 'text-cp-success-text' : 'text-cp-error-text'">{{ usageOutcomeText(detail.outcome) }}</span>
        </div>
        <div class="grid gap-4 sm:grid-cols-2">
          <BaseCard title="Token" padding="compact">
            <UsageTokenCell :record="detail" />
          </BaseCard>
          <BaseCard title="耗时" padding="compact">
            <UsageLatencyCell :record="detail" />
          </BaseCard>
        </div>
        <dl class="m-0 grid grid-cols-[auto_minmax(0,1fr)] gap-x-5 gap-y-3 text-cp-sm">
          <dt class="text-cp-text-secondary">
            请求 ID
          </dt><dd class="m-0 break-all font-mono">
            {{ detail.id }}
          </dd>
          <dt class="text-cp-text-secondary">
            API Key
          </dt><dd class="m-0">
            {{ detail.clientApiKeyName }}
          </dd>
          <dt class="text-cp-text-secondary">
            计费
          </dt><dd class="m-0 font-mono">
            {{ displayMoney(detail.downstreamBilledAmount) }}
          </dd>
          <dt class="text-cp-text-secondary">
            开始
          </dt><dd class="m-0">
            {{ detail.createdAtDisplay }}
          </dd>
          <dt class="text-cp-text-secondary">
            完成
          </dt><dd class="m-0">
            {{ displayDate(detail.original.completedAt) }}
          </dd>
          <template v-if="detail.imageCount !== null || detail.imageRequestedCount !== null">
            <dt class="text-cp-text-secondary">
              图片
            </dt><dd class="m-0">
              {{ detail.imageCount ?? detail.imageRequestedCount }} · {{ detail.imageOutputSize ?? detail.imageRequestedSize ?? '—' }}
            </dd>
          </template>
        </dl>
      </div>
      <template #footer>
        <BaseButton variant="secondary" @click="detailOpen = false">
          关闭
        </BaseButton>
      </template>
    </BaseModal>
  </div>
</template>
