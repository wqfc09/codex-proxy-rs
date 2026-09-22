<script setup lang="ts">
import { Save, ShieldCheck, Timer } from '@lucide/vue'

import BaseButton from '@/components/base/BaseButton.vue'
import BaseCard from '@/components/base/BaseCard.vue'
import BaseFormItem from '@/components/base/BaseForm/FormItem.vue'
import BaseForm from '@/components/base/BaseForm/index.vue'
import BaseInput from '@/components/base/BaseInput.vue'
import BaseSwitch from '@/components/base/BaseSwitch.vue'

defineProps<{
  loading: boolean
  sessionSaving: boolean
  turnstileSaving: boolean
  error: string
  turnstileHasSecret: boolean
  sessionValid: boolean
  turnstileValid: boolean
}>()

const emit = defineEmits<{
  saveSession: []
  saveTurnstile: []
}>()

const adminMinutes = defineModel<string>('adminMinutes', { required: true })
const userMinutes = defineModel<string>('userMinutes', { required: true })
const turnstileEnabled = defineModel<boolean>('turnstileEnabled', { required: true })
const turnstileSiteKey = defineModel<string>('turnstileSiteKey', { required: true })
const turnstileSecretKey = defineModel<string>('turnstileSecretKey', { required: true })
</script>

<template>
  <div class="grid gap-5">
    <BaseCard title="Session TTL" description="新登录会话使用这里的有效期；保存前仍沿用 deploy 启动配置">
      <template #actions>
        <BaseButton
          variant="secondary"
          :loading="sessionSaving"
          :disabled="loading || sessionSaving || !sessionValid"
          @click="emit('saveSession')"
        >
          <template #icon>
            <Save class="size-4" />
          </template>
          保存 Session TTL
        </BaseButton>
      </template>

      <BaseForm class="max-w-4xl sm:grid-cols-2">
        <BaseFormItem label="Admin Session TTL（分钟）" description="仅影响之后创建的 Admin 会话，范围 1 分钟～366 天">
          <BaseInput v-model="adminMinutes" type="number" min="1" max="527040" step="1" aria-label="Admin Session TTL 分钟">
            <template #prefix>
              <Timer class="size-4" />
            </template>
          </BaseInput>
        </BaseFormItem>
        <BaseFormItem label="User Session TTL（分钟）" description="仅影响之后创建的普通 User 会话，范围 1 分钟～366 天">
          <BaseInput v-model="userMinutes" type="number" min="1" max="527040" step="1" aria-label="User Session TTL 分钟">
            <template #prefix>
              <Timer class="size-4" />
            </template>
          </BaseInput>
        </BaseFormItem>
      </BaseForm>
    </BaseCard>

    <BaseCard title="Cloudflare Turnstile">
      <template #actions>
        <BaseButton
          variant="secondary"
          :loading="turnstileSaving"
          :disabled="loading || turnstileSaving || !turnstileValid"
          @click="emit('saveTurnstile')"
        >
          <template #icon>
            <ShieldCheck class="size-4" />
          </template>
          保存 Turnstile
        </BaseButton>
      </template>

      <div class="grid max-w-4xl gap-5">
        <div class="flex min-h-cp-control flex-wrap items-center justify-between gap-4 rounded-cp bg-cp-fill-quaternary px-4 py-3">
          <div>
            <p class="m-0 text-cp font-bold text-cp-text">
              登录时启用 Turnstile
            </p>
            <p class="mt-1 mb-0 text-cp-sm text-cp-text-secondary">
              启用后 Admin 与 User 的密码登录都必须通过 Siteverify
            </p>
          </div>
          <BaseSwitch v-model="turnstileEnabled" label="启用 Turnstile" />
        </div>

        <BaseForm class="sm:grid-cols-2">
          <BaseFormItem label="Site Key" description="会公开给登录页 Widget">
            <BaseInput v-model="turnstileSiteKey" autocomplete="off" aria-label="Turnstile Site Key" />
          </BaseFormItem>
          <BaseFormItem
            label="Secret Key"
            :description="turnstileHasSecret ? '后端已保存 secret；留空表示保持不变' : '尚未保存 secret；启用前必须填写'"
          >
            <BaseInput
              v-model="turnstileSecretKey"
              type="password"
              autocomplete="new-password"
              aria-label="Turnstile Secret Key"
              :placeholder="turnstileHasSecret ? '已保存；留空保持不变' : '输入 Secret Key'"
            />
          </BaseFormItem>
        </BaseForm>
        <p v-if="error" class="m-0 text-cp-sm font-emphasis text-cp-error-text" role="alert">
          {{ error }}
        </p>
      </div>
    </BaseCard>
  </div>
</template>
