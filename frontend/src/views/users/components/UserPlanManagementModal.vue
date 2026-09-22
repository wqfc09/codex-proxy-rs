<script setup lang="ts">
import type { AdminBillingSummary, SubscriptionPlan, UserManagementSummary } from '@/api'
import { CalendarClock, Package, Percent, RotateCcw } from '@lucide/vue'
import { computed, reactive, shallowRef, watch } from 'vue'

import {
  getAdminUserBilling,
  grantAdminUserSubscriptionWithOptions,
  patchAdminUserSubscription,
  resetAdminUserBudget,
  revokeAdminUserSubscription,
} from '@/api'
import BaseButton from '@/components/base/BaseButton.vue'
import BaseCheckbox from '@/components/base/BaseCheckbox.vue'
import BaseEmpty from '@/components/base/BaseEmpty.vue'
import BaseFormItem from '@/components/base/BaseForm/FormItem.vue'
import BaseInput from '@/components/base/BaseInput.vue'
import BaseModal from '@/components/base/BaseModal/index.vue'
import BaseSegmented from '@/components/base/BaseSegmented.vue'
import BaseSelect from '@/components/base/BaseSelect.vue'
import { toast } from '@/components/base/BaseToast'
import { useRequestState } from '@/composables/useRequestState'

type Section = 'plan' | 'dates' | 'multiplier' | 'reset'
type DateMode = 'calendar' | 'offset'

const props = defineProps<{
  user: UserManagementSummary | null
  plans: SubscriptionPlan[]
}>()
const emit = defineEmits<{ updated: [] }>()
const open = defineModel<boolean>({ required: true })

const section = shallowRef<Section>('plan')
const billingRequest = useRequestState()
const { loading, error: loadError } = billingRequest
const loadedUserId = shallowRef<string | null>(null)
const saving = shallowRef(false)
let contextRevision = 0
const billing = shallowRef<AdminBillingSummary | null>(null)
const planId = shallowRef('')
const grantDays = shallowRef('30')
const startsAt = shallowRef('')
const expiresAt = shallowRef('')
const dateMode = shallowRef<DateMode>('calendar')
const dateOffsetDays = shallowRef('+30')
const multiplier = shallowRef('1')
const resetWindows = reactive({ daily: true, weekly: true, monthly: true })

const hasSubscription = computed(() => Boolean(billing.value?.subscription))
const sectionOptions = computed(() => [
  { label: '修改套餐', value: 'plan', icon: Package },
  { label: '套餐日期', value: 'dates', icon: CalendarClock, disabled: !hasSubscription.value },
  { label: '套餐倍率', value: 'multiplier', icon: Percent, disabled: !hasSubscription.value },
  { label: '重置额度', value: 'reset', icon: RotateCcw },
])
const planOptions = computed(() =>
  props.plans
    .filter(plan => plan.enabled)
    .map(plan => ({ label: plan.name, value: plan.id })),
)
const dateModeOptions = [
  { label: '日历调整', value: 'calendar' },
  { label: '增减天数', value: 'offset' },
]
const selectedPlan = computed(() => props.plans.find(plan => plan.id === planId.value) ?? null)
const offsetExpiry = computed(() => {
  const current = billing.value?.subscription
  const days = Number(dateOffsetDays.value)
  if (!current || !Number.isInteger(days) || days === 0)
    return ''
  const next = new Date(Date.parse(current.expiresAt) + days * 86400000)
  return Number.isNaN(next.getTime()) ? '' : toLocalInput(next.toISOString()).replace('T', ' ')
})

function toLocalInput(value: string | undefined) {
  if (!value)
    return ''
  const date = new Date(value)
  if (Number.isNaN(date.getTime()))
    return ''
  const pad = (part: number) => String(part).padStart(2, '0')
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}T${pad(date.getHours())}:${pad(date.getMinutes())}`
}

function sync(summary: AdminBillingSummary) {
  billing.value = summary
  planId.value = summary.plan?.id ?? props.user?.effectivePlan.id ?? ''
  startsAt.value = toLocalInput(summary.subscription?.startsAt) || toLocalInput(new Date().toISOString())
  expiresAt.value = toLocalInput(summary.subscription?.expiresAt)
  multiplier.value = summary.subscription?.multiplier ?? summary.multiplier ?? '1'
  if (!summary.subscription && (section.value === 'dates' || section.value === 'multiplier'))
    section.value = 'plan'
}

async function load() {
  const userId = props.user?.userId
  if (!open.value || !userId)
    return
  const requestId = billingRequest.start()
  loadedUserId.value = null
  billing.value = null
  try {
    const summary = await getAdminUserBilling(userId, { silent: true, signal: billingRequest.signal })
    if (!billingRequest.isCurrent(requestId) || !open.value || props.user?.userId !== userId)
      return
    sync(summary)
    loadedUserId.value = userId
  }
  catch (error) {
    billingRequest.fail(requestId, error)
  }
  finally {
    billingRequest.finish(requestId)
  }
}

async function savePlan() {
  if (!props.user || !billing.value || !planId.value)
    return
  const target = props.plans.find(plan => plan.id === planId.value)
  if (!target)
    return

  const current = billing.value.subscription
  if (target.isBase) {
    if (current)
      await revokeAdminUserSubscription(props.user.userId)
    return
  }

  const reusable = current && ['active', 'pending'].includes(current.effectiveStatus)
  if (reusable && current.planId === target.id)
    return

  await grantAdminUserSubscriptionWithOptions(props.user.userId, {
    planId: target.id,
    durationDays: Number(grantDays.value),
    multiplier: reusable ? current.multiplier ?? billing.value.multiplier ?? '1' : '1',
  })
}

async function saveDates() {
  const subscription = billing.value?.subscription
  if (!props.user || !subscription)
    return
  if (dateMode.value === 'calendar') {
    if (!startsAt.value || !expiresAt.value)
      return
    await patchAdminUserSubscription(props.user.userId, {
      startsAt: new Date(startsAt.value).toISOString(),
      expiresAt: new Date(expiresAt.value).toISOString(),
    })
    return
  }
  const days = Number(dateOffsetDays.value)
  const adjustedExpiry = new Date(Date.parse(subscription.expiresAt) + days * 86400000)
  await patchAdminUserSubscription(props.user.userId, { expiresAt: adjustedExpiry.toISOString() })
}

async function saveMultiplier() {
  if (!props.user || !billing.value?.subscription || !multiplier.value)
    return
  await patchAdminUserSubscription(props.user.userId, { multiplier: multiplier.value })
}

async function saveReset() {
  if (!props.user || (!resetWindows.daily && !resetWindows.weekly && !resetWindows.monthly))
    return
  await resetAdminUserBudget(props.user.userId, { ...resetWindows })
}

const saveDisabled = computed(() => {
  if (loading.value || saving.value || !billing.value || loadedUserId.value !== props.user?.userId)
    return true
  if (section.value === 'plan') {
    const target = props.plans.find(plan => plan.id === planId.value && plan.enabled)
    if (!target)
      return true
    const current = billing.value.subscription
    if (target.isBase)
      return !current
    const days = Number(grantDays.value)
    if (!Number.isInteger(days) || days <= 0 || days > 36500)
      return true
    return current?.planId === target.id && ['active', 'pending'].includes(current.effectiveStatus)
  }
  if (section.value === 'dates') {
    if (!hasSubscription.value)
      return true
    if (dateMode.value === 'offset') {
      const days = Number(dateOffsetDays.value)
      const start = Date.parse(billing.value.subscription!.startsAt)
      const end = Date.parse(billing.value.subscription!.expiresAt) + days * 86400000
      return !Number.isInteger(days) || days === 0 || !Number.isFinite(end) || end <= start
    }
    const start = new Date(startsAt.value).getTime()
    const end = new Date(expiresAt.value).getTime()
    return !Number.isFinite(start) || !Number.isFinite(end) || end <= start
  }
  if (section.value === 'multiplier')
    return !hasSubscription.value || !/^\d{1,10}(?:\.\d{1,10})?$/.test(multiplier.value)
  return !resetWindows.daily && !resetWindows.weekly && !resetWindows.monthly
})

async function save() {
  const userId = props.user?.userId
  if (!userId || saveDisabled.value)
    return
  const context = contextRevision
  const savedSection = section.value
  saving.value = true
  try {
    if (savedSection === 'plan')
      await savePlan()
    else if (savedSection === 'dates')
      await saveDates()
    else if (savedSection === 'multiplier')
      await saveMultiplier()
    else
      await saveReset()

    emit('updated')
    if (context !== contextRevision || !open.value || props.user?.userId !== userId)
      return
    const messages = { plan: '套餐已更新', dates: '套餐日期已更新', multiplier: '套餐倍率已更新', reset: '额度已重置' }
    toast.success(messages[savedSection])
    await load()
  }
  catch {}
  finally {
    saving.value = false
  }
}

watch([open, () => props.user?.userId], ([isOpen]) => {
  contextRevision += 1
  billingRequest.invalidate()
  loadedUserId.value = null
  billing.value = null
  planId.value = ''
  grantDays.value = '30'
  startsAt.value = ''
  expiresAt.value = ''
  dateMode.value = 'calendar'
  dateOffsetDays.value = '+30'
  multiplier.value = '1'
  section.value = 'plan'
  Object.assign(resetWindows, { daily: true, weekly: true, monthly: true })
  if (!isOpen)
    return
  void load()
}, { immediate: true, flush: 'sync' })
</script>

<template>
  <BaseModal
    v-model="open"
    title="套餐管理"
    :description="user?.username"
    tone="info"
    size="md"
    :dismissible="!saving"
  >
    <div class="grid gap-5">
      <BaseSegmented
        v-model="section"
        class="w-full"
        label="套餐管理功能"
        size="sm"
        :disabled="loading || saving"
        :options="sectionOptions"
      />

      <BaseEmpty v-if="loadError" title="套餐管理信息加载失败" description="重新加载后才能修改套餐。" size="sm">
        <template #action>
          <BaseButton variant="secondary" :disabled="saving" @click="load">
            重新加载
          </BaseButton>
        </template>
      </BaseEmpty>

      <div v-else-if="section === 'plan'" class="grid min-w-0 gap-5">
        <BaseFormItem label="套餐" required>
          <BaseSelect
            v-model="planId"
            class="w-full"
            :options="planOptions"
            :disabled="loading || saving"
          />
        </BaseFormItem>
        <BaseFormItem v-if="selectedPlan && !selectedPlan.isBase" label="授予天数" required>
          <BaseInput
            v-model="grantDays"
            type="number"
            min="1"
            max="36500"
            step="1"
            inputmode="numeric"
            placeholder="30"
            :disabled="loading || saving"
          />
          <p class="mt-2 mb-0 text-cp-xs text-cp-text-tertiary">
            从保存时刻开始计算。输入 30 表示授予 30 天，365 表示授予 365 天。
          </p>
        </BaseFormItem>
      </div>

      <div v-else-if="section === 'dates'" class="grid min-w-0 gap-4">
        <BaseSegmented v-model="dateMode" class="w-full" label="日期调整方式" size="sm" :options="dateModeOptions" :disabled="loading || saving" />
        <div v-if="dateMode === 'calendar'" class="grid gap-4 sm:grid-cols-2">
          <BaseFormItem label="开始时间" required>
            <BaseInput v-model="startsAt" type="datetime-local" :disabled="loading || saving" />
          </BaseFormItem>
          <BaseFormItem label="到期时间" required>
            <BaseInput v-model="expiresAt" type="datetime-local" :disabled="loading || saving" />
          </BaseFormItem>
        </div>
        <BaseFormItem v-else label="增减天数" required>
          <BaseInput
            v-model="dateOffsetDays"
            type="text"
            inputmode="numeric"
            placeholder="+30 / -30"
            :disabled="loading || saving"
          />
          <p class="mt-2 mb-0 text-cp-xs text-cp-text-tertiary">
            以当前到期时间为基准；+30 延后 30 天，-30 提前 30 天。
            <span v-if="offsetExpiry">调整后：{{ offsetExpiry }}</span>
          </p>
        </BaseFormItem>
      </div>

      <BaseFormItem v-else-if="section === 'multiplier'" label="倍率" required>
        <BaseInput
          v-model="multiplier"
          type="number"
          min="0"
          step="0.01"
          inputmode="decimal"
          :disabled="loading || saving"
        />
      </BaseFormItem>

      <div v-else class="grid gap-3 rounded-cp bg-cp-fill-quaternary p-4">
        <BaseCheckbox v-model="resetWindows.daily" label="日额度" show-label :disabled="saving" />
        <BaseCheckbox v-model="resetWindows.weekly" label="周额度" show-label :disabled="saving" />
        <BaseCheckbox v-model="resetWindows.monthly" label="月额度" show-label :disabled="saving" />
      </div>
    </div>

    <template #footer>
      <BaseButton variant="secondary" :disabled="saving" @click="open = false">
        关闭
      </BaseButton>
      <BaseButton
        :loading="saving"
        :disabled="saveDisabled"
        @click="save"
      >
        {{ section === 'reset' ? '重置额度' : '保存' }}
      </BaseButton>
    </template>
  </BaseModal>
</template>
