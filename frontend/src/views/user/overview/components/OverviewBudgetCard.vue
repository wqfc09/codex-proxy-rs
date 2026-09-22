<script setup lang="ts">
import type { UserBillingSummary, UserClientKey } from '@/api'
import { computed, shallowRef, watch } from 'vue'
import BaseCard from '@/components/base/BaseCard.vue'
import BaseSegmented from '@/components/base/BaseSegmented.vue'
import { displayMoney } from '../../utils'

const props = withDefaults(defineProps<{ billing: UserBillingSummary, selectedKey: UserClientKey | null, compact?: boolean }>(), { compact: false })
const scope = shallowRef('account')
const options = [{ label: '账户', value: 'account' }, { label: '密钥', value: 'key' }]
watch(() => props.selectedKey?.id, () => {
  scope.value = 'account'
})
const keyScope = computed(() => scope.value === 'key' && props.selectedKey !== null)
const entries = computed(() => {
  const budget = props.billing.budget
  const key = keyScope.value ? props.selectedKey : null
  const windows = key
    ? [
        { label: 'Key 日限额', used: key.dailyUsedUsd, limit: keyLimit(key.dailyLimitUsd) },
        { label: 'Key 7 日限额', used: key.weeklyUsedUsd, limit: keyLimit(key.weeklyLimitUsd) },
      ]
    : [
        { label: '日额度', used: budget.daily.used, limit: budget.daily.effectiveLimit },
        { label: '自然周额度', used: budget.weekly.used, limit: budget.weekly.effectiveLimit },
        { label: '月额度', used: budget.monthly.used, limit: budget.monthly.effectiveLimit },
      ]
  return windows.map((entry) => {
    const limit = entry.limit === null ? null : Number(entry.limit)
    const used = Number(entry.used)
    const state = limit === null ? 'unlimited' : !Number.isFinite(limit) || !Number.isFinite(used) ? 'unknown' : limit === 0 ? 'zero' : 'finite'
    const percent = state === 'finite' && limit !== null ? Math.max(0, Math.min(100, used / limit * 100)) : null
    return { ...entry, state, percent, remaining: percent === null ? null : Math.max(0, 100 - percent) }
  })
})
function keyLimit(value: string) {
  // Key 0=不限；账户额度0=不可用，不混用两种语义。
  return Number(value) === 0 ? null : value
}
</script>

<template>
  <BaseCard :title="keyScope ? '密钥自限额' : '账户额度'" padding="compact" class="min-w-0" :class="compact ? '' : 'min-h-[22rem]'">
    <template v-if="selectedKey" #actions>
      <BaseSegmented v-model="scope" label="额度范围" :options="options" size="sm" />
    </template>
    <dl class="m-0 grid" :class="compact ? 'gap-2.5' : 'gap-[clamp(.75rem,1.8dvh,1.25rem)]'">
      <div v-for="entry in entries" :key="entry.label" class="min-w-0 rounded-cp bg-cp-fill-quaternary px-3.5" :class="compact ? 'py-2.5' : 'py-3'">
        <div class="flex items-center justify-between gap-3">
          <dt class="text-cp-sm font-emphasis text-cp-text-secondary">
            {{ entry.label }}
          </dt>
          <dd class="m-0 text-cp-xs" :class="entry.state === 'zero' ? 'text-cp-warning-text' : 'text-cp-text-tertiary'">
            {{ entry.state === 'unlimited' ? '不限' : entry.state === 'zero' ? '额度未开放' : entry.state === 'unknown' ? '暂不可用' : `剩余 ${entry.remaining?.toFixed(0)}%` }}
          </dd>
        </div>
        <div class="mt-2 flex flex-wrap items-baseline gap-1.5 font-mono tabular-nums">
          <strong class="text-cp-lg font-heavy text-cp-text">{{ displayMoney(entry.used) }}</strong>
          <span class="text-cp-xs text-cp-text-tertiary">{{ entry.state === 'unlimited' ? '已用 · 不限额' : `/ ${displayMoney(entry.limit)}` }}</span>
        </div>
        <div v-if="entry.percent !== null" role="progressbar" :aria-label="`${entry.label}已使用比例`" :aria-valuenow="entry.percent" :aria-valuemin="0" :aria-valuemax="100" class="mt-2.5 h-1.5 overflow-hidden rounded-full bg-cp-fill-tertiary">
          <div class="h-full rounded-full" :class="entry.percent >= 90 ? 'bg-cp-warning' : 'bg-cp-primary'" :style="{ width: `${entry.percent}%` }" />
        </div>
      </div>
    </dl>
    <p v-if="keyScope" class="mt-3 mb-0 text-cp-xs leading-relaxed text-cp-text-tertiary">
      仅限制这把密钥的使用，仍受账户总额度与限流约束。
    </p>
  </BaseCard>
</template>
