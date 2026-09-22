<script setup lang="ts">
import type { AdminUser, ApiKey } from '@/api'
import { RefreshCw, Search } from '@lucide/vue'

import { onMounted, shallowRef } from 'vue'
import { getAccounts, getAdminUsers, getApiKeys } from '@/api'
import BaseIconButton from '@/components/base/BaseIconButton.vue'
import BaseInput from '@/components/base/BaseInput.vue'
import BaseSelect from '@/components/base/BaseSelect.vue'

defineProps<{
  refreshing: boolean
  loading: boolean
}>()

const emit = defineEmits<{
  refresh: []
}>()

const search = defineModel<string>('search', { required: true })
const userId = defineModel<string>('userId', { required: true })
const clientApiKeyId = defineModel<string>('clientApiKeyId', { required: true })
const accountId = defineModel<string>('accountId', { required: true })

const users = shallowRef<AdminUser[]>([])
const keys = shallowRef<ApiKey[]>([])
const accounts = shallowRef<Array<{ id: string, name: string, email: string | null }>>([])
const catalogLoading = shallowRef(false)

function userOptions() {
  return [
    { label: '全部用户', value: '' },
    ...users.value.map(user => ({ label: user.username, value: user.id, description: user.role })),
  ]
}
function keyOptions() {
  return [
    { label: '全部 API Key', value: '' },
    ...keys.value.map(key => ({ label: key.name, value: key.id, description: key.prefix })),
  ]
}
function accountOptions() {
  return [
    { label: '全部 Provider 账号', value: '' },
    ...accounts.value.map(account => ({ label: account.name, value: account.id, description: account.email ?? undefined })),
  ]
}

async function loadCatalogs() {
  catalogLoading.value = true
  try {
    const [userResult, keyResult, accountResult] = await Promise.all([
      getAdminUsers({ silent: true }),
      getApiKeys({ limit: 100, sortBy: 'name', sortDirection: 'asc' }, { silent: true }),
      getAccounts({ page: 1, pageSize: 100, sortBy: 'name', sortDirection: 'asc' }, { silent: true }),
    ])
    users.value = userResult
    keys.value = keyResult.items
    accounts.value = accountResult.items.map(account => ({ id: account.id, name: account.name, email: account.email }))
  }
  catch {
    // 筛选目录加载失败时保留搜索和空选项，列表本身仍可用。
  }
  finally {
    catalogLoading.value = false
  }
}

onMounted(() => {
  void loadCatalogs()
})
</script>

<template>
  <div class="grid w-full min-w-0 grid-cols-1 gap-3 sm:grid-cols-2 min-[1120px]:grid-cols-[minmax(12rem,1.35fr)_repeat(3,minmax(9rem,1fr))_auto]" role="group" aria-label="使用记录筛选与操作">
    <BaseInput v-model="search" placeholder="请求、用户名、Key、账号或模型" aria-label="搜索使用记录" class="min-w-0">
      <template #prefix>
        <Search class="size-4.5 text-cp-text-tertiary" />
      </template>
    </BaseInput>
    <BaseSelect v-model="userId" :options="userOptions()" :disabled="catalogLoading" aria-label="用户筛选" />
    <BaseSelect v-model="clientApiKeyId" :options="keyOptions()" :disabled="catalogLoading" aria-label="API Key 筛选" />
    <BaseSelect v-model="accountId" :options="accountOptions()" :disabled="catalogLoading" aria-label="Provider 账号筛选" />
    <div class="ml-auto flex shrink-0 items-center justify-end gap-2">
      <slot name="actions" />
      <BaseIconButton variant="ghost" size="md" label="刷新使用记录" :loading="refreshing" :disabled="loading || refreshing" @click="emit('refresh')">
        <template #loading>
          <RefreshCw class="size-4.5 animate-spin motion-reduce:animate-none" />
        </template>
        <RefreshCw class="size-4.5" />
      </BaseIconButton>
    </div>
  </div>
</template>
