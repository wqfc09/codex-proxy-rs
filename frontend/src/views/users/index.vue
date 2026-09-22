<script setup lang="ts">
import type { AccountGroupRef, SubscriptionPlan, UserManagementSummary } from '@/api'
import { Copy, Pencil, Plus, Power, RefreshCw, Shuffle } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, reactive, shallowRef, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'

import {
  createAdminUser,
  createSubscriptionPlan,
  deleteAdminUser,
  getAdminUsersManagement,
  getSubscriptionPlans,
  setSubscriptionPlanEnabled,
  updateAdminUser,
  updateSubscriptionPlan,
} from '@/api'
import AccountGroupMarks from '@/components/AccountGroupMarks.vue'
import BaseButton from '@/components/base/BaseButton.vue'
import BaseCard from '@/components/base/BaseCard.vue'
import BaseConfirmModal from '@/components/base/BaseConfirmModal.vue'
import BaseEmpty from '@/components/base/BaseEmpty.vue'
import BaseFormItem from '@/components/base/BaseForm/FormItem.vue'
import BaseIconButton from '@/components/base/BaseIconButton.vue'
import BaseInput from '@/components/base/BaseInput.vue'
import BaseModal from '@/components/base/BaseModal/index.vue'
import BasePageHeader from '@/components/base/BasePageHeader.vue'
import BaseSegmented from '@/components/base/BaseSegmented.vue'
import BaseSelect from '@/components/base/BaseSelect.vue'
import BaseTablePagination from '@/components/base/BaseTable/BaseTablePagination.vue'
import { defineTableColumns } from '@/components/base/BaseTable/columns'
import BaseTable from '@/components/base/BaseTable/index.vue'
import { toast } from '@/components/base/BaseToast'
import { useAccountGroupCatalog } from '@/composables/useAccountGroupCatalog'
import { useAuthStore } from '@/stores/modules/auth'
import { formatDateTime } from '@/utils/date'
import { DEFAULT_ACCOUNT_GROUP_COLOR } from '@/views/groups/constants'
import { displayLimit, displayMoney } from '@/views/user/utils'
import PlanLimitField from './components/PlanLimitField.vue'
import UserAccountManagementModal from './components/UserAccountManagementModal.vue'
import UserManagementActions from './components/UserManagementActions.vue'
import UserPlanManagementModal from './components/UserPlanManagementModal.vue'

const route = useRoute()
const router = useRouter()
const authStore = useAuthStore()

const activeTab = shallowRef(route.query.tab === 'plans' ? 'plans' : 'users')
const loading = shallowRef(false)
const plansLoading = shallowRef(false)
const saving = shallowRef(false)
const listError = shallowRef('')
const plansError = shallowRef('')
let planLoadRevision = 0
const rows = shallowRef<UserManagementSummary[]>([])
const plans = shallowRef<SubscriptionPlan[]>([])
const page = shallowRef(1)
const pageSize = shallowRef(20)
const total = shallowRef(0)
const query = shallowRef('')
const role = shallowRef('')
const status = shallowRef('')
let listController: AbortController | undefined

const {
  groups: groupCatalog,
  loading: groupsLoading,
  loadGroups,
} = useAccountGroupCatalog({ immediate: false })

const accountTarget = shallowRef<UserManagementSummary | null>(null)
const planTarget = shallowRef<UserManagementSummary | null>(null)
const toggleTarget = shallowRef<UserManagementSummary | null>(null)
const deleteTarget = shallowRef<UserManagementSummary | null>(null)
const accountManagementOpen = shallowRef(false)
const planManagementOpen = shallowRef(false)

const createOpen = shallowRef(false)
const createForm = reactive({ username: '', password: '' })
const createPasswordVisible = shallowRef(false)

const toggleOpen = shallowRef(false)
const deleteOpen = shallowRef(false)

const planModalOpen = shallowRef(false)
const editingPlan = shallowRef<SubscriptionPlan | null>(null)
interface MoneyDraft { enabled: boolean, value: string }
const planForm = reactive({
  name: '',
  description: '',
  dailyLimitUsd: { enabled: true, value: '0' } as MoneyDraft,
  weeklyLimitUsd: { enabled: true, value: '0' } as MoneyDraft,
  monthlyLimitUsd: { enabled: true, value: '0' } as MoneyDraft,
})

const userColumns = defineTableColumns<UserManagementSummary>([
  { key: 'user', label: '账户', kind: 'identity', size: 'xl' },
  { key: 'role', label: '角色 / 状态', kind: 'status', size: 'md' },
  { key: 'plan', label: '当前套餐', kind: 'custom', size: 'md' },
  { key: 'groups', label: '上游分组', kind: 'status', size: 'sm' },
  { key: 'accountLimits', label: '并发 / RPM', kind: 'custom', size: 'md' },
  { key: 'multiplier', label: '倍率', kind: 'numeric', size: 'xs' },
  { key: 'expiresAt', label: '到期', kind: 'datetime', size: 'xl' },
  { key: 'budget', label: '额度 / 用量', kind: 'custom', size: 'md' },
  { key: 'actions', label: '操作', kind: 'actions', size: 'lg', fixedWidth: true },
])

const planColumns = defineTableColumns<SubscriptionPlan>([
  { key: 'name', label: '套餐模板', kind: 'identity', size: '2xl' },
  { key: 'enabled', label: '状态', kind: 'status', size: 'sm' },
  { key: 'budget', label: '日 / 周 / 月额度', kind: 'custom', size: '3xl' },
  { key: 'updatedAt', label: '更新时间', kind: 'datetime', size: 'xl' },
  { key: 'actions', label: '操作', kind: 'actions', size: 'md', fixedWidth: true },
])

const pagination = computed(() => ({
  currentPage: page.value,
  pageSize: pageSize.value,
  total: total.value,
}))
const groupCatalogById = computed(() =>
  new Map(groupCatalog.value.map(group => [group.id, group])),
)

function userGroupMarks(row: UserManagementSummary): AccountGroupRef[] {
  return row.groups.map((group) => {
    const catalog = groupCatalogById.value.get(group.groupId)
    return catalog ?? {
      id: group.groupId,
      name: group.name,
      color: DEFAULT_ACCOUNT_GROUP_COLOR,
      enabled: group.enabled,
    }
  })
}

function planMoney(value: string | null | undefined): MoneyDraft {
  return { enabled: value !== null, value: value ?? '0' }
}
function moneyPayload(value: MoneyDraft) {
  return value.enabled ? value.value : null
}
function formatPlanLimit(value: string | null) {
  return value === null ? '不限' : `$${value}`
}
function formatPlanBudget(row: SubscriptionPlan) {
  return `日 ${formatPlanLimit(row.dailyLimitUsd)} · 周 ${formatPlanLimit(row.weeklyLimitUsd)} · 月 ${formatPlanLimit(row.monthlyLimitUsd)}`
}
function formatBudget(row: UserManagementSummary) {
  const daily = row.budget.daily
  return `日 ${displayMoney(daily.used)} / ${daily.effectiveLimit === null ? '不限' : displayMoney(daily.effectiveLimit)}`
}

function tabChanged(value: string) {
  activeTab.value = value
  void router.replace({ query: { ...route.query, tab: value } })
  if (value === 'plans' && !plans.value.length)
    void loadPlans()
}

function generatePassword() {
  const chars = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!@#$%*-_=+'
  const values = new Uint32Array(24)
  crypto.getRandomValues(values)
  createForm.password = Array.from(values, value => chars[value % chars.length]).join('')
  createPasswordVisible.value = true
}

async function copyPassword() {
  if (!createForm.password)
    return
  try {
    await navigator.clipboard.writeText(createForm.password)
    toast.success('密码已复制')
  }
  catch {
    toast.error('复制失败，请手动复制')
  }
}

async function loadUsers() {
  listController?.abort()
  const controller = new AbortController()
  listController = controller
  loading.value = true
  listError.value = ''
  try {
    const result = await getAdminUsersManagement({
      page: page.value,
      pageSize: pageSize.value,
      query: query.value.trim() || undefined,
      role: role.value || undefined,
      status: status.value || undefined,
    }, { signal: controller.signal, silent: true })
    if (controller.signal.aborted || listController !== controller)
      return
    rows.value = result.items
    total.value = result.total
  }
  catch {
    if (!controller.signal.aborted && listController === controller)
      listError.value = '用户列表加载失败'
  }
  finally {
    if (listController === controller)
      loading.value = false
  }
}

async function loadPlans() {
  const revision = ++planLoadRevision
  plansLoading.value = true
  plansError.value = ''
  try {
    const result = await getSubscriptionPlans()
    if (revision === planLoadRevision)
      plans.value = result
  }
  catch {
    if (revision === planLoadRevision)
      plansError.value = '套餐模板加载失败'
  }
  finally {
    if (revision === planLoadRevision)
      plansLoading.value = false
  }
}

async function loadAll() {
  await Promise.all([loadUsers(), loadPlans(), loadGroups()])
}

async function createUser() {
  if (saving.value || !createForm.username.trim() || !createForm.password)
    return
  saving.value = true
  try {
    await createAdminUser({
      username: createForm.username.trim(),
      password: createForm.password,
      enabled: true,
    })
    createOpen.value = false
    Object.assign(createForm, { username: '', password: '' })
    createPasswordVisible.value = false
    toast.success('账户已创建')
    await loadUsers()
  }
  catch {}
  finally {
    saving.value = false
  }
}

function openAccountManagement(row: UserManagementSummary) {
  if (saving.value || accountManagementOpen.value)
    return
  accountTarget.value = { ...row }
  accountManagementOpen.value = true
}

function openPlanManagement(row: UserManagementSummary) {
  if (saving.value || planManagementOpen.value)
    return
  planTarget.value = { ...row }
  planManagementOpen.value = true
}

function requestToggle(row: UserManagementSummary) {
  if (saving.value || toggleOpen.value)
    return
  toggleTarget.value = { ...row }
  toggleOpen.value = true
}

async function toggleAccount() {
  const target = toggleTarget.value
  if (!target || saving.value)
    return
  saving.value = true
  try {
    await updateAdminUser(target.userId, { enabled: !target.enabled })
    toggleOpen.value = false
    toast.success(target.enabled ? '账户已停用' : '账户已启用')
    await loadUsers()
  }
  catch {}
  finally {
    saving.value = false
  }
}

function requestDelete(row: UserManagementSummary) {
  if (saving.value || deleteOpen.value)
    return
  deleteTarget.value = { ...row }
  deleteOpen.value = true
}

async function deleteAccount() {
  const target = deleteTarget.value
  if (!target || saving.value)
    return
  saving.value = true
  try {
    await deleteAdminUser(target.userId)
    deleteOpen.value = false
    toast.success('账户已删除')
    await loadUsers()
  }
  catch {}
  finally {
    saving.value = false
  }
}

function openPlan(plan?: SubscriptionPlan) {
  if (saving.value || planModalOpen.value)
    return
  editingPlan.value = plan ?? null
  Object.assign(planForm, {
    name: plan?.name ?? '',
    description: plan?.description ?? '',
    dailyLimitUsd: planMoney(plan?.dailyLimitUsd),
    weeklyLimitUsd: planMoney(plan?.weeklyLimitUsd),
    monthlyLimitUsd: planMoney(plan?.monthlyLimitUsd),
  })
  planModalOpen.value = true
}

async function savePlan() {
  if (saving.value || !planForm.name.trim())
    return
  const payload = {
    name: planForm.name.trim(),
    description: planForm.description.trim() || null,
    dailyLimitUsd: moneyPayload(planForm.dailyLimitUsd),
    weeklyLimitUsd: moneyPayload(planForm.weeklyLimitUsd),
    monthlyLimitUsd: moneyPayload(planForm.monthlyLimitUsd),
  }
  saving.value = true
  try {
    if (editingPlan.value)
      await updateSubscriptionPlan(editingPlan.value.id, payload)
    else
      await createSubscriptionPlan(payload)
    planModalOpen.value = false
    toast.success('套餐模板已保存')
    await Promise.all([loadPlans(), loadUsers()])
  }
  catch {}
  finally {
    saving.value = false
  }
}

async function togglePlan(plan: SubscriptionPlan) {
  if (plan.isBase || saving.value)
    return
  saving.value = true
  try {
    await setSubscriptionPlanEnabled(plan.id, !plan.enabled)
    toast.success(plan.enabled ? '套餐模板已停用' : '套餐模板已启用')
    await Promise.all([loadPlans(), loadUsers()])
  }
  catch {}
  finally { saving.value = false }
}

async function refreshAfterAccountManagement(userId: string, sessionInvalidated: boolean) {
  if (userId === authStore.currentUser?.id) {
    if (sessionInvalidated) {
      accountManagementOpen.value = false
      authStore.invalidateSession()
      toast.success('账户身份已变更，请重新登录')
      await router.replace('/login')
      return
    }
    if (!await authStore.checkAuth()) {
      await router.replace('/login')
      return
    }
  }
  await Promise.all([loadUsers(), loadGroups()])
}

function refreshAfterPlanManagement() {
  void loadUsers()
}

watch([query, role, status], () => {
  page.value = 1
  if (activeTab.value === 'users')
    void loadUsers()
})
watch([page, pageSize], () => {
  if (activeTab.value === 'users')
    void loadUsers()
})
watch(() => route.query.tab, (value) => {
  activeTab.value = value === 'plans' ? 'plans' : 'users'
})

onMounted(loadAll)
onBeforeUnmount(() => listController?.abort())
</script>

<template>
  <div class="flex min-h-0 w-full flex-col">
    <BasePageHeader class="h-17" title="用户管理">
      <template #actions>
        <BaseIconButton
          variant="ghost"
          size="sm"
          label="刷新"
          :loading="loading || plansLoading || groupsLoading"
          @click="loadAll"
        >
          <RefreshCw class="size-3.5 text-cp-link" />
        </BaseIconButton>
        <BaseButton v-if="activeTab === 'users'" @click="createOpen = true">
          <template #icon>
            <Plus class="size-4" />
          </template>
          创建账户
        </BaseButton>
        <BaseButton v-else @click="openPlan()">
          <template #icon>
            <Plus class="size-4" />
          </template>
          新建模板
        </BaseButton>
      </template>
    </BasePageHeader>

    <div class="mt-4">
      <BaseSegmented
        v-model="activeTab"
        label="用户管理区域"
        :options="[
          { label: '用户', value: 'users' },
          { label: '套餐模板', value: 'plans' },
        ]"
        @update:model-value="tabChanged"
      />
    </div>

    <BaseCard
      v-if="activeTab === 'users'"
      class="mt-4 flex flex-col h-[max(26rem,calc(100dvh-180px))] min-h-0"
    >
      <template #header>
        <div class="flex w-full flex-col gap-3 lg:flex-row lg:items-end">
          <BaseFormItem label="搜索" class="min-w-0 flex-1 lg:max-w-96">
            <BaseInput v-model="query" placeholder="搜索用户名" aria-label="搜索用户" />
          </BaseFormItem>
          <BaseFormItem label="角色" class="w-full lg:w-40">
            <BaseSelect
              v-model="role"
              :options="[
                { label: '全部角色', value: '' },
                { label: '普通用户', value: 'user' },
                { label: '管理员', value: 'admin' },
              ]"
            />
          </BaseFormItem>
          <BaseFormItem label="状态" class="w-full lg:w-40">
            <BaseSelect
              v-model="status"
              :options="[
                { label: '全部状态', value: '' },
                { label: '启用', value: 'enabled' },
                { label: '停用', value: 'disabled' },
              ]"
            />
          </BaseFormItem>
          <span class="pb-2 text-cp-sm text-cp-text-tertiary lg:ml-auto">共 {{ total }} 个账户</span>
        </div>
      </template>

      <template #body>
        <BaseEmpty v-if="listError" :title="listError" description="列表未能更新，请重试。旧数据不会作为成功结果显示。" class="my-auto">
          <template #action>
            <BaseButton :loading="loading" @click="loadUsers">
              重新加载
            </BaseButton>
          </template>
        </BaseEmpty>
        <div v-else class="flex h-full min-h-0 flex-col">
          <BaseTable
            class="min-h-0 flex-1"
            row-key="userId"
            :columns="userColumns"
            :rows="rows"
            :loading="loading"
            empty-text="暂无用户"
          >
            <template #user="{ row }">
              <strong class="block truncate text-cp-text" :title="row.username">
                {{ row.username }}
              </strong>
            </template>

            <template #role="{ row }">
              <span
                class="inline-flex h-6 shrink-0 items-center whitespace-nowrap rounded-lg px-2 text-cp-xs font-bold"
                :class="row.enabled
                  ? 'bg-cp-success-container text-cp-success-on-container'
                  : 'bg-cp-fill-tertiary text-cp-text-quaternary'"
              >
                {{ row.role === 'admin' ? '管理员' : '用户' }} · {{ row.enabled ? '启用' : '停用' }}
              </span>
            </template>

            <template #plan="{ row }">
              <span class="truncate text-cp-text">{{ row.effectivePlan.name }}</span>
            </template>

            <template #groups="{ row }">
              <div class="flex w-full justify-center">
                <AccountGroupMarks :groups="userGroupMarks(row)" />
              </div>
            </template>

            <template #accountLimits="{ row }">
              <span class="whitespace-nowrap font-mono text-cp-sm">
                {{ displayLimit(row.effectiveMaxConcurrency) }} / {{ displayLimit(row.effectiveRequestsPerMinute) }}
              </span>
            </template>

            <template #multiplier="{ row }">
              <span class="font-mono">{{ row.subscription ? row.multiplier : '—' }}</span>
            </template>

            <template #expiresAt="{ row }">
              <span>{{ row.subscription?.expiresAt ? formatDateTime(row.subscription.expiresAt) : '—' }}</span>
            </template>

            <template #budget="{ row }">
              <span class="whitespace-nowrap font-mono text-cp-sm">{{ formatBudget(row) }}</span>
            </template>

            <template #actions="{ row }">
              <UserManagementActions
                :user="row"
                :current-user-id="authStore.currentUser?.id"
                @account="openAccountManagement"
                @plan="openPlanManagement"
                @toggle="requestToggle"
                @delete="requestDelete"
              />
            </template>
          </BaseTable>

          <BaseTablePagination
            :pagination="pagination"
            :loading="loading"
            @page-change="page = $event"
            @page-size-change="pageSize = $event"
          />
        </div>
      </template>
    </BaseCard>

    <BaseCard
      v-else
      class="mt-4 flex flex-col h-[max(26rem,calc(100dvh-180px))] min-h-0"
    >
      <template #body>
        <BaseEmpty v-if="plansError" :title="plansError" class="my-auto">
          <template #action>
            <BaseButton :loading="plansLoading" @click="loadPlans">
              重新加载
            </BaseButton>
          </template>
        </BaseEmpty>
        <div v-else class="flex h-full min-h-0 flex-col">
          <BaseTable
            class="min-h-0 flex-1"
            :columns="planColumns"
            :rows="plans"
            :loading="plansLoading"
            empty-text="暂无套餐模板"
          >
            <template #name="{ row }">
              <div class="grid min-w-0 gap-1">
                <strong class="truncate text-cp-text">{{ row.name }}</strong>
                <span
                  v-if="row.description && row.description !== row.name"
                  class="truncate text-cp-xs text-cp-text-quaternary"
                >
                  {{ row.description }}
                </span>
              </div>
            </template>

            <template #enabled="{ row }">
              <span :class="row.enabled ? 'text-cp-success-text' : 'text-cp-text-tertiary'">
                {{ row.enabled ? '启用' : '停用' }}
              </span>
            </template>

            <template #budget="{ row }">
              <span class="whitespace-nowrap font-mono text-cp-sm">{{ formatPlanBudget(row) }}</span>
            </template>

            <template #updatedAt="{ row }">
              <time class="font-mono text-cp-sm tabular-nums text-cp-text-secondary" :datetime="row.updatedAt">
                {{ formatDateTime(row.updatedAt) }}
              </time>
            </template>

            <template #actions="{ row }">
              <div class="flex items-center gap-1">
                <BaseIconButton
                  variant="ghost"
                  size="sm"
                  label="编辑套餐"
                  @click="openPlan(row)"
                >
                  <Pencil class="size-3.5 text-cp-link" />
                </BaseIconButton>
                <BaseIconButton
                  v-if="!row.isBase"
                  variant="ghost"
                  size="sm"
                  :label="row.enabled ? '停用套餐' : '启用套餐'"
                  @click="togglePlan(row)"
                >
                  <Power
                    class="size-3.5"
                    :class="row.enabled ? 'text-cp-warning' : 'text-cp-success'"
                  />
                </BaseIconButton>
              </div>
            </template>
          </BaseTable>
        </div>
      </template>
    </BaseCard>

    <BaseModal v-model="createOpen" title="创建账户" size="md" :dismissible="!saving">
      <div class="grid gap-4">
        <BaseFormItem label="用户名" required>
          <BaseInput v-model="createForm.username" autocomplete="off" />
        </BaseFormItem>
        <BaseFormItem label="初始密码" required>
          <div class="flex min-w-0 items-center gap-2">
            <BaseInput
              v-model="createForm.password"
              class="min-w-0 flex-1"
              :type="createPasswordVisible ? 'text' : 'password'"
              autocomplete="new-password"
            />
            <BaseIconButton variant="secondary" size="sm" label="随机生成密码" @click="generatePassword">
              <Shuffle class="size-3.5" />
            </BaseIconButton>
            <BaseIconButton
              variant="secondary"
              size="sm"
              label="复制密码"
              :disabled="!createForm.password"
              @click="copyPassword"
            >
              <Copy class="size-3.5" />
            </BaseIconButton>
          </div>
        </BaseFormItem>
      </div>
      <template #footer>
        <BaseButton variant="secondary" :disabled="saving" @click="createOpen = false">
          取消
        </BaseButton>
        <BaseButton :loading="saving" @click="createUser">
          创建
        </BaseButton>
      </template>
    </BaseModal>

    <UserAccountManagementModal
      v-model="accountManagementOpen"
      :user="accountTarget"
      :groups="groupCatalog"
      :groups-loading="groupsLoading"
      @updated="refreshAfterAccountManagement"
    />

    <UserPlanManagementModal
      v-model="planManagementOpen"
      :user="planTarget"
      :plans="plans"
      @updated="refreshAfterPlanManagement"
    />

    <BaseConfirmModal
      v-model="toggleOpen"
      :title="toggleTarget?.enabled ? '停用账户' : '启用账户'"
      :description="toggleTarget?.enabled ? '停用后该账户立即无法继续登录和调用。' : '启用后该账户恢复登录资格。'"
      :confirm-text="toggleTarget?.enabled ? '确认停用' : '确认启用'"
      :loading="saving"
      @confirm="toggleAccount"
    >
      <p class="m-0">
        账户：{{ toggleTarget?.username }}
      </p>
    </BaseConfirmModal>

    <BaseConfirmModal
      v-model="deleteOpen"
      title="删除账号"
      description="删除后不可恢复。历史使用记录将按服务端保留策略继续保留。"
      confirm-text="确认删除"
      destructive
      :loading="saving"
      @confirm="deleteAccount"
    >
      <p class="m-0">
        确定删除“{{ deleteTarget?.username }}”吗？
      </p>
    </BaseConfirmModal>

    <BaseModal
      v-model="planModalOpen"
      :title="editingPlan ? '编辑套餐模板' : '新建套餐模板'"
      size="lg"
      :dismissible="!saving"
    >
      <div class="grid gap-5 sm:grid-cols-2">
        <BaseFormItem label="名称" required>
          <BaseInput v-model="planForm.name" />
        </BaseFormItem>
        <BaseFormItem label="说明">
          <BaseInput v-model="planForm.description" />
        </BaseFormItem>
        <BaseCard class="@container/limits sm:col-span-2" title="计费额度">
          <div class="grid gap-3 @min-[30rem]/limits:grid-cols-3">
            <PlanLimitField v-model="planForm.dailyLimitUsd" label="日额度" kind="money" />
            <PlanLimitField v-model="planForm.weeklyLimitUsd" label="周额度" kind="money" />
            <PlanLimitField v-model="planForm.monthlyLimitUsd" label="月额度" kind="money" />
          </div>
        </BaseCard>
      </div>
      <template #footer>
        <BaseButton variant="secondary" :disabled="saving" @click="planModalOpen = false">
          取消
        </BaseButton>
        <BaseButton :loading="saving" @click="savePlan">
          保存
        </BaseButton>
      </template>
    </BaseModal>
  </div>
</template>
