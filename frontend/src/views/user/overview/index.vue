<script setup lang="ts">
import type { UserUsageRecord } from '@/api'
import { Activity, ChartNoAxesColumn, KeyRound, Package, RefreshCw, Wallet } from '@lucide/vue'
import { computed } from 'vue'
import { useRouter } from 'vue-router'
import BaseButton from '@/components/base/BaseButton.vue'
import BaseCard from '@/components/base/BaseCard.vue'
import BaseEmpty from '@/components/base/BaseEmpty.vue'
import BasePageHeader from '@/components/base/BasePageHeader.vue'
import BaseSelect from '@/components/base/BaseSelect.vue'
import BaseSkeleton from '@/components/base/BaseSkeleton.vue'
import { defineTableColumns } from '@/components/base/BaseTable/columns'
import BaseTable from '@/components/base/BaseTable/index.vue'
import UsageDistributionCard from '@/components/charts/UsageDistributionCard.vue'
import UsageTimeRangeSelect from '@/components/UsageTimeRangeSelect.vue'
import UsageTrendCard from '@/views/usage/components/UsageTrendCard.vue'
import { displayDate, displayLimit, displayMoney, subscriptionStatusText, usageOutcomeText } from '../utils'
import OverviewBudgetCard from './components/OverviewBudgetCard.vue'
import OverviewMetric from './components/OverviewMetric.vue'
import { useUserOverview } from './composables/useUserOverview'

const router = useRouter()
const { timeRange, timeRangeLabel, billing, usage, keys, recent, selectedKeyId, selectedKey, keyOptions, loading, catalogLoading, scopeLoading, error, load } = useUserOverview()
const count = new Intl.NumberFormat('en', { notation: 'compact', maximumFractionDigits: 1 })
const visibleKeys = computed(() => selectedKey.value ? [selectedKey.value] : keys.value.slice(0, 4))
const successRate = computed(() => usage.value?.requestCount ? `${(usage.value.successCount / usage.value.requestCount * 100).toFixed(1)}%` : '—')
const billed = computed(() => usage.value?.requestCount === 0 ? '$0' : displayMoney(usage.value?.billedUsd))
const columns = defineTableColumns<UserUsageRecord>([
  { key: 'startedAt', label: '时间', kind: 'datetime', format: value => displayDate(String(value ?? '')) },
  { key: 'clientApiKeyName', label: 'API Key', kind: 'identity' },
  { key: 'requestedModel', label: '模型', kind: 'mono', emptyText: '未记录' },
  { key: 'totalTokens', label: 'Token', kind: 'numeric', emptyText: '—' },
  { key: 'billedAmountUsd', label: '费用', kind: 'numeric', format: value => displayMoney(value as string | null) },
  { key: 'outcome', label: '结果', kind: 'status', format: value => usageOutcomeText(String(value ?? '')) },
])

function openUsage() {
  void router.push({ path: '/user/usage', query: selectedKeyId.value ? { clientApiKeyId: selectedKeyId.value } : {} })
}
</script>

<template>
  <div class="user-overview w-full min-w-0">
    <BasePageHeader title="个人概览">
      <template #actions>
        <UsageTimeRangeSelect v-model="timeRange" />
        <BaseSelect v-model="selectedKeyId" :options="keyOptions" :disabled="catalogLoading" class="w-52 max-w-full flex-1 sm:flex-none" aria-label="API Key 范围" />
        <BaseButton variant="secondary" :loading="loading" @click="load">
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
    <template v-else>
      <BaseCard v-if="billing" padding="compact" class="overview-account mt-4 min-w-0 border-l-4 border-l-cp-primary">
        <div class="flex flex-wrap items-center justify-between gap-4">
          <div class="flex min-w-0 items-center gap-3.5">
            <span class="inline-flex size-12 shrink-0 items-center justify-center rounded-cp bg-cp-info-container text-cp-info-on-container"><Package class="size-6" /></span>
            <div class="min-w-0">
              <p class="m-0 text-cp-xs text-cp-text-tertiary">
                当前套餐 · {{ billing.effectiveSource === 'basePlan' ? '基础套餐回退' : subscriptionStatusText(billing.subscription?.effectiveStatus) }}
              </p>
              <h2 class="mt-1 mb-0 break-words text-cp-xl font-heavy text-cp-text">
                {{ billing.plan?.name ?? '暂不可用' }}
              </h2>
            </div>
          </div>
          <div class="flex flex-wrap items-center gap-x-6 gap-y-3">
            <div><span class="block text-cp-xs text-cp-text-tertiary">计费倍率</span><strong class="mt-1 block font-mono text-cp-lg tabular-nums text-cp-text">{{ billing.multiplier }}×</strong></div>
            <div><span class="block text-cp-xs text-cp-text-tertiary">账户并发 / RPM</span><strong class="mt-1 block font-mono text-cp-lg tabular-nums text-cp-text">{{ displayLimit(billing.effectiveMaxConcurrency) }} / {{ displayLimit(billing.effectiveRequestsPerMinute) }}</strong></div>
            <BaseButton variant="secondary" size="sm" @click="router.push('/user/plan')">
              套餐详情
            </BaseButton>
          </div>
        </div>
        <p v-if="billing.effectiveMaxConcurrency === 0 || billing.effectiveRequestsPerMinute === 0" class="mt-3 mb-0 text-cp-xs text-cp-warning-text">
          账户限流为 0，当前未开放调用。请联系管理员调整账户限制。
        </p>
      </BaseCard>
      <div class="overview-metrics mt-4" :aria-busy="loading">
        <template v-if="catalogLoading || scopeLoading">
          <BaseSkeleton v-for="index in 3" :key="index" class="h-30 rounded-cp-card" />
        </template>
        <template v-else>
          <OverviewMetric :label="`${timeRangeLabel}请求`" :value="count.format(usage?.requestCount ?? 0)" :caption="`${count.format(usage?.totalTokens ?? 0)} Token`" :icon="ChartNoAxesColumn" tone="info" />
          <OverviewMetric label="已知计费" :value="billed" :caption="usage?.billedUnknownCount ? `${usage.billedUnknownCount} 条计费未知` : timeRangeLabel" :icon="Wallet" />
          <OverviewMetric label="成功率" :value="successRate" :caption="`成功 ${count.format(usage?.successCount ?? 0)} · 未成功 ${count.format(usage?.failureCount ?? 0)}`" :icon="Activity" tone="success" />
        </template>
      </div>

      <div class="overview-primary mt-4">
        <UsageTrendCard :summary="usage" :loading="loading" :title="`${timeRangeLabel}趋势`" />
        <OverviewBudgetCard v-if="billing" :billing="billing" :selected-key="selectedKey" />
        <BaseSkeleton v-else class="h-64 rounded-cp-card" />
      </div>

      <div class="overview-distributions mt-4" :aria-busy="loading">
        <template v-if="loading">
          <BaseSkeleton v-for="index in 2" :key="index" class="h-52 rounded-cp-card" />
        </template>
        <template v-else>
          <UsageDistributionCard title="模型分布" :items="usage?.models ?? []" />
          <UsageDistributionCard title="密钥分布" :items="usage?.clientKeys ?? []" />
        </template>
      </div>

      <div class="overview-bottom mt-4">
        <BaseCard title="最近使用" padding="compact" class="min-h-[20rem] min-w-0">
          <template #actions>
            <BaseButton variant="secondary" size="sm" @click="openUsage">
              查看全部
            </BaseButton>
          </template>
          <div v-if="recent.length || loading" class="h-70 min-w-0">
            <BaseTable :columns="columns" :rows="recent" :loading="loading" density="compact">
              <template #outcome="{ row }">
                <span :class="row.outcome === 'succeeded' ? 'text-cp-success-text' : 'text-cp-error-text'">{{ usageOutcomeText(row.outcome) }}</span>
              </template>
            </BaseTable>
          </div>
          <BaseEmpty v-else surface="none" size="sm" title="暂无使用记录" class="min-h-60 content-center" />
        </BaseCard>
        <BaseCard title="API 密钥" padding="compact" class="min-h-[20rem] min-w-0">
          <template #actions>
            <BaseButton variant="secondary" size="sm" @click="router.push('/user/api-keys')">
              管理密钥
            </BaseButton>
          </template>
          <div v-if="visibleKeys.length" class="grid gap-2">
            <div v-for="key in visibleKeys" :key="key.id" class="flex min-w-0 items-center gap-3 rounded-cp bg-cp-fill-quaternary p-3">
              <KeyRound class="size-4 shrink-0 text-cp-text-tertiary" />
              <div class="min-w-0 flex-1">
                <strong class="block truncate text-cp-sm font-emphasis text-cp-text" :title="key.name">{{ key.name }}</strong>
                <span class="mt-1 block truncate font-mono text-cp-xs text-cp-text-tertiary">{{ key.prefix }}</span>
              </div>
              <span class="shrink-0 text-cp-xs" :class="key.enabled ? 'text-cp-success-text' : 'text-cp-text-tertiary'">{{ key.enabled ? '启用' : '停用' }}</span>
            </div>
          </div>
          <BaseSkeleton v-else-if="loading" class="h-60" />
          <BaseEmpty v-else title="暂无 API 密钥" :icon="KeyRound" surface="none" size="sm" class="min-h-60 content-center">
            <template #action>
              <BaseButton size="sm" @click="router.push('/user/api-keys?create=1')">
                创建密钥
              </BaseButton>
            </template>
          </BaseEmpty>
        </BaseCard>
      </div>
    </template>
  </div>
</template>

<style scoped>
.user-overview {
  container-type: inline-size;
}
.overview-metrics,
.overview-primary,
.overview-distributions,
.overview-bottom {
  display: grid;
  grid-template-columns: minmax(0, 1fr);
  gap: clamp(0.75rem, 1.6dvh, 1.25rem);
}
@container (min-width: 21rem) {
  .overview-metrics {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
@container (min-width: 40rem) {
  .overview-metrics {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }
}
@container (min-width: 46rem) {
  .overview-distributions {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
@container (min-width: 52rem) {
  .overview-primary {
    grid-template-columns: minmax(0, 1.8fr) minmax(17rem, 1fr);
  }
}
@container (min-width: 64rem) {
  .overview-metrics {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }
  .overview-bottom {
    grid-template-columns: minmax(0, 1.8fr) minmax(17rem, 1fr);
  }
}
</style>
