<script setup lang="ts">
import { computed } from 'vue'
import BaseCheckbox from '@/components/base/BaseCheckbox.vue'
import BaseInput from '@/components/base/BaseInput.vue'
import BaseNumberInput from '@/components/base/BaseNumberInput.vue'

export interface PlanLimitDraft {
  enabled: boolean
  value: number | string
}

const props = defineProps<{ label: string, kind: 'number' | 'money', disabled?: boolean }>()
const model = defineModel<PlanLimitDraft>({ required: true })
const numberValue = computed({
  get: () => Number(model.value.value) || 0,
  set: (value: number) => { model.value.value = value },
})
const moneyValue = computed({
  get: () => String(model.value.value),
  set: (value: string) => { model.value.value = value },
})
</script>

<template>
  <div class="grid gap-2 rounded-cp bg-cp-fill-quaternary p-3">
    <div class="flex flex-wrap items-center justify-between gap-x-3 gap-y-2">
      <span class="text-cp-sm font-emphasis text-cp-text">{{ props.label }}</span>
      <BaseCheckbox v-model="model.enabled" :label="`${props.label}启用`" show-label :disabled="props.disabled" />
    </div>
    <BaseNumberInput v-if="props.kind === 'number'" v-model="numberValue" :label="props.label" :disabled="props.disabled || !model.enabled" :min="0" />
    <BaseInput v-else v-model="moneyValue" type="number" min="0" step="any" :disabled="props.disabled || !model.enabled" :aria-label="props.label">
      <template #prefix>
        <span class="font-mono" aria-hidden="true">$</span>
      </template>
    </BaseInput>
  </div>
</template>
