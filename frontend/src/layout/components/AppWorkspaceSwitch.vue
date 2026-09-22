<script setup lang="ts">
import { CircleUserRound, ShieldCheck } from '@lucide/vue'
import { computed, nextTick, shallowRef, useTemplateRef, watch } from 'vue'
import BaseIconButton from '@/components/base/BaseIconButton.vue'
import BasePopover from '@/components/base/BasePopover.vue'
import BaseSegmented from '@/components/base/BaseSegmented.vue'

const props = defineProps<{ collapsed: boolean }>()
const model = defineModel<string>({ required: true })
const open = shallowRef(false)
const panel = useTemplateRef<HTMLElement>('panel')
const trigger = useTemplateRef<InstanceType<typeof BaseIconButton>>('trigger')
const options = [
  { label: '管理员', value: 'admin', icon: ShieldCheck },
  { label: '用户', value: 'user', icon: CircleUserRound },
]
const label = computed(() => `当前：${model.value === 'admin' ? '管理员' : '用户'}，切换工作空间`)

function select(value: string) {
  model.value = value
  open.value = false
}

watch(() => props.collapsed, () => {
  open.value = false
})
watch(open, async (value) => {
  await nextTick()
  if (value)
    panel.value?.querySelector<HTMLButtonElement>('[aria-checked="true"]')?.focus()
  else if (props.collapsed)
    (trigger.value?.$el as HTMLButtonElement | undefined)?.focus()
})
</script>

<template>
  <BaseSegmented
    v-if="!collapsed"
    v-model="model"
    label="工作空间"
    :options="options"
    class="w-full"
  />
  <BasePopover v-else v-model="open" placement="right" class="flex w-full justify-center">
    <template #trigger>
      <BaseIconButton ref="trigger" :label="label" size="lg" aria-haspopup="dialog" :aria-expanded="open">
        <ShieldCheck v-if="model === 'admin'" class="size-5" />
        <CircleUserRound v-else class="size-5" />
      </BaseIconButton>
    </template>
    <div ref="panel" role="dialog" aria-label="切换工作空间" class="w-56 max-w-[calc(100vw-2rem)] p-3">
      <BaseSegmented :model-value="model" label="工作空间" :options="options" class="w-full" @update:model-value="select" />
    </div>
  </BasePopover>
</template>
