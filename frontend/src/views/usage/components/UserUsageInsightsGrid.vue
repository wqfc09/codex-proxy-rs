<script setup lang="ts">
import type { UserUsageSummary } from '@/api'
import { computed } from 'vue'
import BaseCard from '@/components/base/BaseCard.vue'
import BaseEmpty from '@/components/base/BaseEmpty.vue'
import BaseSkeleton from '@/components/base/BaseSkeleton.vue'
import UsageDistributionCard from '@/components/charts/UsageDistributionCard.vue'
import UsageTrendCard from './UsageTrendCard.vue'

const props = defineProps<{ summary: UserUsageSummary | null, loading: boolean }>()
const successRate = computed(() => props.summary?.requestCount ? props.summary.successCount / props.summary.requestCount * 100 : null)
</script>

<template>
  <section class="mt-4 grid min-w-0 gap-4 xl:grid-cols-2" aria-label="个人用量分析">
    <UsageTrendCard :summary="summary" :loading="loading" />
    <BaseCard title="请求健康" padding="compact" class="min-h-[22rem] min-w-0">
      <BaseSkeleton v-if="loading" class="h-70" />
      <div v-else-if="successRate !== null && summary" class="grid min-h-70 content-center gap-6">
        <div class="flex items-end justify-between gap-3">
          <span class="text-cp-sm text-cp-text-secondary">成功率</span>
          <strong class="font-mono text-cp-xl font-heavy text-cp-success-text">{{ successRate.toFixed(1) }}%</strong>
        </div>
        <div role="img" :aria-label="`成功 ${summary.successCount}，未成功 ${summary.failureCount}`" class="flex h-3 overflow-hidden rounded-full bg-cp-error-container">
          <span class="h-full bg-cp-success" :style="{ width: `${successRate}%` }" />
        </div>
        <dl class="m-0 flex justify-between gap-4 text-cp-sm">
          <div>
            <dt class="text-cp-text-secondary">
              成功
            </dt><dd class="m-0 mt-1 font-mono font-heavy text-cp-success-text">
              {{ summary.successCount }}
            </dd>
          </div>
          <div class="text-right">
            <dt class="text-cp-text-secondary">
              未成功
            </dt><dd class="m-0 mt-1 font-mono font-heavy text-cp-error-text">
              {{ summary.failureCount }}
            </dd>
          </div>
        </dl>
      </div>
      <BaseEmpty v-else title="当前范围暂无请求" surface="none" size="sm" class="min-h-70 content-center" />
    </BaseCard>
    <template v-if="loading">
      <BaseSkeleton v-for="index in 2" :key="index" class="h-80 rounded-cp-card" />
    </template>
    <template v-else>
      <UsageDistributionCard title="模型分布" :items="summary?.models ?? []" />
      <UsageDistributionCard title="密钥分布" :items="summary?.clientKeys ?? []" />
    </template>
  </section>
</template>
