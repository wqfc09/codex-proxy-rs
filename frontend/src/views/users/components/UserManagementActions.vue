<script setup lang="ts">
import type { UserManagementSummary } from '@/api'
import { Power, Trash2, Users, WalletCards } from '@lucide/vue'

import BaseIconButton from '@/components/base/BaseIconButton.vue'

defineProps<{
  user: UserManagementSummary
  currentUserId?: string
}>()

const emit = defineEmits<{
  account: [user: UserManagementSummary]
  plan: [user: UserManagementSummary]
  toggle: [user: UserManagementSummary]
  delete: [user: UserManagementSummary]
}>()
</script>

<template>
  <div class="flex items-center justify-start gap-0">
    <BaseIconButton
      variant="ghost"
      size="sm"
      label="账号管理"
      @click.stop="emit('account', user)"
    >
      <Users class="size-3.5 text-cp-link" />
    </BaseIconButton>

    <BaseIconButton
      variant="ghost"
      size="sm"
      label="套餐管理"
      @click.stop="emit('plan', user)"
    >
      <WalletCards class="size-3.5 text-cp-link" />
    </BaseIconButton>

    <BaseIconButton
      variant="ghost"
      size="sm"
      :label="user.userId === currentUserId ? '当前账户不能停用' : user.enabled ? '停用账户' : '启用账户'"
      :disabled="user.userId === currentUserId"
      @click.stop="emit('toggle', user)"
    >
      <Power
        class="size-3.5"
        :class="user.enabled ? 'text-cp-warning' : 'text-cp-success'"
      />
    </BaseIconButton>

    <BaseIconButton
      variant="ghost"
      size="sm"
      :label="user.userId === currentUserId ? '当前账户不能删除' : '删除账号'"
      :disabled="user.userId === currentUserId"
      @click.stop="emit('delete', user)"
    >
      <Trash2 class="size-3.5 text-cp-error" />
    </BaseIconButton>
  </div>
</template>
