<script setup lang="ts">
import { KeyRound, Pencil, ShieldCheck, UserRound } from '@lucide/vue'
import { reactive, shallowRef } from 'vue'
import { updateCurrentPassword, updateCurrentUsername } from '@/api'
import BaseButton from '@/components/base/BaseButton.vue'
import BaseCard from '@/components/base/BaseCard.vue'
import BaseFormItem from '@/components/base/BaseForm/FormItem.vue'
import BaseInput from '@/components/base/BaseInput.vue'
import BaseModal from '@/components/base/BaseModal/index.vue'
import BasePageHeader from '@/components/base/BasePageHeader.vue'
import { toast } from '@/components/base/BaseToast'
import { useAuthStore } from '@/stores/modules/auth'
import { displayLimit } from '../utils'

const authStore = useAuthStore()
const usernameOpen = shallowRef(false)
const passwordOpen = shallowRef(false)
const profileSaving = shallowRef(false)
const passwordSaving = shallowRef(false)
const profile = reactive({ username: '', currentPassword: '' })
const password = reactive({ currentPassword: '', newPassword: '', confirmPassword: '' })

function openUsernameEditor() {
  profile.username = authStore.currentUser?.username ?? ''
  profile.currentPassword = ''
  usernameOpen.value = true
}

function openPasswordEditor() {
  password.currentPassword = ''
  password.newPassword = ''
  password.confirmPassword = ''
  passwordOpen.value = true
}

async function saveProfile() {
  if (profileSaving.value || !profile.username.trim() || !profile.currentPassword)
    return
  profileSaving.value = true
  try {
    const user = await updateCurrentUsername({
      username: profile.username.trim(),
      currentPassword: profile.currentPassword,
    })
    authStore.updateCurrentUser(user)
    profile.currentPassword = ''
    usernameOpen.value = false
    toast.success('用户名已更新')
  }
  catch {}
  finally {
    profileSaving.value = false
  }
}

async function savePassword() {
  if (passwordSaving.value || !password.currentPassword || !password.newPassword)
    return
  if (password.newPassword !== password.confirmPassword) {
    toast.error('两次输入的新密码不一致')
    return
  }
  passwordSaving.value = true
  try {
    await updateCurrentPassword({
      currentPassword: password.currentPassword,
      newPassword: password.newPassword,
    })
    password.currentPassword = ''
    password.newPassword = ''
    password.confirmPassword = ''
    passwordOpen.value = false
    toast.success('密码已更新，其他登录会话已失效')
  }
  catch {}
  finally {
    passwordSaving.value = false
  }
}
</script>

<template>
  <div class="account-settings w-full min-w-0">
    <BasePageHeader title="账户设置" description="查看个人资料并管理登录安全" />

    <BaseCard class="profile-card mt-4 min-w-0" padding="compact">
      <div class="grid gap-6">
        <div class="flex min-w-0 flex-wrap items-center gap-4">
          <span class="inline-flex size-14 shrink-0 items-center justify-center rounded-full bg-cp-info-container text-cp-info-on-container">
            <UserRound class="size-7" />
          </span>
          <div class="min-w-0 flex-1">
            <p class="m-0 text-cp-xs font-emphasis text-cp-text-tertiary">
              个人资料
            </p>
            <h2 class="mt-1 mb-0 break-words text-cp-xl font-heavy text-cp-text">
              {{ authStore.currentUser?.username ?? '—' }}
            </h2>
            <span class="mt-2 inline-flex rounded-full bg-cp-fill-quaternary px-2.5 py-1 text-cp-xs font-emphasis text-cp-text-secondary">
              {{ authStore.isAdmin ? '管理员账户' : '普通用户' }}
            </span>
          </div>
        </div>

        <dl class="m-0 grid min-w-0 gap-3 sm:grid-cols-2 xl:grid-cols-4">
          <div class="rounded-cp bg-cp-fill-quaternary p-4">
            <dt class="text-cp-xs font-emphasis text-cp-text-tertiary">
              账户 ID
            </dt>
            <dd class="m-0 mt-2 break-all font-mono text-cp-sm text-cp-text">
              {{ authStore.currentUser?.id ?? '—' }}
            </dd>
          </div>
          <div class="rounded-cp bg-cp-fill-quaternary p-4">
            <dt class="text-cp-xs font-emphasis text-cp-text-tertiary">
              登录方式
            </dt>
            <dd class="m-0 mt-2 flex items-center gap-2 text-cp-sm font-emphasis text-cp-text">
              <ShieldCheck class="size-4 text-cp-success-text" />
              账户密码
            </dd>
          </div>
          <div class="rounded-cp bg-cp-fill-quaternary p-4">
            <dt class="text-cp-xs font-emphasis text-cp-text-tertiary">
              最大并发
            </dt>
            <dd class="m-0 mt-2 font-mono text-cp font-heavy tabular-nums text-cp-text">
              {{ displayLimit(authStore.currentUser?.maxConcurrency ?? null) }}
            </dd>
          </div>
          <div class="rounded-cp bg-cp-fill-quaternary p-4">
            <dt class="text-cp-xs font-emphasis text-cp-text-tertiary">
              RPM
            </dt>
            <dd class="m-0 mt-2 font-mono text-cp font-heavy tabular-nums text-cp-text">
              {{ displayLimit(authStore.currentUser?.requestsPerMinute ?? null) }}
            </dd>
          </div>
        </dl>

        <div class="flex flex-wrap gap-3 border-t border-cp-split pt-5">
          <BaseButton variant="secondary" @click="openUsernameEditor">
            <template #icon>
              <Pencil class="size-4" />
            </template>
            修改账户名
          </BaseButton>
          <BaseButton variant="secondary" @click="openPasswordEditor">
            <template #icon>
              <KeyRound class="size-4" />
            </template>
            修改密码
          </BaseButton>
        </div>
      </div>
    </BaseCard>

    <BaseModal
      v-model="usernameOpen"
      title="修改账户名"
      description="需要当前密码确认本次修改"
      tone="info"
      size="sm"
      :dismissible="!profileSaving"
    >
      <div class="grid gap-4">
        <BaseFormItem label="用户名" required>
          <BaseInput v-model="profile.username" :disabled="profileSaving" autocomplete="username" />
        </BaseFormItem>
        <BaseFormItem label="当前密码" required>
          <BaseInput
            v-model="profile.currentPassword"
            :disabled="profileSaving"
            type="password"
            autocomplete="current-password"
            placeholder="输入当前密码"
          />
        </BaseFormItem>
      </div>
      <template #footer>
        <BaseButton variant="secondary" :disabled="profileSaving" @click="usernameOpen = false">
          关闭
        </BaseButton>
        <BaseButton
          :loading="profileSaving"
          :disabled="!profile.username.trim() || !profile.currentPassword"
          @click="saveProfile"
        >
          保存
        </BaseButton>
      </template>
    </BaseModal>

    <BaseModal
      v-model="passwordOpen"
      title="修改密码"
      description="当前会话会保留，其他登录会话将失效"
      tone="info"
      size="sm"
      :dismissible="!passwordSaving"
    >
      <div class="grid gap-4">
        <BaseFormItem label="当前密码" required>
          <BaseInput
            v-model="password.currentPassword"
            :disabled="passwordSaving"
            type="password"
            autocomplete="current-password"
          />
        </BaseFormItem>
        <BaseFormItem label="新密码" required>
          <BaseInput
            v-model="password.newPassword"
            :disabled="passwordSaving"
            type="password"
            autocomplete="new-password"
          />
        </BaseFormItem>
        <BaseFormItem
          label="确认新密码"
          required
          :error="password.confirmPassword && password.newPassword !== password.confirmPassword ? '两次输入的密码不一致' : undefined"
        >
          <BaseInput
            v-model="password.confirmPassword"
            :disabled="passwordSaving"
            type="password"
            autocomplete="new-password"
          />
        </BaseFormItem>
      </div>
      <template #footer>
        <BaseButton variant="secondary" :disabled="passwordSaving" @click="passwordOpen = false">
          关闭
        </BaseButton>
        <BaseButton
          :loading="passwordSaving"
          :disabled="!password.currentPassword || !password.newPassword || password.newPassword !== password.confirmPassword"
          @click="savePassword"
        >
          更新密码
        </BaseButton>
      </template>
    </BaseModal>
  </div>
</template>

<style scoped>
.account-settings,
.profile-card {
  width: 100%;
}
</style>
