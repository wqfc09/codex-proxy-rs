<script setup lang="ts">
import type { PlanLimitDraft } from './PlanLimitField.vue'
import type { AccountGroup, UserKeyIdentity, UserManagementSummary } from '@/api'
import { Copy, KeyRound, Network, Settings2, Shuffle } from '@lucide/vue'
import { computed, reactive, shallowRef, watch } from 'vue'

import {
  getAdminUserGroups,
  getAdminUserKeyIdentity,
  replaceAdminUserGroups,
  resetAdminUserPassword,
  updateAdminUser,
  updateAdminUserClientKeyIdentity,
  updateAdminUserKeyIdentity,
} from '@/api'
import AccountGroupMarks from '@/components/AccountGroupMarks.vue'
import BaseButton from '@/components/base/BaseButton.vue'
import BaseCheckbox from '@/components/base/BaseCheckbox.vue'
import BaseEmpty from '@/components/base/BaseEmpty.vue'
import BaseFormItem from '@/components/base/BaseForm/FormItem.vue'
import BaseIconButton from '@/components/base/BaseIconButton.vue'
import BaseInput from '@/components/base/BaseInput.vue'
import BaseModal from '@/components/base/BaseModal/index.vue'
import BaseSegmented from '@/components/base/BaseSegmented.vue'
import BaseSelect from '@/components/base/BaseSelect.vue'
import { toast } from '@/components/base/BaseToast'
import ClientProfileEditor from '@/components/client-profile/ClientProfileEditor.vue'
import XaiClientProfileEditor from '@/components/client-profile/XaiClientProfileEditor.vue'
import { useRequestState } from '@/composables/useRequestState'
import PlanLimitField from './PlanLimitField.vue'

type Section = 'account' | 'groups' | 'identity' | 'password'

const props = defineProps<{
  user: UserManagementSummary | null
  groups: AccountGroup[]
  groupsLoading: boolean
}>()
const emit = defineEmits<{ updated: [userId: string, sessionInvalidated: boolean] }>()
const open = defineModel<boolean>({ required: true })

const section = shallowRef<Section>('account')
const groupsRequest = useRequestState()
const { loading, error: groupsError } = groupsRequest
const groupsLoaded = shallowRef(false)
const identityRequest = useRequestState()
const { loading: identityLoading, error: identityError } = identityRequest
const identityLoaded = shallowRef(false)
const identity = reactive<UserKeyIdentity>({
  openaiClientProfileOverride: null,
  xaiClientProfileOverride: null,
  keyCount: 0,
  keys: [],
})
const identityTarget = shallowRef('user')

async function loadIdentity() {
  const userId = props.user?.userId
  if (!open.value || !userId)
    return
  identityLoaded.value = false
  const id = identityRequest.start()
  try {
    const value = await getAdminUserKeyIdentity(userId, { silent: true, signal: identityRequest.signal })
    if (!identityRequest.isCurrent(id) || !open.value || props.user?.userId !== userId)
      return
    Object.assign(identity, value)
    identityLoaded.value = true
  }
  catch (error) { identityRequest.fail(id, error) }
  finally { identityRequest.finish(id) }
}
const saving = shallowRef(false)
let contextRevision = 0
const assignedGroupIds = shallowRef<string[]>([])
const groupSearch = shallowRef('')
const password = shallowRef('')
const passwordVisible = shallowRef(false)
const accountForm = reactive({
  username: '',
  role: 'user' as 'admin' | 'user',
  maxConcurrency: { enabled: false, value: 0 } as PlanLimitDraft,
  requestsPerMinute: { enabled: false, value: 0 } as PlanLimitDraft,
})

const sectionOptions = [
  { label: '账户配置', value: 'account', icon: Settings2 },
  { label: '上游设置', value: 'groups', icon: Network },
  { label: 'API Key 身份', value: 'identity', icon: KeyRound },
  { label: '修改密码', value: 'password', icon: KeyRound },
]

const identityTargetOptions = computed(() => [
  { label: '用户默认身份', value: 'user' },
  ...identity.keys.map(key => ({ label: `${key.name} · ${key.prefix}`, value: key.id })),
])
const selectedIdentityKey = computed(() =>
  identityTarget.value === 'user' ? null : identity.keys.find(key => key.id === identityTarget.value) ?? null,
)
const activeOpenaiIdentity = computed({
  get: () => selectedIdentityKey.value?.openaiClientProfileOverride ?? (identityTarget.value === 'user' ? identity.openaiClientProfileOverride : null),
  set: (value) => {
    const key = selectedIdentityKey.value
    if (key)
      key.openaiClientProfileOverride = value
    else
      identity.openaiClientProfileOverride = value
  },
})
const activeXaiIdentity = computed({
  get: () => selectedIdentityKey.value?.xaiClientProfileOverride ?? (identityTarget.value === 'user' ? identity.xaiClientProfileOverride : null),
  set: (value) => {
    const key = selectedIdentityKey.value
    if (key)
      key.xaiClientProfileOverride = value
    else
      identity.xaiClientProfileOverride = value
  },
})

const filteredGroups = computed(() => {
  const needle = groupSearch.value.trim().toLocaleLowerCase()
  if (!needle)
    return props.groups
  return props.groups.filter(group =>
    `${group.name} ${group.description ?? ''}`.toLocaleLowerCase().includes(needle),
  )
})

function syncAccountForm() {
  accountForm.username = props.user?.username ?? ''
  accountForm.role = props.user?.role ?? 'user'
  accountForm.maxConcurrency = {
    enabled: props.user?.effectiveMaxConcurrency !== null && props.user?.effectiveMaxConcurrency !== undefined,
    value: props.user?.effectiveMaxConcurrency ?? 0,
  }
  accountForm.requestsPerMinute = {
    enabled: props.user?.effectiveRequestsPerMinute !== null && props.user?.effectiveRequestsPerMinute !== undefined,
    value: props.user?.effectiveRequestsPerMinute ?? 0,
  }
}

async function loadGroups() {
  const userId = props.user?.userId
  if (!open.value || !userId)
    return
  const requestId = groupsRequest.start()
  groupsLoaded.value = false
  assignedGroupIds.value = []
  try {
    const assigned = await getAdminUserGroups(userId, { silent: true, signal: groupsRequest.signal })
    if (!groupsRequest.isCurrent(requestId) || !open.value || props.user?.userId !== userId)
      return
    assignedGroupIds.value = assigned.items.map(group => group.groupId)
    groupsLoaded.value = true
  }
  catch (error) {
    groupsRequest.fail(requestId, error)
  }
  finally {
    groupsRequest.finish(requestId)
  }
}

function toggleGroup(id: string, checked: boolean) {
  assignedGroupIds.value = checked
    ? [...new Set([...assignedGroupIds.value, id])]
    : assignedGroupIds.value.filter(groupId => groupId !== id)
}

function generatePassword() {
  const chars = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!@#$%*-_=+'
  const values = new Uint32Array(24)
  crypto.getRandomValues(values)
  password.value = Array.from(values, value => chars[value % chars.length]).join('')
  passwordVisible.value = true
}

async function copyPassword() {
  if (!password.value)
    return
  try {
    await navigator.clipboard.writeText(password.value)
    toast.success('密码已复制')
  }
  catch {
    toast.error('复制失败，请手动复制')
  }
}

const saveDisabled = computed(() => {
  if (!props.user || loading.value || saving.value)
    return true
  if (section.value === 'groups')
    return props.groupsLoading || !groupsLoaded.value
  if (section.value === 'identity')
    return identityLoading.value || !identityLoaded.value
  if (section.value === 'password')
    return !password.value
  return !accountForm.username.trim()
})

async function save() {
  const userId = props.user?.userId
  if (!userId || saveDisabled.value)
    return
  const context = contextRevision
  const savedSection = section.value
  const sessionInvalidated = savedSection === 'password' || (savedSection === 'account' && accountForm.role !== props.user?.role)
  saving.value = true
  try {
    if (savedSection === 'account') {
      await updateAdminUser(userId, {
        username: accountForm.username.trim(),
        role: accountForm.role,
        maxConcurrency: accountForm.maxConcurrency.enabled ? Number(accountForm.maxConcurrency.value) : null,
        requestsPerMinute: accountForm.requestsPerMinute.enabled ? Number(accountForm.requestsPerMinute.value) : null,
      })
      if (context === contextRevision)
        toast.success('账户配置已更新')
    }
    else if (savedSection === 'groups') {
      await replaceAdminUserGroups(userId, [...assignedGroupIds.value])
      if (context === contextRevision)
        toast.success('上游设置已更新')
    }
    else if (savedSection === 'identity') {
      const data = {
        openaiClientProfileOverride: activeOpenaiIdentity.value,
        xaiClientProfileOverride: activeXaiIdentity.value,
      }
      const key = selectedIdentityKey.value
      if (key)
        await updateAdminUserClientKeyIdentity(userId, key.id, data)
      else
        await updateAdminUserKeyIdentity(userId, data)
      if (context === contextRevision)
        toast.success(key ? `${key.name} 的 API Key 身份已更新` : '用户默认 API Key 身份已更新')
    }
    else {
      if (!password.value)
        return
      await resetAdminUserPassword(userId, password.value)
      if (context === contextRevision) {
        password.value = ''
        passwordVisible.value = false
        toast.success('密码已修改')
      }
    }
    emit('updated', userId, sessionInvalidated)
  }
  catch {}
  finally {
    saving.value = false
  }
}

watch([open, () => props.user?.userId], ([isOpen]) => {
  contextRevision += 1
  groupsRequest.invalidate()
  identityRequest.invalidate()
  identityLoaded.value = false
  identityTarget.value = 'user'
  Object.assign(identity, {
    openaiClientProfileOverride: null,
    xaiClientProfileOverride: null,
    keyCount: 0,
    keys: [],
  })
  groupsLoaded.value = false
  section.value = 'account'
  groupSearch.value = ''
  password.value = ''
  passwordVisible.value = false
  assignedGroupIds.value = []
  if (!isOpen)
    return
  syncAccountForm()
  void loadGroups()
  void loadIdentity()
}, { immediate: true, flush: 'sync' })
</script>

<template>
  <BaseModal
    v-model="open"
    title="账号管理"
    :description="user?.username"
    tone="info"
    size="xl"
    :dismissible="!saving"
  >
    <div class="grid min-h-0 gap-5">
      <BaseSegmented
        v-model="section"
        class="w-full"
        label="账号管理功能"
        size="sm"
        :disabled="loading || saving"
        :options="sectionOptions"
      />

      <div v-if="section === 'account'" class="grid min-w-0 gap-4">
        <div class="grid min-w-0 gap-4 sm:grid-cols-2">
          <BaseFormItem label="用户名" required>
            <BaseInput v-model="accountForm.username" :disabled="saving" />
          </BaseFormItem>
          <BaseFormItem label="账户类型" required>
            <BaseSelect
              v-model="accountForm.role"
              :disabled="saving"
              :options="[
                { label: '普通用户', value: 'user' },
                { label: '管理员', value: 'admin' },
              ]"
            />
          </BaseFormItem>
        </div>
        <div class="grid min-w-0 gap-3 sm:grid-cols-2">
          <PlanLimitField
            v-model="accountForm.maxConcurrency"
            label="并发限制"
            kind="number"
            :disabled="saving"
          />
          <PlanLimitField
            v-model="accountForm.requestsPerMinute"
            label="RPM 限制"
            kind="number"
            :disabled="saving"
          />
        </div>
      </div>

      <div v-else-if="section === 'groups'" class="grid gap-3">
        <BaseInput
          v-model="groupSearch"
          placeholder="搜索上游分组..."
          aria-label="搜索上游分组"
          :disabled="groupsLoading || loading || saving"
        />
        <div v-if="groupsLoading || loading" class="py-10 text-center text-cp-sm text-cp-text-secondary">
          正在加载…
        </div>
        <BaseEmpty v-else-if="groupsError" title="上游设置加载失败" description="重新加载后才能修改分组。" size="sm">
          <template #action>
            <BaseButton variant="secondary" :disabled="saving" @click="loadGroups">
              重新加载
            </BaseButton>
          </template>
        </BaseEmpty>
        <div v-else-if="filteredGroups.length" class="grid max-h-88 gap-1 overflow-y-auto pr-1">
          <div
            v-for="group in filteredGroups"
            :key="group.id"
            class="flex min-h-12 items-center gap-3 rounded-cp px-3 py-2 hover:bg-cp-fill-quaternary"
          >
            <AccountGroupMarks :groups="[group]" />
            <div class="min-w-0 flex-1">
              <strong class="block truncate text-cp-sm text-cp-text">{{ group.name }}</strong>
              <span class="block truncate text-cp-xs text-cp-text-quaternary">
                {{ group.description || (group.enabled ? '已启用' : '已禁用') }}
              </span>
            </div>
            <BaseCheckbox
              :model-value="assignedGroupIds.includes(group.id)"
              :label="`选择${group.name}`"
              :disabled="saving"
              @update:model-value="toggleGroup(group.id, $event)"
            />
          </div>
        </div>
        <div v-else class="py-10 text-center text-cp-sm text-cp-text-secondary">
          暂无匹配分组
        </div>
      </div>

      <div v-else-if="section === 'identity'" class="grid min-w-0 gap-6">
        <BaseEmpty v-if="identityError" title="身份设置加载失败" description="重新加载后才能保存，现有配置不会被空值覆盖。" size="sm">
          <template #action>
            <BaseButton :loading="identityLoading" @click="loadIdentity">
              重新加载
            </BaseButton>
          </template>
        </BaseEmpty>
        <div v-else class="grid min-w-0 gap-6">
          <div class="grid min-w-0 gap-3 rounded-cp bg-cp-fill-quaternary p-3 sm:grid-cols-[minmax(0,1fr)_minmax(15rem,22rem)] sm:items-center">
            <div class="min-w-0">
              <strong class="block text-cp-sm font-emphasis text-cp-text">
                已创建 {{ identity.keyCount }} 个 API Key
              </strong>
              <span class="mt-1 block text-cp-xs text-cp-text-tertiary">
                用户默认身份会被所属 Key 继承；选择具体 Key 可单独覆盖。
              </span>
            </div>
            <BaseSelect
              v-model="identityTarget"
              class="w-full"
              :options="identityTargetOptions"
              :disabled="saving || identityLoading || !identityLoaded"
            />
          </div>

          <div v-if="selectedIdentityKey" class="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-cp-xs text-cp-text-tertiary">
            <span class="font-mono">{{ selectedIdentityKey.prefix }}</span>
            <span>{{ selectedIdentityKey.enabled ? '已启用' : '已停用' }}</span>
            <span v-if="selectedIdentityKey.label">{{ selectedIdentityKey.label }}</span>
          </div>

          <BaseFormItem label="OpenAI 上游身份">
            <ClientProfileEditor
              v-model="activeOpenaiIdentity"
              allow-inherit
              :inherit-label="selectedIdentityKey ? '继承用户' : '全局配置'"
              :inherit-selection="selectedIdentityKey ? identity.openaiClientProfileOverride : undefined"
              :disabled="saving || identityLoading || !identityLoaded"
            />
          </BaseFormItem>
          <BaseFormItem label="xAI 上游身份">
            <XaiClientProfileEditor
              v-model="activeXaiIdentity"
              allow-inherit
              :inherit-label="selectedIdentityKey ? '继承用户' : '全局配置'"
              :inherit-selection="selectedIdentityKey ? identity.xaiClientProfileOverride : undefined"
              :disabled="saving || identityLoading || !identityLoaded"
            />
          </BaseFormItem>
        </div>
      </div>

      <BaseFormItem v-else label="新密码" required>
        <div class="flex min-w-0 items-center gap-2">
          <BaseInput
            v-model="password"
            class="min-w-0 flex-1"
            :type="passwordVisible ? 'text' : 'password'"
            autocomplete="new-password"
            :disabled="saving"
          />
          <BaseIconButton
            variant="secondary"
            size="sm"
            label="随机生成密码"
            :disabled="saving"
            @click="generatePassword"
          >
            <Shuffle class="size-3.5" />
          </BaseIconButton>
          <BaseIconButton
            variant="secondary"
            size="sm"
            label="复制密码"
            :disabled="saving || !password"
            @click="copyPassword"
          >
            <Copy class="size-3.5" />
          </BaseIconButton>
        </div>
      </BaseFormItem>
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
        保存
      </BaseButton>
    </template>
  </BaseModal>
</template>
