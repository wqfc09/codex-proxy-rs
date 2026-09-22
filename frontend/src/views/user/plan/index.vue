<script setup lang="ts">
import type { UserBillingSummary, UserSubscription, UserUsageSummary } from '@/api'
import { CalendarDays, Package, RefreshCw } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, shallowRef, watch } from 'vue'
import { getUserBilling, getUserSubscriptionHistory, getUserUsageSummary } from '@/api'
import BaseButton from '@/components/base/BaseButton.vue'
import BaseCard from '@/components/base/BaseCard.vue'
import BaseEmpty from '@/components/base/BaseEmpty.vue'
import BasePageHeader from '@/components/base/BasePageHeader.vue'
import BaseSegmented from '@/components/base/BaseSegmented.vue'
import BaseSkeleton from '@/components/base/BaseSkeleton.vue'
import BaseTablePagination from '@/components/base/BaseTable/BaseTablePagination.vue'
import UsageTimeRangeSelect from '@/components/UsageTimeRangeSelect.vue'
import { useUsageTimeRange } from '@/composables/useUsageTimeRange'
import UsageTrendCard from '@/views/usage/components/UsageTrendCard.vue'
import OverviewBudgetCard from '../overview/components/OverviewBudgetCard.vue'
import { displayDate, displayMoney, subscriptionStatusText } from '../utils'

const loading = shallowRef(true)
const error = shallowRef('')
const billing = shallowRef<UserBillingSummary | null>(null)
const summary = shallowRef<UserUsageSummary | null>(null)
const now = shallowRef(Date.now())
const activityView = shallowRef<'usage' | 'history'>('usage')
const activityOptions = [
  { label: '套餐用量', value: 'usage' },
  { label: '订阅记录', value: 'history' },
]
const historyItems = shallowRef<UserSubscription[]>([])
const historyLoading = shallowRef(false)
const historyError = shallowRef('')
const historyPage = shallowRef(1)
const historyPageSize = shallowRef(5)
const historyTotal = shallowRef(0)
const historyPagination = computed(() => ({
  currentPage: historyPage.value,
  pageSize: historyPageSize.value,
  total: historyTotal.value,
  pageSizes: [5, 10, 20],
}))
const { timeRange, latestTimeRangeParams } = useUsageTimeRange()
let controller: AbortController | undefined
let historyController: AbortController | undefined

const period = computed(() => {
  const subscription = billing.value?.subscription
  if (!subscription)
    return null
  const start = Date.parse(subscription.startsAt)
  const end = Date.parse(subscription.expiresAt)
  if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start)
    return null
  return {
    percent: Math.max(0, Math.min(100, (now.value - start) / (end - start) * 100)),
    remaining: Math.max(0, Math.ceil((end - now.value) / 86400000)),
  }
})

const rangeText = computed(() =>
  summary.value ? `${displayDate(summary.value.startTime)} — ${displayDate(summary.value.endTime)}` : '',
)

async function loadHistory() {
  historyController?.abort()
  const request = new AbortController()
  historyController = request
  historyLoading.value = true
  historyError.value = ''
  try {
    const page = await getUserSubscriptionHistory(
      { page: historyPage.value, pageSize: historyPageSize.value },
      { signal: request.signal, silent: true },
    )
    if (request.signal.aborted)
      return
    historyItems.value = page.items
    historyPage.value = page.page
    historyPageSize.value = page.pageSize
    historyTotal.value = page.total
  }
  catch {
    if (!request.signal.aborted) {
      historyItems.value = []
      historyError.value = '订阅记录加载失败'
    }
  }
  finally {
    if (historyController === request)
      historyLoading.value = false
  }
}

async function load() {
  controller?.abort()
  const request = new AbortController()
  controller = request
  loading.value = true
  error.value = ''
  now.value = Date.now()
  try {
    const [next, usage] = await Promise.all([
      getUserBilling({ signal: request.signal, silent: true }),
      getUserUsageSummary(latestTimeRangeParams(), { signal: request.signal, silent: true }),
    ])
    if (request.signal.aborted)
      return
    billing.value = next
    summary.value = usage
  }
  catch {
    if (!request.signal.aborted)
      error.value = '套餐信息加载失败'
  }
  finally {
    if (controller === request)
      loading.value = false
  }
}

function refresh() {
  void load()
  if (activityView.value === 'history')
    void loadHistory()
}

function historyPageChanged(page: number) {
  historyPage.value = page
  void loadHistory()
}

function historyPageSizeChanged(size: number) {
  historyPageSize.value = size
  historyPage.value = 1
  void loadHistory()
}

function subscriptionRecordClass(item: UserSubscription) {
  return item.id === billing.value?.subscription?.id
    ? 'border border-cp-primary-border bg-cp-info-container'
    : 'border border-transparent bg-cp-fill-quaternary'
}

function subscriptionStatusClass(status: UserSubscription['effectiveStatus']) {
  if (status === 'active')
    return 'bg-cp-success-container text-cp-success-on-container'
  if (status === 'pending')
    return 'bg-cp-info-container text-cp-info-on-container'
  return 'bg-cp-fill-tertiary text-cp-text-secondary'
}

watch(timeRange, load)
watch(activityView, (value) => {
  if (value === 'history' && historyTotal.value === 0 && !historyLoading.value)
    void loadHistory()
})
onMounted(() => {
  void load()
})
onBeforeUnmount(() => {
  controller?.abort()
  historyController?.abort()
})
</script>

<template>
  <div class="user-plan flex h-[calc(100dvh-2rem)] w-full min-w-0 flex-none! flex-col min-[961px]:h-[calc(100dvh-3rem)]">
    <BasePageHeader title="我的套餐" class="shrink-0">
      <template #actions>
        <BaseButton variant="secondary" :loading="loading || historyLoading" @click="refresh">
          <template #icon>
            <RefreshCw class="size-4" />
          </template>
          刷新
        </BaseButton>
      </template>
    </BasePageHeader>

    <BaseEmpty v-if="error && !loading" :title="error" class="mt-4">
      <template #action>
        <BaseButton @click="load">
          重试
        </BaseButton>
      </template>
    </BaseEmpty>

    <div v-else-if="loading && !billing" class="mt-4 grid gap-4 lg:grid-cols-2">
      <BaseSkeleton v-for="index in 2" :key="index" class="h-88 rounded-cp-card" />
    </div>

    <template v-else-if="billing">
      <div class="mt-4 grid min-w-0 shrink-0 gap-4 lg:grid-cols-2 xl:grid-cols-[minmax(0,1.25fr)_minmax(19rem,1fr)]">
        <BaseCard title="当前套餐" padding="compact" class="min-w-0 border-l-4 border-l-cp-primary">
          <template #actions>
            <div class="grid justify-items-end gap-2">
              <span class="text-cp-sm font-emphasis text-cp-text-secondary">
                {{ billing.effectiveSource === 'basePlan' ? '基础套餐' : subscriptionStatusText(billing.subscription?.effectiveStatus) }}
              </span>
              <span class="inline-flex items-center rounded-cp bg-cp-info-container px-3 py-1.5 text-cp-xs text-cp-text-secondary">
                计费倍率
                <strong class="ml-2 font-mono text-cp font-heavy tabular-nums text-cp-text">{{ billing.multiplier }}×</strong>
              </span>
            </div>
          </template>

          <div v-if="billing.plan" class="grid gap-6">
            <div class="flex min-w-0 items-center gap-3">
              <span class="inline-flex size-11 shrink-0 items-center justify-center rounded-cp bg-cp-info-container text-cp-info-on-container">
                <Package class="size-6" />
              </span>
              <h2 class="m-0 min-w-0 break-words text-cp-xl font-heavy text-cp-text">
                {{ billing.plan.name }}
              </h2>
            </div>

            <div v-if="period && billing.subscription" class="grid gap-3">
              <div class="flex flex-wrap items-center justify-between gap-3 text-cp-sm">
                <span class="flex items-center gap-2 text-cp-text-secondary">
                  <CalendarDays class="size-4" />
                  有效期
                </span>
                <strong class="font-emphasis text-cp-text">
                  {{ billing.subscription.effectiveStatus === 'active' ? `剩余 ${period.remaining} 天` : subscriptionStatusText(billing.subscription.effectiveStatus) }}
                </strong>
              </div>
              <div
                role="progressbar"
                aria-label="套餐有效期"
                :aria-valuenow="period.percent"
                :aria-valuemin="0"
                :aria-valuemax="100"
                class="h-2 overflow-hidden rounded-full bg-cp-fill-tertiary"
              >
                <div class="h-full rounded-full bg-cp-primary" :style="{ width: `${period.percent}%` }" />
              </div>
              <div class="flex flex-wrap justify-between gap-2 text-cp-xs text-cp-text-tertiary">
                <span>{{ displayDate(billing.subscription.startsAt) }}</span>
                <span>{{ displayDate(billing.subscription.expiresAt) }}</span>
              </div>
            </div>

            <p v-if="billing.effectiveSource === 'basePlan'" class="m-0 text-cp-sm text-cp-text-secondary">
              当前未使用有效的显式订阅，按基础套餐额度计费；无独立到期日期。
            </p>
          </div>

          <BaseEmpty
            v-if="!billing.plan"
            title="套餐信息暂不可用"
            :icon="Package"
            size="sm"
            surface="none"
            class="min-h-48 content-center"
          />
        </BaseCard>

        <OverviewBudgetCard :billing="billing" :selected-key="null" compact />
      </div>
      <BaseCard
        class="mt-4 min-h-[24rem] min-w-0 flex-1"
        title="套餐活动"
        padding="compact"
      >
        <template #actions>
          <BaseSegmented v-model="activityView" label="套餐活动" :options="activityOptions" size="sm" />
        </template>

        <template v-if="activityView === 'usage'">
          <div class="flex min-w-0 flex-wrap items-center justify-between gap-3">
            <span class="text-cp-sm font-emphasis text-cp-text-secondary">
              查看当前套餐在所选自然日范围内的实际调用
            </span>
            <UsageTimeRangeSelect v-model="timeRange" />
          </div>

          <div class="mt-4 grid min-w-0 gap-2.5 sm:grid-cols-2 xl:grid-cols-4">
            <div class="rounded-cp bg-cp-fill-quaternary px-3.5 py-3">
              <span class="block text-cp-xs font-emphasis text-cp-text-tertiary">请求</span>
              <strong class="mt-1.5 block font-mono text-cp-lg font-heavy tabular-nums text-cp-text">
                {{ summary?.requestCount ?? '—' }}
              </strong>
            </div>
            <div class="rounded-cp bg-cp-fill-quaternary px-3.5 py-3">
              <span class="block text-cp-xs font-emphasis text-cp-text-tertiary">总 Token</span>
              <strong class="mt-1.5 block font-mono text-cp-lg font-heavy tabular-nums text-cp-text">
                {{ summary?.totalTokens ?? '—' }}
              </strong>
            </div>
            <div class="rounded-cp bg-cp-fill-quaternary px-3.5 py-3">
              <span class="block text-cp-xs font-emphasis text-cp-text-tertiary">已知计费</span>
              <strong class="mt-1.5 block break-all font-mono text-cp-lg font-heavy tabular-nums text-cp-text">
                {{ summary?.requestCount === 0 ? '$0' : displayMoney(summary?.billedUsd) }}
              </strong>
              <span v-if="summary?.billedUnknownCount" class="mt-1 block text-cp-xs text-cp-text-tertiary">
                {{ summary.billedUnknownCount }} 条未知
              </span>
            </div>
            <div class="rounded-cp bg-cp-fill-quaternary px-3.5 py-3">
              <span class="block text-cp-xs font-emphasis text-cp-text-tertiary">统计范围</span>
              <strong class="mt-1.5 block text-cp-sm font-emphasis leading-relaxed text-cp-text">
                {{ rangeText || '—' }}
              </strong>
            </div>
          </div>

          <div class="mt-4 min-h-0 min-w-0 flex-1">
            <UsageTrendCard class="h-full" embedded :summary="summary" :loading="loading" title="套餐用量" />
          </div>
        </template>

        <template v-else>
          <div class="flex min-w-0 flex-wrap items-center gap-3">
            <span class="text-cp-sm font-emphasis text-cp-text-secondary">
              套餐授予、倍率与有效期记录
            </span>
          </div>

          <div class="min-h-0 flex-1 overflow-y-auto pr-1">
            <BaseEmpty
              v-if="historyError && !historyLoading"
              :title="historyError"
              surface="none"
              class="mt-4 min-h-40 content-center"
            >
              <template #action>
                <BaseButton size="sm" @click="loadHistory">
                  重试
                </BaseButton>
              </template>
            </BaseEmpty>

            <div v-else-if="historyLoading && !historyItems.length" class="mt-4 grid gap-2.5">
              <BaseSkeleton v-for="index in historyPageSize" :key="index" class="h-22 rounded-cp" />
            </div>

            <div v-else-if="historyItems.length" class="mt-4 grid min-w-0 gap-2.5">
              <article
                v-for="item in historyItems"
                :key="item.id"
                class="grid min-w-0 gap-4 rounded-cp px-4 py-3.5 md:grid-cols-[minmax(11rem,1.2fr)_minmax(19rem,2fr)_auto] md:items-center"
                :class="subscriptionRecordClass(item)"
              >
                <div class="min-w-0">
                  <div class="flex min-w-0 items-center gap-2">
                    <strong class="min-w-0 truncate text-cp font-heavy text-cp-text" :title="item.planName">
                      {{ item.planName }}
                    </strong>
                    <span
                      class="shrink-0 rounded-full px-2 py-1 text-cp-xs font-emphasis"
                      :class="subscriptionStatusClass(item.effectiveStatus)"
                    >
                      {{ subscriptionStatusText(item.effectiveStatus) }}
                    </span>
                  </div>
                  <span class="mt-1 block text-cp-xs text-cp-text-tertiary">
                    {{ item.id === billing.subscription?.id ? '当前订阅' : '历史订阅' }} · 最近修改 {{ displayDate(item.updatedAt) }}
                  </span>
                </div>

                <div class="min-w-0">
                  <div class="flex flex-wrap items-center gap-x-3 gap-y-1 text-cp-sm text-cp-text-secondary">
                    <span>{{ displayDate(item.startsAt) }}</span>
                    <span class="text-cp-text-quaternary">→</span>
                    <span>{{ displayDate(item.expiresAt) }}</span>
                  </div>
                  <span v-if="item.revokedAt" class="mt-1 block text-cp-xs text-cp-text-tertiary">
                    撤销于 {{ displayDate(item.revokedAt) }}
                  </span>
                </div>

                <div class="md:text-right">
                  <span class="block text-cp-xs text-cp-text-tertiary">计费倍率</span>
                  <strong class="mt-1 block font-mono text-cp font-heavy tabular-nums text-cp-text">
                    {{ item.multiplier }}×
                  </strong>
                </div>
              </article>
            </div>

            <BaseEmpty
              v-else
              title="暂无订阅记录"
              surface="none"
              size="sm"
              class="mt-4 min-h-40 content-center"
            />
          </div>

          <div class="mt-auto shrink-0">
            <BaseTablePagination
              :pagination="historyPagination"
              :loading="historyLoading"
              @page-change="historyPageChanged"
              @page-size-change="historyPageSizeChanged"
            />
          </div>
        </template>
      </BaseCard>
    </template>
  </div>
</template>

<style scoped>
.user-plan {
  container-type: inline-size;
}
</style>
